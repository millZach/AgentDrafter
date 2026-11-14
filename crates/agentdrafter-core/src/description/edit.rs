//! Edits a record in place: a template's objects, a row's own objects and a
//! row's values, as an operation of a change set asks (`crate::change`).
//!
//! The file is read into the strict YAML tree, which keeps where each node
//! is written. An edit changes the tree; the new text rewrites only the
//! objects and values the edits touched, each in the style it was written
//! in, and keeps every other byte of the file, comments included. An object
//! added goes after the list's last, an object deleted takes its lines with
//! it, and a row that had no objects gains `objects:`.
//!
//! The new text is read back before anything is saved and must read as the
//! tree the edits made; a text that does not is refused.
//!
//! A block library's blocks are edited one block at a time: a block an
//! edit sets is written again whole, a key to a line, in the place of its
//! entry, and a block added goes after the library's last; every other
//! block, and every other byte, stays as it was.
//!
//! IDs stay put. An object an edit adds is given an `id` by the caller, and
//! deleting an object writes the place of each later object that has no
//! `id` as its `id`, such as `objects[3]`, so that no object's ID moves.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::emit::{number_text, scalar_text, Context, Emitter};
use super::ids::{ID_KEY, NAME_KEY};
use super::objects::{article, BLOCK_OBJECT_TYPES, OBJECT_TYPES};
use super::polyline::VERTEX_KEYS;
use super::set::ROW_OBJECTS_KEY;
use super::strict_yaml::{self, index_path, key_path, Entry, Kind, Node, Source, Style};
use super::template::ROW_KEY;
use crate::error::{Error, ErrorCode};
use crate::units::{parse_length, LengthError, Unit};

/// A list of objects in a record: a template's own, at `objects`, or one
/// row's, at `rows[N].objects`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum ObjectList {
    Template,
    Row(usize),
}

impl ObjectList {
    /// The path of the list in its file, such as `rows[1].objects`.
    pub(crate) fn path(self) -> String {
        match self {
            ObjectList::Template => "objects".to_owned(),
            ObjectList::Row(row) => key_path(&index_path("rows", row), ROW_OBJECTS_KEY),
        }
    }

    /// The place of the list's object at `index`, such as `objects[3]`,
    /// which is the object's ID in its file when it has no `id`.
    pub(crate) fn place(self, index: usize) -> String {
        index_path(&self.path(), index)
    }
}

/// One object of a list as the record writes it now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Listed {
    /// The ID it carries in its file: its `id`, or its place.
    pub id: String,
    /// Whether the file writes its `id`.
    pub written: bool,
    pub name: Option<String>,
    /// Its type, such as `line`.
    pub kind: String,
}

/// What a change did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Changed {
    /// Whether the object's keys were edited.
    pub edited: bool,
    /// The row's value a text's contents come from and the text to set it
    /// to, when the change set the contents of a text whose template takes
    /// them from the row.
    pub from_row: Option<(String, String)>,
}

/// An insert in a list of objects, or among a block's: its place in the
/// list, the block it inserts, and each attribute value it gives, by tag,
/// as the record writes the value, such as `B` or `{row: REV}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InsertValues {
    pub index: usize,
    pub block: String,
    pub values: Vec<(String, String)>,
}

/// What setting an attribute did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AttributeSet {
    /// The insert's attribute was written.
    Written,
    /// The template takes the attribute's value from the row: the value
    /// `name`, which is set in the sheet's row, not here.
    FromRow(String),
}

/// A record file being edited.
pub(crate) struct RecordFile {
    pub(crate) path: PathBuf,
    /// The text as read, without a byte-order mark.
    text: String,
    bom: bool,
    newline: &'static str,
    root: Node,
    lists: Vec<(ObjectList, ListState)>,
    values: Vec<ValueEdit>,
    /// The tables at the top of the file an edit changed, such as
    /// `layers`, each written again whole, one entry to a line.
    tables: Vec<String>,
    /// The blocks of a block library an edit set or changed, each written
    /// again whole on its own lines.
    blocks: Vec<BlockEdit>,
}

/// A block of a block library an edit set or changed.
struct BlockEdit {
    name: String,
    /// The lines its entry was written on, from the start of its key's
    /// line to just after its value's last line, and the column of its
    /// key; none for a block the file did not have.
    lines: Option<(usize, usize, usize)>,
}

/// What became of one list's objects: where the file wrote each at first,
/// and for each object now, which one it was and which operations touched
/// it.
struct ListState {
    /// Whether the list was written in the file.
    existed: bool,
    /// Where the file wrote each object, in its first order.
    sources: Vec<Option<Source>>,
    /// Each object now, in order.
    items: Vec<Item>,
}

struct Item {
    /// Its place in the file as read, or `None` for an object added.
    origin: Option<usize>,
    edited: bool,
    /// The operations that touched it, by their place in the change set,
    /// in the order they did, each with what it set.
    touches: Vec<(usize, Touch)>,
}

/// What an operation set in an object.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Touch {
    /// The whole object, which it added.
    Whole,
    /// These keys, such as `layer` or `attributes.REV`.
    Keys(Vec<String>),
}

impl Touch {
    /// Whether it set the value at `key`, a path within the object such as
    /// `layer`, `points[2]` or `attributes.REV`.
    fn covers(&self, key: &str) -> bool {
        match self {
            Touch::Whole => true,
            Touch::Keys(keys) => keys.iter().any(|set| {
                key == set
                    || key.starts_with(&format!("{set}."))
                    || key.starts_with(&format!("{set}["))
                    || set.starts_with(&format!("{key}."))
            }),
        }
    }
}

/// A row's value an edit set: where its old text was written.
struct ValueEdit {
    row: usize,
    name: String,
    source: Source,
    key_end: usize,
    key_column: usize,
    in_flow: bool,
}

impl RecordFile {
    /// The record whose text, read from `path`, is `text`.
    pub(crate) fn from_text(path: &Path, text: String) -> Result<Self, Error> {
        let bom = text.starts_with('\u{FEFF}');
        let text = if bom {
            text['\u{FEFF}'.len_utf8()..].to_owned()
        } else {
            text
        };
        let root = strict_yaml::read(&text)
            .map_err(|e| e.or_in_file(path))?
            .ok_or_else(|| Error::new(ErrorCode::MissingKey, "the file is empty").in_file(path))?;
        let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
        Ok(Self {
            path: path.to_owned(),
            text,
            bom,
            newline,
            root,
            lists: Vec::new(),
            values: Vec::new(),
            tables: Vec::new(),
            blocks: Vec::new(),
        })
    }

    /// Whether any edit has been made.
    pub(crate) fn changed(&self) -> bool {
        !self.values.is_empty()
            || !self.tables.is_empty()
            || !self.blocks.is_empty()
            || self.lists.iter().any(|(_, state)| state.dirty())
    }

