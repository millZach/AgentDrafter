//! Reads block libraries, the blocks in them with their attribute
//! definitions, and inserts of those blocks on a sheet.
//!
//! A block library is a file of its own, written in the same strict YAML as
//! a sheet description: a declared unit, the layers, linetypes, hatch
//! patterns and text styles its blocks use, and under `blocks` each block,
//! from its name to its base point and its objects. A block's objects are
//! the objects a sheet holds, and attribute definitions (`attdef`). A sheet
//! names its library under `block_library`, and its inserts name blocks
//! from it.
//!
//! A built sheet holds the blocks it inserts, and the library's layers,
//! linetypes and text styles those blocks use ([`take_from_library`]). A
//! name the sheet defines as well must be defined the same in both, so a
//! block draws in the DWG as its library defines it.

use super::layers::{layers, linetypes, named_entries};
use super::objects::{object, Place, Tables};
use super::strict_yaml::{self, index_path, key_path, Kind, Node, Pos};
use super::text::{character_code, height, justify, rotation, style_name, style_of, width_factor};
use super::{
    angle, at, bad_value, boolean, choice, hatch, plain_number, point, scalar, sequence, text,
    unit, wrong_type, Fields,
};
use crate::error::{Error, ErrorCode};
use crate::model::{
    Attribute, AttributeDefinition, Block, BlockLibrary, Cell, Insert, Layer, Linetype, Object,
    ObjectLinetype, Point3, Sheet, CONTINUOUS,
};
use crate::set::NoticeCode;
use crate::text::TextStyle;
use crate::units::Unit;

/// Something reading a sheet or a block library noticed that did not stop
/// it, such as an attribute tag written in lowercase, which is taken in
/// capitals as AutoCAD keeps it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Noticed {
    pub(super) code: NoticeCode,
    pub(super) message: String,
    /// The path to what it is about in its file, such as
    /// `blocks.B.objects[0].attdef.tag`.
    pub(super) field: String,
    pub(super) at: Pos,
    /// The file it is in, when it is not the file of the sheet's objects:
    /// the block library, or the rows file for a row's own object.
    pub(super) file: Option<std::path::PathBuf>,
}

/// The notice that `written`, an attribute tag written at `field`, is taken
/// in capitals, as `tag`.
fn in_capitals(written: &str, tag: &str, field: String, at: Pos) -> Noticed {
    Noticed {
        code: NoticeCode::TagInCapitals,
        message: format!(
            "the attribute tag `{written}` is taken as `{tag}`: AutoCAD keeps a tag in capitals, \
             and so does AgentDrafter"
        ),
        field,
        at,
        file: None,
    }
}

/// The keys at the top of a block library.
pub(crate) const LIBRARY_KEYS: [&str; 6] = [
    "units",
    "linetypes",
    "layers",
    "hatch_patterns",
    "text_styles",
    "blocks",
];

/// The keys a block takes.
pub(crate) const BLOCK_KEYS: [&str; 2] = ["base_point", "objects"];

/// The key a block's text or attribute definition declares its cell under.
pub(crate) const CELL_KEY: &str = "cell";

/// The keys an attribute definition takes.
pub(crate) const ATTDEF_KEYS: [&str; 20] = [
    "tag",
    "prompt",
    "default",
    "position",
    "height",
    "style",
    "width_factor",
    "rotation",
    "justify",
    "invisible",
    "constant",
    "verify",
    "preset",
    "lock_position",
    "layer",
    "color",
    "linetype",
    "lineweight",
    "linetype_scale",
    CELL_KEY,
];

/// The keys a single line of text takes in a block: a sheet's, and its
/// cell.
pub(crate) const BLOCK_TEXT_KEYS: [&str; 13] = [
    "contents",
    "position",
    "height",
    "style",
    "width_factor",
    "rotation",
    "justify",
    "layer",
    "color",
    "linetype",
    "lineweight",
    "linetype_scale",
    CELL_KEY,
];

/// The keys an insert takes, besides those every object on a sheet takes.
pub(crate) const INSERT_KEYS: [&str; 10] = [
    "block",
    "position",
    "scale",
    "rotation",
    "attributes",
    "layer",
    "color",
    "linetype",
    "lineweight",
    "linetype_scale",
];

/// The longest attribute tag AgentDrafter writes, in characters.
pub(crate) const LONGEST_TAG: usize = 255;

/// Characters a block library's path cannot hold, so that it names the same
/// file on Windows, Linux and macOS: `\` (folders are separated by `/`), the
/// drive separator `:`, and the others Windows refuses in a file name.
pub(crate) const FORBIDDEN_IN_PATHS: &str = "\\:<>\"|?*";

