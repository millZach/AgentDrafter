//! Finding the font a text style names, or a look-alike when it is not there.
//!
//! Fonts are read from the font folders a setting names
//! ([`crate::Settings`]): on a machine with AutoCAD, its `Fonts` folder for
//! the SHX fonts and the system's fonts folder for TrueType ones. There is
//! no default folder. A font is found by its file name, ignoring case, as
//! Windows and AutoCAD do.
//!
//! A font that is not in any folder, or cannot be read, is replaced by a
//! look-alike, and whatever reports the measurement or the build says which
//! (a [`FontUsed`] or a [`FontSubstitution`]). A TrueType font with an open
//! twin of the same widths, such as Liberation Sans for Arial, takes the
//! twin when it is in a font folder. Anything else takes Hershey's Roman
//! Simplex, which is built in ([`super::hershey`]). The DWG still names the
//! font the sheet asked for: the substitution is only in what AgentDrafter
//! measures.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::shx::ShxFont;
use super::truetype::TrueTypeFont;
use super::{hershey, FontKind};
use crate::settings::Settings;

/// Open fonts with the same advance widths as common TrueType fonts, by
/// the file name of the font they stand in for. Each is taken only if it is
/// in a font folder; none is in this repository.
const METRIC_TWINS: [(&str, [&str; 2]); 6] = [
    (
        "arial.ttf",
        ["LiberationSans-Regular.ttf", "Arimo-Regular.ttf"],
    ),
    ("arialbd.ttf", ["LiberationSans-Bold.ttf", "Arimo-Bold.ttf"]),
    (
        "times.ttf",
        ["LiberationSerif-Regular.ttf", "Tinos-Regular.ttf"],
    ),
    (
        "timesbd.ttf",
        ["LiberationSerif-Bold.ttf", "Tinos-Bold.ttf"],
    ),
    (
        "cour.ttf",
        ["LiberationMono-Regular.ttf", "Cousine-Regular.ttf"],
    ),
    (
        "courbd.ttf",
        ["LiberationMono-Bold.ttf", "Cousine-Bold.ttf"],
    ),
];

/// A box in a glyph's or a text's own units, with Y up from the baseline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rect {
    pub left: f64,
    pub right: f64,
    pub bottom: f64,
    pub top: f64,
}

impl Rect {
    pub(crate) fn point(x: f64, y: f64) -> Self {
        Self {
            left: x,
            right: x,
            bottom: y,
            top: y,
        }
    }

    pub(crate) fn union(self, other: Rect) -> Self {
        Self {
            left: self.left.min(other.left),
            right: self.right.max(other.right),
            bottom: self.bottom.min(other.bottom),
            top: self.top.max(other.top),
        }
    }

    pub(crate) fn scaled(self, scale: f64) -> Self {
        Self {
            left: self.left * scale,
            right: self.right * scale,
            bottom: self.bottom * scale,
            top: self.top * scale,
        }
    }

    pub(crate) fn moved(self, dx: f64, dy: f64) -> Self {
        Self {
            left: self.left + dx,
            right: self.right + dx,
            bottom: self.bottom + dy,
            top: self.top + dy,
        }
    }
}

/// One character's metrics, in units where the font's capitals stand 1 tall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Glyph {
    /// Where the pen ends, from where it started: where the next character starts.
    pub advance: (f64, f64),
    /// The box around what the character draws; none for a space.
    pub ink: Option<Rect>,
}

/// The font a measurement used.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FontUsed {
    /// The font that was asked for, such as `romans.shx`.
    pub requested: String,
    /// The font that was used: the one asked for, or its look-alike.
    pub used: String,
    /// Whether a look-alike was used.
    pub substituted: bool,
    /// Why a look-alike was used; none when the font was found.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// A text style whose font was not found, and the look-alike AgentDrafter
/// measures its text with. The DWG still names the style's own font.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FontSubstitution {
    /// The text style.
    pub style: String,
    /// The font the style names.
    pub font: String,
    /// The look-alike used in its place.
    pub used: String,
    /// Why: the font is not in the font folders, or could not be read.
    pub why: String,
}

