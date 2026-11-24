//! Page setup, named plot style references, and transparency for sheets,
//! layers and objects.
//!
//! A drawing uses either colour-dependent plot styles (CTB, AutoCAD's
//! `PSTYLEMODE = 1`) or named plot styles (STB, `PSTYLEMODE = 0`), where
//! layers and objects name a plot style such as `Heavy` or `Normal`.
//! Transparency runs from `0` (opaque) to `90` (90% transparent), as
//! AutoCAD caps it. A page setup (`PAGESETUP`) records the paper, plot
//! area (`extents`, `window` or `layout`), plot scale (`fit` or a ratio),
//! centring, offset, plot style table and whether transparency plots.

use serde::{Deserialize, Serialize};

use super::{is_false, is_true, yes, Paper, BY_BLOCK, BY_LAYER};

/// The highest transparency percentage AutoCAD allows on a layer or object.
pub const MAX_TRANSPARENCY: u8 = 90;

/// The default named plot style in every STB drawing, AutoCAD's `Normal`.
pub const NORMAL_PLOT_STYLE: &str = "Normal";

/// Whether a drawing plots through colour-dependent (CTB) or named (STB)
/// plot styles: AutoCAD's `PSTYLEMODE` (`1` vs `0`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlotStyleMode {
    /// Colour-dependent plot styles (CTB, `PSTYLEMODE = 1`).
    #[default]
    ColorDependent,
    /// Named plot styles (STB, `PSTYLEMODE = 0`).
    Named,
}

impl PlotStyleMode {
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub fn is_color_dependent(&self) -> bool {
        *self == Self::ColorDependent
    }
}

/// An object's own named plot style in an STB drawing: its layer's, its
/// block insert's, or a style named in the STB table (such as `Normal` or
/// `Heavy`).
///
/// It is written as `"ByLayer"`, `"ByBlock"` or the style name in JSON.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum ObjectPlotStyle {
    #[default]
    ByLayer,
    ByBlock,
    Named(String),
}

impl ObjectPlotStyle {
    pub fn is_by_layer(&self) -> bool {
        *self == Self::ByLayer
    }
}

impl From<String> for ObjectPlotStyle {
    fn from(name: String) -> Self {
        match name.as_str() {
            BY_LAYER => Self::ByLayer,
            BY_BLOCK => Self::ByBlock,
            _ => Self::Named(name),
        }
    }
}

impl From<ObjectPlotStyle> for String {
    fn from(style: ObjectPlotStyle) -> Self {
        match style {
            ObjectPlotStyle::ByLayer => BY_LAYER.to_owned(),
            ObjectPlotStyle::ByBlock => BY_BLOCK.to_owned(),
            ObjectPlotStyle::Named(name) => name,
        }
    }
}

/// An object's own transparency: its layer's, its block insert's, or an
/// explicit percentage from `0` (opaque) to `90` (90% transparent).
///
/// It is written as `"ByLayer"`, `"ByBlock"` or an integer `0..=90` in JSON.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "TransparencyJson", into = "TransparencyJson")]
pub enum ObjectTransparency {
    #[default]
    ByLayer,
    ByBlock,
    Percent(u8),
}

impl ObjectTransparency {
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub fn is_by_layer(&self) -> bool {
        *self == Self::ByLayer
    }
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum TransparencyJson {
    Percent(u8),
    Word(String),
}

impl TryFrom<TransparencyJson> for ObjectTransparency {
    type Error = String;

    fn try_from(json: TransparencyJson) -> Result<Self, String> {
        match json {
            TransparencyJson::Percent(p) if p <= MAX_TRANSPARENCY => Ok(Self::Percent(p)),
            TransparencyJson::Percent(p) => Err(format!(
                "transparency {p} is outside 0..={MAX_TRANSPARENCY}"
            )),
            TransparencyJson::Word(word) => match word.as_str() {
                BY_LAYER => Ok(Self::ByLayer),
                BY_BLOCK => Ok(Self::ByBlock),
                _ => Err(format!(
                    "`{word}` is not a transparency; ByLayer, ByBlock or a whole number from 0 to \
                     {MAX_TRANSPARENCY}"
                )),
            },
        }
    }
}

impl From<ObjectTransparency> for TransparencyJson {
    fn from(transparency: ObjectTransparency) -> Self {
        match transparency {
            ObjectTransparency::ByLayer => Self::Word(BY_LAYER.to_owned()),
            ObjectTransparency::ByBlock => Self::Word(BY_BLOCK.to_owned()),
            ObjectTransparency::Percent(p) => Self::Percent(p),
        }
    }
}

/// What area of the drawing a page setup plots, as AutoCAD's `PAGESETUP`
/// names it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "area", rename_all = "snake_case")]
pub enum PlotArea {
    /// The box round everything the sheet draws.
    Extents,
    /// A paper-space layout's paper from `(0, 0)` to `(width, height)`.
    Layout,
    /// A rectangular window in drawing units: lower-left `min` to
    /// upper-right `max`.
    Window { min: [f64; 2], max: [f64; 2] },
}

impl PlotArea {
    /// The area's name as `PAGESETUP` and `query layout` write it.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Extents => "extents",
            Self::Layout => "layout",
            Self::Window { .. } => "window",
        }
    }
}

