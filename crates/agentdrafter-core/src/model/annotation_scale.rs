//! Annotation scales (`ANNOTATIVE`, `CANNOSCALE`, `ANNOALLVISIBLE`).
//!
//! An annotation scale relates a length on paper to a length in model space,
//! such as `1:50` (`1` paper unit to `50` drawing units, factor `0.02`).
//! Annotative text, multiline text, dimensions, multileaders, hatches, and
//! blocks declare their paper size and the list of scales they support.
//! Through a viewport at one of those scales, the drawing pipeline scales them
//! by the inverse factor (`drawing_units / paper_units`) so they land on paper
//! at their declared paper size; through a viewport whose scale they do not
//! support, they are hidden (unless `anno_all_visible` is `true`).

use serde::{Deserialize, Serialize};

/// AutoCAD's default full-size annotation scale (`1:1`).
pub const DEFAULT_ANNOTATION_SCALE: &str = "1:1";

/// Relative tolerance when matching two scale factors (e.g., viewport scale vs
/// an annotation scale's `paper_units / drawing_units`).
const SCALE_MATCH_TOLERANCE: f64 = 1e-5;

/// One named annotation scale, matching AutoCAD's `SCALE` object in
/// `ACAD_SCALELIST`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnnotationScale {
    /// AutoCAD scale name, such as `"1:1"`, `"1:50"`, `"2:1"`, or
    /// `"1/4\" = 1'-0\""`.
    pub name: String,
    /// Paper units, strictly greater than 0 (for example `1.0` in `"1:50"`).
    pub paper_units: f64,
    /// Drawing (model) units, strictly greater than 0 (for example `50.0` in
    /// `"1:50"`).
    pub drawing_units: f64,
}

impl AnnotationScale {
    /// Creates an annotation scale with the given name and positive units.
    pub fn new(name: impl Into<String>, paper_units: f64, drawing_units: f64) -> Self {
        Self {
            name: name.into(),
            paper_units,
            drawing_units,
        }
    }

    /// The built-in `1:1` full-size annotation scale.
    pub fn one_to_one() -> Self {
        Self::new(DEFAULT_ANNOTATION_SCALE, 1.0, 1.0)
    }

    /// Paper units per drawing unit (`paper_units / drawing_units`, AutoCAD's
    /// `CANNOSCALEVALUE`), e.g. `0.02` for `"1:50"`.
    pub fn factor(&self) -> f64 {
        self.paper_units / self.drawing_units
    }

    /// Drawing units per paper unit (`drawing_units / paper_units`), e.g.
    /// `50.0` for `"1:50"`. Multiplying an annotative object's paper size by
    /// this value gives its model-space size at this scale.
    pub fn inverse_factor(&self) -> f64 {
        self.drawing_units / self.paper_units
    }

    /// Whether this scale matches `factor` (`paper / drawing`) within
    /// floating-point tolerance.
    pub fn matches_factor(&self, factor: f64) -> bool {
        if !factor.is_finite() || factor <= 0.0 {
            return false;
        }
        let own = self.factor();
        let ref_mag = own.abs().max(factor.abs()).max(1e-12);
        (own - factor).abs() <= SCALE_MATCH_TOLERANCE * ref_mag
    }

    /// Parses a standard scale name such as `"1:50"`, `"2:1"`, `"1:2.5"`,
    /// `"1/4\" = 1'-0\""`, or `"1\" = 20'"` into an [`AnnotationScale`].
    pub fn from_name(name: &str) -> Option<Self> {
        let trimmed = name.trim();
        let (paper, drawing) = parse_scale_ratio(trimmed)?;
        Some(Self::new(trimmed, paper, drawing))
    }

    /// Constructs a canonical scale from a viewport scale factor
    /// (`paper / drawing`), formatting names like `"1:50"`, `"1:1"`, or
    /// `"2:1"`.
    pub fn from_factor(factor: f64) -> Self {
        if !factor.is_finite() || factor <= 0.0 {
            return Self::one_to_one();
        }
        if (factor - 1.0).abs() <= SCALE_MATCH_TOLERANCE {
            return Self::one_to_one();
        }
        if factor < 1.0 {
            let inv = 1.0 / factor;
            let rounded = inv.round();
            let d = if (inv - rounded).abs() <= SCALE_MATCH_TOLERANCE * rounded.max(1.0) {
                rounded
            } else {
                inv
            };
            Self::new(format!("1:{}", format_scale_number(d)), 1.0, d)
        } else {
            let rounded = factor.round();
            let p = if (factor - rounded).abs() <= SCALE_MATCH_TOLERANCE * rounded.max(1.0) {
                rounded
            } else {
                factor
            };
            Self::new(format!("{}:1", format_scale_number(p)), p, 1.0)
        }
    }
}

