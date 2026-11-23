//! A display list made ready for the GPU: every stroke's segments, each
//! with its pen's width and colour, and every fill cut into triangles, each
//! with its item's place in the display list, so the GPU can draw a later
//! item over an earlier one, as the PNG paints them. The boundary of each
//! clipped viewport is cut into a fan of triangles too, which the GPU marks
//! the stencil with, so that the viewport's items draw only inside it.
//!
//! This is plain computation, tested natively. Each segment and corner has
//! two colours: the one it plots in on paper, the core's own
//! [`Color::on_paper`], so a capture draws each colour index as the PNG
//! does; and the one it shows in on the dark model space, index 7 white,
//! which the page draws with ([`screen_rgba`]). The widths stay
//! in millimetres of paper, so the GPU draws them at any zoom as the PNG
//! draws them at its dots per inch.

use crate::tools::Reach;
use agentdrafter_core::{Bounds, Color, DisplayList, DisplaySnap, FillRule, Paper, Shape};
use lyon_tessellation::geom::point;
use lyon_tessellation::path::Path;
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillRule as LyonFillRule, FillTessellator, FillVertex,
    VertexBuffers,
};
use serde::{Deserialize, Serialize};

/// The margin a render leaves round a drawing with no paper, in millimetres
/// of paper: the viewer's sheet is the PNG's white.
pub const MARGIN_MM: f64 = 5.0;

/// What the viewer reads of a `display-list` reply. Anything else in it is
/// passed over.
#[derive(Debug, Deserialize)]
pub struct Reply {
    pub ok: bool,
    #[serde(default)]
    pub result: Option<SheetReply>,
    #[serde(default)]
    pub error: Option<serde_json::Value>,
}

/// The sheet a `display-list` reply carries.
#[derive(Debug, Deserialize)]
pub struct SheetReply {
    pub source: String,
    #[serde(default)]
    pub name: String,
    pub sheet: String,
    pub sheets: Vec<String>,
    /// On a sheet drawn in several layouts, the layout drawn.
    #[serde(default)]
    pub layout: Option<String>,
    /// On a sheet drawn in several layouts, every one of them, in order.
    #[serde(default)]
    pub layouts: Vec<String>,
    /// The drawing's unit, such as `in` or `mm`; none for a drawing that
    /// states none, such as a DWG whose INSUNITS is 0.
    #[serde(default)]
    pub units: Option<String>,
    pub millimetres_per_unit: f64,
    #[serde(default)]
    pub paper: Option<Paper>,
    #[serde(default)]
    pub extents: Option<Bounds>,
    /// The journal's entry the set is drawn as it was at, if one was asked
    /// for.
    #[serde(default)]
    pub entry: Option<u64>,
    /// What each of the sheet's objects is, by its place, passed to the
    /// page as the core gives it: its ID, name, type and layer, and an
    /// insert's block and attributes.
    #[serde(default)]
    pub objects: Vec<serde_json::Value>,
    /// The object snaps, when the page asked for them: every point the
    /// snaps find on the objects the page draws itself.
    #[serde(default)]
    pub snaps: Vec<DisplaySnap>,
    /// With the snaps, the places among the display list's items of the
    /// fills whose edges the nearest snap finds a point on.
    #[serde(default)]
    pub edges: Vec<usize>,
    /// With `origins`, a drawing set's sheet's template and the sheets
    /// built from it, which an edit to its objects reaches.
    #[serde(default)]
    pub template: Option<Reach>,
    /// With `origins`, on a sheet drawn in a layout, the space an object
    /// drawn on the page goes in.
    #[serde(default)]
    pub space: Option<String>,
    /// With a change set, its preview: what it would do, and on the sheet
    /// drawn what it would add, change and delete and the checks it would
    /// fail ([`crate::preview`]).
    #[serde(default)]
    pub preview: Option<serde_json::Value>,
    pub display_list: DisplayList,
}

/// The clip of a segment drawn whole: a box larger than any sheet.
pub const NO_CLIP: [f32; 4] = [-1e30, -1e30, 1e30, 1e30];

/// What a run of items draws: stroke segments or fill triangles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Strokes,
    Fills,
}

