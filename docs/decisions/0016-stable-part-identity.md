# ADR-0016: Stable Part Identity Across Re-Imports

**Status:** Accepted

## Context

ADR-0008 established that Story Stick reads STEP exports read-only and re-imports them as a design changes in Shapr3D. Re-importing only has value if it can reconcile against what's already been decorated, material, provenance, join relationships, rather than starting every part over from a blank slate on every import. That requires a stable way to say "this body in the new export is the same logical part as that body in the old one."

Verified directly against a real Shapr3D STEP export: STEP carries no feature or parametric history, that doesn't survive CAD interchange in general, not a Shapr3D-specific gap (see ADR-0008). Its own entity numbering (`#123`, `#456`, and so on) isn't a usable substitute either, re-exporting the same unchanged design reassigns different numbers, they're serialization artifacts, not persistent identifiers. Nothing internal to the file survives a re-export that could serve as identity.

What does survive intact is the assembly hierarchy's names, folder/component names and leaf body names, carried faithfully through STEP's `PRODUCT`/`NEXT_ASSEMBLY_USAGE_OCCURRENCE` structure. But Shapr3D's own duplication behavior actively works against relying on names alone: copying a body auto-appends a number "in various ways," inconsistently, with no uniqueness guarantee, confirmed directly against a real export where four distinct bodies in the same folder all carried the literal name `Body 03 (4)`.

## Decision

A part's identity key is its full path, root through every containing folder to its own name, nothing else. Two bodies with the same path and name across two imports are the same logical part; anything else is new and needs decoration from scratch.

Dimensions are explicitly not part of the identity key. They're tracked separately, as a content fingerprint on an already-identified part. If a re-import matches on identity but the fingerprint changed, that's a staleness flag on the existing decoration, "this part's geometry changed since you last confirmed its material," not a new part losing its history.

Uniqueness is enforced per level, not tree-wide: no two siblings, folders or bodies, under the same immediate parent may share a name. Enforcing that at every level guarantees the full path is unique by construction, no whole-tree scan needed.

A naming collision found on import is a hard failure, not silently resolved. The diagnostic reports every colliding path, the count, and each instance's own dimensions, so the user can find the offending body in Shapr3D by matching what's on screen, since the name alone won't distinguish them and nothing else addressable exists to point to instead.

## Alternatives Considered

STEP's own entity IDs as the identity key. Rejected: verified directly, not stable across separate exports of the same design.

Including dimensions in the identity key itself, so a resized part is automatically treated as new. Rejected: resizing is one of the most common edits during ordinary design iteration, far more frequent than an actual rename, and this would silently discard a part's decoration on nearly every edit cycle, the exact friction the intermediate-representation approach exists to avoid. Dimensions still matter, as a fingerprint on an already-identified part, not as identity.

An automatic tiebreaker for name collisions, an occurrence index within a container, insertion order, instead of a hard failure. Rejected: naming discipline is the only guarantee available given nothing else survives export, and silently resolving a collision papers over the exact ambiguity that makes two colliding bodies indistinguishable later. An import error, fixed once in Shapr3D, is cheaper than a wrong match discovered downstream in a BOM.

## Consequences

Import needs a validation pass, before any decoration or resync logic runs, that walks the assembly tree and hard-fails with a full diagnostic, colliding paths, counts, per-instance dimensions, on any sibling-name collision.

The practical discipline this puts on the user is narrower than "name everything carefully." Collision risk concentrates specifically at body/sub-assembly duplication in Shapr3D, that's the one operation that hands back a non-unique name. The habit is to rename immediately after any copy/mirror/array operation, before moving on to the next thing.

This is the identity mechanism [ADR-0009](0009-named-type-templates.md) (named type templates), [ADR-0011](0011-part-sources-and-composition.md) (multi-source parts and resync applicability), [ADR-0012](0012-placement-mechanics.md) (multi-instance placement resync), and [ADR-0015](0015-collision-and-feature-detection.md) (join-relationship keying) already assume and build on.
