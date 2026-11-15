//! How a dimension style shows a length and an angle, as AutoCAD 2026
//! showed made-up lengths and angles in every unit format of its `acad`
//! and `acadiso` templates' styles:
//!
//! - A value is rounded half away from zero, as AutoCAD rounds it: `12.5`
//!   whole inches is `13`, `100.0625` to three decimals `100.063`.
//! - Decimal lengths and angles take the style's decimal separator;
//!   scientific, engineering and Windows desktop lengths, and the seconds
//!   of degrees, minutes and seconds, a period, whatever the separator. A
//!   Windows desktop length groups its thousands with commas, as Windows
//!   writes a number in US English, the laptop's: `2,380.44`.
//! - Engineering and architectural lengths count inches, twelve to the
//!   foot: `1'-0.50"` and `1'-0½"`. Zero feet, and zero inches, are left
//!   out as the style says, the inches always shown when the feet are not.
//!   An architectural or fractional length's fraction is in halves,
//!   quarters and on to 256ths, as fine as the style's precision says,
//!   reduced, and stacked as the style says.
//! - An angle rounded to a whole turn shows as `0`, in degrees and in
//!   degrees, minutes and seconds.

use serde::{Deserialize, Serialize};

use super::{joined, DimensionStyle, TextPiece};

/// How a style shows lengths: AutoCAD's DIMLUNIT.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitFormat {
    /// `1.25E+01`.
    Scientific,
    /// `12.50`, with the style's decimal separator.
    #[default]
    Decimal,
    /// Feet and decimal inches, `1'-0.50"`.
    Engineering,
    /// Feet and fractional inches, `1'-0½"`.
    Architectural,
    /// A whole number and a fraction, `12½`.
    Fractional,
    /// Decimal with a period, as Windows shows it here.
    WindowsDesktop,
}

impl UnitFormat {
    pub const ALL: [UnitFormat; 6] = [
        UnitFormat::Scientific,
        UnitFormat::Decimal,
        UnitFormat::Engineering,
        UnitFormat::Architectural,
        UnitFormat::Fractional,
        UnitFormat::WindowsDesktop,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            UnitFormat::Scientific => "scientific",
            UnitFormat::Decimal => "decimal",
            UnitFormat::Engineering => "engineering",
            UnitFormat::Architectural => "architectural",
            UnitFormat::Fractional => "fractional",
            UnitFormat::WindowsDesktop => "windows_desktop",
        }
    }

    /// AutoCAD's DIMLUNIT for it.
    pub const fn code(self) -> i16 {
        match self {
            UnitFormat::Scientific => 1,
            UnitFormat::Decimal => 2,
            UnitFormat::Engineering => 3,
            UnitFormat::Architectural => 4,
            UnitFormat::Fractional => 5,
            UnitFormat::WindowsDesktop => 6,
        }
    }

    pub fn of_code(code: i16) -> Option<UnitFormat> {
        UnitFormat::ALL.into_iter().find(|f| f.code() == code)
    }

    /// Whether it counts inches and feet, and so needs a sheet in inches.
    pub const fn counts_inches(self) -> bool {
        matches!(self, UnitFormat::Engineering | UnitFormat::Architectural)
    }
}

/// How an architectural or fractional length's fraction shows: AutoCAD's
/// DIMFRAC.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FractionFormat {
    /// Stacked over a bar.
    #[default]
    Horizontal,
    /// Stacked across a slash.
    Diagonal,
    /// On the line, `1/2`.
    NotStacked,
}

impl FractionFormat {
    pub const ALL: [FractionFormat; 3] = [
        FractionFormat::Horizontal,
        FractionFormat::Diagonal,
        FractionFormat::NotStacked,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            FractionFormat::Horizontal => "horizontal",
            FractionFormat::Diagonal => "diagonal",
            FractionFormat::NotStacked => "not_stacked",
        }
    }

    pub const fn code(self) -> i16 {
        match self {
            FractionFormat::Horizontal => 0,
            FractionFormat::Diagonal => 1,
            FractionFormat::NotStacked => 2,
        }
    }

    pub fn of_code(code: i16) -> Option<FractionFormat> {
        FractionFormat::ALL.into_iter().find(|f| f.code() == code)
    }
}

/// How a style shows angles: AutoCAD's DIMAUNIT.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AngularUnit {
    /// `45.5°`.
    #[default]
    DecimalDegrees,
    /// `45°30'0"`.
    DegreesMinutesSeconds,
    /// `50.56g`.
    Gradians,
    /// `0.794r`.
    Radians,
}

