//! A dimension's text with a stacked fraction, laid out as AutoCAD 2026
//! lays out its MTEXT `{\H1x;\S1/2;}`, measured from its own dimensions'
//! text exploded into single lines, in `txt.shx`, at the height `h`:
//!
//! - Each piece is a line of text as high as the rest, the fraction's two
//!   numbers too. The text runs on from piece to piece, each starting where
//!   the one before leaves the pen.
//! - A fraction over a bar: the bar as long as the wider number, each
//!   number in the middle of it, the numerator's baseline `h/6` above the
//!   bar and the denominator's `7h/6` below; after it the pen moves on as
//!   far as the wider number moves it. The box is `7h/3` high, the bar
//!   across its middle.
//! - A fraction across a slash: the numerator from the pen, its baseline on
//!   the box's middle, and the denominator where the numerator leaves the
//!   pen, its baseline `h` lower; between them, in the middle of the gap, a
//!   slash `2h` long at 55°. The box is `2h` high.
//! - The text that is not a fraction sits on the box's bottom, or in its
//!   middle when the text is aligned at its middle, `\A1;`, as AutoCAD
//!   aligns Standard's.
//! - The box reaches from the first piece's ink to the last's.

use super::TextPiece;
use crate::model::MTextBox;

/// The angle of a diagonal fraction's slash, in degrees.
const SLASH_ANGLE: f64 = 55.0;

/// Stacked text laid out in its box's frame: along its baseline from the
/// box's middle, and up.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Stacked {
    pub width: f64,
    pub height: f64,
    /// Each line of text and where its baseline starts.
    pub texts: Vec<(String, [f64; 2])>,
    /// Each bar and slash.
    pub lines: Vec<[[f64; 2]; 2]>,
}

