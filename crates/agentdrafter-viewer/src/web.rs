//! The module the page loads: the mesh drawn with WebGL2.
//!
//! The page owns the view: which drawing point sits at the canvas's top
//! left corner, and how many CSS pixels a drawing unit spans. Each frame it
//! calls [`Viewer::draw`] with those; the GPU is given two numbers and the
//! mesh it already holds, so a pan or a zoom costs nothing to lay out.
//!
//! - Items are painted in the display list's order, as the PNG paints
//!   them: a later item covers an earlier one, and nothing is blended.
//!   Items next to one another that draw strokes, or fills, are drawn by
//!   one draw call, run after run; the GPU applies each draw's triangles,
//!   and each draw, in order. A sheet whose strokes and fills alternate
//!   more than 64 times is drawn instead in two draws, every item at a
//!   depth by its place in the list, a later one nearer, so the depth test
//!   keeps the later item's colour in every sample.
//! - Strokes are drawn as one draw: each segment a quad exactly as wide as
//!   the pen and long enough for its round ends, so a pen is as wide as
//!   the PNG draws it. The samples the quad's sides cover are the pen's;
//!   at a round end the shader works out the share of the pixel the pen
//!   covers, by its distance, and that share of the samples is written
//!   (alpha to coverage). Every sample is written opaque, so where one
//!   stroke's segments overlap at a corner they cover the same samples
//!   once, and the canvas's multisampling smooths the edges. The quads are
//!   plain triangles, not instances, which software WebGL draws several
//!   times faster.
//!   A segment of model space seen through a viewport carries the
//!   viewport's window, and the shader draws none of its ink outside it,
//!   as the PNG and the PDF cut it.
//! - A viewport clipped to a boundary draws through the stencil: the
//!   boundary's fan of triangles is drawn into it first, each inverting
//!   it, which leaves it set inside the boundary by the even-odd rule, and
//!   the viewport's strokes and fills then draw only where it is set, ink
//!   and all, as the PNG's mask and the PDF's clipping path cut them.
//! - Fills are drawn as one draw of their triangles, their edges smoothed
//!   by the multisampling.
//! - Under the ink, as AutoCAD's model space looks on screen: a dark
//!   background, the grid when the page turns it on, and the paper as a
//!   dashed outline, all from one draw of a quad over the canvas whose
//!   shader works out each pixel. Each colour index shows in its screen
//!   colour, index 7 white. A capture for a test draws instead as the PNG
//!   does: white all round, and each index in the colour it plots in.
//! - Over everything, for the drafter's cursor: AutoCAD's crosshair and
//!   pick box where the page puts the cursor, and the AutoSnap marker of
//!   the object snap it caught, which this module finds among the snaps
//!   the display list came with, the running ones, with no round trip
//!   ([`crate::snapping`]); Tab steps to the next in reach. They are
//!   triangles in device pixels, made anew each frame ([`crate::overlay`]).
//! - The drafter's drawing tool, while one runs ([`crate::tools`]): it takes
//!   the clicks, taps and typing the page passes on, says what it wants,
//!   and gives the change set to send; this module draws its rubber band
//!   in the current layer's colour, its guides dashed, the objects a move
//!   would move where they would go, a copy following the cursor dashed
//!   and the copies Copy has placed dashed in the proposal's colour, the
//!   points it has taken, and a halo round each object it has picked.
//! - Over the ink, a halo marks each object a change set added or changed,
//!   green or amber, and the object the reviewer picked, blue, as the
//!   picked prototype marks them; the page fades the change set's halos in
//!   and out as it lands. A halo is a box round what the object draws,
//!   found by the core from the display list, as a pick is.
//! - A change set drawn before it lands, when the reply is a preview
//!   ([`crate::preview`]): over the ink, the sheet faded with model space's
//!   colour, as REFEDIT fades what is not edited; then what the change set
//!   would add in green and change in amber, each in a dashed box, the old
//!   place of what it changes faint with a dashed guide where it only
//!   moves, what it deletes faint red and struck, and a red revision cloud
//!   round each check the sheet would fail.

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{
    HtmlCanvasElement, WebGl2RenderingContext as Gl, WebGlBuffer, WebGlProgram, WebGlShader,
    WebGlUniformLocation, WebGlVertexArrayObject,
};

use agentdrafter_core::{
    object_bounds, outlines, pick, Bounds, Color, DisplayList, DisplaySnap, Outline, Shape,
};

use crate::ghosts::{self, Ghost};
use crate::grid::{first_line, grid};
use crate::mesh::{read_reply, screen_rgba, Kind, Mesh, Run, Span};
use crate::overlay::{crosshair, marker, triangles, Segment};
use crate::preview::{self, Preview};
use crate::snapping::{candidates, Candidate};
use crate::tools::{tool_for, Context, ObjectInfo, SnapRef, Taken, Tools};

const STROKE_VERTEX: &str = r#"#version 300 es
precision highp float;
layout(location = 0) in vec4 a_segment;
layout(location = 1) in float a_lineweight;
layout(location = 2) in vec4 a_color;
layout(location = 3) in float a_item;
layout(location = 4) in vec4 a_clip;
uniform vec2 u_scale;
uniform vec2 u_offset;
uniform vec2 u_size;
uniform float u_items;
uniform float u_pixels_per_mm;
uniform float u_finest;
out vec2 v_at;
flat out vec2 v_a;
flat out vec2 v_b;
flat out float v_half;
flat out vec3 v_color;
flat out vec4 v_clip;
void main() {
  vec2 a = a_segment.xy * u_scale + u_offset;
  // The box the segment's ink is cut at, on the canvas.
  vec2 c0 = a_clip.xy * u_scale + u_offset;
  vec2 c1 = a_clip.zw * u_scale + u_offset;
  v_clip = vec4(min(c0, c1), max(c0, c1));
  vec2 b = a_segment.zw * u_scale + u_offset;
  float half_width = max(a_lineweight * u_pixels_per_mm, u_finest) * 0.5;
  // Along the segment, room for the round ends and their smoothing; across
  // it, the pen's width exactly, so the samples its sides cover are the
  // samples the pen covers.
  float reach = half_width + 1.0;
  vec2 along = b - a;
  float length_ = length(along);
  vec2 dir = length_ > 1e-6 ? along / length_ : vec2(1.0, 0.0);
  vec2 across = vec2(-dir.y, dir.x);
  // The quad's corner: the index is four to a segment.
  int corner = gl_VertexID & 3;
  vec2 p = ((corner & 1) == 0 ? a - dir * reach : b + dir * reach)
         + across * ((corner & 2) == 0 ? -half_width : half_width);
  v_at = p;
  v_a = a;
  v_b = b;
  v_half = half_width;
  v_color = a_color.rgb;
  // A later item is nearer, so it covers an earlier one.
  float depth = 1.0 - 2.0 * (a_item + 1.0) / (u_items + 1.0);
  gl_Position = vec4(p.x / u_size.x * 2.0 - 1.0, 1.0 - p.y / u_size.y * 2.0, depth, 1.0);
}
"#;

const STROKE_FRAGMENT: &str = r#"#version 300 es
precision highp float;
in vec2 v_at;
flat in vec2 v_a;
flat in vec2 v_b;
flat in float v_half;
flat in vec3 v_color;
flat in vec4 v_clip;
uniform bool u_multisampled;
out vec4 o_color;
void main() {
  // A viewport's contents draw no ink past its edge.
  if (v_at.x < v_clip.x || v_at.y < v_clip.y || v_at.x > v_clip.z || v_at.y > v_clip.w) discard;
  vec2 pa = v_at - v_a;
  vec2 ba = v_b - v_a;
  float length2 = dot(ba, ba);
  float t = length2 > 1e-12 ? dot(pa, ba) / length2 : -1.0;
  // Beside the segment, the quad's own edges are the pen's: every sample
  // the quad covers is ink.
  if (t >= 0.0 && t <= 1.0) {
    o_color = vec4(v_color, 1.0);
    return;
  }
  // At a round end, the share of the pixel the pen covers, by its distance.
  float d = length(pa - ba * clamp(t, 0.0, 1.0));
  float coverage = clamp(v_half + 0.5 - d, 0.0, 1.0);
  // Without samples to share out, a pixel is the pen's when it covers half.
  if (coverage <= 0.0 || (!u_multisampled && coverage < 0.5)) discard;
  o_color = vec4(v_color, coverage);
}
"#;

const FILL_VERTEX: &str = r#"#version 300 es
precision highp float;
layout(location = 0) in vec2 a_point;
layout(location = 1) in vec4 a_color;
layout(location = 2) in float a_item;
uniform vec2 u_scale;
uniform vec2 u_offset;
uniform vec2 u_size;
uniform float u_items;
flat out vec3 v_color;
void main() {
  vec2 p = a_point * u_scale + u_offset;
  v_color = a_color.rgb;
  float depth = 1.0 - 2.0 * (a_item + 1.0) / (u_items + 1.0);
  gl_Position = vec4(p.x / u_size.x * 2.0 - 1.0, 1.0 - p.y / u_size.y * 2.0, depth, 1.0);
}
"#;

const FILL_FRAGMENT: &str = r#"#version 300 es
precision highp float;
flat in vec3 v_color;
out vec4 o_color;
void main() {
  o_color = vec4(v_color, 1.0);
}
"#;

const RECT_VERTEX: &str = r#"#version 300 es
precision highp float;
uniform vec4 u_rect;
uniform vec2 u_size;
void main() {
  int corner = gl_VertexID;
  vec2 p = vec2((corner & 1) == 0 ? u_rect.x : u_rect.z, (corner & 2) == 0 ? u_rect.y : u_rect.w);
  gl_Position = vec4(p.x / u_size.x * 2.0 - 1.0, 1.0 - p.y / u_size.y * 2.0, 0.0, 1.0);
}
"#;

