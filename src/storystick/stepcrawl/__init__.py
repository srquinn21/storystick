"""STEP -> parts list.

Parses a Shapr3D STEP export into a quantity-grouped list of parts, each
labeled with its assembly folder path. This package does exactly one job --
it knows nothing about stock sheets, nesting, or rendering.

This is a lightweight, dependency-free STEP reader, not a real CAD kernel:
it treats the file as a graph of `#id = ENTITY(args...)` records and walks
each MANIFOLD_SOLID_BREP down through its topology to collect that body's
own vertex points (bounding box = part dimensions), then walks the
NEXT_ASSEMBLY_USAGE_OCCURRENCE chain upward to reconstruct the Shapr3D
folder tree it lives in. See storystick.stepcrawl._geometry and ._assembly
for the details.

Length units are assumed to be millimeters, matching a typical Shapr3D
STEP export; this reader never checks a file's declared SI_UNIT.
"""

from __future__ import annotations

from storystick.stepcrawl._assembly import build_indices
from storystick.stepcrawl._entities import parse_entities, typed, unquote
from storystick.stepcrawl._geometry import bbox, collect_points
from storystick.stepcrawl._parts import (
    OffGrid,
    PartGroup,
    PartInstance,
    group_parts,
    off_grid,
    with_known_thickness,
)

__all__ = [
    "PartInstance",
    "PartGroup",
    "OffGrid",
    "extract_parts",
    "off_grid",
    "with_known_thickness",
]


def extract_parts(step_path):
    """Parse a Shapr3D STEP export into grouped, quantity-counted parts."""
    entities = parse_entities(step_path)
    breps_by_container, container_pd, ancestor_path = build_indices(entities)

    rows = []  # (path_str, body_name, dx, dy, dz, unreliable)
    for rep_id, container_name, solid_ids in breps_by_container:
        pd_id = container_pd(rep_id)
        prefix = ancestor_path(pd_id) if pd_id is not None else [container_name]
        for solid_id in solid_ids:
            typ, sargs = typed(entities, solid_id)
            if typ != "MANIFOLD_SOLID_BREP":
                continue
            body_name = unquote(sargs[0])
            points, unreliable = collect_points(entities, solid_id)
            if not points:
                continue
            dx, dy, dz = bbox(points)
            path_str = " / ".join(prefix)
            rows.append((path_str, body_name, dx, dy, dz, unreliable))

    rows.sort(key=lambda r: (r[0], r[1]))
    return group_parts(rows)
