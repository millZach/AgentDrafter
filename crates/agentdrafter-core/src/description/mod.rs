//! Reads a sheet description into a [`Sheet`].
//!
//! A sheet description is a strict subset of YAML, written down in
//! `docs/sheet-description.md`. [`strict_yaml`] reads the subset into a tree
//! of text; this module gives the text its meaning and refuses anything the
//! sheet description does not know. Every error names the field it is about.

mod strict_yaml;

use strict_yaml::{index_path, key_path, Entry, Kind, Node, Pos};

use crate::error::{Error, ErrorCode};
use crate::model::{Line, Object, Point3, Sheet};
use crate::units::{parse_length, LengthError, Unit};

/// The object types a sheet description can hold.
const OBJECT_TYPES: [&str; 1] = ["line"];

/// Reads the text of a sheet description into a [`Sheet`].
///
/// Every coordinate comes back in the sheet's declared unit; a number that
/// carries its own unit is converted to it.
pub fn parse_sheet_description(text: &str) -> Result<Sheet, Error> {
    let Some(root) = strict_yaml::read(text)? else {
        return Err(Error::new(
            ErrorCode::MissingKey,
            "the sheet description is empty; it needs `units` and `objects`",
        ));
    };
    let fields = Fields::of(&root, "", &["units", "objects"], "a sheet description")?;
    let units = match fields.get("units") {
        Some(node) => unit(node, "units")?,
        None => {
            return Err(Error::new(
                ErrorCode::MissingUnit,
                format!(
                    "the sheet declares no unit; add `units:` with one of {}",
                    Unit::symbols()
                ),
            )
            .at_field("units")
            .at_position(root.at.line, root.at.column))
        }
    };
    let list = sequence(fields.require("objects")?, "objects", "a list of objects")?;
    let objects = list
        .iter()
        .enumerate()
        .map(|(i, node)| object(node, &index_path("objects", i), units))
        .collect::<Result<_, _>>()?;
    Ok(Sheet { units, objects })
}

fn object(node: &Node, path: &str, units: Unit) -> Result<Object, Error> {
    const SHAPE: &str = "a mapping with one key naming the object's type, such as `line:`";
    let Kind::Mapping(entries) = &node.kind else {
        return Err(wrong_type(node, path, SHAPE));
    };
    if let Some(entry) = entries
        .iter()
        .find(|e| !OBJECT_TYPES.contains(&e.key.as_str()))
    {
        return Err(unknown_key(
            entry,
            path,
            &format!("unknown object type `{}`", entry.key),
            &format!("known types: {}", OBJECT_TYPES.join(", ")),
        ));
    }
    let [entry] = entries.as_slice() else {
        return Err(wrong_type(node, path, SHAPE));
    };
    let path = key_path(path, &entry.key);
    match entry.key.as_str() {
        "line" => line(&entry.value, &path, units).map(Object::Line),
        other => unreachable!("object type `{other}` is listed but not read"),
    }
}

fn line(node: &Node, path: &str, units: Unit) -> Result<Line, Error> {
    let fields = Fields::of(node, path, &["from", "to"], "a line")?;
    let from = point(fields.require("from")?, &key_path(path, "from"), units)?;
    let to = point(fields.require("to")?, &key_path(path, "to"), units)?;
    Ok(Line { from, to })
}

fn point(node: &Node, path: &str, units: Unit) -> Result<Point3, Error> {
    const SHAPE: &str = "a point, a list of three lengths [x, y, z]";
    let items = sequence(node, path, SHAPE)?;
    let [x, y, z] = items else {
        return Err(at(
            Error::new(
                ErrorCode::WrongType,
                format!(
                    "a point has three coordinates [x, y, z]; this one has {}",
                    items.len()
                ),
            ),
            path,
            node.at,
        ));
    };
    Ok(Point3::new(
        length(x, &index_path(path, 0), units)?,
        length(y, &index_path(path, 1), units)?,
        length(z, &index_path(path, 2), units)?,
    ))
}