/// Reads the text of a block library into a [`BlockLibrary`].
///
/// Every length comes back in the library's declared unit; a number that
/// carries its own unit is converted to it.
///
/// An attribute tag written in lowercase is taken in capitals, as AutoCAD
/// keeps it.
pub fn parse_block_library(text: &str) -> Result<BlockLibrary, Error> {
    read_block_library(text).map(|(library, _)| library)
}

/// Reads a block library as [`parse_block_library`] does, with what the
/// reading noticed: each attribute tag written in lowercase and taken in
/// capitals, in the file's order.
pub(crate) fn read_block_library(text: &str) -> Result<(BlockLibrary, Vec<Noticed>), Error> {
    let Some(root) = strict_yaml::read(text)? else {
        return Err(Error::new(
            ErrorCode::MissingKey,
            "the block library is empty; it needs `units` and `blocks`",
        ));
    };
    let fields = Fields::of(&root, "", &LIBRARY_KEYS, "a block library")?;
    let units = match fields.get("units") {
        Some(node) => unit(node, "units")?,
        None => {
            return Err(Error::new(
                ErrorCode::MissingUnit,
                format!(
                    "the block library declares no unit; add `units:` with one of {}",
                    Unit::symbols()
                ),
            )
            .at_field("units")
            .at_position(root.at.line, root.at.column))
        }
    };
    let linetypes = match fields.get("linetypes") {
        Some(node) => linetypes(node, units)?,
        None => Vec::new(),
    };
    let layers = match fields.get("layers") {
        Some(node) => layers(node, &linetypes)?,
        None => Vec::new(),
    };
    let hatch_patterns = match fields.get("hatch_patterns") {
        Some(node) => hatch::hatch_patterns(node, units)?,
        None => Vec::new(),
    };
    let text_styles = match fields.get("text_styles") {
        Some(node) => text::text_styles(node)?,
        None => Vec::new(),
    };
    let tables = Tables {
        units,
        linetypes: &linetypes,
        layers: &layers,
        hatch_patterns: &hatch_patterns,
        styles: &text_styles,
    };
    let entries = named_entries(fields.require("blocks")?, "blocks", "block")?;
    // A block that inserts another is read after it, so that its insert can
    // take the other's attributes; then the blocks go back in the file's
    // order.
    let order = nesting_order(entries)?;
    let mut library = BlockLibrary {
        units,
        linetypes: linetypes.clone(),
        layers: layers.clone(),
        text_styles: text_styles.clone(),
        blocks: Vec::with_capacity(entries.len()),
    };
    let mut noticed = Vec::new();
    for i in order {
        let (read, found) = block(&entries[i].key, &entries[i].value, &tables, Some(&library))?;
        library.blocks.push(read);
        noticed.extend(found);
    }
    library
        .blocks
        .sort_by_key(|b| entries.iter().position(|e| e.key == b.name));
    noticed.sort_by_key(|n| (n.at.line, n.at.column));
    Ok((library, noticed))
}

/// The order to read the blocks of `entries` in: each block after every
/// block it inserts. A block that inserts itself, directly or through
/// others, is an error naming the round.
fn nesting_order(entries: &[strict_yaml::Entry]) -> Result<Vec<usize>, Error> {
    // The blocks each block's objects insert, where it inserts them.
    let mut inserts: Vec<Vec<(usize, Pos)>> = Vec::with_capacity(entries.len());
    for entry in entries {
        let mut found = Vec::new();
        for (name, at, place) in nested_names(&entry.value) {
            let Some(i) = entries.iter().position(|e| e.key == name) else {
                let names: Vec<&str> = entries.iter().map(|e| e.key.as_str()).collect();
                return Err(Error::new(
                    ErrorCode::UnknownName,
                    format!(
                        "unknown block `{name}`; the block library's blocks: {}",
                        names.join(", ")
                    ),
                )
                .at_field(key_path(
                    &index_path(&key_path(&key_path("blocks", &entry.key), "objects"), place),
                    "insert.block",
                ))
                .at_position(at.line, at.column));
            };
            found.push((i, at));
        }
        inserts.push(found);
    }
    let mut order = Vec::with_capacity(entries.len());
    // 0: not yet seen, 1: being read, 2: read.
    let mut state = vec![0_u8; entries.len()];
    fn visit(
        i: usize,
        entries: &[strict_yaml::Entry],
        inserts: &[Vec<(usize, Pos)>],
        state: &mut [u8],
        order: &mut Vec<usize>,
        round: &mut Vec<usize>,
    ) -> Result<(), Error> {
        match state[i] {
            2 => return Ok(()),
            1 => {
                let start = round.iter().position(|&r| r == i).unwrap_or(0);
                let mut names: Vec<&str> = round[start..]
                    .iter()
                    .map(|&r| entries[r].key.as_str())
                    .collect();
                names.push(&entries[i].key);
                let at = inserts[round[round.len() - 1]]
                    .iter()
                    .find(|(to, _)| *to == i)
                    .map(|(_, at)| *at)
                    .unwrap_or(entries[i].key_at);
                return Err(Error::new(
                    ErrorCode::BadValue,
                    format!(
                        "block `{}` inserts itself, through {}; a block cannot insert itself, \
                         directly or through others",
                        entries[i].key,
                        names.join(" to ")
                    ),
                )
                .at_field(key_path("blocks", &entries[round[round.len() - 1]].key))
                .at_position(at.line, at.column));
            }
            _ => {}
        }
        state[i] = 1;
        round.push(i);
        for &(to, _) in &inserts[i] {
            visit(to, entries, inserts, state, order, round)?;
        }
        round.pop();
        state[i] = 2;
        order.push(i);
        Ok(())
    }
    for i in 0..entries.len() {
        visit(
            i,
            entries,
            &inserts,
            &mut state,
            &mut order,
            &mut Vec::new(),
        )?;
    }
    Ok(order)
}

