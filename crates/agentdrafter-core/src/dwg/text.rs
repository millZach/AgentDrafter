//! Text styles, single-line text and multiline text in a DWG.
//!
//! Part of the seam ([`super`]): only this module and its parent name
//! acadrust. A DWG holds what text says in AutoCAD's own coding, so the
//! contents are coded on the way in and decoded on the way out:
//!
//! - In a single line of text, `%%` starts a control code (`%%d` is a
//!   degree sign) and `%%%` is a percent sign. A run of two or more `%` is
//!   written as `%%%` each; a lone `%` is written as it is.
//! - Multiline text reads `\` codes and `{ }` groups: a `\`, `{` or `}` is
//!   written with a `\` before it, a line break as `\P`, and each `%` of a
//!   run of two or more as `\U+0025`. A width factor is AutoCAD's `\W`
//!   code at the start, as its text editor writes it.
//!
//! A drawing's text that the core cannot read back exactly, such as text
//! with formatting codes or a slant, is counted as not read.

use acadrust::entities::{
    AttachmentPoint, DrawingDirection, LineSpacingStyle, MText as DwgMText, Text as DwgText,
    TextHorizontalAlignment as Horizontal, TextVerticalAlignment as Vertical,
};
use acadrust::tables::TextStyle as DwgTextStyle;
use acadrust::types::Vector3;
use acadrust::CadDocument;

use super::{point, vector};
use crate::description::normalize_degrees;
use crate::error::{Error, ErrorCode};
use crate::model::Point3;
use crate::text::{Attachment, Justify, MText, Text, TextStyle, STANDARD, WIDTH_FACTORS};

/// Each justification of a single line of text, with AutoCAD's horizontal
/// and vertical alignment for it.
const JUSTIFICATIONS: [(Justify, Horizontal, Vertical); 13] = [
    (Justify::Left, Horizontal::Left, Vertical::Baseline),
    (Justify::Center, Horizontal::Center, Vertical::Baseline),
    (Justify::Right, Horizontal::Right, Vertical::Baseline),
    (Justify::Middle, Horizontal::Middle, Vertical::Baseline),
    (Justify::TopLeft, Horizontal::Left, Vertical::Top),
    (Justify::TopCenter, Horizontal::Center, Vertical::Top),
    (Justify::TopRight, Horizontal::Right, Vertical::Top),
    (Justify::MiddleLeft, Horizontal::Left, Vertical::Middle),
    (Justify::MiddleCenter, Horizontal::Center, Vertical::Middle),
    (Justify::MiddleRight, Horizontal::Right, Vertical::Middle),
    (Justify::BottomLeft, Horizontal::Left, Vertical::Bottom),
    (Justify::BottomCenter, Horizontal::Center, Vertical::Bottom),
    (Justify::BottomRight, Horizontal::Right, Vertical::Bottom),
];

/// Each attachment of multiline text, with AutoCAD's for it.
const ATTACHMENTS: [(Attachment, AttachmentPoint); 9] = [
    (Attachment::TopLeft, AttachmentPoint::TopLeft),
    (Attachment::TopCenter, AttachmentPoint::TopCenter),
    (Attachment::TopRight, AttachmentPoint::TopRight),
    (Attachment::MiddleLeft, AttachmentPoint::MiddleLeft),
    (Attachment::MiddleCenter, AttachmentPoint::MiddleCenter),
    (Attachment::MiddleRight, AttachmentPoint::MiddleRight),
    (Attachment::BottomLeft, AttachmentPoint::BottomLeft),
    (Attachment::BottomCenter, AttachmentPoint::BottomCenter),
    (Attachment::BottomRight, AttachmentPoint::BottomRight),
];

/// The code multiline text writes a `%` of a run with.
const PERCENT_CODE: &str = "\\U+0025";

/// Adds the sheet's text styles to the drawing. `Standard` is in every
/// drawing: the sheet sets its font and width factor.
pub(super) fn add_text_styles(
    drawing: &mut CadDocument,
    styles: &[TextStyle],
) -> Result<(), Error> {
    for style in styles {
        match drawing.text_styles.get_mut(&style.name) {
            Some(existing) => set_style_properties(existing, style),
            None => {
                let mut dwg_style = DwgTextStyle::new(style.name.clone());
                set_style_properties(&mut dwg_style, style);
                drawing.text_styles.add(dwg_style).map_err(|e| {
                    Error::new(
                        ErrorCode::DwgWrite,
                        format!(
                            "could not add text style `{}` to the drawing: {e}",
                            style.name
                        ),
                    )
                })?;
            }
        }
    }
    Ok(())
}

