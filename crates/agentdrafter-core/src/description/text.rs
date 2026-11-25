//! Reads text styles, single-line text and multiline text.
//!
//! A text style is a mapping from its name to its font and width factor,
//! like a layer. Text and multiline text are objects: `objects.rs` lists
//! them and hands their keys to [`text`] and [`mtext`] here.

use super::layers::{name_error, named_entries, object_color};
use super::strict_yaml::{key_path, Kind, Node};
use super::{angle, at, bad_value, boolean, choice, length, plain_number, point, scalar, Fields};
use crate::error::{Error, ErrorCode};
use crate::text::{
    font_name_problem, inline_font_name_problem, Attachment, Justify, MText, MTextRun, Text,
    TextStyle, FONT_CHOICES, OBLIQUE_ANGLES, STANDARD, TRACKING_FACTORS, WIDTH_FACTORS,
};
use crate::units::Unit;

/// The keys a text style takes.
pub(crate) const TEXT_STYLE_KEYS: [&str; 3] = [
    "font",
    "width_factor",
    super::annotation_scale::ANNOTATIVE_KEY,
];

/// The keys a single line of text takes.
pub(crate) const TEXT_KEYS: [&str; 19] = [
    "contents",
    "field",
    "position",
    "height",
    "style",
    "width_factor",
    "rotation",
    "justify",
    "underline",
    "overline",
    super::annotation_scale::ANNOTATIVE_KEY,
    super::annotation_scale::ANNOTATION_SCALES_KEY,
    "layer",
    "color",
    "linetype",
    "lineweight",
    "linetype_scale",
    "plot_style",
    "transparency",
];

/// The keys multiline text takes.
pub(crate) const MTEXT_KEYS: [&str; 21] = [
    "contents",
    "runs",
    "field",
    "position",
    "height",
    "width",
    "style",
    "width_factor",
    "rotation",
    "justify",
    "underline",
    "overline",
    super::annotation_scale::ANNOTATIVE_KEY,
    super::annotation_scale::ANNOTATION_SCALES_KEY,
    "layer",
    "color",
    "linetype",
    "lineweight",
    "linetype_scale",
    "plot_style",
    "transparency",
];

/// The keys one inline-formatted run of an `mtext` takes (#61).
pub(crate) const MTEXT_RUN_KEYS: [&str; 11] = [
    "text",
    "font",
    "bold",
    "italic",
    "height",
    "width_factor",
    "oblique",
    "tracking",
    "underline",
    "overline",
    "color",
];

pub(super) fn text_styles(node: &Node) -> Result<Vec<TextStyle>, Error> {
    let entries = named_entries(node, "text_styles", "text style")?;
    let mut styles = Vec::with_capacity(entries.len());
    for entry in entries {
        // AutoCAD ignores case in names, so `standard` would be `Standard`
        // in the DWG and another style on the sheet. `Standard` is written
        // exactly, like any reference to it.
        if entry.key != STANDARD && entry.key.eq_ignore_ascii_case(STANDARD) {
            return Err(name_error(
                entry,
                "text_styles",
                format!(
                    "`{}` is the text style `{STANDARD}`, which every drawing has, written in another case, and AutoCAD ignores case in names; to set its font, list it as `{STANDARD}`",
                    entry.key
                ),
            ));
        }
        let path = key_path("text_styles", &entry.key);
        let fields = Fields::of(&entry.value, &path, &TEXT_STYLE_KEYS, "a text style")?;
        let font = font(fields.require_text("font")?, &fields.path_of("font"))?;
        let mut style = TextStyle::new(entry.key.clone(), font);
        if let Some(node) = fields.get("width_factor") {
            style.width_factor = width_factor(node, &fields.path_of("width_factor"))?;
        }
        style.annotative = super::annotation_scale::optional_annotative(&fields)?;
        styles.push(style);
    }
    Ok(styles)
}