/// The block each `insert` among a block's objects names, where, and its
/// place among the objects, as the file writes it, read before the block
/// is.
fn nested_names(block: &Node) -> Vec<(String, Pos, usize)> {
    let Kind::Mapping(keys) = &block.kind else {
        return Vec::new();
    };
    let Some(objects) = keys.iter().find(|k| k.key == "objects") else {
        return Vec::new();
    };
    let Kind::Sequence(items) = &objects.value.kind else {
        return Vec::new();
    };
    items
        .iter()
        .enumerate()
        .filter_map(|(place, item)| {
            let Kind::Mapping(types) = &item.kind else {
                return None;
            };
            let [object] = types.as_slice() else {
                return None;
            };
            let Kind::Mapping(keys) = &object.value.kind else {
                return None;
            };
            if object.key != "insert" {
                return None;
            }
            let block = keys.iter().find(|k| k.key == "block")?;
            match &block.value.kind {
                Kind::Scalar(name) => Some((name.clone(), block.value.at, place)),
                _ => None,
            }
        })
        .collect()
}

fn block(
    name: &str,
    node: &Node,
    tables: &Tables,
    library: Option<&BlockLibrary>,
) -> Result<(Block, Vec<Noticed>), Error> {
    let path = key_path("blocks", name);
    let fields = Fields::of(node, &path, &BLOCK_KEYS, "a block")?;
    let base_point = match fields.get("base_point") {
        Some(node) => point(node, &fields.path_of("base_point"), tables.units)?,
        None => Point3::new(0.0, 0.0, 0.0),
    };
    let objects_path = fields.path_of("objects");
    let objects_node = fields.require("objects")?;
    let items = sequence(objects_node, &objects_path, "a list of objects")?;
    if items.is_empty() {
        return Err(at(
            Error::new(ErrorCode::WrongType, "a block holds at least one object"),
            &objects_path,
            objects_node.at,
        ));
    }
    let mut place = Place::Block {
        noticed: Vec::new(),
        library,
    };
    let mut objects = Vec::with_capacity(items.len());
    let mut cells = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let path = index_path(&objects_path, i);
        objects.push(object(item, &path, tables, &mut place)?);
        if let Some(cell) = cell(item, &path, i, tables.units)? {
            cells.push(cell);
        }
    }
    let Place::Block { noticed, .. } = place else {
        unreachable!("a block's objects are read in a block")
    };
    let block = Block {
        name: name.to_owned(),
        base_point,
        objects,
        cells,
    };
    Ok((block, noticed))
}

