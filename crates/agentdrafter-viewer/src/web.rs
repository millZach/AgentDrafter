//! The module the page loads: the mesh drawn with WebGL2.
//!
//! The page owns the view: which drawing point sits at the canvas's top
//! left corner, and how many CSS pixels a drawing unit spans. Each frame it
//! calls [`Viewer::draw`] with those; the GPU is given two numbers and the
//! mesh it already holds, so a pan or a zoom costs nothing to lay out.
//!
//! - Strokes are drawn as one draw: each segment a quad round the pen's
//!   capsule, its round ends and its coverage worked out per pixel, so a
//!   pen is exactly as wide as the PNG draws it, anti-aliased. The quads
//!   are plain triangles, not instances, which software WebGL draws several
//!   times faster.
//! - Fills are drawn as one draw of their triangles, their edges smoothed
//!   by the canvas's multisampling.
//! - Ink only darkens: blending takes each pixel's darkest ink (`MIN`), so
//!   strokes that meet at a corner do not darken where they overlap, and
//!   the order of the shapes does not matter. Where two colours cross, the
//!   darker shows.

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{
    HtmlCanvasElement, WebGl2RenderingContext as Gl, WebGlBuffer, WebGlProgram, WebGlShader,
    WebGlUniformLocation, WebGlVertexArrayObject,
};

use crate::mesh::{read_reply, Mesh};

const STROKE_VERTEX: &str = r#"#version 300 es
precision highp float;
layout(location = 0) in vec4 a_segment;
layout(location = 1) in float a_lineweight;
layout(location = 2) in vec4 a_color;
uniform vec2 u_scale;
uniform vec2 u_offset;
uniform vec2 u_size;
uniform float u_pixels_per_mm;
uniform float u_finest;
out vec2 v_at;
flat out vec2 v_a;
flat out vec2 v_b;
flat out float v_half;
flat out vec3 v_color;
void main() {
  vec2 a = a_segment.xy * u_scale + u_offset;
  vec2 b = a_segment.zw * u_scale + u_offset;
  float half_width = max(a_lineweight * u_pixels_per_mm, u_finest) * 0.5;
  float reach = half_width + 1.0;
  vec2 along = b - a;
  float length_ = length(along);
  vec2 dir = length_ > 1e-6 ? along / length_ : vec2(1.0, 0.0);
  vec2 across = vec2(-dir.y, dir.x);
  // The quad's corner: the index is four to a segment.
  int corner = gl_VertexID & 3;
  vec2 p = ((corner & 1) == 0 ? a - dir * reach : b + dir * reach)
         + across * ((corner & 2) == 0 ? -reach : reach);
  v_at = p;
  v_a = a;
  v_b = b;
  v_half = half_width;
  v_color = a_color.rgb;
  gl_Position = vec4(p.x / u_size.x * 2.0 - 1.0, 1.0 - p.y / u_size.y * 2.0, 0.0, 1.0);
}
"#;

const STROKE_FRAGMENT: &str = r#"#version 300 es
precision highp float;
in vec2 v_at;
flat in vec2 v_a;
flat in vec2 v_b;
flat in float v_half;
flat in vec3 v_color;
out vec4 o_color;
void main() {
  vec2 pa = v_at - v_a;
  vec2 ba = v_b - v_a;
  float h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-12), 0.0, 1.0);
  float d = length(pa - ba * h);
  float coverage = clamp(v_half + 0.5 - d, 0.0, 1.0);
  if (coverage <= 0.0) discard;
  o_color = vec4(mix(vec3(1.0), v_color, coverage), 1.0);
}
"#;