impl AngularUnit {
    pub const ALL: [AngularUnit; 4] = [
        AngularUnit::DecimalDegrees,
        AngularUnit::DegreesMinutesSeconds,
        AngularUnit::Gradians,
        AngularUnit::Radians,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            AngularUnit::DecimalDegrees => "decimal_degrees",
            AngularUnit::DegreesMinutesSeconds => "degrees_minutes_seconds",
            AngularUnit::Gradians => "gradians",
            AngularUnit::Radians => "radians",
        }
    }

    pub const fn code(self) -> i16 {
        match self {
            AngularUnit::DecimalDegrees => 0,
            AngularUnit::DegreesMinutesSeconds => 1,
            AngularUnit::Gradians => 2,
            AngularUnit::Radians => 3,
        }
    }

    pub fn of_code(code: i16) -> Option<AngularUnit> {
        AngularUnit::ALL.into_iter().find(|u| u.code() == code)
    }
}

/// The degree sign.
pub const DEGREE_SIGN: char = '\u{00B0}';

/// How near a half a value may fall short and still round up, as a share
/// of itself: AutoCAD rounds 6.374999999999991, a radius of 6.375 as its
/// points give it, to 6.38, yet shows an angle of 135 degrees at eight decimals
/// as 135, which a slack of 1e-10 would push to 135.00000001.
const ROUNDING_SLACK: f64 = 1e-12;

/// The most a value may fall short of a half and still round up, in the
/// last digit shown: a hair, so that a value far from a half, however
/// large, rounds as it is.
const MOST_SLACK: f64 = 1e-6;

/// `value`, 0 or more, times `factor`, rounded half away from zero, a
/// value a hair short of a half rounding up, as AutoCAD rounds it.
fn scaled(value: f64, factor: f64) -> u128 {
    let x = value.abs() * factor;
    let short = 0.5 - (x - x.floor());
    let rounded = if short > 0.0 && short <= (x * ROUNDING_SLACK).min(MOST_SLACK) {
        x.ceil()
    } else {
        x.round()
    };
    // A float casts to an integer saturating, so a huge value shows as the
    // largest the integer holds rather than wrapping.
    rounded as u128
}

/// The number `n / 10^decimals` written out: its zeros suppressed as
/// `leading` and `trailing` say, `separator` between the whole part and
/// the decimals.
fn decimal_text(n: u128, decimals: u32, leading: bool, trailing: bool, separator: char) -> String {
    let unit = 10u128.pow(decimals);
    let whole = n / unit;
    let mut shown = whole.to_string();
    let mut fraction = if decimals == 0 {
        String::new()
    } else {
        format!("{:0width$}", n % unit, width = decimals as usize)
    };
    if trailing {
        fraction = fraction.trim_end_matches('0').to_owned();
    }
    if leading && whole == 0 && !fraction.is_empty() {
        shown.clear();
    }
    if !fraction.is_empty() {
        shown.push(separator);
        shown.push_str(&fraction);
    }
    shown
}

/// A decimal number's whole part grouped in thousands with commas, as
/// Windows writes a number in US English: `2,380.44`.
fn grouped(number: &str) -> String {
    let (whole, rest) = number.split_at(number.find('.').unwrap_or(number.len()));
    let digits: Vec<char> = whole.chars().collect();
    let mut out = String::with_capacity(number.len() + digits.len() / 3);
    for (i, c) in digits.iter().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*c);
    }
    out.push_str(rest);
    out
}

/// A fraction `numerator / denominator` reduced.
fn reduced(numerator: u128, denominator: u128) -> (u128, u128) {
    let (mut a, mut b) = (numerator, denominator);
    while b != 0 {
        (a, b) = (b, a % b);
    }
    (numerator / a, denominator / a)
}

impl DimensionStyle {
    /// The precision angles show with: DIMADEC.
    pub fn angle_decimals(&self) -> u8 {
        self.angular_precision
    }

    /// `value` as a decimal number with the style's precision, zeros and
    /// `separator`.
    fn decimal_with(&self, value: f64, separator: char) -> String {
        let decimals = u32::from(self.precision);
        decimal_text(
            scaled(value, 10f64.powi(decimals as i32)),
            decimals,
            self.suppress_leading_zeros,
            self.suppress_trailing_zeros,
            separator,
        )
    }