    /// The objects of `list` as the record writes them now. A row with no
    /// objects has none.
    pub(crate) fn objects(&self, list: ObjectList) -> Vec<Listed> {
        let Some(items) = self.items(list) else {
            return Vec::new();
        };
        items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let (kind, keys) = object_parts(item);
                let id = keys.and_then(|k| scalar_of(k, ID_KEY));
                Listed {
                    written: id.is_some(),
                    id: id.map_or_else(|| list.place(i), str::to_owned),
                    name: keys.and_then(|k| scalar_of(k, NAME_KEY)).map(str::to_owned),
                    kind: kind.unwrap_or_default().to_owned(),
                }
            })
            .collect()
    }

    /// The names `table`, such as `layers`, defines, in the file's order.
    pub(crate) fn table_names(&self, table: &str) -> Vec<String> {
        match mapping_entry(&self.root, table).map(|t| &t.kind) {
            Some(Kind::Mapping(entries)) => entries.iter().map(|e| e.key.clone()).collect(),
            _ => Vec::new(),
        }
    }

    /// The entry `name` of `table` as one line of the record's YAML, such
    /// as `{color: red}`; none when the table does not define it.
    pub(crate) fn table_entry_text(&self, table: &str, name: &str) -> Option<String> {
        let Some(Kind::Mapping(entries)) = mapping_entry(&self.root, table).map(|t| &t.kind) else {
            return None;
        };
        entries
            .iter()
            .find(|e| e.key == name)
            .map(|e| self.emitter().flow(&e.value))
    }

    /// The text value of `key` in the entry `name` of `table`, such as a
    /// layer's `locked`.
    pub(crate) fn table_value(&self, table: &str, name: &str, key: &str) -> Option<String> {
        let Some(Kind::Mapping(entries)) = mapping_entry(&self.root, table).map(|t| &t.kind) else {
            return None;
        };
        let entry = entries.iter().find(|e| e.key == name)?;
        let Kind::Mapping(keys) = &entry.value.kind else {
            return None;
        };
        scalar_of(keys, key).map(str::to_owned)
    }

    /// The names of the entries of `table` whose `key` is `value`, such as
    /// the layers whose linetype is `DASHED`.
    pub(crate) fn table_users(&self, table: &str, key: &str, value: &str) -> Vec<String> {
        let Some(Kind::Mapping(entries)) = mapping_entry(&self.root, table).map(|t| &t.kind) else {
            return Vec::new();
        };
        entries
            .iter()
            .filter(|e| match &e.value.kind {
                Kind::Mapping(keys) => scalar_of(keys, key) == Some(value),
                _ => false,
            })
            .map(|e| e.key.clone())
            .collect()
    }

    /// Adds the entry `name`, with `keys` written in the order of
    /// `order`, to `table` at the top of the file, making the table when
    /// the file has none.
    pub(crate) fn table_add(
        &mut self,
        table: &str,
        order: &[&str],
        name: &str,
        keys: &Map<String, Value>,
    ) -> Result<(), Error> {
        let mut ordered: Vec<(&String, &Value)> = keys.iter().collect();
        ordered.sort_by_key(|(key, _)| order.iter().position(|k| k == key).unwrap_or(order.len()));
        let value = Node::new(Kind::Mapping(
            ordered
                .into_iter()
                .map(|(key, value)| new_entry(key, node_of(value)))
                .collect(),
        ));
        self.table_mut(table)?.push(new_entry(name, value));
        self.note_table(table);
        Ok(())
    }

    /// Sets keys of the entry `name` of `table`: each key of `set` to its
    /// new value, and a null takes a key away. An entry the file does not
    /// write, such as layer `0`, is made.
    pub(crate) fn table_change(
        &mut self,
        table: &str,
        order: &[&str],
        name: &str,
        set: &Map<String, Value>,
    ) -> Result<(), Error> {
        let entries = self.table_mut(table)?;
        if !entries.iter().any(|e| e.key == name) {
            entries.push(new_entry(name, Node::new(Kind::Mapping(Vec::new()))));
        }
        let entry = entries
            .iter_mut()
            .find(|e| e.key == name)
            .expect("just made");
        if !matches!(entry.value.kind, Kind::Mapping(_)) {
            entry.value = Node::new(Kind::Mapping(Vec::new()));
        }
        let Kind::Mapping(keys) = &mut entry.value.kind else {
            unreachable!("a mapping")
        };
        for (key, value) in set {
            let position = keys.iter().position(|e| e.key == *key);
            match (value, position) {
                (Value::Null, Some(p)) => {
                    keys.remove(p);
                }
                (Value::Null, None) => {}
                (value, Some(p)) => keys[p].value = node_of(value),
                (value, None) => keys.push(new_entry(key, node_of(value))),
            }
        }
        keys.sort_by_key(|k| {
            order
                .iter()
                .position(|o| *o == k.key)
                .unwrap_or(order.len())
        });
        self.note_table(table);
        Ok(())
    }

    /// Renames the entry `name` of `table` to `to`.
    pub(crate) fn table_rename(&mut self, table: &str, name: &str, to: &str) -> Result<(), Error> {
        let entries = self.table_mut(table)?;
        if let Some(entry) = entries.iter_mut().find(|e| e.key == name) {
            to.clone_into(&mut entry.key);
        }
        self.note_table(table);
        Ok(())
    }

    /// Sets `key` to `to` in every entry of `table` where it is `from`,
    /// such as each layer's linetype when the linetype is renamed.
    pub(crate) fn table_rename_values(&mut self, table: &str, key: &str, from: &str, to: &str) {
        if self.table_users(table, key, from).is_empty() {
            return;
        }
        let Some(Kind::Mapping(entries)) =
            mapping_entry_mut(&mut self.root, table).map(|t| &mut t.kind)
        else {
            return;
        };
        for entry in entries {
            if let Kind::Mapping(keys) = &mut entry.value.kind {
                for k in keys.iter_mut().filter(|k| k.key == key) {
                    if matches!(&k.value.kind, Kind::Scalar(text) if text == from) {
                        k.value = Node::scalar(to.to_owned());
                    }
                }
            }
        }
        self.note_table(table);
    }

    /// Takes the entry `name` away from `table`.
    pub(crate) fn table_delete(&mut self, table: &str, name: &str) -> Result<(), Error> {
        self.table_mut(table)?.retain(|e| e.key != name);
        self.note_table(table);
        Ok(())
    }

    /// The IDs in its file of each object of `list` whose `key` is
    /// `value`, such as every object on a layer.
    pub(crate) fn object_users(&self, list: ObjectList, key: &str, value: &str) -> Vec<String> {
        let listed = self.objects(list);
        self.items(list)
            .unwrap_or_default()
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                object_parts(item)
                    .1
                    .is_some_and(|keys| scalar_of(keys, key) == Some(value))
            })
            .map(|(i, _)| listed[i].id.clone())
            .collect()
    }

    /// The text value of `key` in the object of `list` at `index`, such
    /// as its `layer`; none when it does not write it.
    pub(crate) fn object_value(&self, list: ObjectList, index: usize, key: &str) -> Option<String> {
        let item = self.items(list)?.get(index)?;
        object_parts(item)
            .1
            .and_then(|keys| scalar_of(keys, key))
            .map(str::to_owned)
    }

    /// Sets `key` to `to` in every object of `list` where it is `from`, as
    /// operation `operation`, and gives how many it set.
    pub(crate) fn rename_values(
        &mut self,
        list: ObjectList,
        key: &str,
        from: &str,
        to: &str,
        operation: usize,
    ) -> usize {
        let users: Vec<usize> = self
            .items(list)
            .unwrap_or_default()
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                object_parts(item)
                    .1
                    .is_some_and(|keys| scalar_of(keys, key) == Some(from))
            })
            .map(|(i, _)| i)
            .collect();
        if users.is_empty() {
            return 0;
        }
        self.state(list);
        for &index in &users {
            if let Ok(items) = self.items_mut(list) {
                if let Some(keys) = object_keys_mut(&mut items[index]) {
                    for k in keys.iter_mut().filter(|k| k.key == key) {
                        k.value = Node::scalar(to.to_owned());
                    }
                }
            }
            self.touch(list, index, operation, Touch::Keys(vec![key.to_owned()]));
        }
        users.len()
    }

    /// Each insert of `list`, with the attribute values it gives as the
    /// record writes them.
    pub(crate) fn inserts(&self, list: ObjectList) -> Vec<InsertValues> {
        self.items(list)
            .unwrap_or_default()
            .iter()
            .enumerate()
            .filter_map(|(index, item)| self.insert_values(index, item))
            .collect()
    }

    /// Takes the value the insert of `list` at `index` gives for the
    /// attribute `tag` away, as operation `operation`, and its
    /// `attributes` with it when no value is left.
    pub(crate) fn remove_attribute(
        &mut self,
        list: ObjectList,
        index: usize,
        tag: &str,
        operation: usize,
    ) -> Result<(), Error> {
        self.state(list);
        let items = self.items_mut(list)?;
        let keys = object_keys_mut(&mut items[index]).ok_or_else(|| record_shape("insert"))?;
        remove_attribute_value(keys, tag);
        let key = key_path("attributes", tag);
        self.touch(list, index, operation, Touch::Keys(vec![key]));
        Ok(())
    }

    /// The names of a block library's blocks, in the file's order.
    pub(crate) fn block_names(&self) -> Vec<String> {
        self.table_names(BLOCKS_KEY)
    }

    /// The block `name` of a block library as one line of its YAML; none
    /// when the library does not define it.
    pub(crate) fn block_text(&self, name: &str) -> Option<String> {
        self.table_entry_text(BLOCKS_KEY, name)
    }

    /// The tags each insert of the block `name` carries, as the library
    /// writes the block now: its attribute definitions' tags, but a
    /// constant one's, whose value is the definition's on every insert.
    /// None when the library does not define the block.
    pub(crate) fn block_carried_tags(&self, name: &str) -> Option<Vec<String>> {
        let block = self.block_node(name)?;
        let mut tags = Vec::new();
        for item in block_items_of(block) {
            if let (Some("attdef"), Some(keys)) = object_parts(item) {
                if scalar_of(keys, "constant") == Some("true") {
                    continue;
                }
                if let Some(tag) = scalar_of(keys, "tag") {
                    tags.push(tag.to_owned());
                }
            }
        }
        Some(tags)
    }

    /// Each insert among the objects of each block of a block library, with
    /// the block whose definition holds it.
    pub(crate) fn block_inserts(&self) -> Vec<(String, InsertValues)> {
        let Some(Kind::Mapping(blocks)) = mapping_entry(&self.root, BLOCKS_KEY).map(|b| &b.kind)
        else {
            return Vec::new();
        };
        let mut found = Vec::new();
        for block in blocks {
            for (index, item) in block_items_of(&block.value).iter().enumerate() {
                if let Some(insert) = self.insert_values(index, item) {
                    found.push((block.key.clone(), insert));
                }
            }
        }
        found
    }

    /// Sets the block `name` of a block library to `objects`, at
    /// `base_point` when one is given: a block it has is replaced whole,
    /// and one it has not is added after its last. Each object is written
    /// with its keys in the order its type lists them.
    pub(crate) fn block_set(
        &mut self,
        name: &str,
        base_point: Option<&Value>,
        objects: &[Value],
    ) -> Result<(), Error> {
        let mut keys = Vec::new();
        if let Some(point) = base_point {
            keys.push(new_entry("base_point", node_of(point)));
        }
        keys.push(new_entry(
            "objects",
            Node::new(Kind::Sequence(objects.iter().map(block_object).collect())),
        ));
        let value = Node::new(Kind::Mapping(keys));
        self.note_block(name)?;
        let blocks = self.table_mut(BLOCKS_KEY)?;
        match blocks.iter_mut().find(|e| e.key == name) {
            Some(entry) => entry.value = value,
            None => blocks.push(new_entry(name, value)),
        }
        Ok(())
    }

    /// Takes the value the insert at `index` among the objects of the block
    /// `block` gives for the attribute `tag` away, and its `attributes`
    /// with it when no value is left.
    pub(crate) fn remove_block_attribute(
        &mut self,
        block: &str,
        index: usize,
        tag: &str,
    ) -> Result<(), Error> {
        self.note_block(block)?;
        let shape = || record_shape_in("a block's list of objects", &self.path);
        let node = mapping_entry_mut(&mut self.root, BLOCKS_KEY)
            .and_then(|blocks| mapping_entry_mut(blocks, block))
            .and_then(|block| mapping_entry_mut(block, "objects"));
        let Some(Node {
            kind: Kind::Sequence(items),
            ..
        }) = node
        else {
            return Err(shape());
        };
        let keys = items
            .get_mut(index)
            .and_then(object_keys_mut)
            .ok_or_else(|| record_shape("insert"))?;
        remove_attribute_value(keys, tag);
        Ok(())
    }

    /// The block `name` of a block library, as the record holds it now.
    fn block_node(&self, name: &str) -> Option<&Node> {
        mapping_entry(mapping_entry(&self.root, BLOCKS_KEY)?, name)
    }

    /// Notes that an edit sets the block `name`, before it does, keeping
    /// where the file wrote its entry.
    fn note_block(&mut self, name: &str) -> Result<(), Error> {
        if self.blocks.iter().any(|b| b.name == name) {
            return Ok(());
        }
        let blocks = mapping_entry(&self.root, BLOCKS_KEY);
        // Blocks written `{}` take their first block a key to a line in
        // their place; blocks written in braces that hold any are not
        // rewritten.
        let in_braces = blocks
            .and_then(|b| b.source)
            .is_some_and(|s| s.style == Style::Flow);
        let any_written = matches!(
            blocks.map(|b| &b.kind),
            Some(Kind::Mapping(entries)) if entries.iter().any(|e| e.key_end > 0)
        );
        if in_braces && any_written {
            return Err(self.cannot_write("its blocks are written in braces"));
        }
        let lines = match blocks.map(|b| &b.kind) {
            Some(Kind::Mapping(entries)) => entries
                .iter()
                .find(|e| e.key == name && e.key_end > 0)
                .map(|e| {
                    let start = line_start(&self.text, e.key_end);
                    let end = e.value.source.map_or_else(
                        || line_end(&self.text, e.key_end),
                        |s| line_end(&self.text, s.end),
                    );
                    (start, end, e.key_at.column.saturating_sub(1))
                }),
            _ => None,
        };
        self.blocks.push(BlockEdit {
            name: name.to_owned(),
            lines,
        });
        Ok(())
    }

    /// The block `name` written whole, a key to a line, its key at
    /// `column`: its base point in brackets and its objects an item to a
    /// line, ending with a line break.
    fn block_lines(&self, name: &str, value: &Node, column: usize) -> String {
        let nl = self.newline;
        let emitter = self.emitter();
        let indent = " ".repeat(column);
        let key = scalar_text(name, Context::Flow, column, nl);
        let Kind::Mapping(keys) = &value.kind else {
            return format!("{indent}{key}: {}{nl}", emitter.flow(value));
        };
        let mut out = format!("{indent}{key}:{nl}");
        for entry in keys {
            out.push_str(&" ".repeat(column + 2));
            out.push_str(&scalar_text(&entry.key, Context::Flow, column + 2, nl));
            match &entry.value.kind {
                Kind::Sequence(items) if entry.key == "objects" && !items.is_empty() => {
                    out.push(':');
                    out.push_str(nl);
                    for item in items {
                        out.push_str(&" ".repeat(column + 4));
                        out.push_str("- ");
                        out.push_str(&emitter.item(item, column + 6));
                        out.push_str(nl);
                    }
                }
                _ => {
                    out.push_str(": ");
                    out.push_str(&emitter.flow(&entry.value));
                    out.push_str(nl);
                }
            }
        }
        out
    }

    /// The splices that write each block an edit set again, where the file
    /// wrote it, and each block added after the last the file wrote.
    fn block_splices(&self, splices: &mut Vec<(usize, usize, String)>) -> Result<(), Error> {
        if self.blocks.is_empty() {
            return Ok(());
        }
        let Some(Kind::Mapping(entries)) = mapping_entry(&self.root, BLOCKS_KEY).map(|b| &b.kind)
        else {
            return Err(record_shape_in("a mapping under `blocks`", &self.path));
        };
        let nl = self.newline;
        // Blocks the file wrote `{}`: every block is new, written a key to
        // a line under `blocks:` in place of the braces.
        let root_entry = match &self.root.kind {
            Kind::Mapping(root) => root.iter().find(|e| e.key == BLOCKS_KEY && e.key_end > 0),
            _ => None,
        };
        if let Some(entry) = root_entry {
            if let Some(source) = entry.value.source.filter(|s| s.style == Style::Flow) {
                let column = entry.key_at.column.saturating_sub(1) + 2;
                let mut with = format!(":{nl}");
                for edit in &self.blocks {
                    if let Some(block) = entries.iter().find(|e| e.key == edit.name) {
                        with.push_str(&self.block_lines(&edit.name, &block.value, column));
                    }
                }
                with.truncate(with.len() - nl.len());
                splices.push((entry.key_end, source.end, with));
                return Ok(());
            }
        }
        // The end of the last block the file wrote, and the column of its
        // blocks' names.
        let written: Vec<(usize, usize)> = entries
            .iter()
            .filter(|e| e.key_end > 0)
            .map(|e| {
                let end = match self.blocks.iter().find(|b| b.name == e.key) {
                    Some(BlockEdit {
                        lines: Some((_, end, _)),
                        ..
                    }) => *end,
                    _ => e.value.source.map_or_else(
                        || line_end(&self.text, e.key_end),
                        |s| line_end(&self.text, s.end),
                    ),
                };
                (end, e.key_at.column.saturating_sub(1))
            })
            .collect();
        let Some(&(last_end, column)) = written.iter().max_by_key(|(end, _)| *end) else {
            return Err(self.cannot_write("its blocks are not written a block to a line"));
        };
        let mut added = String::new();
        for edit in &self.blocks {
            let Some(entry) = entries.iter().find(|e| e.key == edit.name) else {
                continue;
            };
            match edit.lines {
                Some((start, end, column)) => {
                    let mut with = self.block_lines(&edit.name, &entry.value, column);
                    if end == self.text.len() && !self.text.ends_with('\n') {
                        with.truncate(with.len() - nl.len());
                    }
                    splices.push((start, end, with));
                }
                None => added.push_str(&self.block_lines(&edit.name, &entry.value, column)),
            }
        }
        if !added.is_empty() {
            let mut with = String::new();
            if last_end == self.text.len() && !self.text.ends_with('\n') {
                with.push_str(nl);
            }
            with.push_str(&added);
            splices.push((last_end, last_end, with));
        }
        Ok(())
    }

    /// The insert `item`, at `index` in its list, with its block and the
    /// attribute values it gives; none when it is not an insert.
    fn insert_values(&self, index: usize, item: &Node) -> Option<InsertValues> {
        let (Some("insert"), Some(keys)) = object_parts(item) else {
            return None;
        };
        let block = scalar_of(keys, "block")?.to_owned();
        let values = match keys
            .iter()
            .find(|k| k.key == "attributes")
            .map(|k| &k.value.kind)
        {
            Some(Kind::Mapping(values)) => values
                .iter()
                .map(|v| {
                    let text = match &v.value.kind {
                        Kind::Scalar(text) => text.clone(),
                        _ => self.emitter().flow(&v.value),
                    };
                    (v.key.clone(), text)
                })
                .collect(),
            _ => Vec::new(),
        };
        Some(InsertValues {
            index,
            block,
            values,
        })
    }

    /// The entries of `table` at the top of the file, to edit, made when
    /// the file has none.
    fn table_mut(&mut self, table: &str) -> Result<&mut Vec<Entry>, Error> {
        let path = self.path.clone();
        let Kind::Mapping(entries) = &mut self.root.kind else {
            return Err(record_shape_in("a mapping", &path));
        };
        if !entries.iter().any(|e| e.key == table) {
            // Before the objects, where the text writes it.
            let at = entries
                .iter()
                .position(|e| e.key == "objects")
                .unwrap_or(entries.len());
            entries.insert(at, new_entry(table, Node::new(Kind::Mapping(Vec::new()))));
        }
        let entry = entries
            .iter_mut()
            .find(|e| e.key == table)
            .expect("just made");
        if entry.value.is_bare_null() {
            entry.value = Node::new(Kind::Mapping(Vec::new()));
        }
        match &mut entry.value.kind {
            Kind::Mapping(entries) => Ok(entries),
            _ => Err(record_shape_in(
                &format!("a mapping under `{table}`"),
                &path,
            )),
        }
    }

    fn note_table(&mut self, table: &str) {
        if !self.tables.iter().any(|t| t == table) {
            self.tables.push(table.to_owned());
        }
    }

    /// The splice that writes `table` again whole, one entry to a line, in
    /// the place the file wrote it, or else before the file's `objects`.
    fn table_splice(&self, table: &str) -> Result<(usize, usize, String), Error> {
        let Kind::Mapping(root) = &self.root.kind else {
            return Err(record_shape_in("a mapping", &self.path));
        };
        if self.root.source.is_some_and(|s| s.style == Style::Flow) {
            return Err(self.cannot_write("the file is written in braces"));
        }
        let entry = root
            .iter()
            .find(|e| e.key == table)
            .expect("an edited table is in the file");
        let nl = self.newline;
        let emitter = self.emitter();
        let Kind::Mapping(entries) = &entry.value.kind else {
            return Err(record_shape_in(
                &format!("a mapping under `{table}`"),
                &self.path,
            ));
        };
        let render = |column: usize| -> String {
            let indent = " ".repeat(column);
            if entries.is_empty() {
                return format!("{indent}{table}: {{}}{nl}");
            }
            let mut with = format!("{indent}{table}:{nl}");
            for e in entries {
                with.push_str(&" ".repeat(column + 2));
                with.push_str(&scalar_text(&e.key, Context::Flow, column + 2, nl));
                with.push_str(": ");
                with.push_str(&emitter.flow(&e.value));
                with.push_str(nl);
            }
            with
        };
        if entry.key_end > 0 {
            let column = entry.key_at.column.saturating_sub(1);
            let start = line_start(&self.text, entry.key_end);
            let end = entry.value.source.map_or_else(
                || line_end(&self.text, entry.key_end),
                |s| line_end(&self.text, s.end),
            );
            let mut with = render(column);
            if end == self.text.len() && !self.text.ends_with('\n') {
                with.truncate(with.len() - nl.len());
            }
            return Ok((start, end, with));
        }
        // A table the file did not write goes before its objects.
        let column = root
            .first()
            .map_or(0, |e| e.key_at.column.saturating_sub(1));
        match root.iter().find(|e| e.key == "objects" && e.key_end > 0) {
            Some(objects) => {
                let at = line_start(&self.text, objects.key_end);
                Ok((at, at, render(column)))
            }
            None => {
                let at = self.text.len();
                let lead = if self.text.ends_with('\n') { "" } else { nl };
                Ok((at, at, format!("{lead}{}", render(column))))
            }
        }
    }

    /// The operation to blame for what is wrong at `within` in the object
    /// of `list` at `index`, such as `circle.layer`: the last that set that
    /// value, or else the last that touched the object at all. `None` when
    /// no operation touched it.
    pub(crate) fn blame(&self, list: ObjectList, index: usize, within: &str) -> Option<usize> {
        let item = self
            .lists
            .iter()
            .find(|(l, _)| *l == list)
            .and_then(|(_, state)| state.items.get(index))?;
        // The path within the object, after its type: `circle.layer` is
        // `layer`.
        let key = within.split_once('.').map_or("", |(_, key)| key);
        let set_it = item
            .touches
            .iter()
            .rev()
            .find(|(_, touch)| !key.is_empty() && touch.covers(key));
        set_it.or(item.touches.last()).map(|(op, _)| *op)
    }

    /// The object of `list` at `index` as one line of the record's YAML,
    /// such as `line: {from: [0, 0, 0], to: [1, 1, 0], id: a}`.
    pub(crate) fn object_text(&self, list: ObjectList, index: usize) -> String {
        self.items(list)
            .and_then(|items| items.get(index))
            .map(|item| self.emitter().flow(item))
            .map(|text| {
                text.strip_prefix('{')
                    .and_then(|t| t.strip_suffix('}'))
                    .map_or(text.clone(), str::to_owned)
            })
            .unwrap_or_default()
    }

    /// Adds an object of type `kind` with `keys` at the end of `list`, as
    /// operation `operation`, and gives its place in the list. The keys are
    /// written in the order the object's type lists them.
    pub(crate) fn add(
        &mut self,
        list: ObjectList,
        kind: &str,
        keys: &Map<String, Value>,
        operation: usize,
    ) -> Result<usize, Error> {
        let order: Vec<&str> = OBJECT_TYPES
            .iter()
            .find(|(name, _)| *name == kind)
            .map(|(_, keys)| keys.iter().copied().chain([ID_KEY, NAME_KEY]).collect())
            .unwrap_or_default();
        let mut ordered: Vec<(&String, &Value)> = keys.iter().collect();
        ordered.sort_by_key(|(key, _)| order.iter().position(|k| k == key).unwrap_or(order.len()));
        let mut entries = Vec::with_capacity(ordered.len());
        for (key, value) in ordered {
            let mut node = node_of(value);
            if kind == "polyline" && key == "points" {
                order_vertices(&mut node);
            }
            entries.push(new_entry(key, node));
        }
        let object = Node::new(Kind::Mapping(vec![new_entry(
            kind,
            Node::new(Kind::Mapping(entries)),
        )]));
        self.state(list);
        let items = self.items_mut(list)?;
        items.push(object);
        let index = items.len() - 1;
        self.list_state(list).items.push(Item {
            origin: None,
            edited: true,
            touches: vec![(operation, Touch::Whole)],
        });
        Ok(index)
    }

    /// Deletes the object of `list` at `index`, as operation `operation`.
    /// Each later object of the list that has no `id` is given its place as
    /// its `id`, so that its ID does not move; their IDs in the file are
    /// returned.
    pub(crate) fn delete(
        &mut self,
        list: ObjectList,
        index: usize,
        operation: usize,
    ) -> Vec<String> {
        self.state(list);
        let mut kept = Vec::new();
        if let Ok(items) = self.items_mut(list) {
            items.remove(index);
            for (i, item) in items.iter_mut().enumerate().skip(index) {
                let place = list.place(i + 1);
                if let Some(keys) = object_keys_mut(item) {
                    // An `id` that is a bare null is left out, so the
                    // object takes its place: write that in its stead.
                    match keys.iter_mut().find(|e| e.key == ID_KEY) {
                        Some(id) if id.value.is_bare_null() => {
                            id.value = Node::scalar(place.clone());
                            kept.push((i, place));
                        }
                        Some(_) => {}
                        None => {
                            keys.push(new_entry(ID_KEY, Node::scalar(place.clone())));
                            kept.push((i, place));
                        }
                    }
                }
            }
        }
        let state = self.list_state(list);
        state.items.remove(index);
        for (i, _) in &kept {
            let item = &mut state.items[*i];
            item.edited = true;
            item.touches
                .push((operation, Touch::Keys(vec![ID_KEY.to_owned()])));
        }
        kept.into_iter().map(|(_, place)| place).collect()
    }

    /// Sets the keys of the object of `list` at `index`, as operation
    /// `operation`: each value in `set` replaces the key's, and `null`
    /// takes the key away. A text's contents the template takes from the
    /// row are not set here: the row's value and its new text are returned,
    /// for the caller to set in the sheet's row.
    pub(crate) fn change(
        &mut self,
        list: ObjectList,
        index: usize,
        set: &Map<String, Value>,
        operation: usize,
    ) -> Result<Changed, Error> {
        self.state(list);
        let items = self.items_mut(list)?;
        let item = &mut items[index];
        let kind = object_parts(item).0.unwrap_or_default().to_owned();
        let own: Vec<&str> = OBJECT_TYPES
            .iter()
            .find(|(name, _)| *name == kind)
            .map(|(_, keys)| keys.to_vec())
            .unwrap_or_default();
        let mut allowed: Vec<&str> = own
            .iter()
            .copied()
            .filter(|k| !(kind == "insert" && *k == "attributes"))
            .collect();
        allowed.push(NAME_KEY);
        let keys = object_keys_mut(item).ok_or_else(|| record_shape(&kind))?;
        let mut from_row = None;
        let mut edited = false;
        for (key, value) in set {
            let field = key_path("set", key);
            if key == ID_KEY {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    "an object's ID is fixed for its life, so `change` does not set `id`; delete \
                     the object and add it again with the new `id` to give it another",
                )
                .at_field(field));
            }
            if kind == "insert" && key == "attributes" {
                return Err(Error::new(
                    ErrorCode::UnknownKey,
                    format!(
                        "`change` does not set an insert's `attributes`; set each with \
                         `set_attribute`. An insert's change takes {}",
                        allowed.join(", ")
                    ),
                )
                .at_field(field));
            }
            if !allowed.contains(&key.as_str()) {
                return Err(Error::new(
                    ErrorCode::UnknownKey,
                    format!(
                        "unknown key `{key}`; `change` sets keys {} {kind} takes: {}",
                        article(&kind),
                        allowed.join(", ")
                    ),
                )
                .at_field(field));
            }
            let position = keys.iter().position(|e| e.key == *key);
            if key == "contents" {
                if let Some(name) = position.and_then(|p| row_value_name(&keys[p].value)) {
                    let text = match value {
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.to_string(),
                        Value::Bool(b) => b.to_string(),
                        _ => {
                            return Err(Error::new(
                                ErrorCode::WrongType,
                                format!(
                                    "the template takes this text's contents from the row's \
                                     value `{name}`, which is text; give the new contents as text"
                                ),
                            )
                            .at_field(field))
                        }
                    };
                    from_row = Some((name, text));
                    continue;
                }
            }
            match (value, position) {
                (Value::Null, Some(p)) => {
                    keys.remove(p);
                }
                (Value::Null, None) => continue,
                (value, position) => {
                    let mut node = node_of(value);
                    if kind == "polyline" && key == "points" {
                        order_vertices(&mut node);
                    }
                    match position {
                        Some(p) => keys[p].value = node,
                        None => keys.push(new_entry(key, node)),
                    }
                }
            }
            edited = true;
        }
        if edited {
            let keys: Vec<String> = set
                .keys()
                .filter(|k| from_row.is_none() || k.as_str() != "contents")
                .cloned()
                .collect();
            self.touch(list, index, operation, Touch::Keys(keys));
        }
        Ok(Changed { edited, from_row })
    }

    /// Moves the object of `list` at `index` by `by`, in the declared unit
    /// `units`, as operation `operation`: every point it is drawn through
    /// moves, and a coordinate along an axis it does not move along is
    /// left as written.
    pub(crate) fn move_by(
        &mut self,
        list: ObjectList,
        index: usize,
        by: [f64; 3],
        units: Unit,
        operation: usize,
    ) -> Result<(), Error> {
        self.state(list);
        let path = list.place(index);
        let items = self.items_mut(list)?;
        let item = &mut items[index];
        let kind = object_parts(item).0.unwrap_or_default().to_owned();
        let keys = object_keys_mut(item).ok_or_else(|| record_shape(&kind))?;
        let object_path = key_path(&path, &kind);
        let mut moved = Vec::new();
        for entry in keys.iter_mut() {
            let field = key_path(&object_path, &entry.key);
            match (kind.as_str(), entry.key.as_str()) {
                ("line", "from" | "to")
                | ("arc" | "circle" | "ellipse", "center")
                | ("text" | "mtext" | "insert", "position") => {
                    move_point(&mut entry.value, by, units, &field)?;
                }
                ("donut", "center") => move_point(&mut entry.value, by, units, &field)?,
                ("polyline", "points")
                | ("spline", "fit_points" | "control_points")
                | ("solid", "corners") => {
                    move_points(&mut entry.value, by, units, &field)?;
                }
                ("hatch", "boundary") => move_boundary(&mut entry.value, by, units, &field)?,
                _ => continue,
            }
            moved.push(entry.key.clone());
        }
        self.touch(list, index, operation, Touch::Keys(moved));
        Ok(())
    }

    /// Sets the attribute `tag` of the insert of `list` at `index` to
    /// `value`, as operation `operation`. An attribute the template takes
    /// from the row is not set here: the row's value is returned, for the
    /// caller to set in the sheet's row.
    pub(crate) fn set_attribute(
        &mut self,
        list: ObjectList,
        index: usize,
        tag: &str,
        value: &str,
        operation: usize,
    ) -> Result<AttributeSet, Error> {
        self.state(list);
        let items = self.items_mut(list)?;
        let item = &mut items[index];
        let kind = object_parts(item).0.unwrap_or_default().to_owned();
        if kind != "insert" {
            return Err(Error::new(
                ErrorCode::BadValue,
                format!(
                    "the object is {} {kind}, and only an insert has attributes",
                    article(&kind)
                ),
            )
            .at_field("target"));
        }
        let keys = object_keys_mut(item).ok_or_else(|| record_shape(&kind))?;
        let attributes = match keys.iter().position(|e| e.key == "attributes") {
            Some(p) => &mut keys[p].value,
            None => {
                keys.push(new_entry(
                    "attributes",
                    Node::new(Kind::Mapping(Vec::new())),
                ));
                &mut keys.last_mut().expect("just pushed").value
            }
        };
        let Kind::Mapping(values) = &mut attributes.kind else {
            return Err(record_shape(&kind));
        };
        match values.iter().position(|e| e.key == tag) {
            Some(p) => {
                if let Some(name) = row_value_name(&values[p].value) {
                    return Ok(AttributeSet::FromRow(name));
                }
                values[p].value = Node::scalar(value);
            }
            None => values.push(new_entry(tag, Node::scalar(value))),
        }
        let key = key_path("attributes", tag);
        self.touch(list, index, operation, Touch::Keys(vec![key]));
        Ok(AttributeSet::Written)
    }

    /// The text of row `row`'s value `name`, if the row gives it.
    pub(crate) fn row_value(&self, row: usize, name: &str) -> Option<String> {
        let values = mapping_entry(self.row_node(row)?, "values")?;
        let Kind::Mapping(entries) = &values.kind else {
            return None;
        };
        entries
            .iter()
            .find(|e| e.key == name && !e.value.is_bare_null())
            .and_then(|e| match &e.value.kind {
                Kind::Scalar(text) => Some(text.clone()),
                _ => None,
            })
    }

    /// Sets row `row`'s value `name`, which it gives, to `text`.
    pub(crate) fn set_row_value(
        &mut self,
        row: usize,
        name: &str,
        text: &str,
    ) -> Result<(), Error> {
        let field = key_path(&key_path(&index_path("rows", row), "values"), name);
        let missing = || {
            Error::new(
                ErrorCode::MissingValue,
                format!("the row gives no value `{name}` to set"),
            )
            .at_field(field.clone())
        };
        let first = !self.values.iter().any(|v| v.row == row && v.name == name);
        if first {
            let values = self
                .row_node(row)
                .and_then(|node| mapping_entry(node, "values"))
                .ok_or_else(missing)?;
            let in_flow = values.source.is_some_and(|s| s.style == Style::Flow);
            let Kind::Mapping(entries) = &values.kind else {
                return Err(missing());
            };
            let entry = entries.iter().find(|e| e.key == name).ok_or_else(missing)?;
            let source = entry.value.source.ok_or_else(missing)?;
            let edit = ValueEdit {
                row,
                name: name.to_owned(),
                source,
                key_end: entry.key_end,
                key_column: entry.key_at.column.saturating_sub(1),
                in_flow,
            };
            self.values.push(edit);
        }
        let values = self
            .row_node_mut(row)
            .and_then(|node| mapping_entry_mut(node, "values"))
            .ok_or_else(missing)?;
        let Kind::Mapping(entries) = &mut values.kind else {
            return Err(missing());
        };
        let entry = entries
            .iter_mut()
            .find(|e| e.key == name)
            .ok_or_else(missing)?;
        entry.value = Node::scalar(text);
        Ok(())
    }

    /// The places among the template's objects where it puts the row's
    /// value `name`: each object's place in the list and where in it, such
    /// as `attributes.TITLE` or `contents`.
    pub(crate) fn row_value_places(&self, name: &str) -> Vec<(usize, String)> {
        let mut places = Vec::new();
        let Some(items) = self.items(ObjectList::Template) else {
            return places;
        };
        for (i, item) in items.iter().enumerate() {
            let mut item = item.clone();
            let _ = super::template::places_in(&mut item, "", &mut |node, place| {
                if row_value_name(node).as_deref() == Some(name) {
                    let within = place.split_once('.').map_or("", |(_, rest)| rest);
                    places.push((i, within.to_owned()));
                }
                Ok(())
            });
        }
        places
    }

    /// The file's new text, with every edit made, or `None` when nothing
    /// was edited. The text is read back and must read as the edits made
    /// it.
    pub(crate) fn new_text(&self) -> Result<Option<String>, Error> {
        if !self.changed() {
            return Ok(None);
        }
        let mut splices = self.splices()?;
        splices.sort_by_key(|s| std::cmp::Reverse((s.0, s.1)));
        let mut text = self.text.clone();
        let mut floor = usize::MAX;
        for (start, end, with) in splices {
            if end > floor {
                return Err(self.cannot_write("two edits would rewrite the same text"));
            }
            text.replace_range(start..end, &with);
            floor = start;
        }
        match strict_yaml::read(&text) {
            Ok(Some(back)) if back.same_as(&self.root) => {}
            _ => {
                return Err(self.cannot_write(
                    "the text it would write does not read back as the edit made it",
                ))
            }
        }
        if self.bom {
            text.insert(0, '\u{FEFF}');
        }
        Ok(Some(text))
    }

    fn cannot_write(&self, why: &str) -> Error {
        Error::new(
            ErrorCode::WriteFailed,
            format!(
                "could not rewrite the record in place: {why}; nothing was written, so write the \
                 objects of this file as a list, one to an item, and try again"
            ),
        )
        .in_file(&self.path)
    }

    fn emitter(&self) -> Emitter<'_> {
        Emitter {
            text: &self.text,
            newline: self.newline,
        }
    }

    /// Notes that operation `operation` set `what` in the object of `list`
    /// at `index`.
    fn touch(&mut self, list: ObjectList, index: usize, operation: usize, what: Touch) {
        let item = &mut self.list_state(list).items[index];
        item.edited = true;
        item.touches.push((operation, what));
    }

    /// Starts keeping track of `list`, before its first edit.
    fn state(&mut self, list: ObjectList) {
        if self.lists.iter().any(|(l, _)| *l == list) {
            return;
        }
        let (existed, sources) = match self.items(list) {
            Some(items) => (true, items.iter().map(|i| i.source).collect::<Vec<_>>()),
            None => (false, Vec::new()),
        };
        let items = (0..sources.len())
            .map(|i| Item {
                origin: Some(i),
                edited: false,
                touches: Vec::new(),
            })
            .collect();
        self.lists.push((
            list,
            ListState {
                existed,
                sources,
                items,
            },
        ));
    }

    fn list_state(&mut self, list: ObjectList) -> &mut ListState {
        &mut self
            .lists
            .iter_mut()
            .find(|(l, _)| *l == list)
            .expect("a list is tracked before it is edited")
            .1
    }

    fn row_node(&self, row: usize) -> Option<&Node> {
        let Kind::Sequence(rows) = &mapping_entry(&self.root, "rows")?.kind else {
            return None;
        };
        rows.get(row)
    }

    fn row_node_mut(&mut self, row: usize) -> Option<&mut Node> {
        let Kind::Sequence(rows) = &mut mapping_entry_mut(&mut self.root, "rows")?.kind else {
            return None;
        };
        rows.get_mut(row)
    }

    /// The list's objects, or `None` when the file does not write the list.
    fn items(&self, list: ObjectList) -> Option<&[Node]> {
        let parent = match list {
            ObjectList::Template => &self.root,
            ObjectList::Row(row) => self.row_node(row)?,
        };
        match &mapping_entry(parent, "objects")?.kind {
            Kind::Sequence(items) => Some(items),
            _ => None,
        }
    }

    /// The list's objects, to edit. A row that has no objects is given an
    /// empty list.
    fn items_mut(&mut self, list: ObjectList) -> Result<&mut Vec<Node>, Error> {
        let path = self.path.clone();
        let parent = match list {
            ObjectList::Template => &mut self.root,
            ObjectList::Row(row) => self
                .row_node_mut(row)
                .ok_or_else(|| record_shape_in("a row", &path))?,
        };
        let Kind::Mapping(entries) = &mut parent.kind else {
            return Err(record_shape_in("a mapping", &path));
        };
        if !entries.iter().any(|e| e.key == "objects") {
            entries.push(new_entry("objects", Node::new(Kind::Sequence(Vec::new()))));
        }
        let entry = entries
            .iter_mut()
            .find(|e| e.key == "objects")
            .expect("just made");
        match &mut entry.value.kind {
            Kind::Sequence(items) => Ok(items),
            _ => Err(record_shape_in("a list of objects", &path)),
        }
    }

    /// Where the text changes: each a range of bytes and what replaces it.
    fn splices(&self) -> Result<Vec<(usize, usize, String)>, Error> {
        let mut splices: Vec<(usize, usize, String)> = Vec::new();
        // Tables are taken in the order the file holds them, so that two
        // new ones inserted at one place go in that order.
        let mut tables: Vec<&String> = self.tables.iter().collect();
        if let Kind::Mapping(root) = &self.root.kind {
            tables.sort_by_key(|t| root.iter().position(|e| &e.key == *t));
        }
        for table in tables {
            let (start, end, with) = self.table_splice(table)?;
            match splices
                .iter_mut()
                .find(|s| s.0 == start && s.1 == end && start == end)
            {
                Some(same) => same.2.push_str(&with),
                None => splices.push((start, end, with)),
            }
        }
        for (list, state) in &self.lists {
            if state.dirty() {
                self.list_splices(*list, state, &mut splices)?;
            }
        }
        self.block_splices(&mut splices)?;
        for edit in &self.values {
            let start = if edit.source.style == Style::BlockScalar {
                value_start(&self.text, edit.key_end).unwrap_or(edit.source.start)
            } else {
                edit.source.start
            };
            let text = self
                .row_value(edit.row, &edit.name)
                .expect("a value set is in its row");
            let context = if edit.in_flow {
                Context::Flow
            } else {
                Context::Block
            };
            let with = scalar_text(&text, context, edit.key_column + 2, self.newline);
            splices.push((start, edit.source.end, with));
        }
        Ok(splices)
    }

    fn list_splices(
        &self,
        list: ObjectList,
        state: &ListState,
        splices: &mut Vec<(usize, usize, String)>,
    ) -> Result<(), Error> {
        let parent = match list {
            ObjectList::Template => &self.root,
            ObjectList::Row(row) => self.row_node(row).expect("a row edited is in the file"),
        };
        let Kind::Mapping(parent_entries) = &parent.kind else {
            return Err(record_shape_in("a mapping", &self.path));
        };
        let parent_flow = parent.source.is_some_and(|s| s.style == Style::Flow);
        let items = self.items(list).unwrap_or_default();
        let nl = self.newline;
        let emitter = self.emitter();
        let entry = parent_entries.iter().find(|e| e.key == "objects");
        let entry_source = entry.and_then(|e| e.value.source);

        // A row that had no objects gains `objects:`.
        if !state.existed || entry_source.is_none() {
            if items.is_empty() {
                return Ok(());
            }
            let Some(parent_source) = parent.source else {
                return Err(self.cannot_write("the row is not where the file wrote it"));
            };
            let list_node = &entry.expect("the list was made").value;
            if parent_flow {
                splices.push((
                    parent_source.end - 1,
                    parent_source.end - 1,
                    format!(", objects: {}", emitter.flow(list_node)),
                ));
            } else {
                let column = parent_entries
                    .first()
                    .map_or(0, |e| e.key_at.column.saturating_sub(1));
                let at = line_end(&self.text, parent_source.end);
                let mut with = String::new();
                if at == self.text.len() && !self.text.ends_with('\n') {
                    with.push_str(nl);
                }
                with.push_str(&" ".repeat(column));
                with.push_str("objects:");
                with.push_str(nl);
                with.push_str(&block_items(&emitter, items, column + 2, nl));
                with.push_str(nl);
                splices.push((at, at, with));
            }
            return Ok(());
        }
        let entry = entry.expect("the list is in the file");
        let source = entry_source.expect("the list is in the file");
        let key_column = entry.key_at.column.saturating_sub(1);

        // A list in brackets is written again whole.
        if source.style == Style::Flow {
            if parent_flow {
                splices.push((source.start, source.end, emitter.flow(&entry.value)));
            } else if items.is_empty() {
                splices.push((entry.key_end, source.end, ": []".to_owned()));
            } else {
                let with = format!(":{nl}{}", block_items(&emitter, items, key_column + 2, nl));
                splices.push((entry.key_end, source.end, with));
            }
            return Ok(());
        }

        // A list an item to a line: each object rewritten, taken away or
        // added on lines of its own.
        if items.is_empty() {
            splices.push((entry.key_end, source.end, ": []".to_owned()));
            return Ok(());
        }
        let mut column = None;
        let mut ranges = Vec::with_capacity(state.sources.len());
        for item_source in &state.sources {
            let item_source =
                item_source.ok_or_else(|| self.cannot_write("an object has no place"))?;
            let dash = dash_before(&self.text, item_source.start)
                .ok_or_else(|| self.cannot_write("an object is not an item of its list"))?;
            let start = line_start(&self.text, dash);
            if !self.text[start..dash].bytes().all(|b| b == b' ') {
                return Err(self.cannot_write("an object shares its line with another"));
            }
            column.get_or_insert(dash - start);
            ranges.push((start, line_end(&self.text, item_source.end)));
        }
        let column = column.unwrap_or(key_column + 2);
        let item_text = |node: &Node| {
            format!(
                "{}- {}{nl}",
                " ".repeat(column),
                emitter.item(node, column + 2)
            )
        };
        let mut kept = vec![false; ranges.len()];
        for (node, item) in items.iter().zip(&state.items) {
            if let Some(origin) = item.origin {
                kept[origin] = true;
                if item.edited {
                    let (start, end) = ranges[origin];
                    let mut with = item_text(node);
                    if end == self.text.len() && !self.text.ends_with('\n') {
                        with.truncate(with.len() - nl.len());
                    }
                    splices.push((start, end, with));
                }
            }
        }
        for (origin, kept) in kept.iter().enumerate() {
            if !kept {
                let (start, end) = ranges[origin];
                splices.push((start, end, String::new()));
            }
        }
        let added: String = items
            .iter()
            .zip(&state.items)
            .filter(|(_, item)| item.origin.is_none())
            .map(|(node, _)| item_text(node))
            .collect();
        if !added.is_empty() {
            let at = ranges.last().map_or(source.end, |r| r.1);
            let mut with = String::new();
            if at == self.text.len() && !self.text.ends_with('\n') {
                with.push_str(nl);
            }
            with.push_str(&added);
            splices.push((at, at, with));
        }
        Ok(())
    }
}

