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
/// None when `node` is not a list.
pub(super) fn read_parts(node: &Node, path: &str) -> Result<Option<Vec<Part>>, Error> {
    let Kind::Sequence(items) = &node.kind else {
        return Ok(None);
    };
    let mut parts = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let item_path = index_path(path, i);
        match &item.kind {
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
    read_parts(node, "").ok().flatten()
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

/// Why a text could not be matched to its parts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Unsplit {
    /// The text's fixed parts are not the pattern's.
    Differs,
    /// The text can be split more than one way: two of the splits.
    Ambiguous(Vec<(String, String)>, Vec<(String, String)>),
}

/// Each row value of `parts`, by name, each once in the order it first
/// comes, with its text in `text`: the one way the text splits around the
/// fixed parts. A value the parts hold twice takes the same text in both.
/// A text whose fixed parts differ, or that splits more than one way, such
/// as one that puts two row values side by side with no fixed text between,
/// is refused.
pub(crate) fn split(parts: &[Part], text: &str) -> Result<Vec<(String, String)>, Unsplit> {
    let mut found: Vec<Vec<(String, String)>> = Vec::new();
    let mut taken: Vec<(String, String)> = Vec::new();
    matches(parts, text, &mut taken, &mut found);
    let order = names(parts);
    let ordered = |m: Vec<(String, String)>| -> Vec<(String, String)> {
        order
            .iter()
            .filter_map(|n| m.iter().find(|(name, _)| name == n).cloned())
            .collect()
    };
    match found.len() {
        0 => Err(Unsplit::Differs),
        1 => Ok(ordered(found.remove(0))),
        _ => {
            let second = ordered(found.remove(1));
            Err(Unsplit::Ambiguous(ordered(found.remove(0)), second))
        }
    }
}

/// Adds to `found` each way, up to two, that `text` matches `parts` with
/// the values `taken` so far.
fn matches(
    parts: &[Part],
    text: &str,
    taken: &mut Vec<(String, String)>,
    found: &mut Vec<Vec<(String, String)>>,
) {
    if found.len() >= 2 {
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
                matches(rest, after, taken, found);
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
                    matches(rest, after, taken, found);
                }
                return;
            }
            let ends = text
                .char_indices()
                .map(|(i, _)| i)
                .chain(std::iter::once(text.len()));
            for end in ends {
                taken.push((name.clone(), text[..end].to_owned()));
                matches(rest, &text[end..], taken, found);
                taken.pop();
                if found.len() >= 2 {
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
}