/// A length in the declared unit. A number that carries its own unit is converted.
fn length(node: &Node, path: &str, declared: Unit) -> Result<f64, Error> {
    const SHAPE: &str = "a length such as `100` or `4 in`";
    let Kind::Scalar(text) = &node.kind else {
        return Err(wrong_type(node, path, SHAPE));
    };
    let error = match parse_length(text) {
        Ok((value, None)) => return Ok(value),
        Ok((value, Some(own))) => match own.convert(value, declared) {
            converted if converted.is_finite() => return Ok(converted),
            _ => Error::new(
                ErrorCode::BadNumber,
                format!("`{text}` is too large once converted to {declared}"),
            ),
        },
        Err(LengthError::NotANumber) => Error::new(
            ErrorCode::BadNumber,
            format!("expected {SHAPE}, found {}", describe(node)),
        ),
        Err(LengthError::OutOfRange) => {
            Error::new(ErrorCode::BadNumber, format!("`{text}` is too large"))
        }
        Err(LengthError::UnknownUnit(unit)) => unknown_unit(&unit),
    };
    Err(at(error, path, node.at))
}

fn unit(node: &Node, path: &str) -> Result<Unit, Error> {
    let Kind::Scalar(text) = &node.kind else {
        return Err(wrong_type(node, path, "a unit"));
    };
    let error = if text.is_empty() {
        Error::new(
            ErrorCode::MissingUnit,
            format!(
                "the sheet declares no unit; give `units:` one of {}",
                Unit::symbols()
            ),
        )
    } else if let Some(unit) = Unit::from_symbol(text) {
        return Ok(unit);
    } else {
        unknown_unit(text)
    };
    Err(at(error, path, node.at))
}

fn unknown_unit(unit: &str) -> Error {
    Error::new(
        ErrorCode::UnknownUnit,
        format!("unknown unit `{unit}`; known units: {}", Unit::symbols()),
    )
}

fn sequence<'a>(node: &'a Node, path: &str, expected: &str) -> Result<&'a [Node], Error> {
    match &node.kind {
        Kind::Sequence(items) => Ok(items),
        _ => Err(wrong_type(node, path, expected)),
    }
}

/// The keys of one mapping, checked against the keys it may have.
struct Fields<'a> {
    path: &'a str,
    at: Pos,
    entries: &'a [Entry],
}

impl<'a> Fields<'a> {
    /// Reads `node` as a mapping that may hold only the `known` keys.
    /// `what` names the mapping in messages, such as "a line".
    fn of(node: &'a Node, path: &'a str, known: &[&str], what: &str) -> Result<Self, Error> {
        let Kind::Mapping(entries) = &node.kind else {
            return Err(wrong_type(node, path, &format!("{what}, a mapping")));
        };
        if let Some(entry) = entries.iter().find(|e| !known.contains(&e.key.as_str())) {
            return Err(unknown_key(
                entry,
                path,
                &format!("unknown key `{}`", entry.key),
                &format!("{what} takes {}", known.join(", ")),
            ));
        }
        Ok(Self {
            path,
            at: node.at,
            entries,
        })
    }

    fn get(&self, key: &str) -> Option<&'a Node> {
        self.entries.iter().find(|e| e.key == key).map(|e| &e.value)
    }

    fn require(&self, key: &str) -> Result<&'a Node, Error> {
        self.get(key).ok_or_else(|| {
            Error::new(ErrorCode::MissingKey, format!("missing key `{key}`"))
                .at_field(key_path(self.path, key))
                .at_position(self.at.line, self.at.column)
        })
    }
}

fn unknown_key(entry: &Entry, path: &str, what: &str, known: &str) -> Error {
    Error::new(ErrorCode::UnknownKey, format!("{what}; {known}"))
        .at_field(key_path(path, &entry.key))
        .at_position(entry.key_at.line, entry.key_at.column)
}

fn wrong_type(node: &Node, path: &str, expected: &str) -> Error {
    at(
        Error::new(
            ErrorCode::WrongType,
            format!("expected {expected}, found {}", describe(node)),
        ),
        path,
        node.at,
    )
}

/// Places an error at a field and position. The top level has no field name.
fn at(error: Error, path: &str, pos: Pos) -> Error {
    let error = error.at_position(pos.line, pos.column);
    if path.is_empty() {
        error
    } else {
        error.at_field(path)
    }
}

fn describe(node: &Node) -> String {
    match &node.kind {
        Kind::Scalar(text) if text.is_empty() => "nothing".to_owned(),
        Kind::Scalar(text) => format!("`{text}`"),
        Kind::Sequence(_) => "a list".to_owned(),
        Kind::Mapping(_) => "a mapping".to_owned(),
    }
}

#[cfg(test)]
mod tests;
