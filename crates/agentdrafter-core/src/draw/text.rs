//! Text, laid out as AutoCAD lays it out.
//!
//! A single line of text starts where the build records it starts
//! ([`SheetFonts::start`]), which is where AutoCAD draws it from.
//!
//! Multiline text follows AutoCAD's MTEXT, as AutoCAD 2026 laid out
//! made-up multiline text in SHX and TrueType fonts, in every attachment,
//! when it was exploded into single lines:
//!
//! - Lines break where the contents break them and, in a box with a width,
//!   between words where what the line draws would grow wider than the box.
//! - Each line reaches above its baseline by the text's height, or by more
//!   where it draws higher. The first line reaches the top of the box, and
//!   each line after sits two thirds of the height, and its own reach,
//!   below the one before: 5/3 of the height for plain capitals. The box
//!   reaches down to the lowest point the last line draws.
//! - Each line is aligned by what it draws: its left edge on the box's left
//!   side, its middle on the box's middle, or its right edge on the box's
//!   right side, as the text is attached on the left, centre or right.
//!
//! Each line drawn also gives a [`TextRun`]: the characters it draws, laid
//! along its baseline, so an output such as the PDF can keep the text
//! searchable where it draws. The run draws nothing itself.

use super::{Point2, Shape, TextRun};
use crate::model::MTextBox;
use crate::text::fonts::{Font, Rect};
use crate::text::placement::SheetFonts;
use crate::text::{Attachment, MText, Text};

/// How far apart MTEXT's lines are, as a share of the text's height:
/// AutoCAD's single line spacing.
const LINE_SPACING: f64 = 5.0 / 3.0;

/// What a piece of text draws: its shapes, and each line it draws as a run
/// of characters along its baseline.
#[derive(Default)]
pub(super) struct Lettering {
    pub shapes: Vec<Shape>,
    pub runs: Vec<TextRun>,
}

/// What a single line of text draws.
pub(super) fn draw_text(
    text: &Text,
    fonts: &SheetFonts,
    lineweight: f64,
    tolerance: f64,
) -> Lettering {
    let (font, _) = fonts.style(&text.style);
    let start = fonts.start(text);
    let place = Placement {
        origin: [start.x, start.y],
        rotation: text.rotation,
        height: text.height,
        width_factor: text.width_factor,
    };
    let mut out = Drawn::default();
    place.line(font, &text.contents, [0.0, 0.0], tolerance, &mut out);
    out.lettering(lineweight)
}

/// How many lines multiline text is laid out in: each line its contents
/// break, a blank one among them, and each line wrapping makes in its box.
pub(crate) fn mtext_lines(text: &MText, fonts: &SheetFonts) -> usize {
    let (font, style_width) = fonts.style(&text.style);
    let along = text.height * text.width_factor.unwrap_or(style_width);
    wrap(font, &text.contents, text.width / along).len()
}

/// Multiline text's box, as it is laid out: its width, its height and how
/// far its last line draws below its baseline.
pub(crate) fn mtext_box(text: &MText, fonts: &SheetFonts) -> MTextBox {
    let (font, style_width) = fonts.style(&text.style);
    let laid = lay_out(text, font, text.width_factor.unwrap_or(style_width));
    MTextBox {
        width: laid.box_width,
        height: laid.box_height,
        last_descent: laid.descent,
    }
}

/// Multiline text laid out in its own frame, from the top of its box.
struct Laid {
    lines: Vec<String>,
    /// Each line's ink box, from the start of its baseline, in drawing units.
    inks: Vec<Option<Rect>>,
    /// Each line's baseline, down from the top of the box.
    baselines: Vec<f64>,
    box_width: f64,
    box_height: f64,
    /// How far the last line draws below its baseline.
    descent: f64,
}