/// A font, read and ready to measure with.
#[derive(Debug)]
pub(crate) enum Font {
    Shx(ShxFont),
    TrueType(TrueTypeFont),
    Hershey,
}

impl Font {
    /// The glyphs for `text`, in units where the capitals stand 1 tall.
    pub(crate) fn glyphs(&self, text: &str) -> Vec<Glyph> {
        match self {
            Font::Shx(font) => text.chars().filter_map(|c| font.glyph(c)).collect(),
            Font::TrueType(font) => font.glyphs(text),
            Font::Hershey => text.chars().map(hershey::glyph).collect(),
        }
    }

    /// How far the font's descenders reach below the baseline, in units
    /// where the capitals stand 1 tall: what AutoCAD's bottom
    /// justifications lift text by.
    pub(crate) fn descent(&self) -> f64 {
        match self {
            Font::Shx(font) => font.descent(),
            Font::TrueType(font) => font.descent(),
            Font::Hershey => hershey::DESCENT,
        }
    }

    /// `text` laid out on one line from the origin, in units where the
    /// capitals stand 1 tall.
    pub(crate) fn line(&self, text: &str) -> Line {
        let mut pen = (0.0, 0.0);
        let mut ink: Option<Rect> = None;
        let mut extent: Option<Rect> = None;
        let grow = |all: &mut Option<Rect>, part: Rect| {
            *all = Some(all.map_or(part, |all| all.union(part)));
        };
        for glyph in self.glyphs(text) {
            let next = (pen.0 + glyph.advance.0, pen.1 + glyph.advance.1);
            match glyph.ink {
                Some(drawn) => {
                    let drawn = drawn.moved(pen.0, pen.1);
                    grow(&mut ink, drawn);
                    grow(&mut extent, drawn);
                }
                // A character that draws nothing, such as a space, still
                // takes its place on the baseline when AutoCAD justifies.
                None => grow(
                    &mut extent,
                    Rect::point(pen.0, 0.0).union(Rect::point(next.0, 0.0)),
                ),
            }
            pen = next;
        }
        if let (Font::TrueType(_), Some(all)) = (self, extent.as_mut()) {
            // AutoCAD justifies TrueType text by its advance as well as its outlines.
            *all = all
                .union(Rect::point(0.0, all.bottom))
                .union(Rect::point(pen.0, all.bottom));
        }
        Line {
            advance: pen.0,
            ink,
            extent,
        }
    }
}

/// A line of text laid out from the origin, in units where the capitals
/// stand 1 tall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Line {
    /// How far the pen moves along the baseline.
    pub advance: f64,
    /// The box around what the line draws, as AutoCAD's `textbox`
    /// measures it; none when it draws nothing.
    pub ink: Option<Rect>,
    /// The box AutoCAD justifies the line by: what it draws, the place of
    /// each character that draws nothing, and for a TrueType font the
    /// whole advance.
    pub extent: Option<Rect>,
}

/// The font a style names, found in the font folders, or its look-alike,
/// with a report of which was used.
pub(crate) fn find(name: &str, settings: &Settings) -> (Font, FontUsed) {
    let why = match read(name, &settings.font_folders) {
        Ok(font) => {
            let used = FontUsed {
                requested: name.to_owned(),
                used: name.to_owned(),
                substituted: false,
                why: None,
            };
            return (font, used);
        }
        Err(why) => why,
    };
    let twins = METRIC_TWINS
        .iter()
        .find(|(font, _)| font.eq_ignore_ascii_case(name))
        .map_or(&[][..], |(_, twins)| &twins[..]);
    for twin in twins {
        if let Ok(font) = read(twin, &settings.font_folders) {
            let used = FontUsed {
                requested: name.to_owned(),
                used: (*twin).to_owned(),
                substituted: true,
                why: Some(why),
            };
            return (font, used);
        }
    }
    let used = FontUsed {
        requested: name.to_owned(),
        used: hershey::NAME.to_owned(),
        substituted: true,
        why: Some(why),
    };
    (Font::Hershey, used)
}