/// The cell the block's object at `item`, its `index`th, declares, if it
/// declares one: two opposite corners of a box in the block's own
/// coordinates, which only a text and an attribute definition take. The
/// object has been read, so its keys are its type's.
fn cell(item: &Node, path: &str, index: usize, units: Unit) -> Result<Option<Cell>, Error> {
    const SHAPE: &str = "a cell, two opposite corners of a box [[x, y, z], [x, y, z]]";
    let Kind::Mapping(types) = &item.kind else {
        return Ok(None);
    };
    let [object] = types.as_slice() else {
        return Ok(None);
    };
    let Kind::Mapping(keys) = &object.value.kind else {
        return Ok(None);
    };
    let Some(entry) = keys.iter().find(|k| k.key == CELL_KEY) else {
        return Ok(None);
    };
    let path = key_path(&key_path(path, &object.key), CELL_KEY);
    let node = &entry.value;
    let corners = sequence(node, &path, SHAPE)?;
    let [a, b] = corners else {
        return Err(at(
            Error::new(
                ErrorCode::WrongType,
                format!(
                    "a cell is two opposite corners of a box, [[x, y, z], [x, y, z]]; this one \
                     has {} points",
                    corners.len()
                ),
            ),
            &path,
            node.at,
        ));
    };
    let a = point(a, &index_path(&path, 0), units)?;
    let b = point(b, &index_path(&path, 1), units)?;
    if a.x == b.x || a.y == b.y {
        return Err(bad_value(
            node,
            &path,
            "a cell has a width and a height, so its corners differ in x and in y",
        ));
    }
    Ok(Some(Cell {
        object: index,
        min: [a.x.min(b.x), a.y.min(b.y)],
        max: [a.x.max(b.x), a.y.max(b.y)],
    }))
}

/// An attribute definition. A block may define a tag more than once, as
/// AutoCAD allows; an insert then gives that tag's values as a list. A tag
/// written in lowercase is taken in capitals, and `noticed` says so.
pub(super) fn attribute_definition(
    fields: &Fields,
    tables: &Tables,
    layer: String,
    noticed: &mut Vec<Noticed>,
) -> Result<AttributeDefinition, Error> {
    let tag_node = fields.require_text("tag")?;
    let tag_path = fields.path_of("tag");
    let written = scalar(tag_node, &tag_path, "an attribute tag")?;
    if let Some(problem) = tag_problem(written) {
        return Err(bad_value(tag_node, &tag_path, problem));
    }
    let tag = written.to_uppercase();
    if tag != written {
        noticed.push(in_capitals(written, &tag, tag_path.clone(), tag_node.at));
    }
    let prompt = match fields.text("prompt") {
        Some(node) => {
            let path = fields.path_of("prompt");
            let prompt = scalar(node, &path, "the prompt, one line of text")?;
            if let Some(problem) = one_line_problem(prompt, "a prompt") {
                return Err(bad_value(node, &path, problem));
            }
            prompt.to_owned()
        }
        None => String::new(),
    };
    let default = match fields.text("default") {
        Some(node) => value(node, &fields.path_of("default"))?,
        None => String::new(),
    };
    let style = style_name(fields, tables.styles)?;
    let width_factor = match fields.get("width_factor") {
        Some(node) => width_factor(node, &fields.path_of("width_factor"))?,
        None => style_of(&style, tables.styles).map_or(1.0, |s| s.width_factor),
    };
    let mode = |key: &str| match fields.get(key) {
        Some(node) => boolean(node, &fields.path_of(key)),
        None => Ok(false),
    };
    let invisible = mode("invisible")?;
    let constant = mode("constant")?;
    let verify = mode("verify")?;
    let preset = mode("preset")?;
    let lock_position = mode("lock_position")?;
    // A constant attribute asks nobody for its value, so Verify and Preset,
    // which change how AutoCAD asks, do not go with it: AutoCAD's ATTDEF
    // turns them off for a constant attribute.
    if constant {
        for key in ["verify", "preset"] {
            if let (Some(node), true) = (fields.get(key), mode(key)?) {
                return Err(bad_value(
                    node,
                    &fields.path_of(key),
                    format!(
                        "a constant attribute takes no `{key}`: its value is its `default` on \
                         every insert, and AutoCAD asks nobody for it, as its ATTDEF turns \
                         Verify and Preset off for a constant attribute"
                    ),
                ));
            }
        }
    }
    Ok(AttributeDefinition {
        tag,
        prompt,
        default,
        position: point(
            fields.require("position")?,
            &fields.path_of("position"),
            tables.units,
        )?,
        height: height(fields, tables.units)?,
        style,
        width_factor,
        rotation: rotation(fields)?,
        justify: justify(fields)?,
        invisible,
        constant,
        verify,
        preset,
        lock_position,
        layer,
        properties: Default::default(),
    })
}

