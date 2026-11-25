//! Finding the font a text style names, or a look-alike when it is not there.
//!
//! Fonts are read from the font folders a setting names
//! ([`crate::Settings`]), such as AutoCAD's `Fonts` folder on a machine with
//! AutoCAD. A TrueType font is then looked for in the system's own font
//! folders, and the folders inside them, unless the settings leave them
//! out; an SHX font never is. A font is found by its file name, ignoring
//! case, as Windows and AutoCAD do.
//!
//! A font that is not in any folder, or cannot be read, is replaced by a
//! look-alike, and whatever reports the measurement or the build says which
//! (a [`FontUsed`] or a [`FontSubstitution`]). A TrueType font with an open
//! twin of the same widths, such as Liberation Sans for Arial, takes the
//! twin when it is in a font folder. Anything else takes Hershey's Roman
//! Simplex, which is built in ([`super::hershey`]). The DWG still names the
//! font the sheet asked for: the substitution is only in what AgentDrafter
//! measures.

use std::collections::HashSet;
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
const METRIC_TWINS: [(&str, [&str; 2]); 12] = [
    (
        "arial.ttf",
        ["LiberationSans-Regular.ttf", "Arimo-Regular.ttf"],
    ),
    ("arialbd.ttf", ["LiberationSans-Bold.ttf", "Arimo-Bold.ttf"]),
    (
        "ariali.ttf",
        ["LiberationSans-Italic.ttf", "Arimo-Italic.ttf"],
    ),
    (
        "arialbi.ttf",
        ["LiberationSans-BoldItalic.ttf", "Arimo-BoldItalic.ttf"],
    ),
    (
        "times.ttf",
        ["LiberationSerif-Regular.ttf", "Tinos-Regular.ttf"],
    ),
    (
        "timesbd.ttf",
        ["LiberationSerif-Bold.ttf", "Tinos-Bold.ttf"],
    ),
    (
        "timesi.ttf",
        ["LiberationSerif-Italic.ttf", "Tinos-Italic.ttf"],
    ),
    (
        "timesbi.ttf",
        ["LiberationSerif-BoldItalic.ttf", "Tinos-BoldItalic.ttf"],
    ),
    (
        "cour.ttf",
        ["LiberationMono-Regular.ttf", "Cousine-Regular.ttf"],
    ),
    (
        "courbd.ttf",
        ["LiberationMono-Bold.ttf", "Cousine-Bold.ttf"],
    ),
    (
        "couri.ttf",
        ["LiberationMono-Italic.ttf", "Cousine-Italic.ttf"],
    ),
    (
        "courbi.ttf",
        ["LiberationMono-BoldItalic.ttf", "Cousine-BoldItalic.ttf"],
    ),
];

/// Well-known TrueType families mapped to `[regular, bold, italic, bold_italic]`
/// file names so MTEXT inline `\fArial|b1|i0;...` resolves to the right face (#61).
const FAMILY_FACES: [(&str, [&str; 4]); 7] = [
    (
        "arial",
        ["arial.ttf", "arialbd.ttf", "ariali.ttf", "arialbi.ttf"],
    ),
    (
        "helvetica",
        ["arial.ttf", "arialbd.ttf", "ariali.ttf", "arialbi.ttf"],
    ),
    (
        "times new roman",
        ["times.ttf", "timesbd.ttf", "timesi.ttf", "timesbi.ttf"],
    ),
    (
        "courier new",
        ["cour.ttf", "courbd.ttf", "couri.ttf", "courbi.ttf"],
    ),
    (
        "calibri",
        [
            "calibri.ttf",
            "calibrib.ttf",
            "calibrii.ttf",
            "calibriz.ttf",
        ],
    ),
    (
        "verdana",
        [
            "verdana.ttf",
            "verdanab.ttf",
            "verdanai.ttf",
            "verdanaz.ttf",
        ],
    ),
    (
        "tahoma",
        ["tahoma.ttf", "tahomabd.ttf", "tahoma.ttf", "tahomabd.ttf"],
    ),
];

