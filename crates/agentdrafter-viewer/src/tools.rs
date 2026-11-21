//! The drafter's drawing tools: Line, Circle, Rectangle, Text, Move, Copy
//! and Erase, as AutoCAD's LINE, CIRCLE, RECTANG, TEXT, MOVE, COPY and ERASE
//! run them, cut down for beginners (#152, Copy #66). Plain computation, tested natively; `web`
//! feeds it the drafter's clicks, taps and typing and draws what it says.
//!
//! A tool takes points, a snapped one where the cursor caught a snap, picks
//! objects and reads what is typed in the command bar: an alias (`L`, `C`,
//! `REC`, `T`, `M`, `CO`, `E`), a point (`120,80`, `@40,0`, `@5<30`), a length
//! alone towards the cursor, or an option's capital letter. While it runs it
//! says what it wants, in plain words with options to click, and shows its
//! rubber band. When it has what it needs it gives one change set, with the
//! operation the core has for what it does (`add_line`, `add_circle`, a
//! closed `add_polyline`, `add_text`, `move`, `copy`, `delete`), which the page
//! sends through `change`: what the edit means is the core's, not this
//! module's (ADR-0007). Each point taken on a snap goes as a snapped point,
//! so the core works out the exact place; a point on a row's own object, in
//! an edit to a template's object, goes as its coordinates instead, since a
//! template's object is drawn through the same point on every sheet.
//!
//! New objects go on the current layer. An edit to an object that comes
//! from a template says so while the tool runs, and the change set it gives
//! names the sheets the template reaches, for the page to ask first.

use agentdrafter_core::SnapKind;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// One of the drawing tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Line,
    Circle,
    Rectangle,
    Text,
    Move,
    Copy,
    Erase,
}

/// A tool as the command bar knows it: AutoCAD's command and the words
/// that start it.
pub struct ToolSpec {
    pub kind: ToolKind,
    /// AutoCAD's command, as the command bar names the tool running.
    pub command: &'static str,
    /// What typed in the command bar starts it, in any case: AutoCAD's
    /// aliases and its command's own names.
    pub aliases: &'static [&'static str],
}

/// The tools and their aliases, as AutoCAD's are, but `T`, which starts
/// AutoCAD's MTEXT, starts TEXT here: a beginner's text is one line.
pub const TOOLS: [ToolSpec; 7] = [
    ToolSpec {
        kind: ToolKind::Line,
        command: "LINE",
        aliases: &["L", "LINE"],
    },
    ToolSpec {
        kind: ToolKind::Circle,
        command: "CIRCLE",
        aliases: &["C", "CIRCLE"],
    },
    ToolSpec {
        kind: ToolKind::Rectangle,
        command: "RECTANG",
        aliases: &["REC", "RECTANG", "RECTANGLE"],
    },
    ToolSpec {
        kind: ToolKind::Text,
        command: "TEXT",
        aliases: &["T", "DT", "TEXT", "DTEXT"],
    },
    ToolSpec {
        kind: ToolKind::Move,
        command: "MOVE",
        aliases: &["M", "MOVE"],
    },
    ToolSpec {
        kind: ToolKind::Copy,
        command: "COPY",
        aliases: &["CO", "CP", "COPY"],
    },
    ToolSpec {
        kind: ToolKind::Erase,
        command: "ERASE",
        aliases: &["E", "ERASE"],
    },
];

impl ToolKind {
    fn spec(self) -> &'static ToolSpec {
        TOOLS
            .iter()
            .find(|t| t.kind == self)
            .expect("every tool is listed")
    }

    /// AutoCAD's command for it.
    pub fn command(self) -> &'static str {
        self.spec().command
    }

    /// Whether it draws a new object, on the current layer.
    pub fn draws(self) -> bool {
        !matches!(self, ToolKind::Move | ToolKind::Copy | ToolKind::Erase)
    }

    /// Its name, as the page names it.
    pub fn key(self) -> &'static str {
        match self {
            ToolKind::Line => "line",
            ToolKind::Circle => "circle",
            ToolKind::Rectangle => "rectangle",
            ToolKind::Text => "text",
            ToolKind::Move => "move",
            ToolKind::Copy => "copy",
            ToolKind::Erase => "erase",
        }
    }
}

/// The tool a word typed in the command bar starts, or its name as the
/// page names it, in any case.
pub fn tool_for(word: &str) -> Option<ToolKind> {
    let word = word.trim();
    TOOLS
        .iter()
        .find(|t| {
            t.kind.key().eq_ignore_ascii_case(word)
                || t.aliases.iter().any(|a| a.eq_ignore_ascii_case(word))
        })
        .map(|t| t.kind)
}

/// A snap a point was taken on, as the renderer caught it.
#[derive(Clone, Debug, PartialEq)]
pub struct SnapRef {
    pub snap: SnapKind,
    /// The object's place in the sheet's objects.
    pub object: usize,
    pub which: Option<String>,
    /// For an intersection, the other object's place.
    pub with: Option<usize>,
}

/// A point a tool took: where, its height, the snap it was taken on, if
/// any, and whether it was typed.
#[derive(Clone, Debug, PartialEq)]
pub struct Taken {
    pub at: [f64; 2],
    /// Its Z: a snap's own, as its snapped point resolves; for a point typed
    /// relative to another, or a length towards the cursor, that point's;
    /// 0 for a point clicked on no snap, or typed as `x,y`.
    pub z: f64,
    pub snap: Option<SnapRef>,
    pub typed: bool,
}

impl Taken {
    /// A point typed or worked out, on no snap, at height `z`.
    pub fn typed(at: [f64; 2], z: f64) -> Self {
        Taken {
            at,
            z,
            snap: None,
            typed: true,
        }
    }

    /// The point as an operation writes it: `[x, y, z]`, a Z of 0 as `0`.
    fn coordinates(&self) -> Value {
        if self.z == 0.0 {
            json!([self.at[0], self.at[1], 0])
        } else {
            json!([self.at[0], self.at[1], self.z])
        }
    }
}

/// One of the sheet's objects, as the tools need it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjectInfo {
    pub id: Option<String>,
    pub name: Option<String>,
    pub kind: String,
    pub block: Option<String>,
    /// The layer it sits on.
    pub layer: String,
    /// What quick properties edit of it, as `display-list` with `values`
    /// gives them; none when the core did not give them.
    pub values: Option<Value>,
    /// Whether it is written in the sheet's template; none when the core did
    /// not say, as for a sheet description.
    pub from_template: Option<bool>,
    /// Whether it is model space seen through a viewport of the layout
    /// drawn, whose points on the page are the viewport's, not its own.
    pub through_viewport: bool,
}

impl ObjectInfo {
    /// Reads what the core says of an object in a `display-list` reply.
    pub fn of(value: &Value) -> Self {
        let text = |k: &str| value.get(k).and_then(Value::as_str).map(str::to_owned);
        ObjectInfo {
            id: text("id"),
            name: text("name"),
            kind: text("type").unwrap_or_default(),
            block: text("block"),
            layer: text("layer").unwrap_or_default(),
            values: value.get("values").cloned(),
            from_template: text("from").map(|f| f == "template"),
            through_viewport: false,
        }
    }

    /// What the command bar calls it: its name, its ID's own part, or its
    /// block or type.
    pub fn words(&self) -> String {
        if let Some(name) = &self.name {
            return name.clone();
        }
        if let Some(own) = self.id.as_deref().and_then(|id| id.split_once('/')) {
            if !own.1.starts_with("objects[") {
                return own.1.to_owned();
            }
        }
        self.block.clone().unwrap_or_else(|| self.kind.clone())
    }
}

/// A sheet's template and every sheet built from it, as `display-list
/// --origins` gives them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reach {
    pub path: String,
    pub sheets: Vec<String>,
}

/// What the tools know of the sheet shown.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Context {
    /// The sheet, by name.
    pub sheet: String,
    /// Its unit, such as `in`; none when it states none.
    pub units: Option<String>,
    pub objects: Vec<ObjectInfo>,
    /// Its template, on a drawing set's sheet: none for a sheet description
    /// or a DWG, which a change set does not edit.
    pub template: Option<Reach>,
    /// Whether it is the set as it was at an entry, read-only.
    pub past: bool,
    /// On a sheet drawn in a layout, the space a new object goes in, as an
    /// add names it: `paper`, or the layout's name on a sheet drawn in
    /// several; none on a sheet drawn in model space.
    pub space: Option<String>,
    /// On a sheet drawn in several layouts, the one shown.
    pub layout: Option<String>,
}

