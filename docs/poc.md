# Proof of Concept: STEP-to-Cutlist Tool

## Why this exists

Story Stick (see [product.md](./product.md) / [high-level-design.md](./high-level-design.md))
is a large investment: a full CAD-adjacent authoring tool with its own
geometry kernel, event-sourced core, and a Rust/WASM + TypeScript UI
split. Before committing to that, this proof of concept exists to
de-risk it -- by building something much smaller that solves today's
actual pain point (turning a Shapr3D STEP export into a cutlist) and
using it on real projects to find and smooth out the rough edges of the
CAD-modeling workflow itself. What's learned here -- what a woodworker
actually needs out of "plan in CAD, cut in the shop" -- should inform
whether, and how, the larger tool gets built at all.

This POC lives entirely in this repo's `cli` and `core` Rust crates, with
its own, much simpler architecture (no WASM, no TypeScript UI, no event
sourcing) -- it is not a code-level stepping stone toward the design in
`docs/`, only a workflow/lessons-learned one. See the root
[README.md](../README.md) for what it does and how to run it.

## Design principle: Shapr3D is the source of truth

All part identification, dimensioning, and organization happens in
Shapr3D, via the modeler's own body/folder grouping and naming
discipline -- not in storystick. Shapr3D lets bodies be grouped into
folders, and folders nested into subfolders; the STEP export reflects
that nesting directly, and storystick's parts tree mirrors it exactly
(see `cli/src/review/tree.rs`).

storystick's job is purely to *decorate* that structure: assign
material, and generate a cutlist, bill of materials, and project plan
from it. It never edits geometry or part organization -- that discipline
stays entirely on the Shapr3D side. Practically, this means: prefer a
single, full-project STEP export over splitting the export into multiple
STEP files per construction stage (carcasses / doors / face frames /
etc.) -- see "Multi-file project mode" below for why that was considered
and set aside in favor of this.

## What's built today (brief)

- STEP import (a Shapr3D export) into a flat parts list that mirrors the
  CAD folder hierarchy.
- A ratatui TUI: browse the tree, see flagged (ambiguous / unassigned /
  thickness-mismatched) parts, assign material from a stock catalog,
  swap a part's length/width, adjust kerf/trim-allowance, and generate
  the cutlist PDF.
- Bracket-token material autofill: a stock-catalog material's optional
  `match:` list of tokens (e.g. `match: ["[Panel]"]`) seeds an initial
  material guess for any part whose path carries that token -- a
  starting point only, never persisted to the sidecar unless the user
  actually reviews and decides on it (see the sidecar note below).
- Bulk-edit mode (`b`): lists every distinct bracket tag in the tree
  with its part count, a confirmation summary (count + current material
  spread) before committing, then applies one material choice to every
  part carrying that tag via the same picker used for single-part
  assignment. Material only, never the length/width swap -- grain
  orientation stays a per-part decision.
- A sidecar YAML persists material/swap overrides per part, keyed by
  path + raw dimensions, auto-discovered next to the STEP file. The
  sidecar distinguishes "never decided" (autofill is free to keep
  guessing) from "explicitly set to no material" (`material: null`,
  permanent) -- without this, a rejected autofill guess would be
  indistinguishable from an unreviewed part and just come back next run.