impl ListState {
    /// Whether any object of the list was edited, added or taken away.
    fn dirty(&self) -> bool {
        self.items.len() != self.sources.len()
            || self
                .items
                .iter()
                .enumerate()
                .any(|(i, item)| item.edited || item.origin != Some(i))
    }
}

/// The key a block library writes its blocks under.
const BLOCKS_KEY: &str = "blocks";

/// The objects of the block `block`, as a block library writes them; none
/// when it writes no list.
fn block_items_of(block: &Node) -> &[Node] {
    match mapping_entry(block, "objects").map(|o| &o.kind) {
        Some(Kind::Sequence(items)) => items,
        _ => &[],
    }
}

/// Takes the value an insert's `keys` give for the attribute `tag` away,
/// and the `attributes` with it when no value is left.
fn remove_attribute_value(keys: &mut Vec<Entry>, tag: &str) {
    let Some(p) = keys.iter().position(|e| e.key == "attributes") else {
        return;
    };
    if let Kind::Mapping(values) = &mut keys[p].value.kind {
        values.retain(|v| v.key != tag);
        if values.is_empty() {
            keys.remove(p);
        }
    }
}

/// The node a block's object sent as JSON writes: a mapping of one key, its
/// type, whose keys are written in the order the type lists them, then its
/// ID and its name.
fn block_object(value: &Value) -> Node {
    let Value::Object(object) = value else {
        return node_of(value);
    };
    let [(kind, Value::Object(keys))] = object.iter().collect::<Vec<_>>()[..] else {
        return node_of(value);
    };
    let order: Vec<&str> = BLOCK_OBJECT_TYPES
        .iter()
        .find(|(name, _)| name == kind)
        .map(|(_, keys)| keys.iter().copied().chain([ID_KEY, NAME_KEY]).collect())
        .unwrap_or_default();
    let mut ordered: Vec<(&String, &Value)> = keys.iter().collect();
    ordered.sort_by_key(|(key, _)| order.iter().position(|k| k == key).unwrap_or(order.len()));
    let entries = ordered
        .into_iter()
        .map(|(key, value)| {
            let mut node = node_of(value);
            if kind == "polyline" && key == "points" {
                order_vertices(&mut node);
            }
            new_entry(key, node)
        })
        .collect();
    Node::new(Kind::Mapping(vec![new_entry(
        kind,
        Node::new(Kind::Mapping(entries)),
    )]))
}

