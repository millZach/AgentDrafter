//! Dimensions: AutoCAD's DIMENSION with its DIMSTYLE.
//!
//! A dimension measures a length and shows it: linear, along a line at an
//! angle, horizontal unless it gives another; aligned, along the line
//! between its two points; radius and diameter, of a circle or an arc by its
//! centre and a point on it. It is drawn as AutoCAD 2026 draws its own,
//! measured against AutoCAD's own dimensions of made-up geometry in its
//! `acad` and `acadiso` templates:
//!
//! - A linear or aligned dimension draws an extension line from each of its
//!   points, starting the style's offset from the point and reaching its
//!   extension beyond the dimension line; the dimension line between them,
//!   which passes through its location; an arrowhead at each end; and its
//!   text. Text and arrowheads sit between the extension lines when they
//!   fit, AutoCAD's best fit: the text needs its own length along the line
//!   and a gap at each end, the arrowheads their two lengths and a gap each.
//!   When both fit, both are inside; when each fits alone but not both, the
//!   one that needs more room is inside; when only the text fits, the arrowheads
//!   go outside, each with a tail as long as itself; when only the
//!   arrowheads fit, the text goes beyond the second extension line, an
//!   arrowhead's length and a gap past it; when neither fits, both go
//!   outside, the text past the second arrowhead's tail.
//! - A radius or diameter dimension draws a leader from its text to its
//!   arrowhead, on the line from its centre through its location, and the
//!   arrow sits where that line meets the circle. Text placed nearer the
//!   circle than AutoCAD allows is moved out, as AutoCAD moves it: outside,
//!   to two arrowheads' lengths past the circle; inside, to two short of it.
//!
//! Text sits as its style's placement says, one of AutoCAD's two defaults:
//!
//! - `above`, as ISO-25 has it: aligned with the dimension line, read from
//!   the bottom or the right, a gap above it; the dimension line runs
//!   between the extension lines even when the arrowheads are outside them,
//!   and a radius dimension's line runs on to the centre.
//! - `centered`, as Standard has it: horizontal, in the middle of the
//!   dimension line, which stops a gap short of its box. Text outside a
//!   circle or the extension lines ends in a horizontal landing, an
//!   arrowhead long, when its line runs 15° or more from the horizontal, and
//!   a circle a radius or diameter dimension's text is outside of is marked
//!   at its centre by a cross, the centre mark.
//!
//! - An angular dimension draws an arc round its vertex, through its
//!   location, between the two lines or the two points it measures, with an
//!   arrowhead at each end; an extension line from each line or point the
//!   arc does not reach; and its text, fitted as a linear dimension's is,
//!   by the angles the text and arrowheads need on the arc (`angular.rs`).
//! - An ordinate dimension draws a leader from its feature to its
//!   location, kinked when the two are not in line, and its text at the
//!   leader's end (`ordinate.rs`).
//!
//! The text says the measured value, as the style formats it, or the
//! dimension's own text, where `<>` stands for the measured value, as in
//! AutoCAD. A radius's value starts with `R` and a diameter's with `∅` unless
//! the style gives a prefix or a suffix, which an angle takes too. A
//! length shows in the style's unit format and an angle in its angular
//! unit (`units.rs`); a stacked fraction is laid out as AutoCAD lays out its
//! MTEXT (`stack.rs`).

use serde::{Deserialize, Serialize};

use super::leader::{Arrowhead, MTextBox};
use super::{layer_zero, Point3, Properties};
use crate::description::normalize_degrees;
use crate::text::{Attachment, MText, STANDARD};
use crate::units::Unit;

mod angular;
mod ordinate;
mod stack;
mod units;

pub use angular::Sector;
pub use units::{AngularUnit, FractionFormat, UnitFormat, DEGREE_SIGN};

/// The dimension style an imperial sheet's dimensions draw with unless they
/// name another: AutoCAD's own `Standard`.
pub const STANDARD_DIMENSION_STYLE: &str = STANDARD;

/// The dimension style a metric sheet's dimensions draw with unless they
/// name another: AutoCAD's own `ISO-25`.
pub const ISO_DIMENSION_STYLE: &str = "ISO-25";

/// How far from the horizontal, in degrees, the line under horizontal text
/// runs before AutoCAD ends it in a landing.
pub const LANDING_ANGLE: f64 = 15.0;

/// What a value AutoCAD sets in its `\U+2205` code: the diameter sign.
pub const DIAMETER_SIGN: char = '\u{2205}';

/// The text a dimension's own text writes for its measured value: `<>`.
pub const MEASURED: &str = "<>";

/// How far a fit may miss and still fit: rounding in sums of lengths.
const FIT_TOLERANCE: f64 = 1e-9;

/// What a dimension measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DimensionKind {
    /// AutoCAD's DIMLINEAR: the distance between two points along a line at
    /// an angle.
    Linear,
    /// AutoCAD's DIMALIGNED: the distance between two points.
    Aligned,
    /// AutoCAD's DIMRADIUS: a circle's or an arc's radius.
    Radius,
    /// AutoCAD's DIMDIAMETER: a circle's or an arc's diameter.
    Diameter,
    /// AutoCAD's DIMANGULAR of two lines: the angle between them.
    Angular,
    /// AutoCAD's DIMANGULAR of a vertex and two points: the angle at the
    /// vertex between them, as AutoCAD measures an arc's or a circle's.
    ThreePointAngular,
    /// AutoCAD's DIMORDINATE: a feature's X or Y from the origin.
    Ordinate,
}

impl DimensionKind {
    /// Every kind, in the order messages list them.
    pub const ALL: [DimensionKind; 7] = [
        DimensionKind::Linear,
        DimensionKind::Aligned,
        DimensionKind::Radius,
        DimensionKind::Diameter,
        DimensionKind::Angular,
        DimensionKind::ThreePointAngular,
        DimensionKind::Ordinate,
    ];

    /// The name a sheet description writes, such as `linear`.
    pub const fn name(self) -> &'static str {
        match self {
            DimensionKind::Linear => "linear",
            DimensionKind::Aligned => "aligned",
            DimensionKind::Radius => "radius",
            DimensionKind::Diameter => "diameter",
            DimensionKind::Angular => "angular",
            DimensionKind::ThreePointAngular => "three_point_angular",
            DimensionKind::Ordinate => "ordinate",
        }
    }

    /// Whether it measures a circle, from its centre and a point on it.
    pub const fn is_radial(self) -> bool {
        matches!(self, DimensionKind::Radius | DimensionKind::Diameter)
    }

    /// Whether it measures an angle.
    pub const fn is_angular(self) -> bool {
        matches!(
            self,
            DimensionKind::Angular | DimensionKind::ThreePointAngular
        )
    }
}

/// Which of a feature's coordinates an ordinate dimension measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrdinateAxis {
    /// Its X, AutoCAD's X datum: the leader runs up or down.
    X,
    /// Its Y, AutoCAD's Y datum: the leader runs across.
    Y,
}

impl OrdinateAxis {
    pub const ALL: [OrdinateAxis; 2] = [OrdinateAxis::X, OrdinateAxis::Y];

    pub const fn name(self) -> &'static str {
        match self {
            OrdinateAxis::X => "x",
            OrdinateAxis::Y => "y",
        }
    }
}

/// Where a dimension's text sits: AutoCAD's two default placements.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextPlacement {
    /// Horizontal, in the middle of the dimension line, which breaks for
    /// it: Standard's (DIMTAD 0, DIMTIH and DIMTOH on, DIMTOFL off).
    #[default]
    Centered,
    /// Aligned with the dimension line, above it: ISO-25's (DIMTAD 1,
    /// DIMTIH and DIMTOH off, DIMTOFL on).
    Above,
}

