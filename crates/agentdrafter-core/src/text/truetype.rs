//! Reads the metrics of a TrueType font, enough to measure text.
//!
//! The tables are read by ttf-parser (MIT or Apache-2.0). Measuring needs
//! each character's advance and the box around its outline, which the
//! glyph table records for every glyph, and the height AutoCAD scales a
//! font to.
//!
//! AutoCAD scales a TrueType font so that the top of its `A` sits at the
//! text's height. (AutoCAD 2026 measures text in Times New Roman, whose `A`
//! rises above its flat capitals, 2.3% smaller than its flat capitals would
//! give, and text in Arial, whose `A` is as tall as its `H`, to the top of
//! `H`. The font's own cap height, a separate figure in the font, is a unit
//! taller than Arial's `A`.)

use ttf_parser::{Face, GlyphId};

use super::fonts::{Glyph, Rect};

/// A TrueType font, read.
#[derive(Debug)]
pub(crate) struct TrueTypeFont {
    bytes: Vec<u8>,
    /// How tall the capitals stand, in font units: the top of `A`.
    capitals: f64,
    /// How far the descenders reach below the baseline, in font units.
    descent: f64,
}

impl TrueTypeFont {
    /// Reads the bytes of a TrueType font. The error says what is wrong.
    pub(crate) fn parse(bytes: Vec<u8>) -> Result<Self, String> {
        let face =
            Face::parse(&bytes, 0).map_err(|e| format!("it is not a TrueType font ({e})"))?;
        let capitals = face
            .glyph_index('A')
            .and_then(|h| face.glyph_bounding_box(h))
            .map(|b| f64::from(b.y_max))
            .filter(|top| *top > 0.0)
            .or_else(|| face.capital_height().map(f64::from).filter(|c| *c > 0.0))
            .unwrap_or_else(|| f64::from(face.ascender()));
        if capitals <= 0.0 {
            return Err("it gives no height for its capitals".to_owned());
        }
        // The descent Windows gives the font, which AutoCAD's bottom
        // justifications lift text by; the horizontal header's without it.
        let descent = face
            .tables()
            .os2
            .map(|os2| f64::from(os2.windows_descender()).abs())
            .unwrap_or_else(|| f64::from(face.descender()).abs());
        Ok(Self {
            bytes,
            capitals,
            descent,
        })
    }

    /// How far the descenders reach below the baseline, in units where the
    /// capitals stand 1 tall.
    pub(crate) fn descent(&self) -> f64 {
        self.descent / self.capitals
    }

