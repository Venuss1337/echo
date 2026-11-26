//! Text normalization for untrusted tag values and user search input.
//!
//! Tags come from arbitrary files, so every string that reaches SQLite or the
//! renderer passes through here. Cleaning is lossy on purpose: control and
//! bidirectional override characters are removed because they can reorder or
//! hide neighbouring text in a list, and length is capped so one malformed tag
//! cannot make a row, an index entry or an IPC page unbounded.

/// Longest cleaned display string, in Unicode scalar values.
pub(crate) const MAX_TEXT: usize = 512;
/// Longest user search text, in Unicode scalar values.
pub(crate) const MAX_QUERY: usize = 200;
const MAX_TERMS: usize = 8;
const MAX_TERM: usize = 64;

fn is_hostile(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}'
        )
}

/// Removes hostile characters, collapses whitespace runs and truncates.
pub(crate) fn clean(raw: &str) -> String {
    let mut output = String::new();
    let mut length = 0;
    let mut pending_space = false;
    for character in raw.chars() {
        // Tabs and line breaks separate words. Other controls vanish.
        if character.is_whitespace() {
            pending_space = !output.is_empty();
            continue;
        }
        if is_hostile(character) {
            continue;
        }
        let needed = 1 + usize::from(pending_space);
        if length + needed > MAX_TEXT {
            break;
        }
        if pending_space {
            output.push(' ');
            pending_space = false;
        }
        output.push(character);
        length += needed;
    }
    output
}

/// Identity and sort key for artists, albums and titles. It is case-insensitive
/// only: "The Cure" and "Cure" stay distinct artists.
pub(crate) fn key(clean: &str) -> String {
    clean.to_lowercase()
}

/// Builds a prefix-matching FTS5 expression from user text, or `None` when the
/// text holds nothing searchable. Only alphanumeric runs survive and each is
/// quoted, so no FTS operator, column filter or quote can be injected.
pub(crate) fn fts_query(input: &str) -> Option<String> {
    let mut terms = Vec::new();
    let mut current = String::new();
    for character in input.chars().take(MAX_QUERY) {
        if character.is_alphanumeric() {
            current.push(character);
        } else if !current.is_empty() {
            terms.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        terms.push(current);
    }
    let expression = terms
        .into_iter()
        .take(MAX_TERMS)
        .map(|term| format!("\"{}\"*", term.chars().take(MAX_TERM).collect::<String>()))
        .collect::<Vec<_>>()
        .join(" ");
    (!expression.is_empty()).then_some(expression)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_removes_controls_and_bidi_overrides_and_collapses_space() {
        assert_eq!(clean("  A\u{202e}b\u{0}c \t\n d  "), "Abc d");
        assert_eq!(clean("\u{2066}\u{2069}"), "");
        assert_eq!(clean("日本\u{200b}語"), "日本語");
    }

    #[test]
    fn clean_bounds_a_million_character_tag() {
        let huge = "ab ".repeat(400_000);
        let cleaned = clean(&huge);
        assert!(cleaned.chars().count() <= MAX_TEXT);
        assert!(cleaned.starts_with("ab ab"));
        assert!(!cleaned.ends_with(' '));
    }

    #[test]
    fn keys_are_case_insensitive_but_do_not_merge_distinct_names() {
        assert_eq!(key("Björk"), key("BJÖRK"));
        assert_ne!(key("The Cure"), key("Cure"));
    }

    #[test]
    fn search_text_cannot_inject_fts_syntax() {
        assert_eq!(fts_query("  "), None);
        assert_eq!(fts_query("\"*^:-()"), None);
        assert_eq!(
            fts_query("Dark side"),
            Some("\"Dark\"* \"side\"*".to_owned())
        );
        assert_eq!(
            fts_query("title:evil OR NOT a\"b"),
            Some("\"title\"* \"evil\"* \"OR\"* \"NOT\"* \"a\"* \"b\"*".to_owned())
        );
        let many = (0..40)
            .map(|n| format!("w{n}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(fts_query(&many).unwrap().matches('*').count(), MAX_TERMS);
    }
}