impl TextPlacement {
    pub const ALL: [TextPlacement; 2] = [TextPlacement::Centered, TextPlacement::Above];

    pub const fn name(self) -> &'static str {
        match self {
            TextPlacement::Centered => "centered",
            TextPlacement::Above => "above",
        }
    }
}

/// The character between a value's whole part and its decimals:
/// AutoCAD's DIMDSEP.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecimalSeparator {
    #[default]
    Period,
    Comma,
    Space,
}

impl DecimalSeparator {
    pub const ALL: [DecimalSeparator; 3] = [
        DecimalSeparator::Period,
        DecimalSeparator::Comma,
        DecimalSeparator::Space,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            DecimalSeparator::Period => "period",
            DecimalSeparator::Comma => "comma",
            DecimalSeparator::Space => "space",
        }
    }

    pub const fn character(self) -> char {
        match self {
            DecimalSeparator::Period => '.',
            DecimalSeparator::Comma => ',',
            DecimalSeparator::Space => ' ',
        }
    }

    /// The separator AutoCAD codes as `code` in DIMDSEP.
    pub fn of_code(code: i16) -> Option<DecimalSeparator> {
        DecimalSeparator::ALL
            .into_iter()
            .find(|s| i16::try_from(u32::from(s.character())).ok() == Some(code))
    }
}

/// What a dimension draws with: AutoCAD's DIMSTYLE, the values the
/// pipeline draws. Every length is in the sheet's unit before the overall
/// scale, which multiplies them all as AutoCAD's DIMSCALE does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DimensionStyle {
    pub name: String,
    pub arrowhead: Arrowhead,
    /// How long an arrowhead is, more than 0: DIMASZ.
    pub arrow_size: f64,
    /// The text style its text draws with: DIMTXSTY.
    pub text_style: String,
    /// The height of its text's capitals, more than 0: DIMTXT.
    pub text_height: f64,
    /// The space round its text, 0 or more: DIMGAP.
    pub gap: f64,
    /// How far an extension line starts from its point, 0 or more: DIMEXO.
    pub extension_offset: f64,
    /// How far an extension line reaches past the dimension line, 0 or
    /// more: DIMEXE.
    pub extension_beyond: f64,
    /// How far the dimension line reaches past the extension lines when it
    /// has no arrowheads, 0 or more: DIMDLE.
    pub line_extension: f64,
    /// How far a centre mark's arms reach from the centre, 0 or more, 0 for
    /// none: DIMCEN.
    pub center_mark: f64,
    pub placement: TextPlacement,
    /// How lengths show: DIMLUNIT.
    pub unit_format: UnitFormat,
    /// How many decimals a length shows, 0 to 8, or for an architectural or
    /// fractional one how fine its fraction is, to 1/2^precision: DIMDEC.
    pub precision: u8,
    /// How an architectural or fractional length's fraction shows: DIMFRAC.
    pub fraction_format: FractionFormat,
    pub decimal_separator: DecimalSeparator,
    /// Whether `0.5` shows as `.5`: DIMZIN 4.
    pub suppress_leading_zeros: bool,
    /// Whether `2.50` shows as `2.5`, and `2.00` as `2`: DIMZIN 8.
    pub suppress_trailing_zeros: bool,
    /// Whether an engineering or architectural length leaves out zero
    /// feet, `0'-6"` showing as `6"`: DIMZIN 0 and 3.
    pub suppress_zero_feet: bool,
    /// Whether it leaves out zero inches, `1'-0"` showing as `1'`: DIMZIN
    /// 0 and 2.
    pub suppress_zero_inches: bool,
    /// How angles show: DIMAUNIT.
    pub angular_unit: AngularUnit,
    /// How many decimals an angle shows, 0 to 8: DIMADEC. AutoCAD takes -1,
    /// as many as a length, as a variable, and its audit refuses it in a
    /// style.
    pub angular_precision: u8,
    /// Whether an angle's `0.5` shows as `.5`: DIMAZIN 1.
    pub suppress_angular_leading_zeros: bool,
    /// Whether an angle's `2.50` shows as `2.5`: DIMAZIN 2.
    pub suppress_angular_trailing_zeros: bool,
    /// What comes before every value it shows: DIMPOST's part before `<>`.
    pub prefix: String,
    /// What comes after: DIMPOST's part after `<>`.
    pub suffix: String,
    /// What every length of the style is multiplied by, more than 0:
    /// DIMSCALE.
    pub scale: f64,
    /// How far apart a baseline dimension's lines are, 0 or more: DIMDLI.
    pub baseline_spacing: f64,
}

impl DimensionStyle {
    /// AutoCAD 2026's own `Standard`, of its `acad` template, in `units`:
    /// arrows and text 0.18 in, a gap of 0.09, extension lines offset
    /// 0.0625 and reaching 0.18 beyond, a centre mark 0.09, baseline
    /// dimensions 0.38 apart, text centred, four decimals after a period
    /// and no zeros suppressed, angles in whole degrees.
    pub fn standard(units: Unit) -> Self {
        let inch = |x: f64| Unit::In.convert(x, units);
        Self {
            name: STANDARD_DIMENSION_STYLE.to_owned(),
            arrowhead: Arrowhead::ClosedFilled,
            arrow_size: inch(0.18),
            text_style: STANDARD.to_owned(),
            text_height: inch(0.18),
            gap: inch(0.09),
            extension_offset: inch(0.0625),
            extension_beyond: inch(0.18),
            line_extension: 0.0,
            center_mark: inch(0.09),
            placement: TextPlacement::Centered,
            unit_format: UnitFormat::Decimal,
            precision: 4,
            fraction_format: FractionFormat::Horizontal,
            decimal_separator: DecimalSeparator::Period,
            suppress_leading_zeros: false,
            suppress_trailing_zeros: false,
            suppress_zero_feet: true,
            suppress_zero_inches: true,
            angular_unit: AngularUnit::DecimalDegrees,
            angular_precision: 0,
            suppress_angular_leading_zeros: false,
            suppress_angular_trailing_zeros: false,
            prefix: String::new(),
            suffix: String::new(),
            scale: 1.0,
            baseline_spacing: inch(0.38),
        }
    }

    /// AutoCAD 2026's own `ISO-25`, of its `acadiso` template, in `units`:
    /// arrows and text 2.5 mm, a gap of 0.625, extension lines offset 0.625
    /// and reaching 1.25 beyond, a centre mark 2.5, baseline dimensions
    /// 3.75 apart, text above the line, two decimals after a comma and
    /// trailing zeros suppressed, angles in whole degrees.
    pub fn iso_25(units: Unit) -> Self {
        let mm = |x: f64| Unit::Mm.convert(x, units);
        Self {
            name: ISO_DIMENSION_STYLE.to_owned(),
            arrowhead: Arrowhead::ClosedFilled,
            arrow_size: mm(2.5),
            text_style: STANDARD.to_owned(),
            text_height: mm(2.5),
            gap: mm(0.625),
            extension_offset: mm(0.625),
            extension_beyond: mm(1.25),
            line_extension: 0.0,
            center_mark: mm(2.5),
            placement: TextPlacement::Above,
            unit_format: UnitFormat::Decimal,
            precision: 2,
            fraction_format: FractionFormat::Horizontal,
            decimal_separator: DecimalSeparator::Comma,
            suppress_leading_zeros: false,
            suppress_trailing_zeros: true,
            suppress_zero_feet: true,
            suppress_zero_inches: true,
            angular_unit: AngularUnit::DecimalDegrees,
            angular_precision: 0,
            suppress_angular_leading_zeros: false,
            suppress_angular_trailing_zeros: false,
            prefix: String::new(),
            suffix: String::new(),
            scale: 1.0,
            baseline_spacing: mm(3.75),
        }
    }