const FILL_VERTEX: &str = r#"#version 300 es
precision highp float;
layout(location = 0) in vec2 a_point;
layout(location = 1) in vec4 a_color;
uniform vec2 u_scale;
uniform vec2 u_offset;
uniform vec2 u_size;
flat out vec3 v_color;
void main() {
  vec2 p = a_point * u_scale + u_offset;
  v_color = a_color.rgb;
  gl_Position = vec4(p.x / u_size.x * 2.0 - 1.0, 1.0 - p.y / u_size.y * 2.0, 0.0, 1.0);
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

const RECT_FRAGMENT: &str = r#"#version 300 es
precision highp float;
uniform vec3 u_color;
out vec4 o_color;
void main() {
  o_color = vec4(u_color, 1.0);
}
"#;

/// The viewer's background round the paper, as the page's: `#15171a`.
const SURROUND: [f32; 3] = [
    0x15 as f32 / 255.0,
    0x17 as f32 / 255.0,
    0x1a as f32 / 255.0,
];

/// The paper's shadow on the background.
const SHADOW: [f32; 3] = [0.04, 0.045, 0.05];

/// How far the paper's shadow falls, in CSS pixels.
const SHADOW_OFFSET: f64 = 6.0;

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
    origin: [f64; 2],
    paper: [f64; 4],
    millimetres_per_unit: f64,
    segments: i32,
    triangle_corners: i32,
    strokes: WebGlVertexArrayObject,
    fills: WebGlVertexArrayObject,
    buffers: Vec<WebGlBuffer>,
}

