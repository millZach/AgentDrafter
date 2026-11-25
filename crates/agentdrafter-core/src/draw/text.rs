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

use super::{Color, Point2, Shape, TextRun};
use crate::model::{MTextBox, ObjectColor};
use crate::text::fonts::{Font, Rect};
use crate::text::placement::SheetFonts;
use crate::text::{Attachment, MText, MTextRun, Text};

/// How far apart MTEXT's lines are, as a share of the text's height:
/// AutoCAD's single line spacing.
const LINE_SPACING: f64 = 5.0 / 3.0;

/// What a piece of text draws: its shapes, any inline-coloured shapes
/// (MTEXT `\C<n>;`, #61), and each line it draws as a run of characters along
/// its baseline.
#[derive(Default)]
pub(super) struct Lettering {
    pub shapes: Vec<Shape>,
    pub colored: Vec<(Color, Shape)>,
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
    place.line(
        font,
        &text.contents,
        [0.0, 0.0],
        tolerance,
        text.underline,
        text.overline,
        &mut out,
    );
    out.lettering(lineweight)
}

/// How many lines multiline text is laid out in: each line its contents
/// break, a blank one among them, and each line wrapping makes in its box.
pub(crate) fn mtext_lines(text: &MText, fonts: &SheetFonts) -> usize {
    if !text.runs.is_empty() {
        return lay_out_rich(text, fonts).lines.len();
    }
    let (font, style_width) = fonts.style(&text.style);
    let along = text.height * text.width_factor.unwrap_or(style_width);
    wrap(font, &text.contents, text.width / along).len()
}

/// Multiline text's box, as it is laid out: its width, its height and how
/// far its last line draws below its baseline.
pub(crate) fn mtext_box(text: &MText, fonts: &SheetFonts) -> MTextBox {
    if !text.runs.is_empty() {
        let laid = lay_out_rich(text, fonts);
        return MTextBox {
            width: laid.box_width,
            height: laid.box_height,
            last_descent: laid.descent,
            left: laid
                .lines
                .first()
                .and_then(|l| l.ink)
                .map_or(0.0, |ink| ink.left),
            advance: laid.lines.first().map_or(0.0, |l| l.advance),
        };
    }
    let (font, style_width) = fonts.style(&text.style);
    let width_factor = text.width_factor.unwrap_or(style_width);
    let laid = lay_out(text, font, width_factor);
    let along = text.height * width_factor;
    let first = laid.lines.first().map(|line| font.line(line));
    MTextBox {
        width: laid.box_width,
        height: laid.box_height,
        last_descent: laid.descent,
        left: laid
            .inks
            .first()
            .copied()
            .flatten()
            .map_or(0.0, |ink| ink.left),
        advance: first.map_or(0.0, |line| line.advance * along),
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
    if !text.runs.is_empty() {
        return draw_mtext_rich(text, fonts, lineweight, tolerance);
    }
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
        place.line(
            font,
            line,
            [x, top + baseline],
            tolerance,
            text.underline,
            text.overline,
            &mut out,
        );
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
            colored: Vec::new(),
            runs: self.runs,
        }
    }
}

/// Resolved per-segment style for inline-formatted MTEXT (#61).
#[derive(Clone, Copy)]
struct RichStyle<'a> {
    font: &'a Font,
    height: f64,
    width_factor: f64,
    oblique_tan: f64,
    tracking: f64,
    bold_stroke: bool,
    underline: bool,
    overline: bool,
    color: Option<Color>,
}

/// One styled text segment inside a visual line of rich MTEXT.
#[derive(Clone)]
struct RichSeg<'a> {
    text: String,
    style: RichStyle<'a>,
}

/// One laid-out visual line of rich MTEXT.
struct RichLine<'a> {
    segs: Vec<RichSeg<'a>>,
    ink: Option<Rect>,
    advance: f64,
    max_height: f64,
}