/// Resolves an inline MTEXT font override (`name`, `bold`, `italic`, or the
/// style's `base_font` when `name` is `None`). Returns `(font, used, has_bold_face, has_italic_face)`
/// where `has_bold_face` / `has_italic_face` say whether the loaded font file
/// already supplies bold/italic outlines so the renderer knows whether to apply
/// synthetic bold/italic styling (#61).
pub(crate) fn find_inline(
    name: Option<&str>,
    base_font: &str,
    bold: bool,
    italic: bool,
    settings: &Settings,
) -> (Font, FontUsed, bool, bool) {
    let requested = name.unwrap_or(base_font).trim();
    let lower = requested.to_ascii_lowercase();
    let face_idx = usize::from(bold) + 2 * usize::from(italic);
    // Check known TrueType family names or base file names for bold/italic faces.
    let family = FAMILY_FACES
        .iter()
        .find(|(fam, faces)| *fam == lower || faces.iter().any(|f| f.eq_ignore_ascii_case(&lower)));
    if let Some((_, faces)) = family {
        let wanted = faces[face_idx];
        let (font, used) = find(wanted, settings);
        if matches!(font, Font::TrueType(_)) {
            let has_bold = bold && (faces[1] != faces[0]);
            let has_italic = italic && (faces[2] != faces[0]);
            return (font, used, has_bold, has_italic);
        }
        // If bold/italic face wasn't found, fall back to the regular face.
        if face_idx != 0 {
            let (reg_font, reg_used) = find(faces[0], settings);
            if matches!(reg_font, Font::TrueType(_)) {
                return (reg_font, reg_used, false, false);
            }
        }
        return (font, used, false, false);
    }
    // File name with `.ttf` or `.shx` extension.
    if FontKind::of(requested).is_some() {
        let (font, used) = find(requested, settings);
        return (font, used, false, false);
    }
    // Unrecognized family/SHX name without extension: try `<name>.ttf`, then `<name>.shx`.
    let ttf_name = format!("{requested}.ttf");
    if let Ok((font, source)) = read(&ttf_name, settings) {
        let used = FontUsed {
            requested: requested.to_owned(),
            used: ttf_name,
            substituted: false,
            why: None,
            source,
        };
        return (font, used, false, false);
    }
    let shx_name = format!("{requested}.shx");
    if let Ok((font, source)) = read(&shx_name, settings) {
        let used = FontUsed {
            requested: requested.to_owned(),
            used: shx_name,
            substituted: false,
            why: None,
            source,
        };
        return (font, used, false, false);
    }
    let (_, mut used) = find(&ttf_name, settings);
    used.requested = requested.to_owned();
    (Font::Hershey, used, false, false)
}

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

/// Where a font was found, when it was not in a folder the settings name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FontSource {
    /// In one of the system's own font folders.
    System,
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
    /// `system` when the font used was found in one of the system's font
    /// folders; left out when it was in a folder the settings name, or is
    /// the built-in look-alike.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<FontSource>,
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
    /// `system` when the look-alike was found in one of the system's font
    /// folders; left out otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<FontSource>,
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

    /// For a TrueType font, the underline `(position, thickness)` in units
    /// where capitals stand 1 tall, so underline and overline draw as filled
    /// bars scaled with the text. `None` for stroke fonts (SHX and Hershey),
    /// which draw underline and overline as strokes in the text's lineweight.
    pub(crate) fn decoration_bar(&self) -> Option<(f64, f64)> {
        match self {
            Font::TrueType(font) => Some(font.underline_metrics()),
            Font::Shx(_) | Font::Hershey => None,
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
        let textbox = match self {
            Font::TrueType(_) => ink.map(truetype_textbox),
            Font::Shx(_) | Font::Hershey => ink,
        };
        if let (Font::TrueType(_), Some(all)) = (self, extent.as_mut()) {
            // AutoCAD justifies TrueType text by its advance as well as its
            // outlines, and its vertical box (`textbox`) drops any gap between
            // the baseline and the bottom of what the text draws (#59).
            if let Some(tb) = textbox {
                all.bottom = tb.bottom;
                all.top = tb.top;
            }
            *all = all
                .union(Rect::point(0.0, all.bottom))
                .union(Rect::point(pen.0, all.bottom));
        }
        Line {
            advance: pen.0,
            ink,
            textbox,
            extent,
        }
    }
}