/// `items` written an item to a line, each `- ` at `column`, with line
/// breaks between them and none at the end.
fn block_items(emitter: &Emitter, items: &[Node], column: usize, nl: &str) -> String {
    items
        .iter()
        .map(|node| format!("{}- {}", " ".repeat(column), emitter.item(node, column + 2)))
        .collect::<Vec<_>>()
        .join(nl)
}

/// The start of the line `at` is on.
fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map_or(0, |i| i + 1)
}

/// Just after the line break that ends the line `at` is on, or the end of
/// the text.
fn line_end(text: &str, at: usize) -> usize {
    text[at..].find('\n').map_or(text.len(), |i| at + i + 1)
}

/// The `-` of the list item that starts at `start`: the first character
/// before it that is not a space or a line break.
fn dash_before(text: &str, start: usize) -> Option<usize> {
    let before = text[..start].trim_end_matches([' ', '\t', '\n', '\r']);
    before.ends_with('-').then(|| before.len() - 1)
}

/// Where the value of the key that ends at `key_end` starts: after its `:`
/// and any spaces, such as at a block scalar's `|`.
fn value_start(text: &str, key_end: usize) -> Option<usize> {
    let rest = &text[key_end..];
    let after_spaces = rest.trim_start_matches([' ', '\t']);
    let after_colon = after_spaces.strip_prefix(':')?;
    let value = after_colon.trim_start_matches([' ', '\t']);
    Some(text.len() - value.len())
}