/// Items next to one another in the display list that draw the same kind
/// of shape, seen through the same clipped viewport or none, so one draw
/// call draws them, in their order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    pub kind: Kind,
    /// The run's first segment, or first triangle.
    pub first: u32,
    /// How many segments, or triangles, it draws.
    pub count: u32,
    /// The clipped viewport's boundary its items are cut at: its place in
    /// the mesh's `stencils`. None for items drawn whole or cut at a box.
    pub clip: Option<u32>,
}

/// Items next to one another in the display list seen through the same
/// clipped viewport, or none: what a sheet drawn by depth draws at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    /// The boundary its items are cut at, as a run's is.
    pub clip: Option<u32>,
    /// Its first segment and how many.
    pub segments: (u32, u32),
    /// Its first triangle and how many.
    pub triangles: (u32, u32),
}

/// A sheet ready to draw.
#[derive(Debug, PartialEq)]
pub struct Mesh {
    /// The drawing point every position here is measured from, so that
    /// positions keep their precision as 32-bit floats.
    pub origin: [f64; 2],
    /// Each stroke segment: its two ends, `[x0, y0, x1, y1]`, from the
    /// origin. A dot is a segment whose ends are the same.
    pub segments: Vec<[f32; 4]>,
    /// Each segment's lineweight, in millimetres of paper.
    pub widths: Vec<f32>,
    /// Each segment's colour on paper, opaque RGBA.
    pub segment_colors: Vec<[u8; 4]>,
    /// Each segment's colour on a dark screen, opaque RGBA.
    pub segment_screen_colors: Vec<[u8; 4]>,
    /// Each segment's item: its place in the display list, counted from 0.
    pub segment_items: Vec<u32>,
    /// The box each segment's ink is cut at, `[left, bottom, right, top]`
    /// from the origin: its item's clip, a viewport's window, or
    /// [`NO_CLIP`] for an item drawn whole.
    pub segment_clips: Vec<[f32; 4]>,
    /// Every fill's triangles' corners, from the origin.
    pub fill_points: Vec<[f32; 2]>,
    /// Each corner's colour on paper, opaque RGBA.
    pub fill_colors: Vec<[u8; 4]>,
    /// Each corner's colour on a dark screen, opaque RGBA.
    pub fill_screen_colors: Vec<[u8; 4]>,
    /// Each corner's item: its place in the display list, counted from 0.
    pub fill_items: Vec<u32>,
    /// Three corners to each triangle.
    pub triangles: Vec<[u32; 3]>,
    /// How many fills could not be cut into triangles, and are not drawn.
    pub fills_not_drawn: usize,
    /// How many items the display list holds.
    pub items: usize,
    /// The display list's items as runs of one kind, in its order: drawn
    /// one after another, a later item paints over an earlier one.
    pub runs: Vec<Run>,
    /// The display list's items as spans by the clipped viewport they are
    /// seen through, in its order.
    pub spans: Vec<Span>,
    /// The fan of triangles each clipped viewport's boundary is cut into,
    /// among the fill triangles after every item's: its first triangle and
    /// how many. Drawn into the stencil, each inverting it, they leave it
    /// set inside the boundary, by the even-odd rule.
    pub stencils: Vec<(u32, u32)>,
    /// The sheet of paper the drawing sits on, in drawing units:
    /// `[left, bottom, right, top]`.
    pub paper: [f64; 4],
}

/// What a loaded sheet is, for the page.
#[derive(Debug, Serialize, PartialEq)]
pub struct Summary {
    pub source: String,
    pub name: String,
    pub sheet: String,
    pub sheets: Vec<String>,
    /// On a sheet drawn in several layouts, the layout drawn.
    pub layout: Option<String>,
    /// On a sheet drawn in several layouts, every one of them, in order.
    pub layouts: Vec<String>,
    /// The drawing's unit, such as `in` or `mm`; none when it states none.
    pub units: Option<String>,
    pub millimetres_per_unit: f64,
    /// `[left, bottom, right, top]` in drawing units.
    pub paper: [f64; 4],
    /// Whether the sheet declares its paper, by name or by size; a sheet
    /// that does not is drawn on its extents.
    pub paper_declared: bool,
    /// The paper's name, when the sheet declares it by name, such as
    /// `ANSI B landscape`; none for paper declared by size alone, or none.
    pub paper_name: Option<String>,
    pub segments: usize,
    pub triangles: usize,
    pub fills_not_drawn: usize,
    /// The journal's entry the set is drawn as it was at, if one was asked
    /// for.
    pub entry: Option<u64>,
    /// What each of the sheet's objects is, by its place.
    pub objects: Vec<serde_json::Value>,
    /// How many object snaps the sheet came with.
    pub snaps: usize,
    /// With `origins`, its template and the sheets built from it.
    pub template: Option<Reach>,
    /// With a change set, its preview as the core gives it, for the page's
    /// words, without what the renderer alone draws: the items it adds and
    /// changes and what the sheet draws now.
    pub preview: Option<serde_json::Value>,
}