/// An insert of a block from `library`. Its name, which every object on a
/// sheet may carry, is read with its ID ([`super::ids`]).
pub(super) fn insert(
    fields: &Fields,
    units: Unit,
    layer: String,
    library: Option<&BlockLibrary>,
    noticed: &mut Vec<Noticed>,
) -> Result<Insert, Error> {
    let block = block_named(fields, library)?;
    let scale = match fields.get("scale") {
        Some(node) => {
            let path = fields.path_of("scale");
            let scale = plain_number(node, &path, "a scale, a number such as `2`")?;
            if scale <= 0.0 {
                return Err(bad_value(
                    node,
                    &path,
                    "a scale is more than 0: how much the block is enlarged, the same along every axis",
                ));
            }
            scale
        }
        None => 1.0,
    };
    let rotation = match fields.get("rotation") {
        Some(node) => angle(node, &fields.path_of("rotation"))?,
        None => 0.0,
    };
    let attributes = attributes(fields, block, library, noticed)?;
    Ok(Insert {
        block: block.name.clone(),
        position: point(
            fields.require("position")?,
            &fields.path_of("position"),
            units,
        )?,
        scale,
        rotation,
        attributes,
        name: None,
        layer,
        properties: Default::default(),
    })
}

/// The block an insert names, from the sheet's block library.
fn block_named<'a>(fields: &Fields, library: Option<&'a BlockLibrary>) -> Result<&'a Block, Error> {
    let node = fields.require_text("block")?;
    let path = fields.path_of("block");
    let listed = match library {
        Some(library) if library.blocks.is_empty() => {
            "the block library defines no blocks".to_owned()
        }
        Some(library) => format!(
            "the block library's blocks: {}",
            library
                .blocks
                .iter()
                .map(|b| b.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        None => {
            "the sheet names no block library to insert blocks from; name one with `block_library:`"
                .to_owned()
        }
    };
    let name = choice(node, &path, "the name of a block", &listed)?;
    match library.and_then(|library| library.blocks.iter().find(|b| b.name == name)) {
        Some(block) => Ok(block),
        None => Err(at(
            Error::new(
                ErrorCode::UnknownName,
                format!("unknown block `{name}`; {listed}"),
            ),
            &path,
            node.at,
        )),
    }
}

/// The values of every attribute of `block` an insert carries, in the
/// order it defines them: the insert's own, or the definition's default
/// where it gives none.
///
/// A tag the block carries once takes one value; a tag it carries more
/// than once takes a list, one value for each in the order the block
/// defines them, which may be shorter than that but not longer. A bare
/// null, for the tag or for an item of its list, leaves the value out, so
/// the definition's default holds. A tag written in lowercase is taken in
/// capitals, and `noticed` says so.
fn attributes(
    fields: &Fields,
    block: &Block,
    library: Option<&BlockLibrary>,
    noticed: &mut Vec<Noticed>,
) -> Result<Vec<Attribute>, Error> {
    let definitions: Vec<&AttributeDefinition> = block.carried_attributes().collect();
    let mut values: Vec<Option<String>> = vec![None; definitions.len()];
    if let Some(node) = fields.get("attributes") {
        let path = fields.path_of("attributes");
        let Kind::Mapping(entries) = &node.kind else {
            return Err(wrong_type(
                node,
                &path,
                "a mapping from attribute tags to values",
            ));
        };
        let listed = carried_listed(block, &definitions);
        let mut given: Vec<(String, &strict_yaml::Entry)> = Vec::new();
        for entry in entries {
            let entry_path = key_path(&path, &entry.key);
            let tag = entry.key.to_uppercase();
            if let Some((_, first)) = given.iter().find(|(t, _)| *t == tag) {
                return Err(Error::new(
                    ErrorCode::DuplicateKey,
                    format!(
                        "the attribute `{tag}` is given twice, as `{}` at line {} and as `{}` \
                         here; AutoCAD keeps a tag in capitals, so they are one tag. Give it once",
                        first.key, first.key_at.line, entry.key
                    ),
                )
                .at_field(entry_path)
                .at_position(entry.key_at.line, entry.key_at.column));
            }
            given.push((tag.clone(), entry));
            if tag != entry.key {
                noticed.push(in_capitals(
                    &entry.key,
                    &tag,
                    entry_path.clone(),
                    entry.key_at,
                ));
            }
            let places: Vec<usize> = (0..definitions.len())
                .filter(|&i| definitions[i].tag == tag)
                .collect();
            if places.is_empty() {
                // A constant attribute's value is its definition's, on every
                // insert (Zach, 2026-11-13); and an attribute of a block this
                // block inserts is filled in by this block's definition,
                // once, as AutoCAD fills it. An insert sets neither.
                let constant = constant_tag(block, &tag, library);
                let why = match (constant, nested_tag(block, &tag, library)) {
                    (Some((definer, definition)), _) => {
                        let of = if definer == block.name {
                            format!("of block `{definer}`")
                        } else {
                            format!("of block `{definer}`, nested in block `{}`,", block.name)
                        };
                        format!(
                            "the attribute `{tag}` {of} is constant: its value, `{}`, is set once, as its `default` in block `{definer}`'s definition in the block library, as AutoCAD sets it, not by each insert. {listed}",
                            definition.default
                        )
                    }
                    (None, Some((setter, inner))) => {
                        let within = if setter == block.name {
                            String::new()
                        } else {
                            format!(", nested in block `{}`", block.name)
                        };
                        format!(
                            "the attribute `{tag}` belongs to block `{inner}`, which block `{setter}` inserts{within}; its value is set once, in block `{setter}`'s definition in the block library, as AutoCAD sets it, not by each insert. {listed}"
                        )
                    }
                    (None, None) => format!("unknown attribute `{}`; {listed}", entry.key),
                };
                return Err(Error::new(ErrorCode::UnknownName, why)
                    .at_field(entry_path)
                    .at_position(entry.key_at.line, entry.key_at.column));
            }
            // A bare null leaves the value out, so the definition's default holds.
            if entry.value.is_bare_null() {
                continue;
            }
            match (&entry.value.kind, places.as_slice()) {
                (Kind::Sequence(_), [_]) => {
                    return Err(wrong_type(
                        &entry.value,
                        &entry_path,
                        &format!(
                            "one value: block `{}` defines the attribute `{tag}` once, so it \
                             takes one line of text, not a list",
                            block.name
                        ),
                    ));
                }
                (_, [i]) => values[*i] = Some(value(&entry.value, &entry_path)?),
                (Kind::Sequence(items), _) => {
                    if items.len() > places.len() {
                        return Err(bad_value(
                            &entry.value,
                            &entry_path,
                            format!(
                                "block `{}` defines the attribute `{tag}` {} times, so it takes \
                                 at most {} values, one for each in the order the block defines \
                                 them; this list gives {}",
                                block.name,
                                places.len(),
                                places.len(),
                                items.len()
                            ),
                        ));
                    }
                    for (j, item) in items.iter().enumerate() {
                        if !item.is_bare_null() {
                            values[places[j]] = Some(value(item, &index_path(&entry_path, j))?);
                        }
                    }
                }
                (_, _) => {
                    return Err(wrong_type(
                        &entry.value,
                        &entry_path,
                        &format!(
                            "a list: block `{}` defines the attribute `{tag}` {} times, so it \
                             takes a list of values, one for each in the order the block \
                             defines them, such as [first, second]; a value left out, or \
                             written as null, takes its definition's default",
                            block.name,
                            places.len()
                        ),
                    ));
                }
            }
        }
    }
    Ok(definitions
        .iter()
        .zip(values)
        .map(|(definition, value)| Attribute {
            tag: definition.tag.clone(),
            value: value.unwrap_or_else(|| definition.default.clone()),
        })
        .collect())
}

/// The attributes an insert of `block` carries, for an error: each tag
/// once, saying how many times the block defines it when more than once.
fn carried_listed(block: &Block, definitions: &[&AttributeDefinition]) -> String {
    if definitions.is_empty() {
        return format!("block `{}` has no attributes", block.name);
    }
    let mut tags: Vec<(&str, usize)> = Vec::new();
    for definition in definitions {
        match tags.iter_mut().find(|(t, _)| *t == definition.tag) {
            Some((_, n)) => *n += 1,
            None => tags.push((&definition.tag, 1)),
        }
    }
    let each: Vec<String> = tags
        .iter()
        .map(|(tag, n)| match n {
            1 => (*tag).to_owned(),
            n => format!("{tag} ({n} times, given as a list)"),
        })
        .collect();
    format!(
        "block `{}` has the attributes {}",
        block.name,
        each.join(", ")
    )
}

/// The constant attribute `tag` of `block`, or of a block nested in it,
/// however deep, with the name of the block that defines it.
fn constant_tag<'a>(
    block: &'a Block,
    tag: &str,
    library: Option<&'a BlockLibrary>,
) -> Option<(&'a str, &'a AttributeDefinition)> {
    if let Some(definition) = block
        .attribute_definitions()
        .find(|d| d.constant && d.tag == tag)
    {
        return Some((block.name.as_str(), definition));
    }
    block.objects.iter().find_map(|object| {
        let Object::Insert(inner) = object else {
            return None;
        };
        // The reader refuses a block that inserts itself, so this ends.
        let nested = library?.blocks.iter().find(|b| b.name == inner.block)?;
        constant_tag(nested, tag, library)
    })
}

