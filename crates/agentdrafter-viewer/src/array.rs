//! The Array tool's values (#66, #256): rows and columns, items round a
//! centre, or items along a path curve, as its panel and the command bar
//! set them, and where each copy goes. Plain computation, tested natively
//! through [`crate::tools`].
//!
//! The core's `array` operation takes the spacing between items, a polar
//! array's angle to fill, or a path array's curve and optional spacing
//! (docs/change-sets.md, Copy and array). The drafter may give either the
//! spacing between items or the total, first to last, divided evenly;
//! whichever is held, the other follows, worked out here, so the page
//! holds no drawing logic (ADR-0007). A value typed is kept as typed; one
//! worked out is cleared of the noise dividing leaves, as Copy's offsets
//! are.

use serde::Serialize;
use serde_json::{json, Map, Value};

/// The kinds the tool makes: rectangular, polar and path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArrayKind {
    Rectangular,
    Polar,
    Path,
}

impl ArrayKind {
    /// AutoCAD's command for it.
    pub fn command(self) -> &'static str {
        match self {
            ArrayKind::Rectangular => "ARRAYRECT",
            ArrayKind::Polar => "ARRAYPOLAR",
            ArrayKind::Path => "ARRAYPATH",
        }
    }
}

/// The kind a word starts the tool as: `ARRAYRECT`, `ARRAYPOLAR` and
/// `ARRAYPATH`, in any case; none for `AR`, `ARRAY` or anything else.
pub fn kind_for(word: &str) -> Option<ArrayKind> {
    let word = word.trim();
    if word.eq_ignore_ascii_case("ARRAYRECT") {
        Some(ArrayKind::Rectangular)
    } else if word.eq_ignore_ascii_case("ARRAYPOLAR") {
        Some(ArrayKind::Polar)
    } else if word.eq_ignore_ascii_case("ARRAYPATH") {
        Some(ArrayKind::Path)
    } else {
        None
    }
}

/// A path curve sampled from an object's display-list strokes, used to
/// preview a path array (#256) before the core resolves the exact curve.
#[derive(Clone, Debug, PartialEq)]
pub struct PathCurve {
    pub points: Vec<[f64; 2]>,
    pub closed: bool,
}

impl PathCurve {
    /// Whether an object of `kind` can serve as a path array's curve
    /// (docs/change-sets.md, Copy and array).
    pub fn is_path_kind(kind: &str) -> bool {
        matches!(
            kind,
            "line" | "polyline" | "arc" | "circle" | "ellipse" | "spline"
        )
    }

    /// Builds a path curve from an object's stroke runs in order. Returns
    /// `None` when the kind is not a path curve or has no length.
    pub fn from_runs(kind: &str, runs: &[&[[f64; 2]]]) -> Option<Self> {
        if !Self::is_path_kind(kind) {
            return None;
        }
        let mut points: Vec<[f64; 2]> = Vec::new();
        for run in runs {
            for &p in *run {
                if !p[0].is_finite() || !p[1].is_finite() {
                    continue;
                }
                if points
                    .last()
                    .is_some_and(|last| (p[0] - last[0]).hypot(p[1] - last[1]) <= 1e-12)
                {
                    continue;
                }
                points.push(p);
            }
        }
        if points.len() < 2 {
            return None;
        }
        // A dashed circle's stroke ends slightly before its start: close it so
        // its circumference matches the core's circle length.
        if kind == "circle" {
            let first = points[0];
            let last = *points.last().unwrap();
            if (first[0] - last[0]).hypot(first[1] - last[1]) > 1e-12 {
                points.push(first);
            }
        }
        let length: f64 = points
            .windows(2)
            .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
            .sum();
        if length <= 0.0 || !length.is_finite() {
            return None;
        }
        let closed = match kind {
            "circle" => true,
            "line" | "arc" => false,
            _ => {
                points.len() >= 3
                    && (points[0][0] - points.last().unwrap()[0])
                        .hypot(points[0][1] - points.last().unwrap()[1])
                        <= length * 1e-6
            }
        };
        Some(PathCurve { points, closed })
    }