const OVERLAY_VERTEX: &str = r#"#version 300 es
precision highp float;
layout(location = 0) in vec2 a_point;
uniform vec2 u_size;
void main() {
  gl_Position = vec4(a_point.x / u_size.x * 2.0 - 1.0, 1.0 - a_point.y / u_size.y * 2.0, 0.0, 1.0);
}
"#;

/// AutoSnap's yellow, the markers' colour.
const SNAP_YELLOW: [f32; 4] = [
    0xf2 as f32 / 255.0,
    0xe2 as f32 / 255.0,
    0x4b as f32 / 255.0,
    1.0,
];

/// The dark edge round a marker, so it reads on any ink.
const MARKER_EDGE: [f32; 4] = [0.0, 0.0, 0.0, 0.75];

/// The crosshair's colour on model space.
const CROSSHAIR: [f32; 4] = [238.0 / 255.0, 241.0 / 255.0, 244.0 / 255.0, 0.92];

/// What a move would move, drawn where it would go, pale blue, as the
/// picked prototype draws it.
const MOVING: [f32; 4] = [0xd6 as f32 / 255.0, 0xec as f32 / 255.0, 1.0, 0.85];

/// A tool's guides, dashed: from a circle's centre, or a move's base point.
const GUIDE: [f32; 4] = [
    0xc8 as f32 / 255.0,
    0xd0 as f32 / 255.0,
    0xda as f32 / 255.0,
    0.9,
];

/// The points a tool has taken, small squares in the frame's blue.
const TAKEN: [f32; 4] = [0x4a as f32 / 255.0, 0xa3 as f32 / 255.0, 1.0, 1.0];

/// The most segments of the objects a move would move drawn at once.
const MOST_MOVED: usize = 50_000;

const RECT_FRAGMENT: &str = r#"#version 300 es
precision highp float;
uniform vec4 u_color;
out vec4 o_color;
void main() {
  o_color = u_color;
}
"#;

const BACKDROP_FRAGMENT: &str = r#"#version 300 es
precision highp float;
uniform vec2 u_size;
uniform vec3 u_background;
// The grid: where the first line falls from the canvas's left and top
// edges and the device pixels between lines, for the minor lines and the
// major; a step of 0 draws none.
uniform vec4 u_minor;
uniform vec4 u_major;
// How wide a line is, in device pixels.
uniform float u_line;
uniform vec3 u_minor_color;
uniform vec3 u_major_color;
// The paper's edge, [left, top, right, bottom] in device pixels, whole
// pixels, dashed: a line u_line wide just inside each side.
uniform vec4 u_paper;
uniform vec3 u_edge_color;
out vec4 o_color;
bool on_line(float at, float first, float step) {
  return step > 0.0 && mod(at - first + 0.5 * u_line, step) < u_line;
}
bool on_dash(float along) {
  return mod(along, u_line * 10.0) < u_line * 6.0;
}
void main() {
  vec2 p = vec2(gl_FragCoord.x, u_size.y - gl_FragCoord.y);
  vec3 color = u_background;
  if (on_line(p.x, u_minor.x, u_minor.z) || on_line(p.y, u_minor.y, u_minor.w)) color = u_minor_color;
  if (on_line(p.x, u_major.x, u_major.z) || on_line(p.y, u_major.y, u_major.w)) color = u_major_color;
  float l = u_paper.x;
  float t = u_paper.y;
  float r = u_paper.z;
  float b = u_paper.w;
  // Pixel centres sit at half pixels: each side takes the u_line rows or
  // columns that begin on its edge and run inwards, so the line is u_line
  // wide, one CSS pixel, not a pixel either side of the edge.
  bool across = p.x >= l && p.x < r;
  bool down = p.y >= t && p.y < b;
  bool top_or_bottom = (p.y >= t && p.y < t + u_line) || (p.y >= b - u_line && p.y < b);
  bool left_or_right = (p.x >= l && p.x < l + u_line) || (p.x >= r - u_line && p.x < r);
  bool edge = (across && top_or_bottom && on_dash(p.x - l)) || (down && left_or_right && on_dash(p.y - t));
  if (edge) color = u_edge_color;
  o_color = vec4(color, 1.0);
}
"#;

/// A colour as the shaders take it, from `0xrrggbb`.
const fn rgb(hex: u32) -> [f32; 3] {
    [
        ((hex >> 16) & 0xff) as f32 / 255.0,
        ((hex >> 8) & 0xff) as f32 / 255.0,
        (hex & 0xff) as f32 / 255.0,
    ]
}

/// Model space on the dark screen, the frame's canvas colour.
const MODEL_SPACE: [f32; 3] = rgb(0x21_28_30);

/// The grid's minor lines, and its major ones, a little stronger.
const GRID_MINOR: [f32; 3] = rgb(0x27_2f_39);
const GRID_MAJOR: [f32; 3] = rgb(0x31_3a_46);

/// The paper's dashed edge on model space.
const PAPER_EDGE: [f32; 3] = rgb(0x56_60_6e);

/// How the canvas looks: as the drafter sees model space on screen, or as
/// the PNG plots the sheet, for a test to compare.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Look {
    /// Dark model space, the grid when it is on, the paper a dashed
    /// outline, each colour index in its screen colour.
    Screen,
    /// White all round, each colour index in the colour it plots in.
    Paper,
}

/// The most runs of strokes and fills a sheet is drawn in one by one, in
/// order; a sheet whose strokes and fills alternate more often is drawn in
/// two draws, in order by depth.
const MOST_RUNS: usize = 64;

/// How far a halo reaches past what its object draws, in CSS pixels.
const HALO_PAD: f64 = 6.0;

/// How wide a halo's edge is drawn, in CSS pixels.
const HALO_EDGE: f64 = 2.0;

/// A halo's fill and edge, as the picked prototype draws them: an object a
/// change set added, one it changed, and the one the reviewer picked.
const HALOS: [([f32; 4], [f32; 4]); 3] = [
    ([0.133, 0.773, 0.369, 0.24], [0.086, 0.639, 0.290, 1.0]),
    ([0.961, 0.620, 0.043, 0.28], [0.851, 0.467, 0.024, 1.0]),
    ([0.231, 0.510, 0.965, 0.12], [0.145, 0.388, 0.922, 1.0]),
];

/// What a halo marks.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Halo {
    Added = 0,
    Changed = 1,
    Picked = 2,
}

/// A halo round one object's drawing.
struct Mark {
    bounds: Bounds,
    halo: Halo,
}

