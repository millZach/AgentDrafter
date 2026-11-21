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
        let at = At {
            t,
            units,
            resolved,
        };
        let changed = at.object(&kind, keys, &object_path)?;
        self.touch(list, index, operation, Touch::Keys(changed));
        Ok(())
    }
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
                ("dimension", "angle") => {
                    self.angle(&mut entry.value, &field, &|a| t.angle(a))?
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
            if did {
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
                    let was = keys
                        .iter()
                        .position(|e| e.key == "mirrored")
                        .map(|i| (i, matches!(&keys[i].value.kind, Kind::Scalar(s) if s == "true")));
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
            "mleader" | "leader" if scaled => {
                for key in ["arrow_size", "gap", "landing"] {
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
        if matches!(kind, "arc" | "ellipse") && (!matches!(t.how, How::Scale(_))) {
            if self.arc_angles(kind, keys, path)? {
                changed.push("start_angle".to_owned());
                changed.push("end_angle".to_owned());
            }
        }
        Ok(changed)
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
                format!("the record's value here is not a number to {}", self.t.verb()),
            )
            .at_field(field));
        };
        text.trim().parse::<f64>().map_err(|_| {
            Error::new(
                ErrorCode::BadNumber,
                format!("the record's value `{text}` is not a number to {}", self.t.verb()),
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
                format!("the record's length here is not a length to {}", self.t.verb()),
            )
            .at_field(field));
        };
        let value = match parse_length(text) {
            Ok((value, None)) => value,
            Ok((value, Some(own))) => own.convert(value, self.units),
            Err(_) => {
                return Err(Error::new(
                    ErrorCode::BadNumber,
                    format!("the record's length `{text}` is not a length to {}", self.t.verb()),
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
                format!("the record's angle here is not a number of degrees to {}", self.t.verb()),
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
                                if edge_kind.key == "arc" && !matches!(self.t.how, How::Scale(_))
                                {
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
        let corners = [[x, y], [x + width, y], [x, y - height], [x + width, y - height]]
            .map(|[cx, cy]| self.t.point([cx, cy, z]));
        let left = corners.iter().map(|c| c[0]).fold(f64::INFINITY, f64::min);
        let top = corners.iter().map(|c| c[1]).fold(f64::NEG_INFINITY, f64::max);
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
            Transform::mirror([2.0, 2.0, 0.0], [3.0, 3.0, 0.0]).unwrap().how(),
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