/// The value of `key` in the mapping `node`.
fn mapping_entry<'a>(node: &'a Node, key: &str) -> Option<&'a Node> {
    let Kind::Mapping(entries) = &node.kind else {
        return None;
    };
    entries.iter().find(|e| e.key == key).map(|e| &e.value)
}

fn mapping_entry_mut<'a>(node: &'a mut Node, key: &str) -> Option<&'a mut Node> {
    let Kind::Mapping(entries) = &mut node.kind else {
        return None;
    };
    entries
        .iter_mut()
        .find(|e| e.key == key)
        .map(|e| &mut e.value)
}

/// An object's type and its keys: `line` and `{from: ..., to: ...}`.
fn object_parts(item: &Node) -> (Option<&str>, Option<&[Entry]>) {
    let Kind::Mapping(types) = &item.kind else {
        return (None, None);
    };
    let [object] = types.as_slice() else {
        return (None, None);
    };
    match &object.value.kind {
        Kind::Mapping(keys) => (Some(&object.key), Some(keys)),
        _ => (Some(&object.key), None),
    }
}

fn object_keys_mut(item: &mut Node) -> Option<&mut Vec<Entry>> {
    let Kind::Mapping(types) = &mut item.kind else {
        return None;
    };
    let [object] = types.as_mut_slice() else {
        return None;
    };
    match &mut object.value.kind {
        Kind::Mapping(keys) => Some(keys),
        _ => None,
    }
}