/// A program and the places of its uniforms.
struct Program {
    program: WebGlProgram,
    uniforms: Vec<(&'static str, Option<WebGlUniformLocation>)>,
}

impl Program {
    fn new(gl: &Gl, vertex: &str, fragment: &str, names: &[&'static str]) -> Result<Self, JsError> {
        let program = link(gl, vertex, fragment)?;
        let uniforms = names
            .iter()
            .map(|&name| (name, gl.get_uniform_location(&program, name)))
            .collect();
        Ok(Self { program, uniforms })
    }

    fn at(&self, name: &str) -> Option<&WebGlUniformLocation> {
        self.uniforms
            .iter()
            .find(|(n, _)| *n == name)
            .and_then(|(_, at)| at.as_ref())
    }
}

/// A sheet held by the GPU.
struct Loaded {
    /// The display list, which picks and halos are found in.
    list: DisplayList,
    /// The object snaps the sheet came with.
    snaps: Vec<DisplaySnap>,
    /// The outlines of the fills the nearest snap finds a point on.
    outlines: Vec<Outline>,
    /// With a change set, how its preview is marked.
    preview: Option<Preview>,
    origin: [f64; 2],
    paper: [f64; 4],
    millimetres_per_unit: f64,
    /// How many items the display list holds, which sets each one's depth.
    items: f32,
    runs: Vec<Run>,
    spans: Vec<Span>,
    /// Each clipped viewport's boundary's fan: its first triangle and how
    /// many.
    stencils: Vec<(u32, u32)>,
    strokes: WebGlVertexArrayObject,
    fills: WebGlVertexArrayObject,
    /// The segments' colours and the fills' corners', on paper and on
    /// screen: the look picks which the vertex arrays read.
    stroke_colors: [WebGlBuffer; 2],
    fill_colors: [WebGlBuffer; 2],
    buffers: Vec<WebGlBuffer>,
}

/// The renderer the page draws its canvas with.
#[wasm_bindgen]
pub struct Viewer {
    gl: Gl,
    stroke: Program,
    fill: Program,
    rect: Program,
    backdrop: Program,
    overlay: Program,
    /// The crosshair's and the marker's triangles, made anew each frame.
    overlay_points: WebGlBuffer,
    overlay_array: WebGlVertexArrayObject,
    /// The object snaps the cursor catches, by their bits
    /// ([`crate::snapping::bit`]); 0 when snapping is off.
    running: u32,
    /// The snaps in reach of the cursor, in the order it takes them, and
    /// which of them it holds now.
    caught: Vec<Candidate>,
    chosen: usize,
    /// Where the crosshair is, in drawing units, and whether it has the pick
    /// box; none off the drawing.
    cursor: Option<([f64; 2], bool)>,
    /// Whether the crosshair and the marker are drawn: not in a capture.
    overlays: bool,
    /// Whether the canvas has samples to share a pixel's coverage out in.
    multisampled: bool,
    look: Look,
    /// Whether the grid is drawn behind the drawing, on screen.
    grid: bool,
    sheet: Option<Loaded>,
    /// The halos over the ink.
    marks: Vec<Mark>,
    /// How strongly a change set's halos show, from 0 to 1, as the page
    /// fades them.
    pulse: f32,
    /// The drafter's drawing tools.
    tools: Tools,
    /// The current layer's colour on screen, which a rubber band is drawn in.
    layer_color: [f32; 4],
    /// Where a held finger is, in drawing units, which a tool aims at on a
    /// phone, where there is no crosshair.
    tool_pointer: Option<[f64; 2]>,
    /// The proposals waiting on the sheet shown, drawn dashed over it.
    ghosts: Vec<Ghost>,
    /// The sheet loaded and its layout, which a ghost is drawn over only
    /// when it is its own.
    shown: (String, Option<String>),
}

#[wasm_bindgen]
impl Viewer {
    /// A renderer for `canvas`, through its WebGL2 context. An error when
    /// the browser has no WebGL2.
    #[wasm_bindgen(constructor)]
    pub fn new(canvas: HtmlCanvasElement) -> Result<Viewer, JsError> {
        let options = js_sys::Object::new();
        for (key, value) in [
            ("antialias", true),
            ("alpha", false),
            ("depth", true),
            ("stencil", true),
            ("preserveDrawingBuffer", false),
        ] {
            js_sys::Reflect::set(&options, &key.into(), &value.into())
                .map_err(|_| JsError::new("could not ask for a WebGL2 context"))?;
        }
        let gl: Gl = canvas
            .get_context_with_context_options("webgl2", &options)
            .ok()
            .flatten()
            .and_then(|context| context.dyn_into().ok())
            .ok_or_else(|| {
                JsError::new("this browser has no WebGL2, which the viewer draws with")
            })?;
        let stroke = Program::new(
            &gl,
            STROKE_VERTEX,
            STROKE_FRAGMENT,
            &[
                "u_scale",
                "u_offset",
                "u_size",
                "u_items",
                "u_pixels_per_mm",
                "u_finest",
                "u_multisampled",
            ],
        )?;
        let fill = Program::new(
            &gl,
            FILL_VERTEX,
            FILL_FRAGMENT,
            &["u_scale", "u_offset", "u_size", "u_items"],
        )?;
        let rect = Program::new(
            &gl,
            RECT_VERTEX,
            RECT_FRAGMENT,
            &["u_rect", "u_size", "u_color"],
        )?;
        let backdrop = Program::new(
            &gl,
            RECT_VERTEX,
            BACKDROP_FRAGMENT,
            &[
                "u_rect",
                "u_size",
                "u_background",
                "u_minor",
                "u_major",
                "u_line",
                "u_minor_color",
                "u_major_color",
                "u_paper",
                "u_edge_color",
            ],
        )?;
        let overlay = Program::new(&gl, OVERLAY_VERTEX, RECT_FRAGMENT, &["u_size", "u_color"])?;
        let overlay_points = gl
            .create_buffer()
            .ok_or_else(|| JsError::new("the GPU gave no buffer"))?;
        let overlay_array = gl
            .create_vertex_array()
            .ok_or_else(|| JsError::new("the GPU gave no vertex array"))?;
        gl.bind_vertex_array(Some(&overlay_array));
        gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&overlay_points));
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_with_i32(0, 2, Gl::FLOAT, false, 0, 0);
        gl.bind_vertex_array(None);
        let multisampled = gl
            .get_parameter(Gl::SAMPLES)
            .ok()
            .and_then(|samples| samples.as_f64())
            .is_some_and(|samples| samples > 1.0);
        Ok(Viewer {
            gl,
            stroke,
            fill,
            rect,
            backdrop,
            overlay,
            overlay_points,
            overlay_array,
            running: 0,
            caught: Vec::new(),
            chosen: 0,
            cursor: None,
            overlays: true,
            multisampled,
            look: Look::Screen,
            grid: true,
            sheet: None,
            marks: Vec::new(),
            pulse: 1.0,
            tools: Tools::default(),
            layer_color: CROSSHAIR,
            tool_pointer: None,
            ghosts: Vec::new(),
            shown: (String::new(), None),
        })
    }

    /// Loads a sheet from the text of a `display-list` reply, replacing the
    /// one held, and gives what it is as JSON: its sheet, the set's sheets,
    /// its paper and how many segments and triangles it draws.
    pub fn load(&mut self, reply: &str) -> Result<String, JsError> {
        let mut sheet = read_reply(reply).map_err(|message| JsError::new(&message))?;
        let mesh = Mesh::of(&sheet);
        let summary = serde_json::to_string(&mesh.summary(&sheet))
            .map_err(|e| JsError::new(&e.to_string()))?;
        self.unload();
        self.marks.clear();
        self.caught.clear();
        let snaps = std::mem::take(&mut sheet.snaps);
        let edges = std::mem::take(&mut sheet.edges);
        // An object whose drawing on the page is cut at a viewport's window
        // is model space seen through it: its points there are not its own.
        let mut objects: Vec<ObjectInfo> = sheet.objects.iter().map(ObjectInfo::of).collect();
        for item in &sheet.display_list.items {
            if item.clip.is_some() {
                if let Some(o) = objects.get_mut(item.object) {
                    o.through_viewport = true;
                }
            }
        }
        self.shown = (sheet.sheet.clone(), sheet.layout.clone());
        self.tools.load(Context {
            sheet: sheet.sheet.clone(),
            units: sheet.units.clone(),
            objects,
            template: sheet.template.clone(),
            past: sheet.entry.is_some(),
            space: sheet.space.clone(),
            layout: sheet.layout.clone(),
        });
        let ids: Vec<Option<String>> = sheet
            .objects
            .iter()
            .map(|o| o.get("id").and_then(|id| id.as_str()).map(str::to_owned))
            .collect();
        let shown = Preview::of(sheet.preview.as_ref(), &sheet.display_list, &ids);
        let mut loaded = self.upload(&mesh, sheet.millimetres_per_unit, sheet.display_list)?;
        loaded.preview = shown;
        loaded.snaps = snaps;
        loaded.outlines = outlines(&loaded.list, &edges);
        self.bind_colors(&loaded);
        self.sheet = Some(loaded);
        Ok(summary)
    }

    /// The colour the object at `object`, a place in the sheet's objects,
    /// shows in on screen, as `#rrggbb`: the colour of the first thing it
    /// draws. Empty when it draws nothing.
    pub fn object_color(&self, object: u32) -> String {
        self.sheet
            .as_ref()
            .and_then(|sheet| {
                sheet
                    .list
                    .items
                    .iter()
                    .find(|item| item.object == object as usize)
            })
            .map(|item| {
                let [r, g, b, _] = screen_rgba(item.color);
                format!("#{r:02x}{g:02x}{b:02x}")
            })
            .unwrap_or_default()
    }

    /// Which object snaps the cursor catches, by their bits: bit `i` for the
    /// `i`th of `SnapKind::ALL`, in the order `endpoint`, `midpoint`,
    /// `center`, `quadrant`, `insertion`, `node`, `intersection`,
    /// `apparent_intersection`, `perpendicular`, `tangent`, `nearest`; 0
    /// when Object snap (F3) is off.
    pub fn set_running(&mut self, running: u32) {
        self.running = running;
        if running == 0 {
            self.caught.clear();
        }
    }

    /// How many object snaps the sheet came with.
    pub fn snap_count(&self) -> u32 {
        self.sheet.as_ref().map_or(0, |s| s.snaps.len() as u32)
    }

    /// Finds the running snaps within `reach` drawing units of the drawing
    /// point `(x, y)`, nearest first, two within `tie` of each other in
    /// AutoCAD's order, and holds the first; gives how many there are.
    pub fn hover(&mut self, x: f64, y: f64, reach: f64, tie: f64) -> u32 {
        self.chosen = 0;
        // A tool picking objects catches no snaps: a click picks.
        if self.tools.selecting() {
            self.caught.clear();
            return 0;
        }
        self.caught = match &self.sheet {
            Some(sheet) => candidates(
                &sheet.snaps,
                &sheet.list,
                &sheet.outlines,
                self.running,
                [x, y],
                reach,
                tie,
            ),
            None => Vec::new(),
        };
        self.caught.len() as u32
    }

    /// Holds the next snap in reach, as Tab does, round to the first; gives
    /// its place among them.
    pub fn next_snap(&mut self) -> u32 {
        if !self.caught.is_empty() {
            self.chosen = (self.chosen + 1) % self.caught.len();
        }
        self.chosen as u32
    }

    /// Lets go of the snaps in reach: no marker.
    pub fn clear_hover(&mut self) {
        self.caught.clear();
        self.chosen = 0;
    }

    /// The snap the cursor holds, as JSON: its `snap`, its `object` by its
    /// place, `which` and `with` as `query snap` names them, `at` in drawing
    /// units, its `place` among those in reach and their `count`; empty
    /// when it holds none.
    pub fn hovered(&self) -> String {
        let Some(c) = self.caught.get(self.chosen) else {
            return String::new();
        };
        serde_json::json!({
            "snap": c.snap,
            "object": c.object,
            "which": c.which,
            "with": c.with,
            "at": c.at,
            "place": self.chosen,
            "count": self.caught.len(),
        })
        .to_string()
    }

    /// Puts the crosshair at the drawing point `(x, y)`, with the pick box
    /// in its middle when `pick_box`.
    pub fn set_crosshair(&mut self, x: f64, y: f64, pick_box: bool) {
        self.cursor = Some(([x, y], pick_box));
    }

    /// Takes the crosshair away, as when the mouse leaves the drawing.
    pub fn hide_crosshair(&mut self) {
        self.cursor = None;
    }

