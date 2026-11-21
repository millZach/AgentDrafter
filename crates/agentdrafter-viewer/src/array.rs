//! The Array tool's values (#66): rows and columns, or items round a
//! centre, as its panel and the command bar set them, and where each copy
//! goes. Plain computation, tested natively through [`crate::tools`].
//!
//! The core's `array` operation takes the spacing between items, and a
//! polar array's angle to fill (docs/change-sets.md, Copy and array). The
//! drafter may give either the spacing between items or the total, first
//! to last, divided evenly; whichever is held, the other follows, worked
//! out here, so the page holds no drawing logic (ADR-0007). A value typed
//! is kept as typed; one worked out is cleared of the noise dividing
//! leaves, as Copy's offsets are.

use serde::Serialize;
use serde_json::{json, Map, Value};

/// The kinds the tool makes. A path array is the core's, for an agent;
/// `ARRAYPATH` goes to the assistant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArrayKind {
    Rectangular,
    Polar,
}

impl ArrayKind {
    /// AutoCAD's command for it.
    pub fn command(self) -> &'static str {
        match self {
            ArrayKind::Rectangular => "ARRAYRECT",
            ArrayKind::Polar => "ARRAYPOLAR",
        }
    }
}

/// The kind a word starts the tool as: `ARRAYRECT` and `ARRAYPOLAR`, in
/// any case; none for `AR`, `ARRAY` or anything else.
pub fn kind_for(word: &str) -> Option<ArrayKind> {
    let word = word.trim();
    if word.eq_ignore_ascii_case("ARRAYRECT") {
        Some(ArrayKind::Rectangular)
    } else if word.eq_ignore_ascii_case("ARRAYPOLAR") {
        Some(ArrayKind::Polar)
    } else {
        None
    }
}

/// How the spacing is given: between items, or the total, first to last,
/// divided evenly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Spacing {
    Between,
    Total,
}

/// Where one copy goes: turned `turn` degrees anticlockwise about
/// `about`, then moved by `by`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub turn: f64,
    pub about: [f64; 2],
    pub by: [f64; 2],
}

impl Place {
    fn shift(by: [f64; 2]) -> Self {
        Place {
            turn: 0.0,
            about: [0.0, 0.0],
            by,
        }
    }

    /// Where the point `p` of the objects goes in this copy.
    pub fn apply(&self, p: [f64; 2]) -> [f64; 2] {
        let [x, y] = if self.turn == 0.0 {
            p
        } else {
            let (s, c) = self.turn.to_radians().sin_cos();
            let [dx, dy] = [p[0] - self.about[0], p[1] - self.about[1]];
            [
                self.about[0] + dx * c - dy * s,
                self.about[1] + dx * s + dy * c,
            ]
        };
        [x + self.by[0], y + self.by[1]]
    }

    /// The box `[left, bottom, right, top]` holding `b` as this copy puts it.
    pub fn bounds(&self, b: [f64; 4]) -> [f64; 4] {
        let corners =
            [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]].map(|p| self.apply(p));
        corners.iter().fold(
            [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            |a, p| {
                [
                    a[0].min(p[0]),
                    a[1].min(p[1]),
                    a[2].max(p[0]),
                    a[3].max(p[1]),
                ]
            },
        )
    }
}

/// The most copies one array makes, as the core takes it.
pub const MOST_COPIES: f64 = 10_000.0;

/// A value cleared of the noise dividing and multiplying leave, such as
/// 0.39999999999999997 for 0.4, as Copy's offsets are.
fn clean(v: f64) -> f64 {
    let r = (v * 1e10).round() / 1e10;
    if r == 0.0 {
        0.0
    } else {
        r
    }
}

/// Whether an angle to fill is a whole turn, either way round.
pub fn whole_turn(fill: f64) -> bool {
    (fill.abs() - 360.0).abs() < 1e-6
}

/// A length in words, to a thousandth.
fn f3(v: f64) -> String {
    format!("{:.3}", if v.abs() < 5e-4 { 0.0 } else { v })
}

/// An angle in words, to a tenth of a degree, with no trailing zero.
pub fn f1(v: f64) -> String {
    let r = (v * 10.0).round() / 10.0;
    let r = if r == 0.0 { 0.0 } else { r };
    format!("{r}")
}

/// A value as a field shows it: as written when it has six decimals at
/// most, such as `0.12345`; else to four, such as `0.6667` for two thirds.
pub fn shown(v: f64) -> String {
    let v = if v == 0.0 { 0.0 } else { v };
    let plain = format!("{v}");
    match plain.split_once('.') {
        Some((_, decimals)) if decimals.len() > 6 => {
            let s = format!("{v:.4}");
            let s = s.trim_end_matches('0').trim_end_matches('.');
            if s == "-0" {
                "0".to_owned()
            } else {
                s.to_owned()
            }
        }
        _ => plain,
    }
}