/// Reads a `display-list` reply. A reply that is not ok gives its error's
/// message.
pub fn read_reply(text: &str) -> Result<SheetReply, String> {
    let reply: Reply = serde_json::from_str(text)
        .map_err(|e| format!("the display list could not be read: {e}"))?;
    match (reply.ok, reply.result) {
        (true, Some(sheet)) => Ok(sheet),
        _ => Err(reply
            .error
            .as_ref()
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .map_or_else(|| "the display list was refused".to_owned(), str::to_owned)),
    }
}

impl Mesh {
    /// The mesh of a sheet: every stroke segment and every fill triangle,
    /// in the display list's order.
    pub fn of(sheet: &SheetReply) -> Mesh {
        let paper = paper(sheet);
        let origin = [(paper[0] + paper[2]) / 2.0, (paper[1] + paper[3]) / 2.0];
        let at = |p: [f64; 2]| [(p[0] - origin[0]) as f32, (p[1] - origin[1]) as f32];
        let mut mesh = Mesh {
            origin,
            segments: Vec::new(),
            widths: Vec::new(),
            segment_colors: Vec::new(),
            segment_screen_colors: Vec::new(),
            segment_items: Vec::new(),
            segment_clips: Vec::new(),
            fill_points: Vec::new(),
            fill_colors: Vec::new(),
            fill_screen_colors: Vec::new(),
            fill_items: Vec::new(),
            triangles: Vec::new(),
            fills_not_drawn: 0,
            items: sheet.display_list.items.len(),
            runs: Vec::new(),
            spans: Vec::new(),
            stencils: Vec::new(),
            paper,
        };
        let mut tessellator = FillTessellator::new();
        for (place, item) in sheet.display_list.items.iter().enumerate() {
            let place = place as u32;
            let color = rgba(item.color);
            let screen = screen_rgba(item.color);
            // Cut at a clipped viewport's boundary, by the stencil.
            let cut = item
                .clip_boundary
                .filter(|&b| b < sheet.display_list.clip_boundaries.len())
                .map(|b| b as u32);
            let clip = item.clip.map_or(NO_CLIP, |b| {
                let (min, max) = (at(b.min), at(b.max));
                [min[0], min[1], max[0], max[1]]
            });
            let (segments, triangles) = (mesh.segments.len(), mesh.triangles.len());
            match &item.shape {
                Shape::Stroke { lineweight, runs } => {
                    for run in runs {
                        let mut add = |a: [f64; 2], b: [f64; 2]| {
                            let (a, b) = (at(a), at(b));
                            mesh.segments.push([a[0], a[1], b[0], b[1]]);
                            mesh.widths.push(*lineweight as f32);
                            mesh.segment_colors.push(color);
                            mesh.segment_screen_colors.push(screen);
                            mesh.segment_items.push(place);
                            mesh.segment_clips.push(clip);
                        };
                        match run.as_slice() {
                            [] => {}
                            [dot] => add(*dot, *dot),
                            points => {
                                for pair in points.windows(2) {
                                    add(pair[0], pair[1]);
                                }
                            }
                        }
                    }
                }
                Shape::Fill { rule, contours } => {
                    let mut builder = Path::builder();
                    for contour in contours.iter().filter(|c| c.len() >= 3) {
                        let mut points = contour.iter().map(|&p| {
                            let p = at(p);
                            point(p[0], p[1])
                        });
                        if let Some(first) = points.next() {
                            builder.begin(first);
                            for p in points {
                                builder.line_to(p);
                            }
                            builder.end(true);
                        }
                    }
                    let path = builder.build();
                    let rule = match rule {
                        FillRule::EvenOdd => LyonFillRule::EvenOdd,
                        FillRule::NonZero => LyonFillRule::NonZero,
                    };
                    let mut buffers: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
                    let done = tessellator.tessellate_path(
                        &path,
                        &FillOptions::default().with_fill_rule(rule),
                        &mut BuffersBuilder::new(&mut buffers, |v: FillVertex| {
                            v.position().to_array()
                        }),
                    );
                    if done.is_err() {
                        mesh.fills_not_drawn += 1;
                        continue;
                    }
                    let base = mesh.fill_points.len() as u32;
                    mesh.fill_colors
                        .extend(std::iter::repeat_n(color, buffers.vertices.len()));
                    mesh.fill_screen_colors
                        .extend(std::iter::repeat_n(screen, buffers.vertices.len()));
                    mesh.fill_items
                        .extend(std::iter::repeat_n(place, buffers.vertices.len()));
                    mesh.fill_points.extend(buffers.vertices);
                    mesh.triangles.extend(
                        buffers
                            .indices
                            .as_chunks::<3>()
                            .0
                            .iter()
                            .map(|t| t.map(|corner| base + corner)),
                    );
                }
                Shape::Mask { contours } => {
                    let mut builder = Path::builder();
                    for contour in contours.iter().filter(|c| c.len() >= 3) {
                        let mut points = contour.iter().map(|&p| {
                            let p = at(p);
                            point(p[0], p[1])
                        });
                        if let Some(first) = points.next() {
                            builder.begin(first);
                            for p in points {
                                builder.line_to(p);
                            }
                            builder.end(true);
                        }
                    }
                    let path = builder.build();
                    let mut buffers: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
                    let done = tessellator.tessellate_path(
                        &path,
                        &FillOptions::default().with_fill_rule(LyonFillRule::EvenOdd),
                        &mut BuffersBuilder::new(&mut buffers, |v: FillVertex| {
                            v.position().to_array()
                        }),
                    );
                    if done.is_err() {
                        mesh.fills_not_drawn += 1;
                        continue;
                    }
                    let base = mesh.fill_points.len() as u32;
                    mesh.fill_colors.extend(std::iter::repeat_n(
                        [255, 255, 255, 255],
                        buffers.vertices.len(),
                    ));
                    mesh.fill_screen_colors.extend(std::iter::repeat_n(
                        [0x21, 0x28, 0x30, 255],
                        buffers.vertices.len(),
                    ));
                    mesh.fill_items
                        .extend(std::iter::repeat_n(place, buffers.vertices.len()));
                    mesh.fill_points.extend(buffers.vertices);
                    mesh.triangles.extend(
                        buffers
                            .indices
                            .as_chunks::<3>()
                            .0
                            .iter()
                            .map(|t| t.map(|corner| base + corner)),
                    );
                }
            }
            let added = [
                (Kind::Strokes, segments, mesh.segments.len()),
                (Kind::Fills, triangles, mesh.triangles.len()),
            ];
            for (kind, before, after) in added {
                if after > before {
                    mesh.extend_run(kind, before as u32, (after - before) as u32, cut);
                }
            }
            let (segments_now, triangles_now) = (mesh.segments.len(), mesh.triangles.len());
            match mesh.spans.last_mut() {
                Some(span) if span.clip == cut => {
                    span.segments.1 = segments_now as u32 - span.segments.0;
                    span.triangles.1 = triangles_now as u32 - span.triangles.0;
                }
                _ => mesh.spans.push(Span {
                    clip: cut,
                    segments: (segments as u32, (segments_now - segments) as u32),
                    triangles: (triangles as u32, (triangles_now - triangles) as u32),
                }),
            }
        }
        // Each clipped viewport's boundary, a fan from its first corner,
        // after every item's triangles, drawn into the stencil alone.
        for outline in &sheet.display_list.clip_boundaries {
            let first = mesh.triangles.len() as u32;
            let base = mesh.fill_points.len() as u32;
            mesh.fill_points.extend(outline.iter().map(|&p| at(p)));
            mesh.fill_colors
                .extend(std::iter::repeat_n([0, 0, 0, 0], outline.len()));
            mesh.fill_screen_colors
                .extend(std::iter::repeat_n([0, 0, 0, 0], outline.len()));
            mesh.fill_items
                .extend(std::iter::repeat_n(0, outline.len()));
            let corners = outline.len() as u32;
            mesh.triangles
                .extend((1..corners.saturating_sub(1)).map(|i| [base, base + i, base + i + 1]));
            mesh.stencils
                .push((first, mesh.triangles.len() as u32 - first));
        }
        mesh
    }