fn set_style_properties(dwg_style: &mut DwgTextStyle, style: &TextStyle) {
    // The DWG names the font by its file; AutoCAD finds it where it is installed.
    dwg_style.font_file = style.font.clone();
    dwg_style.true_type_font = String::new();
    dwg_style.width_factor = style.width_factor;
}

/// A single line of text whose baseline starts at `start`: its position
/// when it is left-justified, and where AutoCAD would start it otherwise.
pub(super) fn dwg_text(text: &Text, start: Point3) -> DwgText {
    let (_, horizontal, vertical) = JUSTIFICATIONS
        .into_iter()
        .find(|(j, _, _)| *j == text.justify)
        .unwrap_or(JUSTIFICATIONS[0]);
    let mut dwg_text = DwgText::with_value(text_code(&text.contents), vector(start));
    dwg_text.height = text.height;
    dwg_text.rotation = text.rotation.to_radians();
    dwg_text.width_factor = text.width_factor;
    dwg_text.style.clone_from(&text.style);
    dwg_text.horizontal_alignment = horizontal;
    dwg_text.vertical_alignment = vertical;
    if text.justify != Justify::Left {
        // AutoCAD draws the text from its start and keeps the point it is
        // justified at as its alignment point.
        dwg_text.alignment_point = Some(vector(text.position));
    }
    dwg_text
}

pub(super) fn dwg_mtext(text: &MText) -> DwgMText {
    let mut dwg_text = DwgMText::with_value(
        mtext_code(&text.contents, text.width_factor),
        vector(text.position),
    );
    dwg_text.height = text.height;
    dwg_text.rectangle_width = text.width;
    dwg_text.rotation = text.rotation.to_radians();
    dwg_text.style.clone_from(&text.style);
    dwg_text.attachment_point = ATTACHMENTS
        .into_iter()
        .find(|(a, _)| *a == text.justify)
        .map_or(AttachmentPoint::TopLeft, |(_, point)| point);
    dwg_text
}

/// The drawing's text styles the core has a model for, and how many it has not.
pub(super) fn read_text_styles(drawing: &CadDocument) -> (Vec<TextStyle>, usize) {
    let mut styles = Vec::new();
    let mut not_read = 0;
    for dwg_style in drawing.text_styles.iter() {
        // A shape file loaded for a linetype is in the table, but is no text style.
        if dwg_style.is_shape_file {
            continue;
        }
        match read_text_style(dwg_style) {
            Some(style) => styles.push(style),
            None => not_read += 1,
        }
    }
    (styles, not_read)
}

/// A style with a font file, a width factor and nothing else of its own.
fn read_text_style(dwg_style: &DwgTextStyle) -> Option<TextStyle> {
    let (narrowest, widest) = WIDTH_FACTORS;
    let plain = !dwg_style.name.is_empty()
        && !dwg_style.font_file.trim().is_empty()
        && dwg_style.big_font_file.is_empty()
        && dwg_style.height == 0.0
        && dwg_style.oblique_angle == 0.0
        && !dwg_style.flags.backward
        && !dwg_style.flags.upside_down
        && !dwg_style.is_vertical
        && !dwg_style.annotative
        && (narrowest..=widest).contains(&dwg_style.width_factor);
    if !plain {
        return None;
    }
    // A font named without an extension is an SHX font, as AutoCAD reads it.
    let font = if dwg_style.font_file.contains('.') {
        dwg_style.font_file.clone()
    } else {
        format!("{}.shx", dwg_style.font_file)
    };
    Some(TextStyle {
        name: dwg_style.name.clone(),
        font,
        width_factor: dwg_style.width_factor,
    })
}

/// A single line of text the core can read back exactly.
pub(super) fn read_text(dwg_text: &DwgText, layer: String) -> Option<Text> {
    let plain = dwg_text.normal == Vector3::UNIT_Z
        && dwg_text.oblique_angle == 0.0
        && dwg_text.thickness == 0.0
        && dwg_text.generation_flags == 0;
    if !plain {
        return None;
    }
    let (justify, _, _) = JUSTIFICATIONS.into_iter().find(|(_, h, v)| {
        *h == dwg_text.horizontal_alignment && *v == dwg_text.vertical_alignment
    })?;
    let position = if justify == Justify::Left {
        dwg_text.insertion_point
    } else {
        dwg_text.alignment_point?
    };
    Some(Text {
        contents: text_contents(&dwg_text.value)?,
        position: point(position),
        height: dwg_text.height,
        style: style_name(&dwg_text.style),
        width_factor: dwg_text.width_factor,
        rotation: normalize_degrees(dwg_text.rotation.to_degrees()),
        justify,
        layer,
    })
}