    /// The total arc length along the curve's segments.
    pub fn length(&self) -> f64 {
        self.points
            .windows(2)
            .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
            .sum()
    }

    /// Places `count - 1` copies along the curve (the objects themselves are
    /// item 0 at the curve's start), spaced by `spacing` or evenly along the
    /// curve (around it when `closed`). Aligned copies turn with the curve's
    /// tangent difference from its start, matching the core's `array::along`.
    pub fn along(
        &self,
        count: usize,
        spacing: Option<f64>,
        align: bool,
    ) -> Result<Vec<Place>, String> {
        if count < 2 {
            return Ok(Vec::new());
        }
        let mut starts: Vec<(f64, f64)> = Vec::with_capacity(self.points.len().saturating_sub(1));
        let mut total = 0.0;
        for w in self.points.windows(2) {
            let len = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
            starts.push((total, len));
            total += len;
        }
        if total <= 0.0 || !total.is_finite() {
            return Err("The path has no length to run along.".to_owned());
        }
        let step = match spacing {
            Some(s) => {
                if s <= 0.0 || !s.is_finite() {
                    return Err("The spacing along the path is more than 0.".to_owned());
                }
                let run = s * (count - 1) as f64;
                if run > total * (1.0 + 1e-9) {
                    return Err(format!(
                        "{count} items {} apart run {} along the path, which is {} long.",
                        f3(s),
                        f3(run),
                        f3(total)
                    ));
                }
                s
            }
            None if self.closed => total / count as f64,
            None => total / (count - 1) as f64,
        };
        let n = starts.len();
        let at = |distance: f64| -> ([f64; 2], f64) {
            let j = if distance <= 0.0 {
                (0..n).find(|&j| starts[j].1 > 0.0).unwrap_or(0)
            } else {
                (0..n)
                    .find(|&j| {
                        let (seg_start, seg_len) = starts[j];
                        if seg_len <= 0.0 {
                            return false;
                        }
                        let near = (total * 1e-9).min(seg_len * 1e-6);
                        distance < seg_start + seg_len - near
                    })
                    .or_else(|| (0..n).rev().find(|&j| starts[j].1 > 0.0))
                    .unwrap_or(0)
            };
            let (seg_start, seg_len) = starts[j];
            let p0 = self.points[j];
            let p1 = self.points[j + 1];
            let t = if seg_len > 0.0 {
                ((distance - seg_start) / seg_len).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let pt = [p0[0] + (p1[0] - p0[0]) * t, p0[1] + (p1[1] - p0[1]) * t];
            let dir = (p1[1] - p0[1]).atan2(p1[0] - p0[0]);
            (pt, dir)
        };
        let (start, start_dir) = at(0.0);
        Ok((1..count)
            .map(|i| {
                let distance = (step * i as f64).min(total);
                let (pt, here_dir) = at(distance);
                let turn = if align {
                    let deg = clean((here_dir - start_dir).to_degrees()).rem_euclid(360.0);
                    if deg >= 360.0 - 1e-9 {
                        0.0
                    } else {
                        deg
                    }
                } else {
                    0.0
                };
                // Place::apply turns about `about` then adds `by`:
                // start + R(turn)*(p - start) + (pt - start) == pt + R(turn)*(p - start),
                // matching the core's move by (pt - start) then turn about pt.
                Place {
                    turn,
                    about: start,
                    by: [pt[0] - start[0], pt[1] - start[1]],
                }
            })
            .collect())
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

/// A number typed in a field: digits with a point and a sign, as the
/// command bar reads one; none for anything else, a blank among it.
fn number_typed(text: &str) -> Option<f64> {
    crate::tools::number(text)
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
    /// The items along a path, the objects the first.
    pub path_items: f64,
    /// Distance between items along a path when spacing is held between items.
    pub path_spacing: f64,
    /// Whether each copy aligns with the path's direction.
    pub align: bool,
    /// The picked path's length, if any, used to check spacing and sync evenly.
    pub path_length: Option<f64>,
    /// Whether the picked path is closed.
    pub path_closed: bool,
    /// How many objects it copies: the core's limit counts every copy of
    /// every one of them.
    pub objects: usize,
    /// The fields whose text does not read as a number, by key, with the
    /// text: shown as typed, and nothing lands until each reads.
    pub unread: Vec<(String, String)>,
    /// Whether a polar array's items go a whole turn, spread evenly: the
    /// panel's own state, set by the angle typed, kept as the number of
    /// items changes, so a count typed a digit at a time, `1` then `10`,
    /// stays a whole turn.
    whole_kept: bool,
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
    /// unit, six items round a whole turn, and six items evenly along a path.
    pub fn new(kind: ArrayKind, b: Option<[f64; 4]>) -> Self {
        let step = |v: f64| clean(((v + 0.2) / 0.25).ceil() * 0.25);
        let (w, h) = b.map_or((0.8, 0.8), |b| (b[2] - b[0], b[3] - b[1]));
        let row_spacing = -step(h);
        let column_spacing = step(w);
        Setup {
            kind,
            spacing: if kind == ArrayKind::Path {
                Spacing::Total
            } else {
                Spacing::Between
            },
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
            path_items: 6.0,
            path_spacing: 1.0,
            align: true,
            path_length: None,
            path_closed: false,
            objects: 1,
            unread: Vec::new(),
            whole_kept: true,
        }
    }

    /// Records the picked path's arc length and closure, syncing `path_spacing`
    /// when spacing is held evenly along the path or on first pick.
    pub fn set_path(&mut self, length: f64, closed: bool) {
        let first = self.path_length.is_none();
        self.path_length = Some(length);
        self.path_closed = closed;
        if (first || self.spacing == Spacing::Total) && self.path_items >= 2.0 {
            let divisor = if closed {
                self.path_items
            } else {
                self.path_items - 1.0
            };
            self.path_spacing = clean(length / divisor);
        }
    }

    /// Whether a polar array's items go a whole turn, spread evenly. It is
    /// set when an angle is typed, items times the angle between them a
    /// turn, or the angle to fill a turn, and kept as the items change.
    pub fn whole(&self) -> bool {
        self.whole_kept
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
                // A whole turn, either way; else to the last item, which may
                // come to 360 or more, said as wrong, since the core spreads
                // 360 evenly.
                if self.items >= 2.0 {
                    self.angle_fill = if self.whole_kept {
                        360f64.copysign(self.angle_between)
                    } else {
                        clean(self.angle_between * (self.items - 1.0))
                    };
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
                    self.angle_between = if self.whole_kept {
                        clean(self.angle_fill / self.items)
                    } else {
                        clean(self.angle_fill / (self.items - 1.0))
                    };
                }
                if self.path_items >= 2.0 {
                    if let Some(len) = self.path_length {
                        let divisor = if self.path_closed {
                            self.path_items
                        } else {
                            self.path_items - 1.0
                        };
                        self.path_spacing = clean(len / divisor);
                    }
                }
            }
        }
    }

    /// Holds the spacing between items, or the total.
    pub fn hold(&mut self, spacing: Spacing) {
        // With the total held, the angle to fill says whether it is a whole
        // turn: 360 either way is one, spread evenly, as the core reads it,
        // so items that ran a whole turn round to the last one, such as
        // four 120° apart, go 90° apart, as they land. Held there, it stays
        // so going back to the spacing between items.
        if spacing == Spacing::Total && self.items >= 2.0 {
            self.whole_kept = whole_turn(self.angle_fill);
        }
        self.spacing = spacing;
        self.sync();
    }

    /// Sets the value `key` names; false when it names none. The items of
    /// a polar array that goes a whole turn still go a whole turn when
    /// their number changes, as the prototype keeps them.
    pub fn set(&mut self, key: &str, v: f64) -> bool {
        self.unread.retain(|(k, _)| k != key);
        match key {
            "rows" => self.rows = v,
            "columns" => self.columns = v,
            "row_spacing" => self.row_spacing = v,
            "column_spacing" => self.column_spacing = v,
            "row_total" => self.row_total = v,
            "column_total" => self.column_total = v,
            "items" => {
                // A whole turn stays one, whatever the count passes through
                // as it is typed.
                if self.spacing == Spacing::Between && self.whole_kept && v >= 2.0 {
                    self.angle_between = clean(360f64.copysign(self.angle_between) / v);
                }
                self.items = v;
            }
            "angle_between" => {
                self.angle_between = v;
                self.whole_kept = whole_turn(v * self.items);
            }
            "angle_fill" => {
                self.angle_fill = v;
                self.whole_kept = whole_turn(v);
            }
            "path_items" => self.path_items = v,
            "path_spacing" => self.path_spacing = v,
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
            ArrayKind::Path => self.path_items,
        }
    }

    /// How many copies it makes of `objects` objects.
    pub fn copies(&self, objects: usize) -> usize {
        ((self.items_each() - 1.0).max(0.0) * objects as f64) as usize
    }

    /// Takes what is typed in the field `key`: its value when it reads as
    /// a number, else the text, kept as typed, which nothing lands past.
    /// False when `key` names no field.
    pub fn read(&mut self, key: &str, text: &str) -> bool {
        let number = number_typed(text);
        if number.is_some() || self.has(key) {
            self.unread.retain(|(k, _)| k != key);
        }
        match number {
            Some(v) => self.set(key, v),
            None if self.has(key) => {
                self.unread.push((key.to_owned(), text.to_owned()));
                true
            }
            None => false,
        }
    }

    fn has(&self, key: &str) -> bool {
        [
            "rows",
            "columns",
            "row_spacing",
            "column_spacing",
            "row_total",
            "column_total",
            "items",
            "angle_between",
            "angle_fill",
            "path_items",
            "path_spacing",
        ]
        .contains(&key)
    }

    /// What is wrong with the values, in words, if anything: a polar
    /// array's centre aside.
    pub fn problem(&self) -> Option<String> {
        // A field shown whose text is not a number: nothing lands until it is.
        for f in self.fields(None) {
            if self.unread.iter().any(|(k, _)| k == f.key) {
                let label = f.label.split(',').next().unwrap_or(&f.label).to_owned();
                return Some(format!("{label} is not a number: type one."));
            }
        }
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
                    if !self.whole() && self.angle_fill.abs() > 360.0 + 1e-9 {
                        return Some(format!(
                            "{} items {}° apart fill {}°: more than a whole turn.",
                            self.items,
                            f1(self.angle_between.abs()),
                            f1(self.angle_fill.abs())
                        ));
                    }
                    // The last a whole turn round sits on the objects: the
                    // core takes 360 as the items spread evenly round.
                    if !self.whole() && whole_turn(self.angle_fill) {
                        return Some(format!(
                            "{} items {}° apart put the last on the objects, a whole turn round: \
                             give {} items, or {}° apart.",
                            self.items,
                            f1(self.angle_between.abs()),
                            self.items - 1.0,
                            f1(360.0 / self.items)
                        ));
                    }
                }
            }
            ArrayKind::Path => {
                if !whole(self.path_items) || self.path_items < 2.0 {
                    return Some(
                        "Items are a whole number, 2 or more: the objects are the first."
                            .to_owned(),
                    );
                }
                if !total {
                    if self.path_spacing <= 0.0 || !self.path_spacing.is_finite() {
                        return Some("The spacing along the path is more than 0.".to_owned());
                    }
                    if let Some(len) = self.path_length {
                        let run = self.path_spacing * (self.path_items - 1.0);
                        if run > len * (1.0 + 1e-9) {
                            return Some(format!(
                                "{} items {} apart run {} along the path, which is {} long.",
                                self.path_items,
                                f3(self.path_spacing),
                                f3(run),
                                f3(len)
                            ));
                        }
                    }
                }
            }
        }
        let copies = self.copies(self.objects);
        if copies as f64 > MOST_COPIES {
            return Some(format!(
                "This makes {copies} copies; one array makes at most 10,000."
            ));
        }
        None
    }

    /// Its short name, for the tag over it and the journal: `3 × 2`,
    /// `8 round`, `5 round, -180°` when it fills less than a turn, or `6 along`.
    pub fn tag(&self) -> String {
        match self.kind {
            ArrayKind::Rectangular => format!("{} × {}", self.rows, self.columns),
            ArrayKind::Polar if self.whole() => format!("{} round", self.items),
            ArrayKind::Polar => format!("{} round, {}°", self.items, f1(self.angle_fill)),
            ArrayKind::Path => format!("{} along", self.path_items),
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
                let fill = if self.whole() && self.angle_fill < 0.0 {
                    "a whole turn clockwise".to_owned()
                } else if self.whole() {
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
            ArrayKind::Path => {
                let how = if self.spacing == Spacing::Total {
                    "evenly along the path".to_owned()
                } else {
                    format!("{}{unit} apart along the path", f3(self.path_spacing))
                };
                format!(
                    "{} items {how}{}: {copies}",
                    self.path_items,
                    if self.align { "" } else { ", not aligned" }
                )
            }
        }
    }

    /// The panel's fields, `unit` the sheet's unit, such as `in`.
    pub fn fields(&self, unit: Option<&str>) -> Vec<Field> {
        let unit = unit.map(|u| format!(", {u}")).unwrap_or_default();
        // A field whose text does not read shows it as typed.
        let field =
            |key: &'static str, label: String, hint: Option<&'static str>, v: f64, whole: bool| {
                let typed = self.unread.iter().find(|(k, _)| k == key);
                Field {
                    key,
                    label,
                    hint,
                    value: typed.map_or_else(|| shown(v), |(_, t)| t.clone()),
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
            ArrayKind::Path => {
                let mut out = vec![field(
                    "path_items",
                    "Items".to_owned(),
                    Some("the objects are the first"),
                    self.path_items,
                    true,
                )];
                if !total {
                    out.push(field(
                        "path_spacing",
                        format!("Spacing{unit}"),
                        Some("along the path"),
                        self.path_spacing,
                        false,
                    ));
                }
                out
            }
        }
    }

    /// Where each copy goes, the objects themselves left out: their box's
    /// centre is `middle`, which goes round when a polar array's copies are
    /// not turned, as the core's `base_point` left out does; `center` is a
    /// polar array's centre; `path` is a path array's curve. None while the
    /// values are wrong, or a polar array has no centre, or a path array has
    /// no path.
    pub fn places(
        &self,
        middle: [f64; 2],
        center: Option<[f64; 2]>,
        path: Option<&PathCurve>,
    ) -> Vec<Place> {
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
            ArrayKind::Path => {
                let Some(curve) = path else {
                    return Vec::new();
                };
                let spacing = (self.spacing == Spacing::Between).then_some(self.path_spacing);
                curve
                    .along(self.path_items as usize, spacing, self.align)
                    .unwrap_or_default()
            }
        }
    }

    /// The keys of the core's `array` operation, `center` a polar array's
    /// centre and `path` a path array's curve target as the operation takes
    /// them: the spacing between items, worked out from a total held, and the
    /// angle it fills, the items placed as the preview places them:
    /// (items - 1) times the angle between them, or, only when they go a whole
    /// turn spread evenly, left out, or -360 clockwise, so the copies go round
    /// the way the preview shows and keep its order and their IDs; `rotate`
    /// and `align` only when false, as the core turns/aligns them by default.
    pub fn keys(&self, center: Value, path: Value) -> Map<String, Value> {
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
                if !self.whole() {
                    k.insert("angle".into(), json!(self.angle_fill));
                } else if self.angle_fill < 0.0 {
                    k.insert("angle".into(), json!(-360.0));
                }
                if !self.rotate {
                    k.insert("rotate".into(), json!(false));
                }
            }
            ArrayKind::Path => {
                k.insert("kind".into(), json!("path"));
                k.insert("path".into(), path);
                k.insert("count".into(), json!(self.path_items as u64));
                if self.spacing == Spacing::Between {
                    k.insert("spacing".into(), json!(self.path_spacing));
                }
                if !self.align {
                    k.insert("align".into(), json!(false));
                }
            }
        }
        k
    }
}
