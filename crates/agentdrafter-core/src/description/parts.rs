//! A list of parts: a template's text or mtext contents, or an insert's
//! attribute value, written as a list of plain text and values from the
//! row, joined in order, such as
//! `contents: ['PANEL ', {row: PANEL}, ' SLOT ', {row: SLOT}]`. Zach
//! decided on 2026-11-19 (#102) that a value from the row may stand inside
//! other text this way, and that nothing inside a string is read for
//! placeholders: a brace or a backslash in one is text, shown as written.
//!
//! An edit of the joined text is matched against the fixed parts, to find
//! the value of each row value it holds ([`split`]).

use super::strict_yaml::{index_path, Kind, Node};
use super::template::{row_value_name, ROW_KEY};
use super::{bad_value, wrong_type};
use crate::error::Error;

/// One part of a list of parts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    /// Plain text, as written.
    Text(String),
    /// A value from the row, `{row: NAME}`, by its name.
    Row(String),
}

/// The parts of `node`, at `path`, if it is a list: each item plain text or
/// a value from the row. A list that holds anything else, or no value from
/// the row, is an error, since a text without one is written as one string.
/// None when `node` is not a list. When `one_line`, for a text's contents
/// or an attribute's value, which are one line and hold no control
/// character, a fixed part that holds a line break or another control
/// character is an error at that part, since no row can make the joined
/// text build.
pub(super) fn read_parts(
    node: &Node,
    path: &str,
    one_line: bool,
) -> Result<Option<Vec<Part>>, Error> {
    let Kind::Sequence(items) = &node.kind else {
        return Ok(None);
    };
    let mut parts = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let item_path = index_path(path, i);
        match &item.kind {
            Kind::Scalar(text) if one_line && text.chars().any(char::is_control) => {
                let why = match text.chars().find(|c| c.is_control()) {
                    Some('\n' | '\r') => "this part holds a line break, and the text it is part \
                                          of is one line; write the part on one line, or make \
                                          the text an mtext"
                        .to_owned(),
                    Some(c) => format!(
                        "this part holds the control character `{}`, which the text it is part \
                         of cannot hold",
                        c.escape_default()
                    ),
                    None => unreachable!("found above"),
                };
                return Err(bad_value(item, &item_path, why));
            }
            Kind::Scalar(text) if !item.is_bare_null() => parts.push(Part::Text(text.clone())),
            Kind::Mapping(_) => match row_value_name(item, &item_path)? {
                Some(name) => parts.push(Part::Row(name)),
                None => unreachable!("a mapping is a value from the row or an error"),
            },
            _ => {
                return Err(wrong_type(
                    item,
                    &item_path,
                    &format!(
                        "a part of the text: plain text, or a value from the row, \
                         {{{ROW_KEY}: NAME}}"
                    ),
                ))
            }
        }
    }
    if !parts.iter().any(|p| matches!(p, Part::Row(_))) {
        return Err(bad_value(
            node,
            path,
            format!(
                "a list of parts joins plain text and values from the row, {{{ROW_KEY}: NAME}}, \
                 and this one holds no value from the row; write the text as one string"
            ),
        ));
    }
    Ok(Some(parts))
}

/// The parts of `node` as [`read_parts`] reads them, or none when it is
/// not a list of parts.
pub(super) fn parts_of(node: &Node) -> Option<Vec<Part>> {
    read_parts(node, "", false).ok().flatten()
}

/// The names of the row's values the parts hold, each once, in the order
/// they first come.
pub(crate) fn names(parts: &[Part]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for part in parts {
        if let Part::Row(name) = part {
            if !names.contains(name) {
                names.push(name.clone());
            }
        }
    }
    names
}

/// The parts as a template writes them, on one line, such as
/// `['PANEL ', {row: PANEL}, ' SLOT ', {row: SLOT}]`, to name the pattern
/// an edit is matched against.
pub(crate) fn pattern(parts: &[Part]) -> String {
    let each: Vec<String> = parts
        .iter()
        .map(|part| match part {
            Part::Text(text) => format!("'{}'", text.replace('\'', "''").replace('\n', "\\n")),
            Part::Row(name) => format!("{{{ROW_KEY}: {name}}}"),
        })
        .collect();
    format!("[{}]", each.join(", "))
}