/// Multiline text the core can read back exactly: no columns, background,
/// line spacing of its own or formatting codes.
pub(super) fn read_mtext(dwg_text: &DwgMText, layer: String) -> Option<MText> {
    let plain = dwg_text.normal == Vector3::UNIT_Z
        && matches!(
            dwg_text.drawing_direction,
            DrawingDirection::LeftToRight | DrawingDirection::ByStyle
        )
        && dwg_text.line_spacing_factor == 1.0
        && dwg_text.line_spacing_style == LineSpacingStyle::AtLeast
        && dwg_text.background_fill_flags == 0
        && dwg_text.column_data.column_type == 0
        && !dwg_text.is_annotative
        && dwg_text.rectangle_height.unwrap_or(0.0) == 0.0;
    if !plain {
        return None;
    }
    let (contents, width_factor) = mtext_contents(&dwg_text.value)?;
    let (justify, _) = ATTACHMENTS
        .into_iter()
        .find(|(_, a)| *a == dwg_text.attachment_point)?;
    Some(MText {
        contents,
        position: point(dwg_text.insertion_point),
        height: dwg_text.height,
        width: dwg_text.rectangle_width,
        style: style_name(&dwg_text.style),
        width_factor,
        rotation: normalize_degrees(dwg_text.rotation.to_degrees()),
        justify,
        layer,
    })
}

fn style_name(name: &str) -> String {
    if name.is_empty() {
        STANDARD.to_owned()
    } else {
        name.to_owned()
    }
}

/// Codes a single line of text: each `%` of a run of two or more becomes `%%%`.
fn text_code(contents: &str) -> String {
    code_percent_runs(contents, "%%%")
}

/// Each `%` of a run of two or more written as `code`; a lone `%` as it is.
fn code_percent_runs(contents: &str, code: &str) -> String {
    let mut coded = String::with_capacity(contents.len());
    let chars: Vec<char> = contents.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '%' {
            let run = chars[i..].iter().take_while(|&&c| c == '%').count();
            if run == 1 {
                coded.push('%');
            } else {
                coded.extend(std::iter::repeat_n(code, run));
            }
            i += run;
        } else {
            coded.push(chars[i]);
            i += 1;
        }
    }
    coded
}