/// Reads the font file `name` from the first folder that holds it. The
/// error says why it could not be used.
fn read(name: &str, folders: &[PathBuf]) -> Result<Font, String> {
    let kind = FontKind::of(name).ok_or_else(|| {
        format!("`{name}` is neither an SHX font (.shx) nor a TrueType font (.ttf)")
    })?;
    let Some(path) = folders.iter().find_map(|folder| file_in(folder, name)) else {
        return Err(if folders.is_empty() {
            "no font folder is set".to_owned()
        } else {
            "it is not in the font folders".to_owned()
        });
    };
    let bytes = fs::read(&path).map_err(|e| format!("it could not be read ({e})"))?;
    let font = match kind {
        FontKind::Shx => ShxFont::parse(&bytes).map(Font::Shx),
        FontKind::TrueType => TrueTypeFont::parse(bytes).map(Font::TrueType),
    };
    font.map_err(|e| format!("it could not be read: {e}"))
}

/// The file in `folder` named `name`, ignoring case.
fn file_in(folder: &Path, name: &str) -> Option<PathBuf> {
    let exact = folder.join(name);
    if exact.is_file() {
        return Some(exact);
    }
    fs::read_dir(folder)
        .ok()?
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::shx::tests::unifont;
    use crate::text::truetype::tests::made_up_font;

    fn folder(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("agentdrafter-fonts-{}", std::process::id()))
            .join(test);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn settings(folders: &[&Path]) -> Settings {
        Settings {
            font_folders: folders.iter().map(|f| f.to_path_buf()).collect(),
        }
    }

    #[test]
    fn a_font_in_a_folder_is_found_whatever_its_case() {
        let dir = folder("found");
        fs::write(
            dir.join("MadeUp.SHX"),
            unifont(10, &[(65, &[2, 8, 5, 0, 0])]),
        )
        .unwrap();
        let (font, used) = find("madeup.shx", &settings(&[&dir]));
        assert!(matches!(font, Font::Shx(_)), "{font:?}");
        assert_eq!(used.used, "madeup.shx");
        assert!(!used.substituted);
        assert_eq!(used.why, None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_shx_font_takes_the_built_in_look_alike() {
        let dir = folder("missing");
        let (font, used) = find("romans.shx", &settings(&[&dir]));
        assert!(matches!(font, Font::Hershey));
        assert_eq!(used.used, hershey::NAME);
        assert!(used.substituted);
        assert_eq!(used.why.as_deref(), Some("it is not in the font folders"));
        let (_, used) = find("romans.shx", &Settings::default());
        assert_eq!(used.why.as_deref(), Some("no font folder is set"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_truetype_font_takes_its_open_twin_when_there_is_one() {
        let dir = folder("twin");
        fs::write(
            dir.join("LiberationSans-Regular.ttf"),
            made_up_font(&[('A', 700, [0, 0, 600, 700])]),
        )
        .unwrap();
        let (font, used) = find("Arial.ttf", &settings(&[&dir]));
        assert!(matches!(font, Font::TrueType(_)));
        assert_eq!(used.used, "LiberationSans-Regular.ttf");
        assert!(used.substituted);
        let (font, used) = find("verdana.ttf", &settings(&[&dir]));
        assert!(matches!(font, Font::Hershey));
        assert_eq!(used.used, hershey::NAME);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_font_that_cannot_be_read_is_replaced_and_says_why() {
        let dir = folder("broken");
        fs::write(dir.join("broken.shx"), b"not a font").unwrap();
        let (font, used) = find("broken.shx", &settings(&[&dir]));
        assert!(matches!(font, Font::Hershey));
        assert!(
            used.why
                .as_deref()
                .unwrap()
                .starts_with("it could not be read"),
            "{used:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_first_folder_that_holds_a_font_wins() {
        let first = folder("first");
        let second = folder("second");
        fs::write(second.join("a.shx"), unifont(10, &[(65, &[2, 8, 5, 0, 0])])).unwrap();
        fs::write(first.join("a.shx"), unifont(10, &[(65, &[2, 8, 7, 0, 0])])).unwrap();
        let (font, _) = find("a.shx", &settings(&[&first, &second]));
        assert!((font.glyphs("A")[0].advance.0 - 0.7).abs() < 1e-15);
        let _ = fs::remove_dir_all(&first);
        let _ = fs::remove_dir_all(&second);
    }
}