/// Where the attribute `tag` of a block nested in `block`, however deep, is
/// filled in: the block whose definition holds the nested insert that gives
/// its value, and the block that defines the tag. None when no nested
/// insert gives it.
fn nested_tag<'a>(
    block: &'a Block,
    tag: &str,
    library: Option<&'a BlockLibrary>,
) -> Option<(&'a str, &'a str)> {
    block.objects.iter().find_map(|object| {
        let Object::Insert(inner) = object else {
            return None;
        };
        if inner.attributes.iter().any(|a| a.tag == tag) {
            return Some((block.name.as_str(), inner.block.as_str()));
        }
        // The reader refuses a block that inserts itself, so this ends.
        let nested = library?.blocks.iter().find(|b| b.name == inner.block)?;
        nested_tag(nested, tag, library)
    })
}

/// An attribute's value: one line of text, which may be empty.
fn value(node: &Node, path: &str) -> Result<String, Error> {
    let text = scalar(node, path, "the attribute's value, one line of text")?;
    match one_line_problem(text, "an attribute's value").or_else(|| character_code(text)) {
        Some(problem) => Err(bad_value(node, path, problem)),
        None => Ok(text.to_owned()),
    }
}

/// What is wrong with text that is one line, if anything: a line break or
/// another control character.
fn one_line_problem(text: &str, what: &str) -> Option<String> {
    if text.contains(['\n', '\r']) {
        return Some(format!("{what} is one line"));
    }
    text.chars().find(|c| c.is_control()).map(|c| {
        format!(
            "{what} cannot hold the control character `{}`",
            c.escape_default()
        )
    })
}

