//! Reads the paper a sheet description or template declares, and states
//! it in the schema.
//!
//! Zach decided on 2026-11-11 that a sheet's paper is declared. A template,
//! or a sheet description, may say under `paper` what it is drawn on: its
//! width and height, `[17, 11]`, in the declared unit or each with its own,
//! or a standard size by name with the way it lies, `ANSI B landscape`.
//! The paper's lower left corner is at the origin, as a sheet drawn in
//! model space at its paper size draws its trim line from there. Only the
//! checker reads it: a build, a render and a plot are unchanged by it.

use serde_json::{json, Value};

use super::strict_yaml::{index_path, Kind, Node};
use super::{at, bad_value, length, wrong_type};
use crate::error::{Error, ErrorCode};
use crate::model::Paper;
use crate::units::Unit;

/// The key a sheet description or template declares its paper under.
pub(crate) const PAPER_KEY: &str = "paper";

/// The standard sizes a paper may be declared by: ANSI A to E and ISO A0
/// to A4, each with its shorter and its longer side in its own unit.
pub(crate) const STANDARD_SIZES: [(&str, f64, f64, Unit); 10] = [
    ("ANSI A", 8.5, 11.0, Unit::In),
    ("ANSI B", 11.0, 17.0, Unit::In),
    ("ANSI C", 17.0, 22.0, Unit::In),
    ("ANSI D", 22.0, 34.0, Unit::In),
    ("ANSI E", 34.0, 44.0, Unit::In),
    ("ISO A0", 841.0, 1189.0, Unit::Mm),
    ("ISO A1", 594.0, 841.0, Unit::Mm),
    ("ISO A2", 420.0, 594.0, Unit::Mm),
    ("ISO A3", 297.0, 420.0, Unit::Mm),
    ("ISO A4", 210.0, 297.0, Unit::Mm),
];

/// The ways a standard size lies: its longer side across, or up.
const LANDSCAPE: &str = "landscape";
const PORTRAIT: &str = "portrait";

/// Every name a paper may be declared by, such as `ANSI B landscape`, with
/// its width and height in its own unit.
pub(crate) fn paper_names() -> Vec<(String, f64, f64, Unit)> {
    STANDARD_SIZES
        .iter()
        .flat_map(|&(size, short, long, unit)| {
            [
                (format!("{size} {LANDSCAPE}"), long, short, unit),
                (format!("{size} {PORTRAIT}"), short, long, unit),
            ]
        })
        .collect()
}

/// What a paper is, for the errors that refuse one.
fn choices() -> String {
    let names: Vec<String> = paper_names().into_iter().map(|(name, ..)| name).collect();
    format!(
        "a paper is its width and height, two lengths such as [17, 11], or a standard size \
         and the way it lies: {}",
        names.join(", ")
    )
}

/// A length more than 0: a side of the paper.
fn side(node: &Node, path: &str, units: Unit) -> Result<f64, Error> {
    let side = length(node, path, units)?;
    if side > 0.0 {
        Ok(side)
    } else {
        Err(bad_value(
            node,
            path,
            "a side of the paper is a length more than 0",
        ))
    }
}

/// The paper `node` declares, in the declared unit `units`.
pub(super) fn paper(node: &Node, path: &str, units: Unit) -> Result<Paper, Error> {
    match &node.kind {
        Kind::Sequence(items) => {
            let [width, height] = items.as_slice() else {
                return Err(at(
                    Error::new(
                        ErrorCode::WrongType,
                        format!(
                            "a paper's size is two lengths, its width and its height, such as \
                             [17, 11]; this one has {}",
                            items.len()
                        ),
                    ),
                    path,
                    node.at,
                ));
            };
            Ok(Paper {
                name: None,
                width: side(width, &index_path(path, 0), units)?,
                height: side(height, &index_path(path, 1), units)?,
            })
        }
        Kind::Scalar(text) => {
            if let Some((name, width, height, own)) =
                paper_names().into_iter().find(|(name, ..)| name == text)
            {
                return Ok(Paper {
                    name: Some(name),
                    width: own.convert(width, units),
                    height: own.convert(height, units),
                });
            }
            let error = match STANDARD_SIZES.iter().find(|(size, ..)| size == text) {
                Some(&(size, short, long, own)) => Error::new(
                    ErrorCode::BadValue,
                    format!(
                        "a standard size says the way it lies: `{size} {LANDSCAPE}`, {long} by \
                         {short} {own}, or `{size} {PORTRAIT}`, {short} by {long} {own}"
                    ),
                ),
                None => Error::new(
                    ErrorCode::UnknownName,
                    format!("unknown paper `{text}`; {}", choices()),
                ),
            };
            Err(at(error, path, node.at))
        }
        Kind::Mapping(_) => Err(wrong_type(node, path, &format!("a paper; {}", choices()))),
    }
}

/// The schema of the `paper` key of a sheet description or template.
pub(crate) fn paper_property() -> Value {
    let names: Vec<String> = paper_names().into_iter().map(|(name, ..)| name).collect();
    let sizes = STANDARD_SIZES
        .iter()
        .map(|(size, short, long, unit)| format!("{size} {short} by {long} {unit}"))
        .collect::<Vec<_>>()
        .join(", ");
    json!({
        "description": format!(
            "The paper the sheet is drawn on, which the checker holds it to: its width and \
             height, two lengths more than 0 such as [17, 11], in the declared unit or each with \
             its own, or a standard size and the way it lies, landscape with its longer side \
             across or portrait with it up, such as `ANSI B landscape`, 17 by 11 in. The standard \
             sizes: {sizes}. The paper's lower left corner is at the origin, [0, 0]. Only the \
             checker reads it: a build, a render and a plot are the same with it or without it. \
             Without it, the checker reports that the sheet declares no paper."
        ),
        "anyOf": [
            {
                "type": "array",
                "items": {
                    "anyOf": [
                        { "type": "number", "exclusiveMinimum": 0 },
                        { "$ref": "#/$defs/length", "type": "string" },
                    ],
                },
                "minItems": 2,
                "maxItems": 2,
            },
            { "enum": names },
        ],
    })
}