    /// Adds `count` segments or triangles of `kind`, from `first`, cut at
    /// the boundary `clip` or none, to the last run when it is of that kind
    /// and cut there, or as a run of their own.
    fn extend_run(&mut self, kind: Kind, first: u32, count: u32, clip: Option<u32>) {
        match self.runs.last_mut() {
            Some(run) if run.kind == kind && run.clip == clip && run.first + run.count == first => {
                run.count += count;
            }
            _ => self.runs.push(Run {
                kind,
                first,
                count,
                clip,
            }),
        }
    }

    /// What the sheet is, for the page.
    pub fn summary(&self, sheet: &SheetReply) -> Summary {
        // The paper the sheet declares, as `paper` draws it.
        let declared = sheet
            .paper
            .as_ref()
            .filter(|p| p.width > 0.0 && p.height > 0.0);
        Summary {
            source: sheet.source.clone(),
            name: sheet.name.clone(),
            sheet: sheet.sheet.clone(),
            sheets: sheet.sheets.clone(),
            layout: sheet.layout.clone(),
            layouts: sheet.layouts.clone(),
            units: sheet.units.clone(),
            millimetres_per_unit: sheet.millimetres_per_unit,
            paper: self.paper,
            paper_declared: declared.is_some(),
            paper_name: declared.and_then(|p| p.name.clone()),
            segments: self.segments.len(),
            triangles: self.triangles.len(),
            fills_not_drawn: self.fills_not_drawn,
            entry: sheet.entry,
            objects: sheet.objects.clone(),
            snaps: sheet.snaps.len(),
            template: sheet.template.clone(),
            preview: sheet.preview.as_ref().map(|preview| {
                let mut words = preview.clone();
                if let Some(map) = words.as_object_mut() {
                    for drawn in ["added", "changed", "was"] {
                        map.remove(drawn);
                    }
                }
                words
            }),
        }
    }
}