/// The text the parts join to, with each value from the row as `value`
/// gives it; or the names it gives none for, each once, in order.
pub(crate) fn join(
    parts: &[Part],
    value: impl Fn(&str) -> Option<String>,
) -> Result<String, Vec<String>> {
    let mut text = String::new();
    let mut missing: Vec<String> = Vec::new();
    for part in parts {
        match part {
            Part::Text(t) => text.push_str(t),
            Part::Row(name) => match value(name) {
                Some(v) => text.push_str(&v),
                None if !missing.contains(name) => missing.push(name.clone()),
                None => {}
            },
        }
    }
    if missing.is_empty() {
        Ok(text)
    } else {
        Err(missing)
    }
}

/// The parts of an mtext's contents as they show once joined: CR LF is a
/// line break, as LF is, and the line breaks the parts end with are
/// dropped, as [`super::text::shown_mtext`] drops them from the contents,
/// so an edit of the text as it shows matches them.
pub(crate) fn shown_mtext_parts(parts: &[Part]) -> Vec<Part> {
    // Fixed parts side by side are one fixed text, as the reader reads the
    // joined text, so a CR LF across two of them is one line break.
    let mut joined: Vec<Part> = Vec::new();
    for part in parts {
        match (joined.last_mut(), part) {
            (Some(Part::Text(before)), Part::Text(text)) => before.push_str(text),
            _ => joined.push(part.clone()),
        }
    }
    let mut shown: Vec<Part> = joined
        .into_iter()
        .map(|part| match part {
            Part::Text(text) => Part::Text(text.replace("\r\n", "\n")),
            row => row,
        })
        .collect();
    for part in shown.iter_mut().rev() {
        let Part::Text(text) = part else {
            break;
        };
        let kept = text.trim_end_matches(['\n', '\r']).len();
        let emptied = kept == 0;
        text.truncate(kept);
        if !emptied {
            break;
        }
    }
    shown
}

/// Why a text could not be matched to its parts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Unsplit {
    /// The text's fixed parts are not the pattern's.
    Differs,
    /// The text can be split more than one way: two of the splits.
    Ambiguous(Vec<(String, String)>, Vec<(String, String)>),
    /// The text could not be told to split one way only within
    /// [`MOST_STEPS`] steps, as with many row values side by side.
    Unclear,
}

/// The most steps a match takes before it gives up on telling whether a
/// text splits one way only.
const MOST_STEPS: usize = 100_000;

/// Each row value of `parts`, by name, each once in the order it first
/// comes, with its text in `text`: the one way the text splits around the
/// fixed parts. A value the parts hold twice takes the same text in both.
/// A text whose fixed parts differ, or that splits more than one way, such
/// as one that puts two row values side by side with no fixed text between,
/// is refused. The fixed text is checked first, and each row value ends
/// only where the fixed text after it begins, so a text that cannot match
/// is refused at once.
pub(crate) fn split(parts: &[Part], text: &str) -> Result<Vec<(String, String)>, Unsplit> {
    if !fixed_text_fits(parts, text) {
        return Err(Unsplit::Differs);
    }
    let mut found: Vec<Vec<(String, String)>> = Vec::new();
    let mut taken: Vec<(String, String)> = Vec::new();
    let mut steps = 0;
    matches(parts, text, &mut taken, &mut found, &mut steps);
    let order = names(parts);
    let ordered = |m: Vec<(String, String)>| -> Vec<(String, String)> {
        order
            .iter()
            .filter_map(|n| m.iter().find(|(name, _)| name == n).cloned())
            .collect()
    };
    match found.len() {
        _ if found.len() < 2 && steps > MOST_STEPS => Err(Unsplit::Unclear),
        0 => Err(Unsplit::Differs),
        1 => Ok(ordered(found.remove(0))),
        _ => {
            let second = ordered(found.remove(1));
            Err(Unsplit::Ambiguous(ordered(found.remove(0)), second))
        }
    }
}

/// Whether `text` holds the fixed text of `parts` where they put it: it
/// starts with the first part when that is fixed text, ends with the last
/// when that is, and holds each other fixed part, in order, between.
fn fixed_text_fits(parts: &[Part], text: &str) -> bool {
    let mut parts = parts;
    let mut rest = text;
    if let Some((Part::Text(first), more)) = parts.split_first() {
        let Some(after) = rest.strip_prefix(first.as_str()) else {
            return false;
        };
        rest = after;
        parts = more;
    }
    if let Some((Part::Text(last), more)) = parts.split_last() {
        let Some(before) = rest.strip_suffix(last.as_str()) else {
            return false;
        };
        rest = before;
        parts = more;
    }
    for part in parts {
        if let Part::Text(fixed) = part {
            match rest.find(fixed.as_str()) {
                Some(at) => rest = &rest[at + fixed.len()..],
                None => return false,
            }
        }
    }
    true
}