fn plural(n: f64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1.0 { one } else { many })
}

/// The values of an array being set up.
#[derive(Clone, Debug, PartialEq)]
pub struct Setup {
    pub kind: ArrayKind,
    pub spacing: Spacing,
    pub rows: f64,
    pub columns: f64,
    /// How far up each row is from the one before, negative down.
    pub row_spacing: f64,
    /// How far across each column is from the one before, negative left.
    pub column_spacing: f64,
    /// From the first row to the last.
    pub row_total: f64,
    /// From the first column to the last.
    pub column_total: f64,
    /// The items round the centre, the objects the first.
    pub items: f64,
    /// Degrees from one item to the next, anticlockwise.
    pub angle_between: f64,
    /// The angle they fill, as the core takes it: 360 a whole turn.
    pub angle_fill: f64,
    /// Whether each copy turns as it goes round.
    pub rotate: bool,
}

/// A field of the panel, as the page shows it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Field {
    /// What the page passes back when it is typed in.
    pub key: &'static str,
    pub label: String,
    /// What the sign means, under it.
    pub hint: Option<&'static str>,
    /// Its value, as it shows.
    pub value: String,
    /// Whether it takes a whole number, for a phone's keyboard.
    pub whole: bool,
}

impl Setup {
    /// An array of `kind` of objects whose box is `b`: three rows a box
    /// and a little more apart downwards, rounded up to a quarter of the
    /// unit, and six items round a whole turn, as the prototype starts.
    pub fn new(kind: ArrayKind, b: Option<[f64; 4]>) -> Self {
        let step = |v: f64| clean(((v + 0.2) / 0.25).ceil() * 0.25);
        let (w, h) = b.map_or((0.8, 0.8), |b| (b[2] - b[0], b[3] - b[1]));
        let row_spacing = -step(h);
        let column_spacing = step(w);
        Setup {
            kind,
            spacing: Spacing::Between,
            rows: 3.0,
            columns: 1.0,
            row_spacing,
            column_spacing,
            row_total: clean(row_spacing * 2.0),
            column_total: clean(column_spacing * 2.0),
            items: 6.0,
            angle_between: 60.0,
            angle_fill: 360.0,
            rotate: true,
        }
    }

    /// The angle a polar array fills, from the angle between its items: a
    /// whole turn when they go right round, else to the last item.
    fn fill_of(between: f64, items: f64) -> f64 {
        if whole_turn(between * items) {
            360f64.copysign(between)
        } else {
            clean(between * (items - 1.0))
        }
    }

    /// The angle between a polar array's items, from the angle it fills.
    fn between_of(fill: f64, items: f64) -> f64 {
        if whole_turn(fill) {
            clean(fill / items)
        } else {
            clean(fill / (items - 1.0))
        }
    }

    /// Works out the values not held from those that are.
    fn sync(&mut self) {
        match self.spacing {
            Spacing::Between => {
                if self.rows > 1.0 {
                    self.row_total = clean(self.row_spacing * (self.rows - 1.0));
                }
                if self.columns > 1.0 {
                    self.column_total = clean(self.column_spacing * (self.columns - 1.0));
                }
                if self.items >= 2.0 {
                    self.angle_fill = Self::fill_of(self.angle_between, self.items);
                }
            }
            Spacing::Total => {
                if self.rows > 1.0 {
                    self.row_spacing = clean(self.row_total / (self.rows - 1.0));
                }
                if self.columns > 1.0 {
                    self.column_spacing = clean(self.column_total / (self.columns - 1.0));
                }
                if self.items >= 2.0 {
                    self.angle_between = Self::between_of(self.angle_fill, self.items);
                }
            }
        }
    }

    /// Holds the spacing between items, or the total.
    pub fn hold(&mut self, spacing: Spacing) {
        self.spacing = spacing;
        self.sync();
    }

    /// Sets the value `key` names; false when it names none. The items of
    /// a polar array that goes a whole turn still go a whole turn when
    /// their number changes, as the prototype keeps them.
    pub fn set(&mut self, key: &str, v: f64) -> bool {
        match key {
            "rows" => self.rows = v,
            "columns" => self.columns = v,
            "row_spacing" => self.row_spacing = v,
            "column_spacing" => self.column_spacing = v,
            "row_total" => self.row_total = v,
            "column_total" => self.column_total = v,
            "items" => {
                if self.spacing == Spacing::Between && whole_turn(self.angle_fill) && v >= 2.0 {
                    self.angle_between = clean(self.angle_fill / v);
                }
                self.items = v;
            }
            "angle_between" => self.angle_between = v,
            "angle_fill" => self.angle_fill = v,
            _ => return false,
        }
        self.sync();
        true
    }