/// AutoCAD's `textbox` for TrueType text drops any vertical gap between the
/// baseline and the bottom of what the text draws: when the drawn box sits
/// wholly above the baseline (`bottom > 0`), it starts at the baseline with
/// the same thickness (`bottom = 0`, `top = rect.top - rect.bottom`). Text
/// that reaches or crosses the baseline keeps its drawn vertical bounds (#59).
pub(crate) fn truetype_textbox(rect: Rect) -> Rect {
    if rect.bottom > 0.0 {
        rect.moved(0.0, -rect.bottom)
    } else {
        rect
    }
}

/// A line of text laid out from the origin, in units where the capitals
/// stand 1 tall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Line {
    /// How far the pen moves along the baseline.
    pub advance: f64,
    /// The box around what the line draws; none when it draws nothing.
    pub ink: Option<Rect>,
    /// AutoCAD's `textbox` for the line: what it draws, shifted down to the
    /// baseline for a TrueType font when it draws wholly above it (#59);
    /// none when it draws nothing.
    pub textbox: Option<Rect>,
    /// The box AutoCAD justifies the line by: what it draws, the place of
    /// each character that draws nothing, and for a TrueType font the
    /// whole advance and `textbox`'s vertical bounds.
    pub extent: Option<Rect>,
}

/// Finds and parses an `.shx` shape file by name from `extra_dirs` (such as a
/// sheet's folder) followed by `settings.font_folders`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn load_shape_file(
    file: &str,
    settings: &Settings,
    extra_dirs: &[&Path],
) -> Option<super::shx::ShxShapeFile> {
    if file.trim().is_empty() {
        return None;
    }
    let direct = Path::new(file);
    let candidate_name = direct.file_name().and_then(|n| n.to_str()).unwrap_or(file);
    let with_ext = if candidate_name.to_ascii_lowercase().ends_with(".shx") {
        candidate_name.to_owned()
    } else {
        format!("{candidate_name}.shx")
    };
    let found = if direct.is_absolute() && direct.is_file() {
        Some(direct.to_path_buf())
    } else {
        extra_dirs
            .iter()
            .find_map(|dir| {
                let joined = dir.join(file);
                if joined.is_file() {
                    Some(joined)
                } else {
                    file_in(dir, &with_ext)
                }
            })
            .or_else(|| {
                settings
                    .font_folders
                    .iter()
                    .find_map(|folder| file_in(folder, &with_ext))
            })
    }?;
    let bytes = fs::read(&found).ok()?;
    super::shx::ShxShapeFile::parse(&bytes).ok()
}