    /// The name of the style a sheet in `units` draws its dimensions with
    /// unless they name another: `ISO-25` on a metric sheet, `Standard` on
    /// an imperial one.
    pub fn default_name(units: Unit) -> &'static str {
        if units.is_metric() {
            ISO_DIMENSION_STYLE
        } else {
            STANDARD_DIMENSION_STYLE
        }
    }

    /// The style named `name` a sheet in `units` has without listing it:
    /// its unit's default, AutoCAD's own.
    pub fn built_in(name: &str, units: Unit) -> Option<Self> {
        (name == Self::default_name(units)).then(|| {
            if units.is_metric() {
                Self::iso_25(units)
            } else {
                Self::standard(units)
            }
        })
    }

    /// The style's lengths multiplied by its scale, as a dimension draws
    /// them.
    fn sizes(&self) -> Sizes {
        let k = self.scale;
        Sizes {
            arrow: self.arrow_size * k,
            text: self.text_height * k,
            gap: self.gap * k,
            offset: self.extension_offset * k,
            beyond: self.extension_beyond * k,
            line_extension: self.line_extension * k,
            center_mark: self.center_mark * k,
        }
    }

    /// The length `value` as the style shows it, in its unit format, each
    /// stacked fraction written `N/D`.
    pub fn format(&self, value: f64) -> String {
        flat(&self.length_pieces(value))
    }

    /// Whether DIMPOST holds anything: a prefix or a suffix.
    fn has_post(&self) -> bool {
        !self.prefix.is_empty() || !self.suffix.is_empty()
    }
}

/// A style's lengths, scaled.
#[derive(Clone, Copy, Debug)]
struct Sizes {
    arrow: f64,
    text: f64,
    gap: f64,
    offset: f64,
    beyond: f64,
    line_extension: f64,
    center_mark: f64,
}

/// A piece of a dimension's text: plain text, or a fraction AutoCAD
/// stacks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextPiece {
    Plain(String),
    /// A fraction over a bar, or across a slash when `diagonal`.
    Fraction {
        numerator: String,
        denominator: String,
        diagonal: bool,
    },
}

/// Pieces of text as one line: each stacked fraction written `N/D`, a
/// space between it and a number before it.
pub fn flat(pieces: &[TextPiece]) -> String {
    let mut out = String::new();
    for piece in pieces {
        match piece {
            TextPiece::Plain(s) => out.push_str(s),
            TextPiece::Fraction {
                numerator,
                denominator,
                ..
            } => {
                if out.ends_with(|c: char| c.is_ascii_digit()) {
                    out.push(' ');
                }
                out.push_str(numerator);
                out.push('/');
                out.push_str(denominator);
            }
        }
    }
    out
}

/// What a dimension measures, and from what.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Measured {
    /// The distance between its two points along a line at `angle`
    /// degrees, `[0, 360)`.
    Linear { points: [Point3; 2], angle: f64 },
    /// The distance between its two points.
    Aligned { points: [Point3; 2] },
    /// The radius of the circle round `center` through `point`.
    Radius { center: Point3, point: Point3 },
    /// The diameter of the circle round `center` through `point`.
    Diameter { center: Point3, point: Point3 },
    /// The angle between two lines, each through its two points, in the
    /// sector between them that the location is in.
    Angular { lines: [[Point3; 2]; 2] },
    /// The angle at `center` from one point to the other: of the two, the
    /// one the location is in.
    ThreePointAngular { center: Point3, points: [Point3; 2] },
    /// The X or Y of `feature`, from the origin.
    Ordinate { feature: Point3, axis: OrdinateAxis },
}

/// A dimension: AutoCAD's DIMENSION.
///
/// A linear or aligned dimension's location is where its dimension line
/// meets its second extension line, as AutoCAD keeps it; a radius or
/// diameter dimension's is where its text sits along the line from its
/// centre, and its point is where that line meets the circle, its arrow's
/// point; an angular dimension's is on its arc a third of the way round
/// from the arc's first end, as AutoCAD keeps it; an ordinate dimension's
/// is the end of its leader. The description's reader puts each there from
/// what it is given.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dimension {
    #[serde(flatten)]
    pub measured: Measured,
    pub location: Point3,
    pub dimension_style: String,
    /// The dimension's own text, `<>` standing for the measured value;
    /// none to show the measured value alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Whether it continues another dimension, as AutoCAD's DIMCONTINUE
    /// adds it: AutoCAD marks it by overriding DIMTMOVE to 1, which moves
    /// nothing while its text is in its own place.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub continued: bool,
    #[serde(default = "layer_zero")]
    pub layer: String,
    #[serde(flatten)]
    pub properties: Properties,
}

/// One thing a dimension draws, in the order AutoCAD writes it in the
/// dimension's block.
#[derive(Clone, Debug, PartialEq)]
pub enum DimensionPart {
    /// A line, from one end to the other as AutoCAD writes it.
    Line([Point3; 2]),
    /// An arc round `center`, anticlockwise from `start` to `end` degrees.
    Arc {
        center: Point3,
        radius: f64,
        start: f64,
        end: f64,
    },
    /// An arrowhead with its point at `tip`, its body towards `base`, an
    /// arrowhead's length away.
    Arrow { tip: Point3, base: Point3 },
    /// The text.
    Text,
}

/// A line of a stacked text, or one of its bars or slashes, as it draws.
#[derive(Clone, Debug, PartialEq)]
pub enum Lettered {
    /// A line of text starting at `start`, on its baseline, along the
    /// text's rotation.
    Text {
        contents: String,
        start: Point3,
    },
    Line([Point3; 2]),
}

/// A dimension laid out as AutoCAD lays out its block.
#[derive(Clone, Debug, PartialEq)]
pub struct LaidDimension {
    pub parts: Vec<DimensionPart>,
    /// The text as it draws: multiline text attached at its middle, a
    /// stacked fraction written `N/D` in it.
    pub text: MText,
    /// What the text says, piece by piece, as AutoCAD writes its MTEXT.
    pub pieces: Vec<TextPiece>,
    /// The text, when it holds a stacked fraction, as it draws: each line of
    /// text, bar and slash. Empty when the text draws as `text` does.
    pub lettering: Vec<Lettered>,
    /// Where AutoCAD keeps the text's place on the dimension: the middle
    /// of the text for a linear, aligned, angular or ordinate dimension,
    /// and for a radius or diameter dimension the point along its line the
    /// text goes from.
    pub text_point: Point3,
    /// Whether AutoCAD keeps the text as placed where the dimension says,
    /// as it keeps a radius's or diameter's text, rather than at its own
    /// place: not when it moved the text out from too near the circle.
    pub text_placed: bool,
    pub arrowhead: Arrowhead,
    pub arrow_size: f64,
    /// Whether its arrowheads sit on an arc, as an angular dimension's do:
    /// AutoCAD writes each one's corners from the right of its body first.
    pub arrows_on_arc: bool,
    /// Where AutoCAD keeps an angular dimension's arc point when it is not
    /// its location, a third of the way round its arc: two thirds of the
    /// way, when its text is outside.
    pub arc_point: Option<Point3>,
    /// The points AutoCAD marks in the block on layer `Defpoints`.
    pub defpoints: Vec<Point3>,
}