    /// Draws the crosshair and the marker over the drawing, or not, as for
    /// a capture.
    pub fn set_overlays(&mut self, on: bool) {
        self.overlays = on;
    }

    /// Draws the grid behind the drawing, or not, as the drafter's Grid
    /// (F7) says.
    pub fn set_grid(&mut self, on: bool) {
        self.grid = on;
    }

    /// The place in the sheet's objects of the object drawn nearest the
    /// drawing point `(x, y)`, within `reach` drawing units, as the core
    /// picks it from the display list; -1 when nothing is drawn that near.
    pub fn pick(&self, x: f64, y: f64, reach: f64) -> i32 {
        self.sheet
            .as_ref()
            .and_then(|sheet| pick(&sheet.list, [x, y], reach, sheet.millimetres_per_unit))
            .and_then(|object| i32::try_from(object).ok())
            .unwrap_or(-1)
    }

    /// Marks with halos the objects at `changed`, places in the sheet's
    /// objects, each added when its `added` is 1 and changed otherwise, and
    /// the object at `picked`, unless it is -1. An object that draws nothing
    /// is not marked.
    pub fn set_marks(&mut self, changed: &[u32], added: &[u8], picked: i32) {
        self.marks.clear();
        let Some(sheet) = &self.sheet else {
            return;
        };
        for (i, &object) in changed.iter().enumerate() {
            if let Some(bounds) = object_bounds(&sheet.list, object as usize) {
                let halo = if added.get(i) == Some(&1) {
                    Halo::Added
                } else {
                    Halo::Changed
                };
                self.marks.push(Mark { bounds, halo });
            }
        }
        if let Some(bounds) = usize::try_from(picked)
            .ok()
            .and_then(|object| object_bounds(&sheet.list, object))
        {
            self.marks.push(Mark {
                bounds,
                halo: Halo::Picked,
            });
        }
    }

    /// How strongly a change set's halos show, from 0 to 1.
    pub fn set_pulse(&mut self, strength: f64) {
        self.pulse = strength.clamp(0.0, 1.0) as f32;
    }

    /// The box round the objects at `objects`, places in the sheet's
    /// objects, `[left, bottom, right, top]` in drawing units; empty when
    /// none of them draws anything.
    pub fn bounds_of(&self, objects: &[u32]) -> Vec<f64> {
        let Some(sheet) = &self.sheet else {
            return Vec::new();
        };
        let boxes: Vec<Bounds> = objects
            .iter()
            .filter_map(|&object| object_bounds(&sheet.list, object as usize))
            .collect();
        let Some(first) = boxes.first() else {
            return Vec::new();
        };
        let all = boxes.iter().fold(*first, |a, b| Bounds {
            min: [a.min[0].min(b.min[0]), a.min[1].min(b.min[1])],
            max: [a.max[0].max(b.max[0]), a.max[1].max(b.max[1])],
        });
        vec![all.min[0], all.min[1], all.max[0], all.max[1]]
    }

    /// The box round everything a preview marks, `[left, bottom, right,
    /// top]` in drawing units, for the view to come to; empty when the
    /// sheet is no preview, or it marks nothing.
    pub fn preview_extent(&self) -> Vec<f64> {
        self.sheet
            .as_ref()
            .and_then(|sheet| sheet.preview.as_ref()?.extent(&sheet.list))
            .map(|b| vec![b.min[0], b.min[1], b.max[0], b.max[1]])
            .unwrap_or_default()
    }

    /// Draws proposal `proposal` waiting, dashed over the sheet shown, from
    /// the text of `display-list`'s reply with its change set on that
    /// sheet; fainter when `refused`, as the set would refuse it now. It
    /// takes the place of one of the same number. Gives the box round what
    /// it marks, `[left, bottom, right, top]` in drawing units, which its
    /// card is put by; empty when it marks nothing there.
    pub fn add_ghost(
        &mut self,
        proposal: u32,
        reply: &str,
        refused: bool,
    ) -> Result<Vec<f64>, JsError> {
        let ghost = Ghost::of(proposal, reply, refused).map_err(|m| JsError::new(&m))?;
        let extent = ghost
            .extent()
            .map(|b| vec![b.min[0], b.min[1], b.max[0], b.max[1]])
            .unwrap_or_default();
        self.ghosts.retain(|g| g.proposal != proposal);
        self.ghosts.push(ghost);
        Ok(extent)
    }

    /// Draws none of the proposals waiting.
    pub fn clear_ghosts(&mut self) {
        self.ghosts.clear();
    }

    /// The box round what proposal `proposal` marks on the sheet shown,
    /// `[left, bottom, right, top]` in drawing units; empty when it is not
    /// drawn there, or marks nothing.
    pub fn ghost_extent(&self, proposal: u32) -> Vec<f64> {
        let (sheet, layout) = &self.shown;
        self.ghosts
            .iter()
            .find(|g| g.proposal == proposal && g.on(sheet, layout.as_deref()))
            .and_then(Ghost::extent)
            .map(|b| vec![b.min[0], b.min[1], b.max[0], b.max[1]])
            .unwrap_or_default()
    }

    /// Draws as the PNG plots the sheet, white all round and each colour
    /// index in the colour it plots in, or, as the page shows it, as dark
    /// model space. A test draws as the PNG, to compare the canvas with one.
    pub fn set_paper_look(&mut self, paper: bool) {
        self.look = if paper { Look::Paper } else { Look::Screen };
        if let Some(sheet) = &self.sheet {
            self.bind_colors(sheet);
        }
    }

