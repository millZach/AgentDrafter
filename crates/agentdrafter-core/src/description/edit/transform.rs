//! Rotates, scales and mirrors an object of a record in place, as AutoCAD's
//! ROTATE, SCALE and MIRROR do (#67): every point it is drawn through, every
//! direction it is drawn in and, scaled, every length it is drawn with.
//!
//! A turn by a multiple of 90° and a mirror across a line that runs
//! horizontally, vertically or at 45° are worked out exactly, with no
//! platform sine or cosine, so they write the same text, and so the same
//! DWG bytes, on every system (#243). A mirror's matrix takes no sine or
//! cosine at all: it is worked out from the line's direction with plain
//! arithmetic.
//!
//! A mirror keeps text reading forwards, as AutoCAD's MIRRTEXT 0 does: a
//! text's position mirrors, and it reads along its mirrored direction, or
//! that turned a half turn, whichever reads left to right or upwards, with
//! its own justification. It keeps a hatch's pattern running as it did, as
//! AutoCAD's MIRRHATCH 0 does: the boundary and the pattern's origin
//! mirror, and its angle stays. A mirrored insert is mirrored, as AutoCAD
//! keeps it, its X scale negative ([`crate::model::Insert::mirrored`]).

use super::*;
use crate::model::block::readable;

/// A rotation, an even scaling or a mirror, about a point, of the points,
/// directions and lengths an object is drawn with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Transform {
    about: [f64; 3],
    how: How,
    /// What it does to a direction seen from above, `[[xx, xy], [yx, yy]]`.
    m: [[f64; 2]; 2],
}

/// What a [`Transform`] does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum How {
    /// Turns by degrees, anticlockwise.
    Rotate(f64),
    /// Scales by a factor more than 0.
    Scale(f64),
    /// Mirrors across a line through the point, which runs at this many
    /// degrees from the X axis, in `[0, 180)`.
    Mirror(f64),
}

impl Transform {
    /// A turn by `degrees`, anticlockwise, about `about`.
    pub(crate) fn rotate(about: [f64; 3], degrees: f64) -> Self {
        let (sin, cos) = super::super::sin_cos_degrees(degrees);
        Transform {
            about,
            how: How::Rotate(degrees),
            m: [[cos, -sin], [sin, cos]],
        }
    }

    /// An even scaling by `factor` about `about`.
    pub(crate) fn scale(about: [f64; 3], factor: f64) -> Self {
        Transform {
            about,
            how: How::Scale(factor),
            m: [[factor, 0.0], [0.0, factor]],
        }
    }

    /// A mirror across the line through `a` and `b`, as AutoCAD's MIRROR
    /// takes it: upright, through the line seen from above, so a point
    /// keeps its Z. None when the two points are one seen from above.
    pub(crate) fn mirror(a: [f64; 3], b: [f64; 3]) -> Option<Self> {
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let square = dx * dx + dy * dy;
        if square == 0.0 || !square.is_finite() {
            return None;
        }
        // cos 2φ and sin 2φ from the line's direction, with no trigonometry.
        let cos2 = (dx * dx - dy * dy) / square;
        let sin2 = 2.0 * dx * dy / square;
        let axis = if dy == 0.0 {
            0.0
        } else if dx == 0.0 {
            90.0
        } else if dx == dy {
            45.0
        } else if dx == -dy {
            135.0
        } else {
            dy.atan2(dx).to_degrees().rem_euclid(180.0)
        };
        Some(Transform {
            about: a,
            how: How::Mirror(if axis >= 180.0 { 0.0 } else { axis }),
            m: [[cos2, sin2], [sin2, -cos2]],
        })
    }

    pub(crate) fn how(&self) -> How {
        self.how
    }

    pub(crate) fn about(&self) -> [f64; 3] {
        self.about
    }

    /// How much it enlarges a length: 1 but for a scaling.
    pub(crate) fn factor(&self) -> f64 {
        match self.how {
            How::Scale(factor) => factor,
            _ => 1.0,
        }
    }

    /// Whether it mirrors, so that what ran anticlockwise runs clockwise.
    pub(crate) fn mirrors(&self) -> bool {
        matches!(self.how, How::Mirror(_))
    }

    /// The word for what it does, as an error says it: `turn`, `scale` or
    /// `mirror`.
    fn verb(&self) -> &'static str {
        match self.how {
            How::Rotate(_) => "turn",
            How::Scale(_) => "scale",
            How::Mirror(_) => "mirror",
        }
    }

    /// The word for what it has done: `turned`, `scaled` or `mirrored`.
    pub(crate) fn done(&self) -> &'static str {
        match self.how {
            How::Rotate(_) => "turned",
            How::Scale(_) => "scaled",
            How::Mirror(_) => "mirrored",
        }
    }

    /// Where a point lands. A turn and a mirror keep its Z; a scaling
    /// scales it about the point's Z too, as AutoCAD's SCALE does.
    pub(crate) fn point(&self, p: [f64; 3]) -> [f64; 3] {
        let x = p[0] - self.about[0];
        let y = p[1] - self.about[1];
        [
            self.about[0] + self.m[0][0] * x + self.m[0][1] * y,
            self.about[1] + self.m[1][0] * x + self.m[1][1] * y,
            match self.how {
                How::Scale(k) => self.about[2] + k * (p[2] - self.about[2]),
                _ => p[2],
            },
        ]
    }

    /// Where a direction `[dx, dy, dz]` points: its Z scaled with a
    /// scaling.
    fn vector(&self, v: [f64; 3]) -> [f64; 3] {
        [
            self.m[0][0] * v[0] + self.m[0][1] * v[1],
            self.m[1][0] * v[0] + self.m[1][1] * v[1],
            v[2] * self.factor(),
        ]
    }

    /// The direction, in degrees anticlockwise, that a direction at
    /// `degrees` takes, in `[0, 360)`.
    pub(crate) fn angle(&self, degrees: f64) -> f64 {
        let turned = match self.how {
            How::Rotate(by) => degrees + by,
            How::Scale(_) => degrees,
            How::Mirror(axis) => 2.0 * axis - degrees,
        };
        whole_degrees(turned)
    }

    /// The direction a text, an mtext or an attribute at `degrees` reads
    /// in: turned with the object, or, mirrored, the mirrored direction or
    /// that turned a half turn, whichever reads forwards, as AutoCAD's
    /// MIRRTEXT 0 keeps text reading.
    pub(crate) fn text_angle(&self, degrees: f64) -> f64 {
        match self.how {
            How::Mirror(_) => whole_degrees(readable(self.angle(degrees))),
            _ => self.angle(degrees),
        }
    }

    /// Whether an ordinate dimension measuring along X measures along Y
    /// once transformed, and the other way round: turned a quarter either
    /// way, or mirrored across a line at 45 degrees.
    fn swaps_axes(&self) -> bool {
        match self.how {
            How::Rotate(degrees) => whole_degrees(degrees) % 180.0 == 90.0,
            How::Mirror(axis) => axis == 45.0 || axis == 135.0,
            How::Scale(_) => false,
        }
    }

    /// An arc's start and end angles, anticlockwise, once it lands: a
    /// mirror runs it the other way, so its ends swap.
    fn arc_angles(&self, start: f64, end: f64) -> (f64, f64) {
        if self.mirrors() {
            (self.angle(end), self.angle(start))
        } else {
            (self.angle(start), self.angle(end))
        }
    }
}

/// `degrees` in `[0, 360)`, a whole turn read as 0 once written.
fn whole_degrees(degrees: f64) -> f64 {
    let turned = super::super::normalize_degrees(degrees);
    if number_text(turned) == "360" {
        0.0
    } else {
        turned
    }
}

/// What an object draws with that its record does not write, worked out
/// from the sheet, for a scaling to write as the object's own, and a
/// table's size, for a mirror to place it by.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Resolved {
    /// Lengths in the declared unit, each by its key's place in the object,
    /// such as `arrow_size` or `content.height`: a leader's taken from its
    /// style or the sheet's defaults.
    pub lengths: Vec<(&'static str, f64)>,
    /// A table's width and height.
    pub size: Option<[f64; 2]>,
}

/// The object types a transform takes: every type a sheet holds but a
/// viewport, which is a window on the paper.
const TRANSFORMED_TYPES: [&str; 16] = [
    "line",
    "polyline",
    "arc",
    "circle",
    "ellipse",
    "spline",
    "hatch",
    "text",
    "mtext",
    "donut",
    "solid",
    "insert",
    "table",
    "mleader",
    "leader",
    "dimension",
];

