//! A display list made ready for the GPU: every stroke's segments, each
//! with its pen's width and colour, and every fill cut into triangles, each
//! with its item's place in the display list, so the GPU can draw a later
//! item over an earlier one, as the PNG paints them.
//!
//! This is plain computation, tested natively. The colours are the core's
//! own, [`Color::on_paper`], so the screen draws each colour index as the
//! PNG does; the widths stay in millimetres of paper, so the GPU draws them
//! at any zoom as the PNG draws them at its dots per inch.

use agentdrafter_core::{Bounds, Color, DisplayList, FillRule, Paper, Shape};
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
    pub millimetres_per_unit: f64,
    #[serde(default)]
    pub paper: Option<Paper>,
    #[serde(default)]
    pub extents: Option<Bounds>,
    pub display_list: DisplayList,
}

/// What a run of items draws: stroke segments or fill triangles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Strokes,
    Fills,
}

/// Items next to one another in the display list that draw the same kind
/// of shape, so one draw call draws them, in their order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    pub kind: Kind,
    /// The run's first segment, or first triangle.
    pub first: u32,
    /// How many segments, or triangles, it draws.
    pub count: u32,
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
    /// Each segment's item: its place in the display list, counted from 0.
    pub segment_items: Vec<u32>,
    /// Every fill's triangles' corners, from the origin.
    pub fill_points: Vec<[f32; 2]>,
    /// Each corner's colour on paper, opaque RGBA.
    pub fill_colors: Vec<[u8; 4]>,
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
    pub millimetres_per_unit: f64,
    /// `[left, bottom, right, top]` in drawing units.
    pub paper: [f64; 4],
    pub segments: usize,
    pub triangles: usize,
    pub fills_not_drawn: usize,
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
            segment_items: Vec::new(),
            fill_points: Vec::new(),
            fill_colors: Vec::new(),
            fill_items: Vec::new(),
            triangles: Vec::new(),
            fills_not_drawn: 0,
            items: sheet.display_list.items.len(),
            runs: Vec::new(),
            paper,
        };
        let mut tessellator = FillTessellator::new();
        for (place, item) in sheet.display_list.items.iter().enumerate() {
            let place = place as u32;
            let color = rgba(item.color);
            let (segments, triangles) = (mesh.segments.len(), mesh.triangles.len());
            match &item.shape {
                Shape::Stroke { lineweight, runs } => {
                    for run in runs {
                        let mut add = |a: [f64; 2], b: [f64; 2]| {
                            let (a, b) = (at(a), at(b));
                            mesh.segments.push([a[0], a[1], b[0], b[1]]);
                            mesh.widths.push(*lineweight as f32);
                            mesh.segment_colors.push(color);
                            mesh.segment_items.push(place);
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
                    mesh.extend_run(kind, before as u32, (after - before) as u32);
                }
            }
        }
        mesh
    }

    /// Adds `count` segments or triangles of `kind`, from `first`, to the
    /// last run when it is of that kind, or as a run of their own.
    fn extend_run(&mut self, kind: Kind, first: u32, count: u32) {
        match self.runs.last_mut() {
            Some(run) if run.kind == kind && run.first + run.count == first => run.count += count,
            _ => self.runs.push(Run { kind, first, count }),
        }
    }

    /// What the sheet is, for the page.
    pub fn summary(&self, sheet: &SheetReply) -> Summary {
        Summary {
            source: sheet.source.clone(),
            name: sheet.name.clone(),
            sheet: sheet.sheet.clone(),
            sheets: sheet.sheets.clone(),
            millimetres_per_unit: sheet.millimetres_per_unit,
            paper: self.paper,
            segments: self.segments.len(),
            triangles: self.triangles.len(),
            fills_not_drawn: self.fills_not_drawn,
        }
    }
}

/// A colour index as it draws on paper, opaque.
fn rgba(color: Color) -> [u8; 4] {
    let [r, g, b] = color.on_paper();
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
            millimetres_per_unit: 1.0,
            paper,
            extents: display_list.extents(),
            display_list,
        }
    }

    fn item(color: u8, shape: Shape) -> Item {
        Item {
            object: 0,
            part: None,
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
                count: 4
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
                    count: both.triangles.len() as u32
                },
                Run {
                    kind: Kind::Strokes,
                    first: 0,
                    count: 1
                },
            ]
        );
        assert_eq!(both.fill_items.len(), both.fill_points.len());
        assert_eq!(both.segment_items, vec![1]);
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
                Shape::Fill { .. } => 0,
            })
            .sum();
        assert!(expected > 1000, "a loop sheet draws {expected} segments");
        assert_eq!(mesh.segments.len(), expected);
        // A loop sheet draws only strokes: one run, one draw call.
        assert_eq!(mesh.runs.len(), 1);
        let summary = mesh.summary(&sheet);
        assert_eq!(summary.segments, expected);
        assert_eq!(summary.millimetres_per_unit, 25.4);
    }
}
