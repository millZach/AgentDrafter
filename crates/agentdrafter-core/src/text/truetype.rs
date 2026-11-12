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
//!
//! Every character is looked up in the font by [`glyph_index`], which reads
//! a font encoded as a Windows symbol font as Windows and AutoCAD read it.

use ttf_parser::{Face, GlyphId, PlatformId};

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
        let capitals = glyph_index(&face, 'A')
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
                let id = glyph_index(&face, c).unwrap_or(GlyphId(0));
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

    /// The outlines of `text`'s characters, each with its advance, in units
    /// where the capitals stand 1 tall: closed contours that fill by the
    /// non-zero rule, with curves as chords that stray at most `tolerance`
    /// from them. A character the font has no glyph for takes the font's
    /// missing glyph, as [`Self::glyphs`] measures it.
    pub(crate) fn outlines(&self, text: &str, tolerance: f64) -> Vec<(f64, Vec<Vec<[f64; 2]>>)> {
        let Ok(face) = Face::parse(&self.bytes, 0) else {
            return Vec::new();
        };
        let scale = 1.0 / self.capitals;
        text.chars()
            .map(|c| {
                let id = glyph_index(&face, c).unwrap_or(GlyphId(0));
                let advance = face.glyph_hor_advance(id).map_or(0.0, f64::from) * scale;
                let mut contours = Contours {
                    scale,
                    tolerance,
                    contours: Vec::new(),
                };
                face.outline_glyph(id, &mut contours);
                (advance, contours.contours)
            })
            .collect()
    }
}

/// The glyph `face` draws `c` with; none when it holds no glyph for it.
///
/// A font encoded as a Windows symbol font, whose character map has a
/// Windows symbol table, (3, 0), and no Unicode table, holds its characters
/// at U+F000 plus their code: `I`, 0x49, at U+F049. A character below U+0100
/// that such a font holds no glyph for at its own code is looked up there,
/// as Windows and AutoCAD do. ttf-parser reads only Unicode tables, so the
/// symbol table is read here. Any other font is read as ttf-parser reads it.
fn glyph_index(face: &Face, c: char) -> Option<GlyphId> {
    if let Some(id) = face.glyph_index(c) {
        return Some(id);
    }
    let subtables = face.tables().cmap?.subtables;
    if subtables.into_iter().any(|s| s.is_unicode()) {
        return None;
    }
    let symbol = subtables
        .into_iter()
        .find(|s| s.platform_id == PlatformId::Windows && s.encoding_id == 0)?;
    // A code mapped to glyph 0 is mapped to the missing glyph: not found.
    let found = |code: u32| symbol.glyph_index(code).filter(|id| id.0 != 0);
    let code = u32::from(c);
    match found(code) {
        None if code < 0x100 => found(0xF000 + code),
        id => id,
    }
}

/// Collects a glyph's outline as contours of points, curves flattened.
struct Contours {
    scale: f64,
    tolerance: f64,
    contours: Vec<Vec<[f64; 2]>>,
}

impl Contours {
    fn last(&self) -> [f64; 2] {
        self.contours
            .last()
            .and_then(|c| c.last())
            .copied()
            .unwrap_or([0.0, 0.0])
    }

    fn push(&mut self, point: [f64; 2]) {
        match self.contours.last_mut() {
            Some(contour) => contour.push(point),
            None => self.contours.push(vec![point]),
        }
    }

    /// Flattens a curve from the last point through `controls`, a quadratic
    /// with one control point and a cubic with two.
    fn curve(&mut self, controls: &[[f64; 2]], end: [f64; 2]) {
        let start = self.last();
        let mut hull = start;
        let mut length = 0.0;
        for &p in controls.iter().chain([&end]) {
            length += (p[0] - hull[0]).hypot(p[1] - hull[1]);
            hull = p;
        }
        // A chord strays from a curve by about the square of its length over
        // eight times the curve's radius, so chords as long as the square
        // root of the tolerance keep within it wherever the radius is more
        // than an eighth of the capitals' height.
        let steps = ((length / self.tolerance.sqrt()).ceil() as usize).clamp(1, 64);
        for step in 1..=steps {
            let t = step as f64 / steps as f64;
            let u = 1.0 - t;
            let point = match controls {
                [c] => [
                    u * u * start[0] + 2.0 * u * t * c[0] + t * t * end[0],
                    u * u * start[1] + 2.0 * u * t * c[1] + t * t * end[1],
                ],
                [c, d] => [
                    u * u * u * start[0]
                        + 3.0 * u * u * t * c[0]
                        + 3.0 * u * t * t * d[0]
                        + t * t * t * end[0],
                    u * u * u * start[1]
                        + 3.0 * u * u * t * c[1]
                        + 3.0 * u * t * t * d[1]
                        + t * t * t * end[1],
                ],
                _ => end,
            };
            self.push(point);
        }
    }

    fn scaled(&self, x: f32, y: f32) -> [f64; 2] {
        [f64::from(x) * self.scale, f64::from(y) * self.scale]
    }
}