    /// Draws the sheet with the drawing point `(left, top)` at the canvas's
    /// top left corner, `scale` CSS pixels to a drawing unit, and `ratio`
    /// device pixels to a CSS pixel. The canvas's own width and height are
    /// its device pixels.
    pub fn draw(&self, left: f64, top: f64, scale: f64, ratio: f64) {
        let gl = &self.gl;
        let Some(canvas) = gl
            .canvas()
            .and_then(|c| c.dyn_into::<HtmlCanvasElement>().ok())
        else {
            return;
        };
        let (width, height) = (canvas.width() as i32, canvas.height() as i32);
        gl.viewport(0, 0, width, height);
        let [r, g, b] = match self.look {
            Look::Screen => MODEL_SPACE,
            Look::Paper => [1.0; 3],
        };
        gl.clear_color(r, g, b, 1.0);
        gl.clear_depth(1.0);
        gl.clear(Gl::COLOR_BUFFER_BIT);
        let Some(sheet) = &self.sheet else {
            return;
        };
        let size = (width as f32, height as f32);
        let pixels = scale * ratio;
        let device = |x: f64, y: f64| ((x - left) * pixels, (top - y) * pixels);

        // Under everything, opaque: on screen, model space with its grid
        // and the paper's dashed edge; as the PNG, white paper.
        gl.disable(Gl::BLEND);
        gl.disable(Gl::DEPTH_TEST);
        gl.disable(Gl::SAMPLE_ALPHA_TO_COVERAGE);
        let [x0, y0, x1, y1] = sheet.paper;
        let (left_px, top_px) = device(x0, y1);
        let (right_px, bottom_px) = device(x1, y0);
        match self.look {
            Look::Paper => self.rect(
                [left_px, top_px, right_px, bottom_px],
                [1.0, 1.0, 1.0, 1.0],
                size,
            ),
            Look::Screen => self.backdrop(
                sheet,
                (left, top, scale, ratio),
                [left_px, top_px, right_px, bottom_px],
                size,
            ),
        }

        // The ink, in the display list's order.
        let scale_xy = (pixels as f32, -pixels as f32);
        let offset = device(sheet.origin[0], sheet.origin[1]);
        let offset = (offset.0 as f32, offset.1 as f32);
        gl.use_program(Some(&self.fill.program));
        gl.uniform2f(self.fill.at("u_scale"), scale_xy.0, scale_xy.1);
        gl.uniform2f(self.fill.at("u_offset"), offset.0, offset.1);
        gl.uniform2f(self.fill.at("u_size"), size.0, size.1);
        gl.uniform1f(self.fill.at("u_items"), sheet.items);
        gl.use_program(Some(&self.stroke.program));
        gl.uniform2f(self.stroke.at("u_scale"), scale_xy.0, scale_xy.1);
        gl.uniform2f(self.stroke.at("u_offset"), offset.0, offset.1);
        gl.uniform2f(self.stroke.at("u_size"), size.0, size.1);
        gl.uniform1f(self.stroke.at("u_items"), sheet.items);
        gl.uniform1i(
            self.stroke.at("u_multisampled"),
            i32::from(self.multisampled),
        );
        let per_mm = pixels / sheet.millimetres_per_unit.max(f64::MIN_POSITIVE);
        gl.uniform1f(self.stroke.at("u_pixels_per_mm"), per_mm as f32);
        // The finest pen is one CSS pixel wide, as a PNG's is one pixel.
        gl.uniform1f(self.stroke.at("u_finest"), ratio as f32);
        let draw = |kind: Kind, first: u32, count: u32| match kind {
            Kind::Fills => {
                gl.use_program(Some(&self.fill.program));
                gl.bind_vertex_array(Some(&sheet.fills));
                gl.draw_elements_with_i32(
                    Gl::TRIANGLES,
                    (count * 3) as i32,
                    Gl::UNSIGNED_INT,
                    (first * 3 * 4) as i32,
                );
            }
            Kind::Strokes => {
                gl.use_program(Some(&self.stroke.program));
                gl.bind_vertex_array(Some(&sheet.strokes));
                gl.enable(Gl::SAMPLE_ALPHA_TO_COVERAGE);
                gl.draw_elements_with_i32(
                    Gl::TRIANGLES,
                    (count * 6) as i32,
                    Gl::UNSIGNED_INT,
                    (first * 6 * 4) as i32,
                );
                gl.disable(Gl::SAMPLE_ALPHA_TO_COVERAGE);
            }
        };
        // Sets the stencil inside the clipped viewport's boundary `b`, and
        // lets what is drawn next draw only there; none draws whole.
        let cut_at = |b: Option<u32>| {
            let Some(&(first, count)) = b.and_then(|b| sheet.stencils.get(b as usize)) else {
                gl.disable(Gl::STENCIL_TEST);
                return;
            };
            let depth = gl.is_enabled(Gl::DEPTH_TEST);
            gl.enable(Gl::STENCIL_TEST);
            gl.clear_stencil(0);
            gl.clear(Gl::STENCIL_BUFFER_BIT);
            gl.color_mask(false, false, false, false);
            gl.depth_mask(false);
            gl.disable(Gl::DEPTH_TEST);
            gl.stencil_func(Gl::ALWAYS, 0, 0xff);
            gl.stencil_op(Gl::KEEP, Gl::KEEP, Gl::INVERT);
            draw(Kind::Fills, first, count);
            gl.color_mask(true, true, true, true);
            gl.depth_mask(true);
            if depth {
                gl.enable(Gl::DEPTH_TEST);
            }
            gl.stencil_func(Gl::EQUAL, 0xff, 0xff);
            gl.stencil_op(Gl::KEEP, Gl::KEEP, Gl::KEEP);
        };
        if sheet.runs.len() <= MOST_RUNS {
            // Run after run, as the display list orders them: what is drawn
            // later covers what was drawn before, within a draw call and
            // across them.
            let mut cut: Option<u32> = None;
            for run in &sheet.runs {
                if run.clip != cut {
                    cut_at(run.clip);
                    cut = run.clip;
                }
                draw(run.kind, run.first, run.count);
            }
        } else {
            // Too many runs to draw one by one: every fill, then every
            // stroke, each at its item's depth, so each sample keeps the
            // colour of the latest item drawn on it, which is the nearest;
            // span by span, each seen through one clipped viewport or none.
            gl.clear(Gl::DEPTH_BUFFER_BIT);
            gl.enable(Gl::DEPTH_TEST);
            gl.depth_func(Gl::LEQUAL);
            for span in &sheet.spans {
                cut_at(span.clip);
                if span.triangles.1 > 0 {
                    draw(Kind::Fills, span.triangles.0, span.triangles.1);
                }
                if span.segments.1 > 0 {
                    draw(Kind::Strokes, span.segments.0, span.segments.1);
                }
            }
        }
        gl.disable(Gl::STENCIL_TEST);
        gl.disable(Gl::DEPTH_TEST);
        gl.bind_vertex_array(None);

        // A preview's marks, over the ink, on screen; or, on the sheet as
        // it is, the proposals waiting on it.
        if let (Some(preview), Look::Screen) = (&sheet.preview, self.look) {
            self.draw_preview(sheet, preview, &device, (pixels, ratio), size);
        } else if self.look == Look::Screen && !self.ghosts.is_empty() {
            self.draw_ghosts(sheet, &device, (pixels, ratio), size);
        }

        // The halos, over the ink, blended.
        if !self.marks.is_empty() {
            gl.enable(Gl::BLEND);
            gl.blend_func(Gl::SRC_ALPHA, Gl::ONE_MINUS_SRC_ALPHA);
            let pad = HALO_PAD * ratio;
            let edge = (HALO_EDGE * ratio).max(1.0);
            for mark in &self.marks {
                let (left_px, top_px) = device(mark.bounds.min[0], mark.bounds.max[1]);
                let (right_px, bottom_px) = device(mark.bounds.max[0], mark.bounds.min[1]);
                let [l, t, r, b] = [left_px - pad, top_px - pad, right_px + pad, bottom_px + pad];
                let (fill, line) = HALOS[mark.halo as usize];
                let strength = if mark.halo == Halo::Picked {
                    1.0
                } else {
                    self.pulse
                };
                let faded = |[r, g, b, a]: [f32; 4]| [r, g, b, a * strength];
                self.rect([l, t, r, b], faded(fill), size);
                for side in [
                    [l, t, r, t + edge],
                    [l, b - edge, r, b],
                    [l, t, l + edge, b],
                    [r - edge, t, r, b],
                ] {
                    self.rect(side, faded(line), size);
                }
            }
            gl.disable(Gl::BLEND);
        }

        // The tool's picks, rubber band and points, over the ink.
        if self.overlays && self.look == Look::Screen && self.tools.running().is_some() {
            self.draw_tool(sheet, &device, ratio, size);
        }

        // The crosshair and the snap's marker, over everything, on screen.
        if self.overlays && self.look == Look::Screen {
            gl.enable(Gl::BLEND);
            gl.blend_func(Gl::SRC_ALPHA, Gl::ONE_MINUS_SRC_ALPHA);
            if let Some(c) = self.caught.get(self.chosen) {
                let (x, y) = device(c.at[0], c.at[1]);
                let shape = marker(c.snap, x, y, ratio);
                self.lines(&shape, (2.0 + 2.5) * ratio, MARKER_EDGE, size);
                self.lines(&shape, 2.0 * ratio, SNAP_YELLOW, size);
            }
            if let Some((at, pick_box)) = self.cursor {
                let (x, y) = device(at[0], at[1]);
                // A tool taking points has no pick box, as AutoCAD's.
                let pick_box = pick_box && !self.tools.takes_points();
                self.lines(
                    &crosshair(x, y, ratio, pick_box),
                    ratio.max(1.0).round(),
                    CROSSHAIR,
                    size,
                );
            }
            gl.disable(Gl::BLEND);
        }
    }

    /// Starts the drawing tool `word` names, an alias such as `L` or the
    /// page's name such as `line`, with the objects at `picked` selected
    /// first, which Move and Erase take. Gives what came of it as JSON:
    /// lines for the command bar's history under `log`, and under `send`
    /// a change set to send, with what it does, the sheets it changes and,
    /// for an edit to a template's object, what to ask first.
    pub fn tool_start(&mut self, word: &str, picked: &[u32]) -> String {
        let picked: Vec<usize> = picked.iter().map(|&p| p as usize).collect();
        let outcome = match tool_for(word) {
            Some(kind) => self.tools.start(kind, &picked),
            None => self.tools.input(word, None),
        };
        to_json(&outcome)
    }

    /// A click or a tap at the drawing point `(x, y)`: a tool picking
    /// objects picks the one drawn nearest, within `reach` drawing units; a
    /// tool taking points takes the snap the cursor holds, found by
    /// [`Viewer::hover`] at this point, or the point itself, held by Ortho.
    /// Gives what came of it, as [`Viewer::tool_start`] does.
    pub fn tool_click(&mut self, x: f64, y: f64, reach: f64) -> String {
        let outcome = if self.tools.selecting() {
            let place = self
                .sheet
                .as_ref()
                .and_then(|sheet| pick(&sheet.list, [x, y], reach, sheet.millimetres_per_unit));
            self.tools.pick(place)
        } else if self.tools.takes_points() {
            let aim = self.aim_at([x, y]);
            self.tools.point(aim)
        } else {
            Default::default()
        };
        to_json(&outcome)
    }

    /// What was typed in the command bar and entered; empty for Enter,
    /// Space or a right-click. Gives what came of it.
    pub fn tool_input(&mut self, text: &str) -> String {
        let aim = self.pointer().map(|p| self.aim_at(p).at);
        to_json(&self.tools.input(text, aim))
    }

    /// An option of the prompt clicked, by its act.
    pub fn tool_option(&mut self, act: &str) -> String {
        to_json(&self.tools.option(act))
    }

    /// Esc: ends the tool running, keeping what it has drawn.
    pub fn tool_escape(&mut self) -> String {
        to_json(&self.tools.escape())
    }

    /// The line of text being typed, as it is typed.
    pub fn tool_typing(&mut self, text: &str) {
        self.tools.typing(text);
    }

    /// Text typed in a phone's box, at `height` (not a number for the one
    /// it has): added at once.
    pub fn tool_text_box(&mut self, text: &str, height: f64) -> String {
        to_json(
            &self
                .tools
                .text_box(text, Some(height).filter(|h| h.is_finite())),
        )
    }

    /// What the command bar shows of the tool running, as JSON, in a
    /// phone's words when `touch`.
    pub fn tool_state(&self, touch: bool) -> String {
        to_json(&self.tools.state(touch))
    }

    /// The tag over each copy Copy has placed, which waits for Enter, as
    /// JSON: `[{"text": "copy 1 · waits for Enter", "at": [x, y]}]`, `at`
    /// the top right of the box round the copy, in drawing units, for the
    /// page to put the tag by; in a phone's words, `copy 1`, when `touch`.
    pub fn tool_copy_tags(&self, touch: bool) -> String {
        let tags = self.tools.placed_tags(touch);
        if tags.is_empty() {
            return "[]".to_owned();
        }
        let picked = self.tools.state(touch).picked;
        let objects: Vec<u32> = picked.iter().map(|&p| p as u32).collect();
        let b = self.bounds_of(&objects);
        let tags: Vec<serde_json::Value> = tags
            .into_iter()
            .filter(|_| b.len() == 4)
            .map(|(text, [dx, dy])| serde_json::json!({"text": text, "at": [b[2] + dx, b[3] + dy]}))
            .collect();
        to_json(&tags)
    }

    /// Whether the tool running takes the drawing's clicks and taps now: it
    /// takes points, or picks objects.
    pub fn tool_wants_clicks(&self) -> bool {
        self.tools.takes_points() || self.tools.selecting()
    }

