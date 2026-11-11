//! Measuring a line of text: how wide it draws in a font, at a height and
//! a width factor.
//!
//! The measurement is of what the text draws, as AutoCAD's own measure of
//! a text object (AutoLISP's `textbox`) is: from the left edge of the first
//! stroke or outline to the right edge of the last, and from the lowest
//! descender to the highest point. A space at either end draws nothing, so
//! it adds nothing to the width, though it moves the pen. Lengths are in
//! the unit the height is in, along the text's own baseline from where the
//! line starts, before any rotation or justification.

use serde::{Deserialize, Serialize};

use super::fonts::{self, FontUsed, Rect};
use super::{FontKind, WIDTH_FACTORS};
use crate::error::{Error, ErrorCode};
use crate::settings::Settings;

fn one() -> f64 {
    1.0
}

/// A request to measure one line of text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureTextRequest {
    /// The text, one line.
    pub contents: String,
    /// The font's file name: an SHX font such as `romans.shx` or a
    /// TrueType font such as `arial.ttf`.
    pub font: String,
    /// The height of the capitals; the measurement is in the same unit.
    pub height: f64,
    /// How wide characters are drawn, as a share of their normal width.
    #[serde(default = "one")]
    pub width_factor: f64,
}

/// How a line of text measures.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TextMeasurement {
    /// From the left edge of what the text draws to the right edge: `right`
    /// less `left`. 0 for text that draws nothing, such as spaces.
    pub width: f64,
    /// Where what the text draws starts, along the baseline from the start
    /// of the line.
    pub left: f64,
    /// Where what the text draws ends, along the baseline from the start of
    /// the line.
    pub right: f64,
    /// How far below the baseline the text reaches, as a negative number
    /// when it has descenders.
    pub bottom: f64,
    /// How far above the baseline the text reaches.
    pub top: f64,
    /// How far the pen moves along the baseline: where text that followed
    /// would start.
    pub advance: f64,
    /// The font measured with: the one asked for, or its look-alike.
    pub font: FontUsed,
}

/// Measures a line of text, reading fonts from the font folders the
/// environment names ([`Settings::from_env`]).
pub fn measure_text(request: &MeasureTextRequest) -> Result<TextMeasurement, Error> {
    measure_text_with(request, &Settings::from_env())
}

/// Measures a line of text, reading fonts from the font folders `settings` names.
pub fn measure_text_with(
    request: &MeasureTextRequest,
    settings: &Settings,
) -> Result<TextMeasurement, Error> {
    check(request)?;
    let (font, used) = fonts::find(&request.font, settings);
    let line = font.line(&request.contents);
    let along = request.height * request.width_factor;
    let up = request.height;
    let ink = line.ink.unwrap_or(Rect::point(0.0, 0.0));
    let (left, right) = (ink.left * along, ink.right * along);
    Ok(TextMeasurement {
        width: right - left,
        left,
        right,
        bottom: ink.bottom * up,
        top: ink.top * up,
        advance: line.advance * along,
        font: used,
    })
}

