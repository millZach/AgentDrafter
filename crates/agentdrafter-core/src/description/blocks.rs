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
    Attribute, AttributeDefinition, Block, BlockLibrary, Insert, Layer, Linetype, Object, Point3,
    Sheet, CONTINUOUS,
};
use crate::text::TextStyle;
use crate::units::Unit;

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

/// The keys an attribute definition takes.
pub(crate) const ATTDEF_KEYS: [&str; 11] = [
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
    "layer",
];

/// The keys an insert takes, besides those every object on a sheet takes.
pub(crate) const INSERT_KEYS: [&str; 6] = [
    "block",
    "position",
    "scale",
    "rotation",
    "attributes",
    "layer",
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
pub fn parse_block_library(text: &str) -> Result<BlockLibrary, Error> {
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
        layers: &layers,
        hatch_patterns: &hatch_patterns,
        styles: &text_styles,
    };
    let entries = named_entries(fields.require("blocks")?, "blocks", "block")?;
    let blocks = entries
        .iter()
        .map(|entry| block(&entry.key, &entry.value, &tables))
        .collect::<Result<_, _>>()?;
    Ok(BlockLibrary {
        units,
        linetypes,
        layers,
        text_styles,
        blocks,
    })
}

fn block(name: &str, node: &Node, tables: &Tables) -> Result<Block, Error> {
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
    let mut place = Place::Block { tags: Vec::new() };
    let objects = items
        .iter()
        .enumerate()
        .map(|(i, item)| object(item, &index_path(&objects_path, i), tables, &mut place))
        .collect::<Result<_, _>>()?;
    Ok(Block {
        name: name.to_owned(),
        base_point,
        objects,
    })
}

/// An attribute definition. `tags` holds the tags the block has defined
/// before it, each with where it was given.
pub(super) fn attribute_definition(
    fields: &Fields,
    tables: &Tables,
    layer: String,
    tags: &mut Vec<(String, Pos)>,
) -> Result<AttributeDefinition, Error> {
    let tag_node = fields.require("tag")?;
    let tag_path = fields.path_of("tag");
    let tag = scalar(tag_node, &tag_path, "an attribute tag")?;
    if let Some(problem) = tag_problem(tag) {
        return Err(bad_value(tag_node, &tag_path, problem));
    }
    if let Some((_, first)) = tags.iter().find(|(t, _)| t == tag) {
        return Err(at(
            Error::new(
                ErrorCode::DuplicateKey,
                format!(
                    "attribute tag `{tag}` is defined twice in this block, first at line {}; an insert fills in each attribute by its tag",
                    first.line
                ),
            ),
            &tag_path,
            tag_node.at,
        ));
    }
    tags.push((tag.to_owned(), tag_node.at));
    let prompt = match fields.get("prompt") {
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
    let default = match fields.get("default") {
        Some(node) => value(node, &fields.path_of("default"))?,
        None => String::new(),
    };
    let style = style_name(fields, tables.styles)?;
    let width_factor = match fields.get("width_factor") {
        Some(node) => width_factor(node, &fields.path_of("width_factor"))?,
        None => style_of(&style, tables.styles).map_or(1.0, |s| s.width_factor),
    };
    let invisible = match fields.get("invisible") {
        Some(node) => boolean(node, &fields.path_of("invisible"))?,
        None => false,
    };
    Ok(AttributeDefinition {
        tag: tag.to_owned(),
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
        layer,
    })
}

/// An insert of a block from `library`. Its name, which every object on a
/// sheet may carry, is read with its ID ([`super::ids`]).
pub(super) fn insert(
    fields: &Fields,
    units: Unit,
    layer: String,
    library: Option<&BlockLibrary>,
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
    let attributes = attributes(fields, block)?;
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
    })
}

/// The block an insert names, from the sheet's block library.
fn block_named<'a>(fields: &Fields, library: Option<&'a BlockLibrary>) -> Result<&'a Block, Error> {
    let node = fields.require("block")?;
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

/// The values of every attribute of `block`, in the order it defines them:
/// the insert's own, or the definition's default where it gives none.
fn attributes(fields: &Fields, block: &Block) -> Result<Vec<Attribute>, Error> {
    let definitions: Vec<&AttributeDefinition> = block.attribute_definitions().collect();
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
        let listed = if definitions.is_empty() {
            format!("block `{}` has no attributes", block.name)
        } else {
            format!(
                "block `{}` has the attributes {}",
                block.name,
                definitions
                    .iter()
                    .map(|d| d.tag.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        for entry in entries {
            let Some(i) = definitions.iter().position(|d| d.tag == entry.key) else {
                return Err(Error::new(
                    ErrorCode::UnknownName,
                    format!("unknown attribute `{}`; {listed}", entry.key),
                )
                .at_field(key_path(&path, &entry.key))
                .at_position(entry.key_at.line, entry.key_at.column));
            };
            values[i] = Some(value(&entry.value, &key_path(&path, &entry.key))?);
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
/// in capitals and with no spaces.
fn tag_problem(tag: &str) -> Option<String> {
    if tag.is_empty() {
        Some("an attribute tag cannot be empty".to_owned())
    } else if tag.chars().count() > LONGEST_TAG {
        Some(format!(
            "an attribute tag has at most {LONGEST_TAG} characters"
        ))
    } else if tag.chars().any(char::is_whitespace) {
        Some("an attribute tag cannot hold a space".to_owned())
    } else if let Some(c) = tag.chars().find(|c| c.is_control()) {
        Some(format!(
            "an attribute tag cannot hold the control character `{}`",
            c.escape_default()
        ))
    } else if tag.chars().any(char::is_lowercase) {
        Some(format!(
            "an attribute tag is written in capitals, as AutoCAD keeps it: `{}`",
            tag.to_uppercase()
        ))
    } else {
        None
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
    let inserted: Vec<&str> = sheet
        .objects
        .iter()
        .filter_map(|o| match o {
            Object::Insert(insert) => Some(insert.block.as_str()),
            _ => None,
        })
        .collect();
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
    let used_linetypes = library
        .linetypes
        .iter()
        .filter(|t| t.name != CONTINUOUS && used_layers.iter().any(|l| l.linetype == t.name));
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
