# ADR-0007: Geometry Kernel Scope, Hand-Roll First With a Fallback Ladder

**Status:** Superseded by [ADR-0008](0008-authoring-vs-output.md)

## Context

Geometry needs are deliberately narrow: sweep/extrude, revolve, and planar offset for the six fixed operation types, never general boolean CSG.

## Decision

Hand-roll targeted mesh generation for those cases rather than depend on a general-purpose kernel crate. If hand-rolling proves insufficient, fall back in order: `truck` or `fornjot` first, both pure-Rust CAD kernels, both young, evaluate before committing, then OpenCASCADE via WASM (`opencascade.js`) as a last resort. `manifold-3d`, already WASM-packaged and battle-tested, is a candidate specifically for a composite's union-of-footprints computation, independent of whichever sweep/revolve approach is used.

## Alternatives Considered

Depending on a general-purpose CSG/boolean kernel from the start. Rejected, the scope never needs general boolean CSG, so a general kernel is more dependency and complexity than the problem requires.

Committing directly to OpenCASCADE via WASM upfront. Rejected as a starting point, it's the heaviest option, worth trying the lighter pure-Rust options first.

## Consequences

Initial implementation risk sits on hand-rolled mesh generation working for the six fixed primitives. A real evaluation spike is still owed before choosing between `truck`, `fornjot`, or OpenCASCADE if hand-rolling doesn't pan out, that evaluation belongs in `docs/research/` when it happens, not in this ADR. `manifold-3d` adoption for union-of-footprints is decoupled from the sweep/revolve decision and can proceed independently.