    /// `value` as the style shows a length, in pieces where a fraction is
    /// stacked.
    pub fn length_pieces(&self, value: f64) -> Vec<TextPiece> {
        let plain = |s: String| vec![TextPiece::Plain(s)];
        match self.unit_format {
            UnitFormat::Decimal => {
                plain(self.decimal_with(value, self.decimal_separator.character()))
            }
            UnitFormat::WindowsDesktop => plain(grouped(&self.decimal_with(value, '.'))),
            UnitFormat::Scientific => plain(self.scientific(value)),
            UnitFormat::Engineering => plain(self.engineering(value)),
            UnitFormat::Architectural => self.architectural(value),
            UnitFormat::Fractional => self.fractional(value),
        }
    }

    /// `1.25E+01`: one digit, the precision's decimals, and a signed
    /// exponent of two digits or more.
    fn scientific(&self, value: f64) -> String {
        let decimals = u32::from(self.precision);
        let unit = 10u128.pow(decimals);
        let value = value.abs();
        // The mantissa rounds as every length does, half away from zero:
        // 6.375 to two decimals is 6.38E+00, where Rust's own exponent
        // format, rounding a tie to even, gives 6.37E+00.
        let mut exponent = if value > 0.0 && value.is_finite() {
            value.log10().floor() as i32
        } else {
            0
        };
        let mantissa = |exponent: i32| scaled(value / 10f64.powi(exponent), unit as f64);
        let mut n = mantissa(exponent);
        if n >= 10 * unit {
            exponent += 1;
            n = mantissa(exponent);
        } else if n < unit && value > 0.0 {
            exponent -= 1;
            n = mantissa(exponent);
        }
        let mantissa = decimal_text(n, decimals, false, false, '.');
        let sign = if exponent < 0 { '-' } else { '+' };
        format!("{mantissa}E{sign}{:02}", exponent.abs())
    }

    /// Whether zero feet, and zero inches, are left out: DIMZIN's 0 to 3.
    fn feet_and_inches(&self, feet: u128, inches_zero: bool, inches: String) -> String {
        if feet == 0 && self.suppress_zero_feet {
            format!("{inches}\"")
        } else if inches_zero && self.suppress_zero_inches {
            format!("{feet}'")
        } else {
            format!("{feet}'-{inches}\"")
        }
    }

    /// `1'-0.50"`: feet and decimal inches.
    fn engineering(&self, value: f64) -> String {
        let decimals = u32::from(self.precision);
        let unit = 10u128.pow(decimals);
        let n = scaled(value, unit as f64);
        let per_foot = 12 * unit;
        let (feet, rest) = (n / per_foot, n % per_foot);
        let inches = decimal_text(
            rest,
            decimals,
            self.suppress_leading_zeros,
            self.suppress_trailing_zeros,
            '.',
        );
        self.feet_and_inches(feet, rest == 0, inches)
    }

    /// The fraction's denominator: 2 to the precision.
    fn denominator(&self) -> u128 {
        1u128 << self.precision.min(8)
    }

    /// A whole number and a fraction of `1 / denominator`ths, the fraction
    /// as the style stacks it; the whole number left out when it is 0 and
    /// `whole_zero` does not keep it.
    fn whole_and_fraction(&self, whole: u128, numerator: u128, whole_zero: bool) -> Vec<TextPiece> {
        let mut pieces = Vec::new();
        let whole_shown = whole != 0 || numerator == 0 || whole_zero;
        if whole_shown {
            pieces.push(TextPiece::Plain(whole.to_string()));
        }
        if numerator != 0 {
            let (n, d) = reduced(numerator, self.denominator());
            match self.fraction_format {
                FractionFormat::NotStacked => {
                    let gap = if whole_shown { " " } else { "" };
                    pieces.push(TextPiece::Plain(format!("{gap}{n}/{d}")));
                }
                stacked => pieces.push(TextPiece::Fraction {
                    numerator: n.to_string(),
                    denominator: d.to_string(),
                    diagonal: stacked == FractionFormat::Diagonal,
                }),
            }
        }
        pieces
    }

    /// `1'-0½"`: feet and fractional inches.
    fn architectural(&self, value: f64) -> Vec<TextPiece> {
        let d = self.denominator();
        let n = scaled(value, d as f64);
        let per_foot = 12 * d;
        let (feet, rest) = (n / per_foot, n % per_foot);
        let feet_shown = !(feet == 0 && self.suppress_zero_feet);
        let inches = self.whole_and_fraction(rest / d, rest % d, feet_shown);
        if feet_shown && rest == 0 && self.suppress_zero_inches {
            return vec![TextPiece::Plain(format!("{feet}'"))];
        }
        let mut pieces = Vec::with_capacity(inches.len() + 2);
        if feet_shown {
            pieces.push(TextPiece::Plain(format!("{feet}'-")));
        }
        pieces.extend(inches);
        pieces.push(TextPiece::Plain("\"".to_owned()));
        joined(pieces)
    }