fn lay_out(text: &MText, font: &Font, width_factor: f64) -> Laid {
    let h = text.height;
    let along = h * width_factor;
    let lines = wrap(font, &text.contents, text.width / along);
    // Each line's ink box, from the start of its baseline, in drawing units.
    let inks: Vec<Option<Rect>> = lines
        .iter()
        .map(|line| {
            font.line(line).ink.map(|ink| Rect {
                left: ink.left * along,
                right: ink.right * along,
                bottom: ink.bottom * h,
                top: ink.top * h,
            })
        })
        .collect();
    let widest = inks
        .iter()
        .flatten()
        .map(|ink| ink.right - ink.left)
        .fold(0.0, f64::max);
    let box_width = if text.width > 0.0 { text.width } else { widest };
    // How far each line reaches above its baseline: the capitals' height, or
    // more where it draws higher. The next line's baseline sits two thirds
    // of the height below this one's, and its own reach below that.
    let ascent = |ink: &Option<Rect>| ink.map_or(h, |ink| ink.top.max(h));
    let mut baselines = Vec::with_capacity(lines.len());
    let mut baseline = -inks.first().map_or(h, ascent);
    for (i, ink) in inks.iter().enumerate() {
        if i > 0 {
            baseline -= (LINE_SPACING - 1.0) * h + ascent(ink);
        }
        baselines.push(baseline);
    }
    // The box reaches down to the last line's lowest ink.
    let descent = inks
        .last()
        .and_then(|ink| ink.map(|ink| (-ink.bottom).max(0.0)))
        .unwrap_or(0.0);
    let box_height = -baselines.last().copied().unwrap_or(-h) + descent;
    Laid {
        lines,
        inks,
        baselines,
        box_width,
        box_height,
        descent,
    }
}

/// What multiline text draws.
pub(super) fn draw_mtext(
    text: &MText,
    fonts: &SheetFonts,
    lineweight: f64,
    tolerance: f64,
) -> Lettering {
    let (font, style_width) = fonts.style(&text.style);
    let width_factor = text.width_factor.unwrap_or(style_width);
    let place = Placement {
        origin: [text.position.x, text.position.y],
        rotation: text.rotation,
        height: text.height,
        width_factor,
    };
    let Laid {
        lines,
        inks,
        baselines,
        box_width,
        box_height,
        ..
    } = lay_out(text, font, width_factor);
    let (across, up) = attachment(text.justify);
    // The box's left side and top, from the position.
    let left = -box_width * across;
    let top = box_height * up;
    let mut out = Drawn::default();
    for ((line, ink), baseline) in lines.iter().zip(&inks).zip(&baselines) {
        let Some(Rect {
            left: ink_left,
            right: ink_right,
            ..
        }) = *ink
        else {
            continue;
        };
        // The point of the ink as far across as the position is across the
        // box lands on the box at that point: its left edge, middle or right.
        let x = left + box_width * across - (ink_left + (ink_right - ink_left) * across);
        place.line(font, line, [x, top + baseline], tolerance, &mut out);
    }
    out.lettering(lineweight)
}

/// How far across and up from the box's left side and top the position
/// is, each from 0 to 1.
fn attachment(a: Attachment) -> (f64, f64) {
    match a {
        Attachment::TopLeft => (0.0, 0.0),
        Attachment::TopCenter => (0.5, 0.0),
        Attachment::TopRight => (1.0, 0.0),
        Attachment::MiddleLeft => (0.0, 0.5),
        Attachment::MiddleCenter => (0.5, 0.5),
        Attachment::MiddleRight => (1.0, 0.5),
        Attachment::BottomLeft => (0.0, 1.0),
        Attachment::BottomCenter => (0.5, 1.0),
        Attachment::BottomRight => (1.0, 1.0),
    }
}

/// The lines multiline text draws: its own lines, each broken between
/// words where the next word would run past `width`, which is in units of
/// the font's capitals along the line (0 for no box). A word wider than the
/// box stays whole on a line of its own.
pub(super) fn wrap(font: &Font, contents: &str, width: f64) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in contents.split('\n') {
        if width <= 0.0 {
            lines.push(paragraph.to_owned());
            continue;
        }
        let mut line = String::new();
        for word in paragraph.split(' ') {
            if line.is_empty() {
                line.push_str(word);
                continue;
            }
            let candidate = format!("{line} {word}");
            if fits(font, &candidate, width) {
                line = candidate;
            } else {
                lines.push(std::mem::take(&mut line));
                line.push_str(word);
            }
        }
        lines.push(line);
    }
    lines
}

/// Whether what a line of text draws is no wider than `width`, in units of
/// the font's capitals. The line is aligned by what it draws, so that is
/// what must fit: AutoCAD 2026 kept a line whose strokes run 59.81 wide in
/// a box 60 wide, though its pen moved 60.76.
fn fits(font: &Font, line: &str, width: f64) -> bool {
    font.line(line)
        .ink
        .is_none_or(|ink| ink.right - ink.left <= width)
}

/// Where text sits: the start of its first baseline, its turn and its size.
struct Placement {
    origin: Point2,
    rotation: f64,
    height: f64,
    width_factor: f64,
}

/// What text draws so far: strokes, filled outlines and runs of characters.
#[derive(Default)]
struct Drawn {
    strokes: Vec<Vec<Point2>>,
    fills: Vec<Vec<Point2>>,
    runs: Vec<TextRun>,
}