fn check(request: &MeasureTextRequest) -> Result<(), Error> {
    let (narrowest, widest) = WIDTH_FACTORS;
    let problem = if FontKind::of(&request.font).is_none() {
        Some((
            "font",
            format!(
                "`{}` is not a font file name; a font is an SHX font such as `romans.shx` or a TrueType font such as `arial.ttf`",
                request.font
            ),
        ))
    } else if !(request.height.is_finite() && request.height > 0.0) {
        Some(("height", "a height is more than 0".to_owned()))
    } else if !(narrowest..=widest).contains(&request.width_factor) {
        Some((
            "width_factor",
            format!("a width factor is from {narrowest} to {widest}"),
        ))
    } else if request.contents.contains(['\n', '\r']) {
        Some((
            "contents",
            "a measurement is of one line; measure each line on its own".to_owned(),
        ))
    } else {
        None
    };
    match problem {
        Some((field, message)) => Err(Error::new(ErrorCode::BadValue, message).at_field(field)),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;
    use crate::text::hershey;
    use crate::text::shx::tests::unifont;
    use crate::text::truetype::tests::made_up_font;

    fn folder(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("agentdrafter-measure-{}", std::process::id()))
            .join(test);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn request(contents: &str, font: &str, height: f64, width_factor: f64) -> MeasureTextRequest {
        MeasureTextRequest {
            contents: contents.to_owned(),
            font: font.to_owned(),
            height,
            width_factor,
        }
    }

    #[track_caller]
    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} and {b}");
    }

    #[test]
    fn an_shx_font_measures_from_the_first_stroke_to_the_last() {
        let dir = folder("shx");
        // `I`: a stroke at 2 of an advance of 4; space: an advance of 5.
        let i: &[u8] = &[2, 8, 2, 0, 1, 0xA4, 2, 8, 2, 0xF6, 0];
        let space: &[u8] = &[2, 8, 5, 0, 0];
        fs::write(dir.join("madeup.shx"), unifont(10, &[(73, i), (32, space)])).unwrap();
        let settings = Settings {
            font_folders: vec![dir.clone()],
        };
        let m = measure_text_with(&request(" II ", "madeup.shx", 2.5, 1.0), &settings).unwrap();
        // Strokes at 5 + 2 and 5 + 4 + 2 units; capitals are 10 units tall.
        close(m.left, 0.25 * 7.0);
        close(m.right, 0.25 * 11.0);
        close(m.width, 0.25 * 4.0);
        close(m.top, 2.5);
        close(m.bottom, 0.0);
        close(m.advance, 0.25 * 18.0);
        assert!(!m.font.substituted);
        // A width factor stretches along the baseline only.
        let narrow =
            measure_text_with(&request(" II ", "madeup.shx", 2.5, 0.5), &settings).unwrap();
        close(narrow.width, m.width * 0.5);
        close(narrow.top, m.top);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_truetype_font_measures_its_outlines() {
        let dir = folder("ttf");
        fs::write(
            dir.join("madeup.ttf"),
            made_up_font(&[
                ('A', 700, [0, 0, 700, 640]),
                ('H', 800, [100, 0, 700, 640]),
                ('g', 600, [50, -160, 550, 480]),
            ]),
        )
        .unwrap();
        let settings = Settings {
            font_folders: vec![dir.clone()],
        };
        let m = measure_text_with(&request("Hg", "madeup.ttf", 6.4, 1.0), &settings).unwrap();
        close(m.left, 1.0);
        close(m.right, 8.0 + 5.5);
        close(m.bottom, -1.6);
        close(m.top, 6.4);
        close(m.advance, 14.0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_font_is_measured_with_the_look_alike_and_says_so() {
        let m = measure_text_with(
            &request("HI", "romans.shx", 21.0, 1.0),
            &Settings::default(),
        )
        .unwrap();
        // In Hershey's Roman Simplex, H strokes from 4 to 18 of an advance of
        // 22, and I strokes at 4.
        close(m.left, 4.0);
        close(m.right, 22.0 + 4.0);
        assert_eq!(m.font.used, hershey::NAME);
        assert!(m.font.substituted);
    }

    #[test]
    fn text_that_draws_nothing_has_no_width() {
        let m = measure_text_with(
            &request("   ", "romans.shx", 1.0, 1.0),
            &Settings::default(),
        )
        .unwrap();
        assert_eq!((m.width, m.left, m.right), (0.0, 0.0, 0.0));
        assert!(m.advance > 0.0);
    }

    #[test]
    fn a_request_that_cannot_be_measured_names_its_field() {
        let settings = Settings::default();
        for (bad, field) in [
            (request("A", "romans.fon", 1.0, 1.0), "font"),
            (request("A", "romans.shx", 0.0, 1.0), "height"),
            (request("A", "romans.shx", f64::NAN, 1.0), "height"),
            (request("A", "romans.shx", 1.0, 0.001), "width_factor"),
            (request("A\nB", "romans.shx", 1.0, 1.0), "contents"),
        ] {
            let error = measure_text_with(&bad, &settings).unwrap_err();
            assert_eq!(error.code, ErrorCode::BadValue);
            assert_eq!(error.field.as_deref(), Some(field), "{error:?}");
        }
    }
}
