"""Parts: grouping raw solid-body measurements into quantity-counted
PartGroups, and the two dimension-relabeling QC helpers (off_grid,
with_known_thickness). Pure data logic -- no STEP-specific parsing here.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
from typing import Optional, Tuple

MM_PER_IN = 25.4

__all__ = [
    "PartInstance",
    "PartGroup",
    "OffGrid",
    "group_parts",
    "off_grid",
    "with_known_thickness",
]


@dataclass(frozen=True)
class PartInstance:
    """One physical body contributing to a PartGroup."""

    path: str
    unreliable: bool = False


@dataclass(frozen=True)
class PartGroup:
    """A set of interchangeable parts: same top-level folder, same L/W/T.

    length_mm/width_mm/thickness_mm are a guess (largest/middle/smallest of
    the body's three bounding-box dimensions) -- there's no "thickness
    axis" in a STEP file, this is a woodworking convention layered on
    after the fact. See with_known_thickness() for correcting a guess that
    picked a narrow rip's width as if it were the stock's thickness.
    """

    top_folder: str
    length_mm: float
    width_mm: float
    thickness_mm: float
    instances: Tuple[PartInstance, ...]

    @property
    def qty(self) -> int:
        return len(self.instances)

    @property
    def unreliable(self) -> bool:
        return any(i.unreliable for i in self.instances)


@dataclass(frozen=True)
class OffGrid:
    """Signed deviation (inches) from the nearest grid increment, per
    dimension -- only set for a dimension that exceeds tolerance."""

    length_in: Optional[float] = None
    width_in: Optional[float] = None
    thickness_in: Optional[float] = None

    def __bool__(self) -> bool:
        return any(v is not None for v in (self.length_in, self.width_in, self.thickness_in))


def group_parts(rows):
    """rows: iterable of (path, body_name, dx_mm, dy_mm, dz_mm, unreliable)
    -- one row per solid body, `path` being its full assembly-folder
    prefix (root product name first, as returned by ancestor_path).

    Groups bodies into PartGroups by (top-level folder, L/W/T rounded to
    0.001") -- two panels count as "the same part, qty 2" if they agree to
    the nearest thousandth of an inch, regardless of tiny STEP
    floating-point drift. Grouping is scoped to each body's top-level
    folder rather than globally, so mirrored parts with identical
    dimensions (Left Carcass / Right Carcass) stay separate line items
    instead of merging into one "qty 2" -- they live in different
    top-level assemblies and get built/installed as distinct pieces.

    Caller is responsible for row order (e.g. sort by (path, name)) if a
    deterministic PartGroup order matters.
    """
    groups = {}  # (top_folder, key) -> list[(instance_path, l_mm, w_mm, t_mm, unreliable)]
    order = []

    for path, name, dx, dy, dz, unreliable in rows:
        segments = path.split(" / ")
        top_folder = segments[1] if len(segments) > 1 else segments[0]
        l_mm, w_mm, t_mm = sorted((dx, dy, dz), reverse=True)
        key = (
            round(l_mm / MM_PER_IN, 3),
            round(w_mm / MM_PER_IN, 3),
            round(t_mm / MM_PER_IN, 3),
        )
        instance_path = " / ".join(segments[1:] + [name]) if len(segments) > 1 else " / ".join(segments + [name])

        group_key = (top_folder, key)
        if group_key not in groups:
            groups[group_key] = []
            order.append(group_key)
        groups[group_key].append((instance_path, l_mm, w_mm, t_mm, unreliable))

    result = []
    for group_key in order:
        top_folder, _ = group_key
        items = groups[group_key]
        _, l_mm, w_mm, t_mm, _ = items[0]
        instances = tuple(PartInstance(path=p, unreliable=u) for p, _, _, _, u in items)
        result.append(PartGroup(top_folder=top_folder, length_mm=l_mm, width_mm=w_mm, thickness_mm=t_mm, instances=instances))
    return result


def off_grid(part: PartGroup, *, grid_in: float = 1 / 16, tolerance_in: float = 0.005) -> OffGrid:
    """How far off the nearest `grid_in` increment each of part's
    dimensions falls, in inches -- only reported past `tolerance_in`."""

    def delta(value_mm: float) -> Optional[float]:
        value_in = value_mm / MM_PER_IN
        nearest = round(value_in / grid_in) * grid_in
        d = value_in - nearest
        return d if abs(d) > tolerance_in else None

    return OffGrid(
        length_in=delta(part.length_mm),
        width_in=delta(part.width_mm),
        thickness_in=delta(part.thickness_mm),
    )


def with_known_thickness(part: PartGroup, thickness_mm: float, *, tolerance_mm: float = 1.0) -> PartGroup:
    """Relabel `part`'s dimensions given a known material thickness (e.g.
    from an explicit material assignment): whichever of its three measured
    dimensions is closest to `thickness_mm` becomes thickness, and the
    other two are sorted into length/width. Corrects the default
    largest/middle/smallest guess for a piece ripped narrower than it is
    thick. Raises ValueError if no dimension is within tolerance_mm.
    """
    dims = [part.length_mm, part.width_mm, part.thickness_mm]
    idx = min(range(3), key=lambda i: abs(dims[i] - thickness_mm))
    if abs(dims[idx] - thickness_mm) > tolerance_mm:
        raise ValueError(
            f"no dimension of this {part.top_folder} part "
            f"({part.length_mm:.2f} / {part.width_mm:.2f} / {part.thickness_mm:.2f} mm) "
            f"is within {tolerance_mm} mm of known thickness {thickness_mm} mm"
        )
    thickness = dims.pop(idx)
    length, width = sorted(dims, reverse=True)
    return replace(part, length_mm=length, width_mm=width, thickness_mm=thickness)
