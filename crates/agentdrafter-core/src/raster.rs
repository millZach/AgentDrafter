//! Pixels from a display list: the drawing pipeline's last step for a PNG.
//!
//! The display list is drawn on the CPU by tiny-skia (BSD-3-Clause), onto
//! white paper, anti-aliased, and written as a PNG by the png crate (MIT or
//! Apache-2.0) with the output's stamp in a text chunk. Like the display
//! list, this is plain computation: no window, GPU, file system or thread.

use tiny_skia::{
    FillRule as SkiaFillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform,
};

use crate::draw::{Bounds, DisplayList, FillRule, Point2, Shape};

/// The PNG text chunk an output's stamp is written in.
pub(crate) const STAMP_KEYWORD: &str = "AgentDrafter";

/// Where a drawing sits on the image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct View {
    /// The drawing point at the image's top left corner.
    pub top_left: Point2,
    /// How many pixels one drawing unit spans.
    pub pixels_per_unit: f64,
    /// How many pixels one millimetre of paper spans, for lineweights.
    pub pixels_per_millimetre: f64,
    pub width: u32,
    pub height: u32,
}

impl View {
    /// The view that shows `extents` with `margin` pixels round it, at
    /// `pixels_per_unit`, centred in an image of the given size or, without
    /// one, an image just large enough.
    pub(crate) fn around(
        extents: Bounds,
        pixels_per_unit: f64,
        pixels_per_millimetre: f64,
        margin: f64,
        size: Option<(u32, u32)>,
    ) -> View {
        let drawn = (
            extents.width() * pixels_per_unit,
            extents.height() * pixels_per_unit,
        );
        let (width, height) = size.unwrap_or((
            (drawn.0 + 2.0 * margin).ceil().max(1.0) as u32,
            (drawn.1 + 2.0 * margin).ceil().max(1.0) as u32,
        ));
        // Centre the drawing in the image.
        let left = (f64::from(width) - drawn.0) / 2.0;
        let top = (f64::from(height) - drawn.1) / 2.0;
        View {
            top_left: [
                extents.min[0] - left / pixels_per_unit,
                extents.max[1] + top / pixels_per_unit,
            ],
            pixels_per_unit,
            pixels_per_millimetre,
            width,
            height,
        }
    }

    fn pixel(&self, p: Point2) -> (f32, f32) {
        (
            ((p[0] - self.top_left[0]) * self.pixels_per_unit) as f32,
            ((self.top_left[1] - p[1]) * self.pixels_per_unit) as f32,
        )
    }
}

/// Draws a display list onto white paper. None when the image cannot be
/// made, such as one with no pixels.
/// Paints white everything in `pixmap`, drawn through `view`, that lies
/// outside `keep`, in drawing units: what lies off a sheet's paper, where
/// an image is larger than the paper it fits.
pub(crate) fn blank_outside(pixmap: &mut Pixmap, view: &View, keep: Bounds) {
    let (left, top) = view.pixel([keep.min[0], keep.max[1]]);
    let (right, bottom) = view.pixel([keep.max[0], keep.min[1]]);
    let (w, h) = (view.width as f32, view.height as f32);
    let mut paint = Paint::default();
    paint.set_color(tiny_skia::Color::WHITE);
    for (x0, y0, x1, y1) in [
        (0.0, 0.0, w, top),
        (0.0, bottom, w, h),
        (0.0, top, left, bottom),
        (right, top, w, bottom),
    ] {
        if let Some(rect) = tiny_skia::Rect::from_ltrb(x0, y0.max(0.0), x1, y1.min(h)) {
            pixmap.fill_rect(rect, &paint, Transform::identity(), None);
        }
    }
}

