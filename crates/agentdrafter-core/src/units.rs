//! Units of length.
//!
//! A sheet declares one unit and every number on it is in that unit unless
//! the number carries its own, in which case it is converted. An unknown unit
//! is an error; there is no default unit.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A unit of length a sheet description can declare or a number can carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Unit {
    Mm,
    Cm,
    M,
    In,
    Ft,
}

impl Unit {
    /// Every unit, in the order error messages list them.
    pub const ALL: [Unit; 5] = [Unit::Mm, Unit::Cm, Unit::M, Unit::In, Unit::Ft];

    /// The unit's symbol as written in a sheet description, such as `mm`.
    pub const fn symbol(self) -> &'static str {
        match self {
            Unit::Mm => "mm",
            Unit::Cm => "cm",
            Unit::M => "m",
            Unit::In => "in",
            Unit::Ft => "ft",
        }
    }

    /// The unit with this symbol, if there is one. Symbols are lower case.
    pub fn from_symbol(symbol: &str) -> Option<Unit> {
        Unit::ALL.into_iter().find(|u| u.symbol() == symbol)
    }

    /// Whether the unit is metric. A DWG records this beside its unit.
    pub const fn is_metric(self) -> bool {
        matches!(self, Unit::Mm | Unit::Cm | Unit::M)
    }

    /// The length of one of this unit in micrometres, a whole number for
    /// every unit, so the ratio between two units is an exact fraction.
    const fn micrometres(self) -> u64 {
        match self {
            Unit::Mm => 1_000,
            Unit::Cm => 10_000,
            Unit::M => 1_000_000,
            Unit::In => 25_400,
            Unit::Ft => 304_800,
        }
    }

    /// Converts `value` from this unit to `to`. A value already in `to` is
    /// returned unchanged, bit for bit. Between metric units, and between
    /// inches and feet, the conversion is one multiplication or division by
    /// a whole number, so it rounds once.
    pub fn convert(self, value: f64, to: Unit) -> f64 {
        let (from, to) = (self.micrometres(), to.micrometres());
        let common = gcd(from, to);
        // Both terms are below 2^53, so they are exact as f64.
        let (numerator, denominator) = ((from / common) as f64, (to / common) as f64);
        if denominator == 1.0 {
            value * numerator
        } else if numerator == 1.0 {
            value / denominator
        } else {
            value * numerator / denominator
        }
    }

    /// The symbols of every unit, joined for an error message: `mm, cm, m, in, ft`.
    pub(crate) fn symbols() -> String {
        Unit::ALL.map(Unit::symbol).join(", ")
    }
}

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.symbol())
    }
}

const fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Why a piece of text is not a length.
#[derive(Debug, PartialEq)]
pub(crate) enum LengthError {
    /// The text does not start with a plain decimal number.
    NotANumber,
    /// The number is too large to hold.
    OutOfRange,
    /// The number carries a unit that does not exist.
    UnknownUnit(String),
}

/// Reads a length written as a plain decimal number with an optional unit:
/// `100`, `-2.5`, `1e3`, `4 in`, `4in`.
///
/// The number is an optional `-`, digits, an optional `.` and digits, and an
/// optional exponent. Nothing else is a number: no `+`, no leading `.`, no
/// `_` separators, no hexadecimal, no `inf` or `nan`.
pub(crate) fn parse_length(text: &str) -> Result<(f64, Option<Unit>), LengthError> {
    let end = number_end(text).ok_or(LengthError::NotANumber)?;
    let (number, rest) = text.split_at(end);
    let value: f64 = number.parse().map_err(|_| LengthError::NotANumber)?;
    if !value.is_finite() {
        return Err(LengthError::OutOfRange);
    }
    let suffix = rest.strip_prefix(' ').unwrap_or(rest);
    if suffix.is_empty() {
        if rest.is_empty() {
            return Ok((value, None));
        }
        return Err(LengthError::NotANumber);
    }
    if !suffix.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(LengthError::NotANumber);
    }
    match Unit::from_symbol(suffix) {
        Some(unit) => Ok((value, Some(unit))),
        None => Err(LengthError::UnknownUnit(suffix.to_owned())),
    }
}