/// Rich multiline text laid out in its own frame, from the top of its box.
struct LaidRich<'a> {
    lines: Vec<RichLine<'a>>,
    baselines: Vec<f64>,
    box_width: f64,
    box_height: f64,
    descent: f64,
}

fn resolve_run_style<'a>(
    mtext: &MText,
    run: &MTextRun,
    fonts: &'a SheetFonts,
    style_width: f64,
) -> RichStyle<'a> {
    let (font, has_bold_face, has_italic_face) =
        fonts.inline_font(&mtext.style, run.font.as_deref(), run.bold, run.italic);
    let height = run.height.unwrap_or(mtext.height);
    let width_factor = run
        .width_factor
        .or(mtext.width_factor)
        .unwrap_or(style_width);
    let oblique_deg = run.oblique.unwrap_or(if run.italic && !has_italic_face {
        15.0
    } else {
        0.0
    });
    let oblique_tan = oblique_deg.to_radians().tan();
    let tracking = run.tracking.unwrap_or(1.0);
    let bold_stroke = run.bold && !has_bold_face;
    let color = match run.color {
        Some(ObjectColor::Index(i)) => Some(Color(i)),
        _ => None,
    };
    RichStyle {
        font,
        height,
        width_factor,
        oblique_tan,
        tracking,
        bold_stroke,
        underline: run.underline || mtext.underline,
        overline: run.overline || mtext.overline,
        color,
    }
}

