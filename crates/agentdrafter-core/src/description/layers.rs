//! Reads the sheet's linetypes and layers.
//!
//! Both are mappings from a name to what it stands for, so a name is given
//! once. AutoCAD ignores case in these names, so two names that differ only
//! in case are refused as well.

use super::strict_yaml::{index_path, key_path, Entry, Kind, Node};
use super::{
    at, bad_value, boolean, choice, length, plain_number, scalar, sequence, wrong_type, Fields,
};
use crate::error::{Error, ErrorCode};
use crate::model::{
    Layer, Linetype, Lineweight, ObjectColor, ObjectLinetype, ObjectLineweight, Properties,
    BUILT_IN_LINETYPES, BY_BLOCK, BY_LAYER, CONTINUOUS, DEFAULT_LINEWEIGHT, LAYER_ZERO,
};
use crate::units::{parse_length, LengthError, Unit};

/// The keys a linetype takes.
pub(crate) const LINETYPE_KEYS: [&str; 2] = ["description", "pattern"];

/// The keys a layer takes.
pub(crate) const LAYER_KEYS: [&str; 8] = [
    "color",
    "linetype",
    "lineweight",
    "on",
    "frozen",
    "locked",
    "plot",
    "description",
];

/// The longest layer description AgentDrafter writes, in characters.
pub(crate) const LONGEST_LAYER_DESCRIPTION: usize = 255;

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
        let description = match fields.text("description") {
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
        if let Some(node) = fields.get("color") {
            layer.color = colour(node, &fields.path_of("color"), "")?;
        }
        if let Some(node) = fields.text("linetype") {
            layer.linetype = linetype_name(node, &fields.path_of("linetype"), linetypes)?;
        }
        if let Some(node) = fields.get("lineweight") {
            layer.lineweight = Some(lineweight(node, &fields.path_of("lineweight"))?);
        }
        for (key, state) in [
            ("on", &mut layer.on),
            ("frozen", &mut layer.frozen),
            ("locked", &mut layer.locked),
            ("plot", &mut layer.plot),
        ] {
            if let Some(node) = fields.get(key) {
                *state = boolean(node, &fields.path_of(key))?;
            }
        }
        if let Some(node) = fields.text("description") {
            layer.description = layer_description(node, &fields.path_of("description"))?;
        }
        layers.push(layer);
    }
    Ok(layers)
}

/// The keys every object takes for a colour, linetype, lineweight and
/// linetype scale of its own, besides its layer.
pub(crate) const PROPERTY_KEYS: [&str; 4] = ["color", "linetype", "lineweight", "linetype_scale"];

/// An object's own colour, linetype, lineweight and linetype scale: each
/// `ByLayer` unless the object gives it. `linetypes` are those the
/// object's sheet or block library defines.
pub(super) fn properties(fields: &Fields, linetypes: &[Linetype]) -> Result<Properties, Error> {
    let mut properties = Properties::default();
    if let Some(node) = fields.get("color") {
        let path = fields.path_of("color");
        properties.color = match &node.kind {
            Kind::Scalar(text) if text == BY_LAYER => ObjectColor::ByLayer,
            Kind::Scalar(text) if text == BY_BLOCK => ObjectColor::ByBlock,
            _ => ObjectColor::Index(colour(node, &path, "`ByLayer`, `ByBlock`, ")?),
        };
    }
    if let Some(node) = fields.text("linetype") {
        let path = fields.path_of("linetype");
        let mut choices = vec![BY_LAYER, BY_BLOCK, CONTINUOUS];
        choices.extend(linetypes.iter().map(|l| l.name.as_str()));
        properties.linetype = ObjectLinetype::from(named(node, &path, "linetype", &choices)?);
    }
    if let Some(node) = fields.get("lineweight") {
        let path = fields.path_of("lineweight");
        properties.lineweight = match &node.kind {
            Kind::Scalar(text) if text == BY_LAYER => ObjectLineweight::ByLayer,
            Kind::Scalar(text) if text == BY_BLOCK => ObjectLineweight::ByBlock,
            Kind::Scalar(text) if text == DEFAULT_LINEWEIGHT => ObjectLineweight::Default,
            _ => ObjectLineweight::Width(lineweight(node, &path).map_err(|e| Error {
                message: format!(
                    "{}; or `{BY_LAYER}`, `{BY_BLOCK}` or `{DEFAULT_LINEWEIGHT}`",
                    e.message
                ),
                ..e
            })?),
        };
    }
    if let Some(node) = fields.get("linetype_scale") {
        properties.linetype_scale = linetype_scale(node, &fields.path_of("linetype_scale"))?;
    }
    Ok(properties)
}

/// A linetype scale: a number more than 0, with no unit, that a linetype's
/// dashes and gaps are enlarged by.
pub(super) fn linetype_scale(node: &Node, path: &str) -> Result<f64, Error> {
    let scale = plain_number(node, path, "a linetype scale, a number such as `0.5`")?;
    if scale > 0.0 {
        Ok(scale)
    } else {
        Err(bad_value(
            node,
            path,
            "a linetype scale is more than 0: how much a linetype's dashes and gaps are enlarged",
        ))
    }
}

