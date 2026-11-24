//! Text matching and replacement for `replace_text` (`FIND`).
//!
//! Supports three matching modes:
//! - `exact`: literal substring replacement (never interprets `$` in the
//!   replacement as a capture group).
//! - `whole_word`: literal match bounded on both sides by non-word characters
//!   (neither preceded nor followed by an alphanumeric character or `_`), so
//!   `P1` matches `PANEL P1` and `P1-A` without matching `P10` or `AP1`, and
//!   tags with punctuation such as `24VDC+` still match cleanly.
//! - `pattern`: Rust `regex` pattern with standard capture-group expansion
//!   (`$1`, `${name}`, `$$`) and a bounded compiled automaton size.

use regex::{NoExpand, Regex, RegexBuilder};

use crate::error::{Error, ErrorCode};

/// Maximum compiled regex size (1 MiB) so a pathological pattern cannot exhaust
/// memory.
const MAX_REGEX_BYTES: usize = 1 << 20;

/// How `replace_text` matches its `find` string.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum TextMatchKind {
    #[default]
    Exact,
    WholeWord,
    Pattern,
}

impl TextMatchKind {
    pub(crate) const ALL: [&'static str; 3] = ["exact", "whole_word", "pattern"];

    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s {
            "exact" => Some(Self::Exact),
            "whole_word" => Some(Self::WholeWord),
            "pattern" => Some(Self::Pattern),
            _ => None,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::WholeWord => "whole_word",
            Self::Pattern => "pattern",
        }
    }
}

/// Which text-bearing places `replace_text` searches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextScope {
    Text,
    MText,
    Attribute,
}

impl TextScope {
    pub(crate) const ALL: [&'static str; 3] = ["text", "mtext", "attribute"];

    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s {
            "text" => Some(Self::Text),
            "mtext" => Some(Self::MText),
            "attribute" => Some(Self::Attribute),
            _ => None,
        }
    }
}

/// Compiled matcher for a `replace_text` operation.
#[derive(Clone, Debug)]
pub(crate) struct TextMatcher {
    kind: TextMatchKind,
    regex: Regex,
}

impl TextMatcher {
    pub(crate) fn compile(
        find: &str,
        kind: TextMatchKind,
        case_sensitive: bool,
    ) -> Result<Self, Error> {
        if find.is_empty() {
            return Err(Error::new(
                ErrorCode::BadValue,
                "`find` cannot be empty".to_owned(),
            ));
        }
        let pattern = match kind {
            TextMatchKind::Exact | TextMatchKind::WholeWord => regex::escape(find),
            TextMatchKind::Pattern => find.to_owned(),
        };
        let regex = RegexBuilder::new(&pattern)
            .case_insensitive(!case_sensitive)
            .size_limit(MAX_REGEX_BYTES)
            .build()
            .map_err(|e| {
                Error::new(
                    ErrorCode::BadValue,
                    format!("invalid `find` pattern `{find}`: {e}"),
                )
            })?;
        Ok(Self { kind, regex })
    }

    /// Replaces every match in `input` with `replacement`. Returns
    /// `Some((after, count))` when at least one match was found and `after`
    /// differs from `input`; `None` when nothing changed.
    pub(crate) fn replace(&self, input: &str, replacement: &str) -> Option<(String, usize)> {
        match self.kind {
            TextMatchKind::Exact => {
                let mut count = 0usize;
                let mut out = String::with_capacity(input.len());
                let mut last = 0usize;
                for m in self.regex.find_iter(input) {
                    out.push_str(&input[last..m.start()]);
                    out.push_str(replacement);
                    last = m.end();
                    count += 1;
                }
                if count == 0 {
                    return None;
                }
                out.push_str(&input[last..]);
                (out != input).then_some((out, count))
            }
            TextMatchKind::WholeWord => {
                let mut count = 0usize;
                let mut out = String::with_capacity(input.len());
                let mut last = 0usize;
                for m in self.regex.find_iter(input) {
                    if !is_whole_word_match(input, m.start(), m.end()) {
                        continue;
                    }
                    out.push_str(&input[last..m.start()]);
                    out.push_str(replacement);
                    last = m.end();
                    count += 1;
                }
                if count == 0 {
                    return None;
                }
                out.push_str(&input[last..]);
                (out != input).then_some((out, count))
            }
            TextMatchKind::Pattern => {
                let mut count = 0usize;
                let mut out = String::with_capacity(input.len());
                let mut last = 0usize;
                for caps in self.regex.captures_iter(input) {
                    let m = caps.get(0).expect("capture group 0 always exists");
                    out.push_str(&input[last..m.start()]);
                    caps.expand(replacement, &mut out);
                    last = m.end();
                    count += 1;
                }
                if count == 0 {
                    return None;
                }
                out.push_str(&input[last..]);
                // Sanity guard: keep unused import away if we also test NoExpand.
                let _ = NoExpand(replacement);
                (out != input).then_some((out, count))
            }
        }
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_whole_word_match(input: &str, start: usize, end: usize) -> bool {
    let prev_ok = input[..start]
        .chars()
        .next_back()
        .is_none_or(|c| !is_word_char(c));
    let next_ok = input[end..].chars().next().is_none_or(|c| !is_word_char(c));
    prev_ok && next_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_replaces_literally_without_expanding_dollars() {
        let m = TextMatcher::compile("JB-101", TextMatchKind::Exact, true).unwrap();
        assert_eq!(
            m.replace("TO JB-101 / FROM JB-101", "$1-201"),
            Some(("TO $1-201 / FROM $1-201".to_owned(), 2))
        );
        assert_eq!(m.replace("TO jb-101", "JB-201"), None);
    }

    #[test]
    fn exact_case_insensitive_replaces_any_case() {
        let m = TextMatcher::compile("jb-101", TextMatchKind::Exact, false).unwrap();
        assert_eq!(
            m.replace("Panel JB-101 and Jb-101", "JB-201"),
            Some(("Panel JB-201 and JB-201".to_owned(), 2))
        );
    }

    #[test]
    fn whole_word_respects_boundaries_and_punctuation_tags() {
        let m = TextMatcher::compile("P1", TextMatchKind::WholeWord, true).unwrap();
        assert_eq!(
            m.replace("PANEL P1, P10, AP1, P1-A, (P1)", "P2"),
            Some(("PANEL P2, P10, AP1, P2-A, (P2)".to_owned(), 3))
        );
        let punct = TextMatcher::compile("24VDC+", TextMatchKind::WholeWord, true).unwrap();
        assert_eq!(
            punct.replace("24VDC+ SUPPLY NOT 124VDC+", "24VDC_POS"),
            Some(("24VDC_POS SUPPLY NOT 124VDC+".to_owned(), 1))
        );
    }

    #[test]
    fn pattern_supports_capture_groups_and_rejects_bad_regex() {
        let m = TextMatcher::compile(r"FT-(\d{3})", TextMatchKind::Pattern, true).unwrap();
        assert_eq!(
            m.replace("LOOP FT-101 TO FT-102", "FIT-$1"),
            Some(("LOOP FIT-101 TO FIT-102".to_owned(), 2))
        );
        let err = TextMatcher::compile(r"FT-(", TextMatchKind::Pattern, true).unwrap_err();
        assert_eq!(err.code, ErrorCode::BadValue);
    }

    #[test]
    fn no_op_replacement_returns_none() {
        let m = TextMatcher::compile("P-101", TextMatchKind::Exact, true).unwrap();
        assert_eq!(m.replace("P-101", "P-101"), None);
    }
}