/// Lays out `pieces` at the height `h`, each piece's ink and advance
/// measured by `measure`, aligned at the middle when `middle` says.
pub(super) fn stacked(
    pieces: &[TextPiece],
    h: f64,
    middle: bool,
    measure: &dyn Fn(&str) -> MTextBox,
) -> Stacked {
    let diagonal = pieces
        .iter()
        .any(|p| matches!(p, TextPiece::Fraction { diagonal: true, .. }));
    let height = if diagonal { 2.0 * h } else { 7.0 * h / 3.0 };
    let plain_baseline = if middle {
        -h / 2.0
    } else if diagonal {
        -h
    } else {
        -7.0 * h / 6.0
    };
    let mut pen = 0.0;
    let mut ink: Option<(f64, f64)> = None;
    let mut grow = |left: f64, right: f64| {
        ink = Some(ink.map_or((left, right), |(l, r): (f64, f64)| {
            (l.min(left), r.max(right))
        }));
    };
    let mut texts = Vec::new();
    let mut lines = Vec::new();
    for piece in pieces {
        match piece {
            TextPiece::Plain(s) => {
                let m = measure(s);
                grow(pen + m.left, pen + m.left + m.width);
                texts.push((s.clone(), [pen, plain_baseline]));
                pen += m.advance;
            }
            TextPiece::Fraction {
                numerator,
                denominator,
                diagonal: false,
            } => {
                let (n, d) = (measure(numerator), measure(denominator));
                let column = n.width.max(d.width);
                texts.push((
                    numerator.clone(),
                    [pen + (column - n.width) / 2.0 - n.left, h / 6.0],
                ));
                texts.push((
                    denominator.clone(),
                    [pen + (column - d.width) / 2.0 - d.left, -7.0 * h / 6.0],
                ));
                lines.push([[pen, 0.0], [pen + column, 0.0]]);
                grow(pen, pen + column);
                pen += n.advance.max(d.advance);
            }
            TextPiece::Fraction {
                numerator,
                denominator,
                diagonal: true,
            } => {
                let (n, d) = (measure(numerator), measure(denominator));
                texts.push((numerator.clone(), [pen, 0.0]));
                let numerator_end = pen + n.left + n.width;
                grow(pen + n.left, numerator_end);
                pen += n.advance;
                texts.push((denominator.clone(), [pen, -h]));
                let denominator_start = pen + d.left;
                grow(denominator_start, denominator_start + d.width);
                let middle = (numerator_end + denominator_start) / 2.0;
                let (s, c) = SLASH_ANGLE.to_radians().sin_cos();
                lines.push([[middle - c * h, -s * h], [middle + c * h, s * h]]);
                pen += d.advance;
            }
        }
    }
    let (left, right) = ink.unwrap_or((0.0, 0.0));
    let shift = -(left + right) / 2.0;
    for (_, at) in &mut texts {
        at[0] += shift;
    }
    for line in &mut lines {
        for end in line {
            end[0] += shift;
        }
    }
    Stacked {
        width: right - left,
        height,
        texts,
        lines,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `txt.shx`'s digits at the height 2.5: an advance of 2.5 and ink
    /// 5/3 wide, the `1` narrower, the inch mark narrower still.
    fn txt(s: &str) -> MTextBox {
        let (advance, width) = match s {
            "1" => (1.6667, 0.8333),
            "\"" => (1.6667, 0.8333),
            "1'-0" => (7.5, 6.6667),
            "13" => (4.1667, 3.3333),
            "11" => (3.3333, 2.5),
            "16" => (4.1667, 3.3333),
            _ => (2.5, 1.6667),
        };
        MTextBox {
            width,
            height: 2.5,
            last_descent: 0.0,
            left: 0.0,
            advance,
        }
    }

    fn close(a: [f64; 2], b: [f64; 2]) {
        assert!(
            (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3,
            "{a:?} is not {b:?}"
        );
    }

    /// AutoCAD 2026's `1'-0{\H1x;\S1/2;}"` in ISO-25, exploded: its box
    /// 10.8333 wide and 5.8333 high round (6.25, 1188.5417).
    #[test]
    fn a_fraction_over_a_bar_is_laid_out_as_autocad_laid_out_its_own() {
        let pieces = [
            TextPiece::Plain("1'-0".into()),
            TextPiece::Fraction {
                numerator: "1".into(),
                denominator: "2".into(),
                diagonal: false,
            },
            TextPiece::Plain("\"".into()),
        ];
        let laid = stacked(&pieces, 2.5, false, &txt);
        assert!((laid.width - 10.8333).abs() < 1e-3);
        assert!((laid.height - 5.8333).abs() < 1e-3);
        let at = |x: f64, y: f64| [x - 6.25, y - 1188.5417];
        close(laid.texts[0].1, at(0.8333, 1185.625));
        close(laid.texts[1].1, at(8.75, 1188.9583));
        close(laid.texts[2].1, at(8.3333, 1185.625));
        close(laid.texts[3].1, at(10.8333, 1185.625));
        close(laid.lines[0][0], at(8.3333, 1188.5417));
        close(laid.lines[0][1], at(10.0, 1188.5417));
    }

    /// AutoCAD 2026's `13{\H1x;\S11#16;}` in ISO-25, exploded: its box
    /// 10.8333 wide and 5 high round (6.8438, 1338.125).
    #[test]
    fn a_fraction_across_a_slash_is_laid_out_as_autocad_laid_out_its_own() {
        let pieces = [
            TextPiece::Plain("13".into()),
            TextPiece::Fraction {
                numerator: "11".into(),
                denominator: "16".into(),
                diagonal: true,
            },
        ];
        let laid = stacked(&pieces, 2.5, false, &txt);
        assert!((laid.width - 10.8333).abs() < 1e-3);
        assert!((laid.height - 5.0).abs() < 1e-9);
        let at = |x: f64, y: f64| [x - 6.84375, y - 1338.125];
        close(laid.texts[0].1, at(1.4271, 1335.625));
        close(laid.texts[1].1, at(5.5938, 1338.125));
        close(laid.texts[2].1, at(8.9271, 1335.625));
        close(laid.lines[0][0], at(7.0765, 1336.0771));
        close(laid.lines[0][1], at(9.9444, 1340.1729));
    }

    /// In Standard, aligned at its middle, the text beside the fraction
    /// sits in the box's middle: AutoCAD's `\A1;1'-0{\H1x;\S1/2;}"` at 0.18.
    #[test]
    fn text_aligned_at_its_middle_sits_in_the_middle_of_the_box() {
        let pieces = [
            TextPiece::Plain("1'-0".into()),
            TextPiece::Fraction {
                numerator: "1".into(),
                denominator: "2".into(),
                diagonal: false,
            },
        ];
        let small = |s: &str| {
            let m = txt(s);
            MTextBox {
                width: m.width * 0.072,
                advance: m.advance * 0.072,
                ..m
            }
        };
        let laid = stacked(&pieces, 0.18, true, &small);
        assert!((laid.texts[0].1[1] + 0.09).abs() < 1e-9);
        assert!((laid.texts[1].1[1] - 0.03).abs() < 1e-9);
        assert!((laid.texts[2].1[1] + 0.21).abs() < 1e-9);
    }
}
