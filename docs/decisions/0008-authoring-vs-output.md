# ADR-0008: Authoring vs. Output, Story Stick Reads, It Doesn't Author

**Status:** Accepted

## Context

The original plan (ADR-0001 through ADR-0007) had Story Stick as a full CAD-adjacent authoring tool: its own 3D viewport, its own sketch/cut/join UI, an event-sourced Rust/WASM core, and a hand-rolled geometry kernel to generate cut geometry from scratch.

Two things tested that plan against reality. First, a from-scratch script parsing a Shapr3D STEP export into a cutlist and BOM took about an hour to write, far less than the authoring tool would cost, and it already covered part of the value the authoring tool was chasing. Second, reflecting on how the last real project's planning actually worked: a checklist written by hand the night before a build, used to think through the build and catch gotchas ahead of time, not consulted step-by-step at the bench. The dimensions got referenced; the sequence didn't. The value was in writing the checklist, not in having it as a followable artifact, the rehearsal, not the script.

Verified directly against real exported files rather than assumed: a STEP export carries reliable B-rep geometry and named, nested assembly structure, but no feature or parametric history, that doesn't survive CAD interchange in general, not a Shapr3D-specific gap. A DXF export, once real dimensions and notes are added in Shapr3D, carries genuine coordinate-anchored dimension and leader data, though nothing links a dimension or note to a specific 3D model entity by ID, only by position.

Shapr3D (or any CAD tool that exports STEP and DXF) is also already a required part of the workflow independent of this decision, it's needed for 3D printing work regardless of what Story Stick does.

## Decision

Story Stick is not an authoring tool. Design, geometry construction, and drawing production happen in Shapr3D. Story Stick reads those exports, STEP for geometry and hierarchy, DXF for dimensions and notes, read-only and uni-directional. It never writes back to them. Its job is downstream of design: turn exported geometry into a bill of materials and bin-packed stock layouts, with anything the file formats can't carry, grain direction, species, and so on, supplied by the user as metadata decorated onto the parts list.

## Alternatives Considered

The original full authoring tool (ADR-0001 through ADR-0007). Rejected: the precise, tool-assigned, step-by-step instructions it was built to produce turned out not to be what actually gets used at the bench once a build has been rehearsed. A drawing and a parts list, which Shapr3D already produces, cover what's actually referenced during a build. Translating a drawing into a cut sequence is a base craft skill, not a gap this tool needs to fill.

A middle option: keep a dedicated tool, but scope it to "resolution" only, open an exported STEP file inside it and assign tools, generate reference-edge measurements, and produce detailed instructions there instead of authoring from scratch. Rejected for the same underlying reason as the full authoring tool, generated step-by-step instructions aren't the artifact that gets used, regardless of how it's produced.

## Consequences

ADR-0001 through ADR-0007 are superseded by this decision. Their shared premise, that Story Stick authors geometry and therefore needs an event-sourced core, a hand-rolled geometry kernel, and a Rust/WASM-plus-thin-UI split to support that, no longer holds.

What follows from here instead: a read-only STEP/DXF reader, a way to match parts across re-imports as a design changes in Shapr3D (STEP's own entity IDs aren't stable across exports, see [ADR-0016](0016-stable-part-identity.md)), user-supplied metadata decoration for anything the file formats can't carry, and BOM/bin-packing computation as the actual output. Native platform is back on the table as a real option, not WASM by default, since there's no browser-based authoring UI left to justify that constraint.

See [ADR-0011](0011-part-sources-and-composition.md) for the fuller picture: CAD import is one of several sources a Part's geometry can have, not the only one, and Story Stick's actual role is compositional source of truth for the project, not just a passive reader of one export. This doesn't change the boundary drawn here, no geometry construction, no kernel, it corrects the framing.