impl LaidDimension {
    /// The lines it draws, without its arcs, arrowheads and text.
    pub fn lines(&self) -> impl Iterator<Item = [Point3; 2]> + '_ {
        self.parts.iter().filter_map(|p| match p {
            DimensionPart::Line(l) => Some(*l),
            _ => None,
        })
    }

    /// Its arcs: each one's centre, radius, and start and end in degrees.
    pub fn arcs(&self) -> impl Iterator<Item = (Point3, f64, f64, f64)> + '_ {
        self.parts.iter().filter_map(|p| match p {
            DimensionPart::Arc {
                center,
                radius,
                start,
                end,
            } => Some((*center, *radius, *start, *end)),
            _ => None,
        })
    }

    /// Its arrowheads: each one's tip and base.
    pub fn arrows(&self) -> impl Iterator<Item = (Point3, Point3)> + '_ {
        self.parts.iter().filter_map(|p| match p {
            DimensionPart::Arrow { tip, base } => Some((*tip, *base)),
            _ => None,
        })
    }
}

impl Dimension {
    pub fn kind(&self) -> DimensionKind {
        match self.measured {
            Measured::Linear { .. } => DimensionKind::Linear,
            Measured::Aligned { .. } => DimensionKind::Aligned,
            Measured::Radius { .. } => DimensionKind::Radius,
            Measured::Diameter { .. } => DimensionKind::Diameter,
            Measured::Angular { .. } => DimensionKind::Angular,
            Measured::ThreePointAngular { .. } => DimensionKind::ThreePointAngular,
            Measured::Ordinate { .. } => DimensionKind::Ordinate,
        }
    }

    /// Every point that defines it, its location last: its two points; its
    /// centre and its point; its two lines' points; or its feature.
    pub fn points(&self) -> Vec<Point3> {
        let mut points = match &self.measured {
            Measured::Linear { points, .. } | Measured::Aligned { points } => points.to_vec(),
            Measured::Radius { center, point } | Measured::Diameter { center, point } => {
                vec![*center, *point]
            }
            Measured::Angular { lines } => lines.iter().flatten().copied().collect(),
            Measured::ThreePointAngular { center, points } => {
                vec![*center, points[0], points[1]]
            }
            Measured::Ordinate { feature, .. } => vec![*feature],
        };
        points.push(self.location);
        points
    }

    /// Every point that defines it, mutably, its location last.
    pub fn points_mut(&mut self) -> Vec<&mut Point3> {
        let mut points: Vec<&mut Point3> = match &mut self.measured {
            Measured::Linear { points, .. } | Measured::Aligned { points } => {
                points.iter_mut().collect()
            }
            Measured::Radius { center, point } | Measured::Diameter { center, point } => {
                vec![center, point]
            }
            Measured::Angular { lines } => lines.iter_mut().flatten().collect(),
            Measured::ThreePointAngular { center, points } => {
                let [a, b] = points;
                vec![center, a, b]
            }
            Measured::Ordinate { feature, .. } => vec![feature],
        };
        points.push(&mut self.location);
        points
    }

    /// The sector an angular dimension measures, from its lines or points
    /// and its location; none for another kind, or lines that never meet.
    pub fn sector(&self) -> Option<Sector> {
        angular::sector(&self.measured, self.location)
    }

    /// What it measures, as AutoCAD measures it: a length in the sheet's
    /// unit, in the plane, along its line, the radius or the diameter, or
    /// the X or Y from the origin; or an angle, in degrees.
    pub fn measurement(&self) -> f64 {
        match &self.measured {
            Measured::Linear { points, angle } => {
                let (s, c) = angle.to_radians().sin_cos();
                ((points[1].x - points[0].x) * c + (points[1].y - points[0].y) * s).abs()
            }
            Measured::Aligned { points } => {
                (points[1].x - points[0].x).hypot(points[1].y - points[0].y)
            }
            Measured::Radius { center, point } => (point.x - center.x).hypot(point.y - center.y),
            Measured::Diameter { center, point } => {
                2.0 * (point.x - center.x).hypot(point.y - center.y)
            }
            Measured::Angular { .. } | Measured::ThreePointAngular { .. } => {
                self.sector().map_or(0.0, |s| s.sweep)
            }
            Measured::Ordinate { feature, axis } => match axis {
                OrdinateAxis::X => feature.x.abs(),
                OrdinateAxis::Y => feature.y.abs(),
            },
        }
    }

    /// The measured value as `style` shows it, piece by piece: a length in
    /// its unit format, an angle in its angular unit, with the style's
    /// prefix and suffix, or without them a radius's `R` or a diameter's
    /// `∅` before it.
    pub fn measured_pieces(&self, style: &DimensionStyle) -> Vec<TextPiece> {
        let mut pieces = if self.kind().is_angular() {
            vec![TextPiece::Plain(style.angle_text(self.measurement()))]
        } else {
            style.length_pieces(self.measurement())
        };
        let (before, after) = if style.has_post() {
            (style.prefix.clone(), style.suffix.clone())
        } else {
            match self.kind() {
                DimensionKind::Radius => ("R".to_owned(), String::new()),
                DimensionKind::Diameter => (DIAMETER_SIGN.to_string(), String::new()),
                _ => (String::new(), String::new()),
            }
        };
        if !before.is_empty() {
            pieces.insert(0, TextPiece::Plain(before));
        }
        if !after.is_empty() {
            pieces.push(TextPiece::Plain(after));
        }
        joined(pieces)
    }

    /// The measured value as `style` shows it, a stacked fraction written
    /// `N/D`.
    pub fn measured_text(&self, style: &DimensionStyle) -> String {
        flat(&self.measured_pieces(style))
    }

    /// What its text says, piece by piece: its own, each `<>` the measured
    /// value, or the measured value.
    pub fn text_pieces(&self, style: &DimensionStyle) -> Vec<TextPiece> {
        let measured = self.measured_pieces(style);
        let Some(own) = &self.text else {
            return measured;
        };
        let mut pieces = Vec::new();
        for (i, part) in own.split(MEASURED).enumerate() {
            if i > 0 {
                pieces.extend(measured.iter().cloned());
            }
            if !part.is_empty() {
                pieces.push(TextPiece::Plain(part.to_owned()));
            }
        }
        joined(pieces)
    }

    /// What its text says, a stacked fraction written `N/D`.
    pub fn displayed_text(&self, style: &DimensionStyle) -> String {
        flat(&self.text_pieces(style))
    }

