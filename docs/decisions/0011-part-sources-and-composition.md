# ADR-0011: Parts Have Multiple Sources, Story Stick Is the Compositional Source of Truth

**Status:** Accepted

## Context

ADR-0008 established that Story Stick doesn't author geometry, it reads CAD exports (STEP, DXF), read-only. That framing undersold what the architecture actually became through this session's exploration. Composing multiple independently-authored sub-assemblies, a Shaker Door design, a hardware manufacturer's STEP model, via Placement showed that CAD import is one kind of input a Part can have, not the defining one. And plain rectangular stock, a nominal 2x4, a sheet good cut to size, anything with no distinguishing features, never needed a CAD roundtrip at all, typed dimensions were sufficient and were part of the original model before any of this session's pivots.

## Decision

A Part's geometry can come from any of several sources: imported from an external CAD file, typed directly as dimensions, for plain stock with no distinguishing features, or drawn from a persistent hardware/parts catalog. Regardless of source, every Part composes into the project the same way, through Placement.

Story Stick is the source of truth for the project as a whole: its composition, structure, the relationships between parts, and all metadata, grain direction, tool assignment, provenance, warnings. It is not the source of truth for complex part geometry, that stays wherever it was authored, consistent with ADR-0008's boundary. Typed-dimension parts are the one case where Story Stick is trivially also the geometry's source, since a rectangular prism defined by three numbers isn't authoring in the sense ADR-0008 was actually scoped around, no sketch, no extrude, no kernel.

## Alternatives Considered

Treating CAD import as the only real source of a Part's geometry, requiring every part, however simple, to be modeled externally and imported. Rejected: this was never actually the model, even before this session's architecture work, and it adds needless friction for the common case, a nominal 2x4 doesn't need a CAD roundtrip.

Keeping Story Stick framed narrowly as a read-only validation/output tool over one CAD export, matching ADR-0008's original wording exactly. Rejected as an inaccurate description of what the architecture became once sub-assembly composition and placement entered the picture. This doesn't change ADR-0008's actual boundary, no geometry construction, no kernel, it corrects a framing that undersold Story Stick's compositional role.

## Consequences

The Part model needs to track which source a given part's geometry came from, CAD import, typed dimensions, or catalog, since that determines whether re-import/stable-key sync logic (see [ADR-0016](0016-stable-part-identity.md)) applies at all. Typed and catalog parts have no external file to resync against; only CAD-imported ones do.

This ADR doesn't introduce new capability by itself. It formalizes something already true (typed dimensions for simple stock) and names the compositional role Story Stick had already taken on. The mechanics of placement and multi-instance resync for CAD-imported sub-assemblies are a separate decision, ADR-0012.
