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
use crate::model::{ObjectColor, Point3};
use crate::text::{
    Attachment, FontKind, Justify, MText, MTextRun, Text, TextStyle, OBLIQUE_ANGLES, STANDARD,
    TRACKING_FACTORS, WIDTH_FACTORS,
};

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

/// The attachment of multiline text that AutoCAD's `point` is.
pub(super) fn attachment_of(point: AttachmentPoint) -> Option<Attachment> {
    ATTACHMENTS
        .into_iter()
        .find(|(_, p)| *p == point)
        .map(|(attachment, _)| attachment)
}

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
    dwg_style.annotative = style.annotative;
}

/// A single line of text whose baseline starts at `start`: its position
/// when it is left-justified, and where AutoCAD would start it otherwise.
pub(super) fn dwg_text(text: &Text, start: Point3) -> DwgText {
    let (_, horizontal, vertical) = JUSTIFICATIONS
        .into_iter()
        .find(|(j, _, _)| *j == text.justify)
        .unwrap_or(JUSTIFICATIONS[0]);
    let mut dwg_text = DwgText::with_value(
        text_styled_code(&text.contents, text.underline, text.overline),
        vector(start),
    );
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
    let mut dwg_text = DwgMText::with_value(mtext_rich_code(text), vector(text.position));
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
        annotative: dwg_style.annotative,
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
    let (contents, underline, overline) = text_styled_contents(&dwg_text.value)?;
    Some(Text {
        contents,
        position: point(position),
        height: dwg_text.height,
        style: style_name(&dwg_text.style),
        width_factor: dwg_text.width_factor,
        rotation: normalize_degrees(dwg_text.rotation.to_degrees()),
        justify,
        underline,
        overline,
        layer,
        field: None,
        annotative: false,
        annotation_scales: Vec::new(),
        properties: Default::default(),
    })
}

/// Multiline text the core can read back exactly: no columns, background,
/// line spacing of its own or inline formatting codes beyond whole-text
/// underline/overline and width factor.
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
        && dwg_text.rectangle_height.unwrap_or(0.0) == 0.0;
    if !plain {
        return None;
    }
    let parsed = mtext_rich_contents(&dwg_text.value, dwg_text.height)?;
    let (justify, _) = ATTACHMENTS
        .into_iter()
        .find(|(_, a)| *a == dwg_text.attachment_point)?;
    Some(MText {
        contents: parsed.contents,
        runs: parsed.runs,
        position: point(dwg_text.insertion_point),
        height: dwg_text.height,
        width: dwg_text.rectangle_width,
        style: style_name(&dwg_text.style),
        width_factor: parsed.width_factor,
        rotation: normalize_degrees(dwg_text.rotation.to_degrees()),
        justify,
        underline: parsed.underline,
        overline: parsed.overline,
        layer,
        field: None,
        annotative: dwg_text.is_annotative,
        annotation_scales: Vec::new(),
        properties: Default::default(),
    })
}

pub(super) fn style_name(name: &str) -> String {
    if name.is_empty() {
        STANDARD.to_owned()
    } else {
        name.to_owned()
    }
}

/// Codes a single line of text: each `%` of a run of two or more becomes `%%%`.
pub(super) fn text_code(contents: &str) -> String {
    code_percent_runs(contents, "%%%")
}