    /// `12½`: a whole number and a fraction.
    fn fractional(&self, value: f64) -> Vec<TextPiece> {
        let d = self.denominator();
        let n = scaled(value, d as f64);
        self.whole_and_fraction(n / d, n % d, false)
    }

    /// `degrees`, from 0 to 360, as the style shows an angle.
    pub fn angle_text(&self, degrees: f64) -> String {
        let decimals = u32::from(self.angle_decimals());
        let separator = self.decimal_separator.character();
        let leading = self.suppress_angular_leading_zeros;
        let trailing = self.suppress_angular_trailing_zeros;
        let in_turns = |value: f64, turn: f64, mark: &str| {
            let unit = 10u128.pow(decimals);
            let mut n = scaled(value, unit as f64);
            // A value rounded to a whole turn is no turn.
            if n == scaled(turn, unit as f64) {
                n = 0;
            }
            format!(
                "{}{mark}",
                decimal_text(n, decimals, leading, trailing, separator)
            )
        };
        match self.angular_unit {
            AngularUnit::DecimalDegrees => in_turns(degrees, 360.0, &DEGREE_SIGN.to_string()),
            AngularUnit::Gradians => in_turns(degrees * 400.0 / 360.0, 400.0, "g"),
            AngularUnit::Radians => {
                let unit = 10u128.pow(decimals);
                let n = scaled(degrees.to_radians(), unit as f64);
                format!(
                    "{}r",
                    decimal_text(n, decimals, leading, trailing, separator)
                )
            }
            AngularUnit::DegreesMinutesSeconds => self.dms(degrees, decimals),
        }
    }

