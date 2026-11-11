//! Reads AutoCAD SHX fonts, enough to measure text.
//!
//! An SHX font is a compiled shape file: each character is a shape, a short
//! program of pen moves, arcs and pen-up and pen-down codes, in units where
//! the capitals stand `above` units tall. The shape format is documented in
//! AutoCAD's customization guide ("Shapes and shape fonts"). Two file
//! layouts are read: the older `AutoCAD-86 shapes 1.0` and `1.1`, numbered
//! by byte, and `AutoCAD-86 unifont 1.0`, numbered by Unicode code point,
//! which AutoCAD's own text fonts use. Big fonts, for Asian scripts, are not
//! read.
//!
//! Running a shape gives what measuring needs: how far the pen moved and
//! the box around every stroke drawn with the pen down. Run again with a
//! trace, it gives the strokes themselves, which the drawing pipeline
//! ([`crate::draw`]) draws with the text's pen.

use std::collections::HashMap;
use std::f64::consts::{FRAC_PI_4, PI, TAU};

use super::fonts::{Glyph, Rect};
use crate::draw::curves::arc_steps;

/// The deepest a shape may nest subshapes before it is taken as a loop.
const DEEPEST_SUBSHAPE: u8 = 8;

/// The deepest the pen's position stack goes, as in AutoCAD.
const DEEPEST_STACK: usize = 4;

/// The 16 directions a one-byte vector can point in, east first and
/// anticlockwise. The longer of the two components is 1.
const DIRECTIONS: [(f64, f64); 16] = [
    (1.0, 0.0),
    (1.0, 0.5),
    (1.0, 1.0),
    (0.5, 1.0),
    (0.0, 1.0),
    (-0.5, 1.0),
    (-1.0, 1.0),
    (-1.0, 0.5),
    (-1.0, 0.0),
    (-1.0, -0.5),
    (-1.0, -1.0),
    (-0.5, -1.0),
    (0.0, -1.0),
    (0.5, -1.0),
    (1.0, -1.0),
    (1.0, -0.5),
];

/// An SHX font, read.
#[derive(Debug)]
pub(crate) struct ShxFont {
    /// How tall the capitals stand, in shape units.
    above: f64,
    /// How far the descenders reach below the baseline, in shape units.
    below: f64,
    /// Whether shapes are numbered by Unicode code point with two-byte
    /// subshape numbers, as in a unifont.
    unifont: bool,
    /// Each shape's program, by shape number, without its name.
    shapes: HashMap<u32, Vec<u8>>,
}

impl ShxFont {
    /// Reads the bytes of an SHX font. The error says what is wrong.
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, String> {
        let end_of_header = bytes
            .iter()
            .position(|&b| b == 0x1a)
            .filter(|&at| at < 40)
            .ok_or("it does not start with an SHX header")?;
        let header = String::from_utf8_lossy(&bytes[..end_of_header]);
        let header = header.trim_end();
        let mut reader = Reader {
            bytes,
            at: end_of_header + 1,
        };
        match header {
            "AutoCAD-86 unifont 1.0" => Self::parse_unifont(&mut reader),
            "AutoCAD-86 shapes 1.0" | "AutoCAD-86 shapes 1.1" => Self::parse_shapes(&mut reader),
            "AutoCAD-86 bigfont 1.0" => {
                Err("it is a big font, which AgentDrafter does not read".to_owned())
            }
            other => Err(format!(
                "its header `{other}` is not one AgentDrafter reads"
            )),
        }
    }

    fn parse_unifont(reader: &mut Reader) -> Result<Self, String> {
        let count = reader.u32()?;
        let info_length = usize::from(reader.u16()?);
        let info = reader.take(info_length)?;
        let (above, below) = font_info(info)?;
        let mut shapes = HashMap::new();
        // The count includes the font's own information, read above.
        for _ in 1..count {
            if reader.at >= reader.bytes.len() {
                break;
            }
            let number = u32::from(reader.u16()?);
            let length = usize::from(reader.u16()?);
            let definition = reader.take(length)?;
            shapes.insert(number, without_name(definition).to_vec());
        }
        Ok(Self {
            above,
            below,
            unifont: true,
            shapes,
        })
    }