    /// Whether the tool running is typing a text's words.
    pub fn tool_typing_text(&self) -> bool {
        self.tools.state(false).typing_text
    }

    /// The text height a Text tool draws at.
    pub fn tool_text_height(&self) -> f64 {
        self.tools.text_height()
    }

    /// What shows at the cursor while a tool runs, `[words, more]` as JSON:
    /// the length and angle from the last point, a radius or a size; empty
    /// when there is nothing to measure.
    pub fn tool_cursor_words(&self) -> String {
        let Some(p) = self.pointer() else {
            return String::new();
        };
        let aim = self.aim_at(p);
        self.tools
            .cursor_words(&aim)
            .map(|words| to_json(&words))
            .unwrap_or_default()
    }

    /// The fields quick properties edit of the object at `place`, a place
    /// in the sheet's objects, as JSON: each with its value, whether it can
    /// be edited, and what it says of where its value is written.
    pub fn props_fields(&self, place: u32) -> String {
        to_json(&crate::props::fields(self.tools.context(), place as usize))
    }

    /// What an edit of the field `key` of the object at `place` to `typed`
    /// gives, as JSON: the change set to send, with the template's question
    /// when it edits the template, what went wrong with what was typed, or
    /// neither when the value is as it was.
    pub fn props_edit(&self, place: u32, key: &str, typed: &str) -> String {
        to_json(&crate::props::edit(
            self.tools.context(),
            place as usize,
            key,
            typed,
        ))
    }

    /// Ortho (F8): Line and Move held level or upright.
    pub fn set_ortho(&mut self, on: bool) {
        self.tools.ortho = on;
    }

    /// The current layer, which new objects go on, and the colour index it
    /// shows in, which a rubber band is drawn in.
    pub fn set_layer(&mut self, name: &str, color: u8) {
        self.tools.layer = name.to_owned();
        let [r, g, b, _] = screen_rgba(Color(color));
        self.layer_color = [
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
            1.0,
        ];
    }

    /// Where a held finger is, in drawing units, for a tool to aim at.
    pub fn set_tool_pointer(&mut self, x: f64, y: f64) {
        self.tool_pointer = Some([x, y]);
    }

    /// The finger has lifted.
    pub fn clear_tool_pointer(&mut self) {
        self.tool_pointer = None;
    }

    /// The canvas's pixels as drawn last, RGBA, rows from the top. Read it
    /// in the same task as the draw.
    pub fn pixels(&self) -> Result<Vec<u8>, JsError> {
        let gl = &self.gl;
        let (width, height) = (gl.drawing_buffer_width(), gl.drawing_buffer_height());
        let row = width as usize * 4;
        let mut bottom_up = vec![0u8; row * height as usize];
        gl.read_pixels_with_opt_u8_array(
            0,
            0,
            width,
            height,
            Gl::RGBA,
            Gl::UNSIGNED_BYTE,
            Some(&mut bottom_up),
        )
        .map_err(|_| JsError::new("could not read the canvas's pixels"))?;
        Ok(bottom_up
            .chunks_exact(row)
            .rev()
            .flatten()
            .copied()
            .collect())
    }
}

impl Viewer {
    /// Where the tool aims from: a held finger, or the crosshair.
    fn pointer(&self) -> Option<[f64; 2]> {
        self.tool_pointer.or(self.cursor.map(|(at, _)| at))
    }

    /// The point a click at `at` gives the tool running: the snap the cursor
    /// holds, or `at`, held by Ortho.
    fn aim_at(&self, at: [f64; 2]) -> Taken {
        let snap = self.caught.get(self.chosen).map(|c| {
            (
                SnapRef {
                    snap: c.snap,
                    object: c.object,
                    which: c.which.clone(),
                    with: c.with,
                },
                [c.at[0], c.at[1], c.at[2]],
            )
        });
        self.tools.aim(at, snap)
    }

    /// The tool's picks, each with a halo, its rubber band in the current
    /// layer's colour, its guides dashed, the objects a move would move
    /// where they would go, and the points it has taken.
    fn draw_tool(
        &self,
        sheet: &Loaded,
        device: &dyn Fn(f64, f64) -> (f64, f64),
        ratio: f64,
        size: (f32, f32),
    ) {
        let gl = &self.gl;
        gl.enable(Gl::BLEND);
        gl.blend_func(Gl::SRC_ALPHA, Gl::ONE_MINUS_SRC_ALPHA);
        let state = self.tools.state(false);
        let pad = HALO_PAD * ratio;
        let edge = (HALO_EDGE * ratio).max(1.0);
        let (fill, line) = HALOS[Halo::Picked as usize];
        for &place in &state.picked {
            let Some(b) = object_bounds(&sheet.list, place) else {
                continue;
            };
            let (l, t) = device(b.min[0], b.max[1]);
            let (r, bottom) = device(b.max[0], b.min[1]);
            let [l, t, r, bottom] = [l - pad, t - pad, r + pad, bottom + pad];
            self.rect(
                [l, t, r, bottom],
                [fill[0], fill[1], fill[2], fill[3] * 2.0],
                size,
            );
            for side in [
                [l, t, r, t + edge],
                [l, bottom - edge, r, bottom],
                [l, t, l + edge, bottom],
                [r - edge, t, r, bottom],
            ] {
                self.rect(side, line, size);
            }
        }
        let aim = self.pointer().map(|p| self.aim_at(p));
        let band = self.tools.band(aim.as_ref());
        let seg = |[a, b]: [[f64; 2]; 2]| -> Segment {
            let (x0, y0) = device(a[0], a[1]);
            let (x1, y1) = device(b[0], b[1]);
            [x0, y0, x1, y1]
        };
        // What the objects at `picked` draw, shifted by `by`: each run of
        // each item, device pixels, for a ghost; up to MOST_MOVED pieces
        // in all, so a big group does not stall a frame.
        let shifted = |picked: &[usize], by: &[[f64; 2]]| -> Vec<Vec<Segment>> {
            let mut out: Vec<Vec<Segment>> = Vec::new();
            let mut count = 0;
            for &[dx, dy] in by {
                for item in sheet
                    .list
                    .items
                    .iter()
                    .filter(|i| picked.contains(&i.object))
                {
                    let runs = match &item.shape {
                        Shape::Stroke { runs, .. } => runs,
                        Shape::Fill { contours, .. } => contours,
                    };
                    for run in runs {
                        let pieces: Vec<Segment> = run
                            .windows(2)
                            .map(|pair| {
                                seg([
                                    [pair[0][0] + dx, pair[0][1] + dy],
                                    [pair[1][0] + dx, pair[1][1] + dy],
                                ])
                            })
                            .collect();
                        count += pieces.len();
                        out.push(pieces);
                    }
                    if count > MOST_MOVED {
                        return out;
                    }
                }
            }
            out
        };
        // What a move would move, where it would go.
        if let Some((picked, by)) = &band.moving {
            let moved: Vec<Segment> = shifted(picked, &[*by]).concat();
            self.lines(&moved, ratio.max(1.0), MOVING, size);
        }
        // Copy: each copy placed, which waits for Enter, dashed in the
        // proposal's colour, as a proposal's ghost is; and the copy that
        // follows the cursor dashed pale blue, as Move's ghost, while the
        // objects themselves stay where they are.
        let dashed = |runs: Vec<Vec<Segment>>| -> Vec<Segment> {
            runs.iter()
                .flat_map(|run| ghosts::dash_run(run, 6.0 * ratio, 4.0 * ratio))
                .collect()
        };
        if let Some((picked, offsets)) = &band.placed {
            let copies = dashed(shifted(picked, offsets));
            self.lines(&copies, (1.4 * ratio).max(1.0), ghosts::PROPOSAL, size);
        }
        if let Some((picked, by)) = &band.copying {
            let copy = dashed(shifted(picked, &[*by]));
            self.lines(&copy, (1.3 * ratio).max(1.0), MOVING, size);
        }
        let lines: Vec<Segment> = band.lines.iter().copied().map(seg).collect();
        self.lines(&lines, (1.5 * ratio).max(1.0), self.layer_color, size);
        let dash = 5.0 * ratio;
        let mut dashes: Vec<Segment> = Vec::new();
        for guide in &band.guides {
            let [x0, y0, x1, y1] = seg(*guide);
            let length = (x1 - x0).hypot(y1 - y0);
            let at = |d: f64| (x0 + (x1 - x0) * d / length, y0 + (y1 - y0) * d / length);
            let mut t = 0.0;
            while t < length {
                let (ax, ay) = at(t);
                let (bx, by) = at((t + dash).min(length));
                dashes.push([ax, ay, bx, by]);
                t += dash * 1.8;
            }
        }
        self.lines(&dashes, ratio.max(1.0), GUIDE, size);
        let half = 2.0 * ratio;
        for p in &band.points {
            let (x, y) = device(p[0], p[1]);
            self.rect([x - half, y - half, x + half, y + half], TAKEN, size);
        }
        gl.disable(Gl::BLEND);
    }

