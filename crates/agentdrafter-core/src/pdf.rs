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
/// string under the key `AgentDrafter` in the dictionary the PDF's last
/// trailer names as `/Info`, found through its cross-reference sections,
/// tables or streams. The same text anywhere else, such as in a page's
/// text, is not a stamp. A PDF that cannot be read so, however it is
/// broken, has no stamp.
pub(crate) fn stamp_in(bytes: &[u8]) -> Option<String> {
    if !bytes.starts_with(b"%PDF-") {
        return None;
    }
    match document_information(bytes)?.get(STAMP_KEY)? {
        Value::String(text) => String::from_utf8(text.clone()).ok(),
        _ => None,
    }
}

/// The most cross-reference sections, one per update, followed back from
/// the last to find the document information.
const MOST_SECTIONS: usize = 100;

/// The most bytes the streams the stamp is read through, cross-reference
/// streams and an object stream, may unpack to, all of them together.
const MOST_STREAM: usize = 64 * 1024 * 1024;

/// How deep arrays and dictionaries inside one another are read.
const DEEPEST: usize = 32;

/// A PDF's document information: the dictionary its last trailer names as
/// `/Info`, at the place the newest cross-reference section that lists it
/// gives. A section is a table and the trailer after it, or a
/// cross-reference stream, whose dictionary is the trailer, as a PDF saved
/// with object streams keeps it; the document information may then be
/// inside an object stream.
fn document_information(bytes: &[u8]) -> Option<Dictionary> {
    let mut references = CrossReferences::new(bytes)?;
    let (number, generation) = match references.sections[0].trailer.get(b"Info")? {
        Value::Reference(number, generation) => (*number, *generation),
        _ => return None,
    };
    match references.place(number, generation)? {
        Place::At { offset, .. } => {
            let mut lexer = Lexer::at(bytes, offset);
            lexer.object_heading(number, generation)?;
            lexer.dictionary()
        }
        Place::In { stream } => match references.place(stream, 0)? {
            Place::At { offset, .. } => {
                object_in_stream(bytes, offset, stream, number, &mut references.budget)
            }
            Place::In { .. } => None,
        },
    }
}

/// Where an object in use is: at a place in the bytes, or inside the
/// object stream of that number.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Place {
    At { generation: u64, offset: usize },
    In { stream: u64 },
}

impl Place {
    /// The object's generation: an object in an object stream has 0.
    fn generation(self) -> u64 {
        match self {
            Place::At { generation, .. } => generation,
            Place::In { .. } => 0,
        }
    }
}

/// A PDF's cross-reference sections, read from the newest back as far as
/// a lookup needs, within one budget of unpacked bytes.
struct CrossReferences<'a> {
    bytes: &'a [u8],
    /// The sections read so far, the newest first.
    sections: Vec<Section>,
    /// Where the next older section starts, if there is one.
    older: Option<usize>,
    /// How many more bytes streams may unpack to.
    budget: usize,
}