    fn parse_shapes(reader: &mut Reader) -> Result<Self, String> {
        let _first = reader.u16()?;
        let _last = reader.u16()?;
        let count = usize::from(reader.u16()?);
        let mut index = Vec::with_capacity(count);
        for _ in 0..count {
            index.push((reader.u16()?, usize::from(reader.u16()?)));
        }
        let mut info = None;
        let mut shapes = HashMap::new();
        for (number, length) in index {
            let definition = reader.take(length)?;
            if number == 0 {
                info = Some(font_info(definition)?);
            } else {
                shapes.insert(u32::from(number), without_name(definition).to_vec());
            }
        }
        let (above, below) =
            info.ok_or("it has no shape 0, so it is a shape file rather than a font")?;
        Ok(Self {
            above,
            below,
            unifont: false,
            shapes,
        })
    }

    /// How far the descenders reach below the baseline, in units where the
    /// capitals stand 1 tall, as the font's information gives it.
    pub(crate) fn descent(&self) -> f64 {
        self.below / self.above
    }

    /// The glyph for `c`, in units where the capitals stand 1 tall. A
    /// character the font has no shape for draws as `?`, as AutoCAD draws
    /// it; none when the font has no `?` either.
    pub(crate) fn glyph(&self, c: char) -> Option<Glyph> {
        let mut pen = Pen::default();
        self.run(self.shape_for(c), &mut pen, 0).ok()?;
        let scale = 1.0 / self.above;
        Some(Glyph {
            advance: (pen.x * scale, pen.y * scale),
            ink: pen.ink.map(|r| r.scaled(scale)),
        })
    }

    /// The strokes `c` draws and where the pen ends, in units where the
    /// capitals stand 1 tall: each stroke a run of points drawn with the pen
    /// down, arcs as chords that stray at most `tolerance` from them. A run
    /// of one point is a dot. A character the font has no shape for draws as
    /// `?`; none when the font has no `?` either.
    pub(crate) fn strokes(&self, c: char, tolerance: f64) -> Option<((f64, f64), Strokes)> {
        let mut pen = Pen {
            trace: Some(Trace {
                runs: Vec::new(),
                open: false,
                tolerance: tolerance * self.above,
            }),
            ..Pen::default()
        };
        self.run(self.shape_for(c), &mut pen, 0).ok()?;
        let scale = 1.0 / self.above;
        let runs = pen.trace.map_or_else(Vec::new, |trace| trace.runs);
        let strokes = runs
            .into_iter()
            .map(|run| {
                run.into_iter()
                    .map(|(x, y)| [x * scale, y * scale])
                    .collect()
            })
            .collect();
        Some(((pen.x * scale, pen.y * scale), strokes))
    }

    /// The shape that draws `c`: its own, or `?` when the font has none.
    fn shape_for(&self, c: char) -> u32 {
        let number = u32::from(c);
        if self.shapes.contains_key(&number) {
            number
        } else {
            u32::from('?')
        }
    }