- Packing: a rip-first guillotine heuristic that prioritizes never
  stranding a sheet (or leaving an unusable remainder) over keeping
  same-size parts strictly grouped in adjacent strips -- confirmed on a
  real project (1/4" sheets went from 3 down to 2).
- The cutlist PDF is organized for shop assembly, not as one flat
  document: a whole-project Bill of Materials first, then per
  construction-stage section (in build order) -- a front page (section
  title + that section's own BOM + a blank ruled Notes area, since these
  plans travel on a clipboard), that section's own cut-sheet pages, and
  that section's own Parts Index (codes restart at P001 per section).
  Construction stage is read off folder-naming keywords (Carcass, Door,
  Face Frame, Drawer); no dedicated section title page, since the front
  page already carries content worth the paper.
- A recent polish pass: tree divider + column colors, a colored/
  right-justified title bar, a timed-out transient status line with a
  resting keyboard-help line, and a "save before exiting?" confirmation.

## Open design thread: handling a large, single-project export

### The original problem

The user's actual workflow builds by material/construction-stage
sections -- carcasses first, then paint-grade uppers, then doors and
face frames -- exporting each from Shapr3D's "Projects" feature
separately, one at a time. The ask: a bill of materials aggregated
across the whole project, and a cutlist PDF with pages grouped by
section, without giving up the "review one export at a time" habit.

### Considered: a multi-file project mode

Idea: a `project.yaml` manifest listing STEP files (sections) in build
order, plus a project-scoped stock subset (materials "added" to the
project from the global catalog); sections auto-detected via a
`{filename}-{label}.step` naming convention (split on the *last*
hyphen, so a base name that itself contains hyphens, like "Built-In",
still works); running `storystick` with no path in a directory would
create the manifest if missing (an interactive "create experience":
detect candidate sections, pick project stock) or load the existing one.

Set aside in favor of the single-export approach below, because
splitting the project across multiple STEP files re-creates, as a
storystick problem, exactly the aggregation that a single load would get
for free -- and a manifest plus multiple sidecars is more moving parts
(more places to drift out of sync) in service of a habit (reviewing one
chunk of the model at a time) that doesn't actually require multiple
*files*, just a tree big enough to review in chunks. See "Tree UX at
project scale" below.

### Current direction: a single full-project STEP export

Export the whole CAD project as one STEP file; storystick organizes and
displays it in a way that's easy to digest in one pass, rather than
requiring the export to already be pre-split by construction stage. This
makes BOM/cutlist aggregation free -- one `load_parts` call, one `pack`
call, one BOM, no cross-file stitching, no manifest.

**Resolved:** doors and face frames will get the same self-describing
folder naming that carcasses already have ("Left Carcass", "Right
Carcass", "Middle Carcass" -> "Left Door", "Face Frame Rail," etc.), so a
full-project export's paths alone are enough to classify a part's
construction stage. No separate "Projects"-selection-derived data is
needed to reconstruct section grouping for the PDF -- construction-stage
classification (`cli/src/sections.rs`) and bracket-token material
matching (below) share the same "keyword found somewhere in a path"
shape (`core::tags::classify_by_keyword` / `extract_tags`), just against
plain folder-naming keywords instead of `[Bracket]` tokens, so the two
features stay conceptually one mechanism even though they're two thin
functions. **Implemented**: `core::diagrams::group_sheets_by_section`
groups a `Layout`'s sheets by section, and `render_pdf` renders the PDF
section by section (see "What's built today" above).

### Bracket-token conventions (`[Panel]`, `[Backer]`, etc.)

Parts are already named in Shapr3D's tree with bracket tokens intended
for material auto-fill -- e.g. `[Panel]` implying "goes on show-face
plywood," `[Backer]` implying "goes on cheap ply." Planned: an optional
`match:` list of tokens per stock-catalog material entry (e.g.
`match: ["[Panel]"]`), so `load_parts` seeds an initial material guess
whenever a token appears in a part's path -- a starting point only; the
existing flag/correct review workflow still catches anything the token
missed or got wrong. **Implemented** (`cli/src/autofill.rs`,
`cli/src/stock.rs`'s `match:` field). Only accepted when the guessed
material is actually thickness-compatible with the part, and never
frozen into the sidecar unless the user reviews it (see the sidecar
tri-state note above) -- an unreviewed guess is free to change the next
time stock.yaml does.

### Bulk-edit mode (by tag)

Proposed to handle scale once a project is one big export: a mode that
scans the tree for bracket tokens, lists the distinct tags found with a
count each, lets the user pick one, and applies a single material choice
(via the existing picker UI) to every part carrying that tag at once --
with a confirmation summary first (count affected, current material
spread among them) since it's a mass write, not a single-part edit.

**Resolved: material only, not the length/width swap.** Grain
orientation is a per-part, per-position decision (it depends on where in
the cabinet the part lands), not a uniform property of a tag like
`[Panel]` the way "goes on show-face plywood" is -- bulk-swapping a
whole tag group risks silently getting several parts' orientation wrong
at once, a mistake bulk material assignment doesn't share. **Implemented**
(key `b` in the review TUI).

### Tree UX at project scale

The tree already supports arbitrary folder depth (it's built
recursively), but today's UX assumes a tree small enough to expand-all
and scroll through freely. A full-project export will be much deeper and
wider -- multiple physical units, each with multiple carcasses, each
with many sub-parts. Directions discussed, none implemented yet:

- Don't auto-expand by default -- start collapsed, use the existing
  per-folder flagged-count display to guide intentional drill-down.
- A "flagged only" filter/view: once bulk-tag-assignment handles the
  bulk of parts, what's left to check by hand is exactly the flagged
  ones -- filtering down to just those turns "scroll a huge tree" into
  "work through a short punch list."
- Jump-to / fuzzy search by name, rather than only level-by-level
  navigation.

**Resolved: deferred.** Which of these matters most in practice is best
judged once bulk-edit exists and a real full-project export has actually
been tried against it, rather than guessed at now -- revisit this
section once both of those are true.

## Status

The "Open design thread" above is now settled design, and every part of
it is implemented in `cli`/`core` except tree-scaling, which stays
deferred per that section's own "resolved: deferred" -- it's now
possible to try a real full-project export against bracket-token
autofill, bulk-edit, and section-grouped PDF output all together, which
is the trigger that section named for revisiting it. This document
exists so this design survives a context reset; update it if a decision
changes, and split a settled, load-bearing decision out into its own
file once one exists (this repo's ADR convention, in
`docs/decisions/`, applies to those).