    /// Lays the dimension out with `style` on a sheet in `units`, its
    /// text's box measured by `measure`, as AutoCAD 2026 lays out its
    /// block. A text with a stacked fraction is aligned at its middle on an
    /// imperial sheet, as AutoCAD aligns Standard's.
    pub fn laid_out(
        &self,
        style: &DimensionStyle,
        units: Unit,
        measure: &dyn Fn(&MText) -> MTextBox,
    ) -> LaidDimension {
        let sizes = style.sizes();
        let pieces = self.text_pieces(style);
        let mut text = MText {
            contents: flat(&pieces),
            position: self.location,
            height: sizes.text,
            width: 0.0,
            style: style.text_style.clone(),
            width_factor: None,
            rotation: 0.0,
            justify: Attachment::MiddleCenter,
            layer: self.layer.clone(),
            field: None,
            properties: self.properties.clone(),
        };
        let stacked = pieces
            .iter()
            .any(|p| matches!(p, TextPiece::Fraction { .. }))
            .then(|| {
                let piece = |contents: &str| {
                    measure(&MText {
                        contents: contents.to_owned(),
                        ..text.clone()
                    })
                };
                stack::stacked(&pieces, sizes.text, !units.is_metric(), &piece)
            });
        let box_size = match &stacked {
            Some(s) => (s.width, s.height),
            None => {
                let measured = measure(&text);
                (measured.width, measured.height)
            }
        };
        let mut laid = match &self.measured {
            Measured::Linear { points, angle } => {
                let (s, c) = angle.to_radians().sin_cos();
                along(points, [c, s], self.location, style, sizes, box_size)
            }
            Measured::Aligned { points } => {
                let d = [points[1].x - points[0].x, points[1].y - points[0].y];
                let length = d[0].hypot(d[1]);
                along(
                    points,
                    [d[0] / length, d[1] / length],
                    self.location,
                    style,
                    sizes,
                    box_size,
                )
            }
            Measured::Radius { center, point } | Measured::Diameter { center, point } => radial(
                *center,
                (point.x - center.x).hypot(point.y - center.y),
                self.location,
                self.kind() == DimensionKind::Diameter,
                style,
                sizes,
                box_size,
            ),
            Measured::Angular { lines } => match self.sector() {
                Some(sector) => {
                    let mut laid = angular::angular(&sector, style, sizes, box_size);
                    let arc = laid.arc_point.unwrap_or(self.location);
                    laid.defpoints = vec![arc, lines[0][0], lines[1][0], lines[1][1], lines[0][1]];
                    laid
                }
                None => Layout::empty(self.location),
            },
            Measured::ThreePointAngular { center, points } => match self.sector() {
                Some(sector) => {
                    let mut laid = angular::angular(&sector, style, sizes, box_size);
                    let arc = laid.arc_point.unwrap_or(self.location);
                    // The point on the first ray first, as AutoCAD marks them.
                    let first = sector.first_point(points);
                    let second = if first == points[0] {
                        points[1]
                    } else {
                        points[0]
                    };
                    laid.defpoints = vec![*center, first, second, arc];
                    laid
                }
                None => Layout::empty(self.location),
            },
            Measured::Ordinate { feature, axis } => {
                ordinate::ordinate(*feature, *axis, self.location, style, sizes, box_size)
            }
        };
        text.position = laid.center;
        text.rotation = normalize_degrees(laid.rotation);
        let lettering = match stacked {
            Some(stacked) => {
                let along = direction(laid.rotation);
                let up = left(along);
                let at = |[u, v]: [f64; 2]| add(add(laid.center, along, u), up, v);
                let mut lettering: Vec<Lettered> = stacked
                    .texts
                    .into_iter()
                    .map(|(contents, start)| Lettered::Text {
                        contents,
                        start: at(start),
                    })
                    .collect();
                lettering.extend(
                    stacked
                        .lines
                        .into_iter()
                        .map(|[a, b]| Lettered::Line([at(a), at(b)])),
                );
                lettering
            }
            None => Vec::new(),
        };
        LaidDimension {
            parts: std::mem::take(&mut laid.parts),
            text,
            pieces,
            lettering,
            text_point: laid.text_point,
            text_placed: laid.placed,
            arrowhead: style.arrowhead,
            arrow_size: sizes.arrow,
            arrows_on_arc: self.kind().is_angular(),
            arc_point: laid.arc_point,
            defpoints: laid.defpoints,
        }
    }
}

/// Pieces with each run of plain text joined into one.
pub(super) fn joined(pieces: Vec<TextPiece>) -> Vec<TextPiece> {
    let mut out: Vec<TextPiece> = Vec::with_capacity(pieces.len());
    for piece in pieces {
        match (out.last_mut(), piece) {
            (Some(TextPiece::Plain(before)), TextPiece::Plain(after)) => before.push_str(&after),
            (_, piece) => out.push(piece),
        }
    }
    out
}

/// A layout before its text is put together.
struct Layout {
    parts: Vec<DimensionPart>,
    /// The middle of the text's box.
    center: Point3,
    /// The text's rotation, in degrees.
    rotation: f64,
    text_point: Point3,
    placed: bool,
    arc_point: Option<Point3>,
    defpoints: Vec<Point3>,
}

impl Layout {
    /// Nothing drawn but the text, at `at`: what a dimension whose lines
    /// never meet lays out, which the description's reader refuses.
    fn empty(at: Point3) -> Layout {
        Layout {
            parts: vec![DimensionPart::Text],
            center: at,
            rotation: 0.0,
            text_point: at,
            placed: false,
            arc_point: None,
            defpoints: Vec::new(),
        }
    }
}

type V = [f64; 2];

fn add(p: Point3, v: V, k: f64) -> Point3 {
    Point3 {
        x: p.x + v[0] * k,
        y: p.y + v[1] * k,
        z: p.z,
    }
}

fn sub(a: Point3, b: Point3) -> V {
    [a.x - b.x, a.y - b.y]
}

fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn unit(v: V) -> V {
    let l = v[0].hypot(v[1]);
    if l == 0.0 {
        [1.0, 0.0]
    } else {
        [v[0] / l, v[1] / l]
    }
}

/// The direction to the left of `v`.
fn left(v: V) -> V {
    [-v[1], v[0]]
}

/// An angle, in degrees, turned into `(-90, 90]`, as AutoCAD turns
/// aligned text to read from the bottom or the right.
fn readable(degrees: f64) -> f64 {
    let mut a = degrees.rem_euclid(360.0);
    if a > 270.0 {
        a -= 360.0;
    } else if a > 90.0 {
        a -= 180.0;
    }
    a
}

fn direction(degrees: f64) -> V {
    let (s, c) = degrees.to_radians().sin_cos();
    [c, s]
}

/// How far from its middle along `e` a line leaves a box `half` wide and
/// high from its middle, axis aligned.
fn exit(e: V, half: V) -> f64 {
    let along_x = if e[0].abs() > 0.0 {
        half[0] / e[0].abs()
    } else {
        f64::INFINITY
    };
    let along_y = if e[1].abs() > 0.0 {
        half[1] / e[1].abs()
    } else {
        f64::INFINITY
    };
    along_x.min(along_y)
}

/// Whether `v` runs straight across, as AutoCAD takes a horizontal line.
fn level(v: V) -> bool {
    v[1].abs() <= 1e-12 * v[0].abs()
}

/// Whether a line along `v` runs `LANDING_ANGLE` or more from the
/// horizontal, and so ends in a landing under horizontal text.
fn steep(v: V) -> bool {
    v[1].abs().atan2(v[0].abs()).to_degrees() >= LANDING_ANGLE - 1e-9
}