/// The plot scale a page setup plots at: fitted to the printable paper, or
/// a fixed ratio `numerator:denominator` of paper length to drawing length.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlotScale {
    /// Scale to fit the printable paper area.
    Fit,
    /// Fixed ratio: `numerator` paper length per `denominator` drawing
    /// length (dimensionless, e.g. `1:2` has factor `0.5`).
    Ratio { numerator: f64, denominator: f64 },
}

impl Default for PlotScale {
    fn default() -> Self {
        Self::one_to_one()
    }
}

impl PlotScale {
    /// 1:1 full size.
    pub const fn one_to_one() -> Self {
        Self::Ratio {
            numerator: 1.0,
            denominator: 1.0,
        }
    }

    #[allow(clippy::float_cmp)]
    pub fn is_one_to_one(&self) -> bool {
        matches!(
            self,
            Self::Ratio {
                numerator,
                denominator
            } if *numerator == 1.0 && *denominator == 1.0
        )
    }

    /// Paper length per drawing length when the scale is fixed; `None` for
    /// [`PlotScale::Fit`].
    pub fn factor(&self) -> Option<f64> {
        match *self {
            Self::Fit => None,
            Self::Ratio {
                numerator,
                denominator,
            } => Some(numerator / denominator),
        }
    }

    /// Formatted as `"fit"` or `"numerator:denominator"`.
    pub fn display_text(&self) -> String {
        match *self {
            Self::Fit => "fit".to_owned(),
            Self::Ratio {
                numerator,
                denominator,
            } => format!("{numerator}:{denominator}"),
        }
    }
}

/// A sheet's or layout's page setup (`PAGESETUP`): paper, plot area, scale,
/// centring, offset, margins, plot style table and transparency plotting.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageSetup {
    /// The paper to plot onto; falls back to the sheet's or layout's
    /// `paper` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper: Option<Paper>,
    /// What area to plot (`extents`, `layout` or `window`). When omitted,
    /// defaults to `layout` on a layout sheet and `extents` in model space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<PlotArea>,
    /// The plot scale (`fit` or a ratio `paper:drawing`, default `1:1`).
    #[serde(default, skip_serializing_if = "PlotScale::is_one_to_one")]
    pub scale: PlotScale,
    /// Whether the plotted area is centred on the printable paper.
    #[serde(default, skip_serializing_if = "is_false")]
    pub center: bool,
    /// Plot origin offset `[x, y]` in drawing units when `center` is false.
    #[serde(default, skip_serializing_if = "is_zero_offset")]
    pub offset: [f64; 2],
    /// Unprintable printer margins `[left, bottom, right, top]` in
    /// millimetres, read from a DWG's `Layout` when present.
    #[serde(default, skip_serializing_if = "is_zero_margins")]
    pub margins_mm: [f64; 4],
    /// The CTB or STB plot style file name, such as `monochrome.ctb` or
    /// `plant.stb`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plot_style: Option<String>,
    /// Whether `plot_style` came from a DWG's stored `Layout.plot_style_sheet`
    /// rather than an explicit YAML sheet description. When true and the
    /// named file is not present in the configured plot style folders, a
    /// plot falls back to built-in monochrome instead of failing.
    #[serde(skip)]
    pub plot_style_from_dwg: bool,
    /// Whether layer and object transparency is plotted (default `true`).
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub plot_transparency: bool,
    /// Plotter / PC3 name when read from a DWG, such as `DWG To PDF.pc3`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plotter: Option<String>,
}

impl Default for PageSetup {
    fn default() -> Self {
        Self {
            paper: None,
            area: None,
            scale: PlotScale::one_to_one(),
            center: false,
            offset: [0.0, 0.0],
            margins_mm: [0.0, 0.0, 0.0, 0.0],
            plot_style: None,
            plot_style_from_dwg: false,
            plot_transparency: true,
            plotter: None,
        }
    }
}

impl PageSetup {
    /// Whether this page setup uses an STB named plot style table.
    pub fn uses_stb(&self) -> bool {
        self.plot_style
            .as_deref()
            .is_some_and(|name| name.to_ascii_lowercase().ends_with(".stb"))
    }
}

#[allow(clippy::float_cmp)]
pub(crate) fn is_zero_offset(v: &[f64; 2]) -> bool {
    v[0] == 0.0 && v[1] == 0.0
}

#[allow(clippy::float_cmp)]
pub(crate) fn is_zero_margins(v: &[f64; 4]) -> bool {
    v[0] == 0.0 && v[1] == 0.0 && v[2] == 0.0 && v[3] == 0.0
}

#[allow(clippy::trivially_copy_pass_by_ref)]
pub(crate) fn is_zero_u8(n: &u8) -> bool {
    *n == 0
}
