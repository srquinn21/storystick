"""A body's own vertex points (local modeling frame) and its bounding box.

Ignores assembly placement transforms (ITEM_DEFINED_TRANSFORMATION /
AXIS2_PLACEMENT_3D): each body's vertices are already given in its own
local modeling frame, which is what a cut list wants (part dimensions),
not where the body sits in the room. Exact for the rectilinear,
axis-aligned panels typical of cabinetry; a body modeled with an off-axis
rotation relative to its own local frame would need real transform math.
"""

from __future__ import annotations

import math

from storystick.stepcrawl._entities import refs, typed

STRUCTURAL_PASSTHROUGH = {
    "CLOSED_SHELL",
    "ADVANCED_FACE",
    "FACE_BOUND",
    "FACE_OUTER_BOUND",
    "EDGE_LOOP",
    "ORIENTED_EDGE",
    "VERTEX_POINT",
}

ARC_SAMPLES = 64


def get_cartesian(entities, id_):
    _, args = typed(entities, id_)
    return tuple(float(x) for x in args[1].strip("()").split(","))


def get_direction(entities, id_):
    return get_cartesian(entities, id_)


def get_vertex_point(entities, id_):
    _, args = typed(entities, id_)
    return get_cartesian(entities, refs(args[1])[0])


def sub3(a, b):
    return (a[0] - b[0], a[1] - b[1], a[2] - b[2])


def dot3(a, b):
    return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]


def cross3(a, b):
    return (
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    )


def sample_conic_edge(entities, curve_id, curve_type, v1_id, v2_id, same_sense):
    """Sample points along the actual swept arc of a CIRCLE/ELLIPSE edge,
    between its two real endpoints, in the correct direction -- so a bulge
    past the endpoints (an arch, not just a corner fillet) gets captured for
    the bounding box instead of just the two vertices. This is what "the
    max distance along that edge, including the bulge" actually requires:
    you can't get it from the endpoints alone, you have to walk the curve.

    Returns a list of sampled 3D points (empty if the curve type isn't
    handled, e.g. B_SPLINE_CURVE).
    """
    if curve_type not in ("CIRCLE", "ELLIPSE"):
        return []
    _, cargs = typed(entities, curve_id)
    placement_id = refs(cargs[1])[0]
    _, pargs = typed(entities, placement_id)
    origin = get_cartesian(entities, refs(pargs[1])[0])
    zdir = get_direction(entities, refs(pargs[2])[0])
    xdir = get_direction(entities, refs(pargs[3])[0])
    ydir = cross3(zdir, xdir)

    if curve_type == "CIRCLE":
        a = b = float(cargs[2])
    else:  # ELLIPSE: semi-axis along xdir, semi-axis along ydir
        a, b = float(cargs[2]), float(cargs[3])

    def to_local(p):
        d = sub3(p, origin)
        return (dot3(d, xdir), dot3(d, ydir))

    def angle_of(p_local):
        # eccentric angle: divide out each semi-axis before atan2
        return math.atan2(p_local[1] / b, p_local[0] / a)

    p1_l = to_local(get_vertex_point(entities, v1_id))
    p2_l = to_local(get_vertex_point(entities, v2_id))
    theta1, theta2 = angle_of(p1_l), angle_of(p2_l)

    # STEP: same_sense True means v1->v2 follows increasing curve parameter.
    if same_sense:
        sweep = (theta2 - theta1) % (2 * math.pi)
    else:
        sweep = -((theta1 - theta2) % (2 * math.pi))

    points = []
    for i in range(ARC_SAMPLES + 1):
        theta = theta1 + sweep * i / ARC_SAMPLES
        x, y = a * math.cos(theta), b * math.sin(theta)
        points.append(
            (
                origin[0] + x * xdir[0] + y * ydir[0],
                origin[1] + x * xdir[1] + y * ydir[1],
                origin[2] + x * xdir[2] + y * ydir[2],
            )
        )
    return points


def collect_points(entities, brep_id):
    """Collect a body's own points: straight-edge vertices directly, and
    curved edges (CIRCLE/ELLIPSE) by sampling the real swept arc so a bulge
    past the endpoints is captured -- what you actually need to know how
    wide a board to cut the profile from, not just the two tangent points.

    `unreliable` comes back True for a curve type we don't sample (e.g. a
    B_SPLINE_CURVE profile), meaning the bounding box for that body only
    reflects its straight edges and shouldn't be trusted for the 1/16" check.
    """
    points = []
    seen = set()
    unreliable = False

    def walk(id_):
        nonlocal unreliable
        if id_ in seen:
            return
        seen.add(id_)
        typ, args = typed(entities, id_)
        if typ is None:
            return
        if typ == "CARTESIAN_POINT":
            coords = tuple(float(x) for x in args[1].strip("()").split(","))
            points.append(coords)
            return
        if typ == "EDGE_CURVE":
            # name, vertex1, vertex2, curve_geometry, same_sense
            v1_id, v2_id = refs(args[1])[0], refs(args[2])[0]
            curve_refs = refs(args[3])
            curve_type = typed(entities, curve_refs[0])[0] if curve_refs else None
            if curve_type and curve_type != "LINE":
                same_sense = args[4].strip() == ".T."
                arc_points = sample_conic_edge(entities, curve_refs[0], curve_type, v1_id, v2_id, same_sense)
                if arc_points:
                    points.extend(arc_points)
                else:
                    unreliable = True
            for a in (args[1], args[2]):
                for r in refs(a):
                    walk(r)
            return
        if typ in STRUCTURAL_PASSTHROUGH or typ == "MANIFOLD_SOLID_BREP":
            for r in refs(",".join(args)):
                walk(r)
            return
        # unknown/geometry-definition entity (PLANE, DIRECTION, ...): don't follow

    walk(brep_id)
    return points, unreliable


def bbox(points):
    xs = [p[0] for p in points]
    ys = [p[1] for p in points]
    zs = [p[2] for p in points]
    return (
        max(xs) - min(xs),
        max(ys) - min(ys),
        max(zs) - min(zs),
    )
