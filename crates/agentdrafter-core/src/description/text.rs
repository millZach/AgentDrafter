//! Reads text styles, single-line text and multiline text.
//!
//! A text style is a mapping from its name to its font and width factor,
//! like a layer. Text and multiline text are objects: `objects.rs` lists
//! them and hands their keys to [`text`] and [`mtext`] here.

use super::layers::named_entries;
use super::strict_yaml::{key_path, Node};
use super::{angle, at, bad_value, choice, length, plain_number, point, scalar, Fields};
use crate::error::{Error, ErrorCode};
use crate::text::{Attachment, FontKind, Justify, MText, Text, TextStyle, STANDARD, WIDTH_FACTORS};
use crate::units::Unit;

/// The keys a text style takes.
pub(crate) const TEXT_STYLE_KEYS: [&str; 2] = ["font", "width_factor"];

/// The keys a single line of text takes.
pub(crate) const TEXT_KEYS: [&str; 8] = [
    "contents",
    "position",
    "height",
    "style",
    "width_factor",
    "rotation",
    "justify",
    "layer",
];

/// The keys multiline text takes.
pub(crate) const MTEXT_KEYS: [&str; 9] = [
    "contents",
    "position",
    "height",
    "width",
    "style",
    "width_factor",
    "rotation",
    "justify",
    "layer",
];

/// Characters a font's file name cannot hold: folder separators and the
/// characters Windows refuses in a file name.
pub(crate) const FORBIDDEN_IN_FONT_NAMES: &str = "/\\<>:\"|?*";

pub(super) fn text_styles(node: &Node) -> Result<Vec<TextStyle>, Error> {
    let entries = named_entries(node, "text_styles", "text style")?;
    let mut styles = Vec::with_capacity(entries.len());
    for entry in entries {
        let path = key_path("text_styles", &entry.key);
        let fields = Fields::of(&entry.value, &path, &TEXT_STYLE_KEYS, "a text style")?;
        let font = font(fields.require("font")?, &fields.path_of("font"))?;
        let mut style = TextStyle::new(entry.key.clone(), font);
        if let Some(node) = fields.get("width_factor") {
            style.width_factor = width_factor(node, &fields.path_of("width_factor"))?;
        }
        styles.push(style);
    }
    Ok(styles)
}

pub(super) fn text(
    fields: &Fields,
    units: Unit,
    layer: String,
    styles: &[TextStyle],
) -> Result<Text, Error> {
    let contents_node = fields.require("contents")?;
    let contents_path = fields.path_of("contents");
    let contents = scalar(contents_node, &contents_path, "the text, one line")?;
    let problem = if contents.contains(['\n', '\r']) {
        Some("a text is one line; for more lines use `mtext`".to_owned())
    } else {
        contents_problem(contents).or_else(|| character_code(contents))
    };
    if let Some(problem) = problem {
        return Err(bad_value(contents_node, &contents_path, problem));
    }
    let style = style_name(fields, styles)?;
    let width_factor = match fields.get("width_factor") {
        Some(node) => width_factor(node, &fields.path_of("width_factor"))?,
        None => style_of(&style, styles).map_or(1.0, |s| s.width_factor),
    };
    Ok(Text {
        contents: contents.to_owned(),
        position: point(
            fields.require("position")?,
            &fields.path_of("position"),
            units,
        )?,
        height: height(fields, units)?,
        style,
        width_factor,
        rotation: rotation(fields)?,
        justify: justify(fields)?,
        layer,
    })
}

pub(super) fn mtext(
    fields: &Fields,
    units: Unit,
    layer: String,
    styles: &[TextStyle],
) -> Result<MText, Error> {
    let contents_node = fields.require("contents")?;
    let contents_path = fields.path_of("contents");
    let written = scalar(contents_node, &contents_path, "the text")?;
    // A block scalar ends with a line break; text does not end with an empty line.
    let contents = written.trim_end_matches(['\n', '\r']).replace("\r\n", "\n");
    if let Some(problem) = contents_problem(&contents) {
        return Err(bad_value(contents_node, &contents_path, problem));
    }
    let width = match fields.get("width") {
        Some(node) => {
            let path = fields.path_of("width");
            let width = length(node, &path, units)?;
            if width < 0.0 {
                return Err(bad_value(
                    node,
                    &path,
                    "a width is 0 or more: the width lines wrap in, or 0 for lines that break only where the contents break them",
                ));
            }
            width + 0.0
        }
        None => 0.0,
    };
    let width_factor = match fields.get("width_factor") {
        Some(node) => Some(width_factor(node, &fields.path_of("width_factor"))?),
        None => None,
    };
    Ok(MText {
        contents,
        position: point(
            fields.require("position")?,
            &fields.path_of("position"),
            units,
        )?,
        height: height(fields, units)?,
        width,
        style: style_name(fields, styles)?,
        width_factor,
        rotation: rotation(fields)?,
        justify: attachment(fields)?,
        layer,
    })
}

/// What is wrong with text contents, if anything: they are empty, or hold
/// a control character other than a line break.
fn contents_problem(contents: &str) -> Option<String> {
    if contents.trim().is_empty() {
        return Some("text needs contents: something that shows".to_owned());
    }
    contents
        .chars()
        .find(|c| c.is_control() && *c != '\n')
        .map(|c| {
            format!(
                "text cannot hold the control character `{}`",
                c.escape_default()
            )
        })
}

