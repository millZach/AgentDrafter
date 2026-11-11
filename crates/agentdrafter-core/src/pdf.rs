//! A PDF from a display list: the drawing pipeline's last step for a plot
//! (ADR-0011).
//!
//! The page is the sheet's size. Every item of the display list is drawn
//! in its order as vectors, through the plot style's pens: strokes with
//! round ends and corners at the pen's lineweight, fills by their rule. The
//! text draws as the display list draws it, an SHX font's strokes and a
//! TrueType font's filled outlines, so it looks as AutoCAD plots it; and
//! each line of text is written again on top as invisible text, character
//! by character where the line draws, so a PDF reader can search, select
//! and copy it.
//!
//! The invisible text is in Type 3 fonts whose glyphs draw nothing. Each
//! character the sheet uses, at each advance its fonts give it, is a glyph
//! of its own in one of them (256 to a font), exactly as wide as the
//! character's advance, and the font's `ToUnicode` map says which character
//! each glyph is, so any character can be searched for and copied,
//! whatever font draws it. Each line is set in its own frame, the line's
//! baseline and the height of its capitals, as one string: each character
//! then lands where its pen starts and its box reaches where the next one
//! starts, so a reader joins each word whole and parts words at the
//! spaces the text holds.
//!
//! The PDF carries the output's stamp in its document information, under
//! the key `AgentDrafter`, as one line of JSON in plain ASCII. The PDF
//! holds no date and no random identifier, so the same sheet plots to the
//! same bytes. Like the display list, this is plain computation: no
//! window, GPU, file system or thread.

use pdf_writer::types::{LineCapStyle, LineJoinStyle, TextRenderingMode};
use pdf_writer::types::{SystemInfo, UnicodeCmap};
use pdf_writer::{Content, Filter, Name, Pdf, Rect, Ref, Str, TextStr};

use crate::draw::{DisplayList, FillRule, Point2, Shape, TextRun};
use crate::plot_style::PlotStyle;

/// The key in a PDF's document information an output's stamp is written
/// under.
pub(crate) const STAMP_KEY: &[u8] = b"AgentDrafter";

/// Points per millimetre: 72 to the inch.
pub(crate) const POINTS_PER_MILLIMETRE: f64 = 72.0 / 25.4;

/// How finely positions are written: a thousandth of a point, 0.35 µm.
const GRID: f64 = 1000.0;

/// Where the drawing sits on the page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Page {
    /// The drawing point at the page's lower left corner.
    pub origin: Point2,
    /// How many points one drawing unit spans.
    pub points_per_unit: f64,
    /// The page's width in points.
    pub width: f64,
    /// The page's height in points.
    pub height: f64,
}

impl Page {
    /// Where a drawing point is on the page, in points from its lower left
    /// corner.
    fn at(&self, p: Point2) -> (f32, f32) {
        (
            snap((p[0] - self.origin[0]) * self.points_per_unit),
            snap((p[1] - self.origin[1]) * self.points_per_unit),
        )
    }

    /// A drawing vector on the page, in points.
    fn vector(&self, v: Point2) -> [f64; 2] {
        [v[0] * self.points_per_unit, v[1] * self.points_per_unit]
    }
}

/// A number rounded to the grid positions are written on.
fn snap(points: f64) -> f32 {
    ((points * GRID).round() / GRID) as f32
}

/// What the PDF says about itself in its document information.
pub(crate) struct About<'a> {
    /// The sheet's title, such as its name.
    pub title: &'a str,
    /// What made it, such as `agentdrafter 0.0.0`.
    pub producer: &'a str,
    /// How it was made, such as the plot style it was drawn through.
    pub creator: &'a str,
    /// The output's stamp, as JSON.
    pub stamp: &'a str,
}