impl ttf_parser::OutlineBuilder for Contours {
    fn move_to(&mut self, x: f32, y: f32) {
        let point = self.scaled(x, y);
        self.contours.push(vec![point]);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let point = self.scaled(x, y);
        self.push(point);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (control, end) = (self.scaled(x1, y1), self.scaled(x, y));
        self.curve(&[control], end);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (c, d, end) = (self.scaled(x1, y1), self.scaled(x2, y2), self.scaled(x, y));
        self.curve(&[c, d], end);
    }

    fn close(&mut self) {}
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
        made_up(glyphs, false)
    }

    /// The bytes of a font as [`made_up_font`] writes them, encoded as a
    /// Windows symbol font: a Windows symbol table, (3, 0), holds each
    /// character at U+F000 plus its code, and a Mac table, (1, 0), at its
    /// own code, with no Unicode table. Its characters are below U+0100.
    fn made_up_symbol_font(glyphs: &[MadeUpGlyph]) -> Vec<u8> {
        made_up(glyphs, true)
    }

    fn made_up(glyphs: &[MadeUpGlyph], symbol: bool) -> Vec<u8> {
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

        let mut mapped: Vec<(u16, u16)> = all
            .iter()
            .enumerate()
            .filter_map(|(id, (c, _, _))| c.map(|c| (c as u16, id as u16)))
            .collect();
        mapped.sort();
        // A format 4 character map: one segment per character, then the end.
        let format4 = |offset: u16| {
            let segments = mapped.len() as u16 + 1;
            let mut format4 = Vec::new();
            format4.extend(4u16.to_be_bytes());
            format4.extend((16 + 8 * segments).to_be_bytes());
            format4.extend(0u16.to_be_bytes());
            format4.extend((segments * 2).to_be_bytes());
            format4.extend([0u8; 6]); // search hints, which readers do not need
            for (code, _) in &mapped {
                format4.extend((offset + code).to_be_bytes());
            }
            format4.extend(0xFFFFu16.to_be_bytes());
            format4.extend(0u16.to_be_bytes());
            for (code, _) in &mapped {
                format4.extend((offset + code).to_be_bytes());
            }
            format4.extend(0xFFFFu16.to_be_bytes());
            for (code, id) in &mapped {
                format4.extend(id.wrapping_sub(offset + code).to_be_bytes());
            }
            format4.extend(1u16.to_be_bytes());
            format4.extend(vec![0u8; usize::from(segments) * 2]);
            format4
        };
        // Each character map's platform, encoding and table, in that order.
        let maps: Vec<(u16, u16, Vec<u8>)> = if symbol {
            // A format 0 map: the glyph of each of the 256 codes.
            let mut format0 = Vec::new();
            format0.extend(0u16.to_be_bytes());
            format0.extend(262u16.to_be_bytes());
            format0.extend(0u16.to_be_bytes());
            let mut ids = [0u8; 256];
            for (code, id) in &mapped {
                ids[usize::from(*code)] = *id as u8;
            }
            format0.extend(ids);
            vec![(1, 0, format0), (3, 0, format4(0xF000))] // Mac, Windows symbol
        } else {
            vec![(3, 1, format4(0))] // Windows Unicode BMP
        };
        let mut cmap = Vec::new();
        cmap.extend(0u16.to_be_bytes());
        cmap.extend((maps.len() as u16).to_be_bytes());
        let mut at = 4 + 8 * maps.len() as u32;
        for (platform, encoding, table) in &maps {
            cmap.extend(platform.to_be_bytes());
            cmap.extend(encoding.to_be_bytes());
            cmap.extend(at.to_be_bytes());
            at += table.len() as u32;
        }
        for (_, _, table) in maps {
            cmap.extend(table);
        }

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
    fn a_symbol_font_draws_its_characters_from_u_f000_on() {
        // A font encoded as a Windows symbol font holds `A` at U+F041 and
        // `I` at U+F049 (#62).
        let font = TrueTypeFont::parse(made_up_symbol_font(&[
            ('A', 700, [0, 0, 700, 640]),
            ('I', 300, [100, 0, 200, 640]),
        ]))
        .unwrap();
        // `I` measures as its own glyph, scaled to the top of `A`.
        let glyphs = font.glyphs("IZ");
        close(glyphs[0].advance.0, 300.0 / 640.0);
        let i = glyphs[0].ink.unwrap();
        close(i.left, 100.0 / 640.0);
        close(i.right, 200.0 / 640.0);
        close(i.top, 1.0);
        // Z has no glyph at U+005A or U+F05A, so it takes the missing glyph's.
        close(glyphs[1].advance.0, 500.0 / 640.0);
        // And `I` draws as it measures.
        let (advance, contours) = &font.outlines("I", 0.001)[0];
        close(*advance, 300.0 / 640.0);
        let xs = contours.iter().flatten().map(|p| p[0]);
        close(xs.clone().fold(f64::INFINITY, f64::min), 100.0 / 640.0);
        close(xs.fold(f64::NEG_INFINITY, f64::max), 200.0 / 640.0);
    }

    #[test]
    fn a_file_that_is_not_a_font_is_refused() {
        assert!(TrueTypeFont::parse(b"not a font".to_vec()).is_err());
    }
}