pub(super) fn text(
    fields: &Fields,
    units: Unit,
    layer: String,
    styles: &[TextStyle],
    declared_scales: &[crate::model::AnnotationScale],
) -> Result<Text, Error> {
    let field = match fields.get(super::field::FIELD_KEY) {
        Some(node) if !node.is_bare_null() => Some(super::field::parse_field(
            node,
            &fields.path_of(super::field::FIELD_KEY),
        )?),
        _ => None,
    };
    let contents = match fields.text("contents") {
        Some(contents_node) => {
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
            contents.to_owned()
        }
        None if field.is_some() => String::new(),
        None => {
            let _ = fields.require_text("contents")?;
            unreachable!("require_text returns Err when missing")
        }
    };
    let style = style_name(fields, styles)?;
    let width_factor = match fields.get("width_factor") {
        Some(node) => width_factor(node, &fields.path_of("width_factor"))?,
        None => style_of(&style, styles).map_or(1.0, |s| s.width_factor),
    };
    let underline = optional_flag(fields, "underline")?;
    let overline = optional_flag(fields, "overline")?;
    let (annotative, annotation_scales) =
        super::annotation_scale::object_annotation(fields, declared_scales)?;
    Ok(Text {
        contents,
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
        underline,
        overline,
        layer,
        field,
        annotative,
        annotation_scales,
        properties: Default::default(),
    })
}

/// An mtext's contents as written, as they show: a block scalar ends with a
/// line break, and text does not end with an empty line, so line breaks at
/// the end are dropped, and CR LF is a line break, as LF is.
pub(crate) fn shown_mtext(written: &str) -> String {
    written.trim_end_matches(['\n', '\r']).replace("\r\n", "\n")
}

pub(super) fn mtext(
    fields: &Fields,
    units: Unit,
    layer: String,
    styles: &[TextStyle],
    declared_scales: &[crate::model::AnnotationScale],
) -> Result<MText, Error> {
    let field = match fields.get(super::field::FIELD_KEY) {
        Some(node) if !node.is_bare_null() => Some(super::field::parse_field(
            node,
            &fields.path_of(super::field::FIELD_KEY),
        )?),
        _ => None,
    };
    let (contents, runs) = match (fields.text("contents"), fields.get("runs")) {
        (Some(_), Some(runs_node)) => {
            return Err(bad_value(
                runs_node,
                &fields.path_of("runs"),
                "write either `contents` for plain multiline text or `runs` for inline-formatted spans, not both",
            ));
        }
        (None, Some(runs_node)) => {
            let runs = parse_mtext_runs(runs_node, &fields.path_of("runs"), units)?;
            let plain: String = runs.iter().map(|r| r.text.as_str()).collect();
            if let Some(problem) = contents_problem(&plain) {
                return Err(bad_value(runs_node, &fields.path_of("runs"), problem));
            }
            (plain, runs)
        }
        (Some(contents_node), None) => {
            let contents_path = fields.path_of("contents");
            let written = scalar(contents_node, &contents_path, "the text")?;
            let contents = shown_mtext(written);
            if let Some(problem) = contents_problem(&contents) {
                return Err(bad_value(contents_node, &contents_path, problem));
            }
            (contents, Vec::new())
        }
        (None, None) if field.is_some() => (String::new(), Vec::new()),
        (None, None) => {
            let _ = fields.require_text("contents")?;
            unreachable!("require_text returns Err when missing")
        }
    };
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
    let underline = optional_flag(fields, "underline")?;
    let overline = optional_flag(fields, "overline")?;
    let (annotative, annotation_scales) =
        super::annotation_scale::object_annotation(fields, declared_scales)?;
    Ok(MText {
        contents,
        runs,
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
        underline,
        overline,
        layer,
        field,
        annotative,
        annotation_scales,
        properties: Default::default(),
    })
}