    /// A change set's preview over the sheet as it would leave it: the rest
    /// faded, the old places faint, what it deletes struck, what it adds
    /// and changes in their colours in dashed boxes, a guide where an
    /// object only moves, and a cloud round each check it would fail.
    /// `pixels` is device pixels to a drawing unit.
    fn draw_preview(
        &self,
        sheet: &Loaded,
        p: &Preview,
        device: &dyn Fn(f64, f64) -> (f64, f64),
        (pixels, ratio): (f64, f64),
        size: (f32, f32),
    ) {
        let gl = &self.gl;
        gl.enable(Gl::BLEND);
        gl.blend_func(Gl::SRC_ALPHA, Gl::ONE_MINUS_SRC_ALPHA);
        let [r, g, b] = MODEL_SPACE;
        self.rect(
            [0.0, 0.0, f64::from(size.0), f64::from(size.1)],
            [r, g, b, preview::FADE],
            size,
        );
        let seg = |[a, b]: preview::Line| -> Segment {
            let (x0, y0) = device(a[0], a[1]);
            let (x1, y1) = device(b[0], b[1]);
            [x0, y0, x1, y1]
        };
        let corners = |bounds: Bounds, pad: f64| -> [f64; 4] {
            let (l, t) = device(bounds.min[0], bounds.max[1]);
            let (r, b) = device(bounds.max[0], bounds.min[1]);
            [l - pad, t - pad, r + pad, b + pad]
        };
        // A pen's width on the screen, never finer than one and a half CSS
        // pixels, so a mark reads over the faded ink.
        let per_mm = pixels / sheet.millimetres_per_unit.max(f64::MIN_POSITIVE);
        let width = |item: &agentdrafter_core::Item| {
            preview::pen(item)
                .map_or(0.0, |mm| mm * per_mm)
                .max(1.5 * ratio)
        };
        // The old places, and what it deletes, struck through.
        for was in &p.was {
            let lines: Vec<Segment> = was
                .items
                .iter()
                .flat_map(preview::outline)
                .map(seg)
                .collect();
            let color = if was.deleted {
                preview::DELETED
            } else {
                preview::OLD_PLACE
            };
            self.lines(&lines, ratio.max(1.0), color, size);
            if was.deleted {
                if let Some(bounds) = preview::bounds_of(was.items.iter()) {
                    let strike = preview::strike(corners(bounds, 5.0 * ratio));
                    self.lines(&strike, 1.6 * ratio, preview::DELETED, size);
                }
            }
        }
        // What it adds and changes, item by item, at its pen's width.
        for (places, color) in [(&p.added, preview::ADDED), (&p.changed, preview::CHANGED)] {
            for &i in places {
                let item = &sheet.list.items[i];
                let lines: Vec<Segment> = preview::outline(item).into_iter().map(seg).collect();
                self.lines(&lines, width(item), color, size);
            }
        }
        // A dashed box round each object it adds or changes.
        let dash = 4.0 * ratio;
        for (added, bounds) in p.marked_objects(&sheet.list) {
            let [r, g, b, _] = if added {
                preview::ADDED
            } else {
                preview::CHANGED
            };
            let lines: Vec<Segment> = preview::sides(corners(bounds, 4.0 * ratio))
                .into_iter()
                .flat_map(|side| preview::dashed(side, dash, 0.75 * dash))
                .collect();
            self.lines(&lines, ratio.max(1.0), [r, g, b, 0.75], size);
        }
        let guides: Vec<Segment> = p
            .moves
            .iter()
            .flat_map(|m| preview::dashed(seg(*m), 3.0 * ratio, 4.0 * ratio))
            .collect();
        self.lines(&guides, ratio.max(1.0), preview::OLD_PLACE, size);
        // A cloud round each check it would fail.
        let clouds: Vec<Segment> = p
            .clouds
            .iter()
            .flat_map(|b| preview::cloud(corners(*b, 6.0 * ratio), 12.0 * ratio))
            .collect();
        self.lines(&clouds, 1.8 * ratio, preview::FAILS, size);
        gl.disable(Gl::BLEND);
    }

    /// The proposals waiting on the sheet shown, over its ink: what each
    /// would add and change in the proposal's colour at its pen's width,
    /// strokes dashed and fills solid; a dashed box round each object; what
    /// it would delete boxed and struck; and a guide where an object only
    /// moves. One the set would refuse now is fainter.
    fn draw_ghosts(
        &self,
        sheet: &Loaded,
        device: &dyn Fn(f64, f64) -> (f64, f64),
        (pixels, ratio): (f64, f64),
        size: (f32, f32),
    ) {
        let gl = &self.gl;
        gl.enable(Gl::BLEND);
        gl.blend_func(Gl::SRC_ALPHA, Gl::ONE_MINUS_SRC_ALPHA);
        let seg = |[a, b]: preview::Line| -> Segment {
            let (x0, y0) = device(a[0], a[1]);
            let (x1, y1) = device(b[0], b[1]);
            [x0, y0, x1, y1]
        };
        let corners = |bounds: Bounds, pad: f64| -> [f64; 4] {
            let (l, t) = device(bounds.min[0], bounds.max[1]);
            let (r, b) = device(bounds.max[0], bounds.min[1]);
            [l - pad, t - pad, r + pad, b + pad]
        };
        let per_mm = pixels / sheet.millimetres_per_unit.max(f64::MIN_POSITIVE);
        let (name, layout) = &self.shown;
        for ghost in self.ghosts.iter().filter(|g| g.on(name, layout.as_deref())) {
            let fade = if ghost.refused {
                ghosts::REFUSED_ALPHA
            } else {
                1.0
            };
            let [r, g, b, a] = ghosts::PROPOSAL;
            let color = [r, g, b, a * fade];
            let [r, g, b, a] = ghosts::PROPOSAL_DIM;
            let dim = [r, g, b, a * fade];
            for run in &ghost.runs {
                let lines: Vec<Segment> = run.lines.iter().copied().map(seg).collect();
                let width = run.pen.map_or(0.0, |mm| mm * per_mm).max(1.4 * ratio);
                if run.fill {
                    self.lines(&lines, ratio.max(1.0), color, size);
                } else {
                    let dashed = ghosts::dash_run(&lines, 7.0 * ratio, 4.0 * ratio);
                    self.lines(&dashed, width, color, size);
                }
            }
            let dash = 4.0 * ratio;
            let boxes: Vec<Segment> = ghost
                .boxes
                .iter()
                .chain(&ghost.struck)
                .flat_map(|b| preview::sides(corners(*b, 4.0 * ratio)))
                .flat_map(|side| preview::dashed(side, dash, 0.75 * dash))
                .collect();
            self.lines(&boxes, ratio.max(1.0), dim, size);
            let strikes: Vec<Segment> = ghost
                .struck
                .iter()
                .flat_map(|b| preview::strike(corners(*b, 4.0 * ratio)))
                .collect();
            self.lines(&strikes, 1.4 * ratio, color, size);
            let guides: Vec<Segment> = ghost
                .moves
                .iter()
                .flat_map(|m| preview::dashed(seg(*m), 3.0 * ratio, 4.0 * ratio))
                .collect();
            self.lines(&guides, ratio.max(1.0), dim, size);
        }
        gl.disable(Gl::BLEND);
    }