/// Formats a positive scale number cleanly (no trailing `.0`).
fn format_scale_number(v: f64) -> String {
    if (v - v.round()).abs() <= 1e-9 * v.abs().max(1.0) && v.abs() < 1e12 {
        format!("{}", v.round() as i64)
    } else {
        let s = format!("{v:.6}");
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

/// Parses a scale ratio from either colon notation (`"1:50"`, `"2:1"`,
/// `"1:2.5"`) or imperial architectural/engineering notation
/// (`"1/4\" = 1'-0\""`, `"1\" = 20'"`, `"3/32\" = 1'-0\""`).
pub fn parse_scale_ratio(text: &str) -> Option<(f64, f64)> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some((left, right)) = trimmed.split_once(':') {
        let paper = parse_positive_decimal(left.trim())?;
        let drawing = parse_positive_decimal(right.trim())?;
        return Some((paper, drawing));
    }
    if let Some((left, right)) = trimmed.split_once('=') {
        let paper = parse_imperial_length(left.trim())?;
        let drawing = parse_imperial_length(right.trim())?;
        if paper > 0.0 && drawing > 0.0 && paper.is_finite() && drawing.is_finite() {
            return Some((paper, drawing));
        }
    }
    None
}

fn parse_positive_decimal(s: &str) -> Option<f64> {
    if s.is_empty() {
        return None;
    }
    let v: f64 = s.parse().ok()?;
    (v.is_finite() && v > 0.0).then_some(v)
}

/// Parses an inch/foot expression such as `1/4"`, `1'-0"`, `1'`, `20'`, `1.5"`,
/// returning its length in inches.
fn parse_imperial_length(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Some((feet_str, rest)) = s.split_once('\'') {
        let feet = parse_number_or_fraction(feet_str.trim())?;
        let rest = rest.trim().trim_start_matches('-').trim();
        let inches = if rest.is_empty() {
            0.0
        } else {
            let inch_str = rest.strip_suffix('"').unwrap_or(rest).trim();
            parse_number_or_fraction(inch_str)?
        };
        let total = feet * 12.0 + inches;
        return (total.is_finite() && total > 0.0).then_some(total);
    }
    let inch_str = s.strip_suffix('"').unwrap_or(s).trim();
    let inches = parse_number_or_fraction(inch_str)?;
    (inches.is_finite() && inches > 0.0).then_some(inches)
}

fn parse_number_or_fraction(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Some((whole, frac)) = s.split_once(' ') {
        let w: f64 = whole.trim().parse().ok()?;
        let f = parse_simple_fraction(frac.trim())?;
        return (w >= 0.0 && f >= 0.0).then_some(w + f);
    }
    if s.contains('/') {
        return parse_simple_fraction(s);
    }
    let v: f64 = s.parse().ok()?;
    (v.is_finite() && v >= 0.0).then_some(v)
}

fn parse_simple_fraction(s: &str) -> Option<f64> {
    let (num, den) = s.split_once('/')?;
    let n: f64 = num.trim().parse().ok()?;
    let d: f64 = den.trim().parse().ok()?;
    (n.is_finite() && d.is_finite() && n >= 0.0 && d > 0.0).then_some(n / d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_and_imperial_ratios() {
        assert_eq!(parse_scale_ratio("1:1"), Some((1.0, 1.0)));
        assert_eq!(parse_scale_ratio("1:50"), Some((1.0, 50.0)));
        assert_eq!(parse_scale_ratio("2:1"), Some((2.0, 1.0)));
        assert_eq!(parse_scale_ratio("1/4\" = 1'-0\""), Some((0.25, 12.0)));
        assert_eq!(parse_scale_ratio("1\" = 20'"), Some((1.0, 240.0)));
        assert_eq!(parse_scale_ratio("1:0"), None);
        assert_eq!(parse_scale_ratio("bad"), None);
    }

    #[test]
    fn scale_factor_and_matching() {
        let s = AnnotationScale::from_name("1:50").unwrap();
        assert!((s.factor() - 0.02).abs() < 1e-12);
        assert!((s.inverse_factor() - 50.0).abs() < 1e-12);
        assert!(s.matches_factor(0.02));
        assert!(!s.matches_factor(0.01));
        assert_eq!(AnnotationScale::from_factor(0.02).name, "1:50");
        assert_eq!(AnnotationScale::from_factor(2.0).name, "2:1");
        assert_eq!(AnnotationScale::from_factor(1.0).name, "1:1");
    }
}
