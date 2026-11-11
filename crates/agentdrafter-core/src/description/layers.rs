//! Reads the sheet's linetypes and layers.
//!
//! Both are mappings from a name to what it stands for, so a name is given
//! once. AutoCAD ignores case in these names, so two names that differ only
//! in case are refused as well.

use super::strict_yaml::{index_path, key_path, Entry, Kind, Node};
use super::{at, bad_value, length, scalar, sequence, wrong_type, Fields};
use crate::error::{Error, ErrorCode};
use crate::model::{Layer, Linetype, Lineweight, CONTINUOUS, LAYER_ZERO};
use crate::units::{parse_length, LengthError, Unit};

/// The keys a linetype takes.
pub(crate) const LINETYPE_KEYS: [&str; 2] = ["description", "pattern"];

/// The keys a layer takes.
pub(crate) const LAYER_KEYS: [&str; 3] = ["colour", "linetype", "lineweight"];

/// The colours that have a name: AutoCAD colour indexes 1 to 7.
pub(crate) const COLOUR_NAMES: [(&str, u8); 7] = [
    ("red", 1),
    ("yellow", 2),
    ("green", 3),
    ("cyan", 4),
    ("blue", 5),
    ("magenta", 6),
    ("white", 7),
];

/// Linetypes every drawing has, whose names a sheet cannot define again.
const BUILT_IN_LINETYPES: [&str; 3] = ["ByLayer", "ByBlock", CONTINUOUS];

/// Characters AutoCAD does not allow in a layer or linetype name.
pub(crate) const FORBIDDEN_IN_NAMES: &str = "<>/\\\":;?*|=,`";

/// The longest layer or linetype name AutoCAD allows, in characters.
pub(crate) const LONGEST_NAME: usize = 255;

/// The most dashes, gaps and dots a linetype pattern can hold.
pub(crate) const LONGEST_PATTERN: usize = 12;

/// The longest linetype description AutoCAD allows, in characters.
pub(crate) const LONGEST_DESCRIPTION: usize = 47;

pub(super) fn linetypes(node: &Node, units: Unit) -> Result<Vec<Linetype>, Error> {
    let entries = named_entries(node, "linetypes", "linetype")?;
    let mut linetypes = Vec::with_capacity(entries.len());
    for entry in entries {
        if let Some(built_in) = BUILT_IN_LINETYPES
            .iter()
            .find(|b| b.eq_ignore_ascii_case(&entry.key))
        {
            return Err(name_error(
                entry,
                "linetypes",
                format!("`{}` is AutoCAD's own linetype `{built_in}`, which every drawing has; give yours another name", entry.key),
            ));
        }
        let path = key_path("linetypes", &entry.key);
        let fields = Fields::of(&entry.value, &path, &LINETYPE_KEYS, "a linetype")?;
        let description = match fields.get("description") {
            Some(node) => description(node, &fields.path_of("description"))?,
            None => String::new(),
        };
        let pattern = pattern(
            fields.require("pattern")?,
            &fields.path_of("pattern"),
            units,
        )?;
        linetypes.push(Linetype {
            name: entry.key.clone(),
            description,
            pattern,
        });
    }
    Ok(linetypes)
}

pub(super) fn layers(node: &Node, linetypes: &[Linetype]) -> Result<Vec<Layer>, Error> {
    let entries = named_entries(node, "layers", "layer")?;
    let mut layers = Vec::with_capacity(entries.len());
    for entry in entries {
        let path = key_path("layers", &entry.key);
        let fields = Fields::of(&entry.value, &path, &LAYER_KEYS, "a layer")?;
        let mut layer = Layer::new(entry.key.clone());
        if let Some(node) = fields.get("colour") {
            layer.colour = colour(node, &fields.path_of("colour"))?;
        }
        if let Some(node) = fields.get("linetype") {
            layer.linetype = linetype_name(node, &fields.path_of("linetype"), linetypes)?;
        }
        if let Some(node) = fields.get("lineweight") {
            layer.lineweight = Some(lineweight(node, &fields.path_of("lineweight"))?);
        }
        layers.push(layer);
    }
    Ok(layers)
}

/// The name of a layer an object sits on: `0` or one of the sheet's layers.
pub(super) fn layer_name(node: &Node, path: &str, layers: &[Layer]) -> Result<String, Error> {
    let text = scalar(node, path, "the name of a layer")?;
    if text == LAYER_ZERO || layers.iter().any(|l| l.name == text) {
        return Ok(text.to_owned());
    }
    let mut choices = vec![LAYER_ZERO];
    choices.extend(
        layers
            .iter()
            .map(|l| l.name.as_str())
            .filter(|n| *n != LAYER_ZERO),
    );
    Err(unknown_name(node, path, "layer", text, &choices))
}

/// The entries of a mapping from names to definitions, each name checked.
/// `table` is the key the mapping sits under and `what` names one entry.
fn named_entries<'a>(node: &'a Node, table: &str, what: &str) -> Result<&'a [Entry], Error> {
    let Kind::Mapping(entries) = &node.kind else {
        return Err(wrong_type(
            node,
            table,
            &format!("a mapping from {what} names to {what}s"),
        ));
    };
    for (i, entry) in entries.iter().enumerate() {
        check_name(entry, table, what)?;
        if let Some(first) = entries[..i]
            .iter()
            .find(|e| e.key.to_lowercase() == entry.key.to_lowercase())
        {
            return Err(Error::new(
                ErrorCode::DuplicateKey,
                format!(
                    "{what} `{}` is {what} `{}` again, given at line {}: AutoCAD ignores case in names",
                    entry.key, first.key, first.key_at.line
                ),
            )
            .at_field(key_path(table, &entry.key))
            .at_position(entry.key_at.line, entry.key_at.column));
        }
    }
    Ok(entries)
}