/// What a tool wants next.
#[derive(Clone, Debug, PartialEq)]
enum Step {
    LineFirst,
    LineNext {
        points: Vec<Taken>,
    },
    CircleCenter,
    CircleRadius {
        center: Taken,
        diameter: bool,
    },
    RectangleFirst,
    RectangleSecond {
        first: Taken,
    },
    TextStart,
    TextHeight,
    TextType {
        at: Taken,
        lines: Vec<(String, Taken)>,
        current: String,
    },
    Select {
        picked: Vec<usize>,
    },
    MoveBase {
        picked: Vec<usize>,
    },
    MoveSecond {
        picked: Vec<usize>,
        base: Taken,
    },
    CopyBase {
        picked: Vec<usize>,
    },
    /// Copy's second points: each taken places a copy, by how far it is
    /// from the base point, and the tool goes on from the same base point
    /// until Enter, Done or Esc lands them all, as one change set.
    CopySecond {
        picked: Vec<usize>,
        base: Taken,
        placed: Vec<Taken>,
    },
}

#[derive(Clone, Debug, PartialEq)]
struct Running {
    kind: ToolKind,
    step: Step,
}

/// An edit to a template's objects, which the page asks about before it
/// sends it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TemplateEdit {
    /// `This is part of the template.`
    pub title: String,
    /// `Moving it changes 8 sheets.`
    pub detail: String,
    /// The template, as the rows write its path.
    pub path: String,
    /// Every sheet built from it, which the edit changes.
    pub sheets: Vec<String>,
    /// The edit, in a few words, as a preview names it before it lands:
    /// `Move junction-box`.
    pub edit: String,
}

/// A change set a tool has ready to send.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Send {
    /// The change set, `{"operations": [...]}`.
    pub change_set: Value,
    /// What it does, in a few words: `2 lines drawn`.
    pub what: String,
    /// The sheets it changes.
    pub sheets: Vec<String>,
    /// When it edits a template's object: what to ask first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<TemplateEdit>,
}

/// What a step of a tool came to: lines for the command bar's history,
/// and the change sets to send, in order, if the tool has what it needs:
/// two when a tool ended by another kept what it drew and the other sent
/// at once, as Erase on objects already selected does.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Outcome {
    pub log: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sends: Vec<Send>,
}

/// An option of a prompt, to click.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Choice {
    pub label: String,
    /// What it does, as [`Tools::option`] takes it.
    pub act: &'static str,
    /// The option a phone shows as its main button.
    pub main: bool,
}

/// A line of text a Text tool has typed, and where it goes.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TextLine {
    pub text: String,
    pub at: [f64; 2],
}

/// The text a Text tool is typing: each line at its place, the line being
/// typed last, and their height, for the page to show where it will go.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Typing {
    pub height: f64,
    pub lines: Vec<TextLine>,
}

/// What the command bar shows of the tool running.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct State {
    /// The tool running, as the page names it; none when none runs.
    pub tool: Option<ToolKind>,
    /// AutoCAD's command, such as `LINE`.
    pub command: Option<&'static str>,
    /// The layer it draws on, for a tool that draws.
    pub layer: Option<String>,
    /// What it asks for, in plain words.
    pub prompt: String,
    pub options: Vec<Choice>,
    /// What went wrong with the last thing typed or clicked.
    pub error: Option<String>,
    /// That the objects picked are part of the template, and what moving or
    /// erasing them reaches.
    pub warning: Option<TemplateEdit>,
    /// What the command bar's field says when it is empty.
    pub placeholder: String,
    /// Whether what is typed is the text itself, so Space is a space, not
    /// Enter.
    pub typing_text: bool,
    /// Whether it takes points now, so a click takes one and the crosshair
    /// has no pick box.
    pub takes_points: bool,
    /// Whether it picks objects now.
    pub selecting: bool,
    /// The objects it has picked, by their places.
    pub picked: Vec<usize>,
    /// The points it has taken, which the page marks small.
    pub points: Vec<[f64; 2]>,
    /// The text a Text tool is typing.
    pub text: Option<Typing>,
    /// The copies Copy has placed, which wait for Enter, Done or Esc.
    pub placed: usize,
}

/// What a running tool shows before it lands, in drawing units.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Band {
    /// The shape it would draw, in the current layer's colour.
    pub lines: Vec<[[f64; 2]; 2]>,
    /// Guides, dashed: from a circle's centre, or a move's base point.
    pub guides: Vec<[[f64; 2]; 2]>,
    /// The objects a move would move, and by how much.
    pub moving: Option<(Vec<usize>, [f64; 2])>,
    /// The objects a copy would copy, and by how much: the copy following
    /// the cursor, dashed.
    pub copying: Option<(Vec<usize>, [f64; 2])>,
    /// The objects Copy copies, and the offset of each copy placed, which
    /// waits for Enter: dashed in the proposal's colour.
    pub placed: Option<(Vec<usize>, Vec<[f64; 2]>)>,
    /// The points taken so far.
    pub points: Vec<[f64; 2]>,
}

/// How many straight pieces a circle's rubber band is drawn in.
const CIRCLE_PIECES: usize = 96;

/// The drawing tools' state: the tool running, the last one, Ortho, the
/// current layer and the text height.
#[derive(Clone, Debug, PartialEq)]
pub struct Tools {
    running: Option<Running>,
    last: Option<ToolKind>,
    /// Ortho (F8): Line and Move held level or upright.
    pub ortho: bool,
    /// The current layer, which new objects go on.
    pub layer: String,
    text_height: Option<f64>,
    /// The last point taken, which `@dx,dy` is measured from when no tool
    /// has its own, as AutoCAD's LASTPOINT.
    last_point: Option<([f64; 2], f64)>,
    error: Option<String>,
    context: Context,
}

impl Default for Tools {
    fn default() -> Self {
        Tools {
            running: None,
            last: None,
            ortho: false,
            layer: "0".to_owned(),
            text_height: None,
            last_point: None,
            error: None,
            context: Context::default(),
        }
    }
}

/// A coordinate in words, as the status bar gives it.
pub(crate) fn f4(v: f64) -> String {
    format!("{:.4}", if v.abs() < 5e-5 { 0.0 } else { v })
}

/// A length in words.
fn f3(v: f64) -> String {
    format!("{:.3}", if v.abs() < 5e-4 { 0.0 } else { v })
}

pub(crate) fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// A number typed: digits with a point and a sign.
pub(crate) fn number(text: &str) -> Option<f64> {
    let text = text.trim();
    let ok = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+'));
    ok.then(|| text.parse::<f64>().ok())
        .flatten()
        .filter(|v| v.is_finite())
}

/// A point typed, as AutoCAD reads one: `x,y`, `@dx,dy` from `base`, or
/// `@length<angle` from `base`, the angle in degrees anticlockwise from
/// the X axis. `None` when the text is not a point; an error when it is
/// relative and there is no point to measure from.
fn typed_point(text: &str, base: Option<[f64; 2]>) -> Option<Result<[f64; 2], String>> {
    let text = text.trim();
    let (relative, rest) = match text.strip_prefix('@') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let need_base = || "There is no last point yet: type x,y, or click.".to_owned();
    if let Some((x, y)) = rest.split_once(',') {
        let (x, y) = (number(x)?, number(y)?);
        if !relative {
            return Some(Ok([x, y]));
        }
        return Some(base.map(|b| [b[0] + x, b[1] + y]).ok_or_else(need_base));
    }
    if relative {
        if let Some((length, angle)) = rest.split_once('<') {
            let (length, angle) = (number(length)?, number(angle)?.to_radians());
            return Some(
                base.map(|b| [b[0] + length * angle.cos(), b[1] + length * angle.sin()])
                    .ok_or_else(need_base),
            );
        }
    }
    None
}

/// A point clicked on no snap, to a ten-thousandth of the unit, as the
/// status bar shows it, so the record is not written to seventeen digits.
fn tidy(v: f64) -> f64 {
    let r = (v * 1e4).round() / 1e4;
    if r == 0.0 {
        0.0
    } else {
        r
    }
}

/// A difference of coordinates cleared of the noise subtracting leaves,
/// such as 1.2000000000000002 for 1.2.
fn clean(v: f64) -> f64 {
    let r = (v * 1e10).round() / 1e10;
    if r == 0.0 {
        0.0
    } else {
        r
    }
}

/// How far `to` is from `from`, across and up.
fn offset(from: &Taken, to: &Taken) -> [f64; 2] {
    [clean(to.at[0] - from.at[0]), clean(to.at[1] - from.at[1])]
}