/// What a single line of text says, decoded, or none when it holds a code
/// other than `%%%`, such as `%%d`, `%%u` or `\U+00B0`.
fn text_contents(value: &str) -> Option<String> {
    let mut contents = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(c) = rest.chars().next() {
        if let Some(after) = rest.strip_prefix("%%%") {
            contents.push('%');
            rest = after;
        } else if ["%%", "\\U+", "\\M+"]
            .iter()
            .any(|code| rest.starts_with(code))
        {
            return None;
        } else {
            contents.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    Some(contents)
}

/// Codes multiline text, with its width factor at the start when it has one.
fn mtext_code(contents: &str, width_factor: Option<f64>) -> String {
    let mut coded = String::with_capacity(contents.len() + 8);
    if let Some(factor) = width_factor {
        coded.push_str(&format!("\\W{factor};"));
    }
    let escaped: String = contents
        .chars()
        .map(|c| match c {
            '\\' => "\\\\".to_owned(),
            '{' => "\\{".to_owned(),
            '}' => "\\}".to_owned(),
            '\n' => "\\P".to_owned(),
            c => c.to_string(),
        })
        .collect();
    coded.push_str(&code_percent_runs(&escaped, PERCENT_CODE));
    coded
}

/// What multiline text says, decoded, and its width factor, or none when
/// it holds a code the core does not model.
fn mtext_contents(value: &str) -> Option<(String, Option<f64>)> {
    let (width_factor, mut rest) = match value.strip_prefix("\\W") {
        Some(after) => {
            let (number, text) = after.split_once(';')?;
            let factor: f64 = number.parse().ok()?;
            let (narrowest, widest) = WIDTH_FACTORS;
            if !(narrowest..=widest).contains(&factor) {
                return None;
            }
            (Some(factor), text)
        }
        None => (None, value),
    };
    let mut contents = String::with_capacity(rest.len());
    while let Some(c) = rest.chars().next() {
        let (decoded, length) = match c {
            '\\' => {
                if rest.starts_with(PERCENT_CODE) {
                    ('%', PERCENT_CODE.len())
                } else {
                    match rest[1..].chars().next()? {
                        '\\' => ('\\', 2),
                        '{' => ('{', 2),
                        '}' => ('}', 2),
                        'P' => ('\n', 2),
                        _ => return None,
                    }
                }
            }
            '{' | '}' => return None,
            // A run of `%` written as it is reads as a code in AutoCAD.
            '%' if rest.starts_with("%%") => return None,
            c => (c, c.len_utf8()),
        };
        contents.push(decoded);
        rest = &rest[length..];
    }
    Some((contents, width_factor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_line_codes_only_runs_of_percent_signs() {
        for (contents, coded) in [
            ("PUMP 101", "PUMP 101"),
            ("50%", "50%"),
            ("%%d", "%%%%%%d"),
            ("a %%% b", "a %%%%%%%%% b"),
            ("% and %%", "% and %%%%%%"),
        ] {
            assert_eq!(text_code(contents), coded);
            assert_eq!(text_contents(coded).as_deref(), Some(contents));
        }
        // Codes the core does not model are not read.
        assert_eq!(text_contents("90%%d"), None);
        assert_eq!(text_contents("\\U+00B0"), None);
    }

    #[test]
    fn multiline_text_escapes_what_autocad_would_read_as_codes() {
        for (contents, width_factor, coded) in [
            ("ONE\nTWO", None, "ONE\\PTWO"),
            ("a\\b {c}", None, "a\\\\b \\{c\\}"),
            ("100%%", Some(0.8), "\\W0.8;100\\U+0025\\U+0025"),
            ("50%", Some(1.0), "\\W1;50%"),
        ] {
            assert_eq!(mtext_code(contents, width_factor), coded);
            assert_eq!(
                mtext_contents(coded),
                Some((contents.to_owned(), width_factor))
            );
        }
        // Formatting codes and groups are not read.
        for value in [
            "\\fArial;x",
            "{\\H2;big}",
            "a\\Lb",
            "\\W0;x",
            "\\Wwide;x",
            "100%%",
        ] {
            assert_eq!(mtext_contents(value), None, "{value}");
        }
    }

    /// AutoCAD draws a single line of text from its start and works the
    /// start out again only when the text is edited (seen in AutoCAD 2026),
    /// so justified text records both points.
    #[test]
    fn justified_text_records_where_it_starts_and_where_it_is_justified() {
        let mut text = Text {
            contents: "A".into(),
            position: Point3::new(10.0, 20.0, 1.0),
            height: 2.5,
            style: STANDARD.into(),
            width_factor: 1.0,
            rotation: 0.0,
            justify: Justify::Right,
            layer: "0".into(),
        };
        let start = Point3::new(7.5, 20.0, 1.0);
        let written = dwg_text(&text, start);
        assert_eq!(written.insertion_point, vector(start));
        assert_eq!(written.alignment_point, Some(vector(text.position)));
        assert_eq!(read_text(&written, "0".into()), Some(text.clone()));
        text.justify = Justify::Left;
        let written = dwg_text(&text, text.position);
        assert_eq!(written.insertion_point, vector(text.position));
        assert_eq!(written.alignment_point, None);
    }

    #[test]
    fn every_justification_and_attachment_has_one_autocad_alignment() {
        for justify in Justify::ALL {
            assert_eq!(
                JUSTIFICATIONS
                    .iter()
                    .filter(|(j, _, _)| *j == justify)
                    .count(),
                1
            );
        }
        for (i, (_, h, v)) in JUSTIFICATIONS.iter().enumerate() {
            assert!(JUSTIFICATIONS[..i]
                .iter()
                .all(|(_, h2, v2)| (h2, v2) != (h, v)));
        }
        for (i, attachment) in Attachment::ALL.into_iter().enumerate() {
            assert_eq!(ATTACHMENTS[i].0, attachment);
            assert_eq!(ATTACHMENTS[i].1 as usize, i + 1);
        }
    }

    /// `CONTEXT.md` says drawing and object. acadrust's own names, which
    /// say document and entity, are allowed; ours are not.
    #[test]
    fn our_names_here_say_drawing_and_object() {
        let source = include_str!("text.rs");
        let (code, _tests) = source.split_once("#[cfg(test)]").unwrap();
        let mut ours = code.to_owned();
        for acadrust_name in ["CadDocument", "acadrust::entities"] {
            ours = ours.replace(acadrust_name, "");
        }
        for word in ["document", "entit"] {
            let found: Vec<&str> = ours
                .lines()
                .filter(|line| line.to_lowercase().contains(word))
                .collect();
            assert!(found.is_empty(), "`{word}` in our own names: {found:#?}");
        }
    }
}
