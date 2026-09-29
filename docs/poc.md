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
- One project file (`storystick.yaml`), discovered by walking up from
  the current directory git-style -- `storystick` needs no path argument
  at all. A directory with none anywhere above it runs a short creation
  wizard instead of failing (finds the `.step` file, asks which of the
  global stock catalog's materials this project uses). The file holds
  this project's material subset (by name only), bracket-tag rules
  (`autofill`), kerf/trim/output settings, and per-part exceptions -- see
  "Unified project file" below.
- Bracket-tag material rules, project-scoped: `storystick.yaml`'s
  `autofill` maps a tag (e.g. `[Panel]`) to a material name. A part's
  material resolves as its own exception (if any) first, else its tag's
  rule (if any), else unassigned -- see "Unified project file" below for
  why this replaced the earlier stock-catalog-level `match:` idea.
- Bulk-edit mode (`b`): lists every distinct bracket tag in the tree
  with its part count, a confirmation summary (count + current material
  spread) before committing, then sets (or clears) that tag's rule --
  this is the primary, almost only way a rule gets authored. Material
  only, never the length/width swap -- grain orientation stays a
  per-part decision.
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

This idea's shape (a `project.yaml`, discovered by directory, with a
project-scoped stock subset and an interactive create experience) later
came back as "Unified project file" below -- for an unrelated reason
(consolidating what used to be three separate config surfaces into one)
and covering exactly one STEP file per project, not the multi-file
splitting this section rejected. The two shouldn't be conflated: this
section's "no" is still the answer for splitting a project across
multiple STEP exports.

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

### Unified project file (`storystick.yaml`)

Originally: a global stock-catalog `match:` list per material seeded an
autofill guess, and a per-STEP sidecar held per-part overrides
(material/swap), with a tri-state (never-decided / explicitly-cleared /
assigned) so a rejected guess wouldn't just come back next run. Both
ideas got superseded once bulk-edit (below) made a *project-scoped*,
user-authored rule the primary mechanism instead of a guess:

- **`match:` moved off the global catalog and into a per-project
  `autofill` map** (tag -> material name), since which material a tag
  means is a per-project call ("this project's `[Panel]`s are Baltic
  Birch"), not a shop-wide one. The global catalog (`cli/src/stock.rs`)
  went back to just being "what you can buy" -- no tag data at all.
- **The per-STEP sidecar folded into one project file**
  (`cli/src/project.rs`, `storystick.yaml`), discovered by walking up
  from the current directory (git-style) rather than passed on the
  command line -- see `project::discover`. One file now holds this
  project's own material subset (`materials:`, names only, resolved
  against the global catalog -- see `Project::resolve_materials`),
  bracket-tag rules (`autofill:`), kerf/trim/output settings, and
  per-part exceptions (`assignments:`, unchanged in shape from the old
  sidecar's `PartOverride`).
- **A part's material resolves as: its own exception, if any; else its
  tag's rule, if any; else unassigned, flagged** (`review::resolve_material`).
  This replaces the old tri-state entirely -- there's no longer a
  distinct "explicitly no material" state to hold in reserve on *either*
  side. Clearing a part's exception just deletes it and re-resolves from
  the current rule (or unassigned); clearing a tag's rule in bulk-edit
  just deletes that rule entry. A part with no tag and no rule that gets
  cleared stays plain unassigned and flagged -- there's no way to mark it
  "reviewed, intentionally has no material" short of actually assigning
  one, on the assumption that every real part in a cutlist eventually
  needs a real material.
- **Bulk-edit (`b`) is now the primary rule-authoring surface, not an
  optional bulk-write convenience**: confirming a tag's material always
  updates that tag's persistent rule (never stamps an exception onto
  today's matching parts), so a part with the same tag added in a future
  STEP re-export picks up the existing rule automatically -- the actual
  goal the whole tri-state exercise was chasing in the first place,
  achieved more directly once rules are explicit and project-scoped
  rather than silently-reapplied per-part guesses.
- **First run in a new project directory** (`project::discover` finds
  nothing walking up to the filesystem root) **triggers a short creation
  wizard** (`cli/src/wizard.rs`, plain stdin prompts, not a TUI screen):
  find the `.step` file (or ask, if more than one), ask which of the
  global catalog's materials this project uses, save. Bracket-tag rules
  are deliberately *not* asked about here -- bulk-edit is where those get
  authored, against real parts, not guessed at during setup.

**Implemented.**

### Validated material references

A material name is retyped, independently, in up to five places: the
global catalog's own `materials`/`sheets` cross-reference, a project's
`materials:` subset, and a project's `autofill:`/`assignments:` values.
Only two of those five were ever checked against the catalog at load
time (`stock::parse`'s sheet lookup, `Project::resolve_materials`) --
`autofill:`/`assignments:` values flowed straight from YAML into
`Part.material` unchecked. A typo there didn't error: it produced a part
displaying an unresolvable material name, not flagged "no material
assigned" (the field wasn't empty), silently wrong.

Considered introducing a stable material `id` distinct from the display
name (so renaming a material's label can't break a reference). Rejected
for now -- more ceremony than a small, hand-edited catalog format
warrants, and the actual bug isn't that names double as identity, it's
that referential integrity was checked inconsistently.

**Resolved:** `Project::validate_references`, called once right after
`resolve_materials` (before parts ever load), checks every `autofill`
value and `assignments[].material` against the project's own resolved
material subset, erroring with the offending key and a Levenshtein-based
"did you mean" suggestion on a near-miss. A bad or stale name is now a
loud failure at startup on every path, not just two of five.
`review::Part.material` was also changed from a bare `Option<String>` to
`Option<Material>`, resolved once (`review::find_material`) right after
that validation passes, so downstream code (dimension correction, the
picker, bulk-edit) carries a real `Material` instead of re-searching the
catalog by name at each use site. **Implemented.**

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
possible to try a real full-project export against bracket-tag rules,
bulk-edit, section-grouped PDF output, and the unified `storystick.yaml`
project file all together, which is the trigger tree-scaling's own
section named for revisiting it. This document exists so this design
survives a context reset; update it if a decision changes, and split a
settled, load-bearing decision out into its own file once one exists
(this repo's ADR convention, in `docs/decisions/`, applies to those) --
the unified project file and its exception/rule precedence model in
particular are probably due for one.