/// Refuses a name AutoCAD would not accept for a layer or linetype.
fn check_name(entry: &Entry, table: &str, what: &str) -> Result<(), Error> {
    let name = &entry.key;
    let problem = if name.is_empty() {
        format!("a {what} name cannot be empty")
    } else if name.chars().count() > LONGEST_NAME {
        format!("a {what} name has at most {LONGEST_NAME} characters")
    } else if name.trim() != name {
        format!("a {what} name cannot start or end with a space")
    } else if let Some(c) = name
        .chars()
        .find(|c| FORBIDDEN_IN_NAMES.contains(*c) || c.is_control())
    {
        format!(
            "a {what} name cannot hold `{}`; these are not allowed: {}",
            c.escape_default(),
            FORBIDDEN_IN_NAMES
        )
    } else {
        return Ok(());
    };
    Err(name_error(entry, table, problem))
}

fn name_error(entry: &Entry, table: &str, message: String) -> Error {
    Error::new(ErrorCode::BadValue, message)
        .at_field(key_path(table, &entry.key))
        .at_position(entry.key_at.line, entry.key_at.column)
}

fn description(node: &Node, path: &str) -> Result<String, Error> {
    let text = scalar(node, path, "text")?;
    if text.chars().count() > LONGEST_DESCRIPTION {
        return Err(bad_value(
            node,
            path,
            format!("a linetype description has at most {LONGEST_DESCRIPTION} characters"),
        ));
    }
    Ok(text.to_owned())
}

/// A linetype pattern: dashes (positive), gaps (negative) and dots (0).
fn pattern(node: &Node, path: &str, units: Unit) -> Result<Vec<f64>, Error> {
    let items = sequence(node, path, "a list of lengths")?;
    let pattern = items
        .iter()
        .enumerate()
        .map(|(i, item)| length(item, &index_path(path, i), units).map(|l| l + 0.0))
        .collect::<Result<Vec<f64>, _>>()?;
    let problem = if !(2..=LONGEST_PATTERN).contains(&pattern.len()) {
        format!(
            "a pattern has 2 to {LONGEST_PATTERN} dashes, gaps and dots; this one has {}. For a solid line use `{CONTINUOUS}`",
            pattern.len()
        )
    } else if pattern[0] < 0.0 || pattern[1] >= 0.0 {
        "a pattern starts with a dash (a positive length) or a dot (0), then a gap (a negative length)".to_owned()
    } else {
        return Ok(pattern);
    };
    Err(bad_value(node, path, problem))
}

fn colour(node: &Node, path: &str) -> Result<u8, Error> {
    let text = scalar(node, path, "a colour")?;
    if let Some((_, index)) = COLOUR_NAMES.iter().find(|(name, _)| *name == text) {
        return Ok(*index);
    }
    match text.parse::<u8>() {
        Ok(index) if index >= 1 && text.bytes().all(|b| b.is_ascii_digit()) => Ok(index),
        _ => Err(bad_value(
            node,
            path,
            format!(
                "`{text}` is not a colour; a colour is an AutoCAD colour index from 1 to 255, or one of {}",
                colour_names()
            ),
        )),
    }
}

pub(crate) fn colour_names() -> String {
    COLOUR_NAMES.map(|(name, _)| name).join(", ")
}

/// A lineweight: a pen width in millimetres, whatever the declared unit.
fn lineweight(node: &Node, path: &str) -> Result<Lineweight, Error> {
    let text = scalar(node, path, "a lineweight")?;
    let choices = Lineweight::choices();
    let error = match parse_length(text) {
        Ok((millimetres, None | Some(Unit::Mm))) => match Lineweight::from_millimetres(millimetres)
        {
            Some(weight) => return Ok(weight),
            None => Error::new(
                ErrorCode::BadValue,
                format!("`{text}` is not a standard lineweight; one of {choices} (millimetres)"),
            ),
        },
        Ok((_, Some(other))) => Error::new(
            ErrorCode::BadValue,
            format!("a lineweight is a pen width in millimetres, not {other}; one of {choices}"),
        ),
        Err(LengthError::UnknownUnit(unit)) => Error::new(
            ErrorCode::UnknownUnit,
            format!("unknown unit `{unit}`; a lineweight is in millimetres, one of {choices}"),
        ),
        Err(LengthError::NotANumber | LengthError::OutOfRange) => Error::new(
            ErrorCode::BadNumber,
            format!("expected a lineweight in millimetres, one of {choices}; found `{text}`"),
        ),
    };
    Err(at(error, path, node.at))
}

fn linetype_name(node: &Node, path: &str, linetypes: &[Linetype]) -> Result<String, Error> {
    let text = scalar(node, path, "the name of a linetype")?;
    if text == CONTINUOUS || linetypes.iter().any(|l| l.name == text) {
        return Ok(text.to_owned());
    }
    let mut choices = vec![CONTINUOUS];
    choices.extend(linetypes.iter().map(|l| l.name.as_str()));
    Err(unknown_name(node, path, "linetype", text, &choices))
}

fn unknown_name(node: &Node, path: &str, what: &str, name: &str, choices: &[&str]) -> Error {
    at(
        Error::new(
            ErrorCode::UnknownName,
            format!(
                "unknown {what} `{name}`; the sheet's {what}s: {}",
                choices.join(", ")
            ),
        ),
        path,
        node.at,
    )
}