/// The bytes of a one-page PDF of `list`, drawn through `style` on `page`.
pub(crate) fn pdf(list: &DisplayList, page: &Page, style: &PlotStyle, about: &About) -> Vec<u8> {
    let mut alloc = Ref::new(1);
    let mut next = || alloc.bump();
    let catalog = next();
    let pages = next();
    let page_id = next();
    let content_id = next();
    let info = next();

    let codes = Codes::of(&list.text);
    // Each font of the invisible text: the font, its `ToUnicode` map and
    // one drawing per glyph.
    let fonts: Vec<(Ref, Ref, Vec<Ref>)> = (0..codes.fonts())
        .map(|i| {
            (
                next(),
                next(),
                codes.of_font(i).iter().map(|_| next()).collect(),
            )
        })
        .collect();

    let mut writer = Pdf::new();
    writer.catalog(catalog).pages(pages);
    writer.pages(pages).kids([page_id]).count(1);
    {
        let mut sheet = writer.page(page_id);
        sheet
            .media_box(Rect::new(0.0, 0.0, page.width as f32, page.height as f32))
            .parent(pages)
            .contents(content_id);
        let mut resources = sheet.resources();
        let mut named = resources.fonts();
        for (i, (font, _, _)) in fonts.iter().enumerate() {
            named.pair(Name(font_name(i).as_bytes()), *font);
        }
    }

    let content = drawing(list, page, style, &codes);
    writer
        .stream(content_id, &pack(&content))
        .filter(Filter::FlateDecode);

    for (i, (font, map, drawings)) in fonts.iter().enumerate() {
        let glyphs = codes.of_font(i);
        let last = u8::try_from(glyphs.len() - 1).expect("at most 256 codes to a font");
        let names: Vec<String> = (0..glyphs.len()).map(|c| format!("g{c}")).collect();
        let mut t3 = writer.type3_font(*font);
        t3.bbox(Rect::new(0.0, -300.0, 1000.0, 1000.0))
            .matrix([0.001, 0.0, 0.0, 0.001, 0.0, 0.0])
            .first_char(0)
            .last_char(last)
            .widths(glyphs.iter().map(|g| g.width))
            .to_unicode(*map);
        {
            let mut procs = t3.char_procs();
            for (name, drawing) in names.iter().zip(drawings) {
                procs.pair(Name(name.as_bytes()), *drawing);
            }
        }
        t3.encoding_custom()
            .differences()
            .consecutive(0, names.iter().map(|n| Name(n.as_bytes())));
        drop(t3);
        // Each glyph draws nothing, inside a box as wide as it moves the pen
        // and as tall as the capitals with room for descenders, which is
        // where a reader puts the character.
        for (glyph, drawing) in glyphs.iter().zip(drawings) {
            let mut nothing = Content::new();
            let (left, right) = (glyph.width.min(0.0), glyph.width.max(0.0).max(1.0));
            nothing.start_shape_glyph(glyph.width, left, -300.0, right, 1000.0);
            writer.stream(*drawing, &nothing.finish());
        }
        let mut cmap = UnicodeCmap::<u8>::new(
            Name(b"AgentDrafter-UCS"),
            SystemInfo {
                registry: Str(b"Adobe"),
                ordering: Str(b"UCS"),
                supplement: 0,
            },
        );
        for (code, glyph) in glyphs.iter().enumerate() {
            cmap.pair(
                u8::try_from(code).expect("at most 256 codes"),
                glyph.character,
            );
        }
        writer
            .cmap(*map, &pack(&cmap.finish()))
            .filter(Filter::FlateDecode);
    }

    let stamp = ascii_json(about.stamp);
    writer
        .document_info(info)
        .title(TextStr(about.title))
        .producer(TextStr(about.producer))
        .creator(TextStr(about.creator))
        .pair(Name(STAMP_KEY), Str(stamp.as_bytes()));
    writer.finish()
}

/// The page's content: the display list drawn, then its text written
/// invisibly on top.
fn drawing(list: &DisplayList, page: &Page, style: &PlotStyle, codes: &Codes) -> Vec<u8> {
    let mut content = Content::new();
    content
        .set_line_cap(LineCapStyle::RoundCap)
        .set_line_join(LineJoinStyle::RoundJoin);
    let mut stroke: Option<([u8; 3], f32)> = None;
    let mut fill: Option<[u8; 3]> = None;
    let unit = |c: u8| f32::from(c) / 255.0;
    for item in &list.items {
        let rgb = style.color(item.color);
        match &item.shape {
            Shape::Stroke { lineweight, runs } => {
                let width = snap(style.lineweight(item.color, *lineweight) * POINTS_PER_MILLIMETRE);
                let mut drawn = false;
                for run in runs {
                    let points: Vec<(f32, f32)> = run.iter().map(|&p| page.at(p)).collect();
                    let Some(&first) = points.first() else {
                        continue;
                    };
                    if !drawn {
                        let pen = (rgb, width);
                        if stroke.map(|(c, _)| c) != Some(rgb) {
                            content.set_stroke_rgb(unit(rgb[0]), unit(rgb[1]), unit(rgb[2]));
                        }
                        if stroke.map(|(_, w)| w) != Some(width) {
                            content.set_line_width(width);
                        }
                        stroke = Some(pen);
                        drawn = true;
                    }
                    content.move_to(first.0, first.1);
                    if points.iter().all(|&p| p == first) {
                        // A dot: a line of no length, which a round end
                        // draws as a dot as wide as the pen.
                        content.line_to(first.0, first.1);
                        continue;
                    }
                    let mut last = first;
                    for &p in &points[1..] {
                        if p != last {
                            content.line_to(p.0, p.1);
                            last = p;
                        }
                    }
                }
                if drawn {
                    content.stroke();
                }
            }
            Shape::Fill { rule, contours } => {
                let mut drawn = false;
                for contour in contours {
                    let mut points = contour.iter().map(|&p| page.at(p));
                    let Some(first) = points.next() else {
                        continue;
                    };
                    if !drawn {
                        if fill != Some(rgb) {
                            content.set_fill_rgb(unit(rgb[0]), unit(rgb[1]), unit(rgb[2]));
                            fill = Some(rgb);
                        }
                        drawn = true;
                    }
                    content.move_to(first.0, first.1);
                    for (x, y) in points {
                        content.line_to(x, y);
                    }
                    content.close_path();
                }
                if drawn {
                    match rule {
                        FillRule::EvenOdd => content.fill_even_odd(),
                        FillRule::NonZero => content.fill_nonzero(),
                    };
                }
            }
        }
    }
    if !list.text.is_empty() {
        content.begin_text();
        content.set_text_rendering_mode(TextRenderingMode::Invisible);
        for run in &list.text {
            write_run(&mut content, run, page, codes);
        }
        content.end_text();
    }
    content.finish().into_vec()
}

