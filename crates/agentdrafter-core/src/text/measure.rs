//! Measuring a line of text: how wide it draws in a font, at a height and
//! a width factor.
//!
//! The measurement is of what the text draws, as AutoCAD's own measure of
//! a text object (AutoLISP's `textbox`) is: from the left edge of the first
//! stroke or outline to the right edge of the last, and from the lowest
//! descender to the highest point (for a TrueType font whose glyphs sit wholly
//! above the baseline, shifted down to start at the baseline with the same
//! thickness, #59). A space at either end draws nothing, so it adds nothing to
//! the width, though it moves the pen. Lengths are in the unit the height is
//! in, along the text's own baseline from where the line starts, before any
//! rotation or justification.

use serde::{Deserialize, Serialize};

use super::fonts::{self, FontUsed, Rect};
use super::{font_name_problem, WIDTH_FACTORS};
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
    let textbox = line.textbox.unwrap_or(Rect::point(0.0, 0.0));
    let (left, right) = (textbox.left * along, textbox.right * along);
    Ok(TextMeasurement {
        width: right - left,
        left,
        right,
        bottom: textbox.bottom * up,
        top: textbox.top * up,
        advance: line.advance * along,
        font: used,
    })
}

fn check(request: &MeasureTextRequest) -> Result<(), Error> {
    let (narrowest, widest) = WIDTH_FACTORS;
    let problem = if let Some(problem) = font_name_problem(&request.font) {
        Some(("font", problem))
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
            ..Settings::default()
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
            ..Settings::default()
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
    fn truetype_textbox_starts_at_the_baseline_when_drawn_wholly_above_it() {
        let dir = folder("ttf-above-baseline");
        // Made-up TrueType font with elevated `-` and `+` glyphs (above the
        // baseline) alongside baseline-touching `A`, `x`, and descending `C` (#59).
        fs::write(
            dir.join("madeup.ttf"),
            made_up_font(&[
                ('A', 1366, [0, 0, 1369, 1466]),
                ('C', 1479, [102, -25, 1398, 1491]),
                ('x', 1024, [15, 0, 1009, 1062]),
                ('-', 682, [65, 440, 618, 621]),
                ('+', 1196, [114, 237, 1082, 1206]),
            ]),
        )
        .unwrap();
        // Made-up SHX font with an elevated `-` stroke at y = 3..4 (cap height 10).
        // pen up (2), move (8, 1, 3), pen down (1), draw (8, 4, 0), (8, 0, 1),
        // pen up (2), move (8, 1, -4), end (0).
        let shx_dash: &[u8] = &[2, 8, 1, 3, 1, 8, 4, 0, 8, 0, 1, 2, 8, 1, 0xFC, 0];
        fs::write(dir.join("madeup.shx"), unifont(10, &[(45, shx_dash)])).unwrap();
        let settings = Settings {
            font_folders: vec![dir.clone()],
            ..Settings::default()
        };
        // TrueType `-` draws at 440..621 over cap 1466: AutoCAD's `textbox`
        // shifts the box down to the baseline (0..181/1466 ≈ 0.1235).
        let dash = measure_text_with(&request("-", "madeup.ttf", 1.0, 1.0), &settings).unwrap();
        close(dash.bottom, 0.0);
        close(dash.top, (621.0 - 440.0) / 1466.0);
        close(dash.left, 65.0 / 1466.0);
        close(dash.right, 618.0 / 1466.0);
        // TrueType `+` draws at 237..1206: shifts down to 0..969/1466.
        let plus = measure_text_with(&request("+", "madeup.ttf", 1.0, 1.0), &settings).unwrap();
        close(plus.bottom, 0.0);
        close(plus.top, (1206.0 - 237.0) / 1466.0);
        // Text reaching or crossing the baseline keeps its drawn vertical box.
        let x = measure_text_with(&request("x", "madeup.ttf", 1.0, 1.0), &settings).unwrap();
        close(x.bottom, 0.0);
        close(x.top, 1062.0 / 1466.0);
        let ac = measure_text_with(&request("AC", "madeup.ttf", 1.0, 1.0), &settings).unwrap();
        close(ac.bottom, -25.0 / 1466.0);
        close(ac.top, 1491.0 / 1466.0);
        let a_dash = measure_text_with(&request("A-", "madeup.ttf", 1.0, 1.0), &settings).unwrap();
        close(a_dash.bottom, 0.0);
        close(a_dash.top, 1.0);
        // SHX `-` stays boxed where it draws (0.3..0.4), without shifting down.
        let shx = measure_text_with(&request("-", "madeup.shx", 1.0, 1.0), &settings).unwrap();
        close(shx.bottom, 0.3);
        close(shx.top, 0.4);
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
    fn a_font_outside_the_font_folders_is_never_read() {
        let dir = folder("outside");
        let fonts = dir.join("fonts");
        fs::create_dir_all(&fonts).unwrap();
        let i: &[u8] = &[2, 8, 2, 0, 1, 0xA4, 2, 8, 2, 0xF6, 0];
        fs::write(dir.join("outside.shx"), unifont(10, &[(73, i)])).unwrap();
        let settings = Settings {
            font_folders: vec![fonts],
            ..Settings::default()
        };
        let absolute = dir.join("outside.shx").to_string_lossy().into_owned();
        for name in [
            absolute.as_str(),
            "../outside.shx",
            "..\\outside.shx",
            "C:outside.shx",
        ] {
            let error = measure_text_with(&request("I", name, 1.0, 1.0), &settings)
                .expect_err(&format!("`{name}` is outside the font folders"));
            assert_eq!(error.code, ErrorCode::BadValue, "{error:?}");
            assert_eq!(error.field.as_deref(), Some("font"), "{error:?}");
            assert!(error.message.contains("file name alone"), "{error:?}");
        }
        let _ = fs::remove_dir_all(&dir);
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
