//! Bracket-token tags embedded in a part's Shapr3D name (e.g. `[Panel]`,
//! `[Backer]`) -- a naming convention already in use for material
//! auto-fill, and (per docs/poc.md) planned to double as the input to
//! bulk-edit-by-tag and construction-stage classification for PDF section
//! grouping. All three features read the same tokens off the same paths,
//! so this is the one place that knows how to find them.

use regex::Regex;
use std::sync::OnceLock;

fn tag_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[[^\[\]]+\]").unwrap())
}

/// Every bracket-delimited tag appearing anywhere in `path` (e.g.
/// `"Bench / Carcasses / Carcass A / [Panel] Bottom"` -> `["[Panel]"]`),
/// left to right, brackets included -- matching how a stock-catalog
/// material names them (`match: ["[Panel]"]`), so a tag extracted here can
/// be compared against a catalog entry's `match:` list with no
/// reformatting either direction. Duplicate tags within one path are kept,
/// not deduped -- a caller building a distinct set (e.g. bulk-edit's tag
/// listing) does that itself, across many paths at once.
pub fn extract_tags(path: &str) -> Vec<String> {
    tag_re().find_iter(path).map(|m| m.as_str().to_string()).collect()
}

/// Classifies `path` by the first rule (in order) whose `keyword` appears
/// anywhere in it, e.g. `[("Carcass", "Carcasses"), ("Door", "Doors")]`
/// turns `"Living Room Built-In / Left Carcass / Bottom"` into
/// `Some("Carcasses")`. This is the same "keyword found somewhere in a
/// path" shape `extract_tags` uses for bracket tokens -- construction-stage
/// classification for PDF section grouping (see docs/poc.md) is the other
/// planned consumer, picking a label off folder names the same way a
/// material tag gets picked off a body name, just without requiring
/// brackets. Returns `None` when no rule matches, for the caller to fall
/// back on (e.g. an "Unsectioned" bucket) -- this never guesses.
pub fn classify_by_keyword(path: &str, rules: &[(&str, &str)]) -> Option<String> {
    rules.iter().find(|(keyword, _)| path.contains(keyword)).map(|(_, label)| label.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_tags_finds_a_single_bracket_token() {
        assert_eq!(extract_tags("Bench / Carcasses / Carcass A / [Panel] Bottom"), vec!["[Panel]"]);
    }

    #[test]
    fn extract_tags_finds_multiple_tokens_in_order() {
        assert_eq!(extract_tags("[Carcass] Left / [Panel] Bottom"), vec!["[Carcass]", "[Panel]"]);
    }

    #[test]
    fn extract_tags_returns_empty_when_no_brackets_present() {
        assert!(extract_tags("Bench / Carcasses / Carcass A / Bottom").is_empty());
    }

    #[test]
    fn extract_tags_keeps_duplicates_within_one_path() {
        assert_eq!(extract_tags("[Panel] Left / [Panel] Right"), vec!["[Panel]", "[Panel]"]);
    }

    #[test]
    fn extract_tags_ignores_unbalanced_brackets() {
        assert!(extract_tags("Bench / Carcass [A").is_empty());
    }

    #[test]
    fn classify_by_keyword_returns_the_first_matching_rule() {
        let rules = [("Carcass", "Carcasses"), ("Door", "Doors")];
        assert_eq!(classify_by_keyword("Bench / Left Carcass / Bottom", &rules), Some("Carcasses".to_string()));
        assert_eq!(classify_by_keyword("Bench / Left Door / Panel", &rules), Some("Doors".to_string()));
    }

    #[test]
    fn classify_by_keyword_returns_none_when_nothing_matches() {
        let rules = [("Carcass", "Carcasses"), ("Door", "Doors")];
        assert_eq!(classify_by_keyword("Bench / Face Frame / Rail", &rules), None);
    }

    #[test]
    fn classify_by_keyword_rule_order_decides_an_ambiguous_path() {
        // A path matching more than one rule's keyword picks whichever
        // rule was listed first -- never both, never the "better" match.
        let rules = [("Face Frame", "Face Frames"), ("Frame", "Frames")];
        assert_eq!(classify_by_keyword("Bench / Face Frame Rail", &rules), Some("Face Frames".to_string()));
    }
}