/// Writes one line of text, invisibly, each character where its pen
/// starts.
fn write_run(content: &mut Content, run: &TextRun, page: &Page, codes: &Codes) {
    let (x, y) = page.at(run.origin);
    let across = page.vector(run.across);
    let up = page.vector(run.up);
    // A line whose frame has no size draws nowhere a reader could find it.
    if (across[0] * up[1] - across[1] * up[0]).abs() < 1e-9 {
        return;
    }
    content.set_text_matrix([
        across[0] as f32,
        across[1] as f32,
        up[0] as f32,
        up[1] as f32,
        x,
        y,
    ]);
    let mut font = None;
    let mut shown: Vec<u8> = Vec::new();
    for &(c, advance) in &run.characters {
        let (f, code) = codes.code(c, advance);
        if font != Some(f) {
            if !shown.is_empty() {
                content.show(Str(&shown));
                shown.clear();
            }
            content.set_font(Name(font_name(f).as_bytes()), 1.0);
            font = Some(f);
        }
        shown.push(code);
    }
    if !shown.is_empty() {
        content.show(Str(&shown));
    }
}

/// The resource name of the invisible text's font `i`.
fn font_name(i: usize) -> String {
    format!("AD{i}")
}

/// One glyph of the invisible text: the character it is, and its width in
/// thousandths of the font's advances.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Glyph {
    character: char,
    width: f32,
}

/// The code each character of the sheet's text is written with, at each
/// advance it moves the pen: the font, and the byte in it. Glyphs are given
/// codes in the order the text first uses them, 256 to a font.
struct Codes {
    glyphs: Vec<Glyph>,
    places: std::collections::HashMap<(char, u32), usize>,
}

impl Codes {
    fn of(runs: &[TextRun]) -> Codes {
        let mut glyphs: Vec<Glyph> = Vec::new();
        let mut places = std::collections::HashMap::new();
        for run in runs {
            for &(character, advance) in &run.characters {
                let width = Codes::width(advance);
                places
                    .entry((character, width.to_bits()))
                    .or_insert_with(|| {
                        glyphs.push(Glyph { character, width });
                        glyphs.len() - 1
                    });
            }
        }
        Codes { glyphs, places }
    }

    /// A glyph's width for an advance of `advance` of the font's units, in
    /// thousandths, to a hundredth.
    fn width(advance: f64) -> f32 {
        ((advance * 1000.0 * 100.0).round() / 100.0) as f32
    }

    fn fonts(&self) -> usize {
        self.glyphs.len().div_ceil(256)
    }

    fn of_font(&self, font: usize) -> &[Glyph] {
        let end = (font * 256 + 256).min(self.glyphs.len());
        &self.glyphs[font * 256..end]
    }

    fn code(&self, character: char, advance: f64) -> (usize, u8) {
        let at = self.places[&(character, Codes::width(advance).to_bits())];
        (at / 256, (at % 256) as u8)
    }
}

/// `bytes` packed with deflate, as a PDF stream's `FlateDecode` unpacks.
fn pack(bytes: &[u8]) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec_zlib(bytes, 6)
}