/// Populates `sheet.resolved_shapes` for every [`crate::model::ShxShape`] on
/// `sheet` and inside its blocks (and fills missing shape `number`/`name` pairs
/// when resolved from the `.shx` file).
#[cfg(target_arch = "wasm32")]
pub(crate) fn resolve_sheet_shapes(
    _sheet: &mut crate::model::Sheet,
    _settings: &Settings,
    _extra_dirs: &[&Path],
) {
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn resolve_sheet_shapes(
    sheet: &mut crate::model::Sheet,
    settings: &Settings,
    extra_dirs: &[&Path],
) {
    use std::collections::HashMap;
    let mut cache: HashMap<String, Option<super::shx::ShxShapeFile>> = HashMap::new();
    let mut resolved = Vec::new();
    let mut seen: HashSet<(String, i32, String)> = HashSet::new();

    let mut visit = |shape: &crate::model::ShxShape| {
        let key = shape.file.to_lowercase();
        let loaded = cache
            .entry(key.clone())
            .or_insert_with(|| load_shape_file(&shape.file, settings, extra_dirs));
        if let Some(shx) = loaded.as_ref() {
            if let Some((num, name, strokes)) = shx.resolve(shape.number, &shape.name, 0.02) {
                let dedup = (key, num, name.to_lowercase());
                if seen.insert(dedup) {
                    resolved.push(crate::model::ResolvedShape {
                        file: shape.file.clone(),
                        name,
                        number: num,
                        strokes,
                    });
                }
            }
        }
    };

    for obj in &sheet.objects {
        if let crate::model::Object::Shape(s) = obj {
            visit(s);
        }
    }
    for block in sheet.blocks.iter().chain(&sheet.resolved_xrefs) {
        for obj in &block.objects {
            if let crate::model::Object::Shape(s) = obj {
                visit(s);
            }
        }
    }
    sheet.resolved_shapes = resolved;
}

/// The font a style names, found in the font folders, or its look-alike,
/// with a report of which was used.
pub(crate) fn find(name: &str, settings: &Settings) -> (Font, FontUsed) {
    let why = match read(name, settings) {
        Ok((font, source)) => {
            let used = FontUsed {
                requested: name.to_owned(),
                used: name.to_owned(),
                substituted: false,
                why: None,
                source,
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
        if let Ok((font, source)) = read(twin, settings) {
            let used = FontUsed {
                requested: name.to_owned(),
                used: (*twin).to_owned(),
                substituted: true,
                why: Some(why),
                source,
            };
            return (font, used);
        }
    }
    let used = FontUsed {
        requested: name.to_owned(),
        used: hershey::NAME.to_owned(),
        substituted: true,
        why: Some(why),
        source: None,
    };
    (Font::Hershey, used)
}

/// Reads the font file `name` from the first folder the settings name that
/// holds it, or for a TrueType font then from the first of the system's
/// font folders that holds it, and says when it was the system's. The
/// error says why it could not be used.
fn read(name: &str, settings: &Settings) -> Result<(Font, Option<FontSource>), String> {
    let kind = FontKind::of(name).ok_or_else(|| {
        format!("`{name}` is neither an SHX font (.shx) nor a TrueType font (.ttf)")
    })?;
    let named = &settings.font_folders;
    // AutoCAD's SHX fonts live only in its own folder, never the system's.
    let system: &[PathBuf] = match kind {
        FontKind::Shx => &[],
        FontKind::TrueType => &settings.system_font_folders,
    };
    let found = named
        .iter()
        .find_map(|folder| file_in(folder, name))
        .map(|path| (path, None))
        .or_else(|| {
            system
                .iter()
                .find_map(|folder| file_below(folder, name, &mut HashSet::new()))
                .map(|path| (path, Some(FontSource::System)))
        });
    let Some((path, source)) = found else {
        return Err(if named.is_empty() && system.is_empty() {
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
    font.map(|font| (font, source))
        .map_err(|e| format!("it could not be read: {e}"))
}

/// The file named `name`, ignoring case, in `folder` or the folders inside
/// it, however deep: the folder's own first, then each folder inside it in
/// the order of their names. Among the folder's files named alike, one with
/// the name's own case wins, then the first in the order of their names, so
/// that the same font wins whatever order the system lists a folder in. A
/// folder already searched, as through a link back to one above it, is not
/// searched again; `visited` holds each searched folder's real path.
fn file_below(folder: &Path, name: &str, visited: &mut HashSet<PathBuf>) -> Option<PathBuf> {
    let real = fs::canonicalize(folder).ok()?;
    if !visited.insert(real) {
        return None;
    }
    let mut alike: Vec<(std::ffi::OsString, PathBuf)> = Vec::new();
    let mut inside: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(folder).ok()?.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            inside.push(path);
        } else if path.is_file()
            && entry
                .file_name()
                .to_str()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        {
            alike.push((entry.file_name(), path));
        }
    }
    if let Some((_, path)) = alike
        .into_iter()
        .min_by(|(a, _), (b, _)| (a != name, a).cmp(&(b != name, b)))
    {
        return Some(path);
    }
    inside.sort();
    inside
        .iter()
        .find_map(|inner| file_below(inner, name, visited))
}

/// The file in `folder` named `name`, ignoring case.
pub(crate) fn file_in(folder: &Path, name: &str) -> Option<PathBuf> {
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
            ..Settings::default()
        }
    }

    fn with_system(folders: &[&Path], system: &[&Path]) -> Settings {
        Settings {
            system_font_folders: system.iter().map(|f| f.to_path_buf()).collect(),
            ..settings(folders)
        }
    }

    /// A made-up TrueType font whose `A` moves the pen `advance` units.
    fn truetype(advance: u16) -> Vec<u8> {
        made_up_font(&[('A', advance, [0, 0, 600, 700])])
    }

    fn advance(font: &Font) -> f64 {
        font.glyphs("A")[0].advance.0
    }

    /// The advance of `A` in the made-up font `truetype(units)` makes.
    fn advance_of(units: u16) -> f64 {
        advance(&Font::TrueType(
            TrueTypeFont::parse(truetype(units)).unwrap(),
        ))
    }

    #[test]
    fn a_truetype_font_in_no_folder_named_is_found_in_a_system_folder() {
        let named = folder("system-named");
        let system = folder("system-system");
        let inner = system.join("truetype").join("made-up");
        fs::create_dir_all(&inner).unwrap();
        fs::write(inner.join("MadeUp.TTF"), truetype(700)).unwrap();
        let (font, used) = find("madeup.ttf", &with_system(&[&named], &[&system]));
        assert!(matches!(font, Font::TrueType(_)), "{font:?}");
        assert!(!used.substituted);
        assert_eq!(used.source, Some(FontSource::System));
        assert_eq!(
            serde_json::to_value(&used).unwrap()["source"],
            serde_json::json!("system")
        );
        // Without the system's folders it takes a look-alike, as before.
        let (font, used) = find("madeup.ttf", &settings(&[&named]));
        assert!(matches!(font, Font::Hershey));
        assert_eq!(used.why.as_deref(), Some("it is not in the font folders"));
        assert!(serde_json::to_value(&used).unwrap().get("source").is_none());
        let _ = fs::remove_dir_all(&named);
        let _ = fs::remove_dir_all(&system);
    }

    #[test]
    fn the_folders_named_come_first_then_the_system_folders_in_order() {
        let named = folder("order-named");
        let first = folder("order-first");
        let second = folder("order-second");
        fs::write(first.join("a.ttf"), truetype(600)).unwrap();
        fs::write(second.join("a.ttf"), truetype(500)).unwrap();
        let all = with_system(&[&named], &[&first, &second]);
        let (font, used) = find("a.ttf", &all);
        assert_eq!(used.source, Some(FontSource::System));
        let (only_first, _) = find("a.ttf", &settings(&[&first]));
        let (only_second, _) = find("a.ttf", &settings(&[&second]));
        assert_eq!(advance(&font), advance(&only_first));
        assert_ne!(advance(&font), advance(&only_second));
        fs::write(named.join("A.ttf"), truetype(900)).unwrap();
        let (font, used) = find("a.ttf", &all);
        let (only_named, _) = find("a.ttf", &settings(&[&named]));
        assert_eq!(used.source, None);
        assert_eq!(advance(&font), advance(&only_named));
        assert_ne!(advance(&font), advance(&only_first));
        for dir in [&named, &first, &second] {
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn the_folders_inside_a_system_folder_are_searched_in_the_order_of_their_names() {
        let system = folder("inside");
        for (inner, width) in [("b", 500), ("a", 600)] {
            fs::create_dir_all(system.join(inner)).unwrap();
            fs::write(system.join(inner).join("x.ttf"), truetype(width)).unwrap();
        }
        let (font, _) = find("x.ttf", &with_system(&[], &[&system]));
        let (a, _) = find("x.ttf", &settings(&[&system.join("a")]));
        assert_eq!(advance(&font), advance(&a));
        let _ = fs::remove_dir_all(&system);
    }

    #[test]
    fn among_files_named_alike_in_a_system_folder_the_same_one_wins_whatever_order_they_were_made_in(
    ) {
        // Two made-up fonts whose names differ only in case, made in both
        // orders. A file system that ignores case holds only one of them,
        // so there is nothing to choose and the test has nothing to check.
        let first = folder("case-first");
        let second = folder("case-second");
        let (upper, mixed) = (("ADREVIEW.TTF", 500), ("ADReview.ttf", 700));
        for (dir, order) in [(&first, [upper, mixed]), (&second, [mixed, upper])] {
            for (name, advance) in order {
                fs::write(dir.join(name), truetype(advance)).unwrap();
            }
        }
        if fs::read_dir(&first).unwrap().count() < 2 {
            eprintln!("this file system ignores case in file names: nothing to choose between");
            return;
        }
        for dir in [&first, &second] {
            let system = with_system(&[], &[dir]);
            // No file has the name's own case: the first in name order wins.
            let (font, used) = find("adreview.ttf", &system);
            assert_eq!(used.source, Some(FontSource::System));
            assert_eq!(advance(&font), advance_of(500), "{}", dir.display());
            // A file with the name's own case wins over the others.
            let (font, _) = find("ADReview.ttf", &system);
            assert_eq!(advance(&font), advance_of(700), "{}", dir.display());
        }
        let _ = fs::remove_dir_all(&first);
        let _ = fs::remove_dir_all(&second);
    }

    #[test]
    fn a_font_any_number_of_folders_below_a_system_folder_is_found() {
        let system = folder("deep");
        let deep = (0..12).fold(system.clone(), |path, level| {
            path.join(format!("level-{level}"))
        });
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("deep.ttf"), truetype(640)).unwrap();
        let (font, used) = find("deep.ttf", &with_system(&[], &[&system]));
        assert!(matches!(font, Font::TrueType(_)), "{font:?}");
        assert_eq!(used.source, Some(FontSource::System));
        assert_eq!(advance(&font), advance_of(640));
        let _ = fs::remove_dir_all(&system);
    }

    #[test]
    fn a_folder_that_links_back_above_itself_ends_the_search() {
        // A link inside a system folder back to the folder itself, then a
        // font in a folder after the link in name order. Making a link to a
        // folder needs a privilege on Windows; without it there is no loop
        // to make, and the test says so and checks nothing.
        let system = folder("loop");
        fs::create_dir_all(system.join("a")).unwrap();
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&system, system.join("a").join("back"));
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_dir(&system, system.join("a").join("back"));
        if let Err(e) = linked {
            eprintln!("cannot make a link to a folder here ({e}): no loop to search");
            let _ = fs::remove_dir_all(&system);
            return;
        }
        fs::create_dir_all(system.join("z")).unwrap();
        fs::write(system.join("z").join("after.ttf"), truetype(660)).unwrap();
        let settings = with_system(&[], &[&system]);
        let (font, used) = find("after.ttf", &settings);
        assert_eq!(used.source, Some(FontSource::System));
        assert_eq!(advance(&font), advance_of(660));
        let (font, used) = find("nowhere.ttf", &settings);
        assert!(matches!(font, Font::Hershey));
        assert_eq!(used.why.as_deref(), Some("it is not in the font folders"));
        let _ = fs::remove_dir_all(&system);
    }

    #[test]
    fn an_shx_font_is_never_looked_for_in_the_system_folders() {
        let system = folder("shx-system");
        fs::write(
            system.join("madeup.shx"),
            unifont(10, &[(65, &[2, 8, 5, 0, 0])]),
        )
        .unwrap();
        let (font, used) = find("madeup.shx", &with_system(&[], &[&system]));
        assert!(matches!(font, Font::Hershey));
        assert!(used.substituted);
        assert_eq!(used.why.as_deref(), Some("no font folder is set"));
        let _ = fs::remove_dir_all(&system);
    }

    #[test]
    fn an_open_twin_in_a_system_folder_stands_in_and_says_where_it_was() {
        let system = folder("twin-system");
        fs::create_dir_all(system.join("liberation")).unwrap();
        fs::write(
            system.join("liberation").join("LiberationSans-Regular.ttf"),
            truetype(700),
        )
        .unwrap();
        let (font, used) = find("arial.ttf", &with_system(&[], &[&system]));
        assert!(matches!(font, Font::TrueType(_)));
        assert!(used.substituted);
        assert_eq!(used.used, "LiberationSans-Regular.ttf");
        assert_eq!(used.source, Some(FontSource::System));
        let _ = fs::remove_dir_all(&system);
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
