//! Text on a sheet: single-line text, multiline text and the text styles
//! they draw with, and the fonts behind them.
//!
//! A text style names a font by its file name: an AutoCAD SHX font such as
//! `romans.shx`, or a TrueType font such as `arial.ttf`. The DWG carries the
//! name and AutoCAD finds the font on the machine that opens it.
//! AgentDrafter reads the font itself only to measure text ([`measure`]):
//! [`fonts`] finds it in the font folders a setting names, and uses a
//! look-alike when it is not there. [`shx`] reads SHX fonts, [`truetype`]
//! reads TrueType metrics, and [`hershey`] holds the look-alike that is
//! always at hand. [`placement`] works out where justified text starts,
//! which the DWG records. [`outline`] gives what each character draws,
//! which the drawing pipeline ([`crate::draw`]) lays out.

pub(crate) mod fonts;
mod hershey;
pub(crate) mod measure;
pub(crate) mod outline;
pub(crate) mod placement;
pub(crate) mod shx;
pub(crate) mod truetype;

use serde::{Deserialize, Serialize};

use crate::model::Point3;

/// The text style every drawing has. Text draws with it unless it names
/// another; a sheet may list it to set its font.
pub const STANDARD: &str = "Standard";

/// The font of [`STANDARD`] in a drawing whose sheet does not set it:
/// AutoCAD's own `txt.shx`, which a new drawing starts with.
pub(crate) const STANDARD_FONT: &str = "txt.shx";

/// The narrowest and the widest width factor AutoCAD allows.
pub(crate) const WIDTH_FACTORS: (f64, f64) = (0.01, 100.0);

fn standard() -> String {
    STANDARD.to_owned()
}

fn one() -> f64 {
    1.0
}

fn is_one(value: &f64) -> bool {
    *value == 1.0
}

/// A named font that text draws with, and how wide its characters are.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextStyle {
    pub name: String,
    /// The font's file name: an SHX font such as `romans.shx`, or a
    /// TrueType font such as `arial.ttf`.
    pub font: String,
    /// How wide characters are drawn, as a share of their normal width:
    /// 1 is normal, 0.8 is narrower. Text in the style takes it unless it
    /// states its own.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub width_factor: f64,
}

impl TextStyle {
    /// A style with a normal width factor.
    pub fn new(name: impl Into<String>, font: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            font: font.into(),
            width_factor: 1.0,
        }
    }

    /// The style every drawing has, with the font a new drawing gives it.
    pub fn standard() -> Self {
        Self::new(STANDARD, STANDARD_FONT)
    }
}

/// Characters a font's file name cannot hold: the folder separators `/` and
/// `\`, the drive separator `:`, and the others Windows refuses in a file
/// name.
pub(crate) const FORBIDDEN_IN_FONT_NAMES: &str = "/\\<>:\"|?*";

/// What a font is, for the messages that refuse one.
pub(crate) const FONT_CHOICES: &str =
    "a font is the file name of an SHX font, such as `romans.shx`, or a TrueType font, such as `arial.ttf`";

/// Why `name` cannot name a font, or none when it can. A font is named by
/// the file name alone of an SHX or a TrueType font and is read from the
/// font folders only, so a name that holds a folder, a drive or a parent
/// reference, which would reach outside them, is refused. The sheet
/// description and measuring both check a font's name here.
pub(crate) fn font_name_problem(name: &str) -> Option<String> {
    if let Some(c) = name
        .chars()
        .find(|c| FORBIDDEN_IN_FONT_NAMES.contains(*c) || c.is_control())
    {
        Some(format!(
            "a font is named by its file name alone, with no folder, drive or parent folder, so it cannot hold `{}`; {FONT_CHOICES}",
            c.escape_default()
        ))
    } else if FontKind::of(name).is_none() {
        Some(format!("`{name}` is not a font file name; {FONT_CHOICES}"))
    } else {
        None
    }
}

/// The kind of font a file name names, told by its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FontKind {
    /// An AutoCAD SHX font, drawn with strokes: `.shx`.
    Shx,
    /// A TrueType font, drawn with filled outlines: `.ttf`.
    TrueType,
}

impl FontKind {
    /// The kind of font `file` names, by its extension in any case, or none
    /// when it is neither `.shx` nor `.ttf`.
    pub fn of(file: &str) -> Option<FontKind> {
        let (stem, extension) = file.rsplit_once('.')?;
        if stem.is_empty() {
            None
        } else if extension.eq_ignore_ascii_case("shx") {
            Some(FontKind::Shx)
        } else if extension.eq_ignore_ascii_case("ttf") {
            Some(FontKind::TrueType)
        } else {
            None
        }
    }
}