/// JSON with every character outside ASCII written as a `\u` escape, which
/// reads back as the same JSON.
fn ascii_json(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    for c in json.chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            for unit in c.encode_utf16(&mut [0; 2]) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}

/// The stamp written in a PDF's document information, if it has one: the
/// last string written under the key `AgentDrafter`, as a later update of
/// the PDF would add it.
pub(crate) fn stamp_in(bytes: &[u8]) -> Option<String> {
    if !bytes.starts_with(b"%PDF-") {
        return None;
    }
    let key = [b"/".as_slice(), STAMP_KEY].concat();
    let mut found = None;
    let mut from = 0;
    while let Some(at) = find(&bytes[from..], &key) {
        let after = from + at + key.len();
        let rest = &bytes[after..];
        let start = rest.iter().position(|b| !b.is_ascii_whitespace());
        if let Some(text) = start.and_then(|s| literal_string(&rest[s..])) {
            found = Some(text);
        }
        from = after;
    }
    let bytes = found?;
    String::from_utf8(bytes).ok()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// The bytes of the PDF literal string `(...)` at the start of `bytes`,
/// its escapes undone.
fn literal_string(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.first() != Some(&b'(') {
        return None;
    }
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut i = 1;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'\\' => {
                let next = *bytes.get(i + 1)?;
                i += 2;
                match next {
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b't' => out.push(b'\t'),
                    b'b' => out.push(0x08),
                    b'f' => out.push(0x0c),
                    b'\r' => {
                        if bytes.get(i) == Some(&b'\n') {
                            i += 1;
                        }
                    }
                    b'\n' => {}
                    b'0'..=b'7' => {
                        let mut value = u32::from(next - b'0');
                        for _ in 0..2 {
                            match bytes.get(i) {
                                Some(&d @ b'0'..=b'7') => {
                                    value = value * 8 + u32::from(d - b'0');
                                    i += 1;
                                }
                                _ => break,
                            }
                        }
                        out.push((value & 0xff) as u8);
                    }
                    other => out.push(other),
                }
                continue;
            }
            b'(' => depth += 1,
            b')' => {
                if depth == 0 {
                    return Some(out);
                }
                depth -= 1;
            }
            _ => {}
        }
        out.push(b);
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stamp_reads_back_as_it_was_written() {
        let stamp = r#"{"stamp":1,"produced_by":"agentdrafter 0.0.0","sources":[{"kind":"dwg","path":"a (1)\\b é.dwg","sha256":"00"}]}"#;
        let bytes = pdf(
            &DisplayList::default(),
            &Page {
                origin: [0.0, 0.0],
                points_per_unit: 1.0,
                width: 10.0,
                height: 10.0,
            },
            &PlotStyle::monochrome(),
            &About {
                title: "a",
                producer: "agentdrafter 0.0.0",
                creator: "test",
                stamp,
            },
        );
        let read = stamp_in(&bytes).unwrap();
        let a: serde_json::Value = serde_json::from_str(&read).unwrap();
        let b: serde_json::Value = serde_json::from_str(stamp).unwrap();
        assert_eq!(a, b);
        assert!(read.is_ascii());
        assert_eq!(stamp_in(b"%PDF-1.7 no stamp"), None);
        assert_eq!(stamp_in(b"not a pdf /AgentDrafter (x)"), None);
    }

    #[test]
    fn literal_strings_undo_their_escapes() {
        assert_eq!(
            literal_string(br"(a\(b\) (c) \\ \101\n) tail").unwrap(),
            b"a(b) (c) \\ A\n"
        );
        assert_eq!(literal_string(b"(unclosed"), None);
    }

    #[test]
    fn characters_get_codes_256_to_a_font_in_the_order_first_used() {
        let run = |text: &str| TextRun {
            object: 0,
            origin: [0.0, 0.0],
            across: [1.0, 0.0],
            up: [0.0, 1.0],
            characters: text.chars().map(|c| (c, 1.0)).collect(),
        };
        let many: String = (0..300u32)
            .filter_map(|i| char::from_u32(0x4e00 + i))
            .collect();
        let mut wide = run("B");
        wide.characters[0].1 = 2.0;
        let codes = Codes::of(&[run("BAB"), run(&many), wide]);
        assert_eq!(codes.code('B', 1.0), (0, 0));
        assert_eq!(codes.code('A', 1.0), (0, 1));
        // The same character at another advance is a glyph of its own.
        assert_eq!(codes.code('B', 2.0), (1, 46));
        assert_eq!(codes.fonts(), 2);
        assert_eq!(codes.of_font(1).len(), 303 - 256);
        assert_eq!(codes.of_font(1)[302 - 256].width, 2000.0);
    }
}