/// The length from `a` to `b`, and its angle in degrees from the X axis,
/// from 0 to 360.
fn length_angle(a: [f64; 2], b: [f64; 2]) -> (f64, f64) {
    let length = (b[0] - a[0]).hypot(b[1] - a[1]);
    let mut angle = (b[1] - a[1]).atan2(b[0] - a[0]).to_degrees();
    if angle < 0.0 {
        angle += 360.0;
    }
    (length, angle)
}

impl Tools {
    /// The sheet shown, as a `display-list` reply gives it. A tool running
    /// on the same sheet and layout keeps its picks and points, followed by
    /// their objects' IDs; another sheet, or another layout of it, drops
    /// it, with nothing sent: its points were taken on another page.
    pub fn load(&mut self, context: Context) {
        let same_layout = match (&context.layout, &self.context.layout) {
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
            (None, None) => true,
            _ => false,
        };
        let same =
            context.sheet.eq_ignore_ascii_case(&self.context.sheet) && same_layout && !context.past;
        if !same {
            self.running = None;
        } else if let Some(running) = &mut self.running {
            let old = &self.context.objects;
            let new = &context.objects;
            // An object's new place, by its ID; none once it is gone.
            let place = |p: usize| -> Option<usize> {
                let id = old.get(p)?.id.as_deref()?;
                new.iter().position(|o| o.id.as_deref() == Some(id))
            };
            let follow = |t: &mut Taken| {
                if let Some(s) = &mut t.snap {
                    match (place(s.object), s.with.map(place)) {
                        (Some(o), None) => s.object = o,
                        (Some(o), Some(Some(w))) => {
                            s.object = o;
                            s.with = Some(w);
                        }
                        // The point stays where it was taken.
                        _ => t.snap = None,
                    }
                }
            };
            let follow_picked = |picked: &mut Vec<usize>| {
                *picked = picked.iter().filter_map(|&p| place(p)).collect();
            };
            match &mut running.step {
                Step::LineNext { points } => points.iter_mut().for_each(follow),
                Step::CircleRadius { center, .. } => follow(center),
                Step::RectangleSecond { first } => follow(first),
                Step::TextType { at, lines, .. } => {
                    follow(at);
                    lines.iter_mut().for_each(|(_, t)| follow(t));
                }
                Step::Select { picked } | Step::MoveBase { picked } | Step::CopyBase { picked } => {
                    follow_picked(picked)
                }
                Step::MoveSecond { picked, base } => {
                    follow_picked(picked);
                    follow(base);
                }
                Step::CopySecond {
                    picked,
                    base,
                    placed,
                } => {
                    follow_picked(picked);
                    follow(base);
                    placed.iter_mut().for_each(follow);
                }
                _ => {}
            }
            // A move or an erase whose objects have all gone has nothing
            // left to do.
            if let Step::MoveBase { picked }
            | Step::MoveSecond { picked, .. }
            | Step::CopyBase { picked }
            | Step::CopySecond { picked, .. } = &running.step
            {
                if picked.is_empty() {
                    self.running = None;
                }
            }
        }
        self.context = context;
    }

    /// What the tools know of the sheet shown.
    pub fn context(&self) -> &Context {
        &self.context
    }

    /// The tool running, if any.
    pub fn running(&self) -> Option<ToolKind> {
        self.running.as_ref().map(|r| r.kind)
    }

    /// Whether the tool running takes a point now.
    pub fn takes_points(&self) -> bool {
        matches!(
            self.running.as_ref().map(|r| &r.step),
            Some(
                Step::LineFirst
                    | Step::LineNext { .. }
                    | Step::CircleCenter
                    | Step::CircleRadius { .. }
                    | Step::RectangleFirst
                    | Step::RectangleSecond { .. }
                    | Step::TextStart
                    | Step::MoveBase { .. }
                    | Step::MoveSecond { .. }
                    | Step::CopyBase { .. }
                    | Step::CopySecond { .. }
            )
        )
    }

    /// Whether the tool running picks objects now.
    pub fn selecting(&self) -> bool {
        matches!(
            self.running.as_ref().map(|r| &r.step),
            Some(Step::Select { .. })
        )
    }

    /// The height new text is drawn at: the last one typed, or a beginner's
    /// size for the sheet's unit.
    pub fn text_height(&self) -> f64 {
        self.text_height
            .unwrap_or(match self.context.units.as_deref() {
                Some("in") => 0.12,
                Some("ft") => 0.01,
                Some("mm") => 3.0,
                Some("cm") => 0.3,
                Some("m") => 0.003,
                _ => 1.0,
            })
    }

    fn unit(&self) -> String {
        match self.context.units.as_deref() {
            Some(u) => format!(" {u}"),
            None => String::new(),
        }
    }