/// A linear or aligned dimension's layout, its line along `u` through
/// `location`.
fn along(
    points: &[Point3; 2],
    u: V,
    location: Point3,
    style: &DimensionStyle,
    s: Sizes,
    (w, h): (f64, f64),
) -> Layout {
    let n = left(u);
    let onto = |p: Point3| add(p, n, dot(sub(location, p), n));
    let (p1, p2) = (points[0], points[1]);
    let (t1, t2) = (onto(p1), onto(p2));
    let length = dot(sub(t2, t1), sub(t2, t1)).sqrt();
    let e = unit(sub(t2, t1));
    let a = s.arrow;
    let g = s.gap;
    let trim = if style.arrowhead.trims_line() { a } else { 0.0 };
    let mut parts = Vec::new();
    for (p, t) in [(p1, t1), (p2, t2)] {
        let v = unit(sub(t, p));
        parts.push(DimensionPart::Line([
            add(p, v, s.offset),
            add(t, v, s.beyond),
        ]));
    }
    let aligned = style.placement == TextPlacement::Above;
    let angle = readable(e[1].atan2(e[0]).to_degrees());
    let up = left(direction(angle));
    // The text's length along the line, and a gap at each end.
    let half = if aligned {
        w / 2.0 + g
    } else {
        exit(e, [w / 2.0 + g, h / 2.0 + g])
    };
    let fits = |need: f64| need <= length + FIT_TOLERANCE * (1.0 + length);
    let text_fits = fits(2.0 * half);
    // No arrowheads need no room: they always fit, as AutoCAD fits them.
    let bare = style.arrowhead == Arrowhead::None;
    let arrows_fit = bare || fits(2.0 * (a + g));
    let both = if aligned || bare {
        text_fits && arrows_fit
    } else {
        fits(2.0 * half + 2.0 * (a + g))
    };
    // When each fits alone but not both, AutoCAD keeps inside the one that
    // needs more room, the text when they need the same.
    let (text_in, arrows_in) = if both {
        (true, true)
    } else if text_fits && arrows_fit {
        let text_first = 2.0 * half >= 2.0 * (a + g);
        (text_first, !text_first)
    } else if text_fits {
        (true, false)
    } else if arrows_fit {
        (false, true)
    } else {
        (false, false)
    };
    let mid = Point3 {
        x: (t1.x + t2.x) / 2.0,
        y: (t1.y + t2.y) / 2.0,
        z: t1.z,
    };
    // Where each arrow's body ends: inside, towards the other; outside, away.
    let inward = if arrows_in { 1.0 } else { -1.0 };
    let b1 = add(t1, e, inward * a);
    let b2 = add(t2, e, -inward * a);
    let (l1, l2) = (add(t1, e, inward * trim), add(t2, e, -inward * trim));
    let dle = if style.arrowhead == Arrowhead::None {
        s.line_extension
    } else {
        0.0
    };
    // The end of the line beyond the second extension line that outside
    // text starts from: past the arrowhead, and its tail when it is outside.
    let start = add(t2, e, if arrows_in { a } else { 2.0 * a });
    // Horizontal text outside goes the way the line reads, and lands
    // when the line runs steeply.
    let side = if e[0] > 0.0 || (e[0] == 0.0 && e[1] > 0.0) {
        1.0
    } else {
        -1.0
    };
    let landed = !text_in && !aligned && steep(e);
    let center = if text_in {
        if aligned {
            add(mid, up, g + h / 2.0)
        } else {
            mid
        }
    } else if aligned {
        add(add(start, e, g + w / 2.0), up, g + h / 2.0)
    } else if landed {
        add(start, [side, 0.0], a + g + w / 2.0)
    } else {
        add(start, [side, 0.0], g + w / 2.0)
    };
    match (text_in, arrows_in) {
        (true, true) => {
            if aligned {
                parts.push(DimensionPart::Line([add(l1, e, -dle), add(l2, e, dle)]));
            } else {
                let (from, to) = (add(center, e, -half), add(center, e, half));
                if dot(sub(from, l1), e) > 0.0 {
                    parts.push(DimensionPart::Line([add(l1, e, -dle), from]));
                }
                if dot(sub(l2, to), e) > 0.0 {
                    parts.push(DimensionPart::Line([add(l2, e, dle), to]));
                }
            }
        }
        (true, false) => {
            parts.push(DimensionPart::Line([l1, add(t1, e, -2.0 * a)]));
            parts.push(DimensionPart::Line([l2, add(t2, e, 2.0 * a)]));
            if aligned {
                parts.push(DimensionPart::Line([t1, t2]));
            }
        }
        (false, true) => {
            parts.push(DimensionPart::Line([add(l1, e, -dle), l2]));
            if aligned {
                parts.push(DimensionPart::Line([t2, add(start, e, 2.0 * g + w)]));
            } else {
                parts.push(DimensionPart::Line([t2, start]));
            }
        }
        (false, false) => {
            parts.push(DimensionPart::Line([l1, add(t1, e, -2.0 * a)]));
            if aligned {
                parts.push(DimensionPart::Line([l2, add(start, e, 2.0 * g + w)]));
                parts.push(DimensionPart::Line([t1, t2]));
            } else {
                parts.push(DimensionPart::Line([l2, start]));
            }
        }
    }
    if landed {
        parts.push(DimensionPart::Line([start, add(start, [side, 0.0], a)]));
    }
    if style.arrowhead != Arrowhead::None {
        parts.push(DimensionPart::Arrow { tip: t1, base: b1 });
        parts.push(DimensionPart::Arrow { tip: t2, base: b2 });
    }
    parts.push(DimensionPart::Text);
    Layout {
        parts,
        center,
        rotation: if aligned { angle } else { 0.0 },
        text_point: center,
        placed: false,
        arc_point: None,
        defpoints: vec![p1, p2, t2],
    }
}

