# ADR-0015: Collision and Feature Detection

**Status:** Accepted

## Context

Composing real placed parts (ADR-0011, ADR-0012) makes it possible to check things a single flat import couldn't: whether the assembled design actually fits together, and whether every geometric feature, a dado, a bore, a joint between two parts, has accounted-for metadata. Cross-referencing CAD drawing notes against the model was explored first and found unreliable, a real Shapr3D DXF export was checked directly and carries dimension and leader data with real coordinates, but nothing links a note to a specific piece of 3D geometry by identity, only by 2D position, and it depends entirely on the user's drawing habits being complete on any given project. Geometric detection sidesteps that, it doesn't need the drawing at all.

## Decision

Two bodies whose bounding boxes overlap get a real proximity and intersection test, bounding-box overlap as a cheap first filter, a real geometry test only on pairs that survive it. Two outcomes matter: actual volumetric interference, a real design error, surfaced as a build warning, and touching or adjacent surfaces without interference, a candidate join location.

A candidate join location is checked against Story Stick's own stored metadata, keyed by the same stable part-name key used everywhere else (see [ADR-0016](0016-stable-part-identity.md)). No recorded join relationship for that pair means it's flagged, unresolved, non-blocking, the same treatment every other build-plan gap already gets (ADR-0010).

The same proximity math, applied to a single body against its own bounding box or convex hull instead of against another body, detects a feature: a region of missing material relative to raw stock, a dado, a bore, a groove. A bore, a cylindrical surface with circular bounding edges, is unambiguous enough to auto-suggest with real confidence. A channel feature is reliably detected as existing, and reliably classified as a rabbet specifically, an edge-open notch, distinguishable by topology alone, walls on one side versus open to the boundary. Dado versus groove needs the part's already-tracked grain-direction metadata combined with the channel's orientation, a fourth reuse of that one field, joining bin-packing, dado/groove/rabbet classification, and rendering.

## Alternatives Considered

Cross-referencing CAD drawing notes and dimensions against detected geometry by spatial position. Rejected: real dimension and leader data exists once a drawing is actually annotated, but there's no reliable link from a note to a specific model entity, only to a 2D coordinate, and the whole check depends on the user's drawing habits being consistently complete, not a safe foundation for a validation feature.

General mesh-mesh boolean intersection via a CAD kernel. Rejected: this is standard proximity and intersection testing, not construction, doesn't need kernel-scale machinery. Consistent with the earlier conclusion that nothing here actually required OpenCASCADE.

## Consequences

Needs a real geometric library or hand-rolled routines for bounding-box and mesh-proximity testing, `parry3d` was flagged earlier as a plausible fit, worth verifying directly before committing rather than assumed. Feeds straight into the Build Warnings system already established: unresolved joins and unclassified features surface exactly the way any other unresolved metadata does, visible, never blocking.