    /// Runs shape `number`'s program on `pen`, in horizontal text.
    fn run(&self, number: u32, pen: &mut Pen, depth: u8) -> Result<(), String> {
        if depth > DEEPEST_SUBSHAPE {
            return Err("subshapes nest too deep".to_owned());
        }
        let program = self
            .shapes
            .get(&number)
            .ok_or_else(|| format!("no shape {number}"))?;
        let mut code = Reader {
            bytes: program,
            at: 0,
        };
        while code.at < program.len() {
            let op = code.u8()?;
            match op {
                0 => break,
                1 => {
                    // AutoCAD counts the point where the pen comes down as
                    // drawn, even when nothing follows.
                    pen.down = true;
                    pen.mark(pen.x, pen.y);
                    pen.lift();
                    pen.trace_to(pen.x, pen.y);
                }
                2 => {
                    pen.down = false;
                    pen.lift();
                }
                3 => {
                    let divisor = code.u8()?;
                    if divisor == 0 {
                        return Err("a scale divided by 0".to_owned());
                    }
                    pen.scale /= f64::from(divisor);
                }
                4 => pen.scale *= f64::from(code.u8()?),
                5 => {
                    if pen.stack.len() < DEEPEST_STACK {
                        pen.stack.push((pen.x, pen.y));
                    }
                }
                6 => {
                    if let Some((x, y)) = pen.stack.pop() {
                        pen.x = x;
                        pen.y = y;
                        // The pen jumps without drawing.
                        pen.lift();
                    }
                }
                7 => {
                    let sub = if self.unifont {
                        u32::from(code.u16_big_endian()?)
                    } else {
                        u32::from(code.u8()?)
                    };
                    self.run(sub, pen, depth + 1)?;
                }
                8 => {
                    let (dx, dy) = (code.i8()?, code.i8()?);
                    pen.line(f64::from(dx), f64::from(dy));
                }
                9 => loop {
                    let (dx, dy) = (code.i8()?, code.i8()?);
                    if dx == 0 && dy == 0 {
                        break;
                    }
                    pen.line(f64::from(dx), f64::from(dy));
                },
                10 => {
                    let radius = f64::from(code.u8()?);
                    let (clockwise, start, octants) = octants(code.u8()?);
                    let span = if octants == 0 { 8 } else { octants };
                    let start = f64::from(start) * FRAC_PI_4;
                    let sweep = f64::from(span) * FRAC_PI_4;
                    pen.arc(radius, start, if clockwise { -sweep } else { sweep });
                }
                11 => {
                    let start_offset = f64::from(code.u8()?);
                    let end_offset = f64::from(code.u8()?);
                    let radius = f64::from(code.u8()?) * 256.0 + f64::from(code.u8()?);
                    let (clockwise, start, octants) = octants(code.u8()?);
                    let (from, to) =
                        fractional_arc(clockwise, start, octants, start_offset, end_offset);
                    pen.arc(radius, from, to - from);
                }
                12 => {
                    let (dx, dy, bulge) = (code.i8()?, code.i8()?, code.i8()?);
                    pen.bulge(f64::from(dx), f64::from(dy), f64::from(bulge));
                }
                13 => loop {
                    let (dx, dy) = (code.i8()?, code.i8()?);
                    if dx == 0 && dy == 0 {
                        break;
                    }
                    let bulge = code.i8()?;
                    pen.bulge(f64::from(dx), f64::from(dy), f64::from(bulge));
                },
                // The next command is for vertical text only.
                14 => code.at += self.command_length(program, code.at)?,
                vector => {
                    let length = f64::from(vector >> 4);
                    let (dx, dy) = DIRECTIONS[usize::from(vector & 0x0f)];
                    pen.line(dx * length, dy * length);
                }
            }
        }
        Ok(())
    }

    /// How many bytes the command at `at` takes, with its operands.
    fn command_length(&self, program: &[u8], at: usize) -> Result<usize, String> {
        let op = *program.get(at).ok_or("a shape ends inside a command")?;
        let operands = match op {
            0..=2 | 5 | 6 => 0,
            3 | 4 => 1,
            7 => {
                if self.unifont {
                    2
                } else {
                    1
                }
            }
            8 | 10 => 2,
            11 => 5,
            12 => 3,
            9 | 13 => {
                let step = if op == 9 { 2 } else { 3 };
                let mut length = 0;
                loop {
                    let pair = program
                        .get(at + 1 + length..at + 3 + length)
                        .ok_or("a shape ends inside a command")?;
                    if pair == [0, 0] {
                        break length + 2;
                    }
                    length += step;
                }
            }
            14 => 1 + self.command_length(program, at + 1)?,
            _ => 0,
        };
        Ok(1 + operands)
    }
}

/// The capitals' height and the descent from a font's information, which
/// follows its name: `above`, `below`, then mode bytes.
fn font_info(info: &[u8]) -> Result<(f64, f64), String> {
    let rest = without_name(info);
    match rest {
        [above, below, ..] if *above > 0 => Ok((f64::from(*above), f64::from(*below))),
        _ => Err("its font information gives no height for capitals".to_owned()),
    }
}

/// A shape's definition after its name, which ends with a zero byte.
fn without_name(definition: &[u8]) -> &[u8] {
    match definition.iter().position(|&b| b == 0) {
        Some(end) => &definition[end + 1..],
        None => definition,
    }
}

/// An arc's octant byte: its top bit set for a clockwise arc, then the
/// starting octant and the number of octants (0 for all 8), as the
/// shape's `(-)0SC` gives them. The sign is a bit of its own, not a two's
/// complement: AutoCAD 2026 draws `0xC4` as four octants clockwise from
/// octant 4.
fn octants(byte: u8) -> (bool, u8, u8) {
    (byte & 0x80 != 0, (byte >> 4) & 0x07, byte & 0x07)
}