/// Why `tag` cannot be an attribute tag, if it cannot. AutoCAD keeps a tag
/// with no spaces; one written in lowercase it takes in capitals.
fn tag_problem(tag: &str) -> Option<String> {
    if tag.is_empty() {
        Some("an attribute tag cannot be empty".to_owned())
    } else if tag.to_uppercase().chars().count() > LONGEST_TAG {
        // Counted in capitals, as the tag is kept.
        Some(format!(
            "an attribute tag has at most {LONGEST_TAG} characters"
        ))
    } else if tag.chars().any(char::is_whitespace) {
        Some("an attribute tag cannot hold a space".to_owned())
    } else {
        tag.chars().find(|c| c.is_control()).map(|c| {
            format!(
                "an attribute tag cannot hold the control character `{}`",
                c.escape_default()
            )
        })
    }
}

/// The path of a block library as a sheet writes it: relative to the
/// sheet description's folder, with `/` between folders, so that it names
/// the same file on every system.
pub(super) fn library_path<'a>(node: &'a Node, path: &str) -> Result<&'a str, Error> {
    relative_path(
        node,
        path,
        &RelativePath {
            shape: "the path of a block library file, such as `blocks.yaml`",
            whose: "the block library's path",
            from: "the sheet description's folder",
            names: "a file",
        },
    )
}

/// What a relative path is, for its errors.
pub(super) struct RelativePath<'a> {
    /// What belongs there, such as "the path of a block library file, such
    /// as `blocks.yaml`".
    pub shape: &'a str,
    /// Whose path it is, such as "the block library's path".
    pub whose: &'a str,
    /// The folder it is relative to, such as "the sheet description's folder".
    pub from: &'a str,
    /// What it names: "a file" or "a folder".
    pub names: &'a str,
}

/// A path written relative to a folder, with `/` between folders, so that
/// it names the same file or folder on every system: not empty, not
/// starting with `/`, holding none of [`FORBIDDEN_IN_PATHS`] and no control
/// characters, with one `/` between folders.
pub(super) fn relative_path<'a>(
    node: &'a Node,
    path: &str,
    what: &RelativePath,
) -> Result<&'a str, Error> {
    let RelativePath {
        shape,
        whose,
        from,
        names,
    } = what;
    let written = scalar(node, path, shape)?;
    let problem = if written.is_empty() {
        Some(format!("{whose} cannot be empty"))
    } else if written.starts_with('/') {
        Some(format!(
            "{whose} is relative to {from}, so it cannot start with `/`"
        ))
    } else if let Some(c) = written
        .chars()
        .find(|c| FORBIDDEN_IN_PATHS.contains(*c) || c.is_control())
    {
        Some(format!(
            "{whose} cannot hold `{}`: it is relative to {from}, with `/` between folders, and holds none of {FORBIDDEN_IN_PATHS}",
            c.escape_default()
        ))
    } else if written.split('/').any(str::is_empty) {
        Some(format!(
            "{whose} names {names}, with one `/` between folders"
        ))
    } else {
        None
    };
    match problem {
        Some(problem) => Err(bad_value(node, path, problem)),
        None => Ok(written),
    }
}

