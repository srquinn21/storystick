//! Low-level STEP file tokenizing: treat the file as a graph of
//! `#id = ENTITY(args...)` records, addressable by id. No domain knowledge
//! of solids, assemblies, or parts lives here -- just the text-to-graph
//! layer.

use regex::Regex;
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

fn entity_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^#(\d+)\s*=\s*(.+);\s*$").unwrap())
}

fn type_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^([A-Z_0-9]+)\((.*)\)$").unwrap())
}

fn refs_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"#(\d+)").unwrap())
}

/// Split top-level comma-separated args, respecting nesting and quotes.
pub fn split_args(s: &str) -> Vec<String> {
    let mut depth = 0i32;
    let mut in_str = false;
    let mut cur = String::new();
    let mut args = Vec::new();
    for ch in s.chars() {
        if ch == '\'' {
            in_str = !in_str;
            cur.push(ch);
        } else if in_str {
            cur.push(ch);
        } else if ch == '(' {
            depth += 1;
            cur.push(ch);
        } else if ch == ')' {
            depth -= 1;
            cur.push(ch);
        } else if ch == ',' && depth == 0 {
            args.push(std::mem::take(&mut cur));
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        args.push(cur);
    }
    args
}

pub fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('\'') && s.ends_with('\'') {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

pub fn refs(s: &str) -> Vec<i64> {
    refs_re()
        .captures_iter(s)
        .filter_map(|c| c[1].parse::<i64>().ok())
        .collect()
}

/// Reads the file as UTF-8, replacing invalid byte sequences rather than
/// erroring -- matches Python's `errors="replace"`, since a STEP export
/// occasionally carries a stray non-UTF-8 byte in a name field that
/// shouldn't take down the whole parse.
pub fn parse_entities(path: &Path) -> std::io::Result<HashMap<i64, String>> {
    let bytes = std::fs::read(path)?;
    let content = String::from_utf8_lossy(&bytes);
    let mut entities = HashMap::new();
    for line in content.lines() {
        if let Some(caps) = entity_re().captures(line.trim()) {
            if let Ok(id) = caps[1].parse::<i64>() {
                entities.insert(id, caps[2].to_string());
            }
        }
    }
    Ok(entities)
}

/// (type, args) for entity `id`, or (None, []) if the id doesn't exist or
/// doesn't match the ENTITY(args...) shape. Callers always check
/// `typ.is_none()` before touching args, so the args value in that case
/// is never actually read -- an empty vec is as good as anything else.
pub fn typed(entities: &HashMap<i64, String>, id: i64) -> (Option<String>, Vec<String>) {
    let raw = match entities.get(&id) {
        Some(r) => r,
        None => return (None, Vec::new()),
    };
    match type_re().captures(raw.as_str()) {
        Some(caps) => (Some(caps[1].to_string()), split_args(&caps[2])),
        None => (None, Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_args_respects_nesting_and_quotes() {
        let args = split_args("'a,b',(#1,#2),3");
        assert_eq!(args, vec!["'a,b'", "(#1,#2)", "3"]);
    }

    #[test]
    fn unquote_strips_single_quotes() {
        assert_eq!(unquote("'Bench'"), "Bench");
        assert_eq!(unquote("42"), "42");
    }

    #[test]
    fn refs_extracts_all_hash_ids() {
        assert_eq!(refs("(#12,#34)"), vec![12, 34]);
        assert_eq!(refs("no refs here"), Vec::<i64>::new());
    }

    #[test]
    fn typed_splits_type_and_args() {
        let mut entities = HashMap::new();
        entities.insert(1, "PRODUCT('Bench','','',(#2))".to_string());
        let (typ, args) = typed(&entities, 1);
        assert_eq!(typ.as_deref(), Some("PRODUCT"));
        assert_eq!(args[0], "'Bench'");
    }

    #[test]
    fn typed_missing_id_returns_none() {
        let entities = HashMap::new();
        let (typ, args) = typed(&entities, 999);
        assert_eq!(typ, None);
        assert!(args.is_empty());
    }
}