/// Codes a single line of text with whole-line AutoCAD `%%u` (underline) and
/// `%%o` (overline) switches (#60).
pub(super) fn text_styled_code(contents: &str, underline: bool, overline: bool) -> String {
    let mut coded = String::with_capacity(contents.len() + 6);
    if underline {
        coded.push_str("%%u");
    }
    if overline {
        coded.push_str("%%o");
    }
    coded.push_str(&text_code(contents));
    coded
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
pub(super) fn text_contents(value: &str) -> Option<String> {
    let (contents, underline, overline) = text_styled_contents(value)?;
    (!underline && !overline).then_some(contents)
}

/// Decodes a single line of text, honouring whole-line `%%u`/`%%U` (underline)
/// and `%%o`/`%%O` (overline) switches (#60). Returns `None` if partial
/// mid-string underline/overline toggling or any other unmodelled code
/// (`%%d`, `\U+`, `\M+`) is present.
pub(super) fn text_styled_contents(value: &str) -> Option<(String, bool, bool)> {
    let mut contents = String::with_capacity(value.len());
    let mut rest = value;
    let mut cur_u = false;
    let mut cur_o = false;
    let mut seen: Option<(bool, bool)> = None;
    while let Some(c) = rest.chars().next() {
        if let Some(after) = rest.strip_prefix("%%%") {
            if let Some(prev) = seen {
                if prev != (cur_u, cur_o) {
                    return None;
                }
            } else {
                seen = Some((cur_u, cur_o));
            }
            contents.push('%');
            rest = after;
        } else if let Some(after) = rest
            .strip_prefix("%%u")
            .or_else(|| rest.strip_prefix("%%U"))
        {
            cur_u = !cur_u;
            rest = after;
        } else if let Some(after) = rest
            .strip_prefix("%%o")
            .or_else(|| rest.strip_prefix("%%O"))
        {
            cur_o = !cur_o;
            rest = after;
        } else if ["%%", "\\U+", "\\M+"]
            .iter()
            .any(|code| rest.starts_with(code))
        {
            return None;
        } else {
            if let Some(prev) = seen {
                if prev != (cur_u, cur_o) {
                    return None;
                }
            } else {
                seen = Some((cur_u, cur_o));
            }
            contents.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    let (underline, overline) = seen.unwrap_or((cur_u, cur_o));
    Some((contents, underline, overline))
}

/// Codes multiline text, with its width factor at the start when it has one.
pub(super) fn mtext_code(contents: &str, width_factor: Option<f64>) -> String {
    mtext_styled_code(contents, width_factor, false, false)
}

/// Codes multiline text with optional width factor and whole-text underline
/// (`\L...\l`) and overline (`\O...\o`) switches (#60).
pub(super) fn mtext_styled_code(
    contents: &str,
    width_factor: Option<f64>,
    underline: bool,
    overline: bool,
) -> String {
    let mut coded = String::with_capacity(contents.len() + 16);
    if let Some(factor) = width_factor {
        coded.push_str(&format!("\\W{factor};"));
    }
    if underline {
        coded.push_str("\\L");
    }
    if overline {
        coded.push_str("\\O");
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
    if overline {
        coded.push_str("\\o");
    }
    if underline {
        coded.push_str("\\l");
    }
    coded
}

/// What multiline text says, decoded, and its width factor, or none when
/// it holds a code the core does not model (including underline/overline).
pub(super) fn mtext_contents(value: &str) -> Option<(String, Option<f64>)> {
    let (contents, width_factor, underline, overline) = mtext_styled_contents(value)?;
    (!underline && !overline).then_some((contents, width_factor))
}

/// Decodes multiline text, its optional width factor, and whole-text
/// underline (`\L`/`\l`) and overline (`\O`/`\o`) switches (#60). Returns
/// `None` if partial formatting or any unmodelled code is present.
pub(super) fn mtext_styled_contents(value: &str) -> Option<(String, Option<f64>, bool, bool)> {
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
    let mut cur_u = false;
    let mut cur_o = false;
    let mut seen: Option<(bool, bool)> = None;
    while let Some(c) = rest.chars().next() {
        let (decoded, length) = match c {
            '\\' => {
                if rest.starts_with(PERCENT_CODE) {
                    (Some('%'), PERCENT_CODE.len())
                } else {
                    match rest[1..].chars().next()? {
                        '\\' => (Some('\\'), 2),
                        '{' => (Some('{'), 2),
                        '}' => (Some('}'), 2),
                        'P' => (Some('\n'), 2),
                        'L' => {
                            cur_u = true;
                            (None, 2)
                        }
                        'l' => {
                            cur_u = false;
                            (None, 2)
                        }
                        'O' => {
                            cur_o = true;
                            (None, 2)
                        }
                        'o' => {
                            cur_o = false;
                            (None, 2)
                        }
                        _ => return None,
                    }
                }
            }
            '{' | '}' => return None,
            // A run of `%` written as it is reads as a code in AutoCAD.
            '%' if rest.starts_with("%%") => return None,
            c => (Some(c), c.len_utf8()),
        };
        if let Some(ch) = decoded {
            if ch != '\n' {
                if let Some(prev) = seen {
                    if prev != (cur_u, cur_o) {
                        return None;
                    }
                } else {
                    seen = Some((cur_u, cur_o));
                }
            }
            contents.push(ch);
        }
        rest = &rest[length..];
    }
    let (underline, overline) = seen.unwrap_or((cur_u, cur_o));
    Some((contents, width_factor, underline, overline))
}

/// Decoded multiline text, with optional inline-formatted runs (#61).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ParsedMText {
    pub contents: String,
    pub width_factor: Option<f64>,
    pub underline: bool,
    pub overline: bool,
    pub runs: Vec<MTextRun>,
}

/// Escapes plain text for an AutoCAD MTEXT body (`\`, `{`, `}`, `\n`, `%%`).
fn mtext_escape_body(text: &str) -> String {
    let escaped: String = text
        .chars()
        .map(|c| match c {
            '\\' => "\\\\".to_owned(),
            '{' => "\\{".to_owned(),
            '}' => "\\}".to_owned(),
            '\n' => "\\P".to_owned(),
            c => c.to_string(),
        })
        .collect();
    code_percent_runs(&escaped, PERCENT_CODE)
}

/// Codes a rich multiline text for DWG/DXF: delegates to [`mtext_styled_code`]
/// when `runs` is empty, otherwise emits AutoCAD inline formatting groups
/// (`\f`/`\F`, `\H`, `\W`, `\Q`, `\T`, `\C`, `\L`/`\l`, `\O`/`\o`) (#61).
pub(super) fn mtext_rich_code(text: &MText) -> String {
    if text.runs.is_empty() {
        return mtext_styled_code(
            &text.contents,
            text.width_factor,
            text.underline,
            text.overline,
        );
    }
    let mut coded = String::new();
    if let Some(factor) = text.width_factor {
        coded.push_str(&format!("\\W{factor};"));
    }
    if text.underline {
        coded.push_str("\\L");
    }
    if text.overline {
        coded.push_str("\\O");
    }
    for run in &text.runs {
        let has_override = !run.is_plain();
        if has_override {
            coded.push('{');
            if let Some(font) = &run.font {
                if FontKind::of(font) == Some(FontKind::Shx) && !run.bold && !run.italic {
                    coded.push_str(&format!("\\F{font};"));
                } else {
                    let b = u8::from(run.bold);
                    let i = u8::from(run.italic);
                    coded.push_str(&format!("\\f{font}|b{b}|i{i}|c0|p0;"));
                }
            } else if run.bold || run.italic {
                let b = u8::from(run.bold);
                let i = u8::from(run.italic);
                coded.push_str(&format!("\\f|b{b}|i{i}|c0|p0;"));
            }
            if let Some(h) = run.height {
                coded.push_str(&format!("\\H{h};"));
            }
            if let Some(w) = run.width_factor {
                coded.push_str(&format!("\\W{w};"));
            }
            if let Some(q) = run.oblique {
                coded.push_str(&format!("\\Q{q};"));
            }
            if let Some(t) = run.tracking {
                coded.push_str(&format!("\\T{t};"));
            }
            if let Some(color) = run.color {
                let aci = match color {
                    ObjectColor::ByBlock => 0u16,
                    ObjectColor::Index(i) => u16::from(i),
                    ObjectColor::ByLayer => 256u16,
                };
                coded.push_str(&format!("\\C{aci};"));
            }
            if run.underline && !text.underline {
                coded.push_str("\\L");
            }
            if run.overline && !text.overline {
                coded.push_str("\\O");
            }
        }
        coded.push_str(&mtext_escape_body(&run.text));
        if has_override {
            if run.overline && !text.overline {
                coded.push_str("\\o");
            }
            if run.underline && !text.underline {
                coded.push_str("\\l");
            }
            coded.push('}');
        }
    }
    if text.overline {
        coded.push_str("\\o");
    }
    if text.underline {
        coded.push_str("\\l");
    }
    coded
}

/// Active inline formatting state while parsing an AutoCAD MTEXT string (#61).
#[derive(Clone, Debug, Default, PartialEq)]
struct MTextState {
    font: Option<String>,
    bold: bool,
    italic: bool,
    height: Option<f64>,
    width_factor: Option<f64>,
    oblique: Option<f64>,
    tracking: Option<f64>,
    underline: bool,
    overline: bool,
    color: Option<ObjectColor>,
}

/// Parses an AutoCAD MTEXT string with inline formatting codes (`\f`/`\F`,
/// `\H`, `\W`, `\Q`, `\T`, `\L`/`\l`, `\O`/`\o`, `\C`, `\P`/`\N`, `\~`,
/// `\U+XXXX`, `\A`, and `{...}` groups) (#61).
///
/// Returns `None` if an unmodelled code (`\S`, `\p`, `\c`, `\K`, unmatched
/// braces, raw `%%`, or out-of-range numeric parameter) is encountered so
/// originals with unsupported formatting are never silently flattened.
pub(super) fn mtext_rich_contents(value: &str, base_height: f64) -> Option<ParsedMText> {
    // Fast path: plain MTEXT (with optional whole-text \W / \L / \O) keeps
    // `runs` empty for full backwards compatibility.
    if let Some((contents, width_factor, underline, overline)) = mtext_styled_contents(value) {
        return Some(ParsedMText {
            contents,
            width_factor,
            underline,
            overline,
            runs: Vec::new(),
        });
    }

    let mut stack: Vec<MTextState> = Vec::new();
    let mut state = MTextState::default();
    let mut runs: Vec<MTextRun> = Vec::new();
    let mut rest = value;

    let push_char = |runs: &mut Vec<MTextRun>, st: &MTextState, ch: char| {
        let candidate = MTextRun {
            text: String::new(),
            font: st.font.clone(),
            bold: st.bold,
            italic: st.italic,
            height: st.height,
            width_factor: st.width_factor,
            oblique: st.oblique,
            tracking: st.tracking,
            underline: st.underline,
            overline: st.overline,
            color: st.color,
        };
        if let Some(last) = runs.last_mut() {
            if last.same_style(&candidate) {
                last.text.push(ch);
                return;
            }
        }
        let mut run = candidate;
        run.text.push(ch);
        runs.push(run);
    };

    while let Some(c) = rest.chars().next() {
        match c {
            '{' => {
                stack.push(state.clone());
                rest = &rest[1..];
            }
            '}' => {
                state = stack.pop()?;
                rest = &rest[1..];
            }
            '%' if rest.starts_with("%%") => return None,
            '\\' => {
                let after_bs = &rest[1..];
                let code = after_bs.chars().next()?;
                match code {
                    '\\' => {
                        push_char(&mut runs, &state, '\\');
                        rest = &after_bs[1..];
                    }
                    '{' => {
                        push_char(&mut runs, &state, '{');
                        rest = &after_bs[1..];
                    }
                    '}' => {
                        push_char(&mut runs, &state, '}');
                        rest = &after_bs[1..];
                    }
                    '~' => {
                        push_char(&mut runs, &state, ' ');
                        rest = &after_bs[1..];
                    }
                    'P' | 'N' => {
                        push_char(&mut runs, &state, '\n');
                        rest = &after_bs[1..];
                    }
                    'L' => {
                        state.underline = true;
                        rest = &after_bs[1..];
                    }
                    'l' => {
                        state.underline = false;
                        rest = &after_bs[1..];
                    }
                    'O' => {
                        state.overline = true;
                        rest = &after_bs[1..];
                    }
                    'o' => {
                        state.overline = false;
                        rest = &after_bs[1..];
                    }
                    'U' | 'u' if after_bs.get(1..2) == Some("+") => {
                        let hex = after_bs.get(2..6)?;
                        let cp = u32::from_str_radix(hex, 16).ok()?;
                        let ch = char::from_u32(cp)?;
                        push_char(&mut runs, &state, ch);
                        rest = &after_bs[6..];
                    }
                    'f' => {
                        let (arg, tail) = after_bs[1..].split_once(';')?;
                        let mut parts = arg.split('|');
                        let family = parts.next().unwrap_or("").trim();
                        if !family.is_empty() {
                            if crate::text::inline_font_name_problem(family).is_some() {
                                return None;
                            }
                            state.font = Some(family.to_owned());
                        }
                        for flag in parts {
                            if let Some(v) =
                                flag.strip_prefix('b').or_else(|| flag.strip_prefix('B'))
                            {
                                state.bold = v != "0";
                            } else if let Some(v) =
                                flag.strip_prefix('i').or_else(|| flag.strip_prefix('I'))
                            {
                                state.italic = v != "0";
                            } else if flag.starts_with(['c', 'C', 'p', 'P']) {
                                // Windows charset / pitch-and-family metadata: accepted.
                            } else if !flag.is_empty() {
                                return None;
                            }
                        }
                        rest = tail;
                    }
                    'F' => {
                        let (arg, tail) = after_bs[1..].split_once(';')?;
                        let name = arg.trim();
                        if name.is_empty() || crate::text::inline_font_name_problem(name).is_some()
                        {
                            return None;
                        }
                        state.font = Some(name.to_owned());
                        state.bold = false;
                        state.italic = false;
                        rest = tail;
                    }
                    'H' => {
                        let (arg, tail) = after_bs[1..].split_once(';')?;
                        let h = if let Some(num) =
                            arg.strip_suffix('x').or_else(|| arg.strip_suffix('X'))
                        {
                            let factor: f64 = num.parse().ok()?;
                            if !factor.is_finite() || factor <= 0.0 {
                                return None;
                            }
                            state.height.unwrap_or(base_height) * factor
                        } else {
                            let abs: f64 = arg.parse().ok()?;
                            if !abs.is_finite() || abs <= 0.0 {
                                return None;
                            }
                            abs
                        };
                        state.height = Some(h);
                        rest = tail;
                    }
                    'W' => {
                        let (arg, tail) = after_bs[1..].split_once(';')?;
                        let wf = if let Some(num) =
                            arg.strip_suffix('x').or_else(|| arg.strip_suffix('X'))
                        {
                            let factor: f64 = num.parse().ok()?;
                            if !factor.is_finite() || factor <= 0.0 {
                                return None;
                            }
                            state.width_factor.unwrap_or(1.0) * factor
                        } else {
                            arg.parse::<f64>().ok()?
                        };
                        if !wf.is_finite() || !(WIDTH_FACTORS.0..=WIDTH_FACTORS.1).contains(&wf) {
                            return None;
                        }
                        state.width_factor = Some(wf);
                        rest = tail;
                    }
                    'Q' => {
                        let (arg, tail) = after_bs[1..].split_once(';')?;
                        let ang: f64 = arg.parse().ok()?;
                        if !ang.is_finite() || !(OBLIQUE_ANGLES.0..=OBLIQUE_ANGLES.1).contains(&ang)
                        {
                            return None;
                        }
                        state.oblique = Some(ang);
                        rest = tail;
                    }
                    'T' => {
                        let (arg, tail) = after_bs[1..].split_once(';')?;
                        let trk: f64 = arg.parse().ok()?;
                        if !trk.is_finite()
                            || !(TRACKING_FACTORS.0..=TRACKING_FACTORS.1).contains(&trk)
                        {
                            return None;
                        }
                        state.tracking = Some(trk);
                        rest = tail;
                    }
                    'C' => {
                        let (arg, tail) = after_bs[1..].split_once(';')?;
                        let aci: u16 = arg.parse().ok()?;
                        state.color = match aci {
                            0 => Some(ObjectColor::ByBlock),
                            1..=255 => Some(ObjectColor::Index(aci as u8)),
                            256 => Some(ObjectColor::ByLayer),
                            _ => return None,
                        };
                        rest = tail;
                    }
                    'A' => {
                        // Baseline alignment (`\A0;`, `\A1;`, `\A2;`): accepted.
                        let (arg, tail) = after_bs[1..].split_once(';')?;
                        if !matches!(arg, "0" | "1" | "2") {
                            return None;
                        }
                        rest = tail;
                    }
                    _ => return None,
                }
            }
            ch => {
                push_char(&mut runs, &state, ch);
                rest = &rest[ch.len_utf8()..];
            }
        }
    }
    if !stack.is_empty() {
        return None;
    }

    let contents: String = runs.iter().map(|r| r.text.as_str()).collect();
    // If all runs are plain (e.g. only `\A1;` or empty `{}` groups were present),
    // collapse back to plain `MText` with empty `runs`.
    if runs.iter().all(MTextRun::is_plain) {
        return Some(ParsedMText {
            contents,
            width_factor: None,
            underline: false,
            overline: false,
            runs: Vec::new(),
        });
    }
    Some(ParsedMText {
        contents,
        width_factor: None,
        underline: false,
        overline: false,
        runs,
    })
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
        assert_eq!(text_contents("%%uNOTES"), None);
    }

    #[test]
    fn single_line_underline_and_overline_round_trip() {
        for (contents, u, o, coded) in [
            ("NOTES:", true, false, "%%uNOTES:"),
            ("HEADER", false, true, "%%oHEADER"),
            ("TITLE", true, true, "%%u%%oTITLE"),
            ("50%%", true, false, "%%u50%%%%%%"),
            ("%%u", true, false, "%%u%%%%%%u"),
            ("PLAIN", false, false, "PLAIN"),
        ] {
            assert_eq!(text_styled_code(contents, u, o), coded);
            assert_eq!(
                text_styled_contents(coded),
                Some((contents.to_owned(), u, o))
            );
        }
        // Uppercase codes, reversed order and optional closing switches read back.
        assert_eq!(
            text_styled_contents("%%UNOTES:%%U"),
            Some(("NOTES:".to_owned(), true, false))
        );
        assert_eq!(
            text_styled_contents("%%O%%uBOTH%%u%%O"),
            Some(("BOTH".to_owned(), true, true))
        );
        // Partial mid-string toggling and other codes are not flattened.
        for unmodelled in ["A %%uB%%u C", "%%uA%%u B", "A %%oB", "90%%d", "\\U+00B0"] {
            assert_eq!(text_styled_contents(unmodelled), None, "{unmodelled}");
        }
    }

    #[test]
    fn multiline_underline_and_overline_round_trip() {
        for (contents, wf, u, o, coded) in [
            ("NOTES:", None, true, false, "\\LNOTES:\\l"),
            ("ONE\nTWO", Some(0.8), false, true, "\\W0.8;\\OONE\\PTWO\\o"),
            ("BOTH", None, true, true, "\\L\\OBOTH\\o\\l"),
        ] {
            assert_eq!(mtext_styled_code(contents, wf, u, o), coded);
            assert_eq!(
                mtext_styled_contents(coded),
                Some((contents.to_owned(), wf, u, o))
            );
        }
        // Unclosed whole-text switches read back too.
        assert_eq!(
            mtext_styled_contents("\\LNOTES:"),
            Some(("NOTES:".to_owned(), None, true, false))
        );
        // Partial inline formatting is still refused by the plain-text helper.
        assert_eq!(mtext_styled_contents("a\\Lb"), None);
        assert_eq!(mtext_styled_contents("\\La\\lb"), None);
    }

    #[test]
    fn rich_mtext_inline_codes_parse_and_round_trip() {
        // Real-job pattern from #61: `{\fSomeFont|b0|i0|c0|p0;...}`.
        let parsed = mtext_rich_contents("{\\fSomeFont|b0|i0|c0|p0;NOTE:} plain", 2.5).unwrap();
        assert_eq!(parsed.contents, "NOTE: plain");
        assert_eq!(parsed.runs.len(), 2);
        assert_eq!(parsed.runs[0].text, "NOTE:");
        assert_eq!(parsed.runs[0].font.as_deref(), Some("SomeFont"));
        assert!(!parsed.runs[0].bold && !parsed.runs[0].italic);
        assert_eq!(parsed.runs[1].text, " plain");
        assert!(parsed.runs[1].is_plain());

        // Full combination: font, bold, italic, absolute & relative height,
        // width factor, oblique, tracking, underline, overline, colour, nesting,
        // non-breaking space, unicode escape and \A1;.
        let raw = "\\A1;{\\fArial|b1|i1|c0|p34;\\H2x;\\C1;\\LRED\\l} {\\Fromans.shx;\\H3.5;\\W0.8;\\Q15;\\T1.2;\\OTOP\\o}\\P{\\C256;90\\U+00B0\\~C}";
        let parsed = mtext_rich_contents(raw, 2.5).unwrap();
        assert_eq!(parsed.contents, "RED TOP\n90° C");
        assert_eq!(parsed.runs[0].font.as_deref(), Some("Arial"));
        assert!(parsed.runs[0].bold && parsed.runs[0].italic && parsed.runs[0].underline);
        assert_eq!(parsed.runs[0].height, Some(5.0)); // 2x * 2.5
        assert_eq!(parsed.runs[0].color, Some(ObjectColor::Index(1)));
        assert_eq!(parsed.runs[1].text, " ");
        assert!(parsed.runs[1].is_plain());
        assert_eq!(parsed.runs[2].font.as_deref(), Some("romans.shx"));
        assert_eq!(parsed.runs[2].height, Some(3.5));
        assert_eq!(parsed.runs[2].width_factor, Some(0.8));
        assert_eq!(parsed.runs[2].oblique, Some(15.0));
        assert_eq!(parsed.runs[2].tracking, Some(1.2));
        assert!(parsed.runs[2].overline);
        let last = parsed.runs.last().unwrap();
        assert_eq!(last.text, "90° C");
        assert_eq!(last.color, Some(ObjectColor::ByLayer));

        // Round-trip through `mtext_rich_code` -> `mtext_rich_contents`.
        let mt = MText {
            contents: parsed.contents.clone(),
            runs: parsed.runs.clone(),
            position: Point3::new(0.0, 0.0, 0.0),
            height: 2.5,
            width: 0.0,
            style: STANDARD.into(),
            width_factor: None,
            rotation: 0.0,
            justify: Attachment::TopLeft,
            underline: false,
            overline: false,
            layer: "0".into(),
            field: None,
            annotative: false,
            annotation_scales: Vec::new(),
            properties: Default::default(),
        };
        let encoded = mtext_rich_code(&mt);
        let decoded = mtext_rich_contents(&encoded, 2.5).unwrap();
        assert_eq!(decoded.contents, mt.contents);
        assert_eq!(decoded.runs, mt.runs);

        // Partial underline works in rich MTEXT.
        let partial = mtext_rich_contents("a\\Lb", 2.0).unwrap();
        assert_eq!(partial.contents, "ab");
        assert_eq!(partial.runs.len(), 2);
        assert!(!partial.runs[0].underline && partial.runs[1].underline);

        // Unmodelled codes and malformed inputs still return None.
        for bad in [
            "\\S1/2;",
            "\\pqc;centered",
            "\\c255;truecolor",
            "\\Kstrike\\k",
            "{\\fArial;unclosed",
            "\\H0;zero",
            "\\W0;zero",
            "\\Q90;too-slanted",
            "\\T0.5;too-tight",
            "\\C257;bad-aci",
            "100%%",
        ] {
            assert_eq!(mtext_rich_contents(bad, 2.5), None, "{bad}");
        }
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
            underline: false,
            overline: false,
            layer: "0".into(),
            field: None,
            properties: Default::default(),
            annotative: false,
            annotation_scales: Vec::new(),
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
