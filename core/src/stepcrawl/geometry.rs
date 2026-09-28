//! A body's own vertex points (local modeling frame) and its bounding box.
//!
//! Ignores assembly placement transforms (ITEM_DEFINED_TRANSFORMATION /
//! AXIS2_PLACEMENT_3D): each body's vertices are already given in its own
//! local modeling frame, which is what a cut list wants (part dimensions),
//! not where the body sits in the room. Exact for the rectilinear,
//! axis-aligned panels typical of cabinetry; a body modeled with an
//! off-axis rotation relative to its own local frame would need real
//! transform math.

use super::entities::{refs, typed};
use std::collections::{HashMap, HashSet};

type Point3 = (f64, f64, f64);

const ARC_SAMPLES: i32 = 64;

fn structural_passthrough(typ: &str) -> bool {
    matches!(
        typ,
        "CLOSED_SHELL"
            | "ADVANCED_FACE"
            | "FACE_BOUND"
            | "FACE_OUTER_BOUND"
            | "EDGE_LOOP"
            | "ORIENTED_EDGE"
            | "VERTEX_POINT"
    )
}

fn parse_coords(arg: &str) -> Point3 {
    let inner = arg.trim_matches(|c| c == '(' || c == ')');
    let mut it = inner.split(',').map(|x| x.trim().parse::<f64>().unwrap());
    (it.next().unwrap(), it.next().unwrap(), it.next().unwrap())
}

fn get_cartesian(entities: &HashMap<i64, String>, id: i64) -> Point3 {
    let (_, args) = typed(entities, id);
    parse_coords(&args[1])
}

fn get_direction(entities: &HashMap<i64, String>, id: i64) -> Point3 {
    get_cartesian(entities, id)
}

fn get_vertex_point(entities: &HashMap<i64, String>, id: i64) -> Point3 {
    let (_, args) = typed(entities, id);
    get_cartesian(entities, refs(&args[1])[0])
}

fn sub3(a: Point3, b: Point3) -> Point3 {
    (a.0 - b.0, a.1 - b.1, a.2 - b.2)
}

fn dot3(a: Point3, b: Point3) -> f64 {
    a.0 * b.0 + a.1 * b.1 + a.2 * b.2
}

fn cross3(a: Point3, b: Point3) -> Point3 {
    (a.1 * b.2 - a.2 * b.1, a.2 * b.0 - a.0 * b.2, a.0 * b.1 - a.1 * b.0)
}

/// Sample points along the actual swept arc of a CIRCLE/ELLIPSE edge,
/// between its two real endpoints, in the correct direction -- so a bulge
/// past the endpoints (an arch, not just a corner fillet) gets captured
/// for the bounding box instead of just the two vertices. This is what
/// "the max distance along that edge, including the bulge" actually
/// requires: you can't get it from the endpoints alone, you have to walk
/// the curve.
///
/// Returns an empty Vec if the curve type isn't handled (e.g. a
/// B_SPLINE_CURVE).
fn sample_conic_edge(
    entities: &HashMap<i64, String>,
    curve_id: i64,
    curve_type: &str,
    v1_id: i64,
    v2_id: i64,
    same_sense: bool,
) -> Vec<Point3> {
    if curve_type != "CIRCLE" && curve_type != "ELLIPSE" {
        return Vec::new();
    }
    let (_, cargs) = typed(entities, curve_id);
    let placement_id = refs(&cargs[1])[0];
    let (_, pargs) = typed(entities, placement_id);
    let origin = get_cartesian(entities, refs(&pargs[1])[0]);
    let zdir = get_direction(entities, refs(&pargs[2])[0]);
    let xdir = get_direction(entities, refs(&pargs[3])[0]);
    let ydir = cross3(zdir, xdir);

    let (a, b) = if curve_type == "CIRCLE" {
        let r: f64 = cargs[2].trim().parse().unwrap();
        (r, r)
    } else {
        // ELLIPSE: semi-axis along xdir, semi-axis along ydir
        let a: f64 = cargs[2].trim().parse().unwrap();
        let b: f64 = cargs[3].trim().parse().unwrap();
        (a, b)
    };

    let to_local = |p: Point3| -> (f64, f64) {
        let d = sub3(p, origin);
        (dot3(d, xdir), dot3(d, ydir))
    };
    // eccentric angle: divide out each semi-axis before atan2
    let angle_of = |p_local: (f64, f64)| -> f64 { (p_local.1 / b).atan2(p_local.0 / a) };

    let theta1 = angle_of(to_local(get_vertex_point(entities, v1_id)));
    let theta2 = angle_of(to_local(get_vertex_point(entities, v2_id)));

    let two_pi = std::f64::consts::TAU;
    // STEP: same_sense True means v1->v2 follows increasing curve parameter.
    let sweep = if same_sense {
        (theta2 - theta1).rem_euclid(two_pi)
    } else {
        -((theta1 - theta2).rem_euclid(two_pi))
    };

    let mut points = Vec::with_capacity(ARC_SAMPLES as usize + 1);
    for i in 0..=ARC_SAMPLES {
        let theta = theta1 + sweep * (i as f64) / (ARC_SAMPLES as f64);
        let x = a * theta.cos();
        let y = b * theta.sin();
        points.push((
            origin.0 + x * xdir.0 + y * ydir.0,
            origin.1 + x * xdir.1 + y * ydir.1,
            origin.2 + x * xdir.2 + y * ydir.2,
        ));
    }
    points
}