pub(crate) fn rasterize(list: &DisplayList, view: &View) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(view.width, view.height)?;
    pixmap.fill(tiny_skia::Color::WHITE);
    for item in &list.items {
        let [r, g, b] = item.color.on_paper();
        let mut paint = Paint::default();
        paint.set_color_rgba8(r, g, b, 255);
        paint.anti_alias = true;
        match &item.shape {
            Shape::Stroke { lineweight, runs } => {
                // The finest a pen draws is one pixel, as a plotter draws
                // lineweight 0 with its finest line.
                let width = (lineweight * view.pixels_per_millimetre).max(1.0) as f32;
                let stroke = Stroke {
                    width,
                    line_cap: LineCap::Round,
                    line_join: LineJoin::Round,
                    ..Stroke::default()
                };
                let mut lines = PathBuilder::new();
                let mut dots = PathBuilder::new();
                for run in runs {
                    let points: Vec<(f32, f32)> = run.iter().map(|&p| view.pixel(p)).collect();
                    let Some(&first) = points.first() else {
                        continue;
                    };
                    if points.iter().all(|&p| p == first) {
                        dots.push_circle(first.0, first.1, width / 2.0);
                        continue;
                    }
                    lines.move_to(first.0, first.1);
                    for &(x, y) in &points[1..] {
                        lines.line_to(x, y);
                    }
                }
                if let Some(path) = lines.finish() {
                    pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
                }
                if let Some(path) = dots.finish() {
                    pixmap.fill_path(
                        &path,
                        &paint,
                        SkiaFillRule::Winding,
                        Transform::identity(),
                        None,
                    );
                }
            }
            Shape::Fill { rule, contours } => {
                let mut area = PathBuilder::new();
                for contour in contours {
                    let mut points = contour.iter().map(|&p| view.pixel(p));
                    let Some(first) = points.next() else {
                        continue;
                    };
                    area.move_to(first.0, first.1);
                    for (x, y) in points {
                        area.line_to(x, y);
                    }
                    area.close();
                }
                let rule = match rule {
                    FillRule::EvenOdd => SkiaFillRule::EvenOdd,
                    FillRule::NonZero => SkiaFillRule::Winding,
                };
                if let Some(path) = area.finish() {
                    pixmap.fill_path(&path, &paint, rule, Transform::identity(), None);
                }
            }
        }
    }
    Some(pixmap)
}

/// The bytes of a PNG of `pixmap`, opaque RGB, with `stamp` in a text chunk
/// and the image's resolution in dots per metre.
pub(crate) fn png(
    pixmap: &Pixmap,
    stamp: &str,
    pixels_per_millimetre: f64,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, pixmap.width(), pixmap.height());
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let per_metre = (pixels_per_millimetre * 1000.0).round() as u32;
        encoder.set_pixel_dims(Some(png::PixelDimensions {
            xppu: per_metre,
            yppu: per_metre,
            unit: png::Unit::Meter,
        }));
        encoder
            .add_itxt_chunk(STAMP_KEYWORD.to_owned(), stamp.to_owned())
            .map_err(|e| e.to_string())?;
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        // The paper is opaque, so the premultiplied pixels are the colours.
        let rgb: Vec<u8> = pixmap
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|&[r, g, b, _]| [r, g, b])
            .collect();
        writer.write_image_data(&rgb).map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())?;
    }
    Ok(bytes)
}

/// The stamp written in a PNG's text chunk, if it has one.
pub(crate) fn stamp_in(bytes: &[u8]) -> Option<String> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let reader = decoder.read_info().ok()?;
    let info = reader.info();
    info.utf8_text
        .iter()
        .find(|chunk| chunk.keyword == STAMP_KEYWORD)
        .and_then(|chunk| chunk.get_text().ok())
        .or_else(|| {
            info.uncompressed_latin1_text
                .iter()
                .find(|chunk| chunk.keyword == STAMP_KEYWORD)
                .map(|chunk| chunk.text.clone())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::{draw_sheet, MOST_HATCH_DASHES};
    use crate::settings::Settings;
    use crate::text::placement::SheetFonts;

    /// The runs of text the display list carries for the PDF's searchable
    /// text draw nothing: a PNG of the list is the same pixels with them or
    /// without them.
    #[test]
    fn the_runs_of_text_draw_nothing() {
        let sheet = crate::parse_sheet_description(
            r#"units: mm
objects:
  - text: {contents: SEARCHABLE, position: [0, 0, 0], height: 5}
  - mtext: {contents: "TWO\nLINES", position: [0, -5, 0], height: 3, rotation: 30}
"#,
        )
        .unwrap();
        let fonts = SheetFonts::find(&sheet, &Settings::default());
        let list = draw_sheet(&sheet, &fonts, 0.01, MOST_HATCH_DASHES);
        assert_eq!(list.text.len(), 3);
        let view = View::around(list.extents().unwrap(), 20.0, 6.0, 10.0, None);
        let with = rasterize(&list, &view).unwrap();
        let without = DisplayList {
            text: Vec::new(),
            ..list.clone()
        };
        let without = rasterize(&without, &view).unwrap();
        assert_eq!(with.data(), without.data());
        assert!(
            with.data().iter().any(|&c| c < 128),
            "the text itself draws"
        );
    }
}
