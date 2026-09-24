# ADR-0014: Exploded View and Click-to-Inspect

**Status:** Accepted

## Context

Large assemblies, 150-plus parts spanning a wall, get visually messy to inspect as one static view. The folder hierarchy that comes through CAD import is real and reliable, verified directly against an actual STEP export, `PRODUCT`/`NEXT_ASSEMBLY_USAGE_OCCURRENCE` carries genuine nested names, not just a flat body list. That hierarchy is worth using for inspection, not just organization.

## Decision

Clicking a folder triggers an exploded view: every body within that folder's subtree is pushed outward from its own centroid along the vector toward its parent group's centroid, scaled by a factor. Explosion is hierarchical, computed level by level through the nesting, a sub-assembly's own children explode relative to its center, and it in turn explodes relative to the level above, not one flat push of every leaf part in every direction at once.

This is a render-time transform only. It never touches the imported geometry itself, exploding is purely how already-tessellated pieces are positioned for viewing.

Clicking an individual body, exploded or not, raycasts to resolve which part was picked and surfaces its details: dimensions, grain direction, any relationship or resolution metadata, any flags.

## Alternatives Considered

A flat, non-hierarchical explosion, pushing every leaf body away from one global center. Rejected: ignores sub-assembly grouping, reads chaotically for anything with real nested structure, and the actual hierarchy is already available, so there's no reason to discard it in favor of a flatter, worse result.

## Consequences

Needs per-body centroid computation, already required for BOM and bin-packing, so no new geometric capability. Needs a stable mapping from a rendered mesh back to its source part's identity (see [ADR-0016](0016-stable-part-identity.md)) for picking, the same bookkeeping already required for metadata decoration and join detection. No new dependency, this composes entirely from bounding-box and centroid math already planned, plus standard raycasting, already the normal way Three.js handles interaction.
