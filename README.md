# storystick

Cabinetry cut-list tooling: `cd` into a project directory with a Shapr3D
STEP export in it, run `storystick`, assign materials to parts in a
terminal tree view, and generate a printable cutlist PDF -- one command,
no intermediate files, no flags to remember from run to run.

## Install

Requires a Rust toolchain ([rustup.rs](https://rustup.rs)).

```
cargo install --path cli
```

Or run it straight from the workspace without installing:

```
cargo run --release -p storystick
```

### Working on storystick itself

Run this once after cloning:

```
./scripts/dev.sh
```

It builds `storystick` and symlinks the binary onto your `PATH`, and
seeds a starter stock catalog (see below) if you don't already have one.
Because it's a symlink rather than a copy, a plain

```
cargo build --manifest-path cli/Cargo.toml
```

is all you need after that to pick up further edits -- no reinstalling,
no `--force`.

## Set up your stock catalog (once)

`scripts/dev.sh` does this step for you automatically (skipping it if the
file already exists) -- read on if you're setting it up by hand, or want
to know the format.

storystick looks for your stock catalog at `~/.config/storystick/stock.yaml`
by default (override per run with `--stock <path>`). This is the one
shop-wide list of what you can buy -- stable across every project, so it
lives outside any one project directory. Copy the example to get started:

```
mkdir -p ~/.config/storystick
cp scripts/stock.example.yaml ~/.config/storystick/stock.yaml
```

```yaml
materials:
  - name: "Baltic Birch 3/4 (finished 2 sides)"
    thickness_in: 0.75
    sheets:
      - length_in: 96
        width_in: 48
  - name: "Sande Ply 3/4 (utility, unseen parts)"
    thickness_in: 0.75
    sheets:
      - length_in: 96
        width_in: 48
```

Two materials can share a thickness -- that's what lets you keep hidden
parts (stretchers, nailers) off the good plywood by assigning them to the
cheaper material by name while reviewing.

## Starting a project

`cd` into the directory holding a Shapr3D STEP export and run:

```
storystick
```

storystick looks for a `storystick.yaml` by walking up from the current
directory, git-style -- the same file works from any subdirectory below
it, not just the exact folder it lives in. The first time you run it in a
new project directory, there's nothing to find yet, so it walks you
through creating one: it finds your `.step` file (or asks you to pick,
if there's more than one), then asks which of your stock catalog's
materials this project actually uses. That subset -- by name only, never
a copy of the catalog's thickness/sheet-size data -- becomes this
project's own `storystick.yaml`, checked in or kept alongside the model
however you like.

## Usage

Once a `storystick.yaml` exists, `storystick` loads it straight into an
in-terminal tree over the STEP file's parts, mirroring the CAD assembly's
own folder structure. Geometry always comes fresh from the STEP file;
everything else -- this project's material subset, bracket-tag rules,
kerf/trim/output settings, and any part-level overrides -- lives in that
one file, which is also everything `s` (save) writes back to.

### Two ways a part gets a material

A part's own Shapr3D name often carries a bracket tag (`[Panel]`,
`[Backer]`, etc.) -- storystick.yaml can map a tag to a material (its
`autofill:` section), and a part's material resolves in this order:

1. **An exception** -- a material assigned to this one specific part.
2. **Its tag's rule** -- if the part carries a bracket tag with a
   configured rule, it gets that rule's material.
3. **Unassigned** -- flagged, needing your attention.

Bulk-edit (`b`) is how a rule gets authored, almost always -- pick a
bracket tag from a list (each row showing its part count and current rule
material, if any), then pick a material to apply to all of them at once.
That choice is saved as the tag's *rule*, not stamped onto each of today's
parts individually: a
`[Panel]` part added in next week's re-export of the same model picks up
the existing `[Panel]` rule automatically, with nothing more to do. The
single-part picker (`m`) instead carves out an exception for just the
selected part, overriding whatever its tag's rule says. Clearing a part's
exception (choosing "(clear)" in the picker) removes it outright and
falls back to the tag's rule if one applies, or plain unassigned if not
-- there's no third "explicitly no material" state sitting in reserve.

Rows needing attention are flagged (`!`), and a folder shows how many
flagged parts it contains before you even expand it. A part is flagged
whenever it resolves to no material at all -- even if only one stock
material happens to match its thickness today, since that single match
is still an inference, not a decision. Also flagged: ambiguous geometry
(stepcrawl couldn't confidently read a dimension off the STEP file), and
an assigned material whose thickness doesn't actually match any of the
part's measured dimensions -- likely the wrong material got picked (or a
rule doesn't actually fit this particular part, which is worth a look).

Keys:

- `j`/`k` or arrows -- move the selection
- `h`/`l` or Left/Right -- collapse/expand a folder
- `e` / `c` -- expand / collapse every folder
- `Enter` -- expand/collapse a folder, or open the material picker on a part
- `m` -- open the material picker on the selected part directly (sets an
  exception for just this part)
- `b` -- bulk-edit: pick a bracket tag from a list (showing its part
  count and current rule material, if any), then set or clear that
  tag's rule for every part carrying it
- `g` -- swap the selected part's length and width
- `Ctrl-d` / `Ctrl-u` -- half-page down/up
- `s` -- save (writes the whole project file: rules, exceptions, settings)
- `p` -- print: open the kerf/trim-allowance settings, then generate the
  cutlist PDF from the tree's current (even unsaved) state
- `q` / `Esc` -- quit

The material picker is restricted to materials compatible with the
selected part's thickness (and, always, to this project's own material
subset -- never the whole shop catalog); pick "(clear)" to remove.

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
starts pre-filled with this project's saved kerf/trim settings, and
changing them here marks the project dirty (so `s` persists your new
defaults, not just the printed PDF).

Printing generates a PDF, organized for shop assembly one construction
stage at a time rather than as one flat document: a whole-project Bill of
Materials first, then per construction-stage section (Carcasses, Doors,
Face Frames, Drawers -- read off folder-naming keywords, in build order)
a front page (section title, that section's own BOM, and a blank ruled
Notes area, since these plans travel on a clipboard), that section's own
cut-sheet pages (every cut labeled with its own dimensions, ready to mark
up at the bench), and that section's own Parts Index mapping each
on-page code back to its full CAD path.

Options:

- `--stock <path>` -- override the default stock catalog location

## Help

```
storystick --help
```