/// Where a fractional arc starts and ends, in radians: it starts
/// `start_offset` 256ths of an octant into octant `start` and ends
/// `end_offset` 256ths into the last of its `octants` octants, or at that
/// octant's far boundary when `end_offset` is 0. AutoCAD takes each offset
/// to the nearest whole degree: an offset of 108 is 19 degrees, not 18.98.
fn fractional_arc(
    clockwise: bool,
    start: u8,
    octants: u8,
    start_offset: f64,
    end_offset: f64,
) -> (f64, f64) {
    let direction = if clockwise { -1.0 } else { 1.0 };
    let degrees = |offset: f64| (offset * 45.0 / 256.0).round();
    let span = f64::from(if octants == 0 { 8 } else { octants });
    let start = f64::from(start) * 45.0;
    let from = start + direction * degrees(start_offset);
    let to = if end_offset == 0.0 {
        start + direction * span * 45.0
    } else {
        start + direction * ((span - 1.0) * 45.0 + degrees(end_offset))
    };
    (from.to_radians(), to.to_radians())
}

/// A glyph's strokes: runs of points drawn with the pen down.
pub(crate) type Strokes = Vec<Vec<[f64; 2]>>;

/// The pen as a shape runs.
#[derive(Debug)]
struct Pen {
    x: f64,
    y: f64,
    down: bool,
    scale: f64,
    stack: Vec<(f64, f64)>,
    ink: Option<Rect>,
    /// The strokes drawn, when they are wanted as well as the box.
    trace: Option<Trace>,
}

/// The strokes a shape draws, in shape units.
#[derive(Debug)]
struct Trace {
    runs: Vec<Vec<(f64, f64)>>,
    /// Whether the last run goes on from where the pen is.
    open: bool,
    /// How far a chord may stray from the arc it stands for.
    tolerance: f64,
}

impl Default for Pen {
    fn default() -> Self {
        // A shape starts with the pen down at its origin.
        Self {
            x: 0.0,
            y: 0.0,
            down: true,
            scale: 1.0,
            stack: Vec::new(),
            ink: None,
            trace: None,
        }
    }
}

impl Pen {
    fn mark(&mut self, x: f64, y: f64) {
        let point = Rect::point(x, y);
        self.ink = Some(match self.ink {
            Some(ink) => ink.union(point),
            None => point,
        });
    }

    /// Ends the stroke being traced: the next point starts a new one.
    fn lift(&mut self) {
        if let Some(trace) = &mut self.trace {
            trace.open = false;
        }
    }

    /// Traces a stroke on to `(x, y)`, starting a new one if none is open.
    fn trace_to(&mut self, x: f64, y: f64) {
        let Some(trace) = &mut self.trace else {
            return;
        };
        match trace.runs.last_mut() {
            Some(run) if trace.open => {
                if run.last() != Some(&(x, y)) {
                    run.push((x, y));
                }
            }
            _ => {
                trace.runs.push(vec![(x, y)]);
                trace.open = true;
            }
        }
    }

    /// Moves by `(dx, dy)` shape units at the current scale, drawing if the pen is down.
    fn line(&mut self, dx: f64, dy: f64) {
        let (x, y) = (self.x + dx * self.scale, self.y + dy * self.scale);
        if self.down {
            self.mark(self.x, self.y);
            self.mark(x, y);
            self.trace_to(self.x, self.y);
            self.trace_to(x, y);
        }
        self.x = x;
        self.y = y;
    }

