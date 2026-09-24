"""storystick: cabinetry cut-list tooling, as one command with subcommands.

CLI glue only -- no domain logic. Each subcommand does file I/O and unit
conversion (files are in inches for hand-editing; the libraries work in
mm) and calls straight into storystick.stepcrawl / .nesting / .diagrams.

TODO once storystick.stepcrawl.with_known_thickness is implemented: for a
parts.csv row whose `material` column names a Material whose thickness
doesn't match that row's own measured thickness_in (e.g. a narrow rip
mislabeled by stepcrawl's largest/middle/smallest guess), call
with_known_thickness() to relabel length/width/thickness from the
assigned Material's real thickness, instead of trusting the row's own
thickness_in unconditionally as it does today.
"""

from __future__ import annotations

import argparse
import csv
import sys
import textwrap
from pathlib import Path

import yaml

from storystick.diagrams import render_pdf
from storystick.nesting import Material, PackablePart, StockSheet, bill_of_materials, pack
from storystick.stepcrawl import extract_parts
from storystick.units import format_mm_in

MM_PER_IN = 25.4

PARTS_FIELDS = [
    "top_folder",
    "label",
    "path",
    "length_in",
    "width_in",
    "thickness_in",
    "unreliable",
    "material",
]

BOM_FIELDS = ["material", "length_in", "width_in", "thickness_in", "qty"]


def _in(mm: float) -> float:
    return round(mm / MM_PER_IN, 4)


def _mm(inches) -> float:
    return float(inches) * MM_PER_IN


def _leaf(path: str) -> str:
    return path.split(" / ")[-1]


def _open_out_text(path: str):
    """'-' means stdout; anything else is a path resolved against the
    current working directory (pathlib/open's normal default), so relative
    paths work the same as absolute ones -- no extra handling needed."""
    if path == "-":
        return sys.stdout
    return open(Path(path), "w", newline="")


def _close_out(out):
    if out is not sys.stdout:
        out.close()


# --- parts: STEP file -> parts.csv ---


def cmd_parts(args: argparse.Namespace) -> None:
    """One row per physical body, not per PartGroup. stepcrawl groups
    interchangeable bodies together (same folder, same L/W/T) because
    that's a meaningful fact about the geometry, but collapsing a group
    into one qty-N row here -- with its member paths packed into a single
    cell -- makes individual paths unreadable/uneditable and invites the
    qty/members pair drifting out of sync. A flat one-row-per-body file
    doesn't have that problem, and identical parts still land on adjacent
    rows (stepcrawl's own grouping order), so hand-editing a material
    override for a whole spec is still just: select the block, fill down.
    """
    parts = extract_parts(args.step_path)

    out = _open_out_text(args.out)
    try:
        writer = csv.DictWriter(out, fieldnames=PARTS_FIELDS)
        writer.writeheader()
        for part in parts:
            for instance in part.instances:
                writer.writerow(
                    {
                        "top_folder": part.top_folder,
                        "label": _leaf(instance.path),
                        "path": instance.path,
                        "length_in": _in(part.length_mm),
                        "width_in": _in(part.width_mm),
                        "thickness_in": _in(part.thickness_mm),
                        "unreliable": instance.unreliable,
                        "material": "",
                    }
                )
    finally:
        _close_out(out)


# --- cutlist: parts.csv + stock.yaml -> one printable PDF (BOM page + a
# cut-diagram page per sheet), with the BOM also echoed to the console.
#
# There's no separate "diagram" or "bom" step: both come from the same
# pack() call, and since bin-packing is NP-hard (we optimize, we don't
# solve), two separate runs aren't guaranteed to agree even on identical
# input. Treating them as independently regenerable risks the BOM silently
# drifting from whatever the diagram actually shows. So one command, one
# calculation, both outputs, every time.
#
# No intermediate JSON either (YAGNI) -- nothing downstream consumes it
# now that rendering isn't a separate step; Layout only ever exists
# in-memory between pack() and render_pdf() within this one call.


def _read_parts_csv(path: Path):
    """One CSV row is one physical body -- see cmd_parts -- so this is a
    direct 1:1 read, no reassembly needed. `path` (the full CAD path) is
    the part's identity; `label` is only ever a display convenience.
    `material`, when filled in, pins this part to one specific Material
    by name (see storystick.nesting.PackablePart.material_name) -- blank
    means "any material at this part's own thickness_in is fine."
    """
    parts = []
    with open(path, newline="") as f:
        for row in csv.DictReader(f):
            parts.append(
                PackablePart(
                    label=row["path"] or row["label"] or row["top_folder"],
                    length_mm=_mm(row["length_in"]),
                    width_mm=_mm(row["width_in"]),
                    thickness_mm=_mm(row["thickness_in"]),
                    qty=1,
                    material_name=(row.get("material") or "").strip() or None,
                )
            )
    return parts


def _read_stock_yaml(path: Path):
    """stock.yaml has two sections: `materials` (a name -> thickness
    catalog -- see storystick.nesting.Material) and `sheets` (purchasable
    sizes, each naming which material they're a sheet of). A sheet's
    thickness always comes from its named material, never repeated
    per-sheet, so two sheet sizes of the same material can't drift out of
    sync on thickness.
    """
    with open(path) as f:
        doc = yaml.safe_load(f) or {}

    materials = {
        entry["name"]: Material(name=entry["name"], thickness_mm=_mm(entry["thickness_in"]))
        for entry in doc.get("materials", [])
    }

    stock = []
    for entry in doc.get("sheets", []):
        material_name = entry["material"]
        if material_name not in materials:
            raise ValueError(f"stock.yaml: sheet references unknown material {material_name!r}")
        stock.append(
            StockSheet(
                material=materials[material_name],
                length_mm=_mm(entry["length_in"]),
                width_mm=_mm(entry["width_in"]),
            )
        )
    return stock