/// The renderer the page draws its canvas with.
#[wasm_bindgen]
pub struct Viewer {
    gl: Gl,
    stroke: Program,
    fill: Program,
    rect: Program,
    surround: [f32; 3],
    sheet: Option<Loaded>,
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
            ("depth", false),
            ("stencil", false),
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
                "u_pixels_per_mm",
                "u_finest",
            ],
        )?;
        let fill = Program::new(
            &gl,
            FILL_VERTEX,
            FILL_FRAGMENT,
            &["u_scale", "u_offset", "u_size"],
        )?;
        let rect = Program::new(
            &gl,
            RECT_VERTEX,
            RECT_FRAGMENT,
            &["u_rect", "u_size", "u_color"],
        )?;
        Ok(Viewer {
            gl,
            stroke,
            fill,
            rect,
            surround: SURROUND,
            sheet: None,
        })
    }

    /// Loads a sheet from the text of a `display-list` reply, replacing the
    /// one held, and gives what it is as JSON: its sheet, the set's sheets,
    /// its paper and how many segments and triangles it draws.
    pub fn load(&mut self, reply: &str) -> Result<String, JsError> {
        let sheet = read_reply(reply).map_err(|message| JsError::new(&message))?;
        let mesh = Mesh::of(&sheet);
        let summary = serde_json::to_string(&mesh.summary(&sheet))
            .map_err(|e| JsError::new(&e.to_string()))?;
        self.unload();
        self.sheet = Some(self.upload(&mesh, sheet.millimetres_per_unit)?);
        Ok(summary)
    }

    /// Draws white round the paper, as a PNG's paper is everywhere, or,
    /// as the page does, its background. A test draws white, to compare the
    /// canvas with a PNG.
    pub fn set_white_surround(&mut self, white: bool) {
        self.surround = if white { [1.0; 3] } else { SURROUND };
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
        let [r, g, b] = self.surround;
        gl.clear_color(r, g, b, 1.0);
        gl.clear(Gl::COLOR_BUFFER_BIT);
        let Some(sheet) = &self.sheet else {
            return;
        };
        let size = (width as f32, height as f32);
        let pixels = scale * ratio;
        let device = |x: f64, y: f64| ((x - left) * pixels, (top - y) * pixels);

        // The paper and its shadow, opaque.
        gl.disable(Gl::BLEND);
        let [x0, y0, x1, y1] = sheet.paper;
        let (left_px, top_px) = device(x0, y1);
        let (right_px, bottom_px) = device(x1, y0);
        let shadow = SHADOW_OFFSET * ratio;
        if self.surround == SURROUND {
            self.rect(
                [
                    left_px + shadow,
                    top_px + shadow,
                    right_px + shadow,
                    bottom_px + shadow,
                ],
                SHADOW,
                size,
            );
        }
        self.rect(
            [left_px, top_px, right_px, bottom_px],
            [1.0, 1.0, 1.0],
            size,
        );

        // The ink: each pixel takes the darkest drawn on it.
        gl.enable(Gl::BLEND);
        gl.blend_equation(Gl::MIN);
        let scale_xy = (pixels as f32, -pixels as f32);
        let offset = device(sheet.origin[0], sheet.origin[1]);
        let offset = (offset.0 as f32, offset.1 as f32);
        if sheet.triangle_corners > 0 {
            gl.use_program(Some(&self.fill.program));
            gl.uniform2f(self.fill.at("u_scale"), scale_xy.0, scale_xy.1);
            gl.uniform2f(self.fill.at("u_offset"), offset.0, offset.1);
            gl.uniform2f(self.fill.at("u_size"), size.0, size.1);
            gl.bind_vertex_array(Some(&sheet.fills));
            gl.draw_elements_with_i32(Gl::TRIANGLES, sheet.triangle_corners, Gl::UNSIGNED_INT, 0);
        }
        if sheet.segments > 0 {
            gl.use_program(Some(&self.stroke.program));
            gl.uniform2f(self.stroke.at("u_scale"), scale_xy.0, scale_xy.1);
            gl.uniform2f(self.stroke.at("u_offset"), offset.0, offset.1);
            gl.uniform2f(self.stroke.at("u_size"), size.0, size.1);
            let per_mm = pixels / sheet.millimetres_per_unit.max(f64::MIN_POSITIVE);
            gl.uniform1f(self.stroke.at("u_pixels_per_mm"), per_mm as f32);
            // The finest pen is one CSS pixel wide, as a PNG's is one pixel.
            gl.uniform1f(self.stroke.at("u_finest"), ratio as f32);
            gl.bind_vertex_array(Some(&sheet.strokes));
            gl.draw_elements_with_i32(Gl::TRIANGLES, sheet.segments * 6, Gl::UNSIGNED_INT, 0);
        }
        gl.bind_vertex_array(None);
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
    fn rect(&self, rect: [f64; 4], color: [f32; 3], size: (f32, f32)) {
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
        gl.uniform3f(self.rect.at("u_color"), color[0], color[1], color[2]);
        gl.draw_arrays(Gl::TRIANGLE_STRIP, 0, 4);
    }

    /// Gives the GPU the mesh's buffers.
    fn upload(&self, mesh: &Mesh, millimetres_per_unit: f64) -> Result<Loaded, JsError> {
        let gl = &self.gl;
        let mut buffers = Vec::new();
        let mut buffer = |target: u32, bytes: &[u8]| -> Result<(), JsError> {
            let buffer = gl
                .create_buffer()
                .ok_or_else(|| JsError::new("the GPU gave no buffer"))?;
            gl.bind_buffer(target, Some(&buffer));
            gl.buffer_data_with_u8_array(target, bytes, Gl::STATIC_DRAW);
            buffers.push(buffer);
            Ok(())
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
        buffer(
            Gl::ARRAY_BUFFER,
            &four(mesh.segment_colors.as_flattened(), 4),
        )?;
        gl.enable_vertex_attrib_array(2);
        gl.vertex_attrib_pointer_with_i32(2, 4, Gl::UNSIGNED_BYTE, true, 0, 0);
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
        buffer(Gl::ARRAY_BUFFER, mesh.fill_colors.as_flattened())?;
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_with_i32(1, 4, Gl::UNSIGNED_BYTE, true, 0, 0);
        let corners: Vec<u8> = mesh
            .triangles
            .as_flattened()
            .iter()
            .flat_map(|i| i.to_le_bytes())
            .collect();
        buffer(Gl::ELEMENT_ARRAY_BUFFER, &corners)?;
        gl.bind_vertex_array(None);

        Ok(Loaded {
            origin: mesh.origin,
            paper: mesh.paper,
            millimetres_per_unit,
            segments: mesh.segments.len() as i32,
            triangle_corners: (mesh.triangles.len() * 3) as i32,
            strokes,
            fills,
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

/// Little-endian bytes of `values`, as WebAssembly and the GPU read them.
fn floats(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
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