/// Measures a sequence of styled segments along one baseline in drawing units.
fn measure_rich_line(segs: &[RichSeg<'_>], base_height: f64) -> (Option<Rect>, f64, f64) {
    let mut x = 0.0;
    let mut ink: Option<Rect> = None;
    let mut max_height = 0.0f64;
    for seg in segs {
        if seg.text.is_empty() {
            continue;
        }
        max_height = max_height.max(seg.style.height);
        let along = seg.style.height * seg.style.width_factor;
        let up = seg.style.height;
        let mut pen = 0.0;
        for glyph in seg.style.font.glyphs(&seg.text) {
            if let Some(drawn) = glyph.ink {
                let mut left = x + (pen + drawn.left) * along;
                let mut right = x + (pen + drawn.right) * along;
                let bottom = drawn.bottom * up;
                let top = drawn.top * up;
                if seg.style.oblique_tan != 0.0 {
                    let s0 = bottom * seg.style.oblique_tan;
                    let s1 = top * seg.style.oblique_tan;
                    left += s0.min(s1);
                    right += s0.max(s1);
                }
                let r = Rect {
                    left,
                    right,
                    bottom,
                    top,
                };
                ink = Some(ink.map_or(r, |all| all.union(r)));
            }
            pen += glyph.advance.0 * seg.style.tracking;
        }
        x += pen * along;
    }
    if max_height == 0.0 {
        max_height = base_height;
    }
    (ink, x, max_height)
}

fn lay_out_rich<'a>(text: &MText, fonts: &'a SheetFonts) -> LaidRich<'a> {
    let (_, style_width) = fonts.style(&text.style);
    // Split runs on '\n' into paragraphs of styled segments.
    let mut paragraphs: Vec<Vec<RichSeg<'a>>> = vec![Vec::new()];
    for run in &text.runs {
        let style = resolve_run_style(text, run, fonts, style_width);
        for (i, piece) in run.text.split('\n').enumerate() {
            if i > 0 {
                paragraphs.push(Vec::new());
            }
            if !piece.is_empty() {
                paragraphs.last_mut().unwrap().push(RichSeg {
                    text: piece.to_owned(),
                    style,
                });
            }
        }
    }

    // Wrap each paragraph between spaces when `text.width > 0`.
    let mut visual_lines: Vec<Vec<RichSeg<'a>>> = Vec::new();
    for para in paragraphs {
        if text.width <= 0.0 || para.is_empty() {
            visual_lines.push(para);
            continue;
        }
        // Tokenize into alternating word chunks (possibly spanning multiple
        // inline styles) and space chunks.
        let mut words: Vec<Vec<RichSeg<'a>>> = Vec::new();
        let mut spaces: Vec<Vec<RichSeg<'a>>> = Vec::new();
        let mut cur_word: Vec<RichSeg<'a>> = Vec::new();
        let mut cur_space: Vec<RichSeg<'a>> = Vec::new();
        let mut in_space = false;
        for seg in para {
            for ch in seg.text.chars() {
                if ch == ' ' {
                    if !in_space && !cur_word.is_empty() {
                        words.push(std::mem::take(&mut cur_word));
                    }
                    in_space = true;
                    cur_space.push(RichSeg {
                        text: " ".to_owned(),
                        style: seg.style,
                    });
                } else {
                    if in_space {
                        spaces.push(std::mem::take(&mut cur_space));
                        in_space = false;
                    }
                    if let Some(last) = cur_word.last_mut() {
                        if std::ptr::eq(last.style.font, seg.style.font)
                            && last.style.height == seg.style.height
                            && last.style.width_factor == seg.style.width_factor
                            && last.style.oblique_tan == seg.style.oblique_tan
                            && last.style.tracking == seg.style.tracking
                            && last.style.bold_stroke == seg.style.bold_stroke
                            && last.style.underline == seg.style.underline
                            && last.style.overline == seg.style.overline
                            && last.style.color == seg.style.color
                        {
                            last.text.push(ch);
                            continue;
                        }
                    }
                    cur_word.push(RichSeg {
                        text: ch.to_string(),
                        style: seg.style,
                    });
                }
            }
        }
        if !cur_word.is_empty() {
            words.push(cur_word);
        } else if !cur_space.is_empty() && words.is_empty() {
            visual_lines.push(cur_space);
            continue;
        }
        let mut line: Vec<RichSeg<'a>> = Vec::new();
        for (idx, word) in words.into_iter().enumerate() {
            if line.is_empty() {
                line = word;
                continue;
            }
            let sep = spaces
                .get(idx.saturating_sub(1))
                .cloned()
                .unwrap_or_default();
            let mut candidate = line.clone();
            candidate.extend(sep.iter().cloned());
            candidate.extend(word.iter().cloned());
            let (cand_ink, _, _) = measure_rich_line(&candidate, text.height);
            let fits = cand_ink.is_none_or(|r| r.right - r.left <= text.width);
            if fits {
                line = candidate;
            } else {
                visual_lines.push(std::mem::take(&mut line));
                line = word;
            }
        }
        visual_lines.push(line);
    }

    let lines: Vec<RichLine<'a>> = visual_lines
        .into_iter()
        .map(|segs| {
            let (ink, advance, max_height) = measure_rich_line(&segs, text.height);
            RichLine {
                segs,
                ink,
                advance,
                max_height,
            }
        })
        .collect();
    let widest = lines
        .iter()
        .filter_map(|l| l.ink.map(|r| r.right - r.left))
        .fold(0.0, f64::max);
    let box_width = if text.width > 0.0 { text.width } else { widest };
    let ascent = |l: &RichLine<'_>| l.ink.map_or(l.max_height, |r| r.top.max(l.max_height));
    let mut baselines = Vec::with_capacity(lines.len());
    let mut baseline = -lines.first().map_or(text.height, ascent);
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            baseline -= (LINE_SPACING - 1.0) * line.max_height + ascent(line);
        }
        baselines.push(baseline);
    }
    let descent = lines
        .last()
        .and_then(|l| l.ink.map(|r| (-r.bottom).max(0.0)))
        .unwrap_or(0.0);
    let box_height = -baselines.last().copied().unwrap_or(-text.height) + descent;
    LaidRich {
        lines,
        baselines,
        box_width,
        box_height,
        descent,
    }
}