def _print_bom_table(lines, out=sys.stderr) -> None:
    if not lines:
        return
    rows = [
        (
            str(line.qty) + "x",
            line.stock.material.name,
            f"{format_mm_in(line.stock.length_mm)} x {format_mm_in(line.stock.width_mm)} x {format_mm_in(line.stock.thickness_mm)}",
        )
        for line in lines
    ]
    qty_w = max(len(r[0]) for r in rows) + 1
    label_w = max(len(r[1]) for r in rows) + 2
    print("bill of materials:", file=out)
    for qty, label, dims in rows:
        print(f"  {qty:<{qty_w}}{label:<{label_w}}{dims}", file=out)


def _write_bom_csv(lines, path: Path) -> None:
    with open(path, "w", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=BOM_FIELDS)
        writer.writeheader()
        for line in lines:
            writer.writerow(
                {
                    "material": line.stock.material.name,
                    "length_in": _in(line.stock.length_mm),
                    "width_in": _in(line.stock.width_mm),
                    "thickness_in": _in(line.stock.thickness_mm),
                    "qty": line.qty,
                }
            )


def _write_pdf(pdf_bytes: bytes, path: str) -> None:
    if path == "-":
        sys.stdout.buffer.write(pdf_bytes)
        return
    with open(Path(path), "wb") as f:
        f.write(pdf_bytes)


def cmd_cutlist(args: argparse.Namespace) -> None:
    parts = _read_parts_csv(args.parts_csv)
    stock = _read_stock_yaml(args.stock_yaml)
    trim_allowance_mm = args.trim_allowance_in * MM_PER_IN
    layout = pack(parts, stock, kerf_mm=args.kerf_in * MM_PER_IN, trim_allowance_mm=trim_allowance_mm)

    if layout.unplaced:
        print(f"warning: {len(layout.unplaced)} part(s) could not be placed", file=sys.stderr)

    bom = bill_of_materials(layout)
    _print_bom_table(bom)
    if args.bom_csv:
        _write_bom_csv(bom, args.bom_csv)

    _write_pdf(render_pdf(layout, bom, trim_allowance_mm=trim_allowance_mm), args.out)


# --- argument parser ---


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="storystick",
        description="Cabinetry cut-list tooling: STEP file -> parts list -> printable cutlist PDF (BOM + cut diagrams).",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=textwrap.dedent(
            """\
            examples:
              storystick parts model.step -o parts.csv
              storystick cutlist parts.csv stock.yaml -o cutlist.pdf
              storystick cutlist parts.csv stock.yaml -o cutlist.pdf --bom-csv bom.csv

            paths may be relative to the current directory or absolute.
            omit -o (or pass -o -) to write to stdout.
            """
        ),
    )
    subparsers = parser.add_subparsers(dest="command", metavar="command")

    p_parts = subparsers.add_parser(
        "parts",
        help="STEP file -> parts.csv",
        description="Parse a Shapr3D STEP export into parts.csv, one row per physical "
        "body with its full CAD assembly path, ready to hand-edit (the material column) "
        "before `cutlist`.",
    )
    p_parts.add_argument("step_path", type=Path, help="path to a Shapr3D STEP export")
    p_parts.add_argument("-o", "--out", default="-", metavar="PATH", help="output CSV path (default: stdout)")
    p_parts.set_defaults(func=cmd_parts)

    p_cutlist = subparsers.add_parser(
        "cutlist",
        help="parts.csv + stock.yaml -> printable cutlist PDF",
        description="Nest a parts.csv (from `parts`, optionally hand-edited) onto a "
        "stock.yaml catalog, bucketed by thickness (or by a specific named material, "
        "when a part's `material` column pins one), and write one printable PDF: a "
        "bill-of-materials page, then one page per stock sheet with every cut on it "
        "labeled with its own dimensions, ready to cross off with a marker as you go. "
        "Always prints the bill of materials to the console too -- it's a view of "
        "this same packing result, not something to regenerate separately.",
    )
    p_cutlist.add_argument("parts_csv", type=Path, help="parts list, from `storystick parts`")
    p_cutlist.add_argument("stock_yaml", type=Path, help="stock sheet catalog (what you can buy, not what's on hand)")
    p_cutlist.add_argument("-o", "--out", default="-", metavar="PATH", help="output PDF path (default: stdout)")
    p_cutlist.add_argument("--bom-csv", type=Path, metavar="PATH", help="also save the bill of materials as CSV")
    p_cutlist.add_argument("--kerf-in", type=float, default=1 / 8, help="saw kerf, inches (default: 1/8\")")
    p_cutlist.add_argument(
        "--trim-allowance-in",
        type=float,
        default=0.0,
        help="extra rough-cut margin per part, inches (default: 0, i.e. off). When set, the "
        "PDF shows a dashed rough-cut outline plus the final trim-to size on every part.",
    )
    p_cutlist.set_defaults(func=cmd_cutlist)

    return parser


def main(argv=None) -> None:
    parser = build_parser()
    args = parser.parse_args(argv)
    if not hasattr(args, "func"):
        parser.print_help()
        raise SystemExit(1)
    args.func(args)


if __name__ == "__main__":
    main()