/// Adds to `found` each way, up to two, that `text` matches `parts` with
/// the values `taken` so far, counting its steps in `steps` and giving up
/// past [`MOST_STEPS`].
fn matches(
    parts: &[Part],
    text: &str,
    taken: &mut Vec<(String, String)>,
    found: &mut Vec<Vec<(String, String)>>,
    steps: &mut usize,
) {
    *steps += 1;
    if found.len() >= 2 || *steps > MOST_STEPS {
        return;
    }
    let Some((first, rest)) = parts.split_first() else {
        if text.is_empty() && !found.contains(taken) {
            found.push(taken.clone());
        }
        return;
    };
    match first {
        Part::Text(fixed) => {
            if let Some(after) = text.strip_prefix(fixed.as_str()) {
                matches(rest, after, taken, found, steps);
            }
        }
        Part::Row(name) => {
            // A value already taken must come again as it was.
            if let Some(value) = taken
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
            {
                if let Some(after) = text.strip_prefix(value.as_str()) {
                    matches(rest, after, taken, found, steps);
                }
                return;
            }
            // The value ends where the fixed text after it begins, or at
            // the end when it is the last part; beside another value, at
            // any character.
            let ends: Vec<usize> = match rest.first() {
                None => vec![text.len()],
                Some(Part::Text(next)) => text
                    .char_indices()
                    .map(|(i, _)| i)
                    .chain(std::iter::once(text.len()))
                    .filter(|&i| text[i..].starts_with(next.as_str()))
                    .collect(),
                Some(Part::Row(_)) => text
                    .char_indices()
                    .map(|(i, _)| i)
                    .chain(std::iter::once(text.len()))
                    .collect(),
            };
            for end in ends {
                if !fixed_text_fits(rest, &text[end..]) {
                    continue;
                }
                taken.push((name.clone(), text[..end].to_owned()));
                matches(rest, &text[end..], taken, found, steps);
                taken.pop();
                if found.len() >= 2 || *steps > MOST_STEPS {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str) -> Part {
        Part::Row(name.to_owned())
    }
    fn text(t: &str) -> Part {
        Part::Text(t.to_owned())
    }
    fn pairs(p: &[(&str, &str)]) -> Vec<(String, String)> {
        p.iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect()
    }

    #[test]
    fn a_text_splits_around_its_fixed_parts_one_way_only() {
        let parts = [text("PANEL "), row("PANEL"), text(" SLOT "), row("SLOT")];
        assert_eq!(
            split(&parts, "PANEL P2 SLOT 4"),
            Ok(pairs(&[("PANEL", "P2"), ("SLOT", "4")]))
        );
        assert_eq!(split(&parts, "BOARD P2 SLOT 4"), Err(Unsplit::Differs));
        // A value that holds a fixed part's text can split two ways.
        assert!(matches!(
            split(&parts, "PANEL A SLOT B SLOT C"),
            Err(Unsplit::Ambiguous(..))
        ));
        // Two values side by side split as many ways as the text is long.
        let side = [row("A"), row("B")];
        assert!(matches!(split(&side, "XY"), Err(Unsplit::Ambiguous(..))));
        // A value the parts hold twice takes the same text in both.
        let twice = [row("A"), text("-"), row("A")];
        assert_eq!(split(&twice, "7-7"), Ok(pairs(&[("A", "7")])));
        assert_eq!(split(&twice, "7-8"), Err(Unsplit::Differs));
        assert_eq!(
            pattern(&parts),
            "['PANEL ', {row: PANEL}, ' SLOT ', {row: SLOT}]"
        );
    }

    #[test]
    fn a_text_that_cannot_match_many_values_side_by_side_is_refused_at_once() {
        // Eight values side by side, then fixed text the edit lacks: the
        // fixed text is checked first.
        let mut side: Vec<Part> = ["A", "B", "C", "D", "E", "F", "G", "H"]
            .iter()
            .map(|n| row(n))
            .collect();
        side.push(text("!"));
        assert_eq!(split(&side, &"x".repeat(40)), Err(Unsplit::Differs));
        // With it, the first two of its many splits are found.
        assert!(matches!(
            split(&side, &format!("{}!", "x".repeat(40))),
            Err(Unsplit::Ambiguous(..))
        ));
        // A value that must come again as it was, which no split of the
        // values before it allows, gives up past the most steps.
        side.push(row("A"));
        assert_eq!(
            split(&side, &format!("{}!y", "x".repeat(40))),
            Err(Unsplit::Unclear)
        );
    }
}
