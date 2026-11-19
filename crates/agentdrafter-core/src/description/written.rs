//! Where a drawing set's sheet's values are written: a text's contents and
//! each attribute value of an insert, by the object itself, by the sheet's
//! row, as a value its template takes from the row, whole or joined from a
//! list of parts, or nowhere, so the attribute shows its definition's
//! default. Quick properties say so before a value is edited, since a
//! change set writes an edit where the value is (`docs/change-sets.md`):
//! the template, every sheet built from it following; the row's values, on
//! this sheet alone, in every place each fills; or, for an attribute the
//! insert leaves unset, the insert itself.

use std::collections::BTreeMap;

use super::blocks::tag_in_capitals;
use super::ids::ObjectRecord;
use super::parts::{self, parts_of};
use super::set::Row;
use super::strict_yaml::{self, Kind, Node};
use super::template::{places_in, row_value_name};
use crate::model::Block;

/// Where one of a sheet's values is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    /// By the object itself, in its template or in its row.
    Own,
    /// By the sheet's row, as its value of this name, which the template
    /// takes in its place.
    Row(String),
    /// By the sheet's row, as the values of these names, in the order they
    /// first come, which the template joins with plain text in a list of
    /// parts (#102).
    Parts(Vec<String>),
    /// Nowhere: an attribute the insert gives no value for, or a bare null,
    /// which shows its definition's default.
    Unset,
}

/// The value an insert's record gives for one tag.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Given {
    One(Source),
    /// A list: for a tag its block defines more than once, each item, in
    /// order; for a tag it defines once, a list of parts, whose values from
    /// the row are `parts`, when it is one.
    List {
        items: Vec<Source>,
        parts: Option<Vec<String>>,
    },
}

/// Where an object's record writes the values a template may take from a
/// row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ObjectSources {
    /// A text's or an mtext's contents.
    pub contents: Option<Source>,
    /// Each tag an insert's record gives a value for, in capitals.
    attributes: Vec<(String, Given)>,
}

impl ObjectSources {
    /// Where the insert's value for `tag` is written: its `occurrence`th,
    /// counted from 0, for a tag its block defines more than once. Without
    /// `occurrence` the block defines the tag once, and a list given for it
    /// is a list of parts.
    pub(crate) fn attribute(&self, tag: &str, occurrence: Option<usize>) -> Source {
        let at = occurrence.unwrap_or(0);
        match self.attributes.iter().find(|(t, _)| t == tag) {
            None => Source::Unset,
            // A single value of a tag defined more than once is its first.
            Some((_, Given::One(source))) if at == 0 => source.clone(),
            Some((_, Given::One(_))) => Source::Unset,
            Some((
                _,
                Given::List {
                    parts: Some(names), ..
                },
            )) if occurrence.is_none() => Source::Parts(names.clone()),
            Some((_, Given::List { items, .. })) => items.get(at).cloned().unwrap_or(Source::Unset),
        }
    }
}

/// Where each value of a set's sheet is written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SheetSources {
    /// For each of the sheet's objects, in the order of its records.
    pub objects: Vec<ObjectSources>,
    /// How many places on the sheet each of the row's values fills: every
    /// place its template takes it, whole or in a list of parts, each of
    /// which an edit to it changes.
    pub fills: BTreeMap<String, usize>,
}

/// Where each value of the sheet `row` names is written, from its
/// template's text and its row's own objects, for its objects' `records`,
/// in their order; `blocks` are the blocks the sheet inserts, which say
/// whether a list given for an attribute is a list of parts. A template
/// that cannot be read says nothing: its objects' values are taken as
/// their own.
pub(crate) fn sheet_sources(
    template: &str,
    row: &Row,
    records: &[ObjectRecord],
    blocks: &[Block],
) -> SheetSources {
    let template = template.strip_prefix('\u{FEFF}').unwrap_or(template);
    let root = strict_yaml::read(template).ok().flatten();
    let items: &[Node] = match root.as_ref().map(|r| &r.kind) {
        Some(Kind::Mapping(entries)) => entries
            .iter()
            .find(|e| e.key == "objects")
            .and_then(|e| match &e.value.kind {
                Kind::Sequence(items) => Some(items.as_slice()),
                _ => None,
            })
            .unwrap_or_default(),
        _ => &[],
    };
    let carries = |block: &str, tag: &str| {
        blocks
            .iter()
            .find(|b| b.name == block)
            .map(|b| b.carried_attributes().filter(|d| d.tag == tag).count())
    };
    let mut fills: BTreeMap<String, usize> = BTreeMap::new();
    for item in items {
        let mut item = item.clone();
        let _ = places_in(&mut item, "", &carries, &mut |node, _, parts_here| {
            // A list of parts fills one place with each value it holds.
            match parts_here.take().then(|| parts_of(node)).flatten() {
                Some(parts) => {
                    for name in parts::names(&parts) {
                        *fills.entry(name).or_default() += 1;
                    }
                }
                None => {
                    if let Ok(Some(name)) = row_value_name(node, "") {
                        *fills.entry(name).or_default() += 1;
                    }
                }
            }
            Ok(())
        });
    }
    let objects = records
        .iter()
        .map(|record| {
            // Its place in its list, the template's or the row's: the last
            // index of its field, `objects[3]` or `rows[1].objects[0]`.
            let place = record
                .object
                .field
                .rsplit_once('[')
                .and_then(|(_, i)| i.trim_end_matches(']').parse::<usize>().ok());
            let item = place.and_then(|i| {
                if record.in_row {
                    row.objects.get(i)
                } else {
                    items.get(i)
                }
            });
            item.map(object_sources).unwrap_or_default()
        })
        .collect();
    SheetSources { objects, fills }
}

/// Where the object `item`, as its record writes it, writes its contents
/// and its attribute values.
fn object_sources(item: &Node) -> ObjectSources {
    let mut sources = ObjectSources::default();
    let Kind::Mapping(types) = &item.kind else {
        return sources;
    };
    let [object] = types.as_slice() else {
        return sources;
    };
    let Kind::Mapping(keys) = &object.value.kind else {
        return sources;
    };
    let find = |key: &str| keys.iter().find(|k| k.key == key).map(|k| &k.value);
    match object.key.as_str() {
        "text" | "mtext" => {
            sources.contents = find("contents").map(|contents| match parts_of(contents) {
                Some(parts) => Source::Parts(parts::names(&parts)),
                None => source_of(contents),
            });
        }
        "insert" => {
            if let Some(Kind::Mapping(values)) = find("attributes").map(|a| &a.kind) {
                for value in values {
                    let given = match &value.value.kind {
                        Kind::Sequence(items) => Given::List {
                            items: items.iter().map(source_of).collect(),
                            parts: parts_of(&value.value).map(|parts| parts::names(&parts)),
                        },
                        _ => Given::One(source_of(&value.value)),
                    };
                    sources
                        .attributes
                        .push((tag_in_capitals(&value.key), given));
                }
            }
        }
        _ => {}
    }
    sources
}

/// Where a value written at `node` comes from.
fn source_of(node: &Node) -> Source {
    if node.is_bare_null() {
        return Source::Unset;
    }
    match row_value_name(node, "") {
        Ok(Some(name)) => Source::Row(name),
        _ => Source::Own,
    }
}
