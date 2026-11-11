//! Where justified text starts.
//!
//! A DWG records two points for a single line of text that is not
//! left-justified: the point it is justified at, and where its baseline
//! starts. AutoCAD draws the text from the start and works the start out
//! again only when the text is edited, so a build works it out as AutoCAD
//! does, from the font's widths. A font that is not in the font folders is
//! replaced by its look-alike here too, so its text may start a little
//! away from where AutoCAD would put it; the build's result names every
//! such font.
//!
//! AutoCAD's rules, read from AutoCAD 2026's placing of every
//! justification in two SHX and two TrueType fonts:
//!
//! - Text is centred or right-aligned on what it draws, counting the place
//!   of a character that draws nothing, such as a space at either end. For
//!   a TrueType font the whole advance counts as well.
//! - The top is the height of the capitals, the middle half that, and the
//!   bottom the font's descent below the baseline. `middle` centres the
//!   text on what it draws, from its lowest point to its highest.

use super::fonts::{find, Font, FontSubstitution, FontUsed};
use super::{Justify, Text, TextStyle, STANDARD};
use crate::model::{Object, Point3, Sheet};
use crate::settings::Settings;

/// Where a justification puts text across its baseline.
enum Across {
    Left,
    Center,
    Right,
}

/// Where a justification puts text up and down.
enum Up {
    Baseline,
    Bottom,
    Middle,
    Top,
    /// The middle of what the text draws: `middle`.
    Drawn,
}

fn parts(justify: Justify) -> (Across, Up) {
    match justify {
        Justify::Left => (Across::Left, Up::Baseline),
        Justify::Center => (Across::Center, Up::Baseline),
        Justify::Right => (Across::Right, Up::Baseline),
        Justify::Middle => (Across::Center, Up::Drawn),
        Justify::TopLeft => (Across::Left, Up::Top),
        Justify::TopCenter => (Across::Center, Up::Top),
        Justify::TopRight => (Across::Right, Up::Top),
        Justify::MiddleLeft => (Across::Left, Up::Middle),
        Justify::MiddleCenter => (Across::Center, Up::Middle),
        Justify::MiddleRight => (Across::Right, Up::Middle),
        Justify::BottomLeft => (Across::Left, Up::Bottom),
        Justify::BottomCenter => (Across::Center, Up::Bottom),
        Justify::BottomRight => (Across::Right, Up::Bottom),
    }
}

/// The fonts of a sheet's text styles, each found or replaced once.
pub(crate) struct SheetFonts {
    styles: Vec<(TextStyle, Font, FontUsed)>,
}

impl SheetFonts {
    /// Finds the font of every style the sheet defines, and of `Standard`
    /// when text draws with it.
    pub(crate) fn find(sheet: &Sheet, settings: &Settings) -> Self {
        let mut styles: Vec<TextStyle> = sheet.text_styles.clone();
        let standard_used = sheet.objects.iter().any(|object| match object {
            Object::Text(text) => text.style == STANDARD,
            Object::MText(text) => text.style == STANDARD,
            _ => false,
        });
        if standard_used && !styles.iter().any(|s| s.name == STANDARD) {
            styles.insert(0, TextStyle::standard());
        }
        let styles = styles
            .into_iter()
            .map(|style| {
                let (font, used) = find(&style.font, settings);
                (style, font, used)
            })
            .collect();
        Self { styles }
    }

    /// The styles whose fonts were replaced, and by what.
    pub(crate) fn substitutions(&self) -> Vec<FontSubstitution> {
        self.styles
            .iter()
            .filter(|(_, _, used)| used.substituted)
            .map(|(style, _, used)| FontSubstitution {
                style: style.name.clone(),
                font: style.font.clone(),
                used: used.used.clone(),
                why: used.why.clone().unwrap_or_default(),
            })
            .collect()
    }