    /// Draws an arc of `radius` from the pen, which sits on the circle at
    /// angle `start`, sweeping `sweep` radians (anticlockwise when positive).
    ///
    /// The arc's box is taken as AutoCAD takes it, which can reach past the
    /// arc: an arc of up to half a turn is bounded by its chord and the
    /// chord moved out to the arc's middle, and a longer one by its whole
    /// circle. A half circle that starts on an axis comes out exact; a
    /// quarter does not. (Seen in AutoCAD 2026's `textbox` of `isocp.shx`.)
    fn arc(&mut self, radius: f64, start: f64, sweep: f64) {
        let radius = radius * self.scale;
        let (cx, cy) = (self.x - radius * start.cos(), self.y - radius * start.sin());
        let on_circle = |angle: f64| (cx + radius * angle.cos(), cy + radius * angle.sin());
        if self.down {
            if sweep.abs() > PI + 1e-12 {
                self.mark(cx - radius, cy - radius);
                self.mark(cx + radius, cy + radius);
            } else {
                let (x0, y0) = (self.x, self.y);
                let (x1, y1) = on_circle(start + sweep);
                let (mx, my) = on_circle(start + sweep / 2.0);
                // From the chord's middle out to the arc's middle.
                let (dx, dy) = (mx - (x0 + x1) / 2.0, my - (y0 + y1) / 2.0);
                for (x, y) in [(x0, y0), (x1, y1), (x0 + dx, y0 + dy), (x1 + dx, y1 + dy)] {
                    self.mark(x, y);
                }
            }
            if let Some(tolerance) = self.trace.as_ref().map(|t| t.tolerance) {
                let steps = arc_steps(radius, sweep, tolerance);
                self.trace_to(self.x, self.y);
                for step in 1..=steps {
                    let (x, y) = on_circle(start + sweep * f64::from(step) / f64::from(steps));
                    self.trace_to(x, y);
                }
            }
        }
        (self.x, self.y) = on_circle(start + sweep);
    }

    /// Moves by `(dx, dy)` along an arc whose bulge is `bulge` / 127: the
    /// height of the arc over half its chord, anticlockwise when positive.
    fn bulge(&mut self, dx: f64, dy: f64, bulge: f64) {
        if bulge == 0.0 {
            self.line(dx, dy);
            return;
        }
        let (dx, dy) = (dx * self.scale, dy * self.scale);
        let chord = dx.hypot(dy);
        let bulge = bulge / 127.0;
        // The swept angle is four times the arctangent of the bulge.
        let sweep = 4.0 * bulge.atan();
        let radius = chord / (2.0 * (sweep / 2.0).sin().abs());
        // From the chord's middle, the centre lies to the left for an
        // anticlockwise arc.
        let (mx, my) = (self.x + dx / 2.0, self.y + dy / 2.0);
        let apothem = radius * (sweep / 2.0).cos();
        let (nx, ny) = (-dy / chord, dx / chord);
        let side = sweep.signum();
        let (cx, cy) = (mx + side * nx * apothem, my + side * ny * apothem);
        let start = (self.y - cy).atan2(self.x - cx).rem_euclid(TAU);
        let saved = self.scale;
        // `arc` scales the radius; it is already in drawing units here.
        self.scale = 1.0;
        self.arc(radius, start, sweep);
        self.scale = saved;
    }
}