fn draw_mtext_rich(text: &MText, fonts: &SheetFonts, lineweight: f64, tolerance: f64) -> Lettering {
    let laid = lay_out_rich(text, fonts);
    let (across, up) = attachment(text.justify);
    let left = -laid.box_width * across;
    let top = laid.box_height * up;
    let (sin, cos) = text.rotation.to_radians().sin_cos();
    let origin = [text.position.x, text.position.y];

    let mut shapes: Vec<Shape> = Vec::new();
    let mut colored: Vec<(Color, Shape)> = Vec::new();
    let mut runs: Vec<TextRun> = Vec::new();

    for (line, &baseline) in laid.lines.iter().zip(&laid.baselines) {
        let Some(Rect {
            left: ink_left,
            right: ink_right,
            ..
        }) = line.ink
        else {
            continue;
        };
        let mut cursor_x =
            left + laid.box_width * across - (ink_left + (ink_right - ink_left) * across);
        let y_base = top + baseline;
        for seg in &line.segs {
            if seg.text.is_empty() {
                continue;
            }
            let st = seg.style;
            let along = st.height * st.width_factor;
            let up_h = st.height;
            let lw = if st.bold_stroke {
                lineweight * 1.5
            } else {
                lineweight
            };
            let place = |pen: (f64, f64), p: Point2| -> Point2 {
                let shear = p[1] * st.oblique_tan;
                let u = cursor_x + (pen.0 + p[0]) * along + shear * up_h;
                let v = y_base + (pen.1 + p[1]) * up_h;
                [origin[0] + u * cos - v * sin, origin[1] + u * sin + v * cos]
            };
            let mut seg_strokes: Vec<Vec<Point2>> = Vec::new();
            let mut seg_fills: Vec<Vec<Point2>> = Vec::new();
            let mut pen = (0.0, 0.0);
            let mut tr = TextRun {
                object: 0,
                part: None,
                nested: Vec::new(),
                cell: None,
                viewport: None,
                drawn: None,
                origin: place(pen, [0.0, 0.0]),
                across: [along * cos, along * sin],
                up: [-up_h * sin, up_h * cos],
                characters: Vec::new(),
            };
            let tol = tolerance / up_h.max(along).max(f64::MIN_POSITIVE);
            for glyph in st.font.outlines(&seg.text, tol) {
                for stroke in &glyph.strokes {
                    seg_strokes.push(stroke.iter().map(|&p| place(pen, p)).collect());
                }
                for contour in &glyph.fills {
                    seg_fills.push(contour.iter().map(|&p| place(pen, p)).collect());
                }
                let inked = glyph.strokes.iter().chain(&glyph.fills).flatten();
                for &p in inked {
                    let at = [pen.0 + p[0], pen.1 + p[1]];
                    tr.drawn = Some(match tr.drawn {
                        None => [at, at],
                        Some([lo, hi]) => [
                            [lo[0].min(at[0]), lo[1].min(at[1])],
                            [hi[0].max(at[0]), hi[1].max(at[1])],
                        ],
                    });
                }
                let adv = glyph.advance.0 * st.tracking;
                tr.characters.push((glyph.character, adv));
                pen = (pen.0 + adv, pen.1 + glyph.advance.1);
            }
            if (st.underline || st.overline) && pen.0 > 0.0 {
                let from = (0.0, 0.0);
                let to = (pen.0, 0.0);
                if let Some((u_pos, u_thick)) = st.font.decoration_bar() {
                    let thick = u_thick.max(0.04);
                    if st.underline {
                        let y_top = u_pos.min(-0.05);
                        let y_bot = y_top - thick;
                        seg_fills.push(vec![
                            place(from, [0.0, y_bot]),
                            place(to, [0.0, y_bot]),
                            place(to, [0.0, y_top]),
                            place(from, [0.0, y_top]),
                            place(from, [0.0, y_bot]),
                        ]);
                    }
                    if st.overline {
                        let gap = (-u_pos).max(0.05);
                        let y_bot = 1.0 + gap;
                        let y_top = y_bot + thick;
                        seg_fills.push(vec![
                            place(from, [0.0, y_bot]),
                            place(to, [0.0, y_bot]),
                            place(to, [0.0, y_top]),
                            place(from, [0.0, y_top]),
                            place(from, [0.0, y_bot]),
                        ]);
                    }
                } else {
                    if st.underline {
                        seg_strokes.push(vec![place(from, [0.0, -0.2]), place(to, [0.0, -0.2])]);
                    }
                    if st.overline {
                        seg_strokes.push(vec![place(from, [0.0, 1.2]), place(to, [0.0, 1.2])]);
                    }
                }
            }
            if !seg_strokes.is_empty() {
                let s = Shape::Stroke {
                    lineweight: lw,
                    runs: seg_strokes,
                };
                match st.color {
                    Some(c) => colored.push((c, s)),
                    None => shapes.push(s),
                }
            }
            if !seg_fills.is_empty() {
                let s = Shape::Fill {
                    rule: super::FillRule::NonZero,
                    contours: seg_fills,
                };
                match st.color {
                    Some(c) => colored.push((c, s)),
                    None => shapes.push(s),
                }
            }
            if !tr.characters.is_empty() {
                runs.push(tr);
            }
            cursor_x += pen.0 * along;
        }
    }
    Lettering {
        shapes,
        colored,
        runs,
    }
}