#[allow(clippy::too_many_arguments)]
fn walk(
    entities: &HashMap<i64, String>,
    id: i64,
    seen: &mut HashSet<i64>,
    points: &mut Vec<Point3>,
    unreliable: &mut bool,
) {
    if !seen.insert(id) {
        return;
    }
    let (typ, args) = typed(entities, id);
    let typ = match typ {
        Some(t) => t,
        None => return,
    };

    if typ == "CARTESIAN_POINT" {
        points.push(parse_coords(&args[1]));
        return;
    }

    if typ == "EDGE_CURVE" {
        // name, vertex1, vertex2, curve_geometry, same_sense
        let v1_id = refs(&args[1])[0];
        let v2_id = refs(&args[2])[0];
        let curve_refs = refs(&args[3]);
        let curve_type = if !curve_refs.is_empty() {
            typed(entities, curve_refs[0]).0
        } else {
            None
        };
        if let Some(ct) = curve_type {
            if ct != "LINE" {
                let same_sense = args[4].trim() == ".T.";
                let arc_points = sample_conic_edge(entities, curve_refs[0], &ct, v1_id, v2_id, same_sense);
                if !arc_points.is_empty() {
                    points.extend(arc_points);
                } else {
                    *unreliable = true;
                }
            }
        }
        for a in [&args[1], &args[2]] {
            for r in refs(a) {
                walk(entities, r, seen, points, unreliable);
            }
        }
        return;
    }

    if structural_passthrough(&typ) || typ == "MANIFOLD_SOLID_BREP" {
        let joined = args.join(",");
        for r in refs(&joined) {
            walk(entities, r, seen, points, unreliable);
        }
        // unknown/geometry-definition entity (PLANE, DIRECTION, ...): don't follow
    }
}

/// Collect a body's own points: straight-edge vertices directly, and
/// curved edges (CIRCLE/ELLIPSE) by sampling the real swept arc so a bulge
/// past the endpoints is captured -- what you actually need to know how
/// wide a board to cut the profile from, not just the two tangent points.
///
/// `unreliable` comes back true for a curve type not sampled (e.g. a
/// B_SPLINE_CURVE profile), meaning the bounding box for that body only
/// reflects its straight edges and shouldn't be trusted for the 1/16"
/// check.
pub fn collect_points(entities: &HashMap<i64, String>, brep_id: i64) -> (Vec<Point3>, bool) {
    let mut points = Vec::new();
    let mut seen = HashSet::new();
    let mut unreliable = false;
    walk(entities, brep_id, &mut seen, &mut points, &mut unreliable);
    (points, unreliable)
}

pub fn bbox(points: &[Point3]) -> (f64, f64, f64) {
    let (mut x_min, mut x_max) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut y_min, mut y_max) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut z_min, mut z_max) = (f64::INFINITY, f64::NEG_INFINITY);
    for &(x, y, z) in points {
        x_min = x_min.min(x);
        x_max = x_max.max(x);
        y_min = y_min.min(y);
        y_max = y_max.max(y);
        z_min = z_min.min(z);
        z_max = z_max.max(z);
    }
    (x_max - x_min, y_max - y_min, z_max - z_min)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bbox_computes_dimensions() {
        let points = vec![(0.0, 0.0, 0.0), (10.0, 20.0, 5.0), (2.0, 3.0, 1.0)];
        assert_eq!(bbox(&points), (10.0, 20.0, 5.0));
    }

    #[test]
    fn collect_points_walks_a_simple_line_edge_box() {
        // A minimal box: 8 corner points chained by LINE edges into one
        // loop, wrapped in the usual structural-passthrough chain.
        let mut entities: HashMap<i64, String> = HashMap::new();
        let corners = [
            (0.0, 0.0, 0.0),
            (10.0, 0.0, 0.0),
            (10.0, 5.0, 0.0),
            (0.0, 5.0, 0.0),
            (0.0, 0.0, 2.0),
            (10.0, 0.0, 2.0),
            (10.0, 5.0, 2.0),
            (0.0, 5.0, 2.0),
        ];
        // Points: ids 100..107
        for (i, (x, y, z)) in corners.iter().enumerate() {
            entities.insert(100 + i as i64, format!("CARTESIAN_POINT('',({x},{y},{z}))"));
        }
        // Vertices: ids 200..207, wrapping points 100..107
        for i in 0..8 {
            entities.insert(200 + i, format!("VERTEX_POINT('',#{})", 100 + i));
        }
        // A dummy LINE curve (contents never read for LINE type)
        entities.insert(300, "LINE('',#900,#901)".to_string());
        // Edge curves chaining vertex i -> vertex (i+1)%8, ids 400..407
        for i in 0..8 {
            let v1 = 200 + i;
            let v2 = 200 + (i + 1) % 8;
            entities.insert(400 + i, format!("EDGE_CURVE('',#{v1},#{v2},#300,.T.)"));
        }
        // Oriented edges wrapping each edge curve, ids 500..507
        for i in 0..8 {
            entities.insert(500 + i, format!("ORIENTED_EDGE('',*,*,#{},.T.)", 400 + i));
        }
        let oriented_refs: String = (0..8).map(|i| format!("#{}", 500 + i)).collect::<Vec<_>>().join(",");
        entities.insert(600, format!("EDGE_LOOP('',({oriented_refs}))"));
        entities.insert(700, "FACE_BOUND('',#600,.T.)".to_string());
        entities.insert(800, "ADVANCED_FACE('',(#700),#999,.T.)".to_string());
        entities.insert(900, "CLOSED_SHELL('',(#800))".to_string());
        entities.insert(1000, "MANIFOLD_SOLID_BREP('[Test]',#900)".to_string());

        let (points, unreliable) = collect_points(&entities, 1000);
        assert!(!unreliable);
        let (dx, dy, dz) = bbox(&points);
        assert_eq!((dx, dy, dz), (10.0, 5.0, 2.0));
    }
}