impl<'a> CrossReferences<'a> {
    /// The PDF's cross-references, with its newest section, the one its
    /// last `startxref` names, read.
    fn new(bytes: &'a [u8]) -> Option<CrossReferences<'a>> {
        let keyword = b"startxref";
        let at = bytes.windows(keyword.len()).rposition(|w| w == keyword)?;
        let mut references = CrossReferences {
            bytes,
            sections: Vec::new(),
            older: Lexer::at(bytes, at + keyword.len()).offset(),
            budget: MOST_STREAM,
        };
        references.read_older()?;
        Some(references)
    }

    /// Reads the next older section. `None` when there is none, or it
    /// cannot be read.
    fn read_older(&mut self) -> Option<()> {
        if self.sections.len() == MOST_SECTIONS {
            return None;
        }
        let section = cross_reference(self.bytes, self.older?, &mut self.budget)?;
        // An update lists only what it changed; the section before it
        // lists the rest.
        self.older = match section.trailer.get(b"Prev") {
            Some(Value::Number(at)) => Some(usize::try_from(*at).ok()?),
            _ => None,
        };
        self.sections.push(section);
        Some(())
    }

    /// Where the newest section that lists object `number` of generation
    /// `generation` in use puts it.
    fn place(&mut self, number: u64, generation: u64) -> Option<Place> {
        let mut i = 0;
        loop {
            if i == self.sections.len() {
                self.read_older()?;
            }
            if let Some(place) = self.sections[i].places.find(number, generation) {
                return Some(place);
            }
            i += 1;
        }
    }
}

/// One cross-reference section: where the objects in use it lists are, and
/// its trailer.
struct Section {
    places: Places,
    trailer: Dictionary,
}

/// Where a section's objects in use are: a table's, each by its number, or
/// a cross-reference stream's rows, looked up as they are asked for.
enum Places {
    Table(Vec<(u64, Place)>),
    Stream(Rows),
}

impl Places {
    fn find(&self, number: u64, generation: u64) -> Option<Place> {
        let place = match self {
            Places::Table(places) => places
                .iter()
                .find(|(n, place)| *n == number && place.generation() == generation)
                .map(|&(_, place)| place),
            Places::Stream(rows) => rows.find(number),
        }?;
        (place.generation() == generation).then_some(place)
    }
}

/// A cross-reference stream's rows: one for each object it lists, in
/// fields of the widths its `/W` gives, for the runs of object numbers its
/// `/Index` gives. Every row is in the data, as checked when it was read.
struct Rows {
    data: Vec<u8>,
    widths: [usize; 3],
    /// Each run's first object number and how many it holds.
    runs: Vec<(u64, u64)>,
}

impl Rows {
    /// Where the row of object `number` puts it, if the stream lists it in
    /// use: type 1, at a place, with its generation, or type 2, in an
    /// object stream. A free row, or one of a type a reader does not know,
    /// lists nothing.
    fn find(&self, number: u64) -> Option<Place> {
        let [type_width, second_width, third_width] = self.widths;
        let width = type_width + second_width + third_width;
        let mut before = 0u64;
        for &(first, count) in &self.runs {
            if let Some(into) = number.checked_sub(first).filter(|into| *into < count) {
                let at = usize::try_from(before + into).ok()?.checked_mul(width)?;
                let row = self.data.get(at..at.checked_add(width)?)?;
                let field = |bytes: &[u8]| bytes.iter().fold(0u64, |n, &b| n << 8 | u64::from(b));
                let (kind, rest) = row.split_at(type_width);
                let (second, third) = rest.split_at(second_width);
                // With no width for the type, every row is of type 1.
                return match if type_width == 0 { 1 } else { field(kind) } {
                    1 => Some(Place::At {
                        generation: field(third),
                        offset: usize::try_from(field(second)).ok()?,
                    }),
                    2 => Some(Place::In {
                        stream: field(second),
                    }),
                    _ => None,
                };
            }
            before += count;
        }
        None
    }
}

/// The cross-reference section at `at`: a table and the trailer after it,
/// or a cross-reference stream, whose data unpacks within `budget`.
fn cross_reference(bytes: &[u8], at: usize, budget: &mut usize) -> Option<Section> {
    let mut lexer = Lexer::at(bytes, at);
    match lexer.next()? {
        Token::Word(b"xref") => {}
        Token::Word(_) => return cross_reference_stream(bytes, at, budget),
        _ => return None,
    }
    let mut places = Vec::new();
    loop {
        match lexer.next()? {
            Token::Word(b"trailer") => break,
            Token::Word(first) => {
                let first = whole_number(first)?;
                let count = lexer.number()?;
                for number in first..first.checked_add(count)? {
                    let offset = lexer.offset()?;
                    let generation = lexer.number()?;
                    if lexer.next()? == Token::Word(b"n") {
                        places.push((number, Place::At { generation, offset }));
                    }
                }
            }
            _ => return None,
        }
    }
    Some(Section {
        places: Places::Table(places),
        trailer: lexer.dictionary()?,
    })
}

/// The cross-reference stream whose object starts at `at`, whose data
/// unpacks within `budget`. Its dictionary is the section's trailer, and
/// its data a row for each object its `/Index` lists, in fields of the
/// widths its `/W` gives, none wider than 8 bytes: the row's type, 1 for
/// an object at a place and 2 for one in an object stream, then the
/// object's place and its generation, or the object stream's number and
/// the object's index in it. A stream whose rows the data does not hold
/// is not read.
fn cross_reference_stream(bytes: &[u8], at: usize, budget: &mut usize) -> Option<Section> {
    let mut lexer = Lexer::at(bytes, at);
    let (_, _) = (lexer.number()?, lexer.number()?);
    if lexer.next()? != Token::Word(b"obj") {
        return None;
    }
    let dictionary = lexer.dictionary()?;
    if dictionary.get(b"Type") != Some(&Value::Name(b"XRef".to_vec())) {
        return None;
    }
    let widths: Vec<usize> = dictionary
        .numbers(b"W")?
        .into_iter()
        .map(|w| usize::try_from(w).ok().filter(|w| *w <= 8))
        .collect::<Option<_>>()?;
    let widths: [usize; 3] = widths.try_into().ok()?;
    let width: usize = widths.iter().sum();
    let index = match dictionary.get(b"Index") {
        None => vec![0, dictionary.number(b"Size")?],
        Some(_) => dictionary.numbers(b"Index")?,
    };
    if width == 0 || index.is_empty() || index.len() % 2 != 0 {
        return None;
    }
    let runs: Vec<(u64, u64)> = index.chunks(2).map(|run| (run[0], run[1])).collect();
    let data = lexer.stream(&dictionary, budget)?;
    let mut rows = 0u64;
    for &(first, count) in &runs {
        first.checked_add(count)?;
        rows = rows.checked_add(count)?;
    }
    if usize::try_from(rows).ok()?.checked_mul(width)? > data.len() {
        return None;
    }
    Some(Section {
        places: Places::Stream(Rows { data, widths, runs }),
        trailer: dictionary,
    })
}

/// The dictionary that is object `number` inside the object stream
/// `stream`, whose object starts at `at` and whose data unpacks within
/// `budget`. The stream's data starts with a number and a place for each
/// object it holds, the places counted from its `/First` byte.
fn object_in_stream(
    bytes: &[u8],
    at: usize,
    stream: u64,
    number: u64,
    budget: &mut usize,
) -> Option<Dictionary> {
    let mut lexer = Lexer::at(bytes, at);
    lexer.object_heading(stream, 0)?;
    let dictionary = lexer.dictionary()?;
    if dictionary.get(b"Type") != Some(&Value::Name(b"ObjStm".to_vec())) {
        return None;
    }
    let count = dictionary.number(b"N")?;
    let first = usize::try_from(dictionary.number(b"First")?).ok()?;
    let data = lexer.stream(&dictionary, budget)?;
    if first > data.len() {
        return None;
    }
    let mut heading = Lexer::at(&data[..first], 0);
    for _ in 0..count {
        let (n, offset) = (heading.number()?, heading.offset()?);
        if n == number {
            return Lexer::at(&data, first.checked_add(offset)?).dictionary();
        }
    }
    None
}

/// The data of a stream, unpacked within `budget`, which it then takes
/// from: by deflate, the filter cross-reference and object streams are
/// packed with, with or without a PNG predictor, or by no filter. A stream
/// packed any other way, or whose predictor's rows its data does not hold,
/// is not read.
fn unpacked(packed: &[u8], dictionary: &Dictionary, budget: &mut usize) -> Option<Vec<u8>> {
    // A filter and its parameters are written alone or in a list of one.
    let alone = |value: Option<&Value>| match value {
        Some(Value::Array(items)) => match &items[..] {
            [one] => Some(one.clone()),
            _ => Some(Value::Other),
        },
        other => other.cloned(),
    };
    let data = match alone(dictionary.get(b"Filter")) {
        None if packed.len() <= *budget => packed.to_vec(),
        None => return None,
        Some(Value::Name(name)) if name == b"FlateDecode" => {
            miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(packed, *budget).ok()?
        }
        Some(_) => return None,
    };
    *budget -= data.len();
    let parameters = match alone(dictionary.get(b"DecodeParms")) {
        None => return Some(data),
        Some(Value::Dictionary(parameters)) => parameters,
        Some(_) => return None,
    };
    let number = |key: &[u8], default: u64| match parameters.get(key) {
        None => Some(default),
        Some(Value::Number(n)) => Some(*n),
        Some(_) => None,
    };
    match number(b"Predictor", 1)? {
        1 => Some(data),
        10..=15 if number(b"BitsPerComponent", 8)? == 8 => {
            let pixel = usize::try_from(number(b"Colors", 1)?)
                .ok()
                .filter(|c| (1..=32).contains(c))?;
            let width = usize::try_from(number(b"Columns", 1)?)
                .ok()?
                .checked_mul(pixel)?;
            png_unpredicted(&data, width, pixel)
        }
        _ => None,
    }
}

/// `data` with its PNG predictors undone: rows of `width` bytes, each after
/// a byte naming how it was predicted, `pixel` bytes to a pixel. Data that
/// is not whole rows of that width is not read, and nothing is set aside
/// for a row before that is known.
fn png_unpredicted(data: &[u8], width: usize, pixel: usize) -> Option<Vec<u8>> {
    let stride = width.checked_add(1)?;
    if width == 0 || data.is_empty() || !data.len().is_multiple_of(stride) {
        return None;
    }
    let mut out: Vec<u8> = Vec::with_capacity(data.len());
    let mut above = vec![0u8; width];
    for row in data.chunks_exact(stride) {
        let (&predictor, row) = row.split_first()?;
        let mut line = vec![0u8; width];
        for i in 0..width {
            let left = if i >= pixel { line[i - pixel] } else { 0 };
            let up = above[i];
            let corner = if i >= pixel { above[i - pixel] } else { 0 };
            let guess = match predictor {
                0 => 0,
                1 => left,
                2 => up,
                3 => ((u16::from(left) + u16::from(up)) / 2) as u8,
                4 => paeth(left, up, corner),
                _ => return None,
            };
            line[i] = row[i].wrapping_add(guess);
        }
        out.extend_from_slice(&line);
        above = line;
    }
    Some(out)
}

/// PNG's Paeth predictor: of the byte to the left, the one above and the
/// one above to the left, the one nearest to `left + up - corner`.
fn paeth(left: u8, up: u8, corner: u8) -> u8 {
    let guess = i16::from(left) + i16::from(up) - i16::from(corner);
    let near = |b: u8| (guess - i16::from(b)).abs();
    if near(left) <= near(up) && near(left) <= near(corner) {
        left
    } else if near(up) <= near(corner) {
        up
    } else {
        corner
    }
}

/// A token of a PDF's syntax, as far as reading its stamp needs.
#[derive(Debug, PartialEq)]
enum Token<'a> {
    /// `<<`
    DictionaryStart,
    /// `>>`
    DictionaryEnd,
    /// `[`
    ArrayStart,
    /// `]`
    ArrayEnd,
    /// A name, such as `/Info`, without its slash.
    Name(&'a [u8]),
    /// A string, literal `(...)` or hexadecimal `<...>`, its escapes undone.
    String(Vec<u8>),
    /// Anything else: a number or a keyword, such as `obj` or `R`.
    Word(&'a [u8]),
}

/// A value in a dictionary, as far as reading a stamp needs.
#[derive(Clone, Debug, PartialEq)]
enum Value {
    String(Vec<u8>),
    /// An indirect reference, `number generation R`.
    Reference(u64, u64),
    /// A whole number.
    Number(u64),
    /// A name, without its slash.
    Name(Vec<u8>),
    Array(Vec<Value>),
    Dictionary(Dictionary),
    /// Anything else: a keyword, or a number that is not whole.
    Other,
}

/// A dictionary's keys, without their slashes, and values.
#[derive(Clone, Debug, PartialEq)]
struct Dictionary(Vec<(Vec<u8>, Value)>);

impl Dictionary {
    fn get(&self, key: &[u8]) -> Option<&Value> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// The whole number under `key`.
    fn number(&self, key: &[u8]) -> Option<u64> {
        match self.get(key)? {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// The list of whole numbers under `key`.
    fn numbers(&self, key: &[u8]) -> Option<Vec<u64>> {
        match self.get(key)? {
            Value::Array(items) => items
                .iter()
                .map(|item| match item {
                    Value::Number(n) => Some(*n),
                    _ => None,
                })
                .collect(),
            _ => None,
        }
    }
}

/// Reads a PDF's tokens in turn from a place in its bytes.
struct Lexer<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Lexer<'a> {
    fn at(bytes: &'a [u8], at: usize) -> Lexer<'a> {
        Lexer { bytes, at }
    }

    /// The next token, past any white space and comments.
    fn next(&mut self) -> Option<Token<'a>> {
        loop {
            match *self.bytes.get(self.at)? {
                b'%' => {
                    while !matches!(self.bytes.get(self.at), None | Some(b'\r' | b'\n')) {
                        self.at += 1;
                    }
                }
                b if is_white(b) => self.at += 1,
                _ => break,
            }
        }
        let rest = &self.bytes[self.at..];
        let (token, length) = match rest {
            [b'<', b'<', ..] => (Token::DictionaryStart, 2),
            [b'>', b'>', ..] => (Token::DictionaryEnd, 2),
            [b'[', ..] => (Token::ArrayStart, 1),
            [b']', ..] => (Token::ArrayEnd, 1),
            [b'(', ..] => {
                let (text, length) = literal_string(rest)?;
                (Token::String(text), length)
            }
            [b'<', ..] => {
                let (text, length) = hex_string(rest)?;
                (Token::String(text), length)
            }
            [b'/', name @ ..] => {
                let length = word_length(name);
                (Token::Name(&name[..length]), 1 + length)
            }
            _ => {
                // A stray delimiter, such as `)`, is a word of its own.
                let length = word_length(rest).max(1);
                (Token::Word(&rest[..length]), length)
            }
        };
        self.at += length;
        Some(token)
    }

    /// The next token, a whole number.
    fn number(&mut self) -> Option<u64> {
        match self.next()? {
            Token::Word(word) => whole_number(word),
            _ => None,
        }
    }

    /// The next token, a whole number that is a place in the bytes.
    fn offset(&mut self) -> Option<usize> {
        usize::try_from(self.number()?).ok()
    }

    /// The heading of object `number` of generation `generation`, `number
    /// generation obj`, or `None`.
    fn object_heading(&mut self, number: u64, generation: u64) -> Option<()> {
        let heading = (self.number()?, self.number()?, self.next()?);
        (heading == (number, generation, Token::Word(b"obj"))).then_some(())
    }

    /// The dictionary that starts at the next token.
    fn dictionary(&mut self) -> Option<Dictionary> {
        if self.next()? != Token::DictionaryStart {
            return None;
        }
        self.rest_of_dictionary(0)
    }

    /// The rest of a dictionary whose `<<` has been read, inside `depth`
    /// others.
    fn rest_of_dictionary(&mut self, depth: usize) -> Option<Dictionary> {
        let mut entries = Vec::new();
        loop {
            match self.next()? {
                Token::DictionaryEnd => return Some(Dictionary(entries)),
                Token::Name(key) => {
                    let value = self.value(depth)?;
                    entries.push((key.to_vec(), value));
                }
                _ => return None,
            }
        }
    }

    /// The value that starts at the next token, inside `depth` arrays and
    /// dictionaries.
    fn value(&mut self, depth: usize) -> Option<Value> {
        Some(match self.next()? {
            Token::String(text) => Value::String(text),
            Token::Word(word) => match whole_number(word) {
                Some(number) => {
                    let mut ahead = Lexer::at(self.bytes, self.at);
                    match (ahead.number(), ahead.next()) {
                        (Some(generation), Some(Token::Word(b"R"))) => {
                            self.at = ahead.at;
                            Value::Reference(number, generation)
                        }
                        _ => Value::Number(number),
                    }
                }
                None => Value::Other,
            },
            Token::Name(name) => Value::Name(name.to_vec()),
            Token::DictionaryStart if depth < DEEPEST => {
                Value::Dictionary(self.rest_of_dictionary(depth + 1)?)
            }
            Token::ArrayStart if depth < DEEPEST => {
                let mut items = Vec::new();
                loop {
                    let mut ahead = Lexer::at(self.bytes, self.at);
                    if ahead.next()? == Token::ArrayEnd {
                        self.at = ahead.at;
                        break Value::Array(items);
                    }
                    items.push(self.value(depth + 1)?);
                }
            }
            Token::DictionaryStart | Token::ArrayStart | Token::DictionaryEnd | Token::ArrayEnd => {
                return None
            }
        })
    }

    /// The data of the stream whose dictionary, `dictionary`, has just been
    /// read, unpacked within `budget`: the bytes after the keyword `stream`
    /// and its line break, as many as its `/Length` says, or up to
    /// `endstream` when its length is not written as a number there.
    fn stream(&mut self, dictionary: &Dictionary, budget: &mut usize) -> Option<Vec<u8>> {
        if self.next()? != Token::Word(b"stream") {
            return None;
        }
        let rest = match self.bytes.get(self.at..)? {
            [b'\r', b'\n', rest @ ..] | [b'\n', rest @ ..] => rest,
            _ => return None,
        };
        let packed = match dictionary.number(b"Length") {
            Some(length) => rest.get(..usize::try_from(length).ok()?)?,
            None => {
                let end = rest.windows(9).position(|w| w == b"endstream")?;
                let mut packed = &rest[..end];
                while let [before @ .., b'\r' | b'\n'] = packed {
                    packed = before;
                }
                packed
            }
        };
        unpacked(packed, dictionary, budget)
    }
}

/// Whether `b` is white space in a PDF.
fn is_white(b: u8) -> bool {
    matches!(b, b'\0' | b'\t' | b'\n' | b'\x0c' | b'\r' | b' ')
}

/// How many bytes at the start of `bytes` run up to white space or a
/// delimiter.
fn word_length(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .position(|&b| is_white(b) || b"()<>[]{}/%".contains(&b))
        .unwrap_or(bytes.len())
}

/// `word` as a whole number, if it is one.
fn whole_number(word: &[u8]) -> Option<u64> {
    if word.is_empty() || !word.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(word).ok()?.parse().ok()
}

/// The bytes of the PDF hexadecimal string `<...>` at the start of
/// `bytes`, and how many bytes it takes.
fn hex_string(bytes: &[u8]) -> Option<(Vec<u8>, usize)> {
    let end = bytes.iter().position(|&b| b == b'>')?;
    let digits = bytes[1..end]
        .iter()
        .filter(|&&b| !is_white(b))
        .map(|&b| char::from(b).to_digit(16).map(|d| d as u8))
        .collect::<Option<Vec<u8>>>()?;
    // A last digit alone is followed by a 0.
    let text = digits
        .chunks(2)
        .map(|pair| pair[0] << 4 | pair.get(1).copied().unwrap_or(0))
        .collect();
    Some((text, end + 1))
}

/// The bytes of the PDF literal string `(...)` at the start of `bytes`,
/// its escapes undone, and how many bytes it takes.
fn literal_string(bytes: &[u8]) -> Option<(Vec<u8>, usize)> {
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
                    return Some((out, i + 1));
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
pub(crate) mod tests {
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

    /// A one-page PDF someone else wrote, whose page shows `shown` as text
    /// and whose document information holds `info`.
    pub(crate) fn their_pdf(shown: &str, info: &[(&str, &str)]) -> Vec<u8> {
        let [catalog, pages, page, content, font, about] = [1, 2, 3, 4, 5, 6].map(Ref::new);
        let mut writer = Pdf::new();
        writer.catalog(catalog).pages(pages);
        writer.pages(pages).kids([page]).count(1);
        {
            let mut sheet = writer.page(page);
            sheet
                .media_box(Rect::new(0.0, 0.0, 595.0, 842.0))
                .parent(pages)
                .contents(content);
            sheet.resources().fonts().pair(Name(b"F1"), font);
        }
        writer.type1_font(font).base_font(Name(b"Helvetica"));
        let text = format!("BT /F1 10 Tf 20 800 Td ({shown}) Tj ET");
        writer.stream(content, text.as_bytes());
        {
            let mut dictionary = writer.document_info(about);
            for (key, value) in info {
                dictionary.pair(Name(key.as_bytes()), Str(value.as_bytes()));
            }
        }
        writer.finish()
    }

    #[test]
    fn the_stamp_is_read_only_from_the_document_information() {
        let stamp = r#"{"stamp":1,"produced_by":"agentdrafter 0.0.0","sources":[]}"#;
        let example = format!("/AgentDrafter ({stamp})");
        // A page that shows a stamp as text, as a page about AgentDrafter
        // might: its bytes hold the key and the string, but its document
        // information holds no stamp.
        let shown = their_pdf(&example, &[("Title", "a manual")]);
        assert!(shown
            .windows(example.len())
            .any(|w| w == example.as_bytes()));
        let info = document_information(&shown).unwrap();
        assert_eq!(
            info.get(b"Title"),
            Some(&Value::String(b"a manual".to_vec()))
        );
        assert_eq!(stamp_in(&shown), None);
        // Nor is a stamp inside another entry's value one.
        let inside = their_pdf("a sheet", &[("Title", &example)]);
        assert_eq!(stamp_in(&inside), None);
        // Under its key in the document information, it is.
        let stamped = their_pdf("a sheet", &[("Title", "a"), ("AgentDrafter", stamp)]);
        assert_eq!(stamp_in(&stamped).as_deref(), Some(stamp));
    }

    #[test]
    fn a_stamp_is_still_read_after_a_later_update_of_the_pdf() {
        let stamp = r#"{"stamp":1,"produced_by":"agentdrafter 0.0.0","sources":[]}"#;
        let mut bytes = their_pdf("a sheet", &[("AgentDrafter", stamp)]);
        // An update appended as a reader that saves in place appends it: a
        // section listing only what changed, here nothing, and a trailer
        // that points back to the section before.
        let tail = String::from_utf8_lossy(&bytes[bytes.len() - 40..]).into_owned();
        let previous: usize = tail
            .split_whitespace()
            .rev()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let section = bytes.len();
        bytes.extend(
            format!(
                "\nxref\n0 0\ntrailer\n<< /Size 7 /Root 1 0 R /Info 6 0 R /Prev {previous} >>\n\
                 startxref\n{section}\n%%EOF\n"
            )
            .as_bytes(),
        );
        assert_eq!(stamp_in(&bytes).as_deref(), Some(stamp));
    }

    #[test]
    fn each_png_predictor_is_undone() {
        // Two bytes to a pixel, rows of four bytes, the first row
        // [1, 2, 3, 4] and the second [5, 7, 9, 11], each row written once
        // with each of the five predictors: none, sub, up, average, Paeth.
        let first = [1u8, 2, 3, 4];
        let rows: [(u8, [u8; 4]); 5] = [
            (0, [5, 7, 9, 11]),
            // Less the byte a pixel to the left.
            (1, [5, 7, 4, 4]),
            // Less the byte above.
            (2, [4, 5, 6, 7]),
            // Less the mean of left and above, rounded down: 5 - 1 / 2,
            // 7 - 2 / 2, 9 - (5 + 3) / 2, 11 - (7 + 4) / 2.
            (3, [5, 6, 5, 6]),
            // Less Paeth's choice: above at first, then left (5 and 7).
            (4, [4, 5, 4, 4]),
        ];
        for (predictor, row) in rows {
            let mut data = vec![0];
            data.extend(first);
            data.push(predictor);
            data.extend(row);
            assert_eq!(
                png_unpredicted(&data, 4, 2).unwrap(),
                [1, 2, 3, 4, 5, 7, 9, 11],
                "predictor {predictor}"
            );
        }
        assert_eq!(png_unpredicted(&[5, 1, 2, 3, 4], 4, 2), None);
        assert_eq!(png_unpredicted(&[0, 1, 2], 4, 2), None);
    }

    /// The PDF of about 200 bytes the review of #57 crashed `stale` with: a
    /// cross-reference stream whose PNG predictor's `/Columns` is the
    /// largest whole number.
    pub(crate) fn crashing_pdf() -> Vec<u8> {
        let data = miniz_oxide::deflate::compress_to_vec_zlib(&[2, 1, 0, 9, 0], 6);
        let mut bytes = format!(
            "%PDF-1.7\n1 0 obj\n<< /Type /XRef /Size 1 /W [1 1 1] /Info 1 0 R /Filter \
             /FlateDecode /DecodeParms << /Predictor 12 /Columns 18446744073709551615 >> \
             /Length {} >>\nstream\n",
            data.len()
        )
        .into_bytes();
        bytes.extend(data);
        bytes.extend(b"\nendstream\nendobj\nstartxref\n9\n%%EOF\n");
        bytes
    }

    /// A PDF of an object stream holding its document information, object
    /// 2, `<< /AgentDrafter (stamp) >>`, and a cross-reference stream that
    /// lists it, in rows of the widths `/W [1 2 1]`, three bytes a field.
    /// `objects` is written into the object stream's dictionary in place of
    /// its true `/N` and `/First` when given, and `xref` into the
    /// cross-reference stream's; `columns`, when given, sends the rows
    /// through PNG's predictor with that `/Columns`.
    fn object_stream_pdf(objects: Option<&str>, xref: &str, columns: Option<&str>) -> Vec<u8> {
        let inside = b"2 0 << /AgentDrafter (stamp) >>";
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(inside, 6);
        let counts = objects.map_or_else(|| "/N 1 /First 4".to_owned(), str::to_owned);
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let stream_at = bytes.len();
        bytes.extend(
            format!(
                "1 0 obj\n<< /Type /ObjStm {counts} /Filter /FlateDecode /Length {} >>\nstream\n",
                packed.len()
            )
            .as_bytes(),
        );
        bytes.extend(&packed);
        bytes.extend(b"\nendstream\nendobj\n");
        let xref_at = bytes.len();
        let rows: [[u8; 4]; 4] = [
            [0, 0, 0, 0],
            [1, 0, stream_at as u8, 0],
            [2, 0, 1, 0],
            [1, (xref_at >> 8) as u8, xref_at as u8, 0],
        ];
        let (data, parameters) = match columns {
            None => (rows.concat(), String::new()),
            Some(columns) => (
                rows.iter()
                    .flat_map(|row| [&[0][..], row].concat())
                    .collect(),
                format!("/DecodeParms << /Predictor 12 /Columns {columns} >>"),
            ),
        };
        let data = miniz_oxide::deflate::compress_to_vec_zlib(&data, 6);
        bytes.extend(
            format!(
                "3 0 obj\n<< /Type /XRef /Size 4 /Info 2 0 R {xref} /Filter /FlateDecode \
                 {parameters} /Length {} >>\nstream\n",
                data.len()
            )
            .as_bytes(),
        );
        bytes.extend(data);
        bytes.extend(format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
        bytes
    }

    #[test]
    fn a_hostile_pdf_reads_as_not_stamped_and_never_panics() {
        // The review's crash: the predictor's rows were set aside before
        // they were checked against the data.
        let crashing = crashing_pdf();
        assert!(crashing.len() < 250, "{}", crashing.len());
        assert_eq!(stamp_in(&crashing), None);

        // The same PDF, well formed, reads, with its predictor and without.
        let fields = "/W [1 2 1]";
        assert_eq!(
            stamp_in(&object_stream_pdf(None, fields, None)).as_deref(),
            Some("stamp")
        );
        assert_eq!(
            stamp_in(&object_stream_pdf(None, fields, Some("4"))).as_deref(),
            Some("stamp")
        );
        // Each number a hostile PDF could make huge, or make disagree with
        // the data it describes, makes it read as not stamped.
        let huge = "18446744073709551615";
        for columns in [huge, "4294967295", "3", "5", "0"] {
            let pdf = object_stream_pdf(None, fields, Some(columns));
            assert_eq!(stamp_in(&pdf), None, "/Columns {columns}");
        }
        for broken in [
            format!("/Colors {huge} /Columns"),
            format!("/BitsPerComponent {huge} /Columns"),
            "/Colors 33 /Columns".to_owned(),
        ] {
            let pdf = object_stream_pdf(None, fields, Some(&format!("4 {broken} 4")));
            assert_eq!(stamp_in(&pdf), None, "{broken}");
        }
        for xref in [
            format!("/W [1 {huge} 1]"),
            "/W [8 8 8]".to_owned(),
            "/W [1 2]".to_owned(),
            "/W [0 0 0]".to_owned(),
            format!("/W [1 2 1] /Index [0 {huge}]"),
            format!("/W [1 2 1] /Index [{huge} 4]"),
            "/W [1 2 1] /Index [0 1000000000000]".to_owned(),
            "/W [1 2 1] /Index [0 5]".to_owned(),
            "/W [1 2 1] /Index [0 2 2]".to_owned(),
            "/W [1 2 1] /Index []".to_owned(),
        ] {
            assert_eq!(
                stamp_in(&object_stream_pdf(None, &xref, None)),
                None,
                "{xref}"
            );
        }
        for objects in [
            format!("/N {huge} /First {huge}"),
            format!("/N 1 /First {huge}"),
            "/N 1 /First 99".to_owned(),
            format!("/N {huge} /First 0"),
        ] {
            let pdf = object_stream_pdf(Some(&objects), fields, None);
            assert_eq!(stamp_in(&pdf), None, "{objects}");
        }
    }

    #[test]
    fn a_stream_packed_another_way_is_not_read() {
        let stream = |dictionary: &str, data: &[u8]| {
            let mut bytes =
                format!("<< {dictionary} /Length {} >>\nstream\n", data.len()).into_bytes();
            bytes.extend(data);
            bytes.extend(b"\nendstream");
            let mut lexer = Lexer::at(&bytes, 0);
            let dictionary = lexer.dictionary().unwrap();
            let mut budget = MOST_STREAM;
            lexer.stream(&dictionary, &mut budget)
        };
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(b"1 2 3", 6);
        assert_eq!(stream("", b"1 2 3").unwrap(), b"1 2 3");
        assert_eq!(stream("/Filter /FlateDecode", &packed).unwrap(), b"1 2 3");
        assert_eq!(stream("/Filter [/FlateDecode]", &packed).unwrap(), b"1 2 3");
        assert_eq!(stream("/Filter /LZWDecode", b"1 2 3"), None);
        assert_eq!(
            stream("/Filter [/ASCIIHexDecode /FlateDecode]", &packed),
            None
        );
        assert_eq!(
            stream(
                "/Filter /FlateDecode /DecodeParms << /Predictor 2 >>",
                &packed
            ),
            None
        );
    }

    #[test]
    fn literal_strings_undo_their_escapes() {
        assert_eq!(
            literal_string(br"(a\(b\) (c) \\ \101\n) tail").unwrap(),
            (b"a(b) (c) \\ A\n".to_vec(), 22)
        );
        assert_eq!(literal_string(b"(unclosed"), None);
    }

    #[test]
    fn characters_get_codes_256_to_a_font_in_the_order_first_used() {
        let run = |text: &str| TextRun {
            object: 0,
            part: None,
            drawn: None,
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