fn parse_mtext_runs(node: &Node, path: &str, units: Unit) -> Result<Vec<MTextRun>, Error> {
    let Kind::Sequence(items) = &node.kind else {
        return Err(at(
            Error::new(
                ErrorCode::WrongType,
                "runs is a list of formatted text spans, such as `- {text: 'NOTES:', bold: true}`",
            ),
            path,
            node.at,
        ));
    };
    if items.is_empty() {
        return Err(bad_value(
            node,
            path,
            "runs needs at least one span of text",
        ));
    }
    let mut out = Vec::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        let item_path = format!("{path}[{idx}]");
        let rf = Fields::of(item, &item_path, &MTEXT_RUN_KEYS, "an mtext run")?;
        let text_node = rf.require_text("text")?;
        let text_path = rf.path_of("text");
        let raw_text = scalar(text_node, &text_path, "the run's text")?.replace("\r\n", "\n");
        if let Some(c) = raw_text.chars().find(|c| c.is_control() && *c != '\n') {
            return Err(bad_value(
                text_node,
                &text_path,
                format!(
                    "text cannot hold the control character `{}`",
                    c.escape_default()
                ),
            ));
        }
        let font = match rf.text("font") {
            Some(f_node) => {
                let f_path = rf.path_of("font");
                let name = scalar(f_node, &f_path, "a font family or file name")?;
                if let Some(problem) = inline_font_name_problem(name) {
                    return Err(bad_value(f_node, &f_path, problem));
                }
                Some(name.trim().to_owned())
            }
            None => None,
        };
        let bold = optional_flag(&rf, "bold")?;
        let italic = optional_flag(&rf, "italic")?;
        let height = match rf.get("height") {
            Some(h_node) => {
                let h_path = rf.path_of("height");
                let h = length(h_node, &h_path, units)?;
                if h <= 0.0 {
                    return Err(bad_value(
                        h_node,
                        &h_path,
                        "a text height is more than 0: the height of the capitals",
                    ));
                }
                Some(h)
            }
            None => None,
        };
        let wf = match rf.get("width_factor") {
            Some(w_node) => Some(width_factor(w_node, &rf.path_of("width_factor"))?),
            None => None,
        };
        let oblique = match rf.get("oblique") {
            Some(q_node) => {
                let q_path = rf.path_of("oblique");
                let deg = plain_number(q_node, &q_path, "an oblique angle in degrees")?;
                if !(OBLIQUE_ANGLES.0..=OBLIQUE_ANGLES.1).contains(&deg) {
                    return Err(bad_value(
                        q_node,
                        &q_path,
                        format!(
                            "an oblique angle is from {} to {} degrees",
                            OBLIQUE_ANGLES.0, OBLIQUE_ANGLES.1
                        ),
                    ));
                }
                Some(deg)
            }
            None => None,
        };
        let tracking = match rf.get("tracking") {
            Some(t_node) => {
                let t_path = rf.path_of("tracking");
                let trk = plain_number(t_node, &t_path, "a tracking factor")?;
                if !(TRACKING_FACTORS.0..=TRACKING_FACTORS.1).contains(&trk) {
                    return Err(bad_value(
                        t_node,
                        &t_path,
                        format!(
                            "a tracking factor is from {} to {}",
                            TRACKING_FACTORS.0, TRACKING_FACTORS.1
                        ),
                    ));
                }
                Some(trk)
            }
            None => None,
        };
        let underline = optional_flag(&rf, "underline")?;
        let overline = optional_flag(&rf, "overline")?;
        let color = match rf.get("color") {
            Some(c_node) => Some(object_color(c_node, &rf.path_of("color"))?),
            None => None,
        };
        out.push(MTextRun {
            text: raw_text,
            font,
            bold,
            italic,
            height,
            width_factor: wf,
            oblique,
            tracking,
            underline,
            overline,
            color,
        });
    }
    Ok(out)
}

fn optional_flag(fields: &Fields, key: &str) -> Result<bool, Error> {
    match fields.get(key) {
        Some(node) => boolean(node, &fields.path_of(key)),
        None => Ok(false),
    }
}

/// What is wrong with text contents, if anything: they are empty, or hold
/// a control character other than a line break.
pub(super) fn contents_problem(contents: &str) -> Option<String> {
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
pub(super) fn character_code(contents: &str) -> Option<String> {
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
    let text = choice(node, path, "a font", FONT_CHOICES)?;
    match font_name_problem(text) {
        Some(problem) => Err(bad_value(node, path, problem)),
        None => Ok(text.to_owned()),
    }
}

/// A width factor: a number from 0.01 to 100, as AutoCAD allows.
pub(super) fn width_factor(node: &Node, path: &str) -> Result<f64, Error> {
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
pub(super) fn height(fields: &Fields, units: Unit) -> Result<f64, Error> {
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

pub(super) fn rotation(fields: &Fields) -> Result<f64, Error> {
    match fields.get("rotation") {
        Some(node) => angle(node, &fields.path_of("rotation")),
        None => Ok(0.0),
    }
}

/// The style the text draws with: `Standard` or one of the sheet's.
pub(super) fn style_name(fields: &Fields, styles: &[TextStyle]) -> Result<String, Error> {
    text_style_at(fields, "style", styles)
}

/// The text style named under `key`: `Standard` or one of the sheet's.
/// Without it, `Standard`.
pub(super) fn text_style_at(
    fields: &Fields,
    key: &str,
    styles: &[TextStyle],
) -> Result<String, Error> {
    let Some(node) = fields.text(key) else {
        return Ok(STANDARD.to_owned());
    };
    let path = fields.path_of(key);
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

pub(super) fn style_of<'a>(name: &str, styles: &'a [TextStyle]) -> Option<&'a TextStyle> {
    styles.iter().find(|s| s.name == name)
}

pub(super) fn justify(fields: &Fields) -> Result<Justify, Error> {
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