/// A layer's description: one line of text, at most
/// [`LONGEST_LAYER_DESCRIPTION`] characters, with no control characters.
fn layer_description(node: &Node, path: &str) -> Result<String, Error> {
    let text = scalar(node, path, "a description, one line of text")?;
    if text.chars().any(char::is_control) {
        return Err(bad_value(
            node,
            path,
            "a layer's description is one line of text, with no control characters",
        ));
    }
    if text.chars().count() > LONGEST_LAYER_DESCRIPTION {
        return Err(bad_value(
            node,
            path,
            format!(
                "a layer's description is at most {LONGEST_LAYER_DESCRIPTION} characters; this one has {}",
                text.chars().count()
            ),
        ));
    }
    Ok(text.to_owned())
}

/// The name of a layer an object sits on: `0` or one of the sheet's layers.
pub(super) fn layer_name(node: &Node, path: &str, layers: &[Layer]) -> Result<String, Error> {
    let mut choices = vec![LAYER_ZERO];
    choices.extend(
        layers
            .iter()
            .map(|l| l.name.as_str())
            .filter(|n| *n != LAYER_ZERO),
    );
    named(node, path, "layer", &choices)
}

/// The entries of a mapping from names to definitions, each name checked.
/// `table` is the key the mapping sits under and `what` names one entry.
pub(super) fn named_entries<'a>(
    node: &'a Node,
    table: &str,
    what: &str,
) -> Result<&'a [Entry], Error> {
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

pub(super) fn name_error(entry: &Entry, table: &str, message: String) -> Error {
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

/// A colour index, by number or name. `more` names what else the value
/// may be, for the error, such as `ByLayer, ByBlock, `.
fn colour(node: &Node, path: &str, more: &str) -> Result<u8, Error> {
    let choices = format!(
        "a colour is {more}an AutoCAD colour index from 1 to 255, or one of {}",
        colour_names()
    );
    let text = choice(node, path, "a colour", &choices)?;
    if let Some((_, index)) = COLOUR_NAMES.iter().find(|(name, _)| *name == text) {
        return Ok(*index);
    }
    match text.parse::<u8>() {
        Ok(index) if index >= 1 && text.bytes().all(|b| b.is_ascii_digit()) => Ok(index),
        _ => Err(bad_value(
            node,
            path,
            format!("`{text}` is not a colour; {choices}"),
        )),
    }
}

pub(crate) fn colour_names() -> String {
    COLOUR_NAMES.map(|(name, _)| name).join(", ")
}

/// A lineweight: a pen width in millimetres, whatever the declared unit,
/// written out as one of the standard widths.
fn lineweight(node: &Node, path: &str) -> Result<Lineweight, Error> {
    let choices = Lineweight::choices();
    let text = choice(
        node,
        path,
        "a lineweight",
        &format!("one of {choices} (millimetres)"),
    )?;
    let error = match parse_length(text) {
        Ok((_, None | Some(Unit::Mm))) => {
            let number = text
                .strip_suffix("mm")
                .map_or(text, |n| n.strip_suffix(' ').unwrap_or(n));
            match written_lineweight(number) {
                Some(weight) => return Ok(weight),
                None => Error::new(
                    ErrorCode::BadValue,
                    format!(
                        "`{text}` is not a standard lineweight written out; one of {choices} (millimetres)"
                    ),
                ),
            }
        }
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

/// The standard lineweight that `number` writes out in millimetres: digits,
/// then a `.` and digits if any, with any zeros before or after, such as
/// `0.35`, `0.350` or `1.0`. A sign or an exponent is not written out.
/// `schema.rs` states the same spellings as a pattern.
fn written_lineweight(number: &str) -> Option<Lineweight> {
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    let digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    if whole.is_empty() || !digits(whole) || !digits(fraction) {
        return None;
    }
    let whole = match whole.trim_start_matches('0') {
        "" => "0",
        whole => whole,
    };
    let spelled = match fraction.trim_end_matches('0') {
        "" => whole.to_owned(),
        fraction => format!("{whole}.{fraction}"),
    };
    Lineweight::ALL
        .into_iter()
        .find(|weight| weight.to_string() == spelled)
}

fn linetype_name(node: &Node, path: &str, linetypes: &[Linetype]) -> Result<String, Error> {
    let mut choices = vec![CONTINUOUS];
    choices.extend(linetypes.iter().map(|l| l.name.as_str()));
    named(node, path, "linetype", &choices)
}

/// The name of a `what`, a layer or a linetype, that must be one of
/// `choices`. Any other value is an error that lists them.
fn named(node: &Node, path: &str, what: &str, choices: &[&str]) -> Result<String, Error> {
    let listed = format!("the sheet's {what}s: {}", choices.join(", "));
    let text = choice(node, path, &format!("the name of a {what}"), &listed)?;
    if choices.contains(&text) {
        return Ok(text.to_owned());
    }
    Err(at(
        Error::new(
            ErrorCode::UnknownName,
            format!("unknown {what} `{text}`; {listed}"),
        ),
        path,
        node.at,
    ))
}