/// A radius or diameter dimension's layout, of the circle round `c` with
/// radius `r`, its text towards `location`.
fn radial(
    c: Point3,
    r: f64,
    location: Point3,
    diameter: bool,
    style: &DimensionStyle,
    s: Sizes,
    (w, h): (f64, f64),
) -> Layout {
    let d = unit(sub(location, c));
    let reach = dot(sub(location, c), sub(location, c)).sqrt();
    let inside = reach < r;
    let tip = add(c, d, r);
    let far = add(c, d, -r);
    let a = s.arrow;
    let g = s.gap;
    let trim = if style.arrowhead.trims_line() { a } else { 0.0 };
    // Outside, the arrow points in from beyond the circle; inside, out.
    let out = if inside { -1.0 } else { 1.0 };
    let base = add(tip, d, out * a);
    let line_end = add(tip, d, out * trim);
    let has_arrow = style.arrowhead != Arrowhead::None;
    let mut parts = Vec::new();
    let (center, rotation, text_point);
    // Whether the text was moved out from nearer the circle than AutoCAD
    // allows: AutoCAD then keeps it as text at its own place.
    let moved;
    if style.placement == TextPlacement::Above {
        let angle = readable(d[1].atan2(d[0]).to_degrees());
        let up = left(direction(angle));
        let least = r + 2.0 * a + w / 2.0 + g;
        moved = !inside && reach < least;
        let along = if inside {
            reach.min(r - 2.0 * a - w / 2.0 - g)
        } else {
            reach.max(least)
        };
        let at = add(c, d, along);
        center = add(at, up, g + h / 2.0);
        rotation = angle;
        text_point = if moved { center } else { at };
        parts.push(DimensionPart::Line([
            add(at, d, out * (w / 2.0 + g)),
            line_end,
        ]));
        if diameter && !inside && has_arrow {
            let far_base = add(far, d, -trim);
            parts.push(DimensionPart::Line([far_base, add(far, d, -2.0 * a)]));
        }
        parts.push(DimensionPart::Text);
        if !inside {
            parts.push(DimensionPart::Line([if diameter { far } else { c }, tip]));
        }
        if has_arrow {
            if diameter && !inside {
                parts.push(DimensionPart::Arrow {
                    tip: far,
                    base: add(far, d, -a),
                });
            }
            parts.push(DimensionPart::Arrow { tip, base });
        }
    } else {
        moved = !inside && reach < r + 2.0 * a;
        let elbow_at = if inside {
            reach.min(r - 2.0 * a)
        } else {
            reach.max(r + 2.0 * a)
        };
        let elbow = add(c, d, elbow_at);
        // The text sits on the side away from the arrow.
        let side = if (d[0] < 0.0) == inside { 1.0 } else { -1.0 };
        let x = [side, 0.0];
        rotation = 0.0;
        if steep(d) {
            center = add(elbow, x, a + g + w / 2.0);
            text_point = center;
            parts.push(DimensionPart::Line([add(elbow, x, a), elbow]));
            parts.push(DimensionPart::Line([elbow, line_end]));
        } else if level(d) && inside {
            // Inside, on a level line, AutoCAD keeps the text's middle no
            // nearer the arrow than two arrowheads, a gap and half the
            // text, as text above its line, past the centre when the
            // circle is too small, and runs the line from the text on.
            let along = reach.min(r - 2.0 * a - g - w / 2.0);
            center = add(c, d, along);
            text_point = center;
            parts.push(DimensionPart::Line([add(center, d, w / 2.0 + g), line_end]));
        } else if level(d) {
            center = elbow;
            text_point = elbow;
            parts.push(DimensionPart::Line([
                add(elbow, x, -(w / 2.0 + g)),
                line_end,
            ]));
        } else {
            center = add(elbow, x, g + w / 2.0);
            text_point = elbow;
            parts.push(DimensionPart::Line([elbow, line_end]));
        }
        parts.push(DimensionPart::Text);
        if has_arrow {
            parts.push(DimensionPart::Arrow { tip, base });
        }
        if !inside && s.center_mark > 0.0 {
            let m = s.center_mark;
            parts.push(DimensionPart::Line([
                add(c, [1.0, 0.0], -m),
                add(c, [1.0, 0.0], m),
            ]));
            parts.push(DimensionPart::Line([
                add(c, [0.0, 1.0], -m),
                add(c, [0.0, 1.0], m),
            ]));
        }
    }
    Layout {
        parts,
        center,
        rotation,
        text_point,
        placed: !moved,
        arc_point: None,
        defpoints: vec![tip, if diameter { far } else { c }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point3 {
        Point3::new(x, y, 0.0)
    }

    /// The boxes AutoCAD 2026 gave the texts in `txt.shx` in the spike's
    /// own drawings, 2.5 high, by their contents.
    fn txt_box(height: f64) -> impl Fn(&MText) -> MTextBox {
        move |t: &MText| {
            let em = t.height / height;
            let width = match t.contents.as_str() {
                "50" | "30" | "40" => 3.75,
                "1" => 0.8333,
                "7" | "4" | "3" | "2" | "5" | "6" => 1.6667,
                "R15" => 5.8333,
                "\u{2205}30" => 6.25,
                other => 1.25 * other.chars().count() as f64 + 0.416_667,
            } * em;
            MTextBox {
                width,
                height: t.height,
                last_descent: 0.0,
                ..MTextBox::default()
            }
        }
    }

    fn close(a: Point3, b: Point3) {
        assert!(
            (a.x - b.x).abs() < 1e-4 && (a.y - b.y).abs() < 1e-4,
            "{a:?} is not {b:?}"
        );
    }

    fn linear(p1: Point3, p2: Point3, location: Point3, angle: f64) -> Dimension {
        Dimension {
            measured: Measured::Linear {
                points: [p1, p2],
                angle,
            },
            location,
            dimension_style: ISO_DIMENSION_STYLE.to_owned(),
            text: None,
            continued: false,
            layer: "0".to_owned(),
            properties: Properties::default(),
        }
    }

    fn lines(laid: &LaidDimension) -> Vec<[Point3; 2]> {
        laid.lines().collect()
    }

    #[test]
    fn an_iso_linear_dimension_is_laid_out_as_autocad_laid_out_its_own() {
        let style = DimensionStyle::iso_25(Unit::Mm);
        let laid = linear(p(10.0, 10.0), p(60.0, 10.0), p(35.0, 25.0), 0.0).laid_out(
            &style,
            Unit::Mm,
            &txt_box(2.5),
        );
        let l = lines(&laid);
        close(l[0][0], p(10.0, 10.625));
        close(l[0][1], p(10.0, 26.25));
        close(l[2][0], p(12.5, 25.0));
        close(l[2][1], p(57.5, 25.0));
        close(laid.text.position, p(35.0, 26.875));
        assert_eq!(laid.text.contents, "50");
        let arrows: Vec<_> = laid.arrows().collect();
        close(arrows[0].0, p(10.0, 25.0));
        close(arrows[0].1, p(12.5, 25.0));
    }

    /// AutoCAD's sweep of lengths in ISO-25: arrows outside below 6.25,
    /// text outside below its length and two gaps.
    #[test]
    fn iso_text_and_arrows_go_outside_as_autocad_put_them() {
        let style = DimensionStyle::iso_25(Unit::Mm);
        let one = linear(p(0.0, 0.0), p(1.0, 0.0), p(0.5, 8.0), 0.0).laid_out(
            &style,
            Unit::Mm,
            &txt_box(2.5),
        );
        // Both outside: tails, the line under the text, and the line between.
        let l = lines(&one);
        close(l[2][0], p(-2.5, 8.0));
        close(l[2][1], p(-5.0, 8.0));
        close(l[3][0], p(3.5, 8.0));
        close(l[3][1], p(8.0833, 8.0));
        close(l[4][0], p(0.0, 8.0));
        close(l[4][1], p(1.0, 8.0));
        close(one.text.position, p(7.0417, 9.875));
        let three = linear(p(0.0, 40.0), p(3.0, 40.0), p(1.5, 48.0), 0.0).laid_out(
            &style,
            Unit::Mm,
            &txt_box(2.5),
        );
        close(three.text.position, p(1.5, 49.875));
        assert_eq!(lines(&three).len(), 5);
        let seven = linear(p(0.0, 120.0), p(7.0, 120.0), p(3.5, 128.0), 0.0).laid_out(
            &style,
            Unit::Mm,
            &txt_box(2.5),
        );
        let l = lines(&seven);
        assert_eq!(l.len(), 3);
        close(l[2][0], p(2.5, 128.0));
        close(l[2][1], p(4.5, 128.0));
    }

    #[test]
    fn aligned_text_reads_from_the_bottom_or_the_right() {
        assert_eq!(readable(120.0), -60.0);
        assert_eq!(readable(90.0), 90.0);
        assert_eq!(readable(270.0), 90.0);
        assert_eq!(readable(210.0), 30.0);
        assert_eq!(readable(330.0), -30.0);
    }

    #[test]
    fn an_iso_radius_sits_on_its_line_and_is_moved_out_when_too_near() {
        let style = DimensionStyle::iso_25(Unit::Mm);
        let r45 = 0.5f64.sqrt();
        let radius = Dimension {
            measured: Measured::Radius {
                center: p(200.0, 100.0),
                point: p(200.0 + 15.0 * r45, 100.0 + 15.0 * r45),
            },
            location: p(200.0 + 25.0 * r45, 100.0 + 25.0 * r45),
            ..linear(p(0.0, 0.0), p(1.0, 0.0), p(0.0, 1.0), 0.0)
        };
        let laid = radius.laid_out(&style, Unit::Mm, &txt_box(2.5));
        assert_eq!(laid.text.contents, "R15");
        close(laid.text_point, p(217.6777, 117.6777));
        let l = lines(&laid);
        close(l[0][0], p(220.182, 120.182));
        close(l[0][1], p(212.3744, 112.3744));
        close(l[1][0], p(200.0, 100.0));
        let near = Dimension {
            location: p(200.0 + 16.0 * r45, 100.0 + 16.0 * r45),
            ..radius
        };
        // AutoCAD moved it to 23.5417 from the centre, and keeps it as
        // text at its own place, by its middle, 1.875 above the line.
        let moved = near.laid_out(&style, Unit::Mm, &txt_box(2.5));
        assert!(!moved.text_placed);
        close(
            moved.text_point,
            p(
                200.0 + 23.5417 * r45 - 1.875 * r45,
                100.0 + 23.5417 * r45 + 1.875 * r45,
            ),
        );
    }

    /// AutoCAD 2026's DIMRADIUS in Standard, inside circles 0.1 and 0.3 in
    /// across: on a level line the text's middle is kept two arrowheads, a
    /// gap and half the text short of the arrow, past the centre when the
    /// circle is too small, the line running on from the text; on a steep
    /// line the landing is kept two arrowheads short, past the centre too.
    #[test]
    fn a_standard_radius_inside_a_small_circle_is_laid_out_as_autocad_laid_it_out() {
        let style = DimensionStyle::standard(Unit::In);
        let measure = |t: &MText| MTextBox {
            width: 0.96,
            height: t.height,
            last_descent: 0.0,
            ..MTextBox::default()
        };
        let inside = |r: f64, d: f64, degrees: f64| {
            let u = direction(degrees);
            Dimension {
                measured: Measured::Radius {
                    center: p(0.0, 0.0),
                    point: p(r * u[0], r * u[1]),
                },
                location: p(d * u[0], d * u[1]),
                dimension_style: STANDARD_DIMENSION_STYLE.to_owned(),
                text: None,
                continued: false,
                layer: "0".to_owned(),
                properties: Properties::default(),
            }
            .laid_out(&style, Unit::In, &measure)
        };
        for (r, d) in [(0.3, 0.05), (0.3, 0.1), (0.3, 0.25)] {
            let level = inside(r, d, 0.0);
            close(level.text.position, p(-0.63, 0.0));
            close(level.text_point, p(-0.63, 0.0));
            let l = lines(&level);
            assert_eq!(l.len(), 1);
            close(l[0][0], p(-0.06, 0.0));
            close(l[0][1], p(0.12, 0.0));
        }
        let wide = inside(2.0, 0.3, 0.0);
        close(wide.text.position, p(0.3, 0.0));
        close(lines(&wide)[0][0], p(0.87, 0.0));
        let steep = inside(0.1, 0.05, 30.0);
        let u = direction(30.0);
        let elbow = p(-0.26 * u[0], -0.26 * u[1]);
        close(lines(&steep)[0][1], elbow);
        close(
            steep.text.position,
            p(elbow.x - 0.18 - 0.09 - 0.48, elbow.y),
        );
    }

    #[test]
    fn a_standard_radius_lands_its_text_when_steep_and_not_when_shallow() {
        let style = DimensionStyle::standard(Unit::In);
        let measure = |t: &MText| MTextBox {
            width: 0.93,
            height: t.height,
            last_descent: 0.0,
            ..MTextBox::default()
        };
        let at = |degrees: f64| {
            let d = direction(degrees);
            Dimension {
                measured: Measured::Radius {
                    center: p(0.0, 0.0),
                    point: p(1.5 * d[0], 1.5 * d[1]),
                },
                location: p(2.5 * d[0], 2.5 * d[1]),
                dimension_style: STANDARD_DIMENSION_STYLE.to_owned(),
                text: None,
                continued: false,
                layer: "0".to_owned(),
                properties: Properties::default(),
            }
            .laid_out(&style, Unit::In, &measure)
        };
        let steep = at(45.0);
        let d = direction(45.0);
        close(steep.text_point, p(2.5 * d[0] + 0.735, 2.5 * d[1]));
        assert_eq!(steep.text.contents, "R1.5000");
        let shallow = at(10.0);
        let d = direction(10.0);
        close(shallow.text_point, p(2.5 * d[0], 2.5 * d[1]));
        close(shallow.text.position, p(2.5 * d[0] + 0.555, 2.5 * d[1]));
        // Outside, the centre is marked.
        assert_eq!(lines(&steep).len(), 4);
    }

    #[test]
    fn a_value_is_formatted_as_the_style_says() {
        let iso = DimensionStyle::iso_25(Unit::Mm);
        assert_eq!(iso.format(49.641_016), "49,64");
        assert_eq!(iso.format(50.0), "50");
        assert_eq!(iso.format(2.5), "2,5");
        let standard = DimensionStyle::standard(Unit::In);
        assert_eq!(standard.format(5.0), "5.0000");
        let leading = DimensionStyle {
            suppress_leading_zeros: true,
            ..standard
        };
        assert_eq!(leading.format(0.5), ".5000");
    }

    #[test]
    fn a_prefix_or_suffix_takes_the_place_of_the_radius_sign() {
        let style = DimensionStyle {
            suffix: " mm".to_owned(),
            ..DimensionStyle::iso_25(Unit::Mm)
        };
        let radius = Dimension {
            measured: Measured::Radius {
                center: p(0.0, 0.0),
                point: p(6.0, 0.0),
            },
            ..linear(p(0.0, 0.0), p(1.0, 0.0), p(20.0, 0.0), 0.0)
        };
        assert_eq!(radius.displayed_text(&style), "6 mm");
        let own = Dimension {
            text: Some("HOLE <> TYP".to_owned()),
            ..radius
        };
        assert_eq!(
            own.displayed_text(&DimensionStyle::iso_25(Unit::Mm)),
            "HOLE R6 TYP"
        );
    }

    /// AutoCAD 2026 in ISO-25 with its text centred: 8 between points
    /// 7.996 apart, where the arrowheads need more room than the text and
    /// each fits alone, keeps the arrowheads inside and puts the text out.
    #[test]
    fn centred_text_gives_way_to_arrowheads_that_need_more_room() {
        let style = DimensionStyle {
            placement: TextPlacement::Centered,
            unit_format: UnitFormat::Fractional,
            precision: 4,
            ..DimensionStyle::iso_25(Unit::Mm)
        };
        let laid = linear(
            p(1760.0, 1540.0),
            p(1767.996, 1540.0),
            p(1764.0, 1548.0),
            0.0,
        )
        .laid_out(&style, Unit::Mm, &txt_box(2.5));
        assert_eq!(laid.text.contents, "8");
        close(laid.text.position, p(1771.9543, 1548.0));
        let arrows: Vec<_> = laid.arrows().collect();
        close(arrows[0].1, p(1762.5, 1548.0));
    }

    /// AutoCAD 2026 puts a style's suffix after an angle too.
    #[test]
    fn an_angle_takes_the_styles_prefix_and_suffix() {
        let style = DimensionStyle {
            suffix: " TYP".to_owned(),
            ..DimensionStyle::iso_25(Unit::Mm)
        };
        let angle = Dimension {
            measured: Measured::ThreePointAngular {
                center: p(0.0, 0.0),
                points: [p(10.0, 0.0), p(0.0, 10.0)],
            },
            ..linear(p(0.0, 0.0), p(1.0, 0.0), p(5.0, 5.0), 0.0)
        };
        assert_eq!(angle.displayed_text(&style), "90\u{b0} TYP");
    }
}