    /// How many items an object becomes, itself among them.
    fn items_each(&self) -> f64 {
        match self.kind {
            ArrayKind::Rectangular => self.rows * self.columns,
            ArrayKind::Polar => self.items,
        }
    }

    /// How many copies it makes of `objects` objects.
    pub fn copies(&self, objects: usize) -> usize {
        ((self.items_each() - 1.0).max(0.0) * objects as f64) as usize
    }

    /// What is wrong with the values, in words, if anything: a polar
    /// array's centre aside.
    pub fn problem(&self) -> Option<String> {
        let whole = |v: f64| v.fract() == 0.0 && v.is_finite();
        let total = self.spacing == Spacing::Total;
        match self.kind {
            ArrayKind::Rectangular => {
                if !(whole(self.rows)
                    && whole(self.columns)
                    && self.rows >= 1.0
                    && self.columns >= 1.0)
                {
                    return Some("Rows and columns are whole numbers, 1 or more.".to_owned());
                }
                if self.rows * self.columns < 2.0 {
                    return Some(
                        "One row and one column make no copy: give 2 or more of one.".to_owned(),
                    );
                }
                if self.rows > 1.0 && self.row_spacing == 0.0 {
                    return Some(
                        if total {
                            "The rows need a distance, first to last, other than 0."
                        } else {
                            "Rows need a spacing other than 0."
                        }
                        .to_owned(),
                    );
                }
                if self.columns > 1.0 && self.column_spacing == 0.0 {
                    return Some(
                        if total {
                            "The columns need a distance, first to last, other than 0."
                        } else {
                            "Columns need a spacing other than 0."
                        }
                        .to_owned(),
                    );
                }
            }
            ArrayKind::Polar => {
                if !whole(self.items) || self.items < 2.0 {
                    return Some(
                        "Items are a whole number, 2 or more: the objects are the first."
                            .to_owned(),
                    );
                }
                if total {
                    if self.angle_fill == 0.0 || self.angle_fill.abs() > 360.0 {
                        return Some(
                            "The angle to fill is more than 0 and at most 360, either way."
                                .to_owned(),
                        );
                    }
                } else {
                    if self.angle_between == 0.0 {
                        return Some("The angle between items is other than 0.".to_owned());
                    }
                    if self.angle_fill.abs() > 360.0 + 1e-9 {
                        return Some(format!(
                            "{} items {}° apart fill {}°: more than a whole turn.",
                            self.items,
                            f1(self.angle_between.abs()),
                            f1(self.angle_fill.abs())
                        ));
                    }
                }
            }
        }
        if self.items_each() - 1.0 > MOST_COPIES {
            return Some("One array makes at most 10,000 copies of each object.".to_owned());
        }
        None
    }

    /// Its short name, for the tag over it and the journal: `3 × 2`,
    /// `8 round`, or `5 round, -180°` when it fills less than a turn.
    pub fn tag(&self) -> String {
        match self.kind {
            ArrayKind::Rectangular => format!("{} × {}", self.rows, self.columns),
            ArrayKind::Polar if whole_turn(self.angle_fill) => format!("{} round", self.items),
            ArrayKind::Polar => format!("{} round, {}°", self.items, f1(self.angle_fill)),
        }
    }

    /// It in words, `unit` the sheet's unit with a space before it:
    /// `3 rows × 2 columns, rows 0.500 in apart down, columns 1.000 in
    /// apart right: 5 copies`.
    pub fn words(&self, unit: &str, objects: usize) -> String {
        let copies = self.copies(objects);
        let copies = format!("{copies} {}", if copies == 1 { "copy" } else { "copies" });
        match self.kind {
            ArrayKind::Rectangular => {
                let mut w = format!(
                    "{} × {}",
                    plural(self.rows, "row", "rows"),
                    plural(self.columns, "column", "columns")
                );
                if self.rows > 1.0 {
                    let way = if self.row_spacing < 0.0 { "down" } else { "up" };
                    w.push_str(&format!(
                        ", rows {}{unit} apart {way}",
                        f3(self.row_spacing.abs())
                    ));
                }
                if self.columns > 1.0 {
                    let way = if self.column_spacing < 0.0 {
                        "left"
                    } else {
                        "right"
                    };
                    w.push_str(&format!(
                        ", columns {}{unit} apart {way}",
                        f3(self.column_spacing.abs())
                    ));
                }
                format!("{w}: {copies}")
            }
            ArrayKind::Polar => {
                let fill = if whole_turn(self.angle_fill) {
                    "a whole turn".to_owned()
                } else {
                    let way = if self.angle_fill < 0.0 {
                        "clockwise"
                    } else {
                        "anticlockwise"
                    };
                    format!("{}° {way}", f1(self.angle_fill.abs()))
                };
                format!(
                    "{} items round the centre, {fill}, {}° apart{}: {copies}",
                    self.items,
                    f1(self.angle_between.abs()),
                    if self.rotate { "" } else { ", not turned" }
                )
            }
        }
    }