/// Where a single line of text sits against its position, as AutoCAD's
/// TEXT command justifies it.
///
/// `Left`, `Center` and `Right` sit on the baseline; `Middle` centres the
/// text both ways. The others name the top, middle or bottom of the
/// capitals (bottom is the lowest descender) and the left, centre or right.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Justify {
    #[default]
    Left,
    Center,
    Right,
    Middle,
    TopLeft,
    TopCenter,
    TopRight,
    MiddleLeft,
    MiddleCenter,
    MiddleRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl Justify {
    /// Every justification, in the order error messages list them.
    pub const ALL: [Justify; 13] = [
        Justify::Left,
        Justify::Center,
        Justify::Right,
        Justify::Middle,
        Justify::TopLeft,
        Justify::TopCenter,
        Justify::TopRight,
        Justify::MiddleLeft,
        Justify::MiddleCenter,
        Justify::MiddleRight,
        Justify::BottomLeft,
        Justify::BottomCenter,
        Justify::BottomRight,
    ];

    /// The name a sheet description writes, such as `top_left`.
    pub const fn name(self) -> &'static str {
        match self {
            Justify::Left => "left",
            Justify::Center => "center",
            Justify::Right => "right",
            Justify::Middle => "middle",
            Justify::TopLeft => "top_left",
            Justify::TopCenter => "top_center",
            Justify::TopRight => "top_right",
            Justify::MiddleLeft => "middle_left",
            Justify::MiddleCenter => "middle_center",
            Justify::MiddleRight => "middle_right",
            Justify::BottomLeft => "bottom_left",
            Justify::BottomCenter => "bottom_center",
            Justify::BottomRight => "bottom_right",
        }
    }
}

/// Where multiline text sits against its position: the corner, edge
/// middle or centre of the text's box, as AutoCAD's MTEXT justifies it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Attachment {
    #[default]
    TopLeft,
    TopCenter,
    TopRight,
    MiddleLeft,
    MiddleCenter,
    MiddleRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl Attachment {
    /// Every attachment, in the order error messages list them, which is
    /// also AutoCAD's order, from 1.
    pub const ALL: [Attachment; 9] = [
        Attachment::TopLeft,
        Attachment::TopCenter,
        Attachment::TopRight,
        Attachment::MiddleLeft,
        Attachment::MiddleCenter,
        Attachment::MiddleRight,
        Attachment::BottomLeft,
        Attachment::BottomCenter,
        Attachment::BottomRight,
    ];

    /// The name a sheet description writes, such as `top_left`.
    pub const fn name(self) -> &'static str {
        match self {
            Attachment::TopLeft => "top_left",
            Attachment::TopCenter => "top_center",
            Attachment::TopRight => "top_right",
            Attachment::MiddleLeft => "middle_left",
            Attachment::MiddleCenter => "middle_center",
            Attachment::MiddleRight => "middle_right",
            Attachment::BottomLeft => "bottom_left",
            Attachment::BottomCenter => "bottom_center",
            Attachment::BottomRight => "bottom_right",
        }
    }
}

/// A single line of text, as AutoCAD's TEXT draws it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Text {
    /// What the text says, exactly as it shows: one line.
    pub contents: String,
    /// The point the text is justified at.
    pub position: Point3,
    /// The height of its capitals, in the declared unit.
    pub height: f64,
    /// The text style it draws with.
    #[serde(default = "standard")]
    pub style: String,
    /// How wide its characters are, as a share of their normal width.
    #[serde(default = "one")]
    pub width_factor: f64,
    /// Degrees anticlockwise from the X axis, in `[0, 360)`.
    #[serde(default)]
    pub rotation: f64,
    /// Where the text sits against its position.
    #[serde(default)]
    pub justify: Justify,
    #[serde(default = "crate::model::layer_zero")]
    pub layer: String,
}

/// Text of one or more lines in a box, as AutoCAD's MTEXT draws it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MText {
    /// What the text says, exactly as it shows; lines are separated by a
    /// line break (`\n`).
    pub contents: String,
    /// The point the text's box is attached at.
    pub position: Point3,
    /// The height of its capitals, in the declared unit.
    pub height: f64,
    /// The width of the box lines wrap in, in the declared unit; 0 when
    /// lines break only where the contents break them.
    #[serde(default)]
    pub width: f64,
    /// The text style it draws with.
    #[serde(default = "standard")]
    pub style: String,
    /// How wide its characters are, as a share of their normal width; none
    /// to take the style's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width_factor: Option<f64>,
    /// Degrees anticlockwise from the X axis, in `[0, 360)`.
    #[serde(default)]
    pub rotation: f64,
    /// Where the box sits against its position.
    #[serde(default)]
    pub justify: Attachment,
    #[serde(default = "crate::model::layer_zero")]
    pub layer: String,
}
