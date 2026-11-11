//! Made-up CTB plot styles for tests. AutoCAD's own CTB files are
//! Autodesk's and never enter this repository, so tests write their own, in
//! the format AutoCAD writes: a heading, three numbers and zlib-packed text
//! of a section per pen. The core's own tests and its integration tests both
//! include this file, so there is one writer.

#![allow(dead_code)]

/// A colour a CTB writes for red, green and blue, as AutoCAD writes a pen's
/// own colour.
pub fn rgb(r: u8, g: u8, b: u8) -> i64 {
    i64::from((0xC2u32 << 24 | u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)) as i32)
}

/// The colour a CTB writes for a pen that plots the object's own colour.
pub const OBJECT_COLOR: i64 = -1;

/// The colour `monochrome.ctb` writes for each of its pens: black.
pub const BLACK: i64 = -16_777_216;

/// The lineweights in millimetres of AutoCAD's own lineweight table, as a
/// CTB writes them.
pub const LINEWEIGHTS: [f64; 27] = [
    0.0, 0.05, 0.09, 0.1, 0.13, 0.15, 0.18, 0.2, 0.25, 0.3, 0.35, 0.4, 0.45, 0.5, 0.53, 0.6, 0.65,
    0.7, 0.8, 0.9, 1.0, 1.06, 1.2, 1.4, 1.58, 2.0, 2.11,
];

/// The text of a made-up CTB file. Each pen not in `pens` plots black at
/// the object's own lineweight, as `monochrome.ctb` sets its pens; each in
/// `pens` is `(colour index, colour, lineweight)`, the colour and the
/// lineweight as the file writes them: the lineweight 0 for the object's
/// own, and `n` for the table's place `n - 1`, so 14 for 0.5 mm and 21 for
/// 1 mm. `extra` is written into every pen's section.
pub fn made_up_ctb_text(pens: &[(u8, i64, i64)], extra: &str) -> String {
    let mut text = String::from(
        "description=\"made up for a test\naci_table_available=TRUE\nscale_factor=1.0\n\
         apply_factor=FALSE\ncustom_lineweight_display_units=0\naci_table{\n",
    );
    for i in 0..255 {
        text.push_str(&format!(" {i}=\"Color_{}\n", i + 1));
    }
    text.push_str("}\nplot_style{\n");
    for i in 0..255u16 {
        let index = u8::try_from(i + 1).unwrap();
        let (color, lineweight) = pens
            .iter()
            .find(|(at, _, _)| *at == index)
            .map_or((BLACK, 0), |&(_, c, w)| (c, w));
        text.push_str(&format!(
            " {i}{{\n  name=\"Color_{index}\n  localized_name=\"Color_{index}\n  description=\"\n  \
             color={color}\n  color_policy=5\n  physical_pen_number=0\n  virtual_pen_number=0\n  \
             screen=100\n  linepattern_size=0.5\n  linetype=31\n  adaptive_linetype=TRUE\n  \
             lineweight={lineweight}\n  fill_style=73\n  end_style=4\n  join_style=5\n{extra} }}\n"
        ));
    }
    text.push_str("}\ncustom_lineweight_table{\n");
    for (i, mm) in LINEWEIGHTS.iter().enumerate() {
        text.push_str(&format!(" {i}={mm:?}\n"));
    }
    text.push_str("}\n");
    text
}

/// The bytes of a CTB file holding `text`, packed as AutoCAD packs it.
pub fn ctb_file(text: &str) -> Vec<u8> {
    let body = text.as_bytes();
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(body, 6);
    let mut bytes = b"PIAFILEVERSION_2.0,CTBVER1,compress\r\npmzlibcodec".to_vec();
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in &packed {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    bytes.extend(((b << 16) | a).to_le_bytes());
    bytes.extend(u32::try_from(body.len()).unwrap().to_le_bytes());
    bytes.extend(u32::try_from(packed.len()).unwrap().to_le_bytes());
    bytes.extend(packed);
    bytes
}

/// The bytes of a made-up CTB file whose pens are `pens`, as
/// [`made_up_ctb_text`] writes them.
pub fn made_up_ctb(pens: &[(u8, i64, i64)]) -> Vec<u8> {
    ctb_file(&made_up_ctb_text(pens, ""))
}