/// A colour index as it draws on paper, opaque.
fn rgba(color: Color) -> [u8; 4] {
    let [r, g, b] = color.on_paper();
    [r, g, b, 255]
}

/// A colour index as the dark model space shows it, opaque: index 7 white,
/// and the other standard colours, 1 to 9, as the picked look of the frame
/// (#150) shows them, lifted where AutoCAD's own would be hard to read on
/// the dark ground, its pure blue above all; every other index in its own
/// colour.
pub fn screen_rgba(color: Color) -> [u8; 4] {
    let [r, g, b] = match color.0 {
        1 => [0xff, 0x4a, 0x4a],
        2 => [0xff, 0xff, 0x3a],
        3 => [0x3a, 0xff, 0x3a],
        4 => [0x3a, 0xff, 0xff],
        5 => [0x4f, 0x7c, 0xff],
        6 => [0xff, 0x4a, 0xff],
        7 => [0xff, 0xff, 0xff],
        8 => [0x8a, 0x8f, 0x96],
        9 => [0xc0, 0xc0, 0xc0],
        _ => color.rgb(),
    };
    [r, g, b, 255]
}

/// The sheet of paper, `[left, bottom, right, top]`: the paper the record
/// declares, or else the extents with the margin a render leaves round
/// them, as the PNG's white is.
pub fn paper(sheet: &SheetReply) -> [f64; 4] {
    if let Some(paper) = &sheet.paper {
        if paper.width > 0.0 && paper.height > 0.0 {
            return [0.0, 0.0, paper.width, paper.height];
        }
    }
    let margin = if sheet.millimetres_per_unit > 0.0 {
        MARGIN_MM / sheet.millimetres_per_unit
    } else {
        MARGIN_MM
    };
    match sheet.extents.or_else(|| sheet.display_list.extents()) {
        Some(b) => [
            b.min[0] - margin,
            b.min[1] - margin,
            b.max[0] + margin,
            b.max[1] + margin,
        ],
        None => [-margin, -margin, margin, margin],
    }
}

#[cfg(test)]
mod tests {
    use agentdrafter_core::{display_list_with, DisplayListRequest, Item, Settings};

    use super::*;

