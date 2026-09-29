# storystick

Cabinetry cut-list tooling: open a Shapr3D STEP export directly, assign
materials to parts in a terminal tree view, and generate a printable
cutlist PDF (bill of materials + one labeled diagram per sheet) -- one
command, no intermediate files to manage by hand.

## Install

Requires a Rust toolchain ([rustup.rs](https://rustup.rs)).

```
cargo install --path cli
```

Or run it straight from the workspace without installing:

```
cargo run --release -p storystick -- <args>
```

## Set up your stock catalog (once)

storystick looks for your stock catalog at `~/.config/storystick/stock.yaml`
by default (override per run with `--stock <path>`). Copy the example to
get started:

```
mkdir -p ~/.config/storystick
cp scripts/stock.example.yaml ~/.config/storystick/stock.yaml
```

```yaml
materials:
  - name: "Baltic Birch 3/4 (finished 2 sides)"
    thickness_in: 0.75
    match: ["[Panel]"]
  - name: "Sande Ply 3/4 (utility, unseen parts)"
    thickness_in: 0.75
    match: ["[Backer]"]

sheets:
  - material: "Baltic Birch 3/4 (finished 2 sides)"
    length_in: 96
    width_in: 48
  - material: "Sande Ply 3/4 (utility, unseen parts)"
    length_in: 96
    width_in: 48
```

Two materials can share a thickness -- that's what lets you keep hidden
parts (stretchers, nailers) off the good plywood by assigning them to the
cheaper material by name while reviewing.

`match:` (optional, per material) lists the Shapr3D `[Bracket]` tokens
that should seed this material as a part's initial guess on load -- a
starting point only, applied when it's thickness-compatible with the
part; the review TUI's flag/correct workflow still catches anything it
missed or got wrong, and an unreviewed guess is never written to the
sidecar, so it stays free to change if the catalog does.

This catalog is meant to be stable across projects (a shop's materials
don't change per model), so it lives at a fixed location instead of being
passed on every run.

## Usage

```
storystick model.step
```

Opens an in-terminal tree over the STEP file's parts, mirroring the CAD
assembly's own folder structure -- no parts.csv or other intermediate
file to look at. Geometry always comes fresh from the STEP file; the only
thing that persists between runs is which material you assigned and
whether you swapped a part's length/width, saved next to the STEP file
as `model.materials.yaml`, keyed by `"path @ dimensions"` (dimensions
are part of the key because Shapr3D doesn't guarantee sibling part names
are unique, and it also means a part that's genuinely resized loses its
old overrides rather than silently keeping ones that might no longer
apply). It's a plain map you can read, diff, or edit by hand if you want;
a part you've only assigned a material to (the common case) is just one
`material:` line, e.g.:

```yaml
"Bench / Left Carcass / Body 03 @ 29.6250x17.2500x0.7500":
  material: Baltic Birch 3/4 (finished 2 sides)
```

A swapped part adds a `swapped: true` line. A part you've explicitly
cleared back to "no material" (rather than never having reviewed it at
all) is saved as `material: null` -- this is what keeps bracket-token
autofill (above) from re-suggesting a guess you already rejected.

Rows needing attention are flagged (`!`), and a folder shows how many
flagged parts it contains before you even expand it. A part is flagged
whenever it has no material assigned, even if only one stock material
happens to match its thickness today -- that single match is still an
inference, not a decision you made, and it becomes silently wrong the
moment a second material at that thickness joins the catalog. Also
flagged: ambiguous geometry (stepcrawl couldn't confidently read a
dimension off the STEP file), and an assigned material whose thickness
doesn't actually match any of the part's measured dimensions -- likely
the wrong material got picked.

Keys:

- `j`/`k` or arrows -- move the selection
- `h`/`l` or Left/Right -- collapse/expand a folder
- `e` / `c` -- expand / collapse every folder
- `Enter` -- expand/collapse a folder, or open the material picker on a part
- `m` -- open the material picker on the selected part directly
- `b` -- bulk-edit: pick a bracket tag (e.g. `[Panel]`), see a summary of
  how many parts carry it and what they're currently set to, then apply
  one material choice to all of them at once
- `g` -- swap the selected part's length and width
- `Ctrl-d` / `Ctrl-u` -- half-page down/up
- `s` -- save material assignments to the sidecar
- `p` -- print: open the kerf/trim-allowance settings, then generate the
  cutlist PDF from the tree's current (even unsaved) state
- `q` / `Esc` -- quit

The material picker is restricted to materials compatible with the
selected part's thickness; pick "(clear -- match by thickness alone)" to
unassign.

Of a part's three measured dimensions, the longer of the two in-plane
ones is guessed as length, the other as width, and the smallest as
thickness -- there's no "thickness axis" in a STEP file, this is just a
woodworking convention layered on after the fact. That guess is usually
right, but breaks for a part ripped narrower than it is thick (a narrow
trim strip, say): assigning it a material re-checks the guess against
that material's real thickness and relabels the three dimensions if a
better match turns up, automatically. Printing never rotates a part to
pack more densely -- grain always runs with length, and that's a fixed,
one-time choice per part, not something the packer searches over -- so
`g` is how you tell it a part's *other* edge should run with the grain
instead: it swaps length and width outright, on top of any material
correction.

`p` opens a small settings screen -- `Tab` (or the arrow keys) switches
between the two fields, type to edit, `Enter` prints, `Esc` cancels. It
starts pre-filled with the current session's values (`--kerf-in`/
`--trim-allowance-in` at launch, or whatever you last printed with).

Printing generates a PDF, organized
for shop assembly one construction stage at a time rather than as one
flat document: a whole-project Bill of Materials first, then per
construction-stage section (Carcasses, Doors, Face Frames, Drawers --
read off folder-naming keywords, in build order) a front page (section
title, that section's own BOM, and a blank ruled Notes area, since these
plans travel on a clipboard), that section's own cut-sheet pages (every
cut labeled with its own dimensions, ready to mark up at the bench), and
that section's own Parts Index mapping each on-page code back to its
full CAD path.

Options:

- `--stock <path>` -- override the default stock catalog location
- `--out <path>` -- cutlist PDF output path (default: `model.cutlist.pdf`
  next to the STEP file)
- `--kerf-in 0.125` -- saw kerf, inches (default `1/8`); the print
  screen's starting value, adjustable per print without restarting
- `--trim-allowance-in 0.25` -- extra rough-cut margin per part (default
  `0`, off); same starting-value role as `--kerf-in`. When set, each part
  shows a dashed rough-cut outline plus the final trim-to size, for a
  rough-with-track-saw / finish-with-table-saw workflow.

## Help

```
storystick --help
```
