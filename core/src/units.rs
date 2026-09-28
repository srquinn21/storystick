//! Shared display-formatting for inch measurements -- used by the CLI's
//! console BOM table and the PDF renderer, so both agree on the same
//! woodworking fractional convention instead of drifting independently.
//! Deliberately not used for parts.csv/bom.csv, which stay decimal since
//! those are edited and re-imported, not read at the bench.

pub const MM_PER_IN: f64 = 25.4;
pub const DEFAULT_DENOMINATOR: i64 = 32;

pub fn mm_to_in(value_mm: f64) -> f64 {
    value_mm / MM_PER_IN
}

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 {
        a.abs()
    } else {
        gcd(b, a % b)
    }
}

/// Format a decimal inch measurement as a woodworking-style mixed
/// fraction (35.25 -> "35 1/4\""), rounded to the nearest 1/denominator
/// (32 -> 1/32", matching craft tolerance) and reduced to its simplest
/// form (24/32 -> 3/4). A whole number prints with no fraction
/// (96.0 -> "96\""); a sub-inch value drops the leading zero
/// (0.75 -> "3/4\"", not "0 3/4\"").
pub fn format_inches_with(value_in: f64, denominator: i64) -> String {
    let total = (value_in * denominator as f64).round() as i64;
    let whole = total.div_euclid(denominator);
    let remainder = total.rem_euclid(denominator);
    if remainder == 0 {
        return format!("{whole}\"");
    }
    let g = gcd(remainder, denominator);
    let num = remainder / g;
    let den = denominator / g;
    if whole == 0 {
        format!("{num}/{den}\"")
    } else {
        format!("{whole} {num}/{den}\"")
    }
}

pub fn format_inches(value_in: f64) -> String {
    format_inches_with(value_in, DEFAULT_DENOMINATOR)
}

pub fn format_mm_in_with(value_mm: f64, denominator: i64) -> String {
    format_inches_with(mm_to_in(value_mm), denominator)
}

pub fn format_mm_in(value_mm: f64) -> String {
    format_mm_in_with(value_mm, DEFAULT_DENOMINATOR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_number_has_no_fraction() {
        assert_eq!(format_inches(96.0), "96\"");
    }

    #[test]
    fn sub_inch_drops_leading_zero() {
        assert_eq!(format_inches(0.75), "3/4\"");
    }

    #[test]
    fn mixed_fraction_reduces() {
        assert_eq!(format_inches(35.25), "35 1/4\"");
        // 28.875 = 28 + 28/32, reduces to 7/8
        assert_eq!(format_inches(28.875), "28 7/8\"");
    }

    #[test]
    fn rounds_to_nearest_thirty_second() {
        // 30.0 + a hair under 1/32 (0.03125) should round back to 30"
        assert_eq!(format_inches(30.0 + 0.01), "30\"");
    }

    #[test]
    fn mm_conversion_matches_known_value() {
        // 787.4mm is exactly 31in
        assert_eq!(format_mm_in(787.4), "31\"");
    }

    #[test]
    fn custom_denominator_is_respected() {
        assert_eq!(format_inches_with(0.5, 2), "1/2\"");
    }
}