    /// The glyphs for `text`, in units where the capitals stand 1 tall. A
    /// character the font has no glyph for takes the font's missing-glyph
    /// box, as AutoCAD draws it.
    pub(crate) fn glyphs(&self, text: &str) -> Vec<Glyph> {
        let Ok(face) = Face::parse(&self.bytes, 0) else {
            // It parsed when it was read.
            return Vec::new();
        };
        let scale = 1.0 / self.capitals;
        text.chars()
            .map(|c| {
                let id = face.glyph_index(c).unwrap_or(GlyphId(0));
                let advance = face.glyph_hor_advance(id).map_or(0.0, f64::from);
                let ink = face.glyph_bounding_box(id).map(|b| Rect {
                    left: f64::from(b.x_min) * scale,
                    right: f64::from(b.x_max) * scale,
                    bottom: f64::from(b.y_min) * scale,
                    top: f64::from(b.y_max) * scale,
                });
                Glyph {
                    advance: (advance * scale, 0.0),
                    ink,
                }
            })
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A glyph of a made-up font: its character, advance and box, in font
    /// units. A box of all zeros is a glyph with no outline, such as a space.
    pub(crate) type MadeUpGlyph = (char, u16, [i16; 4]);

    /// The bytes of a small TrueType font holding `glyphs`, each a square
    /// filling its box, with 1000 units to the em. Glyph 0 is the missing
    /// glyph, 500 wide. Written here; no real font is copied.
    pub(crate) fn made_up_font(glyphs: &[MadeUpGlyph]) -> Vec<u8> {
        let mut all: Vec<(Option<char>, u16, [i16; 4])> = vec![(None, 500, [50, 0, 450, 700])];
        all.extend(glyphs.iter().map(|&(c, a, b)| (Some(c), a, b)));
        let count = all.len() as u16;

        let mut glyf = Vec::new();
        let mut loca = vec![0u32];
        for (_, _, [x0, y0, x1, y1]) in &all {
            if [*x0, *y0, *x1, *y1] != [0, 0, 0, 0] {
                // One contour of four on-curve points around the box.
                glyf.extend(1i16.to_be_bytes());
                for v in [x0, y0, x1, y1] {
                    glyf.extend(v.to_be_bytes());
                }
                glyf.extend(3u16.to_be_bytes()); // last point of the contour
                glyf.extend(0u16.to_be_bytes()); // no instructions
                glyf.extend([1u8; 4]); // flags: on curve, two-byte coordinates
                let xs = [*x0, *x1, *x1, *x0];
                let ys = [*y0, *y0, *y1, *y1];
                let mut last = 0;
                for x in xs {
                    glyf.extend((x - last).to_be_bytes());
                    last = x;
                }
                last = 0;
                for y in ys {
                    glyf.extend((y - last).to_be_bytes());
                    last = y;
                }
                while glyf.len() % 4 != 0 {
                    glyf.push(0);
                }
            }
            loca.push(glyf.len() as u32);
        }

        let mut head = Vec::new();
        head.extend(0x0001_0000u32.to_be_bytes()); // version
        head.extend(0x0001_0000u32.to_be_bytes()); // font revision
        head.extend(0u32.to_be_bytes()); // checksum adjustment
        head.extend(0x5F0F_3CF5u32.to_be_bytes()); // magic
        head.extend(0u16.to_be_bytes()); // flags
        head.extend(1000u16.to_be_bytes()); // units per em
        head.extend([0u8; 16]); // created, modified
        for v in [0i16, -200, 1000, 800] {
            head.extend(v.to_be_bytes()); // font box
        }
        head.extend([0u8; 6]); // mac style, smallest size, direction hint
        head.extend(1i16.to_be_bytes()); // long offsets in loca
        head.extend(0i16.to_be_bytes()); // glyph data format

        let mut hhea = Vec::new();
        hhea.extend(0x0001_0000u32.to_be_bytes());
        for v in [800i16, -200, 0] {
            hhea.extend(v.to_be_bytes()); // ascender, descender, line gap
        }
        hhea.extend(1000u16.to_be_bytes()); // widest advance
        hhea.extend([0u8; 22]);
        hhea.extend(count.to_be_bytes()); // advances in hmtx

        let mut maxp = Vec::new();
        maxp.extend(0x0000_5000u32.to_be_bytes());
        maxp.extend(count.to_be_bytes());

        let mut hmtx = Vec::new();
        for (_, advance, [x0, ..]) in &all {
            hmtx.extend(advance.to_be_bytes());
            hmtx.extend(x0.to_be_bytes());
        }

        // A format 4 character map: one segment per character, then the end.
        let mut mapped: Vec<(u16, u16)> = all
            .iter()
            .enumerate()
            .filter_map(|(id, (c, _, _))| c.map(|c| (c as u16, id as u16)))
            .collect();
        mapped.sort();
        let segments = mapped.len() as u16 + 1;
        let mut format4 = Vec::new();
        format4.extend(4u16.to_be_bytes());
        format4.extend((16 + 8 * segments).to_be_bytes());
        format4.extend(0u16.to_be_bytes());
        format4.extend((segments * 2).to_be_bytes());
        format4.extend([0u8; 6]); // search hints, which readers do not need
        for (code, _) in &mapped {
            format4.extend(code.to_be_bytes());
        }
        format4.extend(0xFFFFu16.to_be_bytes());
        format4.extend(0u16.to_be_bytes());
        for (code, _) in &mapped {
            format4.extend(code.to_be_bytes());
        }
        format4.extend(0xFFFFu16.to_be_bytes());
        for (code, id) in &mapped {
            format4.extend(id.wrapping_sub(*code).to_be_bytes());
        }
        format4.extend(1u16.to_be_bytes());
        format4.extend(vec![0u8; usize::from(segments) * 2]);
        let mut cmap = Vec::new();
        cmap.extend(0u16.to_be_bytes());
        cmap.extend(1u16.to_be_bytes());
        cmap.extend(3u16.to_be_bytes()); // Windows
        cmap.extend(1u16.to_be_bytes()); // Unicode BMP
        cmap.extend(12u32.to_be_bytes());
        cmap.extend(format4);

        let mut loca_bytes = Vec::new();
        for offset in loca {
            loca_bytes.extend(offset.to_be_bytes());
        }

        let tables: [(&[u8; 4], Vec<u8>); 7] = [
            (b"cmap", cmap),
            (b"glyf", glyf),
            (b"head", head),
            (b"hhea", hhea),
            (b"hmtx", hmtx),
            (b"loca", loca_bytes),
            (b"maxp", maxp),
        ];
        let mut font = Vec::new();
        font.extend(0x0001_0000u32.to_be_bytes());
        font.extend((tables.len() as u16).to_be_bytes());
        font.extend([0u8; 6]);
        let mut offset = 12 + 16 * tables.len();
        let mut data = Vec::new();
        for (tag, table) in &tables {
            font.extend(*tag);
            font.extend(0u32.to_be_bytes()); // checksum, which readers do not need
            font.extend((offset as u32).to_be_bytes());
            font.extend((table.len() as u32).to_be_bytes());
            let mut padded = table.clone();
            while padded.len() % 4 != 0 {
                padded.push(0);
            }
            offset += padded.len();
            data.extend(padded);
        }
        font.extend(data);
        font
    }

    #[track_caller]
    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-12, "{a} and {b}");
    }

    #[test]
    fn glyphs_scale_to_the_top_of_a() {
        let font = TrueTypeFont::parse(made_up_font(&[
            ('A', 700, [0, 0, 700, 640]),
            ('H', 800, [100, 0, 700, 620]),
            ('g', 600, [50, -160, 550, 480]),
            (' ', 250, [0, 0, 0, 0]),
        ]))
        .unwrap();
        let glyphs = font.glyphs("Hg Z");
        assert_eq!(glyphs.len(), 4);
        close(glyphs[0].advance.0, 800.0 / 640.0);
        let h = glyphs[0].ink.unwrap();
        close(h.left, 100.0 / 640.0);
        close(h.top, 620.0 / 640.0);
        let g = glyphs[1].ink.unwrap();
        close(g.bottom, -0.25);
        assert_eq!(glyphs[2].ink, None);
        close(glyphs[2].advance.0, 250.0 / 640.0);
        // Z has no glyph, so it takes the missing glyph's.
        close(glyphs[3].advance.0, 500.0 / 640.0);
        close(glyphs[3].ink.unwrap().right, 450.0 / 640.0);
    }

    #[test]
    fn a_file_that_is_not_a_font_is_refused() {
        assert!(TrueTypeFont::parse(b"not a font".to_vec()).is_err());
    }
}