fn scalar_of<'a>(keys: &'a [Entry], key: &str) -> Option<&'a str> {
    keys.iter()
        .find(|e| e.key == key && !e.value.is_bare_null())
        .and_then(|e| match &e.value.kind {
            Kind::Scalar(text) => Some(text.as_str()),
            _ => None,
        })
}

/// The name of the row's value a template puts at `node`, if it is
/// `{row: NAME}`.
fn row_value_name(node: &Node) -> Option<String> {
    let Kind::Mapping(entries) = &node.kind else {
        return None;
    };
    match entries.as_slice() {
        [entry] if entry.key == ROW_KEY => match &entry.value.kind {
            Kind::Scalar(name) => Some(name.clone()),
            _ => None,
        },
        _ => None,
    }
}

fn new_entry(key: &str, value: Node) -> Entry {
    Entry {
        key: key.to_owned(),
        key_at: super::strict_yaml::Pos { line: 0, column: 0 },
        key_end: 0,
        value,
    }
}

/// The node a JSON value writes: text for a string, a number or `true` and
/// `false`, a list for an array, a mapping for an object, and a bare null
/// for `null`, which the file's reader then reads as it reads its own
/// (rule 8 of the YAML subset): an optional text or path field so written
/// is left out, and a required one, or any other, is an error.
fn node_of(value: &Value) -> Node {
    match value {
        Value::String(text) => Node::scalar(text.clone()),
        Value::Number(number) => Node::scalar(number.to_string()),
        Value::Bool(b) => Node::scalar(b.to_string()),
        Value::Null => Node::bare_null(),
        Value::Array(items) => Node::new(Kind::Sequence(items.iter().map(node_of).collect())),
        Value::Object(map) => Node::new(Kind::Mapping(
            map.iter()
                .map(|(key, item)| new_entry(key, node_of(item)))
                .collect(),
        )),
    }
}