impl Drawn {
    fn lettering(self, lineweight: f64) -> Lettering {
        let mut shapes = Vec::new();
        if !self.strokes.is_empty() {
            shapes.push(Shape::Stroke {
                lineweight,
                runs: self.strokes,
            });
        }
        if !self.fills.is_empty() {
            shapes.push(Shape::Fill {
                rule: super::FillRule::NonZero,
                contours: self.fills,
            });
        }
        Lettering {
            shapes,
            runs: self.runs,
        }
    }
}

impl Placement {
    /// Draws one line of text whose baseline starts at `start`, in the
    /// text's own frame before it is turned.
    fn line(&self, font: &Font, contents: &str, start: Point2, tolerance: f64, out: &mut Drawn) {
        let (sin, cos) = self.rotation.to_radians().sin_cos();
        let along = self.height * self.width_factor;
        let up = self.height;
        let place = |pen: (f64, f64), p: Point2| -> Point2 {
            let u = start[0] + (pen.0 + p[0]) * along;
            let v = start[1] + (pen.1 + p[1]) * up;
            [
                self.origin[0] + u * cos - v * sin,
                self.origin[1] + u * sin + v * cos,
            ]
        };
        let mut pen = (0.0, 0.0);
        // The run of characters along the baseline: one unit of the font's
        // advances across, the capitals' height up, both turned with the
        // text.
        let run_from = |pen: (f64, f64)| TextRun {
            object: 0,
            part: None,
            nested: Vec::new(),
            cell: None,
            drawn: None,
            origin: place(pen, [0.0, 0.0]),
            across: [along * cos, along * sin],
            up: [-up * sin, up * cos],
            characters: Vec::new(),
        };
        let mut run = run_from(pen);
        // Where the run's pen started, for the box round what it draws.
        let mut run_pen = pen;
        // A glyph is enlarged by its height up and by `along` across, so
        // its tolerance is divided by the larger to stay within the drawing
        // units' tolerance both ways.
        let tolerance = tolerance / self.height.max(along).max(f64::MIN_POSITIVE);
        for glyph in font.outlines(contents, tolerance) {
            for stroke in &glyph.strokes {
                out.strokes
                    .push(stroke.iter().map(|&p| place(pen, p)).collect());
            }
            for contour in &glyph.fills {
                out.fills
                    .push(contour.iter().map(|&p| place(pen, p)).collect());
            }
            let inked = glyph.strokes.iter().chain(&glyph.fills).flatten();
            for &p in inked {
                let at = [pen.0 + p[0] - run_pen.0, pen.1 + p[1] - run_pen.1];
                run.drawn = Some(match run.drawn {
                    None => [at, at],
                    Some([lo, hi]) => [
                        [lo[0].min(at[0]), lo[1].min(at[1])],
                        [hi[0].max(at[0]), hi[1].max(at[1])],
                    ],
                });
            }
            run.characters.push((glyph.character, glyph.advance.0));
            pen = (pen.0 + glyph.advance.0, pen.1 + glyph.advance.1);
            // An SHX character can move the pen off the line; the characters
            // after it run along a baseline of their own.
            if glyph.advance.1 != 0.0 {
                out.runs.push(std::mem::replace(&mut run, run_from(pen)));
                run_pen = pen;
            }
        }
        out.runs.push(run);
        out.runs.retain(|run| !run.characters.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AutoCAD 2026 broke the example sheet's wrapped note, in Roman
    /// Simplex's widths at 2.5 high and 0.8 wide in a box 60 wide, at these
    /// words.
    #[test]
    fn a_long_line_wraps_between_words() {
        let lines = wrap(
            &Font::Hershey,
            "1. THIS TEXT WRAPS IN A BOX 60 MM WIDE, AND KEEPS {BRACES}, A \\ AND 100% AS WRITTEN.",
            60.0 / (2.5 * 0.8),
        );
        assert_eq!(
            lines,
            [
                "1. THIS TEXT WRAPS IN A BOX 60 MM",
                "WIDE, AND KEEPS {BRACES}, A \\ AND",
                "100% AS WRITTEN."
            ]
        );
    }

    #[test]
    fn without_a_width_lines_break_only_where_the_contents_do() {
        assert_eq!(wrap(&Font::Hershey, "A B\nC", 0.0), ["A B", "C"]);
        assert_eq!(wrap(&Font::Hershey, "A\n\nB", 0.0), ["A", "", "B"]);
    }
}