/// Reads the little-endian numbers of a font file or a shape's program.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, length: usize) -> Result<&[u8], String> {
        let end = self.at + length;
        let taken = self
            .bytes
            .get(self.at..end)
            .ok_or("the file ends before its last shape does")?;
        self.at = end;
        Ok(taken)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn i8(&mut self) -> Result<i8, String> {
        Ok(i8::from_le_bytes([self.u8()?]))
    }

    fn u16(&mut self) -> Result<u16, String> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u16_big_endian(&mut self) -> Result<u16, String> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The bytes of a unifont holding `shapes`, each a number and its
    /// program, with capitals `above` units tall. Shape programs are
    /// written here; no real font is copied.
    pub(crate) fn unifont(above: u8, shapes: &[(u16, &[u8])]) -> Vec<u8> {
        let mut bytes = b"AutoCAD-86 unifont 1.0\r\n\x1a".to_vec();
        bytes.extend((shapes.len() as u32 + 1).to_le_bytes());
        let info = [b"MADE UP\0".as_slice(), &[above, 4, 0, 0, 0, 0]].concat();
        bytes.extend((info.len() as u16).to_le_bytes());
        bytes.extend(&info);
        for (number, program) in shapes {
            let definition = [&[0u8][..], program].concat();
            bytes.extend(number.to_le_bytes());
            bytes.extend((definition.len() as u16).to_le_bytes());
            bytes.extend(definition);
        }
        bytes
    }

    /// The bytes of an old-style shape font holding `shapes`.
    fn shapes_font(above: u8, shapes: &[(u16, &[u8])]) -> Vec<u8> {
        let info = [b"MADE UP\0".as_slice(), &[above, 4, 0, 0]].concat();
        let mut all: Vec<(u16, Vec<u8>)> = vec![(0, info)];
        all.extend(shapes.iter().map(|(n, p)| (*n, [&[0u8][..], p].concat())));
        let mut bytes = b"AutoCAD-86 shapes 1.0\r\n\x1a".to_vec();
        bytes.extend(0u16.to_le_bytes());
        bytes.extend(255u16.to_le_bytes());
        bytes.extend((all.len() as u16).to_le_bytes());
        for (number, definition) in &all {
            bytes.extend(number.to_le_bytes());
            bytes.extend((definition.len() as u16).to_le_bytes());
        }
        for (_, definition) in &all {
            bytes.extend(definition);
        }
        bytes.extend(b"EOF");
        bytes
    }

    #[track_caller]
    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-12, "{a} and {b}");
    }

    /// An `I`: pen up, move to (2, 0), down, up 10, pen up, on to (4, 0).
    const I: &[u8] = &[2, 8, 2, 0, 1, 0xA4, 2, 8, 2, 0xF6, 0];

    #[test]
    fn a_stroke_gives_its_box_and_the_pen_its_advance() {
        let font = ShxFont::parse(&unifont(10, &[(u16::from(b'I'), I)])).unwrap();
        let glyph = font.glyph('I').unwrap();
        close(glyph.advance.0, 0.4);
        close(glyph.advance.1, 0.0);
        let ink = glyph.ink.unwrap();
        close(ink.left, 0.2);
        close(ink.right, 0.2);
        close(ink.bottom, 0.0);
        close(ink.top, 1.0);
    }

    #[test]
    fn both_file_layouts_read_the_same_shape() {
        let old = ShxFont::parse(&shapes_font(10, &[(u16::from(b'I'), I)])).unwrap();
        let new = ShxFont::parse(&unifont(10, &[(u16::from(b'I'), I)])).unwrap();
        assert_eq!(old.glyph('I'), new.glyph('I'));
    }

    #[test]
    fn a_missing_character_draws_as_a_question_mark() {
        let font = ShxFont::parse(&unifont(10, &[(u16::from(b'?'), I)])).unwrap();
        assert_eq!(font.glyph('Z'), font.glyph('?'));
        let none = ShxFont::parse(&unifont(10, &[(u16::from(b'I'), I)])).unwrap();
        assert_eq!(none.glyph('Z'), None);
    }

    #[test]
    fn vertical_only_commands_are_skipped() {
        // 14 skips the next command, here a long move, in horizontal text.
        let program = [14, 8, 100, 100, 14, 9, 5, 5, 0, 0, 2, 8, 3, 0, 0];
        let font = ShxFont::parse(&unifont(10, &[(65, &program)])).unwrap();
        let glyph = font.glyph('A').unwrap();
        close(glyph.advance.0, 0.3);
        assert_eq!(glyph.ink, None);
    }

    #[test]
    fn scale_codes_subshapes_and_the_stack_move_the_pen() {
        // Shape B: draw (1, 0) twice the size, inside a push and pop, then
        // shape A as a subshape (big-endian in a unifont).
        let a: &[u8] = &[2, 8, 3, 0, 0];
        let b: &[u8] = &[5, 4, 2, 8, 1, 0, 6, 3, 2, 7, 0, 65, 0];
        let font = ShxFont::parse(&unifont(10, &[(65, a), (66, b)])).unwrap();
        let glyph = font.glyph('B').unwrap();
        // Pushed at 0, drew to 2, popped back to 0; then A moves 3.
        close(glyph.advance.0, 0.3);
        let ink = glyph.ink.unwrap();
        close(ink.right, 0.2);
    }

    #[test]
    fn an_octant_arc_reaches_its_extremes() {
        // From (0, 0), a half circle of radius 5 starting at octant 4
        // (west of its centre) anticlockwise through 4 octants: centre (5, 0),
        // down through (5, -5) to (10, 0).
        let program = [10, 5, 0x44, 0];
        let font = ShxFont::parse(&unifont(10, &[(65, &program)])).unwrap();
        let glyph = font.glyph('A').unwrap();
        close(glyph.advance.0, 1.0);
        close(glyph.advance.1, 0.0);
        let ink = glyph.ink.unwrap();
        close(ink.bottom, -0.5);
        close(ink.top, 0.0);
        // Clockwise, the same half circle goes over the top.
        let program = [10, 5, 0x80 | 0x44, 0];
        let font = ShxFont::parse(&unifont(10, &[(65, &program)])).unwrap();
        let ink = font.glyph('A').unwrap().ink.unwrap();
        close(ink.top, 0.5);
        close(ink.bottom, 0.0);
    }

    #[test]
    fn a_quarter_arc_is_boxed_as_autocad_boxes_it() {
        // From (0, 0), radius 10, octant 6 (below its centre) anticlockwise
        // through 2 octants: centre (0, 10), to (10, 10). The arc stays in
        // [0, 10] both ways, but AutoCAD's box is its chord and the chord
        // moved out to the arc's middle, which reaches 10 (1 - cos 45°) / √2
        // past it to the right and below.
        let program = [10, 10, 0x62, 0];
        let font = ShxFont::parse(&unifont(10, &[(65, &program)])).unwrap();
        let glyph = font.glyph('A').unwrap();
        close(glyph.advance.0, 1.0);
        close(glyph.advance.1, 1.0);
        let reach = (1.0 - FRAC_PI_4.cos()) / 2f64.sqrt();
        let ink = glyph.ink.unwrap();
        close(ink.left, 0.0);
        close(ink.right, 1.0 + reach);
        close(ink.bottom, -reach);
        close(ink.top, 1.0);
    }

    #[test]
    fn an_arc_longer_than_half_a_turn_is_boxed_by_its_circle() {
        // From (0, 0), radius 10, octant 3 clockwise through 6 octants:
        // AutoCAD boxes it by the whole circle around its centre.
        let program = [10, 10, 0x80 | 0x36, 0];
        let font = ShxFont::parse(&unifont(10, &[(65, &program)])).unwrap();
        let ink = font.glyph('A').unwrap().ink.unwrap();
        let (cx, cy) = (FRAC_PI_4.cos(), -FRAC_PI_4.sin());
        close(ink.left, cx - 1.0);
        close(ink.right, cx + 1.0);
        close(ink.bottom, cy - 1.0);
        close(ink.top, cy + 1.0);
    }

    #[test]
    fn the_point_where_the_pen_comes_down_counts_as_drawn() {
        // Pen up, move to (3, 0), pen down, pen up, move to (5, 0).
        let program = [2, 8, 3, 0, 1, 2, 8, 2, 0, 0];
        let font = ShxFont::parse(&unifont(10, &[(65, &program)])).unwrap();
        let ink = font.glyph('A').unwrap().ink.unwrap();
        close(ink.left, 0.3);
        close(ink.right, 0.3);
        close(ink.top, 0.0);
    }

    #[test]
    fn a_fractional_arc_takes_its_offsets_to_whole_degrees() {
        // 108 256ths of an octant is 18.98 degrees; AutoCAD takes 19. An end
        // offset of 0 ends the arc at its last octant's far boundary.
        let (from, to) = fractional_arc(false, 6, 2, 108.0, 0.0);
        close(from.to_degrees(), 289.0);
        close(to.to_degrees(), 360.0);
        // AutoCAD's own example: 55 to 100 degrees.
        let (from, to) = fractional_arc(false, 1, 2, 56.0, 56.0);
        close(from.to_degrees(), 55.0);
        close(to.to_degrees(), 100.0);
        let (from, to) = fractional_arc(true, 6, 6, 0.0, 147.0);
        close(from.to_degrees(), 270.0);
        close(to.to_degrees(), 270.0 - 5.0 * 45.0 - 26.0);
    }

    #[test]
    fn a_bulge_arc_of_127_is_a_half_circle() {
        // From (0, 0) to (10, 0) with bulge 127, anticlockwise: below the chord.
        let program = [12, 10, 0, 127, 0];
        let font = ShxFont::parse(&unifont(10, &[(65, &program)])).unwrap();
        let glyph = font.glyph('A').unwrap();
        close(glyph.advance.0, 1.0);
        let ink = glyph.ink.unwrap();
        close(ink.bottom, -0.5);
        close(ink.top, 0.0);
    }

    #[test]
    fn a_file_that_is_not_a_font_is_refused() {
        assert!(ShxFont::parse(b"not a font").is_err());
        let truncated = &unifont(10, &[(65, I)])[..40];
        assert!(ShxFont::parse(truncated).is_err());
        assert!(ShxFont::parse(b"AutoCAD-86 bigfont 1.0\r\n\x1a").is_err());
    }
}