    fn unit_name(&self) -> &'static str {
        match self.context.units.as_deref() {
            Some("in") => "inches",
            Some("ft") => "feet",
            Some("mm") => "millimetres",
            Some("cm") => "centimetres",
            Some("m") => "metres",
            _ => "drawing units",
        }
    }

    fn object_words(&self, place: usize) -> String {
        self.context
            .objects
            .get(place)
            .map_or_else(|| "?".to_owned(), ObjectInfo::words)
    }

    /// A point in the command bar's history: its snap, then where it is.
    fn point_words(&self, t: &Taken) -> String {
        let at = format!("{}, {}", f4(t.at[0]), f4(t.at[1]));
        match &t.snap {
            Some(s) => {
                let of = match s.with {
                    Some(w) => format!(
                        "{} and {}",
                        self.object_words(s.object),
                        self.object_words(w)
                    ),
                    None => self.object_words(s.object),
                };
                format!("{} of {of}, {at}", snap_label(s.snap))
            }
            None => at,
        }
    }

    /// Starts `kind`, ending the tool running first, as another tool does:
    /// lines and text it has drawn are kept. `picked` are the objects
    /// selected before, which Move and Erase take, as AutoCAD's do when
    /// objects are selected first.
    pub fn start(&mut self, kind: ToolKind, picked: &[usize]) -> Outcome {
        self.error = None;
        if self.context.template.is_none() || self.context.past {
            self.error = Some(if self.context.past {
                "This is the set as it was at an entry, read-only: go back to live to draw."
                    .to_owned()
            } else {
                "The drawing tools change a drawing set; this is a sheet description or a DWG, \
                 which a change set does not edit."
                    .to_owned()
            });
            return Outcome::default();
        }
        let mut outcome = self.end(false);
        self.last = Some(kind);
        let mut picked: Vec<usize> = picked
            .iter()
            .copied()
            .filter(|&p| self.nameable(p))
            .collect();
        if matches!(kind, ToolKind::Move | ToolKind::Copy) {
            if let Some(&p) = picked.iter().find(|&&p| self.through_viewport(p)) {
                self.error = Some(self.viewport_words(kind, p));
                picked.retain(|&p| !self.through_viewport(p));
            }
        }
        let step = match kind {
            ToolKind::Line => Step::LineFirst,
            ToolKind::Circle => Step::CircleCenter,
            ToolKind::Rectangle => Step::RectangleFirst,
            ToolKind::Text => Step::TextStart,
            ToolKind::Move if !picked.is_empty() => Step::MoveBase {
                picked: picked.clone(),
            },
            ToolKind::Copy if !picked.is_empty() => Step::CopyBase {
                picked: picked.clone(),
            },
            ToolKind::Move | ToolKind::Copy | ToolKind::Erase => Step::Select {
                picked: picked.clone(),
            },
        };
        outcome.log.push(if picked.is_empty() {
            kind.command().to_owned()
        } else {
            format!("{} {}", kind.command(), self.picked_words(&picked))
        });
        self.running = Some(Running { kind, step });
        // Erase with objects already selected erases them at once, after
        // what the tool it ended kept, if any: both are sent, in order.
        if kind == ToolKind::Erase && !picked.is_empty() {
            let done = self.finish_select();
            outcome.log.extend(done.log);
            outcome.sends.extend(done.sends);
        }
        outcome
    }

    /// Whether the object at `place` is model space seen through a
    /// viewport, whose points on the page are not its own.
    fn through_viewport(&self, place: usize) -> bool {
        self.context
            .objects
            .get(place)
            .is_some_and(|o| o.through_viewport)
    }

    /// Why Move and Copy do not take an object seen through a viewport.
    fn viewport_words(&self, kind: ToolKind, place: usize) -> String {
        let (tool, verb) = if kind == ToolKind::Copy {
            ("Copy", "copies")
        } else {
            ("Move", "moves")
        };
        format!(
            "{} is in model space, seen through a viewport: {tool} {verb} only what is drawn \
             on the paper here.",
            self.object_words(place)
        )
    }

    /// Whether a change set can name the object at `place`: it has an ID.
    fn nameable(&self, place: usize) -> bool {
        self.context
            .objects
            .get(place)
            .is_some_and(|o| o.id.is_some())
    }

    fn picked_words(&self, picked: &[usize]) -> String {
        match picked {
            [one] => self.object_words(*one),
            _ => plural(picked.len(), "object", "objects"),
        }
    }

    /// Ends the tool running: Esc, Done, or another tool. A tool that has
    /// drawn something keeps it, as AutoCAD's LINE and TEXT do; anything
    /// else ends with nothing changed. `said` says so in the history.
    fn end(&mut self, said: bool) -> Outcome {
        let Some(running) = self.running.take() else {
            return Outcome::default();
        };
        match running.step {
            Step::LineNext { points } if points.len() >= 2 => self.land_lines(&points),
            // Copy keeps the copies it placed, as AutoCAD's COPY does: Esc
            // lands them as Enter does.
            Step::CopySecond {
                picked,
                base,
                placed,
            } if !placed.is_empty() => self.land_copy(&picked, &base, &placed),
            Step::TextType {
                at,
                mut lines,
                current,
            } => {
                if !current.trim().is_empty() {
                    lines.push((current, at));
                }
                if lines.is_empty() {
                    self.cancelled(running.kind, said)
                } else {
                    self.land_text(&lines)
                }
            }
            _ => self.cancelled(running.kind, said),
        }
    }

    fn cancelled(&self, kind: ToolKind, said: bool) -> Outcome {
        Outcome {
            log: if said {
                vec![format!("{}: cancelled, nothing changed", kind.command())]
            } else {
                Vec::new()
            },
            sends: Vec::new(),
        }
    }

    /// Esc: ends the tool running, keeping what it has drawn.
    pub fn escape(&mut self) -> Outcome {
        self.error = None;
        self.end(true)
    }

    /// Clears what went wrong, as its OK does.
    pub fn clear_error(&mut self) {
        self.error = None;
    }

    /// The point the next one is measured from: Ortho, a length typed alone
    /// and the rubber band use it.
    pub fn base(&self) -> Option<[f64; 2]> {
        self.base_taken().map(|t| t.at)
    }

    /// The point the next one is measured from, as it was taken.
    fn base_taken(&self) -> Option<&Taken> {
        match &self.running.as_ref()?.step {
            Step::LineNext { points } => points.last(),
            Step::CircleRadius { center, .. } => Some(center),
            Step::RectangleSecond { first } => Some(first),
            Step::MoveSecond { base, .. } | Step::CopySecond { base, .. } => Some(base),
            _ => None,
        }
    }

    /// Whether Ortho holds the next point level or upright with the base.
    fn ortho_applies(&self) -> bool {
        self.ortho
            && matches!(
                self.running.as_ref().map(|r| &r.step),
                Some(Step::LineNext { .. } | Step::MoveSecond { .. } | Step::CopySecond { .. })
            )
    }

    /// Where the next point would go: on the snap the cursor holds, or at
    /// the cursor, held level or upright by Ortho. A snap wins over Ortho,
    /// as AutoCAD's object snaps override it.
    pub fn aim(&self, cursor: [f64; 2], snap: Option<(SnapRef, [f64; 3])>) -> Taken {
        if let Some((snap, at)) = snap {
            return Taken {
                at: [at[0], at[1]],
                z: at[2],
                snap: Some(snap),
                typed: false,
            };
        }
        let mut at = cursor;
        if self.ortho_applies() {
            if let Some(base) = self.base() {
                if (at[0] - base[0]).abs() >= (at[1] - base[1]).abs() {
                    at[1] = base[1];
                } else {
                    at[0] = base[0];
                }
            }
        }
        Taken {
            at,
            z: 0.0,
            snap: None,
            typed: false,
        }
    }

    /// Takes a point clicked, tapped or typed.
    pub fn point(&mut self, point: Taken) -> Outcome {
        self.error = None;
        let mut point = point;
        if point.snap.is_none() && !point.typed {
            // A coordinate Ortho took from the base point stays exactly
            // the base's, so the line is exactly level or upright.
            let base = self.base();
            for k in 0..2 {
                if base.is_none_or(|b| b[k] != point.at[k]) {
                    point.at[k] = tidy(point.at[k]);
                }
            }
        }
        let Some(running) = self.running.take() else {
            return Outcome::default();
        };
        let kind = running.kind;
        let mut log = Vec::new();
        let where_ = self.point_words(&point);
        let next = match running.step {
            Step::LineFirst => {
                log.push(format!("LINE from {where_}"));
                Some(Step::LineNext {
                    points: vec![point.clone()],
                })
            }
            Step::LineNext { mut points } => {
                let last = points.last().map(|p| p.at).unwrap_or(point.at);
                if (point.at[0] - last[0]).hypot(point.at[1] - last[1]) < 1e-9 {
                    Some(Step::LineNext { points })
                } else {
                    let (length, angle) = length_angle(last, point.at);
                    let ortho = if self.ortho && point.snap.is_none() && !point.typed {
                        " (Ortho)"
                    } else {
                        ""
                    };
                    log.push(format!(
                        "LINE to {where_}: {}{} at {:.1}°{ortho}",
                        f3(length),
                        self.unit(),
                        angle
                    ));
                    points.push(point.clone());
                    Some(Step::LineNext { points })
                }
            }
            Step::CircleCenter => {
                log.push(format!("CIRCLE centre {where_}"));
                Some(Step::CircleRadius {
                    center: point.clone(),
                    diameter: false,
                })
            }
            Step::CircleRadius { center, diameter } => {
                let mut radius = (point.at[0] - center.at[0]).hypot(point.at[1] - center.at[1]);
                // A free click gives a radius to a ten-thousandth, as it
                // gives its points.
                if point.snap.is_none() && !point.typed {
                    radius = tidy(radius);
                }
                if radius < 1e-9 {
                    Some(Step::CircleRadius { center, diameter })
                } else {
                    let through = match &point.snap {
                        Some(_) => format!(" through {}", self.point_words(&point)),
                        None => String::new(),
                    };
                    self.last_point = Some((point.at, point.z));
                    let mut outcome = self.land_circle(&center, radius, &through);
                    outcome.log.splice(0..0, log);
                    return outcome;
                }
            }
            Step::RectangleFirst => {
                log.push(format!("RECTANG first corner {where_}"));
                Some(Step::RectangleSecond {
                    first: point.clone(),
                })
            }
            Step::RectangleSecond { first } => {
                if (point.at[0] - first.at[0]).abs() < 1e-9
                    || (point.at[1] - first.at[1]).abs() < 1e-9
                {
                    self.error = Some(
                        "The corners must differ across and up: that would draw a line.".to_owned(),
                    );
                    Some(Step::RectangleSecond { first })
                } else {
                    self.last_point = Some((point.at, point.z));
                    return self.land_rectangle(&first, &point);
                }
            }
            Step::TextStart => {
                log.push(format!("TEXT starts at {where_}"));
                Some(Step::TextType {
                    at: point.clone(),
                    lines: Vec::new(),
                    current: String::new(),
                })
            }
            Step::MoveBase { picked } => {
                log.push(format!("MOVE base point {where_}"));
                Some(Step::MoveSecond {
                    picked,
                    base: point.clone(),
                })
            }
            Step::MoveSecond { picked, base } => {
                if (point.at[0] - base.at[0]).hypot(point.at[1] - base.at[1]) < 1e-9 {
                    Some(Step::MoveSecond { picked, base })
                } else {
                    self.last_point = Some((point.at, point.z));
                    let mut outcome = self.land_move(&picked, &base, &point);
                    outcome
                        .log
                        .insert(0, format!("MOVE to {}", self.point_words(&point)));
                    return outcome;
                }
            }
            Step::CopyBase { picked } => {
                log.push(format!("COPY base point {where_}"));
                Some(Step::CopySecond {
                    picked,
                    base: point.clone(),
                    placed: Vec::new(),
                })
            }
            Step::CopySecond {
                picked,
                base,
                mut placed,
            } => {
                if (point.at[0] - base.at[0]).hypot(point.at[1] - base.at[1]) < 1e-9 {
                    self.error = Some(
                        "That is the base point: a copy there would sit on the original."
                            .to_owned(),
                    );
                } else {
                    let [dx, dy] = offset(&base, &point);
                    placed.push(point.clone());
                    log.push(format!(
                        "COPY {}: {}, {}{} from the base point, dashed until Enter",
                        placed.len(),
                        f4(dx),
                        f4(dy),
                        self.unit()
                    ));
                }
                Some(Step::CopySecond {
                    picked,
                    base,
                    placed,
                })
            }
            other => Some(other),
        };
        self.last_point = Some((point.at, point.z));
        self.running = next.map(|step| Running { kind, step });
        Outcome {
            log,
            sends: Vec::new(),
        }
    }

    /// Picks the object at `place`, or takes it out when it is picked, in
    /// a tool that picks objects; nothing when nothing was there.
    pub fn pick(&mut self, place: Option<usize>) -> Outcome {
        self.error = None;
        let Some(place) = place else {
            return Outcome::default();
        };
        let words = self.object_words(place);
        let nameable = self.nameable(place);
        let viewport = self.through_viewport(place);
        let Some(Running {
            kind,
            step: Step::Select { picked },
        }) = &mut self.running
        else {
            return Outcome::default();
        };
        let command = kind.command();
        if matches!(kind, ToolKind::Move | ToolKind::Copy) && viewport {
            let kind = *kind;
            self.error = Some(self.viewport_words(kind, place));
            return Outcome::default();
        }
        if !nameable {
            self.error = Some(format!(
                "{words} has no ID, so a change set cannot name it."
            ));
            return Outcome::default();
        }
        let line = if let Some(i) = picked.iter().position(|&p| p == place) {
            picked.remove(i);
            format!("{command}: {words} taken out")
        } else {
            picked.push(place);
            format!(
                "{command}: {words} picked, {}",
                plural(picked.len(), "object", "objects")
            )
        };
        Outcome {
            log: vec![line],
            sends: Vec::new(),
        }
    }

    /// The picking is done: Move asks for its base point, Erase erases.
    fn finish_select(&mut self) -> Outcome {
        let Some(Running {
            kind,
            step: Step::Select { picked },
        }) = self.running.clone()
        else {
            return Outcome::default();
        };
        if picked.is_empty() {
            return self.end(true);
        }
        if kind == ToolKind::Move {
            self.running = Some(Running {
                kind,
                step: Step::MoveBase { picked },
            });
            return Outcome::default();
        }
        if kind == ToolKind::Copy {
            self.running = Some(Running {
                kind,
                step: Step::CopyBase { picked },
            });
            return Outcome::default();
        }
        self.running = None;
        self.land_erase(&picked)
    }

    /// The line of text being typed, as it is typed.
    pub fn typing(&mut self, text: &str) {
        if let Some(Running {
            step: Step::TextType { current, .. },
            ..
        }) = &mut self.running
        {
            *current = text.replace(['\r', '\n'], "");
        }
    }

    /// Text typed in a phone's small box, at its height: added at once.
    pub fn text_box(&mut self, text: &str, height: Option<f64>) -> Outcome {
        if let Some(h) = height.filter(|h| h.is_finite() && *h > 0.0) {
            self.text_height = Some(h);
        }
        if text.trim().is_empty() {
            self.error = Some("Type the text first.".to_owned());
            return Outcome::default();
        }
        self.typing(text);
        self.end(true)
    }

    /// An option clicked: `done`, `cancel`, `undo`, `close`, `diameter`,
    /// `radius`, `height` or `ok`.
    pub fn option(&mut self, act: &str) -> Outcome {
        match act {
            "ok" => {
                self.error = None;
                Outcome::default()
            }
            // Cancel drops the line being typed, as in a phone's text box;
            // Esc keeps it, as AutoCAD's TEXT does.
            "cancel" => {
                if let Some(Running {
                    step: Step::TextType { current, .. },
                    ..
                }) = &mut self.running
                {
                    current.clear();
                }
                self.escape()
            }
            // Done finishes the text, the line being typed with it.
            "done"
                if matches!(
                    self.running.as_ref().map(|r| &r.step),
                    Some(Step::TextType { .. })
                ) =>
            {
                self.error = None;
                self.end(true)
            }
            "done" => self.input("", None),
            "undo" => self.input("U", None),
            "close" => self.input("C", None),
            "diameter" => self.input("D", None),
            "radius" => self.input("R", None),
            "height" => self.input("H", None),
            _ => Outcome::default(),
        }
    }

    /// What was typed in the command bar and entered, `aim` the point the
    /// cursor aims at, which a length typed alone goes towards. Empty is
    /// Enter, Space or a right-click: Done, or with no tool running, the
    /// last tool again.
    pub fn input(&mut self, text: &str, aim: Option<[f64; 2]>) -> Outcome {
        self.error = None;
        let typed = text.trim();
        let upper = typed.to_ascii_uppercase();
        let Some(running) = self.running.clone() else {
            if typed.is_empty() {
                return match self.last {
                    Some(kind) => self.start(kind, &[]),
                    None => Outcome::default(),
                };
            }
            if let Some(kind) = tool_for(typed) {
                return self.start(kind, &[]);
            }
            self.error = Some(format!(
                "“{typed}” is not a drawing tool: try L, C, REC, T, M, CO or E."
            ));
            return Outcome {
                log: vec![format!("“{typed}” is not a drawing tool")],
                sends: Vec::new(),
            };
        };
        let kind = running.kind;
        // Typing the text itself: Enter takes the line, Enter on an empty
        // line finishes.
        if let Step::TextType {
            at,
            mut lines,
            current,
        } = running.step.clone()
        {
            let line = if typed.is_empty() {
                current.clone()
            } else {
                text.to_owned()
            };
            if line.trim().is_empty() {
                return self.end(true);
            }
            let next = Taken::typed([at.at[0], at.at[1] - self.text_height() * 5.0 / 3.0], at.z);
            lines.push((line.clone(), at));
            self.running = Some(Running {
                kind,
                step: Step::TextType {
                    at: next,
                    lines,
                    current: String::new(),
                },
            });
            return Outcome {
                log: vec![format!("TEXT: “{line}”; Enter again to finish")],
                sends: Vec::new(),
            };
        }
        if let Step::TextHeight = running.step {
            if typed.is_empty() {
                self.running = Some(Running {
                    kind,
                    step: Step::TextStart,
                });
                return Outcome::default();
            }
            return match number(typed).filter(|h| *h > 0.0) {
                Some(h) => {
                    self.text_height = Some(h);
                    self.running = Some(Running {
                        kind,
                        step: Step::TextStart,
                    });
                    Outcome {
                        log: vec![format!("TEXT height {h}{}", self.unit())],
                        sends: Vec::new(),
                    }
                }
                None => {
                    self.error =
                        Some("Type the height as a number more than 0, such as 0.12.".to_owned());
                    Outcome::default()
                }
            };
        }
        // Another tool's alias starts it, but not the letters this tool's
        // own options take.
        let own_option = matches!(
            (&running.step, upper.as_str()),
            (Step::LineNext { .. }, "C" | "U") | (Step::CircleRadius { .. }, "D")
        );
        if let Some(other) = tool_for(typed).filter(|_| !own_option) {
            return self.start(other, &[]);
        }
        if typed.is_empty() {
            return match running.step {
                Step::Select { .. } => self.finish_select(),
                _ => self.end(true),
            };
        }
        match (&running.step, upper.as_str()) {
            (Step::Select { .. }, _) => {
                self.error = Some("Click objects, then Enter or Done.".to_owned());
                return Outcome::default();
            }
            (Step::LineNext { points }, "U" | "UNDO") => {
                let mut points = points.clone();
                points.pop();
                self.running = Some(Running {
                    kind,
                    step: if points.is_empty() {
                        Step::LineFirst
                    } else {
                        Step::LineNext { points }
                    },
                });
                return Outcome {
                    log: vec!["LINE: last point taken back".to_owned()],
                    sends: Vec::new(),
                };
            }
            (
                Step::CopySecond {
                    picked,
                    base,
                    placed,
                },
                "U" | "UNDO",
            ) => {
                if placed.is_empty() {
                    self.error = Some("No copy is placed yet to take back.".to_owned());
                    return Outcome::default();
                }
                let mut placed = placed.clone();
                placed.pop();
                let left = plural(placed.len(), "copy", "copies");
                self.running = Some(Running {
                    kind,
                    step: Step::CopySecond {
                        picked: picked.clone(),
                        base: base.clone(),
                        placed,
                    },
                });
                return Outcome {
                    log: vec![format!("COPY: last copy taken back, {left} placed")],
                    sends: Vec::new(),
                };
            }
            (Step::LineNext { points }, "C" | "CLOSE") => {
                if points.len() < 3 {
                    self.error = Some("Close needs two lines drawn first.".to_owned());
                    return Outcome::default();
                }
                let mut points = points.clone();
                points.push(points[0].clone());
                self.running = None;
                return self.land_lines(&points);
            }
            (Step::CircleRadius { center, .. }, "D" | "DIAMETER" | "R" | "RADIUS") => {
                // Each word sets its own mode: RADIUS never turns Diameter on.
                let diameter = matches!(upper.as_str(), "D" | "DIAMETER");
                self.running = Some(Running {
                    kind,
                    step: Step::CircleRadius {
                        center: center.clone(),
                        diameter,
                    },
                });
                return Outcome {
                    log: vec![format!(
                        "CIRCLE: type the {}",
                        if diameter { "diameter" } else { "radius" }
                    )],
                    sends: Vec::new(),
                };
            }
            (Step::TextStart, "H" | "HEIGHT") => {
                self.running = Some(Running {
                    kind,
                    step: Step::TextHeight,
                });
                return Outcome::default();
            }
            _ => {}
        }
        let base = self.base_taken().map(|t| (t.at, t.z)).or(self.last_point);
        match typed_point(typed, base.map(|b| b.0)) {
            Some(Ok(at)) => {
                // A point typed relative to another is at its height.
                let z = if typed.starts_with('@') {
                    base.map_or(0.0, |b| b.1)
                } else {
                    0.0
                };
                return self.point(Taken::typed(at, z));
            }
            Some(Err(why)) => {
                self.error = Some(why);
                return Outcome::default();
            }
            None => {}
        }
        if let Some(length) = number(typed) {
            if let Step::CircleRadius { center, diameter } = &running.step {
                let radius = if *diameter { length / 2.0 } else { length };
                if radius <= 0.0 {
                    self.error = Some("A radius is more than 0.".to_owned());
                    return Outcome::default();
                }
                let center = center.clone();
                self.running = None;
                let how = if *diameter {
                    format!(" (diameter {length} typed)")
                } else {
                    " (radius typed)".to_owned()
                };
                return self.land_circle(&center, radius, &how);
            }
            if matches!(
                running.step,
                Step::LineNext { .. } | Step::MoveSecond { .. } | Step::CopySecond { .. }
            ) {
                if let Some(base) = self.base() {
                    // As AutoCAD's direct distance entry: that far towards
                    // the cursor, held by Ortho.
                    let toward = aim.map(|a| self.aim(a, None).at);
                    let (mut dx, mut dy) =
                        toward.map_or((1.0, 0.0), |a| (a[0] - base[0], a[1] - base[1]));
                    let far = dx.hypot(dy);
                    if far < 1e-12 {
                        (dx, dy) = (1.0, 0.0);
                    }
                    let far = dx.hypot(dy);
                    let z = self.base_taken().map_or(0.0, |t| t.z);
                    return self.point(Taken::typed(
                        [base[0] + length * dx / far, base[1] + length * dy / far],
                        z,
                    ));
                }
            }
            self.error = Some("A length alone needs a first point: type x,y or click.".to_owned());
            return Outcome::default();
        }
        let lengths = matches!(
            running.step,
            Step::LineNext { .. } | Step::MoveSecond { .. } | Step::CopySecond { .. }
        );
        self.error = Some(format!(
            "“{typed}” is not a point. Type x,y, @dx,dy{}, or click an option.",
            if lengths {
                ", @length<angle or a length"
            } else {
                ""
            }
        ));
        Outcome::default()
    }

    /// A point as an operation takes it: a snapped point where the snap
    /// names one the core can find, else its coordinates. In an edit to a
    /// template's object, a snap on a row's own object goes as coordinates:
    /// a template's object is drawn through the same point on every sheet.
    fn point_value(&self, t: &Taken, template_edit: bool) -> Value {
        if let Some(s) = &t.snap {
            if let Some(v) = self.snapped(s, template_edit) {
                return v;
            }
        }
        t.coordinates()
    }

    fn snapped(&self, s: &SnapRef, template_edit: bool) -> Option<Value> {
        if !matches!(
            s.snap,
            SnapKind::Endpoint
                | SnapKind::Midpoint
                | SnapKind::Center
                | SnapKind::Quadrant
                | SnapKind::Insertion
                | SnapKind::Node
                | SnapKind::Intersection
        ) {
            return None;
        }
        let object = |place: usize| -> Option<&ObjectInfo> {
            let o = self.context.objects.get(place)?;
            (!template_edit || o.from_template == Some(true)).then_some(o)
        };
        let mut snap = json!({"id": object(s.object)?.id.as_ref()?, "snap": s.snap});
        if let Some(which) = &s.which {
            snap["which"] = json!(which);
        }
        if let Some(with) = s.with {
            snap["with"] = json!({"id": object(with)?.id.as_ref()?});
        }
        Some(json!({ "snap": snap }))
    }

    /// An add on this sheet, in its row, on the current layer, in the space
    /// the page shows: a layout's paper space on a sheet drawn in a layout.
    fn add(&self, op: &str) -> Value {
        let mut op = json!({"op": op, "sheet": self.context.sheet, "layer": self.layer});
        if let Some(space) = &self.context.space {
            op["space"] = json!(space);
        }
        op
    }

    fn adds(&self, operations: Vec<Value>, what: String) -> Outcome {
        Outcome {
            log: vec![format!("{what}: sent")],
            sends: vec![Send {
                change_set: json!({ "operations": operations }),
                what,
                sheets: vec![self.context.sheet.clone()],
                template: None,
            }],
        }
    }

    fn land_lines(&mut self, points: &[Taken]) -> Outcome {
        let operations: Vec<Value> = points
            .windows(2)
            .map(|pair| {
                let mut op = self.add("add_line");
                op["from"] = self.point_value(&pair[0], false);
                op["to"] = self.point_value(&pair[1], false);
                op
            })
            .collect();
        let n = operations.len();
        self.adds(operations, format!("{} drawn", plural(n, "line", "lines")))
    }

    fn land_circle(&mut self, center: &Taken, radius: f64, how: &str) -> Outcome {
        let mut op = self.add("add_circle");
        op["center"] = self.point_value(center, false);
        op["radius"] = json!(radius);
        self.adds(
            vec![op],
            format!("Circle drawn, radius {}{}{how}", f3(radius), self.unit()),
        )
    }

    fn land_rectangle(&mut self, first: &Taken, second: &Taken) -> Outcome {
        let [x0, y0] = first.at;
        let [x1, y1] = second.at;
        let mut op = self.add("add_polyline");
        op["points"] = json!([
            self.point_value(first, false),
            Taken::typed([x1, y0], first.z).coordinates(),
            self.point_value(second, false),
            Taken::typed([x0, y1], first.z).coordinates(),
        ]);
        op["closed"] = json!(true);
        self.adds(
            vec![op],
            format!(
                "Rectangle drawn, {} × {}{}",
                f3((x1 - x0).abs()),
                f3((y1 - y0).abs()),
                self.unit()
            ),
        )
    }

    fn land_text(&mut self, lines: &[(String, Taken)]) -> Outcome {
        let height = self.text_height();
        let operations: Vec<Value> = lines
            .iter()
            .map(|(text, at)| {
                let mut op = self.add("add_text");
                op["position"] = self.point_value(at, false);
                op["contents"] = json!(text);
                op["height"] = json!(height);
                op
            })
            .collect();
        let what = match lines {
            [(one, _)] => format!("Text “{one}” added"),
            _ => format!("{} lines of text added", lines.len()),
        };
        self.adds(operations, what)
    }

    /// Of the objects at `picked`, those written in the template, and the
    /// template's reach, when there are any.
    fn template_part(&self, picked: &[usize]) -> Option<(Vec<usize>, &Reach)> {
        let reach = self.context.template.as_ref()?;
        let of: Vec<usize> = picked
            .iter()
            .copied()
            .filter(|&p| {
                self.context
                    .objects
                    .get(p)
                    .is_some_and(|o| o.from_template == Some(true))
            })
            .collect();
        (!of.is_empty()).then_some((of, reach))
    }

    /// What an edit by `kind` to the objects at `picked` says when some are
    /// the template's.
    fn template_edit(
        &self,
        kind: ToolKind,
        picked: &[usize],
        copies: usize,
    ) -> Option<TemplateEdit> {
        let (of, reach) = self.template_part(picked)?;
        let n = picked.len();
        let all = of.len() == n;
        let title = if all {
            if n > 1 {
                format!("These {n} objects are part of the template.")
            } else {
                "This is part of the template.".to_owned()
            }
        } else if of.len() == 1 {
            "One of these is part of the template.".to_owned()
        } else {
            format!("{} of these are part of the template.", of.len())
        };
        let them = if all && n > 1 { "them" } else { "it" };
        let sheets = reach.sheets.len();
        let detail = match kind {
            // A copy of a template's object is written in the template.
            ToolKind::Copy if sheets > 1 => format!(
                "This copies {them} on every sheet built from this template: {sheets} sheets."
            ),
            ToolKind::Copy => {
                format!("This copies {them} in the template, which only this sheet is built from.")
            }
            _ => {
                let verb = if kind == ToolKind::Move {
                    "Moving"
                } else {
                    "Erasing"
                };
                if sheets > 1 {
                    format!("{verb} {them} changes {sheets} sheets.")
                } else {
                    format!(
                        "{verb} {them} changes the template, which only this sheet is built from."
                    )
                }
            }
        };
        let edit = match kind {
            ToolKind::Copy => format!("Copy {}", self.copy_words(picked, copies)),
            ToolKind::Move => format!("Move {}", self.picked_words(picked)),
            _ => format!("Erase {}", self.picked_words(picked)),
        };
        Some(TemplateEdit {
            title,
            detail,
            path: reach.path.clone(),
            sheets: reach.sheets.clone(),
            edit,
        })
    }

    /// The sheets an edit to the objects at `picked` changes.
    fn reached(&self, picked: &[usize]) -> Vec<String> {
        match self.template_part(picked) {
            Some((_, reach)) => reach.sheets.clone(),
            None => vec![self.context.sheet.clone()],
        }
    }

    fn target(&self, place: usize) -> Value {
        json!({"id": self.context.objects[place].id})
    }

    fn template_object(&self, place: usize) -> bool {
        self.context
            .objects
            .get(place)
            .is_some_and(|o| o.from_template == Some(true))
    }

    fn land_move(&mut self, picked: &[usize], base: &Taken, to: &Taken) -> Outcome {
        // The core applies one move after another, working a snapped point
        // out on the sheet as it stands then: a snap on an object moved
        // before would be found where it went, and the next object would
        // move by less. So a move of several objects goes by coordinates,
        // the same for each. One object's own move works its `from` and
        // `to` out before it moves (docs/change-sets.md), so its snapped
        // points stay, a snap on itself among them.
        let plain = picked.len() > 1;
        let point = |t: &Taken, template: bool| {
            if plain {
                // Each point at its own height, as its snap resolves, so
                // the group moves as each single move would.
                t.coordinates()
            } else {
                self.point_value(t, template)
            }
        };
        let operations: Vec<Value> = picked
            .iter()
            .map(|&p| {
                let template = self.template_object(p);
                json!({
                    "op": "move",
                    "target": self.target(p),
                    "from": point(base, template),
                    "to": point(to, template),
                })
            })
            .collect();
        let what = format!("Moved {}", self.picked_words(picked));
        Outcome {
            log: Vec::new(),
            sends: vec![Send {
                change_set: json!({ "operations": operations }),
                what,
                sheets: self.reached(picked),
                template: self.template_edit(ToolKind::Move, picked, 0),
            }],
        }
    }

    fn land_erase(&mut self, picked: &[usize]) -> Outcome {
        let operations: Vec<Value> = picked
            .iter()
            .map(|&p| json!({"op": "delete", "target": self.target(p)}))
            .collect();
        let what = format!("Erased {}", self.picked_words(picked));
        Outcome {
            log: Vec::new(),
            sends: vec![Send {
                change_set: json!({ "operations": operations }),
                what,
                sheets: self.reached(picked),
                template: self.template_edit(ToolKind::Erase, picked, 0),
            }],
        }
    }

    /// The objects a copy copies, and how many copies when more than one:
    /// `FT-101, 2 copies`.
    fn copy_words(&self, picked: &[usize], copies: usize) -> String {
        let objects = self.picked_words(picked);
        if copies > 1 {
            format!("{objects}, {copies} copies")
        } else {
            objects
        }
    }

    /// The copies Copy placed, as one change set: one `copy` operation.
    /// One copy goes `from` the base point `to` the second, each a snapped
    /// point where it was taken on one, as Move sends one object's; several
    /// go as `offsets`, each how far its point is from the base point, as
    /// AutoCAD's COPY places several. The core copies each object to each
    /// place, writing a template's object's copies in the template.
    fn land_copy(&mut self, picked: &[usize], base: &Taken, placed: &[Taken]) -> Outcome {
        let template = picked.iter().any(|&p| self.template_object(p));
        let mut op = json!({"op": "copy"});
        match picked {
            [one] => op["target"] = self.target(*one),
            _ => {
                op["targets"] = Value::Array(picked.iter().map(|&p| self.target(p)).collect());
            }
        }
        match placed {
            [to] => {
                op["from"] = self.point_value(base, template);
                op["to"] = self.point_value(to, template);
            }
            _ => {
                op["offsets"] = Value::Array(
                    placed
                        .iter()
                        .map(|t| {
                            let [dx, dy] = offset(base, t);
                            let dz = clean(t.z - base.z);
                            if dz == 0.0 {
                                json!([dx, dy, 0])
                            } else {
                                json!([dx, dy, dz])
                            }
                        })
                        .collect(),
                );
            }
        }
        let what = format!("Copied {}", self.copy_words(picked, placed.len()));
        Outcome {
            log: Vec::new(),
            sends: vec![Send {
                change_set: json!({ "operations": [op] }),
                what,
                sheets: self.reached(picked),
                template: self.template_edit(ToolKind::Copy, picked, placed.len()),
            }],
        }
    }

    /// The tag over each copy Copy has placed, and how far that copy is
    /// from its objects: `copy 1 · waits for Enter`, or on a phone `copy 1`.
    pub fn placed_tags(&self, touch: bool) -> Vec<(String, [f64; 2])> {
        let Some(Running {
            step: Step::CopySecond { base, placed, .. },
            ..
        }) = &self.running
        else {
            return Vec::new();
        };
        placed
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let n = i + 1;
                let words = if touch {
                    format!("copy {n}")
                } else {
                    format!("copy {n} · waits for Enter")
                };
                (words, offset(base, t))
            })
            .collect()
    }

    /// What the command bar shows: `touch` says a phone's words, a tap
    /// where a mouse clicks.
    pub fn state(&self, touch: bool) -> State {
        let mut state = State {
            error: self.error.clone(),
            placeholder: if touch {
                "L, C, REC, T, M, CO, E".to_owned()
            } else {
                String::new()
            },
            ..State::default()
        };
        let Some(running) = &self.running else {
            return state;
        };
        let kind = running.kind;
        let click = if touch { "Tap" } else { "Click" };
        let hold = if touch { ", or hold to snap" } else { "" };
        let choice = |label: &str, act: &'static str, main: bool| Choice {
            label: label.to_owned(),
            act,
            main,
        };
        let cancel = choice("Cancel", "cancel", false);
        state.tool = Some(kind);
        state.command = Some(kind.command());
        state.layer = kind.draws().then(|| self.layer.clone());
        state.takes_points = self.takes_points();
        state.selecting = self.selecting();
        state.placeholder = "x,y  @dx,dy  length".to_owned();
        let (prompt, options): (String, Vec<Choice>) = match &running.step {
            Step::LineFirst => (
                format!("{click} the first point{hold}, or type x,y"),
                vec![cancel],
            ),
            Step::LineNext { points } => {
                let mut options = vec![choice("Undo", "undo", false)];
                if points.len() >= 3 {
                    options.push(choice("Close", "close", false));
                }
                options.push(choice("Done", "done", true));
                (
                    format!("{click} the next point, or type @dx,dy or a length"),
                    options,
                )
            }
            Step::CircleCenter => (format!("{click} the centre{hold}"), vec![cancel]),
            Step::CircleRadius { diameter, .. } => (
                if *diameter {
                    format!(
                        "Type the diameter, or {} a point on the circle",
                        click.to_lowercase()
                    )
                } else {
                    format!("{click} a point on the circle, or type the radius")
                },
                vec![
                    if *diameter {
                        choice("Radius", "radius", false)
                    } else {
                        choice("Diameter", "diameter", false)
                    },
                    cancel,
                ],
            ),
            Step::RectangleFirst => (format!("{click} the first corner{hold}"), vec![cancel]),
            Step::RectangleSecond { .. } => (
                format!("{click} the opposite corner, or type its size as @width,height"),
                vec![cancel],
            ),
            Step::TextStart => (
                format!("{click} where the text starts{hold}"),
                vec![
                    choice(&format!("Height {}", self.text_height()), "height", false),
                    cancel,
                ],
            ),
            Step::TextHeight => {
                state.placeholder = format!("{}", self.text_height());
                (
                    format!(
                        "Type the text’s height in {} (now {})",
                        self.unit_name(),
                        self.text_height()
                    ),
                    vec![cancel],
                )
            }
            Step::TextType { .. } => {
                state.typing_text = true;
                state.placeholder = "the text".to_owned();
                (
                    if touch {
                        "Type the text, then Add".to_owned()
                    } else {
                        "Type the text: it shows on the drawing. Enter starts a new line, Enter \
                         again finishes"
                            .to_owned()
                    },
                    vec![choice("Done", "done", true), cancel],
                )
            }
            Step::Select { picked } => {
                state.placeholder = String::new();
                let n = picked.len();
                let verb = match kind {
                    ToolKind::Erase => "erase",
                    ToolKind::Copy => "copy",
                    _ => "move",
                };
                let enter = if touch { "" } else { " Enter" };
                if n == 0 {
                    (
                        format!("{click} the objects to {verb}, then{enter}"),
                        vec![cancel],
                    )
                } else {
                    let done = if kind == ToolKind::Erase {
                        format!("Erase {n}")
                    } else {
                        "Done".to_owned()
                    };
                    (
                        format!(
                            "{} picked. {click} more, or{enter}",
                            plural(n, "object", "objects")
                        ),
                        vec![choice(&done, "done", true), cancel],
                    )
                }
            }
            Step::MoveBase { picked } => (
                format!(
                    "{click} a point to move {} by: the base point{hold}",
                    if picked.len() > 1 { "them" } else { "it" }
                ),
                vec![cancel],
            ),
            Step::MoveSecond { .. } => (
                format!("{click} where that point goes, or type @dx,dy or a length"),
                vec![cancel],
            ),
            Step::CopyBase { picked } => (
                format!(
                    "{click} a point to copy {} from: the base point{hold}",
                    if picked.len() > 1 { "them" } else { "it" }
                ),
                vec![cancel],
            ),
            Step::CopySecond { placed, .. } => {
                let k = placed.len();
                if k == 0 {
                    (
                        format!("{click} where the copy goes, or type @dx,dy or a length"),
                        vec![choice("Done", "done", true)],
                    )
                } else {
                    (
                        format!(
                            "{} placed, dashed until you finish. {click} where the next goes, or \
                             type @dx,dy or a length",
                            plural(k, "copy", "copies")
                        ),
                        vec![
                            choice("Undo", "undo", false),
                            choice(&format!("Done: copy {k}"), "done", true),
                        ],
                    )
                }
            }
        };
        state.prompt = prompt;
        state.options = options;
        if let Step::Select { picked }
        | Step::MoveBase { picked }
        | Step::MoveSecond { picked, .. }
        | Step::CopyBase { picked }
        | Step::CopySecond { picked, .. } = &running.step
        {
            let copies = match &running.step {
                Step::CopySecond { placed, .. } => placed.len(),
                _ => 0,
            };
            state.picked = picked.clone();
            state.warning = self.template_edit(kind, picked, copies).map(|mut w| {
                w.detail.push_str(" You will be asked before it lands.");
                w
            });
        }
        state.points = match &running.step {
            Step::LineNext { points } => points.iter().map(|p| p.at).collect(),
            Step::CircleRadius { center, .. } => vec![center.at],
            Step::RectangleSecond { first } => vec![first.at],
            Step::TextType { at, .. } => vec![at.at],
            Step::MoveSecond { base, .. } | Step::CopySecond { base, .. } => vec![base.at],
            _ => Vec::new(),
        };
        if let Step::CopySecond { placed, .. } = &running.step {
            state.placed = placed.len();
        }
        if let Step::TextType { at, lines, current } = &running.step {
            let mut shown: Vec<TextLine> = lines
                .iter()
                .map(|(text, t)| TextLine {
                    text: text.clone(),
                    at: t.at,
                })
                .collect();
            shown.push(TextLine {
                text: current.clone(),
                at: at.at,
            });
            state.text = Some(Typing {
                height: self.text_height(),
                lines: shown,
            });
        }
        state
    }

    /// What the tool shows at `aim`, the point the next click would take.
    pub fn band(&self, aim: Option<&Taken>) -> Band {
        let Some(running) = &self.running else {
            return Band::default();
        };
        let mut band = Band {
            points: self.state(false).points,
            ..Band::default()
        };
        if let Step::LineNext { points } = &running.step {
            for pair in points.windows(2) {
                band.lines.push([pair[0].at, pair[1].at]);
            }
        }
        if let Step::CopySecond {
            picked,
            base,
            placed,
        } = &running.step
        {
            if !placed.is_empty() {
                let offsets = placed.iter().map(|t| offset(base, t)).collect();
                band.placed = Some((picked.clone(), offsets));
            }
        }
        let (Some(aim), Some(base)) = (aim, self.base()) else {
            return band;
        };
        let to = aim.at;
        match &running.step {
            Step::LineNext { .. } => band.lines.push([base, to]),
            Step::CircleRadius { .. } => {
                let r = (to[0] - base[0]).hypot(to[1] - base[1]);
                if r > 0.0 {
                    let at = |i: usize| {
                        let a = i as f64 / CIRCLE_PIECES as f64 * std::f64::consts::TAU;
                        [base[0] + r * a.cos(), base[1] + r * a.sin()]
                    };
                    band.lines
                        .extend((0..CIRCLE_PIECES).map(|i| [at(i), at(i + 1)]));
                }
                band.guides.push([base, to]);
            }
            Step::RectangleSecond { .. } => {
                let corners = [base, [to[0], base[1]], to, [base[0], to[1]]];
                for i in 0..4 {
                    band.lines.push([corners[i], corners[(i + 1) % 4]]);
                }
            }
            Step::MoveSecond { picked, .. } => {
                band.moving = Some((picked.clone(), [to[0] - base[0], to[1] - base[1]]));
                band.guides.push([base, to]);
            }
            Step::CopySecond { picked, .. } => {
                band.copying = Some((picked.clone(), [to[0] - base[0], to[1] - base[1]]));
                band.guides.push([base, to]);
            }
            _ => {}
        }
        band
    }

    /// What shows at the cursor, as AutoCAD's dynamic input shows it: the
    /// length and angle from the last point, a circle's radius, or a
    /// rectangle's size; none when the tool has no point to measure from.
    pub fn cursor_words(&self, aim: &Taken) -> Option<(String, Option<String>)> {
        let running = self.running.as_ref()?;
        let base = self.base()?;
        let to = aim.at;
        Some(match running.step {
            Step::CircleRadius { diameter, .. } => {
                let r = (to[0] - base[0]).hypot(to[1] - base[1]);
                if diameter {
                    (format!("D {}{}", f3(2.0 * r), self.unit()), None)
                } else {
                    (format!("R {}{}", f3(r), self.unit()), None)
                }
            }
            Step::RectangleSecond { .. } => (
                format!(
                    "{} × {}{}",
                    f3((to[0] - base[0]).abs()),
                    f3((to[1] - base[1]).abs()),
                    self.unit()
                ),
                None,
            ),
            _ => {
                let (length, angle) = length_angle(base, to);
                (
                    format!("{}{}", f3(length), self.unit()),
                    Some(format!("{angle:.1}°")),
                )
            }
        })
    }
}

/// A snap's name, as the tag and the history give it.
pub fn snap_label(kind: SnapKind) -> &'static str {
    match kind {
        SnapKind::Endpoint => "Endpoint",
        SnapKind::Midpoint => "Midpoint",
        SnapKind::Center => "Centre",
        SnapKind::Quadrant => "Quadrant",
        SnapKind::Insertion => "Insertion",
        SnapKind::Node => "Node",
        SnapKind::Intersection => "Intersection",
        SnapKind::ApparentIntersection => "Apparent intersection",
        SnapKind::Perpendicular => "Perpendicular",
        SnapKind::Tangent => "Tangent",
        SnapKind::Nearest => "Nearest",
    }
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
