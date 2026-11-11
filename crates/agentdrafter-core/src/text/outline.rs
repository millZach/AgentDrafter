//! What a font draws: each character's strokes or filled outlines, for the
//! drawing pipeline ([`crate::draw`]).
//!
//! An SHX font and the built-in look-alike draw strokes, which a text draws
//! with its layer's pen. A TrueType font draws closed outlines, which are
//! filled, as AutoCAD fills TrueType text when it plots. Advances agree with
//! the measurements in [`super::fonts`], so drawn text sits where measured
//! text says it does.

use super::fonts::Font;
use super::hershey;

/// One character as a font draws it, in units where the capitals stand 1
/// tall, from where the pen starts.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct GlyphOutline {
    /// Where the pen ends: where the next character starts.
    pub advance: (f64, f64),
    /// Strokes drawn with the pen down; a run of one point is a dot.
    pub strokes: Vec<Vec<[f64; 2]>>,
    /// Closed contours, filled by the non-zero rule.
    pub fills: Vec<Vec<[f64; 2]>>,
}

impl Font {
    /// The outline of each character of `text` the font draws, curves as
    /// chords that stray at most `tolerance` from them, in units where the
    /// capitals stand 1 tall. A character the font cannot draw is skipped
    /// as [`Font::glyphs`] skips it.
    pub(crate) fn outlines(&self, text: &str, tolerance: f64) -> Vec<GlyphOutline> {
        match self {
            Font::Shx(font) => text
                .chars()
                .filter_map(|c| font.strokes(c, tolerance))
                .map(|(advance, strokes)| GlyphOutline {
                    advance,
                    strokes,
                    fills: Vec::new(),
                })
                .collect(),
            Font::TrueType(font) => font
                .outlines(text, tolerance)
                .into_iter()
                .map(|(advance, fills)| GlyphOutline {
                    advance: (advance, 0.0),
                    strokes: Vec::new(),
                    fills,
                })
                .collect(),
            Font::Hershey => text
                .chars()
                .map(|c| {
                    let (advance, strokes) = hershey::strokes(c);
                    GlyphOutline {
                        advance: (advance, 0.0),
                        strokes,
                        fills: Vec::new(),
                    }
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::shx::tests::unifont;
    use crate::text::shx::ShxFont;
    use crate::text::truetype::tests::made_up_font;
    use crate::text::truetype::TrueTypeFont;

    #[test]
    fn outlines_advance_as_far_as_measuring_says() {
        let i: &[u8] = &[2, 8, 2, 0, 1, 0xA4, 2, 8, 2, 0xF6, 0];
        let shx = Font::Shx(ShxFont::parse(&unifont(10, &[(73, i)])).unwrap());
        let truetype = Font::TrueType(
            TrueTypeFont::parse(made_up_font(&[('I', 300, [100, 0, 200, 700])])).unwrap(),
        );
        for font in [shx, truetype, Font::Hershey] {
            let outlines = font.outlines("II", 0.001);
            let glyphs = font.glyphs("II");
            assert_eq!(outlines.len(), glyphs.len());
            for (outline, glyph) in outlines.iter().zip(&glyphs) {
                assert!((outline.advance.0 - glyph.advance.0).abs() < 1e-12);
                assert!(!outline.strokes.is_empty() || !outline.fills.is_empty());
            }
        }
    }

    #[test]
    fn an_shx_stroke_runs_from_where_the_pen_comes_down_to_where_it_lifts() {
        // `I`: pen up, to (2, 0), down, up 10, up, on to (4, 0).
        let i: &[u8] = &[2, 8, 2, 0, 1, 0xA4, 2, 8, 2, 0xF6, 0];
        let font = Font::Shx(ShxFont::parse(&unifont(10, &[(73, i)])).unwrap());
        let outline = &font.outlines("I", 0.001)[0];
        assert_eq!(outline.strokes, vec![vec![[0.2, 0.0], [0.2, 1.0]]]);
        assert_eq!(outline.advance, (0.4, 0.0));
    }

    #[test]
    fn an_shx_arc_is_drawn_as_chords_close_to_it() {
        // A half circle of radius 5 from (0, 0) below its chord to (10, 0).
        let program = [10, 5, 0x44, 0];
        let font = Font::Shx(ShxFont::parse(&unifont(10, &[(65, &program)])).unwrap());
        let outline = &font.outlines("A", 0.001)[0];
        let run = &outline.strokes[0];
        assert!(run.len() > 10, "{run:?}");
        for p in run {
            let r = (p[0] - 0.5).hypot(p[1]);
            assert!((r - 0.5).abs() < 1e-9, "{p:?} is off the circle");
            assert!(p[1] <= 1e-12, "{p:?} is above the chord");
        }
    }

    #[test]
    fn a_truetype_outline_is_its_contours() {
        let font = Font::TrueType(
            TrueTypeFont::parse(made_up_font(&[('A', 700, [0, 0, 700, 700])])).unwrap(),
        );
        let outline = &font.outlines("A", 0.001)[0];
        assert_eq!(outline.fills.len(), 1);
        let xs: Vec<f64> = outline.fills[0].iter().map(|p| p[0]).collect();
        assert!(xs.contains(&0.0) && xs.contains(&1.0), "{xs:?}");
    }
}