    /// The panel's fields, `unit` the sheet's unit, such as `in`.
    pub fn fields(&self, unit: Option<&str>) -> Vec<Field> {
        let unit = unit.map(|u| format!(", {u}")).unwrap_or_default();
        let field =
            |key: &'static str, label: String, hint: Option<&'static str>, v: f64, whole: bool| {
                Field {
                    key,
                    label,
                    hint,
                    value: shown(v),
                    whole,
                }
            };
        let total = self.spacing == Spacing::Total;
        match self.kind {
            ArrayKind::Rectangular => vec![
                field("rows", "Rows".to_owned(), None, self.rows, true),
                field("columns", "Columns".to_owned(), None, self.columns, true),
                if total {
                    field(
                        "row_total",
                        format!("Rows, first to last{unit}"),
                        Some("+ up, − down"),
                        self.row_total,
                        false,
                    )
                } else {
                    field(
                        "row_spacing",
                        format!("Row spacing{unit}"),
                        Some("+ up, − down"),
                        self.row_spacing,
                        false,
                    )
                },
                if total {
                    field(
                        "column_total",
                        format!("Columns, first to last{unit}"),
                        Some("+ right, − left"),
                        self.column_total,
                        false,
                    )
                } else {
                    field(
                        "column_spacing",
                        format!("Column spacing{unit}"),
                        Some("+ right, − left"),
                        self.column_spacing,
                        false,
                    )
                },
            ],
            ArrayKind::Polar => vec![
                field(
                    "items",
                    "Items".to_owned(),
                    Some("the objects are the first"),
                    self.items,
                    true,
                ),
                if total {
                    field(
                        "angle_fill",
                        "Angle to fill, °".to_owned(),
                        Some("360 a whole turn, − clockwise"),
                        self.angle_fill,
                        false,
                    )
                } else {
                    field(
                        "angle_between",
                        "Angle between, °".to_owned(),
                        Some("+ anticlockwise, − clockwise"),
                        self.angle_between,
                        false,
                    )
                },
            ],
        }
    }

    /// Where each copy goes, the objects themselves left out: their box's
    /// centre is `middle`, which goes round when a polar array's copies are
    /// not turned, as the core's `base_point` left out does; `center` is a
    /// polar array's centre. None while the values are wrong, or a polar
    /// array has no centre.
    pub fn places(&self, middle: [f64; 2], center: Option<[f64; 2]>) -> Vec<Place> {
        if self.problem().is_some() {
            return Vec::new();
        }
        match self.kind {
            ArrayKind::Rectangular => {
                let mut out = Vec::new();
                for r in 0..self.rows as usize {
                    for c in 0..self.columns as usize {
                        if r + c > 0 {
                            out.push(Place::shift([
                                c as f64 * self.column_spacing,
                                r as f64 * self.row_spacing,
                            ]));
                        }
                    }
                }
                out
            }
            ArrayKind::Polar => {
                let Some(center) = center else {
                    return Vec::new();
                };
                (1..self.items as usize)
                    .map(|k| {
                        let turn = Place {
                            turn: self.angle_between * k as f64,
                            about: center,
                            by: [0.0, 0.0],
                        };
                        if self.rotate {
                            turn
                        } else {
                            let to = turn.apply(middle);
                            Place::shift([to[0] - middle[0], to[1] - middle[1]])
                        }
                    })
                    .collect()
            }
        }
    }

    /// The keys of the core's `array` operation, `center` a polar array's
    /// centre as the operation takes it: the spacing between items, worked
    /// out from a total held, and the angle it fills, left out for a whole
    /// turn; `rotate` only when false, as the core turns them by default.
    pub fn keys(&self, center: Value) -> Map<String, Value> {
        let mut k = Map::new();
        match self.kind {
            ArrayKind::Rectangular => {
                k.insert("kind".into(), json!("rectangular"));
                k.insert("rows".into(), json!(self.rows as u64));
                k.insert("columns".into(), json!(self.columns as u64));
                if self.rows > 1.0 {
                    k.insert("row_spacing".into(), json!(self.row_spacing));
                }
                if self.columns > 1.0 {
                    k.insert("column_spacing".into(), json!(self.column_spacing));
                }
            }
            ArrayKind::Polar => {
                k.insert("kind".into(), json!("polar"));
                k.insert("center".into(), center);
                k.insert("count".into(), json!(self.items as u64));
                if !whole_turn(self.angle_fill) {
                    k.insert("angle".into(), json!(self.angle_fill));
                }
                if !self.rotate {
                    k.insert("rotate".into(), json!(false));
                }
            }
        }
        k
    }
}