/// AutoCAD reads `\U+` and four hexadecimal digits in a single line of text
/// as the character with that code, and `\M+` and five as a multibyte one,
/// so such contents would not show as written.
fn character_code(contents: &str) -> Option<String> {
    let bytes = contents.as_bytes();
    (0..bytes.len()).find_map(|i| {
        let digits = match bytes.get(i..i + 3)? {
            [b'\\', b'U' | b'u', b'+'] => 4,
            [b'\\', b'M' | b'm', b'+'] => 5,
            _ => return None,
        };
        let code = bytes.get(i + 3..i + 3 + digits)?;
        code.iter().all(u8::is_ascii_hexdigit).then(|| {
            format!(
                "AutoCAD reads `{}` in text as a character code, not as written; write the character itself",
                &contents[i..i + 3 + digits]
            )
        })
    })
}

/// A font: the file name of an SHX or a TrueType font.
fn font(node: &Node, path: &str) -> Result<String, Error> {
    const CHOICES: &str =
        "a font is the file name of an SHX font, such as `romans.shx`, or a TrueType font, such as `arial.ttf`";
    let text = choice(node, path, "a font", CHOICES)?;
    let problem = if let Some(c) = text
        .chars()
        .find(|c| FORBIDDEN_IN_FONT_NAMES.contains(*c) || c.is_control())
    {
        format!(
            "a font is named by its file name alone, which cannot hold `{}`; {CHOICES}",
            c.escape_default()
        )
    } else if FontKind::of(text).is_none() {
        format!("`{text}` is not a font file name; {CHOICES}")
    } else {
        return Ok(text.to_owned());
    };
    Err(bad_value(node, path, problem))
}

/// A width factor: a number from 0.01 to 100, as AutoCAD allows.
fn width_factor(node: &Node, path: &str) -> Result<f64, Error> {
    let (narrowest, widest) = WIDTH_FACTORS;
    let factor = plain_number(
        node,
        path,
        "a width factor, a number such as `0.8` (1 is normal)",
    )?;
    if (narrowest..=widest).contains(&factor) {
        Ok(factor)
    } else {
        Err(bad_value(
            node,
            path,
            format!("a width factor is from {narrowest} to {widest}; 1 is normal"),
        ))
    }
}

/// The text's height, a length more than 0.
fn height(fields: &Fields, units: Unit) -> Result<f64, Error> {
    let node = fields.require("height")?;
    let path = fields.path_of("height");
    let height = length(node, &path, units)?;
    if height > 0.0 {
        Ok(height)
    } else {
        Err(bad_value(
            node,
            &path,
            "a text height is more than 0: the height of the capitals",
        ))
    }
}

fn rotation(fields: &Fields) -> Result<f64, Error> {
    match fields.get("rotation") {
        Some(node) => angle(node, &fields.path_of("rotation")),
        None => Ok(0.0),
    }
}

/// The style the text draws with: `Standard` or one of the sheet's.
fn style_name(fields: &Fields, styles: &[TextStyle]) -> Result<String, Error> {
    let Some(node) = fields.get("style") else {
        return Ok(STANDARD.to_owned());
    };
    let path = fields.path_of("style");
    let mut choices = vec![STANDARD];
    choices.extend(
        styles
            .iter()
            .map(|s| s.name.as_str())
            .filter(|n| *n != STANDARD),
    );
    let listed = format!("the sheet's text styles: {}", choices.join(", "));
    let text = choice(node, &path, "the name of a text style", &listed)?;
    if choices.contains(&text) {
        return Ok(text.to_owned());
    }
    Err(at(
        Error::new(
            ErrorCode::UnknownName,
            format!("unknown text style `{text}`; {listed}"),
        ),
        &path,
        node.at,
    ))
}

fn style_of<'a>(name: &str, styles: &'a [TextStyle]) -> Option<&'a TextStyle> {
    styles.iter().find(|s| s.name == name)
}

fn justify(fields: &Fields) -> Result<Justify, Error> {
    let names = Justify::ALL.map(Justify::name);
    named_choice(fields, "a text", &names)
        .map(|found| found.map_or(Justify::Left, |i| Justify::ALL[i]))
}

fn attachment(fields: &Fields) -> Result<Attachment, Error> {
    let names = Attachment::ALL.map(Attachment::name);
    named_choice(fields, "multiline text", &names)
        .map(|found| found.map_or(Attachment::TopLeft, |i| Attachment::ALL[i]))
}

/// The index in `names` of the `justify` value, or none when it is not given.
fn named_choice(fields: &Fields, what: &str, names: &[&str]) -> Result<Option<usize>, Error> {
    let Some(node) = fields.get("justify") else {
        return Ok(None);
    };
    let path = fields.path_of("justify");
    let listed = format!("{what} is justified {}", names.join(", "));
    let text = choice(node, &path, "a justification", &listed)?;
    match names.iter().position(|n| *n == text) {
        Some(i) => Ok(Some(i)),
        None => Err(bad_value(
            node,
            &path,
            format!("`{text}` is not a justification; {listed}"),
        )),
    }
}