/// Adds to `sheet` the blocks its inserts place, as `library` defines them,
/// and the library's layers, linetypes and text styles those blocks use.
///
/// A layer, linetype or text style the sheet defines as well must be
/// defined the same in both, or the block would not draw in the DWG as the
/// library defines it. `fields` are the sheet description's, for the place
/// of such an error. `0`, `Continuous` and `Standard` the library does not
/// define are the drawing's own, as in AutoCAD.
pub(super) fn take_from_library(
    sheet: &mut Sheet,
    library: &BlockLibrary,
    fields: &Fields,
) -> Result<(), Error> {
    let mut inserted: Vec<&str> = sheet
        .objects
        .iter()
        .filter_map(|o| match o {
            Object::Insert(insert) => Some(insert.block.as_str()),
            _ => None,
        })
        .collect();
    // And every block those blocks insert, however deep.
    let mut i = 0;
    while i < inserted.len() {
        if let Some(block) = library.blocks.iter().find(|b| b.name == inserted[i]) {
            for object in &block.objects {
                if let Object::Insert(nested) = object {
                    if !inserted.contains(&nested.block.as_str()) {
                        inserted.push(&nested.block);
                    }
                }
            }
        }
        i += 1;
    }
    let blocks: Vec<Block> = library
        .blocks
        .iter()
        .filter(|b| inserted.contains(&b.name.as_str()))
        .cloned()
        .collect();
    let objects = || blocks.iter().flat_map(|b| &b.objects);
    let used_layers: Vec<&Layer> = library
        .layers
        .iter()
        .filter(|l| objects().any(|o| o.layer() == l.name))
        .collect();
    let used_linetypes = library.linetypes.iter().filter(|t| {
        t.name != CONTINUOUS
                && (used_layers.iter().any(|l| l.linetype == t.name)
                    || objects().any(|o| {
                        matches!(&o.properties().linetype, ObjectLinetype::Named(n) if *n == t.name)
                    }))
    });
    let used_styles = library.text_styles.iter().filter(|s| {
        objects().any(|o| match o {
            Object::Text(t) => t.style == s.name,
            Object::MText(t) => t.style == s.name,
            Object::AttributeDefinition(d) => d.style == s.name,
            _ => false,
        })
    });
    take(
        &mut sheet.linetypes,
        used_linetypes,
        |t: &Linetype| &t.name,
        ("linetype", "linetypes"),
        fields,
    )?;
    take(
        &mut sheet.layers,
        used_layers.into_iter(),
        |l: &Layer| &l.name,
        ("layer", "layers"),
        fields,
    )?;
    take(
        &mut sheet.text_styles,
        used_styles,
        |s: &TextStyle| &s.name,
        ("text style", "text_styles"),
        fields,
    )?;
    sheet.blocks = blocks;
    Ok(())
}

/// Adds each of `theirs` to `ours` unless `ours` defines it the same
/// already. `what` names one entry and the table it sits under.
fn take<'a, T: Clone + PartialEq + 'a>(
    ours: &mut Vec<T>,
    theirs: impl Iterator<Item = &'a T>,
    name: impl Fn(&T) -> &str,
    (what, table): (&str, &str),
    fields: &Fields,
) -> Result<(), Error> {
    for entry in theirs {
        let theirs_name = name(entry);
        let Some(our) = ours
            .iter()
            .find(|o| name(o).to_lowercase() == theirs_name.to_lowercase())
        else {
            ours.push(entry.clone());
            continue;
        };
        if our == entry {
            continue;
        }
        let our_name = name(our);
        let message = if our_name == theirs_name {
            format!(
                "{what} `{our_name}` is defined here and differently in the block library, whose blocks use it; define it the same in both, or only in the block library"
            )
        } else {
            format!(
                "{what} `{our_name}` is the block library's {what} `{theirs_name}` in another case, and AutoCAD ignores case in names; write it as the block library does, and define it the same"
            )
        };
        let error = Error::new(ErrorCode::BadValue, message);
        let place = fields.get(table).and_then(|node| match &node.kind {
            Kind::Mapping(entries) => entries.iter().find(|e| e.key == our_name),
            _ => None,
        });
        // Every entry of `ours` that is not the library's was listed by the
        // sheet, so its key is found.
        return Err(match place {
            Some(entry) => error
                .at_field(key_path(table, &entry.key))
                .at_position(entry.key_at.line, entry.key_at.column),
            None => error.at_field(table),
        });
    }
    Ok(())
}