/// Writes the keys of each of a polyline's points that is a mapping in the
/// order a sheet description lists them: its point first.
fn order_vertices(points: &mut Node) {
    let Kind::Sequence(items) = &mut points.kind else {
        return;
    };
    for item in items {
        if let Kind::Mapping(keys) = &mut item.kind {
            keys.sort_by_key(|k| {
                VERTEX_KEYS
                    .iter()
                    .position(|v| *v == k.key)
                    .unwrap_or(VERTEX_KEYS.len())
            });
        }
    }
}

fn move_point(node: &mut Node, by: [f64; 3], units: Unit, field: &str) -> Result<(), Error> {
    let Kind::Sequence(coordinates) = &mut node.kind else {
        return Err(not_a_point(field));
    };
    if coordinates.len() != 3 {
        return Err(not_a_point(field));
    }
    for (axis, coordinate) in coordinates.iter_mut().enumerate() {
        if by[axis] == 0.0 {
            continue;
        }
        let path = index_path(field, axis);
        let Kind::Scalar(text) = &coordinate.kind else {
            return Err(not_a_point(field));
        };
        let value = match parse_length(text) {
            Ok((value, None)) => value,
            Ok((value, Some(own))) => own.convert(value, units),
            Err(
                LengthError::NotANumber | LengthError::OutOfRange | LengthError::UnknownUnit(_),
            ) => {
                return Err(Error::new(
                    ErrorCode::BadNumber,
                    format!("the record's coordinate `{text}` is not a length to move"),
                )
                .at_field(path))
            }
        };
        let moved = value + by[axis];
        if !moved.is_finite() {
            return Err(Error::new(
                ErrorCode::BadNumber,
                "the coordinate would be too large once moved",
            )
            .at_field(path));
        }
        *coordinate = Node::scalar(number_text(moved));
    }
    Ok(())
}

