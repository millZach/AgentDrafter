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
//!   text on its `textbox`: what it draws, from its lowest point to its
//!   highest, except that for a TrueType font whose glyphs sit wholly above
//!   the baseline AutoCAD's `textbox` drops the gap below them to start at
//!   the baseline (#59).

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
    /// when text, on the sheet or in a block it inserts, draws with it.
    pub(crate) fn find(sheet: &Sheet, settings: &Settings) -> Self {
        let mut styles: Vec<TextStyle> = sheet.text_styles.clone();
        let in_blocks = sheet.blocks.iter().flat_map(|b| &b.objects);
        let standard_used = sheet
            .objects
            .iter()
            .chain(in_blocks)
            .any(|object| match object {
                Object::Text(text) => text.style == STANDARD,
                Object::MText(text) => text.style == STANDARD,
                Object::AttributeDefinition(definition) => definition.style == STANDARD,
                Object::Table(table) => {
                    let rows = sheet.table_style(&table.table_style).is_some_and(|style| {
                        [&style.title, &style.header, &style.data]
                            .iter()
                            .any(|row| row.text_style == STANDARD)
                    });
                    let cells = table
                        .cells
                        .iter()
                        .flatten()
                        .any(|cell| cell.style.as_deref() == Some(STANDARD));
                    rows || cells
                }
                Object::MLeader(mleader) => match mleader.content.text() {
                    Some(text) => match &text.style {
                        Some(style) => style == STANDARD,
                        None => sheet
                            .mleader_style(&mleader.mleader_style)
                            .is_some_and(|style| style.text_style == STANDARD),
                    },
                    // Block content draws its block's text, which the
                    // blocks' text styles account for.
                    None => false,
                },
                Object::Leader(leader) => leader
                    .annotation
                    .as_ref()
                    .is_some_and(|a| a.style == STANDARD),
                Object::Dimension(dimension) => sheet
                    .dimension_style(&dimension.dimension_style)
                    .is_some_and(|style| style.text_style == STANDARD),
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
                source: used.source,
            })
            .collect()
    }

    /// The font text in `style` draws with, and the style's width factor:
    /// the look-alike, at a normal width, for a style the sheet does not
    /// have.
    pub(crate) fn style(&self, style: &str) -> (&Font, f64) {
        self.styles
            .iter()
            .find(|(s, _, _)| s.name == style)
            .map_or((&Font::Hershey, 1.0), |(s, font, _)| (font, s.width_factor))
    }

    /// Where `text`'s baseline starts: its position when it is
    /// left-justified, and otherwise where AutoCAD would start it.
    pub(crate) fn start(&self, text: &Text) -> Point3 {
        if text.justify == Justify::Left {
            return text.position;
        }
        let (font, _) = self.style(&text.style);
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
            field: None,
            properties: Default::default(),
            annotative: false,
            annotation_scales: Vec::new(),
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
                    source: None,
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

    #[test]
    fn middle_justified_truetype_above_baseline_matches_autocad_probe() {
        use crate::text::shx::tests::unifont;
        use crate::text::shx::ShxFont;
        use crate::text::truetype::tests::made_up_font;
        use crate::text::truetype::TrueTypeFont;

        // Made-up TrueType font with glyphs reproducing the #59 AutoCAD 2026
        // probe's vertical geometry (cap height 1466; `-` at y=440..621,
        // `+` at y=237..1206, `x` at y=0..1062, `A`/`B` at y=0..1466, `C` at
        // y=-25..1491).
        let ttf = TrueTypeFont::parse(made_up_font(&[
            ('A', 1366, [0, 0, 1369, 1466]),
            ('B', 1366, [150, 0, 1257, 1466]),
            ('C', 1479, [102, -25, 1398, 1491]),
            ('x', 1024, [15, 0, 1009, 1062]),
            ('-', 682, [65, 440, 618, 621]),
            ('+', 1196, [114, 237, 1082, 1206]),
            (' ', 569, [0, 0, 0, 0]),
        ]))
        .unwrap();
        // Made-up SHX font (cap height 10) with elevated `-` (y=3..4),
        // elevated `+` (y=2..8), and baseline-touching `A`, `B`, `C`, `x`.
        let shx_dash: &[u8] = &[2, 8, 1, 3, 1, 8, 4, 0, 8, 0, 1, 2, 8, 1, 0xFC, 0];
        let shx_plus: &[u8] = &[2, 8, 1, 2, 1, 8, 4, 0, 8, 0, 6, 2, 8, 1, 0xF8, 0];
        let shx_base: &[u8] = &[1, 8, 6, 0, 8, 0, 10, 2, 8, 1, 0xF6, 0];
        let shx_x: &[u8] = &[1, 8, 5, 0, 8, 0, 7, 2, 8, 1, 0xF9, 0];
        let shx = ShxFont::parse(&unifont(
            10,
            &[
                (u16::from(b'-'), shx_dash),
                (u16::from(b'+'), shx_plus),
                (u16::from(b'A'), shx_base),
                (u16::from(b'B'), shx_base),
                (u16::from(b'C'), shx_base),
                (u16::from(b'x'), shx_x),
            ],
        ))
        .unwrap();

        let used = |name: &str| FontUsed {
            requested: name.into(),
            used: name.into(),
            substituted: false,
            why: None,
            source: None,
        };
        let sheet_fonts = SheetFonts {
            styles: vec![
                (
                    TextStyle {
                        name: "PROBE_TTF".into(),
                        font: "probe.ttf".into(),
                        width_factor: 1.0,
                        annotative: false,
                    },
                    Font::TrueType(ttf),
                    used("probe.ttf"),
                ),
                (
                    TextStyle {
                        name: "PROBE_SHX".into(),
                        font: "probe.shx".into(),
                        width_factor: 1.0,
                        annotative: false,
                    },
                    Font::Shx(shx),
                    used("probe.shx"),
                ),
            ],
        };

        let make =
            |contents: &str, style: &str, height: f64, justify: Justify, rotation: f64| Text {
                contents: contents.to_owned(),
                position: Point3::new(1.0, 1.5, 3.0),
                height,
                style: style.to_owned(),
                width_factor: 1.0,
                rotation,
                justify,
                layer: "0".to_owned(),
                field: None,
                properties: Default::default(),
                annotative: false,
                annotation_scales: Vec::new(),
            };

        // Check all 60 combinations of the #59 probe: 5 strings × 2 fonts ×
        // 2 heights × 3 justifications (`middle`, `middle_center`, `center`).
        for height in [0.0938, 1.0] {
            for contents in ["-", "+", "x", "ABC", "A-B"] {
                // TrueType `middle`: shifts up by `ink.bottom * height` when
                // drawn wholly above the baseline (`-`, `+`), and stays at the
                // drawn midpoint when touching or crossing it (`x`, `ABC`, `A-B`).
                let (ttf_font, _) = sheet_fonts.style("PROBE_TTF");
                let ttf_ink = ttf_font.line(contents).ink.unwrap();
                let drawn_mid_y = 1.5 - (ttf_ink.bottom + ttf_ink.top) / 2.0 * height;
                let expected_lift = ttf_ink.bottom.max(0.0) * height;
                let got_ttf_middle =
                    sheet_fonts.start(&make(contents, "PROBE_TTF", height, Justify::Middle, 0.0));
                assert!(
                    ((got_ttf_middle.y - drawn_mid_y) - expected_lift).abs() < 1e-12,
                    "TTF middle {contents:?} h={height}: delta={} expected={expected_lift}",
                    got_ttf_middle.y - drawn_mid_y
                );

                // TrueType `middle_center` and `center` are unaffected.
                let got_mc = sheet_fonts.start(&make(
                    contents,
                    "PROBE_TTF",
                    height,
                    Justify::MiddleCenter,
                    0.0,
                ));
                assert!((got_mc.y - (1.5 - height / 2.0)).abs() < 1e-12);
                let got_c =
                    sheet_fonts.start(&make(contents, "PROBE_TTF", height, Justify::Center, 0.0));
                assert!((got_c.y - 1.5).abs() < 1e-12);

                // SHX `middle` stays centred on its drawn box (no baseline shift).
                let (shx_font, _) = sheet_fonts.style("PROBE_SHX");
                let shx_ink = shx_font.line(contents).ink.unwrap();
                let shx_expected_y = 1.5 - (shx_ink.bottom + shx_ink.top) / 2.0 * height;
                let got_shx_middle =
                    sheet_fonts.start(&make(contents, "PROBE_SHX", height, Justify::Middle, 0.0));
                assert!(
                    (got_shx_middle.y - shx_expected_y).abs() < 1e-12,
                    "SHX middle {contents:?} h={height}: got={} expected={shx_expected_y}",
                    got_shx_middle.y
                );
                let got_shx_mc = sheet_fonts.start(&make(
                    contents,
                    "PROBE_SHX",
                    height,
                    Justify::MiddleCenter,
                    0.0,
                ));
                assert!((got_shx_mc.y - (1.5 - height / 2.0)).abs() < 1e-12);
                let got_shx_c =
                    sheet_fonts.start(&make(contents, "PROBE_SHX", height, Justify::Center, 0.0));
                assert!((got_shx_c.y - 1.5).abs() < 1e-12);
            }
        }

        // Exact AutoCAD 2026 probe deltas from #59 (to 6 decimal places):
        let (ttf_font, _) = sheet_fonts.style("PROBE_TTF");
        for (contents, height, expected_delta) in [
            ("-", 0.0938, 0.028153),
            ("+", 0.0938, 0.015164),
            ("-", 1.0, 0.300136),
            ("+", 1.0, 0.161664),
        ] {
            let ink = ttf_font.line(contents).ink.unwrap();
            let old_y = 1.5 - (ink.bottom + ink.top) / 2.0 * height;
            let new_y = sheet_fonts
                .start(&make(contents, "PROBE_TTF", height, Justify::Middle, 0.0))
                .y;
            assert!(
                ((new_y - old_y) - expected_delta).abs() < 1e-6,
                "{contents:?} h={height}: delta={} expected={expected_delta}",
                new_y - old_y
            );
        }

        // Leading/trailing spaces in TrueType do not undo the vertical shift:
        // `" - "` has the same `v` offset as `"-"`.
        let dash_y = sheet_fonts
            .start(&make("-", "PROBE_TTF", 1.0, Justify::Middle, 0.0))
            .y;
        let spaced_dash_y = sheet_fonts
            .start(&make(" - ", "PROBE_TTF", 1.0, Justify::Middle, 0.0))
            .y;
        assert!((dash_y - spaced_dash_y).abs() < 1e-12);

        // Rotation turns the adjusted `(u, v)` offset with the text.
        let rot90 = sheet_fonts.start(&make("-", "PROBE_TTF", 1.0, Justify::Middle, 90.0));
        let expected_v = -((621.0 - 440.0) / 1466.0) / 2.0;
        let expected_u = -(682.0 / 1466.0) / 2.0;
        close(rot90, 1.0 - expected_v, 1.5 + expected_u);
    }
}
