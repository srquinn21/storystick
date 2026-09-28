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
  - name: "Sande Ply 3/4 (utility, unseen parts)"
    thickness_in: 0.75

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
thing that persists between runs is which material you assigned to which
part, saved next to the STEP file as `model.materials.yaml` (a plain
`"path @ dimensions" -> material` map you can read, diff, or edit by hand
if you want -- dimensions are part of the key because Shapr3D doesn't
guarantee sibling part names are unique, and it also means a part that's
genuinely resized loses its old assignment rather than silently keeping
one that might no longer apply).

Rows needing attention are flagged (`!`), and a folder shows how many
flagged parts it contains before you even expand it: ambiguous geometry,
or an unassigned part whose thickness matches more than one stock
material (generating the cutlist would then route it onto whichever one
it finds space on first).

Keys:

- `j`/`k` or arrows -- move the selection
- `h`/`l` or Left/Right -- collapse/expand a folder
- `Enter` -- expand/collapse a folder, or open the material picker on a part
- `m` -- open the material picker on the selected part directly
- `Ctrl-d` / `Ctrl-u` -- half-page down/up
- `s` -- save material assignments to the sidecar
- `c` -- generate the cutlist PDF from the tree's current (even unsaved) state
- `q` / `Esc` -- quit

The material picker is restricted to materials compatible with the
selected part's thickness; pick "(clear -- match by thickness alone)" to
unassign.

Generating the cutlist prints the bill of materials to the console and
writes a PDF: a BOM page, one page per sheet (every cut labeled with its
own dimensions, ready to mark up at the bench), and a Parts Index mapping
each on-page code back to its full CAD path.

Options:

- `--stock <path>` -- override the default stock catalog location
- `--out <path>` -- cutlist PDF output path (default: `model.cutlist.pdf`
  next to the STEP file)
- `--kerf-in 0.125` -- saw kerf, inches (default `1/8`)
- `--trim-allowance-in 0.25` -- extra rough-cut margin per part (default
  `0`, off). When set, each part shows a dashed rough-cut outline plus
  the final trim-to size, for a rough-with-track-saw /
  finish-with-table-saw workflow.

## Help

```
storystick --help
```