    /// Draws `segments`, device pixels, `width` device pixels wide in `color`.
    fn lines(&self, segments: &[Segment], width: f64, color: [f32; 4], size: (f32, f32)) {
        if segments.is_empty() {
            return;
        }
        let gl = &self.gl;
        let points = triangles(segments, width);
        gl.use_program(Some(&self.overlay.program));
        gl.uniform2f(self.overlay.at("u_size"), size.0, size.1);
        gl.uniform4f(
            self.overlay.at("u_color"),
            color[0],
            color[1],
            color[2],
            color[3],
        );
        gl.bind_vertex_array(Some(&self.overlay_array));
        gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&self.overlay_points));
        gl.buffer_data_with_u8_array(Gl::ARRAY_BUFFER, &floats(&points), Gl::DYNAMIC_DRAW);
        gl.draw_arrays(Gl::TRIANGLES, 0, (points.len() / 2) as i32);
        gl.bind_vertex_array(None);
    }

    /// Model space under the drawing: its colour, the grid when it is on,
    /// and the paper's dashed edge at `paper`, `[left, top, right, bottom]`
    /// in device pixels; one quad over the canvas.
    fn backdrop(
        &self,
        sheet: &Loaded,
        (left, top, scale, ratio): (f64, f64, f64, f64),
        paper: [f64; 4],
        size: (f32, f32),
    ) {
        let gl = &self.gl;
        let p = &self.backdrop;
        gl.use_program(Some(&p.program));
        gl.uniform4f(p.at("u_rect"), 0.0, 0.0, size.0, size.1);
        gl.uniform2f(p.at("u_size"), size.0, size.1);
        let [r, g, b] = MODEL_SPACE;
        gl.uniform3f(p.at("u_background"), r, g, b);
        // A grid line, and the paper's edge, one CSS pixel wide, in whole
        // device pixels.
        gl.uniform1f(p.at("u_line"), ratio.max(1.0).round() as f32);
        let pixels = scale * ratio;
        let lines = |step: f64| -> [f32; 4] {
            [
                first_line(left, step, pixels, false) as f32,
                first_line(top, step, pixels, true) as f32,
                (step * pixels) as f32,
                (step * pixels) as f32,
            ]
        };
        let shown = if self.grid {
            grid(sheet.millimetres_per_unit, scale)
        } else {
            None
        };
        let none = [0.0f32; 4];
        let (minor, major) = shown.map_or((none, none), |g| {
            (
                if g.minor_shown { lines(g.minor) } else { none },
                lines(g.major),
            )
        });
        gl.uniform4f(p.at("u_minor"), minor[0], minor[1], minor[2], minor[3]);
        gl.uniform4f(p.at("u_major"), major[0], major[1], major[2], major[3]);
        let [r, g, b] = GRID_MINOR;
        gl.uniform3f(p.at("u_minor_color"), r, g, b);
        let [r, g, b] = GRID_MAJOR;
        gl.uniform3f(p.at("u_major_color"), r, g, b);
        // Rounded to the pixel, so the edge is crisp.
        let at = |v: f64| v.round() as f32;
        gl.uniform4f(
            p.at("u_paper"),
            at(paper[0]),
            at(paper[1]),
            at(paper[2]),
            at(paper[3]),
        );
        let [r, g, b] = PAPER_EDGE;
        gl.uniform3f(p.at("u_edge_color"), r, g, b);
        gl.draw_arrays(Gl::TRIANGLE_STRIP, 0, 4);
    }

    /// Points the vertex arrays at the colours of the look: on paper or on
    /// screen.
    fn bind_colors(&self, sheet: &Loaded) {
        let gl = &self.gl;
        let which = usize::from(self.look == Look::Screen);
        gl.bind_vertex_array(Some(&sheet.strokes));
        gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&sheet.stroke_colors[which]));
        gl.vertex_attrib_pointer_with_i32(2, 4, Gl::UNSIGNED_BYTE, true, 0, 0);
        gl.bind_vertex_array(Some(&sheet.fills));
        gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&sheet.fill_colors[which]));
        gl.vertex_attrib_pointer_with_i32(1, 4, Gl::UNSIGNED_BYTE, true, 0, 0);
        gl.bind_vertex_array(None);
    }

    fn rect(&self, rect: [f64; 4], color: [f32; 4], size: (f32, f32)) {
        let gl = &self.gl;
        gl.use_program(Some(&self.rect.program));
        gl.uniform4f(
            self.rect.at("u_rect"),
            rect[0] as f32,
            rect[1] as f32,
            rect[2] as f32,
            rect[3] as f32,
        );
        gl.uniform2f(self.rect.at("u_size"), size.0, size.1);
        gl.uniform4f(
            self.rect.at("u_color"),
            color[0],
            color[1],
            color[2],
            color[3],
        );
        gl.draw_arrays(Gl::TRIANGLE_STRIP, 0, 4);
    }

    /// Gives the GPU the mesh's buffers.
    fn upload(
        &self,
        mesh: &Mesh,
        millimetres_per_unit: f64,
        list: DisplayList,
    ) -> Result<Loaded, JsError> {
        let gl = &self.gl;
        let mut buffers = Vec::new();
        let mut buffer = |target: u32, bytes: &[u8]| -> Result<WebGlBuffer, JsError> {
            let buffer = gl
                .create_buffer()
                .ok_or_else(|| JsError::new("the GPU gave no buffer"))?;
            gl.bind_buffer(target, Some(&buffer));
            gl.buffer_data_with_u8_array(target, bytes, Gl::STATIC_DRAW);
            buffers.push(buffer.clone());
            Ok(buffer)
        };
        let vertex_array = || {
            gl.create_vertex_array()
                .ok_or_else(|| JsError::new("the GPU gave no vertex array"))
        };

        // Each segment four times, once for each corner of its quad, and
        // two triangles to each quad: one plain draw, which every GPU, and
        // a software one, draws fast.
        let four = |values: &[u8], size: usize| -> Vec<u8> {
            values
                .chunks_exact(size)
                .flat_map(|value| std::iter::repeat_n(value, 4).flatten().copied())
                .collect()
        };
        let strokes = vertex_array()?;
        gl.bind_vertex_array(Some(&strokes));
        buffer(
            Gl::ARRAY_BUFFER,
            &four(&floats(mesh.segments.as_flattened()), 16),
        )?;
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_with_i32(0, 4, Gl::FLOAT, false, 0, 0);
        buffer(Gl::ARRAY_BUFFER, &four(&floats(&mesh.widths), 4))?;
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_with_i32(1, 1, Gl::FLOAT, false, 0, 0);
        // Both looks' colours; `bind_colors` points the array at one.
        let stroke_screen = buffer(
            Gl::ARRAY_BUFFER,
            &four(mesh.segment_screen_colors.as_flattened(), 4),
        )?;
        let stroke_paper = buffer(
            Gl::ARRAY_BUFFER,
            &four(mesh.segment_colors.as_flattened(), 4),
        )?;
        gl.enable_vertex_attrib_array(2);
        gl.vertex_attrib_pointer_with_i32(2, 4, Gl::UNSIGNED_BYTE, true, 0, 0);
        buffer(Gl::ARRAY_BUFFER, &four(&places(&mesh.segment_items), 4))?;
        gl.enable_vertex_attrib_array(3);
        gl.vertex_attrib_pointer_with_i32(3, 1, Gl::FLOAT, false, 0, 0);
        buffer(
            Gl::ARRAY_BUFFER,
            &four(&floats(mesh.segment_clips.as_flattened()), 16),
        )?;
        gl.enable_vertex_attrib_array(4);
        gl.vertex_attrib_pointer_with_i32(4, 4, Gl::FLOAT, false, 0, 0);
        let quads: Vec<u8> = (0..mesh.segments.len() as u32)
            .flat_map(|s| [0, 1, 2, 2, 1, 3].map(|corner| 4 * s + corner))
            .flat_map(u32::to_le_bytes)
            .collect();
        buffer(Gl::ELEMENT_ARRAY_BUFFER, &quads)?;

        let fills = vertex_array()?;
        gl.bind_vertex_array(Some(&fills));
        buffer(Gl::ARRAY_BUFFER, &floats(mesh.fill_points.as_flattened()))?;
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_with_i32(0, 2, Gl::FLOAT, false, 0, 0);
        let fill_screen = buffer(Gl::ARRAY_BUFFER, mesh.fill_screen_colors.as_flattened())?;
        let fill_paper = buffer(Gl::ARRAY_BUFFER, mesh.fill_colors.as_flattened())?;
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_with_i32(1, 4, Gl::UNSIGNED_BYTE, true, 0, 0);
        buffer(Gl::ARRAY_BUFFER, &places(&mesh.fill_items))?;
        gl.enable_vertex_attrib_array(2);
        gl.vertex_attrib_pointer_with_i32(2, 1, Gl::FLOAT, false, 0, 0);
        let corners: Vec<u8> = mesh
            .triangles
            .as_flattened()
            .iter()
            .flat_map(|i| i.to_le_bytes())
            .collect();
        buffer(Gl::ELEMENT_ARRAY_BUFFER, &corners)?;
        gl.bind_vertex_array(None);

        Ok(Loaded {
            list,
            snaps: Vec::new(),
            outlines: Vec::new(),
            preview: None,
            origin: mesh.origin,
            paper: mesh.paper,
            millimetres_per_unit,
            items: mesh.items as f32,
            runs: mesh.runs.clone(),
            spans: mesh.spans.clone(),
            stencils: mesh.stencils.clone(),
            strokes,
            fills,
            stroke_colors: [stroke_paper, stroke_screen],
            fill_colors: [fill_paper, fill_screen],
            buffers,
        })
    }

    /// Lets the GPU free the sheet it holds.
    fn unload(&mut self) {
        if let Some(sheet) = self.sheet.take() {
            let gl = &self.gl;
            gl.delete_vertex_array(Some(&sheet.strokes));
            gl.delete_vertex_array(Some(&sheet.fills));
            for buffer in &sheet.buffers {
                gl.delete_buffer(Some(buffer));
            }
        }
    }
}

/// A reply to the page, as JSON.
fn to_json(value: &impl serde::Serialize) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

/// Little-endian bytes of `values`, as WebAssembly and the GPU read them.
fn floats(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// Items' places in the display list as the GPU reads them, 32-bit floats,
/// which hold every whole number up to 16,777,216 exactly.
fn places(items: &[u32]) -> Vec<u8> {
    items
        .iter()
        .flat_map(|&place| (place as f32).to_le_bytes())
        .collect()
}

/// A shader of `kind` compiled from `source`.
fn compile(gl: &Gl, kind: u32, source: &str) -> Result<WebGlShader, JsError> {
    let shader = gl
        .create_shader(kind)
        .ok_or_else(|| JsError::new("the GPU gave no shader"))?;
    gl.shader_source(&shader, source);
    gl.compile_shader(&shader);
    if gl
        .get_shader_parameter(&shader, Gl::COMPILE_STATUS)
        .as_bool()
        .unwrap_or(false)
    {
        return Ok(shader);
    }
    let log = gl.get_shader_info_log(&shader).unwrap_or_default();
    Err(JsError::new(&format!("a shader did not compile: {log}")))
}

/// A program linked from a vertex and a fragment shader.
fn link(gl: &Gl, vertex: &str, fragment: &str) -> Result<WebGlProgram, JsError> {
    let program = gl
        .create_program()
        .ok_or_else(|| JsError::new("the GPU gave no program"))?;
    gl.attach_shader(&program, &compile(gl, Gl::VERTEX_SHADER, vertex)?);
    gl.attach_shader(&program, &compile(gl, Gl::FRAGMENT_SHADER, fragment)?);
    gl.link_program(&program);
    if gl
        .get_program_parameter(&program, Gl::LINK_STATUS)
        .as_bool()
        .unwrap_or(false)
    {
        return Ok(program);
    }
    let log = gl.get_program_info_log(&program).unwrap_or_default();
    Err(JsError::new(&format!("a program did not link: {log}")))
}

/// Whether `word`, typed in the command bar, starts a drawing tool: one of
/// its aliases, such as `L` or `RECTANG`, or the page's name for it, in any
/// case. Words that do not go to the drafter's assistant (#156).
#[wasm_bindgen]
pub fn is_tool_word(word: &str) -> bool {
    tool_for(word).is_some()
}

/// The colour the colour index `index` shows in on the dark model space,
/// as `#rrggbb`, for the Layer box's swatches.
#[wasm_bindgen]
pub fn screen_color(index: u8) -> String {
    let [r, g, b, _] = screen_rgba(Color(index));
    format!("#{r:02x}{g:02x}{b:02x}")
}
