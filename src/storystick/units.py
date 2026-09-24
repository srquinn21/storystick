"""Shared display-formatting for inch measurements -- used by cli.py's
console BOM table and diagrams' PDF, so both agree on the same
woodworking fractional convention instead of drifting independently.
Deliberately not used for parts.csv/bom.csv, which stay decimal since
those are edited and re-imported, not read at the bench.
"""

from __future__ import annotations

from fractions import Fraction

MM_PER_IN = 25.4


def mm_to_in(value_mm: float) -> float:
    return value_mm / MM_PER_IN


def format_inches(value_in: float, *, denominator: int = 32) -> str:
    """Format a decimal inch measurement as a woodworking-style mixed
    fraction (35.25 -> '35 1/4"'), rounded to the nearest 1/denominator
    (default 1/32", matching craft tolerance) and reduced to its simplest
    form (24/32 -> 3/4). A whole number prints with no fraction
    (96.0 -> '96"'); a sub-inch value drops the leading zero
    (0.75 -> '3/4"', not '0 3/4"').
    """
    total = round(value_in * denominator)
    whole, remainder = divmod(total, denominator)
    if remainder == 0:
        return f'{whole}"'
    frac = Fraction(remainder, denominator)
    if whole == 0:
        return f'{frac.numerator}/{frac.denominator}"'
    return f'{whole} {frac.numerator}/{frac.denominator}"'


def format_mm_in(value_mm: float, *, denominator: int = 32) -> str:
    return format_inches(mm_to_in(value_mm), denominator=denominator)