    fn sheet(items: Vec<Item>, paper: Option<Paper>) -> SheetReply {
        let display_list = DisplayList {
            items,
            ..DisplayList::default()
        };
        SheetReply {
            source: "made-up.yaml".to_owned(),
            name: "made-up.yaml".to_owned(),
            sheet: "made-up".to_owned(),
            sheets: vec!["made-up".to_owned()],
            layout: None,
            layouts: Vec::new(),
            units: None,
            millimetres_per_unit: 1.0,
            paper,
            extents: display_list.extents(),
            entry: None,
            objects: Vec::new(),
            snaps: Vec::new(),
            edges: Vec::new(),
            template: None,
            space: None,
            preview: None,
            display_list,
        }
    }

    fn item(color: u8, shape: Shape) -> Item {
        Item {
            object: 0,
            part: None,
            nested: Vec::new(),
            cell: None,
            viewport: None,
            clip: None,
            clip_boundary: None,
            color: Color(color),
            shape,
        }
    }

    /// The area the triangles cover, in square drawing units.
    fn area(mesh: &Mesh) -> f64 {
        mesh.triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| mesh.fill_points[i as usize].map(f64::from));
                ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() / 2.0
            })
            .sum()
    }

    #[test]
    fn a_segment_seen_through_a_viewport_is_cut_at_its_window() {
        let mut clipped = item(
            1,
            Shape::Stroke {
                lineweight: 2.11,
                runs: vec![vec![[0.0, 5.0], [10.0, 5.0]]],
            },
        );
        clipped.clip = Some(agentdrafter_core::Bounds {
            min: [0.0, 0.0],
            max: [10.0, 10.0],
        });
        let whole = item(
            7,
            Shape::Stroke {
                lineweight: 0.5,
                runs: vec![vec![[0.0, 0.0], [10.0, 0.0]]],
            },
        );
        let mesh = Mesh::of(&sheet(vec![clipped, whole], None));
        // The window, from the origin, the paper's middle, [5, 2.5].
        assert_eq!(mesh.origin, [5.0, 2.5]);
        assert_eq!(mesh.segment_clips, vec![[-5.0, -2.5, 5.0, 7.5], NO_CLIP]);
    }

    #[test]
    fn a_clipped_viewports_items_draw_through_its_boundarys_fan() {
        let stroke = |color, cut| {
            let mut i = item(
                color,
                Shape::Stroke {
                    lineweight: 0.5,
                    runs: vec![vec![[0.0, 5.0], [10.0, 5.0]]],
                },
            );
            if cut {
                i.clip = Some(agentdrafter_core::Bounds {
                    min: [0.0, 0.0],
                    max: [10.0, 10.0],
                });
                i.clip_boundary = Some(0);
            }
            i
        };
        let mut reply = sheet(
            vec![
                stroke(7, false),
                stroke(1, true),
                stroke(1, true),
                stroke(7, false),
            ],
            None,
        );
        // A notched outline of five corners: a fan of three triangles.
        reply.display_list.clip_boundaries = vec![vec![
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 10.0],
            [5.0, 5.0],
            [0.0, 10.0],
        ]];
        let mesh = Mesh::of(&reply);
        // The clipped items are a run of their own, between the others.
        let cut: Vec<Option<u32>> = mesh.runs.iter().map(|r| r.clip).collect();
        assert_eq!(cut, [None, Some(0), None]);
        assert_eq!(mesh.runs[1].count, 2);
        // The fan follows every item's triangles, from the first corner.
        assert_eq!(mesh.stencils, [(0, 3)]);
        assert_eq!(mesh.triangles.len(), 3);
        assert_eq!(mesh.fill_items.len(), mesh.fill_points.len());
        // Spans for a sheet drawn by depth, by the boundary they are cut at.
        let spans: Vec<(Option<u32>, (u32, u32))> =
            mesh.spans.iter().map(|s| (s.clip, s.segments)).collect();
        assert_eq!(spans, [(None, (0, 1)), (Some(0), (1, 2)), (None, (3, 1))]);
    }

    #[test]
    fn a_stroke_is_its_segments_each_with_its_pen_and_its_colour_on_paper() {
        let sheet = sheet(
            vec![
                item(
                    7,
                    Shape::Stroke {
                        lineweight: 0.5,
                        runs: vec![
                            vec![[0.0, 0.0], [10.0, 0.0], [10.0, 5.0]],
                            vec![[2.0, 2.0]],
                            vec![],
                        ],
                    },
                ),
                item(
                    1,
                    Shape::Stroke {
                        lineweight: 0.0,
                        runs: vec![vec![[0.0, 5.0], [10.0, 5.0]]],
                    },
                ),
            ],
            None,
        );
        let mesh = Mesh::of(&sheet);
        // The paper is the extents with a 5 mm margin; the origin its
        // middle.
        assert_eq!(mesh.paper, [-5.0, -5.0, 15.0, 10.0]);
        assert_eq!(mesh.origin, [5.0, 2.5]);
        assert_eq!(
            mesh.segments,
            vec![
                [-5.0, -2.5, 5.0, -2.5],
                [5.0, -2.5, 5.0, 2.5],
                // A run of one point is a dot: a segment with no length.
                [-3.0, -0.5, -3.0, -0.5],
                [-5.0, 2.5, 5.0, 2.5],
            ]
        );
        assert_eq!(mesh.widths, vec![0.5, 0.5, 0.5, 0.0]);
        // On paper index 7 plots black; on the dark screen it shows white.
        assert_eq!(mesh.segment_colors[0], [0, 0, 0, 255]);
        assert_eq!(mesh.segment_screen_colors[0], [255, 255, 255, 255]);
        assert_eq!(mesh.segment_colors[3], [255, 0, 0, 255]);
        assert_eq!(mesh.segment_screen_colors[3], [0xff, 0x4a, 0x4a, 255]);
        // The pure blue the PNG plots is lifted on the dark screen, and an
        // index past the standard nine shows in its own colour.
        assert_eq!(screen_rgba(Color(5)), [0x4f, 0x7c, 0xff, 255]);
        assert_eq!(rgba(Color(5)), [0, 0, 255, 255]);
        assert_eq!(screen_rgba(Color(30)), [255, 127, 0, 255]);
        assert_eq!(mesh.segment_screen_colors.len(), mesh.segments.len());
        // Each segment knows its item's place, so the later item draws over
        // the earlier.
        assert_eq!(mesh.segment_items, vec![0, 0, 0, 1]);
        assert_eq!(mesh.items, 2);
        // Strokes after strokes are one run, one draw call.
        assert_eq!(
            mesh.runs,
            vec![Run {
                kind: Kind::Strokes,
                first: 0,
                count: 4,
                clip: None,
            }]
        );
        // Index 7 draws black on paper, as the PNG draws it; 1 is red.
        assert_eq!(
            mesh.segment_colors,
            vec![
                [0, 0, 0, 255],
                [0, 0, 0, 255],
                [0, 0, 0, 255],
                [255, 0, 0, 255]
            ]
        );
        assert!(mesh.triangles.is_empty());
    }

    #[test]
    fn a_fill_is_cut_into_triangles_by_its_rule() {
        let square = |size: f64| vec![[0.0, 0.0], [size, 0.0], [size, size], [0.0, size]];
        let hole: Vec<[f64; 2]> = square(4.0)
            .into_iter()
            .map(|p| [p[0] + 3.0, p[1] + 3.0])
            .collect();
        let fill = |rule| {
            item(
                5,
                Shape::Fill {
                    rule,
                    contours: vec![square(10.0), hole.clone()],
                },
            )
        };
        let paper = Some(Paper {
            name: None,
            width: 10.0,
            height: 10.0,
        });
        // An island inside a hatch's boundary is left out.
        let even_odd = Mesh::of(&sheet(vec![fill(FillRule::EvenOdd)], paper.clone()));
        assert_eq!(even_odd.paper, [0.0, 0.0, 10.0, 10.0]);
        assert!((area(&even_odd) - (100.0 - 16.0)).abs() < 1e-3);
        assert!(even_odd.fill_colors.iter().all(|&c| c == [0, 0, 255, 255]));
        assert_eq!(even_odd.fill_colors.len(), even_odd.fill_points.len());
        // Two contours wound the same way are filled whole by a TrueType
        // outline's rule.
        let non_zero = Mesh::of(&sheet(vec![fill(FillRule::NonZero)], paper));
        assert!((area(&non_zero) - 100.0).abs() < 1e-3);
        assert_eq!(non_zero.fills_not_drawn, 0);
        // A stroke after a fill is a later item than every corner of it.
        let line = item(
            1,
            Shape::Stroke {
                lineweight: 0.5,
                runs: vec![vec![[0.0, 5.0], [10.0, 5.0]]],
            },
        );
        let both = Mesh::of(&sheet(vec![fill(FillRule::EvenOdd), line], None));
        assert!(both.fill_items.iter().all(|&place| place == 0));
        // A fill, then a stroke: two runs, drawn in that order.
        assert_eq!(
            both.runs,
            vec![
                Run {
                    kind: Kind::Fills,
                    first: 0,
                    count: both.triangles.len() as u32,
                    clip: None,
                },
                Run {
                    kind: Kind::Strokes,
                    first: 0,
                    count: 1,
                    clip: None,
                },
            ]
        );
        assert_eq!(both.fill_items.len(), both.fill_points.len());
        assert_eq!(both.segment_items, vec![1]);
    }

    #[test]
    fn paper_declared_by_size_alone_is_told_from_none_and_no_unit_is_carried_when_none_is() {
        let line = || {
            item(
                7,
                Shape::Stroke {
                    lineweight: 0.0,
                    runs: vec![vec![[0.0, 0.0], [10.0, 5.0]]],
                },
            )
        };
        // Paper declared by its size alone: declared, with no name.
        let sized = sheet(
            vec![line()],
            Some(Paper {
                name: None,
                width: 17.0,
                height: 11.0,
            }),
        );
        let summary = Mesh::of(&sized).summary(&sized);
        assert!(summary.paper_declared);
        assert_eq!(summary.paper_name, None);
        assert_eq!(summary.paper, [0.0, 0.0, 17.0, 11.0]);
        // No paper: drawn on its extents, not declared; and no unit stated.
        let bare = sheet(vec![line()], None);
        let summary = Mesh::of(&bare).summary(&bare);
        assert!(!summary.paper_declared);
        assert_eq!(summary.units, None);
    }

    #[test]
    fn a_reply_that_is_not_ok_gives_its_message() {
        let refused = r#"{"ok":false,"error":{"code":"unknown_name","message":"there is no sheet `X` to draw"}}"#;
        assert_eq!(
            read_reply(refused).unwrap_err(),
            "there is no sheet `X` to draw"
        );
        assert!(read_reply("not json")
            .unwrap_err()
            .starts_with("the display list could not be read"));
    }

    /// The core's reply for a demo plant loop sheet reads whole, and every
    /// point of every run becomes part of a segment.
    #[test]
    fn a_demo_plant_sheet_reads_from_the_cores_reply() {
        let set = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/demo-plant");
        let mut request = DisplayListRequest::new(set);
        request.sheet = Some("dp-ld-201".to_owned());
        let report = display_list_with(&request, &Settings::default()).unwrap();
        let reply = serde_json::json!({ "ok": true, "result": report }).to_string();
        let sheet = read_reply(&reply).unwrap();
        assert_eq!(sheet.sheet, "DP-LD-201");
        assert_eq!(sheet.sheets.len(), 8);
        let mesh = Mesh::of(&sheet);
        assert_eq!(mesh.paper, [0.0, 0.0, 17.0, 11.0]);
        let expected: usize = report
            .display_list
            .items
            .iter()
            .map(|item| match &item.shape {
                Shape::Stroke { runs, .. } => runs
                    .iter()
                    .map(|run| run.len().saturating_sub(1).max(usize::from(run.len() == 1)))
                    .sum(),
                Shape::Fill { .. } | Shape::Mask { .. } => 0,
            })
            .sum();
        assert!(expected > 1000, "a loop sheet draws {expected} segments");
        assert_eq!(mesh.segments.len(), expected);
        // A loop sheet draws only strokes: one run, one draw call.
        assert_eq!(mesh.runs.len(), 1);
        let summary = mesh.summary(&sheet);
        assert_eq!(summary.segments, expected);
        assert_eq!(summary.millimetres_per_unit, 25.4);
        assert_eq!(summary.paper_name.as_deref(), Some("ANSI B landscape"));
        assert!(summary.paper_declared);
        assert_eq!(summary.units.as_deref(), Some("in"));
    }

    #[test]
    fn wipeout_mask_tessellates_in_background_color() {
        let mask = item(
            1,
            Shape::Mask {
                contours: vec![vec![[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [0.0, 2.0]]],
            },
        );
        let mesh = Mesh::of(&sheet(vec![mask], None));
        assert!((area(&mesh) - 8.0).abs() < 1e-4);
        assert_eq!(mesh.fill_colors.first(), Some(&[255, 255, 255, 255]));
        assert_eq!(
            mesh.fill_screen_colors.first(),
            Some(&[0x21, 0x28, 0x30, 255])
        );
    }
}
