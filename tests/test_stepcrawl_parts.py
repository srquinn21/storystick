"""Unit tests for storystick.stepcrawl's pure grouping/QC logic
(group_parts, off_grid, with_known_thickness) -- operates on plain rows
and PartGroups, no STEP file involved.
"""

import pytest

from storystick.stepcrawl import PartGroup, PartInstance
from storystick.stepcrawl._parts import group_parts, off_grid, with_known_thickness

MM_PER_IN = 25.4


def _in(v):
    return v * MM_PER_IN


def test_group_parts_counts_identical_dimensions_as_one_group():
    rows = [
        ("Root / Bench / Carcasses / Carcass A", "[Panel] Bottom", _in(30.125), _in(16.0), _in(0.75), False),
        ("Root / Bench / Carcasses / Carcass B", "[Panel] Bottom", _in(30.125), _in(16.0), _in(0.75), False),
    ]

    groups = group_parts(rows)

    assert len(groups) == 1
    assert groups[0].qty == 2
    assert groups[0].top_folder == "Bench"


def test_group_parts_keeps_mirrored_parts_in_different_top_folders_separate():
    rows = [
        ("Root / Console A / Carcasses / Carcass A", "[Panel] Left", _in(29.75), _in(16.0), _in(0.75), False),
        ("Root / Console B / Carcasses / Carcass B", "[Panel] Right", _in(29.75), _in(16.0), _in(0.75), False),
    ]

    groups = group_parts(rows)

    assert len(groups) == 2, "identical dims in different top-level folders must stay separate line items"


def test_group_parts_tolerates_floating_point_drift():
    rows = [
        ("Root / Bench / Carcass A", "[Backer]", _in(31.0) + 1e-6, _in(17.0), _in(0.25), False),
        ("Root / Bench / Carcass B", "[Backer]", _in(31.0) - 1e-6, _in(17.0), _in(0.25), False),
    ]

    groups = group_parts(rows)

    assert len(groups) == 1


def test_group_parts_instance_path_drops_root_but_keeps_top_folder():
    rows = [("Root Product / Bench / Carcasses / Carcass A", "[Backer]", _in(31.0), _in(17.0), _in(0.25), False)]

    groups = group_parts(rows)

    assert groups[0].instances[0].path == "Bench / Carcasses / Carcass A / [Backer]"


def test_off_grid_flags_dimension_past_tolerance():
    part = PartGroup(
        top_folder="Bench",
        length_mm=_in(30.0 + 0.02),  # ~0.02" off a 1/16" increment
        width_mm=_in(16.0),
        thickness_mm=_in(0.75),
        instances=(PartInstance(path="Bench / X"),),
    )

    result = off_grid(part, tolerance_in=0.005)

    assert result.length_in is not None
    assert result.width_in is None
    assert result.thickness_in is None
    assert bool(result) is True


def test_off_grid_silent_within_tolerance():
    part = PartGroup(
        top_folder="Bench",
        length_mm=_in(30.0),
        width_mm=_in(16.0),
        thickness_mm=_in(0.75),
        instances=(PartInstance(path="Bench / X"),),
    )

    result = off_grid(part)

    assert not result


def test_with_known_thickness_corrects_a_narrow_rip():
    # Ripped from 3/4" stock down to a 1/4" wide strip: naive largest/
    # middle/smallest guessing calls 0.25" the thickness.
    misguessed = PartGroup(
        top_folder="Bench",
        length_mm=_in(24.0),
        width_mm=_in(0.75),
        thickness_mm=_in(0.25),
        instances=(PartInstance(path="Bench / X"),),
    )

    corrected = with_known_thickness(misguessed, thickness_mm=_in(0.75), tolerance_mm=1.0)

    assert corrected.thickness_mm == pytest.approx(_in(0.75))
    assert corrected.width_mm == pytest.approx(_in(0.25))
    assert corrected.length_mm == pytest.approx(_in(24.0))
    assert corrected.instances == misguessed.instances


def test_with_known_thickness_raises_when_nothing_matches():
    part = PartGroup(
        top_folder="Bench",
        length_mm=_in(24.0),
        width_mm=_in(16.0),
        thickness_mm=_in(0.75),
        instances=(PartInstance(path="Bench / X"),),
    )

    with pytest.raises(ValueError):
        with_known_thickness(part, thickness_mm=_in(0.25), tolerance_mm=1.0)