/// The byte length of the decimal number at the start of `text`, if it
/// starts with one.
fn number_end(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let digits = |from: usize| {
        bytes[from..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count()
    };
    let mut i = usize::from(bytes.first() == Some(&b'-'));
    let whole = digits(i);
    if whole == 0 {
        return None;
    }
    i += whole;
    if bytes.get(i) == Some(&b'.') {
        let fraction = digits(i + 1);
        if fraction == 0 {
            return None;
        }
        i += 1 + fraction;
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(bytes.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        let exponent = digits(j);
        if exponent > 0 {
            i = j + exponent;
        }
    }
    Some(i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_numbers_have_no_unit() {
        assert_eq!(parse_length("100"), Ok((100.0, None)));
        assert_eq!(parse_length("-2.5"), Ok((-2.5, None)));
        assert_eq!(parse_length("0"), Ok((0.0, None)));
        assert_eq!(parse_length("1e3"), Ok((1000.0, None)));
        assert_eq!(parse_length("1.5E-2"), Ok((0.015, None)));
    }

    #[test]
    fn a_number_can_carry_its_own_unit() {
        assert_eq!(parse_length("4 in"), Ok((4.0, Some(Unit::In))));
        assert_eq!(parse_length("4in"), Ok((4.0, Some(Unit::In))));
        assert_eq!(parse_length("-0.5 m"), Ok((-0.5, Some(Unit::M))));
        assert_eq!(parse_length("2 ft"), Ok((2.0, Some(Unit::Ft))));
    }

    #[test]
    fn an_unknown_unit_is_named() {
        assert_eq!(
            parse_length("3 furlong"),
            Err(LengthError::UnknownUnit("furlong".into()))
        );
        assert_eq!(
            parse_length("3 MM"),
            Err(LengthError::UnknownUnit("MM".into()))
        );
    }

    #[test]
    fn only_plain_decimals_are_numbers() {
        for text in [
            "", "abc", "+1", ".5", "5.", "1_000", "0x10", "inf", "nan", ".inf", "1 ", "1  mm",
            "1 mm2", "1-", "--1", "1,5", "~", "null",
        ] {
            assert_eq!(
                parse_length(text),
                Err(LengthError::NotANumber),
                "{text:?} should not be a number"
            );
        }
    }

    #[test]
    fn a_huge_number_is_out_of_range() {
        assert_eq!(parse_length("1e400"), Err(LengthError::OutOfRange));
    }

    #[test]
    fn conversion_is_exact_for_the_same_unit_and_right_across_units() {
        assert_eq!(Unit::Mm.convert(0.1, Unit::Mm).to_bits(), 0.1f64.to_bits());
        assert_eq!(Unit::In.convert(1.0, Unit::Mm), 25.4);
        assert_eq!(Unit::Ft.convert(1.0, Unit::In), 12.0);
        assert_eq!(Unit::In.convert(18.0, Unit::Ft), 1.5);
        assert_eq!(Unit::Ft.convert(-1.0, Unit::Mm), -304.8);
        assert_eq!(Unit::M.convert(1.5, Unit::Mm), 1500.0);
        assert_eq!(Unit::Mm.convert(2.5, Unit::M), 0.0025);
        assert_eq!(Unit::Cm.convert(2.0, Unit::Mm), 20.0);
        assert_eq!(Unit::Mm.convert(25.4, Unit::In), 1.0);
    }

    #[test]
    fn symbols_round_trip() {
        for unit in Unit::ALL {
            assert_eq!(Unit::from_symbol(unit.symbol()), Some(unit));
        }
        assert_eq!(Unit::symbols(), "mm, cm, m, in, ft");
    }
}