impl RecordFile {
    /// Why the object of `list` at `index` cannot be transformed by `t`,
    /// when the sheet description cannot hold the result: a viewport, a
    /// table turned, scaled or mirrored across a slanting line, and a
    /// multileader's block content mirrored.
    pub(crate) fn refuses_transform(
        &self,
        list: ObjectList,
        index: usize,
        t: &Transform,
    ) -> Option<String> {
        let item = self.items(list)?.get(index)?;
        let (kind, keys) = object_parts(item);
        let kind = kind?;
        let what = t.done();
        match (kind, t.how) {
            ("viewport", _) => Some(format!(
                "a viewport is a window on the paper and is not {what}; move it, or change its \
                 view with change_viewport"
            )),
            ("table", How::Rotate(degrees)) if whole_degrees(degrees) != 0.0 => Some(
                "a table hangs from its top left corner, its rows running down and its columns \
                 across, and has no rotation of its own, so it is not turned"
                    .to_owned(),
            ),
            ("table", How::Scale(factor)) if factor != 1.0 => Some(
                "a table's text is as high as its table style or its cells say, so it is not \
                 scaled; change its columns and rows instead"
                    .to_owned(),
            ),
            ("table", How::Mirror(axis)) if axis != 0.0 && axis != 90.0 => Some(
                "a table stays upright, so it is mirrored only across a line that runs across \
                 or up the sheet, which leaves it upright"
                    .to_owned(),
            ),
            ("mleader", How::Mirror(_))
                if keys
                    .and_then(|keys| keys.iter().find(|e| e.key == "content"))
                    .and_then(|content| mapping_entry(&content.value, "block"))
                    .is_some() =>
            {
                Some(
                    "a multileader's block content is held upright, so a multileader with \
                     block content is not mirrored"
                        .to_owned(),
                )
            }
            ("dimension", How::Rotate(degrees))
                if is_ordinate(keys) && whole_degrees(degrees) % 90.0 != 0.0 =>
            {
                Some(
                    "an ordinate dimension measures a feature's x or its y, along the sheet's \
                     axes, so it is turned only by a multiple of 90 degrees, which leaves it \
                     measuring along one of them"
                        .to_owned(),
                )
            }
            ("dimension", How::Mirror(axis)) if is_ordinate(keys) && axis % 45.0 != 0.0 => Some(
                "an ordinate dimension measures a feature's x or its y, along the sheet's axes, \
                 so it is mirrored only across a line that runs across or up the sheet or at 45 \
                 degrees, which leaves it measuring along one of them"
                    .to_owned(),
            ),
            _ => None,
        }
    }

    /// Transforms the object of `list` at `index` by `t`, in the declared
    /// unit `units`, as operation `operation`: every point it is drawn
    /// through lands where `t` puts it, every direction it is drawn in
    /// turns with it, and, scaled, every length it is drawn with is
    /// scaled, those `resolved` gives written as its own. A coordinate, an
    /// angle or a length it leaves where it was is left as written.
    pub(crate) fn transform(
        &mut self,
        list: ObjectList,
        index: usize,
        t: &Transform,
        resolved: &Resolved,
        units: Unit,
        operation: usize,
    ) -> Result<(), Error> {
        if let Some(why) = self.refuses_transform(list, index, t) {
            return Err(Error::new(ErrorCode::BadValue, why).at_field(list.place(index)));
        }
        self.state(list);
        let path = list.place(index);
        let items = self.items_mut(list)?;
        let item = &mut items[index];
        let kind = object_parts(item).0.unwrap_or_default().to_owned();
        if !TRANSFORMED_TYPES.contains(&kind.as_str()) {
            return Err(Error::new(
                ErrorCode::BadValue,
                format!("{} {kind} is not {}", article(&kind), t.done()),
            )
            .at_field(path));
        }
        let keys = object_keys_mut(item).ok_or_else(|| record_shape(&kind))?;
        let object_path = key_path(&path, &kind);
        let at = At { t, units, resolved };
        let changed = at.object(&kind, keys, &object_path)?;
        self.touch(list, index, operation, Touch::Keys(changed));
        Ok(())
    }
}

/// Whether a dimension's keys make it an ordinate dimension.
fn is_ordinate(keys: Option<&[Entry]>) -> bool {
    keys.and_then(|keys| scalar_of(keys, "kind")) == Some("ordinate")
}

/// A transform at work on one object's keys.
struct At<'a> {
    t: &'a Transform,
    units: Unit,
    resolved: &'a Resolved,
}