fn move_points(node: &mut Node, by: [f64; 3], units: Unit, field: &str) -> Result<(), Error> {
    let Kind::Sequence(points) = &mut node.kind else {
        return Err(not_a_point(field));
    };
    for (i, point) in points.iter_mut().enumerate() {
        let at = index_path(field, i);
        // A polyline's point may be a mapping that gives its point.
        if let Kind::Mapping(keys) = &mut point.kind {
            for key in keys.iter_mut().filter(|k| k.key == "point") {
                move_point(&mut key.value, by, units, &key_path(&at, "point"))?;
            }
        } else {
            move_point(point, by, units, &at)?;
        }
    }
    Ok(())
}

/// Moves each loop of a hatch's boundary: a polyline's points, and each
/// edge's ends and centre.
fn move_boundary(node: &mut Node, by: [f64; 3], units: Unit, field: &str) -> Result<(), Error> {
    let Kind::Sequence(loops) = &mut node.kind else {
        return Err(not_a_point(field));
    };
    for (i, boundary_loop) in loops.iter_mut().enumerate() {
        let loop_path = index_path(field, i);
        let Kind::Mapping(kinds) = &mut boundary_loop.kind else {
            continue;
        };
        for kind in kinds.iter_mut() {
            let kind_path = key_path(&loop_path, &kind.key);
            match (kind.key.as_str(), &mut kind.value.kind) {
                ("polyline", Kind::Mapping(keys)) => {
                    for key in keys.iter_mut().filter(|k| k.key == "points") {
                        move_points(&mut key.value, by, units, &key_path(&kind_path, "points"))?;
                    }
                }
                ("edges", Kind::Sequence(edges)) => {
                    for (j, edge) in edges.iter_mut().enumerate() {
                        let edge_path = index_path(&kind_path, j);
                        let Kind::Mapping(edge_kinds) = &mut edge.kind else {
                            continue;
                        };
                        for edge_kind in edge_kinds.iter_mut() {
                            let Kind::Mapping(keys) = &mut edge_kind.value.kind else {
                                continue;
                            };
                            for key in keys.iter_mut() {
                                if matches!(key.key.as_str(), "from" | "to" | "center") {
                                    let path =
                                        key_path(&key_path(&edge_path, &edge_kind.key), &key.key);
                                    move_point(&mut key.value, by, units, &path)?;
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn not_a_point(field: &str) -> Error {
    Error::new(
        ErrorCode::WrongType,
        "the record's point here is not a list of three lengths, so the object cannot be moved",
    )
    .at_field(field)
}

fn record_shape(kind: &str) -> Error {
    Error::new(
        ErrorCode::WrongType,
        format!(
            "the record does not write this {kind} as a mapping of its keys, so it cannot be \
             edited"
        ),
    )
}

fn record_shape_in(what: &str, path: &Path) -> Error {
    Error::new(
        ErrorCode::WrongType,
        format!("the record does not hold {what} where an edit needs one"),
    )
    .in_file(path)
}