impl Placement {
    /// Draws one line of text whose baseline starts at `start`, in the
    /// text's own frame before it is turned.
    #[allow(clippy::too_many_arguments)]
    fn line(
        &self,
        font: &Font,
        contents: &str,
        start: Point2,
        tolerance: f64,
        underline: bool,
        overline: bool,
        out: &mut Drawn,
    ) {
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
        let add_decorations = |from: (f64, f64), to: (f64, f64), out: &mut Drawn| {
            if (!underline && !overline) || to.0 <= from.0 {
                return;
            }
            if let Some((u_pos, u_thick)) = font.decoration_bar() {
                let thick = u_thick.max(0.04);
                let mut push_bar = |y_bot: f64, y_top: f64| {
                    out.fills.push(vec![
                        place(from, [0.0, y_bot]),
                        place(to, [0.0, y_bot]),
                        place(to, [0.0, y_top]),
                        place(from, [0.0, y_top]),
                        place(from, [0.0, y_bot]),
                    ]);
                };
                if underline {
                    let y_top = u_pos.min(-0.05);
                    push_bar(y_top - thick, y_top);
                }
                if overline {
                    let gap = (-u_pos).max(0.05);
                    let y_bot = 1.0 + gap;
                    push_bar(y_bot, y_bot + thick);
                }
            } else {
                // SHX and Hershey stroke fonts: single stroke below baseline
                // and above cap height in the text's lineweight (#60).
                if underline {
                    out.strokes
                        .push(vec![place(from, [0.0, -0.2]), place(to, [0.0, -0.2])]);
                }
                if overline {
                    out.strokes
                        .push(vec![place(from, [0.0, 1.2]), place(to, [0.0, 1.2])]);
                }
            }
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
            viewport: None,
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
            let run_end = (pen.0 + glyph.advance.0, run_pen.1);
            pen = (pen.0 + glyph.advance.0, pen.1 + glyph.advance.1);
            // An SHX character can move the pen off the line; the characters
            // after it run along a baseline of their own.
            if glyph.advance.1 != 0.0 {
                add_decorations(run_pen, run_end, out);
                out.runs.push(std::mem::replace(&mut run, run_from(pen)));
                run_pen = pen;
            }
        }
        add_decorations(run_pen, (pen.0, run_pen.1), out);
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

    #[test]
    fn underline_and_overline_draw_strokes_for_shx_and_bars_for_truetype() {
        use crate::model::Point3;
        use crate::settings::Settings;
        use crate::text::truetype::tests::made_up_font;
        use crate::text::{Justify, TextStyle};

        // Hershey (stroke font): underline adds a stroke at y = -0.2 * h and
        // overline at y = 1.2 * h, spanning the line's advance.
        let sheet =
            crate::description::parse_sheet_description("units: mm\nobjects: []\n").unwrap();
        let fonts = SheetFonts::find(&sheet, &Settings::default());
        let plain = Text {
            contents: "AB".into(),
            position: Point3::new(10.0, 20.0, 0.0),
            height: 5.0,
            style: "Standard".into(),
            width_factor: 1.0,
            rotation: 0.0,
            justify: Justify::Left,
            underline: false,
            overline: false,
            layer: "0".into(),
            field: None,
            annotative: false,
            annotation_scales: Vec::new(),
            properties: Default::default(),
        };
        let plain_strokes = match draw_text(&plain, &fonts, 0.25, 0.01).shapes.as_slice() {
            [Shape::Stroke { runs, .. }] => runs.len(),
            other => panic!("expected one stroke shape: {other:?}"),
        };
        let styled = Text {
            underline: true,
            overline: true,
            ..plain.clone()
        };
        let styled_runs = match draw_text(&styled, &fonts, 0.25, 0.01).shapes.as_slice() {
            [Shape::Stroke { runs, .. }] => runs.clone(),
            other => panic!("expected one stroke shape: {other:?}"),
        };
        assert_eq!(styled_runs.len(), plain_strokes + 2);
        let under = &styled_runs[plain_strokes];
        let over = &styled_runs[plain_strokes + 1];
        assert!((under[0][0] - 10.0).abs() < 1e-9);
        assert!((under[0][1] - 19.0).abs() < 1e-9); // 20 - 0.2 * 5
        assert!(under[1][0] > 10.0);
        assert!((under[1][1] - 19.0).abs() < 1e-9);
        assert!((over[0][1] - 26.0).abs() < 1e-9); // 20 + 1.2 * 5
        assert!((over[1][1] - 26.0).abs() < 1e-9);

        // TrueType font: underline and overline draw filled bars.
        let dir = std::env::temp_dir().join(format!("agentdrafter-uline-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("madeup.ttf"),
            made_up_font(&[('A', 700, [50, 0, 650, 700])]),
        )
        .unwrap();
        let mut ttf_sheet =
            crate::description::parse_sheet_description("units: mm\nobjects: []\n").unwrap();
        ttf_sheet
            .text_styles
            .push(TextStyle::new("ttf", "madeup.ttf"));
        let settings = Settings {
            font_folders: vec![dir.clone()],
            ..Settings::default()
        };
        let ttf_fonts = SheetFonts::find(&ttf_sheet, &settings);
        let ttf_plain = Text {
            style: "ttf".into(),
            contents: "AA".into(),
            ..plain
        };
        let plain_fills = match draw_text(&ttf_plain, &ttf_fonts, 0.25, 0.01)
            .shapes
            .as_slice()
        {
            [Shape::Fill { contours, .. }] => contours.len(),
            other => panic!("expected one fill shape: {other:?}"),
        };
        let ttf_styled = Text {
            underline: true,
            overline: true,
            ..ttf_plain
        };
        let styled_fills = match draw_text(&ttf_styled, &ttf_fonts, 0.25, 0.01)
            .shapes
            .as_slice()
        {
            [Shape::Fill { contours, .. }] => contours.clone(),
            other => panic!("expected one fill shape: {other:?}"),
        };
        assert_eq!(styled_fills.len(), plain_fills + 2);
        // Underline bar sits strictly below y = 20; overline bar sits above y = 25.
        assert!(styled_fills[plain_fills].iter().all(|p| p[1] < 20.0));
        assert!(styled_fills[plain_fills + 1].iter().all(|p| p[1] > 25.0));

        // MText with underline skips blank lines.
        let mtext = MText {
            contents: "A\n\nB".into(),
            runs: Vec::new(),
            position: Point3::new(0.0, 10.0, 0.0),
            height: 2.0,
            width: 0.0,
            style: "Standard".into(),
            width_factor: None,
            rotation: 0.0,
            justify: Attachment::TopLeft,
            underline: true,
            overline: false,
            layer: "0".into(),
            field: None,
            annotative: false,
            annotation_scales: Vec::new(),
            properties: Default::default(),
        };
        let plain_m = MText {
            underline: false,
            ..mtext.clone()
        };
        let m_plain_n = match draw_mtext(&plain_m, &fonts, 0.25, 0.01).shapes.as_slice() {
            [Shape::Stroke { runs, .. }] => runs.len(),
            other => panic!("expected stroke: {other:?}"),
        };
        let m_under_n = match draw_mtext(&mtext, &fonts, 0.25, 0.01).shapes.as_slice() {
            [Shape::Stroke { runs, .. }] => runs.len(),
            other => panic!("expected stroke: {other:?}"),
        };
        // Two non-empty lines (`A` and `B`) get 1 underline stroke each; blank line gets none.
        assert_eq!(m_under_n, m_plain_n + 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rich_mtext_draws_inline_fonts_heights_colors_and_wraps() {
        use crate::model::{Object, ObjectColor, Point3};
        use crate::settings::Settings;
        use crate::text::truetype::tests::made_up_font;
        use crate::text::MTextRun;

        let dir =
            std::env::temp_dir().join(format!("agentdrafter-rich-mtext-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("madeup.ttf"),
            made_up_font(&[
                ('A', 700, [50, 0, 650, 700]),
                ('B', 700, [50, 0, 650, 700]),
                (' ', 300, [0, 0, 0, 0]),
            ]),
        )
        .unwrap();

        let mtext = MText {
            contents: "AB BA".into(),
            runs: vec![
                MTextRun {
                    text: "AB".into(),
                    font: Some("madeup.ttf".into()),
                    height: Some(4.0),
                    underline: true,
                    color: Some(ObjectColor::Index(1)),
                    ..MTextRun::default()
                },
                MTextRun {
                    text: " ".into(),
                    ..MTextRun::default()
                },
                MTextRun {
                    text: "BA".into(),
                    bold: true,
                    italic: true,
                    ..MTextRun::default()
                },
            ],
            position: Point3::new(0.0, 20.0, 0.0),
            height: 2.0,
            width: 0.0,
            style: "Standard".into(),
            width_factor: None,
            rotation: 0.0,
            justify: Attachment::TopLeft,
            underline: false,
            overline: false,
            layer: "0".into(),
            field: None,
            annotative: false,
            annotation_scales: Vec::new(),
            properties: Default::default(),
        };
        let mut sheet =
            crate::description::parse_sheet_description("units: mm\nobjects: []\n").unwrap();
        sheet.objects.push(Object::MText(mtext.clone()));
        let settings = Settings {
            font_folders: vec![dir.clone()],
            ..Settings::default()
        };
        let fonts = SheetFonts::find(&sheet, &settings);

        let drawn = draw_mtext(&mtext, &fonts, 0.25, 0.01);
        // First run uses TrueType (`madeup.ttf`) + inline color 1 -> filled shape in `colored`.
        assert_eq!(drawn.colored.len(), 1);
        assert_eq!(drawn.colored[0].0, Color(1));
        assert!(matches!(drawn.colored[0].1, Shape::Fill { .. }));
        // Third run uses Hershey with bold stroke (0.25 * 1.5 = 0.375) in default `shapes`.
        assert_eq!(drawn.shapes.len(), 1);
        match &drawn.shapes[0] {
            Shape::Stroke { lineweight, .. } => assert!((lineweight - 0.375).abs() < 1e-9),
            other => panic!("expected bold stroke: {other:?}"),
        }
        // Searchable text runs are emitted for every non-empty segment.
        let joined: String = drawn
            .runs
            .iter()
            .flat_map(|r| r.characters.iter().map(|(c, _)| *c))
            .collect();
        assert_eq!(joined, "AB BA");

        // Box height reflects the taller 4.0 run on the line.
        let bx = mtext_box(&mtext, &fonts);
        assert!(bx.height >= 4.0 - 1e-9, "height={}", bx.height);

        // Wrapping breaks between words across styled spans when width is tight.
        let wrapped = MText {
            width: 6.0,
            ..mtext
        };
        assert_eq!(mtext_lines(&wrapped, &fonts), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