impl At<'_> {
    /// Transforms an object of type `kind` by its `keys`, and gives the
    /// keys it changed.
    fn object(&self, kind: &str, keys: &mut Vec<Entry>, path: &str) -> Result<Vec<String>, Error> {
        let t = self.t;
        let k = t.factor();
        let scaled = k != 1.0;
        let mut changed: Vec<String> = Vec::new();
        let solid = hatch_is_solid(keys);
        if kind == "dimension" && self.clear_angular_location(keys, path) {
            changed.push("location".to_owned());
        }
        for entry in keys.iter_mut() {
            let field = key_path(path, &entry.key);
            let did = match (kind, entry.key.as_str()) {
                ("line", "from" | "to")
                | ("arc" | "circle" | "ellipse" | "donut", "center")
                | ("text" | "mtext" | "insert", "position")
                | ("dimension", "center" | "point" | "location" | "feature") => {
                    self.point(&mut entry.value, &field)?
                }
                ("polyline", "points") => self.vertices(&mut entry.value, &field)?,
                ("spline", "fit_points" | "control_points")
                | ("solid", "corners")
                | ("leader" | "dimension", "points") => self.points(&mut entry.value, &field)?,
                ("mleader", "leaders") | ("dimension", "lines") => {
                    let Kind::Sequence(lines) = &mut entry.value.kind else {
                        return Err(self.not_a_point(&field));
                    };
                    let mut any = false;
                    for (i, line) in lines.iter_mut().enumerate() {
                        any |= self.points(line, &index_path(&field, i))?;
                    }
                    any
                }
                ("ellipse", "major_axis") => self.vector(&mut entry.value, &field)?,
                ("polyline", "width")
                | ("arc" | "circle", "radius")
                | ("donut", "outside_diameter" | "inside_diameter")
                | ("text" | "mtext", "height")
                | ("mtext", "width")
                | ("mleader" | "leader", "arrow_size" | "gap")
                | ("mleader", "landing")
                    if scaled =>
                {
                    self.length(&mut entry.value, &field)?
                }
                ("insert", "scale") | ("hatch", "scale") if scaled && !solid => {
                    self.number(&mut entry.value, &field)?
                }
                ("text" | "mtext", "rotation") => {
                    self.angle(&mut entry.value, &field, &|a| t.text_angle(a))?
                }
                ("insert", "rotation") => match t.how {
                    How::Mirror(_) => {
                        self.angle(&mut entry.value, &field, &|a| t.angle(a) + 180.0)?
                    }
                    _ => self.angle(&mut entry.value, &field, &|a| t.angle(a))?,
                },
                // A linear dimension measures the same either way along its
                // line, so its angle is kept in [0, 180).
                ("dimension", "angle") => {
                    self.angle(&mut entry.value, &field, &|a| t.angle(a) % 180.0)?
                }
                // An ordinate dimension turned a quarter, or mirrored across
                // a line at 45 degrees, measures along the other axis.
                ("dimension", "axis") if t.swaps_axes() => {
                    let other = match &entry.value.kind {
                        Kind::Scalar(axis) if axis == "x" => "y",
                        Kind::Scalar(axis) if axis == "y" => "x",
                        _ => {
                            return Err(Error::new(
                                ErrorCode::BadValue,
                                format!(
                                    "the record's ordinate axis here is not `x` or `y`, so it \
                                     cannot be {}",
                                    t.done()
                                ),
                            )
                            .at_field(field))
                        }
                    };
                    entry.value = Node::scalar(other.to_owned());
                    true
                }
                ("hatch", "angle") if !solid && !t.mirrors() => {
                    self.angle(&mut entry.value, &field, &|a| t.angle(a))?
                }
                ("hatch", "boundary") => self.boundary(&mut entry.value, &field)?,
                ("hatch", "origin") if !solid => self.point(&mut entry.value, &field)?,
                ("table", "position") => self.table_position(&mut entry.value, &field)?,
                ("mleader", "content") => self.mleader_content(&mut entry.value, &field)?,
                ("leader", "annotation") if scaled => {
                    self.own_length(&mut entry.value, "height", "annotation.height", &field)?
                }
                _ => false,
            };
            if did && !changed.contains(&entry.key) {
                changed.push(entry.key.clone());
            }
        }
        // Keys the object draws with that it does not write: a turn or a
        // mirror's rotation of a text, an mtext or an insert, a patterned
        // hatch's angle and origin, an insert's scale, and, scaled, a
        // leader's lengths from its style.
        let written = |keys: &[Entry], key: &str| keys.iter().any(|e| e.key == key);
        match kind {
            "text" | "mtext" if !written(keys, "rotation") => {
                let r = t.text_angle(0.0);
                if r != 0.0 {
                    keys.push(new_entry("rotation", Node::scalar(number_text(r))));
                    changed.push("rotation".to_owned());
                }
            }
            "insert" => {
                if !written(keys, "rotation") {
                    let r = match t.how {
                        How::Mirror(_) => whole_degrees(t.angle(0.0) + 180.0),
                        _ => t.angle(0.0),
                    };
                    if r != 0.0 {
                        keys.push(new_entry("rotation", Node::scalar(number_text(r))));
                        changed.push("rotation".to_owned());
                    }
                }
                if scaled && !written(keys, "scale") {
                    keys.push(new_entry("scale", Node::scalar(number_text(k))));
                    changed.push("scale".to_owned());
                }
                if t.mirrors() {
                    let was = keys.iter().position(|e| e.key == "mirrored").map(|i| {
                        (
                            i,
                            matches!(&keys[i].value.kind, Kind::Scalar(s) if s == "true"),
                        )
                    });
                    match was {
                        Some((i, true)) => {
                            keys.remove(i);
                            changed.push("mirrored".to_owned());
                        }
                        Some((i, false)) => {
                            keys[i].value = Node::scalar("true".to_owned());
                            changed.push("mirrored".to_owned());
                        }
                        None => {
                            keys.push(new_entry("mirrored", Node::scalar("true".to_owned())));
                            changed.push("mirrored".to_owned());
                        }
                    }
                    // Its attributes read forwards after every mirror, as
                    // MIRRTEXT 0 leaves them: a mirrored insert's always do,
                    // and one mirrored back keeps them so with
                    // `readable_attributes`.
                    let now_mirrored = !matches!(was, Some((_, true)));
                    let readable = keys.iter().position(|e| e.key == "readable_attributes");
                    match (now_mirrored, readable) {
                        (true, Some(i)) => {
                            keys.remove(i);
                            changed.push("readable_attributes".to_owned());
                        }
                        (false, Some(i)) => {
                            if !matches!(&keys[i].value.kind, Kind::Scalar(s) if s == "true") {
                                keys[i].value = Node::scalar("true".to_owned());
                                changed.push("readable_attributes".to_owned());
                            }
                        }
                        (false, None) => {
                            keys.push(new_entry(
                                "readable_attributes",
                                Node::scalar("true".to_owned()),
                            ));
                            changed.push("readable_attributes".to_owned());
                        }
                        (true, None) => {}
                    }
                }
            }
            "hatch" if !solid => {
                if !written(keys, "angle") && !t.mirrors() {
                    let r = t.angle(0.0);
                    if r != 0.0 {
                        keys.push(new_entry("angle", Node::scalar(number_text(r))));
                        changed.push("angle".to_owned());
                    }
                }
                if scaled && !written(keys, "scale") {
                    keys.push(new_entry("scale", Node::scalar(number_text(k))));
                    changed.push("scale".to_owned());
                }
                // A patterned hatch written without an origin is laid from
                // [0, 0, 0], which goes with it.
                if !written(keys, "origin") {
                    let mut node = Node::new(Kind::Sequence(
                        ["0", "0", "0"].map(|c| Node::scalar(c.to_owned())).to_vec(),
                    ));
                    if self.point(&mut node, "origin")? {
                        keys.push(new_entry("origin", node));
                        changed.push("origin".to_owned());
                    }
                }
            }
            // A linear dimension written without its angle measures across
            // the sheet: turned or mirrored, it measures along its turned
            // direction.
            "dimension" if scalar_of(keys, "kind") == Some("linear") && !written(keys, "angle") => {
                let r = t.angle(0.0) % 180.0;
                if r != 0.0 {
                    keys.push(new_entry("angle", Node::scalar(number_text(r))));
                    changed.push("angle".to_owned());
                }
            }
            "mleader" | "leader" if scaled => {
                let lengths: &[&str] = if kind == "mleader" {
                    &["arrow_size", "gap", "landing"]
                } else {
                    &["arrow_size", "gap"]
                };
                for &key in lengths {
                    if written(keys, key) {
                        continue;
                    }
                    if let Some(length) = self.resolved_length(key) {
                        keys.push(new_entry(key, Node::scalar(number_text(length * k))));
                        changed.push(key.to_owned());
                    }
                }
            }
            _ => {}
        }
        // An arc's and an ellipse's angles go together: a mirror swaps them.
        if matches!(kind, "arc" | "ellipse")
            && !matches!(t.how, How::Scale(_))
            && self.arc_angles(kind, keys, path)?
        {
            changed.push("start_angle".to_owned());
            changed.push("end_angle".to_owned());
        }
        Ok(changed)
    }

    /// Puts an angular dimension's location midway round the sector it
    /// measures when it lies on, or a hair off, one of its rays, and says
    /// whether it did. The sheet's reader takes a location on a ray as in
    /// the sector anticlockwise from it, which a mirror runs the other way,
    /// and a turn may leave it a hair either side; one midway lands inside
    /// the sector the transformed dimension measures, so it measures the
    /// sector transformed. A location clear of its rays is left as written.
    fn clear_angular_location(&self, keys: &mut [Entry], path: &str) -> bool {
        use crate::model::{Dimension, Measured, Point3};
        let read = |key: &str| -> Option<Point3> {
            let node = &keys.iter().find(|e| e.key == key)?.value;
            let [x, y, z] = self.read_point(node, path).ok()?;
            Some(Point3 { x, y, z })
        };
        let pair = |node: &Node| -> Option<[Point3; 2]> {
            let Kind::Sequence(points) = &node.kind else {
                return None;
            };
            let [a, b] = points.as_slice() else {
                return None;
            };
            let [a, b] = [a, b].map(|p| self.read_point(p, path).ok());
            let ([ax, ay, az], [bx, by, bz]) = (a?, b?);
            Some([
                Point3 {
                    x: ax,
                    y: ay,
                    z: az,
                },
                Point3 {
                    x: bx,
                    y: by,
                    z: bz,
                },
            ])
        };
        let measured = match scalar_of(keys, "kind") {
            Some("angular") => {
                let node = keys.iter().find(|e| e.key == "lines").map(|e| &e.value);
                let Some(Kind::Sequence(lines)) = node.map(|n| &n.kind) else {
                    return false;
                };
                let [first, second] = lines.as_slice() else {
                    return false;
                };
                let (Some(first), Some(second)) = (pair(first), pair(second)) else {
                    return false;
                };
                Measured::Angular {
                    lines: [first, second],
                }
            }
            Some("three_point_angular") => {
                let points = keys.iter().find(|e| e.key == "points").map(|e| &e.value);
                let (Some(center), Some(points)) = (read("center"), points.and_then(pair)) else {
                    return false;
                };
                Measured::ThreePointAngular { center, points }
            }
            _ => return false,
        };
        let Some(location) = read("location") else {
            return false;
        };
        let probe = Dimension {
            measured,
            location,
            dimension_style: String::new(),
            text: None,
            continued: false,
            layer: String::new(),
            properties: Default::default(),
        };
        let Some(sector) = probe.sector() else {
            return false;
        };
        if sector.holds_clear(location) {
            return false;
        }
        let (dx, dy) = (location.x - sector.vertex.x, location.y - sector.vertex.y);
        let middle = sector.middle((dx * dx + dy * dy).sqrt(), location.z);
        let Some(entry) = keys.iter_mut().find(|e| e.key == "location") else {
            return false;
        };
        entry.value = Node::new(Kind::Sequence(
            [middle.x, middle.y, middle.z]
                .map(|c| Node::scalar(number_text(c)))
                .to_vec(),
        ));
        true
    }

    /// Reads a point `[x, y, z]` in the declared unit.
    fn read_point(&self, node: &Node, field: &str) -> Result<[f64; 3], Error> {
        let Kind::Sequence(coordinates) = &node.kind else {
            return Err(self.not_a_point(field));
        };
        if coordinates.len() != 3 {
            return Err(self.not_a_point(field));
        }
        let mut p = [0.0; 3];
        for (axis, c) in p.iter_mut().enumerate() {
            *c = self.coordinate(coordinates, axis, field)?;
        }
        Ok(p)
    }

    /// A length the object draws with that its record does not write.
    fn resolved_length(&self, key: &str) -> Option<f64> {
        self.resolved
            .lengths
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, length)| *length)
    }

    fn not_a_point(&self, field: &str) -> Error {
        Error::new(
            ErrorCode::WrongType,
            format!(
                "the record's point here is not a list of three lengths, so the object cannot \
                 be {}",
                self.t.done()
            ),
        )
        .at_field(field)
    }

    /// Reads coordinate `axis` of the point `coordinates` holds, at
    /// `field`, in the declared unit.
    fn coordinate(&self, coordinates: &[Node], axis: usize, field: &str) -> Result<f64, Error> {
        let Kind::Scalar(text) = &coordinates[axis].kind else {
            return Err(self.not_a_point(field));
        };
        match parse_length(text) {
            Ok((value, None)) => Ok(value),
            Ok((value, Some(own))) => Ok(own.convert(value, self.units)),
            Err(_) => Err(Error::new(
                ErrorCode::BadNumber,
                format!(
                    "the record's coordinate `{text}` is not a length to {}",
                    self.t.verb()
                ),
            )
            .at_field(index_path(field, axis))),
        }
    }

    /// Writes `value` as coordinate `axis` of `coordinates`.
    fn write(&self, node: &mut Node, value: f64, field: &str) -> Result<(), Error> {
        if !value.is_finite() {
            return Err(Error::new(
                ErrorCode::BadNumber,
                format!("the value would be too large once {}", self.t.done()),
            )
            .at_field(field));
        }
        *node = Node::scalar(number_text(value));
        Ok(())
    }

    /// Transforms a point `[x, y, z]`, and says whether it moved.
    fn point(&self, node: &mut Node, field: &str) -> Result<bool, Error> {
        let Kind::Sequence(coordinates) = &mut node.kind else {
            return Err(self.not_a_point(field));
        };
        if coordinates.len() != 3 {
            return Err(self.not_a_point(field));
        }
        let mut p = [0.0; 3];
        for (axis, c) in p.iter_mut().enumerate() {
            *c = self.coordinate(coordinates, axis, field)?;
        }
        let landed = self.t.point(p);
        let mut moved = false;
        for axis in 0..3 {
            // A coordinate the transform leaves where it was is left as
            // written, a Z that a turn or a mirror keeps among them.
            if landed[axis] != p[axis] {
                self.write(
                    &mut coordinates[axis],
                    landed[axis],
                    &index_path(field, axis),
                )?;
                moved = true;
            }
        }
        Ok(moved)
    }

    /// Transforms a direction `[dx, dy, dz]`.
    fn vector(&self, node: &mut Node, field: &str) -> Result<bool, Error> {
        let Kind::Sequence(coordinates) = &mut node.kind else {
            return Err(self.not_a_point(field));
        };
        if coordinates.len() != 3 {
            return Err(self.not_a_point(field));
        }
        let mut v = [0.0; 3];
        for (axis, c) in v.iter_mut().enumerate() {
            *c = self.coordinate(coordinates, axis, field)?;
        }
        let landed = self.t.vector(v);
        let mut moved = false;
        for axis in 0..3 {
            if landed[axis] != v[axis] {
                self.write(
                    &mut coordinates[axis],
                    landed[axis],
                    &index_path(field, axis),
                )?;
                moved = true;
            }
        }
        Ok(moved)
    }

    /// Transforms each point of a list.
    fn points(&self, node: &mut Node, field: &str) -> Result<bool, Error> {
        let Kind::Sequence(points) = &mut node.kind else {
            return Err(self.not_a_point(field));
        };
        let mut any = false;
        for (i, point) in points.iter_mut().enumerate() {
            any |= self.point(point, &index_path(field, i))?;
        }
        Ok(any)
    }

    /// Transforms a polyline's points: each point, a mapping's by its
    /// `point`; a mirror runs its arc segments the other way, and a scaling
    /// scales its widths.
    fn vertices(&self, node: &mut Node, field: &str) -> Result<bool, Error> {
        let Kind::Sequence(points) = &mut node.kind else {
            return Err(self.not_a_point(field));
        };
        let mut any = false;
        for (i, point) in points.iter_mut().enumerate() {
            let at = index_path(field, i);
            let Kind::Mapping(keys) = &mut point.kind else {
                any |= self.point(point, &at)?;
                continue;
            };
            for key in keys.iter_mut() {
                let path = key_path(&at, &key.key);
                any |= match key.key.as_str() {
                    "point" => self.point(&mut key.value, &path)?,
                    "bulge" | "arc_angle" if self.t.mirrors() => {
                        self.negate(&mut key.value, &path)?
                    }
                    "start_width" | "end_width" if self.t.factor() != 1.0 => {
                        self.length(&mut key.value, &path)?
                    }
                    _ => false,
                };
            }
        }
        Ok(any)
    }

    /// Negates a plain number, such as a bulge.
    fn negate(&self, node: &mut Node, field: &str) -> Result<bool, Error> {
        let value = self.plain(node, field)?;
        if value == 0.0 {
            return Ok(false);
        }
        self.write(node, -value, field)?;
        Ok(true)
    }

    /// Reads a plain number.
    fn plain(&self, node: &Node, field: &str) -> Result<f64, Error> {
        let Kind::Scalar(text) = &node.kind else {
            return Err(Error::new(
                ErrorCode::WrongType,
                format!(
                    "the record's value here is not a number to {}",
                    self.t.verb()
                ),
            )
            .at_field(field));
        };
        text.trim().parse::<f64>().map_err(|_| {
            Error::new(
                ErrorCode::BadNumber,
                format!(
                    "the record's value `{text}` is not a number to {}",
                    self.t.verb()
                ),
            )
            .at_field(field)
        })
    }

    /// Scales a plain number, such as an insert's scale.
    fn number(&self, node: &mut Node, field: &str) -> Result<bool, Error> {
        let value = self.plain(node, field)?;
        self.write(node, value * self.t.factor(), field)?;
        Ok(true)
    }

    /// Scales a length, written in the declared unit once scaled.
    fn length(&self, node: &mut Node, field: &str) -> Result<bool, Error> {
        let Kind::Scalar(text) = &node.kind else {
            return Err(Error::new(
                ErrorCode::WrongType,
                format!(
                    "the record's length here is not a length to {}",
                    self.t.verb()
                ),
            )
            .at_field(field));
        };
        let value = match parse_length(text) {
            Ok((value, None)) => value,
            Ok((value, Some(own))) => own.convert(value, self.units),
            Err(_) => {
                return Err(Error::new(
                    ErrorCode::BadNumber,
                    format!(
                        "the record's length `{text}` is not a length to {}",
                        self.t.verb()
                    ),
                )
                .at_field(field))
            }
        };
        self.write(node, value * self.t.factor(), field)?;
        Ok(true)
    }

    /// Sets an angle in degrees to what `f` makes of it, in `[0, 360)`.
    fn angle(&self, node: &mut Node, field: &str, f: &dyn Fn(f64) -> f64) -> Result<bool, Error> {
        let Kind::Scalar(text) = &node.kind else {
            return Err(Error::new(
                ErrorCode::WrongType,
                format!(
                    "the record's angle here is not a number of degrees to {}",
                    self.t.verb()
                ),
            )
            .at_field(field));
        };
        let Ok(degrees) = text.trim().parse::<f64>() else {
            return Err(Error::new(
                ErrorCode::BadNumber,
                format!(
                    "the record's angle `{text}` is not a number of degrees to {}",
                    self.t.verb()
                ),
            )
            .at_field(field));
        };
        let landed = whole_degrees(f(degrees));
        if landed == degrees {
            return Ok(false);
        }
        *node = Node::scalar(number_text(landed));
        Ok(true)
    }

    /// An arc's start and end angles, or an ellipse's, which a mirror
    /// swaps: an arc's turn with it, and an ellipse's, measured from its
    /// major axis, which turns with it, stay but for a mirror, which
    /// measures them the other way round.
    fn arc_angles(&self, kind: &str, keys: &mut [Entry], path: &str) -> Result<bool, Error> {
        let start = keys.iter().position(|e| e.key == "start_angle");
        let end = keys.iter().position(|e| e.key == "end_angle");
        let (Some(s), Some(e)) = (start, end) else {
            return Ok(false);
        };
        let read = |i: usize| -> Result<f64, Error> {
            let field = key_path(path, &keys[i].key);
            self.plain(&keys[i].value, &field)
        };
        let (a, b) = (read(s)?, read(e)?);
        let (new_a, new_b) = if kind == "ellipse" {
            if !self.t.mirrors() {
                return Ok(false);
            }
            (whole_degrees(-b), whole_degrees(-a))
        } else {
            self.t.arc_angles(a, b)
        };
        if (new_a, new_b) == (a, b) {
            return Ok(false);
        }
        keys[s].value = Node::scalar(number_text(new_a));
        keys[e].value = Node::scalar(number_text(new_b));
        Ok(true)
    }

    /// Transforms each loop of a hatch's boundary: a polyline's points, and
    /// each edge's ends, centre, radius and angles.
    fn boundary(&self, node: &mut Node, field: &str) -> Result<bool, Error> {
        let Kind::Sequence(loops) = &mut node.kind else {
            return Err(self.not_a_point(field));
        };
        let mut any = false;
        for (i, boundary_loop) in loops.iter_mut().enumerate() {
            let loop_path = index_path(field, i);
            let Kind::Mapping(kinds) = &mut boundary_loop.kind else {
                continue;
            };
            for kind in kinds.iter_mut() {
                let kind_path = key_path(&loop_path, &kind.key);
                match (kind.key.as_str(), &mut kind.value.kind) {
                    ("polyline", Kind::Mapping(keys)) => {
                        for key in keys.iter_mut().filter(|k| k.key == "points") {
                            any |= self.points(&mut key.value, &key_path(&kind_path, "points"))?;
                        }
                    }
                    ("edges", Kind::Sequence(edges)) => {
                        for (j, edge) in edges.iter_mut().enumerate() {
                            let edge_path = index_path(&kind_path, j);
                            let Kind::Mapping(edge_kinds) = &mut edge.kind else {
                                continue;
                            };
                            for edge_kind in edge_kinds.iter_mut() {
                                let within = key_path(&edge_path, &edge_kind.key);
                                let Kind::Mapping(keys) = &mut edge_kind.value.kind else {
                                    continue;
                                };
                                for key in keys.iter_mut() {
                                    let path = key_path(&within, &key.key);
                                    any |= match key.key.as_str() {
                                        "from" | "to" | "center" => {
                                            self.point(&mut key.value, &path)?
                                        }
                                        "radius" if self.t.factor() != 1.0 => {
                                            self.length(&mut key.value, &path)?
                                        }
                                        _ => false,
                                    };
                                }
                                if edge_kind.key == "arc" && !matches!(self.t.how, How::Scale(_)) {
                                    any |= self.arc_angles("arc", keys, &within)?;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(any)
    }

    /// Places a table, mirrored across a line that runs across or up the
    /// sheet, by the top left corner of its mirrored box, so it hangs
    /// where its mirror image does and reads forwards; turned by a whole
    /// turn or scaled by 1, it stays.
    fn table_position(&self, node: &mut Node, field: &str) -> Result<bool, Error> {
        let How::Mirror(_) = self.t.how else {
            return Ok(false);
        };
        let Kind::Sequence(coordinates) = &mut node.kind else {
            return Err(self.not_a_point(field));
        };
        if coordinates.len() != 3 {
            return Err(self.not_a_point(field));
        }
        let x = self.coordinate(coordinates, 0, field)?;
        let y = self.coordinate(coordinates, 1, field)?;
        let z = self.coordinate(coordinates, 2, field)?;
        let [width, height] = self.resolved.size.unwrap_or([0.0, 0.0]);
        let corners = [
            [x, y],
            [x + width, y],
            [x, y - height],
            [x + width, y - height],
        ]
        .map(|[cx, cy]| self.t.point([cx, cy, z]));
        let left = corners.iter().map(|c| c[0]).fold(f64::INFINITY, f64::min);
        let top = corners
            .iter()
            .map(|c| c[1])
            .fold(f64::NEG_INFINITY, f64::max);
        let mut moved = false;
        for (axis, value) in [(0, left), (1, top)] {
            let was = if axis == 0 { x } else { y };
            if value != was {
                self.write(&mut coordinates[axis], value, &index_path(field, axis))?;
                moved = true;
            }
        }
        Ok(moved)
    }

    /// A multileader's content: its text's height, scaled, written as its
    /// own when its style gives it; or its block's rotation, turned, and
    /// scale, scaled. Its text is laid out level again where its landing
    /// puts it, as every multileader's is.
    fn mleader_content(&self, node: &mut Node, field: &str) -> Result<bool, Error> {
        let Kind::Mapping(keys) = &mut node.kind else {
            return Ok(false);
        };
        let block = keys.iter().any(|e| e.key == "block");
        let t = self.t;
        if !block {
            return if t.factor() != 1.0 {
                self.own_length(node, "height", "content.height", field)
            } else {
                Ok(false)
            };
        }
        let mut any = false;
        for key in keys.iter_mut() {
            let path = key_path(field, &key.key);
            any |= match key.key.as_str() {
                "rotation" => self.angle(&mut key.value, &path, &|a| t.angle(a))?,
                "scale" if t.factor() != 1.0 => self.number(&mut key.value, &path)?,
                _ => false,
            };
        }
        if !keys.iter().any(|e| e.key == "rotation") {
            let r = t.angle(0.0);
            if r != 0.0 {
                keys.push(new_entry("rotation", Node::scalar(number_text(r))));
                any = true;
            }
        }
        if t.factor() != 1.0 && !keys.iter().any(|e| e.key == "scale") {
            keys.push(new_entry("scale", Node::scalar(number_text(t.factor()))));
            any = true;
        }
        Ok(any)
    }

    /// Scales the length `key` of the mapping `node`, or writes the one
    /// `resolved` gives it at `place`, scaled, when it does not write one.
    fn own_length(
        &self,
        node: &mut Node,
        key: &str,
        place: &str,
        field: &str,
    ) -> Result<bool, Error> {
        let Kind::Mapping(keys) = &mut node.kind else {
            return Ok(false);
        };
        if let Some(entry) = keys.iter_mut().find(|e| e.key == key) {
            return self.length(&mut entry.value, &key_path(field, key));
        }
        match self.resolved_length(place) {
            Some(length) => {
                keys.push(new_entry(
                    key,
                    Node::scalar(number_text(length * self.t.factor())),
                ));
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A quarter turn about the origin.
    fn turn() -> Transform {
        Transform::rotate([0.0; 3], 90.0)
    }

    /// Doubled about the origin.
    fn doubled() -> Transform {
        Transform::scale([0.0; 3], 2.0)
    }

    /// Mirrored across the line x = 10, up the sheet.
    fn mirrored() -> Transform {
        Transform::mirror([10.0, 0.0, 0.0], [10.0, 1.0, 0.0]).unwrap()
    }

    /// `object` as its record writes it once `t` has transformed it, the
    /// lengths `resolved` gives written as its own; or the error.
    fn transformed(object: &str, t: Transform, resolved: &Resolved) -> Result<String, Error> {
        let text = format!("units: mm\nobjects:\n  - {object}\n");
        let mut file = RecordFile::from_text(Path::new("t.yaml"), text).unwrap();
        file.transform(ObjectList::Template, 0, &t, resolved, Unit::Mm, 0)?;
        // The new text reads back as the tree the edit made.
        file.new_text().unwrap();
        Ok(file.object_text(ObjectList::Template, 0))
    }

    #[track_caller]
    fn lands(object: &str, t: Transform, wanted: &str) {
        assert_eq!(
            transformed(object, t, &Resolved::default()).unwrap(),
            wanted,
            "{object}"
        );
    }

    #[test]
    fn each_kind_turns_by_a_quarter_exactly() {
        for (object, wanted) in [
            (
                "line: {from: [1, 2, 0], to: [3, 4, 0]}",
                "line: {from: [-2, 1, 0], to: [-4, 3, 0]}",
            ),
            (
                "polyline: {points: [[0, 0, 0], {point: [4, 0, 0], bulge: 0.5, start_width: 1, end_width: 0}, [4, 4, 0]]}",
                "polyline: {points: [[0, 0, 0], {point: [0, 4, 0], bulge: 0.5, start_width: 1, end_width: 0}, [-4, 4, 0]]}",
            ),
            (
                "arc: {center: [5, 0, 0], radius: 2, start_angle: 0, end_angle: 90}",
                "arc: {center: [0, 5, 0], radius: 2, start_angle: 90, end_angle: 180}",
            ),
            (
                "circle: {center: [5, 0, 0], radius: 2}",
                "circle: {center: [0, 5, 0], radius: 2}",
            ),
            (
                "ellipse: {center: [5, 0, 0], major_axis: [2, 0, 0], ratio: 0.5, start_angle: 10, end_angle: 100}",
                "ellipse: {center: [0, 5, 0], major_axis: [0, 2, 0], ratio: 0.5, start_angle: 10, end_angle: 100}",
            ),
            (
                "spline: {control_points: [[0, 0, 0], [1, 1, 0], [2, 0, 0], [3, 1, 0]]}",
                "spline: {control_points: [[0, 0, 0], [-1, 1, 0], [0, 2, 0], [-1, 3, 0]]}",
            ),
            (
                "hatch: {pattern: GRID, angle: 30, boundary: [{polyline: {points: [[1, 0, 0], [2, 0, 0], [2, 2, 0]]}}, {edges: [{line: {from: [5, 0, 0], to: [7, 0, 0]}}, {arc: {center: [6, 0, 0], radius: 1, start_angle: 0, end_angle: 180}}]}]}",
                "hatch: {pattern: GRID, angle: 120, boundary: [{polyline: {points: [[0, 1, 0], [0, 2, 0], [-2, 2, 0]]}}, {edges: [{line: {from: [0, 5, 0], to: [0, 7, 0]}}, {arc: {center: [0, 6, 0], radius: 1, start_angle: 90, end_angle: 270}}]}]}",
            ),
            (
                "text: {contents: A, position: [1, 2, 0], height: 2.5, rotation: 30}",
                "text: {contents: A, position: [-2, 1, 0], height: 2.5, rotation: 120}",
            ),
            (
                "mtext: {contents: A, position: [1, 2, 0], height: 2.5}",
                "mtext: {contents: A, position: [-2, 1, 0], height: 2.5, rotation: 90}",
            ),
            (
                "donut: {center: [1, 0, 0], outside_diameter: 2}",
                "donut: {center: [0, 1, 0], outside_diameter: 2}",
            ),
            (
                "solid: {corners: [[1, 0, 0], [2, 0, 0], [1, 1, 0]]}",
                "solid: {corners: [[0, 1, 0], [0, 2, 0], [-1, 1, 0]]}",
            ),
            (
                "insert: {block: B, position: [1, 2, 0], rotation: 300, mirrored: true}",
                "insert: {block: B, position: [-2, 1, 0], rotation: 30, mirrored: true}",
            ),
            (
                "mleader: {leaders: [[[1, 0, 0], [5, 5, 0]]], content: {text: T}}",
                "mleader: {leaders: [[[0, 1, 0], [-5, 5, 0]]], content: {text: T}}",
            ),
            (
                "mleader: {leaders: [[[1, 0, 0], [5, 5, 0]]], content: {block: B, rotation: 10}}",
                "mleader: {leaders: [[[0, 1, 0], [-5, 5, 0]]], content: {block: B, rotation: 100}}",
            ),
            (
                "leader: {points: [[1, 0, 0], [5, 5, 0]], annotation: {text: N}}",
                "leader: {points: [[0, 1, 0], [-5, 5, 0]], annotation: {text: N}}",
            ),
            (
                "dimension: {kind: linear, angle: 30, points: [[1, 0, 0], [10, 0, 0]], location: [5, -3, 0]}",
                "dimension: {kind: linear, angle: 120, points: [[0, 1, 0], [0, 10, 0]], location: [3, 5, 0]}",
            ),
            // Written without its angle, it measures across the sheet: turned,
            // it measures along its turned direction.
            (
                "dimension: {kind: linear, points: [[1, 0, 0], [10, 0, 0]], location: [5, -3, 0]}",
                "dimension: {kind: linear, points: [[0, 1, 0], [0, 10, 0]], location: [3, 5, 0], angle: 90}",
            ),
            (
                "dimension: {kind: aligned, points: [[1, 0, 0], [10, 0, 0]], location: [5, -3, 0]}",
                "dimension: {kind: aligned, points: [[0, 1, 0], [0, 10, 0]], location: [3, 5, 0]}",
            ),
            (
                "dimension: {kind: angular, lines: [[[1, 0, 0], [5, 0, 0]], [[1, 0, 0], [1, 5, 0]]], location: [2, 2, 0]}",
                "dimension: {kind: angular, lines: [[[0, 1, 0], [0, 5, 0]], [[0, 1, 0], [-5, 1, 0]]], location: [-2, 2, 0]}",
            ),
        ] {
            lands(object, turn(), wanted);
        }
        // About a point of its own, three quarters clockwise is a quarter
        // anticlockwise, with no noise of a sine.
        lands(
            "line: {from: [0.3, 0.7, 2], to: [0.1, 0.2, 2]}",
            Transform::rotate([0.1, 0.2, 0.0], -270.0),
            "line: {from: [-0.4, 0.4, 2], to: [0.1, 0.2, 2]}",
        );
    }

    #[test]
    fn each_kind_scales_its_points_and_lengths() {
        for (object, wanted) in [
            (
                "line: {from: [1, 2, 0], to: [3, 4, 0]}",
                "line: {from: [2, 4, 0], to: [6, 8, 0]}",
            ),
            (
                "polyline: {points: [[0, 0, 0], {point: [4, 0, 0], bulge: 0.5, start_width: 1, end_width: 0}, [4, 4, 0]]}",
                "polyline: {points: [[0, 0, 0], {point: [8, 0, 0], bulge: 0.5, start_width: 2, end_width: 0}, [8, 8, 0]]}",
            ),
            (
                "polyline: {points: [[1, 0, 0], [2, 0, 0]], width: 0.5}",
                "polyline: {points: [[2, 0, 0], [4, 0, 0]], width: 1}",
            ),
            (
                "arc: {center: [5, 0, 0], radius: 2, start_angle: 0, end_angle: 90}",
                "arc: {center: [10, 0, 0], radius: 4, start_angle: 0, end_angle: 90}",
            ),
            (
                "circle: {center: [5, 0, 0], radius: 2 mm}",
                "circle: {center: [10, 0, 0], radius: 4}",
            ),
            (
                "ellipse: {center: [5, 0, 0], major_axis: [2, 0, 0], ratio: 0.5}",
                "ellipse: {center: [10, 0, 0], major_axis: [4, 0, 0], ratio: 0.5}",
            ),
            (
                "hatch: {pattern: GRID, scale: 1.5, boundary: [{edges: [{line: {from: [5, 0, 0], to: [7, 0, 0]}}, {arc: {center: [6, 0, 0], radius: 1, start_angle: 0, end_angle: 180}}]}]}",
                "hatch: {pattern: GRID, scale: 3, boundary: [{edges: [{line: {from: [10, 0, 0], to: [14, 0, 0]}}, {arc: {center: [12, 0, 0], radius: 2, start_angle: 0, end_angle: 180}}]}]}",
            ),
            (
                "hatch: {pattern: SOLID, boundary: [{polyline: {points: [[1, 0, 0], [2, 0, 0], [2, 2, 0]]}}]}",
                "hatch: {pattern: SOLID, boundary: [{polyline: {points: [[2, 0, 0], [4, 0, 0], [4, 4, 0]]}}]}",
            ),
            (
                "text: {contents: A, position: [1, 2, 0], height: 2.5, rotation: 30}",
                "text: {contents: A, position: [2, 4, 0], height: 5, rotation: 30}",
            ),
            (
                "mtext: {contents: A, position: [1, 2, 0], height: 2.5, width: 10}",
                "mtext: {contents: A, position: [2, 4, 0], height: 5, width: 20}",
            ),
            (
                "donut: {center: [1, 0, 0], outside_diameter: 2, inside_diameter: 1}",
                "donut: {center: [2, 0, 0], outside_diameter: 4, inside_diameter: 2}",
            ),
            (
                "insert: {block: B, position: [1, 2, 0], scale: 1.5}",
                "insert: {block: B, position: [2, 4, 0], scale: 3}",
            ),
            (
                "insert: {block: B, position: [1, 2, 0]}",
                "insert: {block: B, position: [2, 4, 0], scale: 2}",
            ),
            (
                "mleader: {leaders: [[[1, 0, 0], [5, 5, 0]]], content: {block: B}}",
                "mleader: {leaders: [[[2, 0, 0], [10, 10, 0]]], content: {block: B, scale: 2}}",
            ),
            (
                "dimension: {kind: radius, center: [1, 1, 0], point: [3, 1, 0], location: [5, 5, 0]}",
                "dimension: {kind: radius, center: [2, 2, 0], point: [6, 2, 0], location: [10, 10, 0]}",
            ),
        ] {
            lands(object, doubled(), wanted);
        }
        // A scaling scales Z about the base point's Z, as AutoCAD's does.
        lands(
            "line: {from: [1, 2, 3], to: [1, 2, 1]}",
            Transform::scale([1.0, 2.0, 1.0], 3.0),
            "line: {from: [1, 2, 7], to: [1, 2, 1]}",
        );
    }

    #[test]
    fn a_leaders_lengths_from_its_style_are_written_as_its_own_when_scaled() {
        let resolved = Resolved {
            lengths: vec![
                ("arrow_size", 2.5),
                ("landing", 8.0),
                ("gap", 2.0),
                ("content.height", 3.0),
                ("annotation.height", 2.5),
            ],
            size: None,
        };
        assert_eq!(
            transformed(
                "mleader: {leaders: [[[1, 0, 0], [5, 5, 0]]], content: {text: T}, gap: 1}",
                doubled(),
                &resolved
            )
            .unwrap(),
            "mleader: {leaders: [[[2, 0, 0], [10, 10, 0]]], content: {text: T, height: 6}, gap: 2, arrow_size: 5, landing: 16}"
        );
        assert_eq!(
            transformed(
                "leader: {points: [[1, 0, 0], [5, 5, 0]], annotation: {text: N}}",
                doubled(),
                &resolved
            )
            .unwrap(),
            "leader: {points: [[2, 0, 0], [10, 10, 0]], annotation: {text: N, height: 5}, arrow_size: 5, gap: 4}"
        );
        // Turned, it writes none of them.
        assert_eq!(
            transformed(
                "leader: {points: [[1, 0, 0], [5, 5, 0]]}",
                turn(),
                &resolved
            )
            .unwrap(),
            "leader: {points: [[0, 1, 0], [-5, 5, 0]]}"
        );
    }

    #[test]
    fn each_kind_mirrors_with_its_text_reading_forwards() {
        for (object, wanted) in [
            (
                "line: {from: [1, 2, 0], to: [3, 4, 0]}",
                "line: {from: [19, 2, 0], to: [17, 4, 0]}",
            ),
            (
                "polyline: {points: [[0, 0, 0], {point: [4, 0, 0], bulge: 0.5, start_width: 1, end_width: 0}, {point: [4, 4, 0], arc_angle: -90}, [8, 4, 0]]}",
                "polyline: {points: [[20, 0, 0], {point: [16, 0, 0], bulge: -0.5, start_width: 1, end_width: 0}, {point: [16, 4, 0], arc_angle: 90}, [12, 4, 0]]}",
            ),
            (
                "arc: {center: [5, 0, 0], radius: 2, start_angle: 0, end_angle: 90}",
                "arc: {center: [15, 0, 0], radius: 2, start_angle: 90, end_angle: 180}",
            ),
            (
                "ellipse: {center: [5, 0, 0], major_axis: [2, 0, 0], ratio: 0.5, start_angle: 10, end_angle: 100}",
                "ellipse: {center: [15, 0, 0], major_axis: [-2, 0, 0], ratio: 0.5, start_angle: 260, end_angle: 350}",
            ),
            (
                "spline: {fit_points: [[0, 0, 0], [1, 1, 0], [2, 0, 0]]}",
                "spline: {fit_points: [[20, 0, 0], [19, 1, 0], [18, 0, 0]]}",
            ),
            // A hatch's pattern keeps its direction, as MIRRHATCH 0 keeps
            // it; its origin goes with its boundary.
            (
                "hatch: {pattern: GRID, angle: 30, boundary: [{edges: [{line: {from: [5, 0, 0], to: [7, 0, 0]}}, {arc: {center: [6, 0, 0], radius: 1, start_angle: 0, end_angle: 180}}]}]}",
                "hatch: {pattern: GRID, angle: 30, boundary: [{edges: [{line: {from: [15, 0, 0], to: [13, 0, 0]}}, {arc: {center: [14, 0, 0], radius: 1, start_angle: 0, end_angle: 180}}]}], origin: [20, 0, 0]}",
            ),
            (
                "text: {contents: A, position: [1, 2, 0], height: 2.5, rotation: 30, justify: right}",
                "text: {contents: A, position: [19, 2, 0], height: 2.5, rotation: 330, justify: right}",
            ),
            (
                "text: {contents: A, position: [1, 2, 0], height: 2.5}",
                "text: {contents: A, position: [19, 2, 0], height: 2.5}",
            ),
            (
                "mtext: {contents: A, position: [1, 2, 0], height: 2.5, rotation: 90}",
                "mtext: {contents: A, position: [19, 2, 0], height: 2.5, rotation: 90}",
            ),
            (
                "donut: {center: [1, 0, 0], outside_diameter: 2}",
                "donut: {center: [19, 0, 0], outside_diameter: 2}",
            ),
            (
                "solid: {corners: [[1, 0, 0], [2, 0, 0], [1, 1, 0]]}",
                "solid: {corners: [[19, 0, 0], [18, 0, 0], [19, 1, 0]]}",
            ),
            (
                "insert: {block: B, position: [1, 2, 0], rotation: 30, scale: 1.5}",
                "insert: {block: B, position: [19, 2, 0], rotation: 330, scale: 1.5, mirrored: true}",
            ),
            // Mirrored again, it is not mirrored, and its attributes keep
            // reading forwards.
            (
                "insert: {block: B, position: [1, 2, 0], rotation: 330, mirrored: true}",
                "insert: {block: B, position: [19, 2, 0], rotation: 30, readable_attributes: true}",
            ),
            (
                "insert: {block: B, position: [1, 2, 0], rotation: 120, mirrored: true}",
                "insert: {block: B, position: [19, 2, 0], rotation: 240, readable_attributes: true}",
            ),
            // And mirrored a third time, it is mirrored, which reads them
            // forwards of itself.
            (
                "insert: {block: B, position: [1, 2, 0], rotation: 240, readable_attributes: true}",
                "insert: {block: B, position: [19, 2, 0], rotation: 120, mirrored: true}",
            ),
            (
                "mleader: {leaders: [[[1, 0, 0], [5, 5, 0]]], content: {text: T}}",
                "mleader: {leaders: [[[19, 0, 0], [15, 5, 0]]], content: {text: T}}",
            ),
            (
                "leader: {points: [[1, 0, 0], [5, 5, 0]]}",
                "leader: {points: [[19, 0, 0], [15, 5, 0]]}",
            ),
            (
                "dimension: {kind: linear, angle: 30, points: [[1, 0, 0], [10, 0, 0]], location: [5, -3, 0]}",
                "dimension: {kind: linear, angle: 150, points: [[19, 0, 0], [10, 0, 0]], location: [15, -3, 0]}",
            ),
            (
                "dimension: {kind: linear, points: [[1, 0, 0], [10, 0, 0]], location: [5, -3, 0]}",
                "dimension: {kind: linear, points: [[19, 0, 0], [10, 0, 0]], location: [15, -3, 0]}",
            ),
            (
                "dimension: {kind: ordinate, feature: [4, 6, 0], axis: x, location: [4, 9, 0]}",
                "dimension: {kind: ordinate, feature: [16, 6, 0], axis: x, location: [16, 9, 0]}",
            ),
        ] {
            lands(object, mirrored(), wanted);
            // What it lands as is a sheet's object, as the sheet's reader
            // reads it: an insert needs its block's library, so it is read
            // in the operations' tests.
            if !wanted.starts_with("insert:") {
                let sheet = format!(
                    "units: mm
hatch_patterns:
  GRID: {{lines: [{{delta: [0, 1]}}]}}
objects:
  - {wanted}
"
                );
                if let Err(e) = crate::description::parse_sheet_description(&sheet) {
                    panic!("{wanted} does not read back: {}", e.message);
                }
            }
        }
        // Across a line that runs across the sheet, an insert turns a half
        // turn as well as mirroring, as AutoCAD's MIRROR leaves it.
        lands(
            "insert: {block: B, position: [1, 2, 0]}",
            Transform::mirror([0.0, 0.0, 0.0], [1.0, 0.0, 0.0]).unwrap(),
            "insert: {block: B, position: [1, -2, 0], rotation: 180, mirrored: true}",
        );
        // Across a line at 45 degrees, exactly, a text reading up.
        lands(
            "text: {contents: A, position: [0.3, 0.1, 0], height: 1}",
            Transform::mirror([0.0, 0.0, 0.0], [0.7, 0.7, 0.0]).unwrap(),
            "text: {contents: A, position: [0.1, 0.3, 0], height: 1, rotation: 90}",
        );
    }

    /// The sector a dimension's record measures once the sheet's reader
    /// reads it: its first ray's angle and its sweep.
    fn sector_of(object: &str) -> (f64, f64) {
        let sheet = format!("units: mm\nobjects:\n  - {object}\n");
        let sheet = crate::description::parse_sheet_description(&sheet).unwrap();
        let crate::model::Object::Dimension(d) = &sheet.objects[0] else {
            panic!("a dimension")
        };
        let s = d.sector().unwrap();
        (s.start, s.sweep)
    }

    #[test]
    fn a_mirrored_angular_dimension_measures_the_mirrored_sector() {
        let across = Transform::mirror([0.0; 3], [1.0, 0.0, 0.0]).unwrap();
        // Its location on its first ray: the quarter from 0 to 90.
        let three = "dimension: {kind: three_point_angular, center: [0, 0, 0], points: [[10, 0, 0], [0, 10, 0]], location: [5, 0, 0]}";
        assert_eq!(sector_of(three), (0.0, 90.0));
        let landed = transformed(three, across, &Resolved::default()).unwrap();
        // Mirrored across the X axis it measures the quarter from 270 to 360,
        // not the three quarters from 0 to 270.
        let (start, sweep) = sector_of(&landed);
        assert!(
            (start - 270.0).abs() < 1e-9 && (sweep - 90.0).abs() < 1e-9,
            "{landed}: {start} {sweep}"
        );
        // Two lines, the location on one of them, mirrored up the sheet.
        let lines = "dimension: {kind: angular, lines: [[[0, 0, 0], [10, 0, 0]], [[0, 0, 0], [10, 10, 0]]], location: [5, 0, 0]}";
        assert_eq!(sector_of(lines), (0.0, 45.0));
        let landed = transformed(lines, mirrored(), &Resolved::default()).unwrap();
        let (start, sweep) = sector_of(&landed);
        assert!(
            (start - 135.0).abs() < 1e-9 && (sweep - 45.0).abs() < 1e-9,
            "{landed}: {start} {sweep}"
        );
        // Turned an odd angle, the location on a ray stays in its sector.
        let turned = Transform::rotate([3.0, 1.0, 0.0], 33.0);
        let landed = transformed(three, turned, &Resolved::default()).unwrap();
        let (_, sweep) = sector_of(&landed);
        assert!((sweep - 90.0).abs() < 1e-9, "{landed}: {sweep}");
        // A location clear of its rays is left as written, moved.
        lands(
            "dimension: {kind: three_point_angular, center: [0, 0, 0], points: [[10, 0, 0], [0, 10, 0]], location: [3, 4, 0]}",
            across,
            "dimension: {kind: three_point_angular, center: [0, 0, 0], points: [[10, 0, 0], [0, -10, 0]], location: [3, -4, 0]}",
        );
    }

    #[test]
    fn an_ordinate_dimension_measures_along_its_turned_or_mirrored_axis() {
        let ordinate = "dimension: {kind: ordinate, feature: [4, 6, 0], axis: x, location: [4, 9, 0]}";
        // A quarter turn either way measures along Y; a half turn along X.
        lands(
            ordinate,
            turn(),
            "dimension: {kind: ordinate, feature: [-6, 4, 0], axis: y, location: [-9, 4, 0]}",
        );
        lands(
            ordinate,
            Transform::rotate([0.0; 3], -90.0),
            "dimension: {kind: ordinate, feature: [6, -4, 0], axis: y, location: [9, -4, 0]}",
        );
        lands(
            ordinate,
            Transform::rotate([0.0; 3], 180.0),
            "dimension: {kind: ordinate, feature: [-4, -6, 0], axis: x, location: [-4, -9, 0]}",
        );
        // Mirrored across a line at 45 degrees it measures along the other
        // axis; across or up the sheet, along its own.
        lands(
            ordinate,
            Transform::mirror([0.0; 3], [1.0, 1.0, 0.0]).unwrap(),
            "dimension: {kind: ordinate, feature: [6, 4, 0], axis: y, location: [9, 4, 0]}",
        );
        lands(
            ordinate,
            Transform::mirror([0.0; 3], [1.0, 0.0, 0.0]).unwrap(),
            "dimension: {kind: ordinate, feature: [4, -6, 0], axis: x, location: [4, -9, 0]}",
        );
        // Each lands as a sheet's reader reads it, its leader along its axis.
        for t in [turn(), Transform::mirror([0.0; 3], [1.0, 1.0, 0.0]).unwrap()] {
            let landed = transformed(ordinate, t, &Resolved::default()).unwrap();
            let sheet = format!("units: mm\nobjects:\n  - {landed}\n");
            crate::description::parse_sheet_description(&sheet).unwrap();
        }
        // A turn or a mirror that leaves it measuring along neither axis is
        // refused.
        for t in [
            Transform::rotate([0.0; 3], 30.0),
            Transform::mirror([0.0; 3], [2.0, 1.0, 0.0]).unwrap(),
        ] {
            let e = transformed(ordinate, t, &Resolved::default()).unwrap_err();
            assert_eq!(e.code, ErrorCode::BadValue);
            assert!(e.message.contains("ordinate dimension"), "{}", e.message);
        }
        // A linear dimension turns by any angle.
        assert!(transformed(
            "dimension: {kind: linear, points: [[1, 0, 0], [10, 0, 0]], location: [5, -3, 0]}",
            Transform::rotate([0.0; 3], 30.0),
            &Resolved::default()
        )
        .is_ok());
    }

    #[test]
    fn a_table_mirrors_upright_by_its_box_and_is_not_turned_or_scaled() {
        let table =
            "table: {position: [0, 10, 0], columns: [4, 6], rows: [2, 3], cells: [[a, b], [c, d]]}";
        let sized = Resolved {
            lengths: Vec::new(),
            size: Some([10.0, 5.0]),
        };
        assert_eq!(
            transformed(table, mirrored(), &sized).unwrap(),
            "table: {position: [10, 10, 0], columns: [4, 6], rows: [2, 3], cells: [[a, b], [c, d]]}"
        );
        let across = Transform::mirror([0.0, 0.0, 0.0], [1.0, 0.0, 0.0]).unwrap();
        assert_eq!(
            transformed(table, across, &sized).unwrap(),
            "table: {position: [0, -5, 0], columns: [4, 6], rows: [2, 3], cells: [[a, b], [c, d]]}"
        );
        for (t, words) in [
            (turn(), "no rotation of its own"),
            (doubled(), "is not scaled"),
            (
                Transform::mirror([0.0; 3], [1.0, 2.0, 0.0]).unwrap(),
                "only across a line that runs across",
            ),
        ] {
            let e = transformed(table, t, &sized).unwrap_err();
            assert_eq!(e.code, ErrorCode::BadValue);
            assert!(e.message.contains(words), "{}", e.message);
        }
    }

    #[test]
    fn what_the_description_cannot_hold_is_refused() {
        let e = transformed(
            "mleader: {leaders: [[[1, 0, 0], [5, 5, 0]]], content: {block: B}}",
            mirrored(),
            &Resolved::default(),
        )
        .unwrap_err();
        assert!(e.message.contains("block content"), "{}", e.message);
        let e = transformed(
            "viewport: {center: [5, 5, 0], width: 4, height: 3, view_center: [0, 0, 0], scale: 1}",
            turn(),
            &Resolved::default(),
        )
        .unwrap_err();
        assert!(e.message.contains("viewport"), "{}", e.message);
    }

    #[test]
    fn a_quarter_turn_and_an_upright_mirror_are_exact() {
        let quarter = Transform::rotate([1.0, 1.0, 0.0], 90.0);
        assert_eq!(quarter.point([3.0, 1.0, 5.0]), [1.0, 3.0, 5.0]);
        let half = Transform::rotate([0.0, 0.0, 0.0], -180.0);
        assert_eq!(half.point([0.3, 0.7, 0.0]), [-0.3, -0.7, 0.0]);
        for (b, p, landed) in [
            // Up the sheet, across it and at 45 degrees either way.
            ([2.0, 7.0, 0.0], [5.0, 1.0, 0.0], [-1.0, 1.0, 0.0]),
            ([9.0, 2.0, 0.0], [5.0, 3.0, 4.0], [5.0, 1.0, 4.0]),
            ([3.0, 3.0, 0.0], [5.0, 1.0, 0.0], [1.0, 5.0, 0.0]),
            ([3.0, 1.0, 0.0], [4.0, 4.0, 0.0], [0.0, 0.0, 0.0]),
        ] {
            let mirror = Transform::mirror([2.0, 2.0, 0.0], b).unwrap();
            assert_eq!(mirror.point(p), landed, "across the line to {b:?}");
        }
        assert_eq!(
            Transform::mirror([2.0, 2.0, 0.0], [3.0, 3.0, 0.0])
                .unwrap()
                .how(),
            How::Mirror(45.0)
        );
        assert!(Transform::mirror([2.0, 2.0, 0.0], [2.0, 2.0, 9.0]).is_none());
    }

    #[test]
    fn a_scaling_scales_about_the_points_z_too() {
        let t = Transform::scale([1.0, 1.0, 1.0], 2.0);
        assert_eq!(t.point([2.0, 3.0, 1.0]), [3.0, 5.0, 1.0]);
        assert_eq!(t.point([1.0, 1.0, 2.0]), [1.0, 1.0, 3.0]);
        assert_eq!(t.angle(30.0), 30.0);
    }

    #[test]
    fn mirrored_text_reads_forwards() {
        let up = Transform::mirror([0.0; 3], [0.0, 1.0, 0.0]).unwrap();
        assert_eq!(up.angle(30.0), 150.0);
        assert_eq!(up.text_angle(30.0), 330.0);
        assert_eq!(up.text_angle(0.0), 0.0);
        let across = Transform::mirror([0.0; 3], [1.0, 0.0, 0.0]).unwrap();
        assert_eq!(across.text_angle(0.0), 0.0);
        assert_eq!(across.text_angle(30.0), 330.0);
        // An arc runs the other way: its ends swap.
        assert_eq!(up.arc_angles(0.0, 90.0), (90.0, 180.0));
    }
}