    /// `45°30'0"`: whole degrees at precision 0, and minutes at 1 and 2,
    /// seconds at 3 and 4, and the seconds' decimals from 5 on.
    fn dms(&self, degrees: f64, decimals: u32) -> String {
        let (per_degree, second_decimals): (u128, u32) = match decimals {
            0 => (1, 0),
            1 | 2 => (60, 0),
            3 | 4 => (3600, 0),
            d => (3600 * 10u128.pow(d - 4), d - 4),
        };
        let mut n = scaled(degrees, per_degree as f64);
        if n == 360 * per_degree {
            n = 0;
        }
        let whole = n / per_degree;
        let rest = n % per_degree;
        match decimals {
            0 => format!("{whole}{DEGREE_SIGN}"),
            1 | 2 => format!("{whole}{DEGREE_SIGN}{rest}'"),
            _ => {
                let per_minute = per_degree / 60;
                let (minutes, seconds) = (rest / per_minute, rest % per_minute);
                let seconds = decimal_text(seconds, second_decimals, false, false, '.');
                format!("{whole}{DEGREE_SIGN}{minutes}'{seconds}\"")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::Unit;

    fn standard(set: impl FnOnce(&mut DimensionStyle)) -> DimensionStyle {
        let mut style = DimensionStyle::standard(Unit::In);
        set(&mut style);
        style
    }

    /// What a length's pieces say, with each stacked fraction written as
    /// AutoCAD writes it in its MTEXT.
    fn coded(style: &DimensionStyle, value: f64) -> String {
        style
            .length_pieces(value)
            .iter()
            .map(|p| match p {
                TextPiece::Plain(s) => s.clone(),
                TextPiece::Fraction {
                    numerator,
                    denominator,
                    diagonal,
                } => format!(
                    "{{\\H1x;\\S{numerator}{}{denominator};}}",
                    if *diagonal { '#' } else { '/' }
                ),
            })
            .collect()
    }

    /// AutoCAD 2026's texts for the lengths of its sweep, in Standard.
    #[test]
    fn lengths_show_as_autocad_showed_them_in_every_unit_format() {
        let sci = standard(|s| {
            s.unit_format = UnitFormat::Scientific;
            s.precision = 2;
        });
        assert_eq!(coded(&sci, 0.5), "5.00E-01");
        assert_eq!(coded(&sci, 13.375), "1.34E+01");
        assert_eq!(coded(&sci, 100.0625), "1.00E+02");
        assert_eq!(coded(&sci, 6.375), "6.38E+00");
        assert_eq!(coded(&sci, 12.75), "1.28E+01");
        assert_eq!(coded(&sci, 9.996), "1.00E+01");
        assert_eq!(coded(&sci, 0.0), "0.00E+00");
        let eng = |zin: u8, decimals: u8| {
            standard(|s| {
                s.unit_format = UnitFormat::Engineering;
                s.precision = decimals;
                s.suppress_zero_feet = matches!(zin & 3, 0 | 3);
                s.suppress_zero_inches = matches!(zin & 3, 0 | 2);
                s.suppress_leading_zeros = zin & 4 != 0;
                s.suppress_trailing_zeros = zin & 8 != 0;
            })
        };
        assert_eq!(coded(&eng(0, 2), 0.5), "0.50\"");
        assert_eq!(coded(&eng(0, 2), 12.0), "1'");
        assert_eq!(coded(&eng(0, 2), 12.5), "1'-0.50\"");
        assert_eq!(coded(&eng(0, 2), 7.996), "8.00\"");
        // A radius of 6.375 as its points give it.
        assert_eq!(coded(&eng(0, 2), 6.374_999_999_999_991), "6.38\"");
        assert_eq!(coded(&eng(1, 2), 12.0), "1'-0.00\"");
        assert_eq!(coded(&eng(1, 2), 3.1375), "0'-3.14\"");
        assert_eq!(coded(&eng(2, 2), 0.5), "0'-0.50\"");
        assert_eq!(coded(&eng(2, 2), 96.0), "8'");
        assert_eq!(coded(&eng(3, 2), 96.0), "8'-0.00\"");
        assert_eq!(coded(&eng(8, 4), 1.0), "1\"");
        assert_eq!(coded(&eng(8, 4), 100.0625), "8'-4.0625\"");
        assert_eq!(coded(&eng(4, 4), 12.5), "1'-.5000\"");
        let arch = |zin: u8, decimals: u8, fraction: FractionFormat| {
            standard(|s| {
                s.unit_format = UnitFormat::Architectural;
                s.precision = decimals;
                s.fraction_format = fraction;
                s.suppress_zero_feet = matches!(zin & 3, 0 | 3);
                s.suppress_zero_inches = matches!(zin & 3, 0 | 2);
            })
        };
        let h = FractionFormat::Horizontal;
        assert_eq!(coded(&arch(0, 4, h), 0.5), "{\\H1x;\\S1/2;}\"");
        assert_eq!(coded(&arch(0, 4, h), 1.0), "1\"");
        assert_eq!(coded(&arch(0, 4, h), 12.0), "1'");
        assert_eq!(coded(&arch(0, 4, h), 12.5), "1'-0{\\H1x;\\S1/2;}\"");
        assert_eq!(coded(&arch(0, 4, h), 0.03), "0\"");
        assert_eq!(coded(&arch(0, 4, h), 1.0 / 3.0), "{\\H1x;\\S5/16;}\"");
        assert_eq!(coded(&arch(1, 4, h), 0.5), "0'-0{\\H1x;\\S1/2;}\"");
        assert_eq!(coded(&arch(1, 4, h), 0.03), "0'-0\"");
        assert_eq!(coded(&arch(1, 4, h), 12.0), "1'-0\"");
        assert_eq!(coded(&arch(2, 4, h), 0.03), "0'");
        assert_eq!(coded(&arch(2, 4, h), 12.0), "1'");
        assert_eq!(coded(&arch(3, 4, h), 12.0), "1'-0\"");
        assert_eq!(coded(&arch(3, 4, h), 0.5), "{\\H1x;\\S1/2;}\"");
        assert_eq!(coded(&arch(0, 0, h), 12.5), "1'-1\"");
        assert_eq!(coded(&arch(0, 8, h), 7.996), "7{\\H1x;\\S255/256;}\"");
        assert_eq!(
            coded(&arch(0, 4, FractionFormat::Diagonal), 13.375),
            "1'-1{\\H1x;\\S3#8;}\""
        );
        let flat = FractionFormat::NotStacked;
        assert_eq!(coded(&arch(0, 4, flat), 12.5), "1'-0 1/2\"");
        assert_eq!(coded(&arch(0, 4, flat), 0.0625), "1/16\"");
        let frac = |decimals: u8, fraction: FractionFormat| {
            standard(|s| {
                s.unit_format = UnitFormat::Fractional;
                s.precision = decimals;
                s.fraction_format = fraction;
            })
        };
        assert_eq!(coded(&frac(4, h), 0.5), "{\\H1x;\\S1/2;}");
        assert_eq!(coded(&frac(4, h), 100.0625), "100{\\H1x;\\S1/16;}");
        assert_eq!(coded(&frac(4, h), 0.03), "0");
        assert_eq!(coded(&frac(4, flat), 12.5), "12 1/2");
        assert_eq!(coded(&frac(2, h), 100.0625), "100");
        let win = standard(|s| {
            s.unit_format = UnitFormat::WindowsDesktop;
            s.precision = 3;
            s.suppress_trailing_zeros = true;
            s.decimal_separator = crate::model::DecimalSeparator::Comma;
        });
        assert_eq!(coded(&win, 100.0625), "100.063");
        assert_eq!(coded(&win, 2380.4375), "2,380.438");
        assert_eq!(coded(&win, 1_234_567.0), "1,234,567");
        assert_eq!(coded(&win, 12.0), "12");
    }

    /// A value a hair short of a half rounds up, and only such a value: an
    /// exact one, however many digits it shows, stays as it is.
    #[test]
    fn only_a_value_a_hair_short_of_a_half_rounds_up() {
        let mut iso = DimensionStyle::iso_25(Unit::Mm);
        iso.precision = 8;
        assert_eq!(coded(&iso, 10000.0), "10000");
        assert_eq!(coded(&iso, 10_000.000_000_003), "10000");
        iso.angular_precision = 8;
        iso.suppress_angular_trailing_zeros = true;
        assert_eq!(iso.angle_text(135.0), "135\u{b0}");
        assert_eq!(iso.angle_text(1f64.atan2(-1.0).to_degrees()), "135\u{b0}");
        iso.precision = 2;
        assert_eq!(coded(&iso, 6.374_999_999_999_991), "6,38");
        assert_eq!(coded(&iso, 6.374), "6,37");
    }

    /// AutoCAD 2026's texts for the angles of its sweep.
    #[test]
    fn angles_show_as_autocad_showed_them_in_every_angular_unit() {
        let iso = |unit: AngularUnit, decimals: u8, azin: u8| {
            let mut s = DimensionStyle::iso_25(Unit::Mm);
            s.angular_unit = unit;
            s.angular_precision = decimals;
            s.suppress_angular_leading_zeros = azin & 1 != 0;
            s.suppress_angular_trailing_zeros = azin & 2 != 0;
            s
        };
        let deg = AngularUnit::DecimalDegrees;
        assert_eq!(iso(deg, 0, 0).angle_text(45.5), "46\u{b0}");
        assert_eq!(iso(deg, 2, 0).angle_text(120.758333), "120,76\u{b0}");
        assert_eq!(iso(deg, 2, 2).angle_text(30.0), "30\u{b0}");
        assert_eq!(iso(deg, 2, 1).angle_text(0.25), ",25\u{b0}");
        assert_eq!(iso(deg, 2, 0).angle_text(7.5), "7,50\u{b0}");
        assert_eq!(iso(deg, 1, 0).angle_text(359.9999), "0,0\u{b0}");
        let dms = AngularUnit::DegreesMinutesSeconds;
        assert_eq!(iso(dms, 0, 0).angle_text(45.5), "46\u{b0}");
        assert_eq!(iso(dms, 1, 0).angle_text(120.758333), "120\u{b0}45'");
        assert_eq!(iso(dms, 1, 0).angle_text(359.9999), "0\u{b0}0'");
        assert_eq!(iso(dms, 3, 0).angle_text(120.758333), "120\u{b0}45'30\"");
        assert_eq!(iso(dms, 4, 3).angle_text(30.0), "30\u{b0}0'0\"");
        assert_eq!(iso(dms, 5, 0).angle_text(359.9999), "359\u{b0}59'59.6\"");
        assert_eq!(
            iso(dms, 8, 0).angle_text(120.758333),
            "120\u{b0}45'29.9988\""
        );
        assert_eq!(iso(AngularUnit::Gradians, 2, 0).angle_text(30.0), "33,33g");
        assert_eq!(
            iso(AngularUnit::Gradians, 4, 2).angle_text(359.9999),
            "399,9999g"
        );
        assert_eq!(iso(AngularUnit::Radians, 3, 0).angle_text(90.0), "1,571r");
        assert_eq!(iso(AngularUnit::Radians, 3, 3).angle_text(30.0), ",524r");
        assert_eq!(iso(AngularUnit::Radians, 0, 0).angle_text(359.9999), "6r");
    }
}
