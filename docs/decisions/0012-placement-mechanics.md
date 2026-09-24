# ADR-0012: Placement Mechanics, Span-Derived Dimensions and Multi-Instance Resync

**Status:** Accepted

## Context

ADR-0011 established that every Part composes into a project through Placement, regardless of where its geometry came from. Working through a real build (a bottom cabinet) surfaced that Placement needs to do more than position parts. A face-frame rail spanning between two stiles needs its own length to stay derived from those stiles' actual positions, not typed once and left to drift, otherwise there's no real reason to compose in Story Stick instead of just typing static numbers directly into Shapr3D.

Separately, composing sub-assemblies from CAD import means the same imported design, a door, a hardware model, gets placed more than once across a project. Updating the source design needs to reach every placed instance, not one at a time, that's the exact cumbersome, manual re-place workflow that motivated moving composition into Story Stick in the first place.

## Decision

**Span-derived dimensions.** A Part's dimension can be defined as the distance between two reference points or edges, on other parts, or Assembly Table anchors, optionally plus or minus a fixed offset. This is tracked as a directed dependency: this part depends on those references, and its dimension is recomputed whenever a referenced point moves, for example after a re-import changes a part's size. This is dependency-tracked measurement, not general geometric constraint solving, no angular constraints, no tangency, no arbitrary equation systems, just "this dimension equals the distance between these two points." A dependency cycle, two parts each deriving a dimension from the other, is a validation error, flagged, never silently resolved. Deliberately not solver-grade.

**Multi-instance placement and resync.** A CAD-imported sub-assembly can be placed more than once in a project, each placement its own instance sharing the same source. Re-importing an updated export of that source, matched by the same stable name-based key already used for ordinary re-imports (see [ADR-0016](0016-stable-part-identity.md)), updates every placed instance from that one re-import, not one at a time.

## Alternatives Considered

General parametric constraint solving, the full BIM approach. Rejected: solves a broader problem than the actual use cases need. Tenons, mortises, and spans in cabinetry are span relationships between reference points, not arbitrary sketch constraint networks, and a full solver reopens the construction-UI and kernel-scale cost ADR-0008 walked away from.

Static, one-time-computed dimensions for typed parts, no live dependency tracking. Rejected: this was the actual alternative on the table this session, and it fails the real test, if a typed part's dimension can't stay derived from a live reference, composing in Story Stick has no advantage over modeling everything in Shapr3D with fixed numbers.

Per-instance resync, requiring each placed copy of a sub-assembly to be updated by hand when its source changes. Rejected: reproduces the exact problem that motivated this whole direction, Shapr3D's lack of linked sub-assemblies (see ADR-0008's context).

## Consequences

The Part/Placement model needs a dependency graph for span-derived dimensions, topological recompute on change, and cycle detection as a validation error, not a solver. It also needs a distinction between a sub-assembly's source, the imported design, and its instances, each individual placement, with resync applied at the source and propagated to every instance.

Both of these compose with the stable name-based key already established for re-import matching (see [ADR-0016](0016-stable-part-identity.md)). No new identity mechanism, just two more uses of the one already decided.