    /// Where `text`'s baseline starts: its position when it is
    /// left-justified, and otherwise where AutoCAD would start it.
    pub(crate) fn start(&self, text: &Text) -> Point3 {
        if text.justify == Justify::Left {
            return text.position;
        }
        let font = self
            .styles
            .iter()
            .find(|(style, _, _)| style.name == text.style)
            .map_or(&Font::Hershey, |(_, font, _)| font);
        let (u, v) = offset(font, text);
        let (sin, cos) = text.rotation.to_radians().sin_cos();
        Point3::new(
            text.position.x + u * cos - v * sin,
            text.position.y + u * sin + v * cos,
            text.position.z,
        )
    }
}

/// From where text is justified to where its baseline starts, along and
/// up its own baseline, before rotation.
fn offset(font: &Font, text: &Text) -> (f64, f64) {
    let line = font.line(&text.contents);
    let Some(extent) = line.extent else {
        return (0.0, 0.0);
    };
    let along = text.height * text.width_factor;
    let (across, up) = parts(text.justify);
    let u = match across {
        Across::Left => 0.0,
        Across::Center => -(extent.left + extent.right) / 2.0 * along,
        Across::Right => -extent.right * along,
    };
    let v = match up {
        Up::Baseline => 0.0,
        Up::Bottom => font.descent() * text.height,
        Up::Middle => -text.height / 2.0,
        Up::Top => -text.height,
        Up::Drawn => -(extent.bottom + extent.top) / 2.0 * text.height,
    };
    (u, v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::hershey;

    fn text(contents: &str, justify: Justify, rotation: f64) -> Text {
        Text {
            contents: contents.to_owned(),
            position: Point3::new(100.0, 50.0, 3.0),
            height: 21.0,
            style: STANDARD.to_owned(),
            width_factor: 0.5,
            rotation,
            justify,
            layer: "0".to_owned(),
        }
    }

    fn fonts() -> SheetFonts {
        SheetFonts {
            styles: vec![(TextStyle::standard(), Font::Hershey, {
                FontUsed {
                    requested: "txt.shx".into(),
                    used: hershey::NAME.into(),
                    substituted: true,
                    why: None,
                }
            })],
        }
    }

    #[track_caller]
    fn close(p: Point3, x: f64, y: f64) {
        assert!(
            (p.x - x).abs() < 1e-9 && (p.y - y).abs() < 1e-9 && p.z == 3.0,
            "{p:?} is not ({x}, {y})"
        );
    }

    #[test]
    fn justified_text_starts_where_autocad_would_start_it() {
        let fonts = fonts();
        // In Hershey's Roman Simplex at 21 high and half width, `HI` draws
        // from 4 to 26 units across (2 to 13 here) and from 0 to 21 up.
        close(fonts.start(&text("HI", Justify::Left, 0.0)), 100.0, 50.0);
        close(fonts.start(&text("HI", Justify::Right, 0.0)), 87.0, 50.0);
        close(fonts.start(&text("HI", Justify::Center, 0.0)), 92.5, 50.0);
        close(fonts.start(&text("HI", Justify::TopLeft, 0.0)), 100.0, 29.0);
        close(
            fonts.start(&text("HI", Justify::MiddleRight, 0.0)),
            87.0,
            39.5,
        );
        close(
            fonts.start(&text("HI", Justify::BottomCenter, 0.0)),
            92.5,
            57.0,
        );
        close(fonts.start(&text("HI", Justify::Middle, 0.0)), 92.5, 39.5);
        // Turned a quarter, the offsets turn with it.
        close(fonts.start(&text("HI", Justify::Right, 90.0)), 100.0, 37.0);
    }

    #[test]
    fn a_space_at_the_end_counts_when_justifying() {
        let fonts = fonts();
        // The space after I runs from 30 to 46 units, to 23 here at half width.
        close(fonts.start(&text("HI ", Justify::Right, 0.0)), 77.0, 50.0);
    }
}
