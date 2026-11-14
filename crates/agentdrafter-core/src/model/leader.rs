//! Leaders: AutoCAD's MULTILEADER with its MLEADERSTYLE, and the older
//! LEADER.
//!
//! A multileader points at something with one or more leader lines, each
//! from its arrow's point through any bends to where it meets the landing,
//! a short horizontal line, the dogleg, beyond which its content, multiline
//! text, sits. Every leader line of one multileader meets the landing at
//! the same point. The landing runs right when the first leader line's
//! last segment runs right or straight up or down, and left when it runs
//! left, and the text sits beyond it, a gap away, as AutoCAD 2026 laid out
//! made-up multileaders:
//!
//! - On the right, the text's box is attached at its top left corner, and
//!   on the left at its top right, at the landing's end and the gap beyond.
//! - Up, the landing meets the text where its attachment says: the top of
//!   the box, the middle of the top line, the middle of the text, the
//!   middle of the bottom line or the bottom of the box. A line here
//!   reaches from the text's height above its baseline down to the lowest
//!   point the last line draws, which AutoCAD measures for the top line
//!   and the bottom line alike; the box is the multiline text's own.
//!
//! A multileader style is what a multileader draws with unless it gives its
//! own: the text style and height, the arrowhead and its size, the
//! landing's length and its gap to the text, the attachment, and the pen of
//! the leader lines. AutoCAD's own Standard style is in every drawing.
//!
//! The older leader is one leader line, arrow first, with an optional
//! multiline text annotation beyond its end, as AutoCAD's LEADER and
//! QLEADER draw one. Its last segment, when it is steeper than 15° from
//! the horizontal and there is an annotation, ends in a hook: a horizontal
//! segment as long as the arrow, which AutoCAD adds as it draws, towards
//! the annotation. The annotation sits a gap beyond the end, attached at
//! the middle of its left side, or of its right side when the leader runs
//! left.
//!
//! An arrow is placed at a point: pointing one at an object by its ID or
//! name waits for object snaps (#74).

use serde::{Deserialize, Serialize};

use super::{layer_zero, ObjectColor, ObjectLineweight, Point3, Properties};
use crate::text::{Attachment, MText, STANDARD};
use crate::units::Unit;

/// The multileader style every drawing has, which a multileader draws with
/// unless it names another.
pub const STANDARD_MLEADER_STYLE: &str = STANDARD;

/// How far from the horizontal, in degrees, an older leader's last segment
/// may run before AutoCAD hooks it.
pub const HOOK_ANGLE: f64 = 15.0;

/// The arrowheads a leader draws, of AutoCAD's named arrowheads: those the
/// pipeline draws exactly as AutoCAD 2026 drew them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Arrowhead {
    /// A filled triangle, a sixth of its length wide on each side of the
    /// line: AutoCAD's default.
    #[default]
    ClosedFilled,
    /// A filled dot, as wide as the arrow is long.
    Dot,
    /// Two lines, the triangle's sides, open at its back.
    Open,
    /// No arrowhead.
    None,
}

impl Arrowhead {
    /// Every arrowhead, in the order messages list them.
    pub const ALL: [Arrowhead; 4] = [
        Arrowhead::ClosedFilled,
        Arrowhead::Dot,
        Arrowhead::Open,
        Arrowhead::None,
    ];

    /// The name a sheet description writes, such as `closed_filled`.
    pub const fn name(self) -> &'static str {
        match self {
            Arrowhead::ClosedFilled => "closed_filled",
            Arrowhead::Dot => "dot",
            Arrowhead::Open => "open",
            Arrowhead::None => "none",
        }
    }

    /// The block AutoCAD draws the arrowhead from, as it names it; none for
    /// the closed filled arrowhead, which AutoCAD draws without one.
    pub const fn block(self) -> Option<&'static str> {
        match self {
            Arrowhead::ClosedFilled => None,
            Arrowhead::Dot => Some("_Dot"),
            Arrowhead::Open => Some("_Open"),
            Arrowhead::None => Some("_None"),
        }
    }

    /// The arrowhead AutoCAD draws from the block `name`, in any case;
    /// none for a block that is not one of these arrowheads'.
    pub fn of_block(name: &str) -> Option<Arrowhead> {
        if name.eq_ignore_ascii_case("_ClosedFilled") {
            return Some(Arrowhead::ClosedFilled);
        }
        Arrowhead::ALL
            .into_iter()
            .find(|a| a.block().is_some_and(|b| b.eq_ignore_ascii_case(name)))
    }

    /// Whether the leader line starts an arrow's length back from its
    /// point, behind the arrowhead, as AutoCAD 2026 drew it: for every
    /// arrowhead but none.
    pub const fn trims_line(self) -> bool {
        !matches!(self, Arrowhead::None)
    }
}

/// Where a multileader's landing meets its text, up the text: AutoCAD's
/// attachments for text on the left and the right of a horizontal landing,
/// but those that underline the text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaderAttachment {
    TopOfTopLine,
    /// AutoCAD's default.
    #[default]
    MiddleOfTopLine,
    MiddleOfText,
    MiddleOfBottomLine,
    BottomOfBottomLine,
}

impl LeaderAttachment {
    /// Every attachment, in AutoCAD's order, from 0.
    pub const ALL: [LeaderAttachment; 5] = [
        LeaderAttachment::TopOfTopLine,
        LeaderAttachment::MiddleOfTopLine,
        LeaderAttachment::MiddleOfText,
        LeaderAttachment::MiddleOfBottomLine,
        LeaderAttachment::BottomOfBottomLine,
    ];

    /// The name a sheet description writes, such as `middle_of_top_line`.
    pub const fn name(self) -> &'static str {
        match self {
            LeaderAttachment::TopOfTopLine => "top_of_top_line",
            LeaderAttachment::MiddleOfTopLine => "middle_of_top_line",
            LeaderAttachment::MiddleOfText => "middle_of_text",
            LeaderAttachment::MiddleOfBottomLine => "middle_of_bottom_line",
            LeaderAttachment::BottomOfBottomLine => "bottom_of_bottom_line",
        }
    }

    /// AutoCAD's number for it.
    pub fn code(self) -> i16 {
        LeaderAttachment::ALL
            .iter()
            .position(|a| *a == self)
            .unwrap_or(0) as i16
    }

    /// The attachment AutoCAD numbers `code`, when it is one of these.
    pub fn of_code(code: i16) -> Option<LeaderAttachment> {
        usize::try_from(code)
            .ok()
            .and_then(|i| LeaderAttachment::ALL.get(i).copied())
    }

    /// How far below the top of the text's box the landing meets it, for
    /// text `height` high in a box `measured`, as AutoCAD 2026 laid out
    /// made-up multileaders in SHX and TrueType fonts: a line counts from
    /// the text's height above its baseline down to the lowest point the
    /// last line draws.
    pub fn below_top(self, height: f64, measured: MTextBox) -> f64 {
        let line = height + measured.last_descent;
        match self {
            LeaderAttachment::TopOfTopLine => 0.0,
            LeaderAttachment::MiddleOfTopLine => line / 2.0,
            LeaderAttachment::MiddleOfText => measured.height / 2.0,
            LeaderAttachment::MiddleOfBottomLine => measured.height - line / 2.0,
            LeaderAttachment::BottomOfBottomLine => measured.height,
        }
    }
}

/// Multiline text's box as the pipeline lays it out, for placing a
/// leader's text: its width, its height and how far its last line draws
/// below its baseline.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MTextBox {
    pub width: f64,
    pub height: f64,
    pub last_descent: f64,
}

/// What a multileader draws with unless it gives its own: AutoCAD's
/// MLEADERSTYLE.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MLeaderStyle {
    pub name: String,
    /// The text style its text draws with.
    pub text_style: String,
    /// The height of its text's capitals, more than 0.
    pub text_height: f64,
    pub arrowhead: Arrowhead,
    /// How long the arrowhead is, more than 0.
    pub arrow_size: f64,
    /// How long the landing is, 0 or more: 0 for none.
    pub landing: f64,
    /// The space between the landing's end and the text, 0 or more.
    pub landing_gap: f64,
    /// Where the landing meets the text.
    pub attachment: LeaderAttachment,
    /// The colour the leader lines, landing and arrowheads draw in:
    /// `ByBlock`, the multileader's own, unless the style gives another.
    pub leader_color: ObjectColor,
    /// The lineweight they draw with: `ByBlock`, the multileader's own,
    /// unless the style gives another.
    pub leader_lineweight: ObjectLineweight,
}

impl MLeaderStyle {
    /// The `Standard` multileader style a sheet in `units` has unless it
    /// lists its own, as AutoCAD 2026's own templates have it: on a metric
    /// sheet text and arrows 4 mm, a landing 8 mm and a gap of 2 mm, and on
    /// an imperial one 0.18, 0.36 and 0.09 in; text in the `Standard` text
    /// style, a closed filled arrowhead, the landing at the middle of the
    /// top line, and the leader lines `ByBlock`.
    pub fn standard(units: Unit) -> Self {
        let (size, landing, gap) = if units.is_metric() {
            let mm = |x: f64| Unit::Mm.convert(x, units);
            (mm(4.0), mm(8.0), mm(2.0))
        } else {
            let inch = |x: f64| Unit::In.convert(x, units);
            (inch(0.18), inch(0.36), inch(0.09))
        };
        Self {
            name: STANDARD_MLEADER_STYLE.to_owned(),
            text_style: STANDARD.to_owned(),
            text_height: size,
            arrowhead: Arrowhead::ClosedFilled,
            arrow_size: size,
            landing,
            landing_gap: gap,
            attachment: LeaderAttachment::MiddleOfTopLine,
            leader_color: ObjectColor::ByBlock,
            leader_lineweight: ObjectLineweight::ByBlock,
        }
    }
}

/// A multileader's content: multiline text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MLeaderContent {
    /// What the text says, exactly as it shows; lines are separated by a
    /// line break (`\n`).
    pub text: String,
    /// The text style it draws with, instead of its multileader style's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// The height of its capitals, instead of its multileader style's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    /// Where the landing meets it, instead of where its multileader
    /// style's does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<LeaderAttachment>,
}

/// One or more leader lines, a landing and multiline text: AutoCAD's
/// MULTILEADER.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MLeader {
    /// Each leader line's points, from its arrow's point to where it meets
    /// the landing, which is the same point for all of them.
    pub leaders: Vec<Vec<Point3>>,
    pub content: MLeaderContent,
    /// The landing's length, instead of its multileader style's: 0 for
    /// none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub landing: Option<f64>,
    /// The arrowhead, instead of its multileader style's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrowhead: Option<Arrowhead>,
    /// The arrowhead's length, instead of its multileader style's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrow_size: Option<f64>,
    /// The multileader style it draws with.
    #[serde(default = "standard_mleader_style")]
    pub mleader_style: String,
    #[serde(default = "layer_zero")]
    pub layer: String,
    #[serde(flatten)]
    pub properties: Properties,
}

fn standard_mleader_style() -> String {
    STANDARD_MLEADER_STYLE.to_owned()
}

/// A multileader laid out with its style: what it draws, and what AutoCAD
/// records of it.
#[derive(Clone, Debug, PartialEq)]
pub struct LaidMLeader {
    /// Each leader line, from its arrow's point to the landing.
    pub lines: Vec<Vec<Point3>>,
    /// Where every leader line meets the landing.
    pub connection: Point3,
    /// 1 when the landing runs right, -1 when it runs left.
    pub side: f64,
    /// The landing's length, 0 for none.
    pub landing: f64,
    pub landing_gap: f64,
    pub arrowhead: Arrowhead,
    pub arrow_size: f64,
    pub attachment: LeaderAttachment,
    /// The text, attached at its top left or top right corner.
    pub text: MText,
    /// The text's box, as the pipeline lays it out.
    pub measured: MTextBox,
}

impl LaidMLeader {
    /// The landing's far end.
    pub fn landing_end(&self) -> Point3 {
        Point3 {
            x: self.connection.x + self.side * self.landing,
            ..self.connection
        }
    }

    /// The point AutoCAD records as the content's base: the landing's end
    /// when the text is on the right, and a gap short of the text box's
    /// left side, level with the landing, when it is on the left, as
    /// AutoCAD 2026 wrote made-up multileaders laying them out.
    pub fn content_base(&self) -> Point3 {
        if self.side > 0.0 {
            self.landing_end()
        } else {
            Point3 {
                x: self.text.position.x - self.measured.width - self.landing_gap,
                ..self.connection
            }
        }
    }
}

impl MLeader {
    /// The side its landing runs to, from the first leader line's last
    /// segment: 1 for the right, when it runs right or straight up or
    /// down, and -1 for the left.
    pub fn side(&self) -> f64 {
        let last = self.leaders.first().and_then(|points| {
            let n = points.len();
            (n >= 2).then(|| points[n - 1].x - points[n - 2].x)
        });
        match last {
            Some(dx) if dx < 0.0 => -1.0,
            _ => 1.0,
        }
    }

    /// Where every leader line meets the landing: the first one's last
    /// point.
    pub fn connection(&self) -> Point3 {
        self.leaders
            .first()
            .and_then(|points| points.last())
            .copied()
            .unwrap_or(Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            })
    }

    /// Lays the multileader out with `style`, measuring its text's box
    /// with `measure`.
    pub fn laid_out(
        &self,
        style: &MLeaderStyle,
        measure: &dyn Fn(&MText) -> MTextBox,
    ) -> LaidMLeader {
        let side = self.side();
        let connection = self.connection();
        let landing = self.landing.unwrap_or(style.landing);
        let height = self.content.height.unwrap_or(style.text_height);
        let attachment = self.content.attachment.unwrap_or(style.attachment);
        let mut text = MText {
            contents: self.content.text.clone(),
            position: connection,
            height,
            width: 0.0,
            style: self
                .content
                .style
                .clone()
                .unwrap_or_else(|| style.text_style.clone()),
            width_factor: None,
            rotation: 0.0,
            justify: if side > 0.0 {
                Attachment::TopLeft
            } else {
                Attachment::TopRight
            },
            layer: self.layer.clone(),
            properties: self.properties.clone(),
        };
        let measured = measure(&text);
        text.position = Point3 {
            x: connection.x + side * (landing + style.landing_gap),
            y: connection.y + attachment.below_top(height, measured),
            z: connection.z,
        };
        LaidMLeader {
            lines: self.leaders.clone(),
            connection,
            side,
            landing,
            landing_gap: style.landing_gap,
            arrowhead: self.arrowhead.unwrap_or(style.arrowhead),
            arrow_size: self.arrow_size.unwrap_or(style.arrow_size),
            attachment,
            text,
            measured,
        }
    }
}

/// An older leader's annotation: multiline text beyond its end.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LeaderAnnotation {
    /// What the text says, exactly as it shows; lines are separated by a
    /// line break (`\n`).
    pub text: String,
    /// The text style it draws with.
    #[serde(default = "standard")]
    pub style: String,
    /// The height of its capitals, more than 0.
    pub height: f64,
}

fn standard() -> String {
    STANDARD.to_owned()
}

/// One leader line, arrow first, with an optional annotation: AutoCAD's
/// LEADER.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Leader {
    /// Its points, from its arrow's point to its end.
    pub points: Vec<Point3>,
    #[serde(default)]
    pub arrowhead: Arrowhead,
    /// How long the arrowhead is, more than 0; the hook is as long.
    pub arrow_size: f64,
    /// The space between the leader's end and its annotation, 0 or more.
    pub gap: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotation: Option<LeaderAnnotation>,
    #[serde(default = "layer_zero")]
    pub layer: String,
    #[serde(flatten)]
    pub properties: Properties,
}

impl Leader {
    /// The arrow size and the gap an older leader on a sheet in `units` has
    /// unless it gives its own, as AutoCAD 2026's dimension styles have
    /// them: 2.5 mm and 0.625 mm on a metric sheet, as ISO-25 has, and 0.18
    /// in and 0.09 in on an imperial one, as Standard has.
    pub fn default_sizes(units: Unit) -> (f64, f64) {
        if units.is_metric() {
            let mm = |x: f64| Unit::Mm.convert(x, units);
            (mm(2.5), mm(0.625))
        } else {
            let inch = |x: f64| Unit::In.convert(x, units);
            (inch(0.18), inch(0.09))
        }
    }

    /// The side its annotation sits on, from its last segment: 1 for the
    /// right, when it runs right or straight up or down, and -1 for the
    /// left.
    pub fn side(&self) -> f64 {
        let n = self.points.len();
        if n >= 2 && self.points[n - 1].x - self.points[n - 2].x < 0.0 {
            -1.0
        } else {
            1.0
        }
    }

    /// Whether AutoCAD hooks its end: it has an annotation and its last
    /// segment runs more than 15° from the horizontal.
    pub fn hooked(&self) -> bool {
        let n = self.points.len();
        if self.annotation.is_none() || n < 2 {
            return false;
        }
        let (a, b) = (self.points[n - 2], self.points[n - 1]);
        let (dx, dy) = ((b.x - a.x).abs(), (b.y - a.y).abs());
        if dx == 0.0 && dy == 0.0 {
            return false;
        }
        dy.atan2(dx).to_degrees() > HOOK_ANGLE + 1e-8
    }

    /// The path AutoCAD draws, arrow first: its points, with the hook's
    /// start put in before its end when it is hooked.
    pub fn path(&self) -> Vec<Point3> {
        let mut path = self.points.clone();
        if self.hooked() {
            if let Some(&end) = self.points.last() {
                let start = Point3 {
                    x: end.x - self.side() * self.arrow_size,
                    ..end
                };
                path.insert(path.len() - 1, start);
            }
        }
        path
    }

    /// Its annotation as multiline text, attached at the middle of its
    /// left side a gap beyond the leader's end, or of its right side when
    /// the leader runs left.
    pub fn annotation_text(&self) -> Option<MText> {
        let annotation = self.annotation.as_ref()?;
        let end = *self.points.last()?;
        let side = self.side();
        Some(MText {
            contents: annotation.text.clone(),
            position: Point3 {
                x: end.x + side * self.gap,
                ..end
            },
            height: annotation.height,
            width: 0.0,
            style: annotation.style.clone(),
            width_factor: None,
            rotation: 0.0,
            justify: if side > 0.0 {
                Attachment::MiddleLeft
            } else {
                Attachment::MiddleRight
            },
            layer: self.layer.clone(),
            properties: self.properties.clone(),
        })
    }
}

/// What an arrowhead draws at `tip`, its line coming from `from`, `size`
/// long: the triangles it fills and the lines it strokes, as AutoCAD
/// 2026's arrowhead blocks draw. None when the two points are one.
pub fn arrowhead_shapes(
    arrowhead: Arrowhead,
    tip: [f64; 2],
    from: [f64; 2],
    size: f64,
) -> Option<ArrowShapes> {
    let (dx, dy) = (tip[0] - from[0], tip[1] - from[1]);
    let length = dx.hypot(dy);
    if length == 0.0 {
        return None;
    }
    // Along: from the tip back towards the line; across: to its left.
    let (ux, uy) = (-dx / length, -dy / length);
    let at = |back: f64, side: f64| -> [f64; 2] {
        [
            tip[0] + size * (back * ux - side * uy),
            tip[1] + size * (back * uy + side * ux),
        ]
    };
    let wing = 1.0 / 6.0;
    let mut shapes = ArrowShapes::default();
    match arrowhead {
        Arrowhead::ClosedFilled => {
            shapes.fills.push(vec![tip, at(1.0, -wing), at(1.0, wing)]);
        }
        Arrowhead::Dot => {
            shapes.dot = Some((tip, size / 2.0));
            shapes.lines.push(vec![at(0.5, 0.0), at(1.0, 0.0)]);
        }
        Arrowhead::Open => {
            shapes.lines.push(vec![at(1.0, wing), tip, at(1.0, -wing)]);
            shapes.lines.push(vec![tip, at(1.0, 0.0)]);
        }
        Arrowhead::None => {}
    }
    Some(shapes)
}

/// What an arrowhead draws: filled triangles, a filled dot, its centre and
/// radius, and lines.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArrowShapes {
    pub fills: Vec<Vec<[f64; 2]>>,
    pub dot: Option<([f64; 2], f64)>,
    pub lines: Vec<Vec<[f64; 2]>>,
}

/// A leader line's points with its first segment trimmed back by the
/// arrowhead's length, as AutoCAD draws the line behind an arrowhead.
pub fn trimmed(points: &[[f64; 2]], arrowhead: Arrowhead, size: f64) -> Vec<[f64; 2]> {
    let mut points = points.to_vec();
    if arrowhead.trims_line() && points.len() >= 2 {
        let (tip, next) = (points[0], points[1]);
        let length = (next[0] - tip[0]).hypot(next[1] - tip[1]);
        if length > 0.0 && size < length {
            let t = size / length;
            points[0] = [
                tip[0] + (next[0] - tip[0]) * t,
                tip[1] + (next[1] - tip[1]) * t,
            ];
        }
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AutoCAD 2026 laid out made-up multileaders 2.5 high, their landing
    /// 5 long and 1 from the text, in romans.shx and arial.ttf, and put
    /// their text this far above the landing for each attachment: here
    /// each is the box the pipeline gives the same text in those fonts.
    #[test]
    fn the_landing_meets_the_text_where_autocad_put_it() {
        let h = 2.5;
        let cases: [(&str, MTextBox, [f64; 5]); 4] = [
            // romans, NOTE then ggg: the last line draws 5/6 below its baseline.
            (
                "romans NOTE, ggg",
                MTextBox {
                    width: 0.0,
                    height: 7.5,
                    last_descent: 2.5 / 3.0,
                },
                [0.0, 1.666_666_7, 3.75, 5.833_333_3, 7.5],
            ),
            (
                "romans ggg, NOTE",
                MTextBox {
                    width: 0.0,
                    height: 6.666_666_7,
                    last_descent: 0.0,
                },
                [0.0, 1.25, 3.333_333_3, 5.416_666_7, 6.666_666_7],
            ),
            // arial, the O of NOTE reaching 0.0444 above the capitals.
            (
                "arial NOTE, ggg",
                MTextBox {
                    width: 0.0,
                    height: 7.446,
                    last_descent: 0.735,
                },
                [0.0, 1.6175, 3.723, 5.8285, 7.446],
            ),
            (
                "arial NOTE",
                MTextBox {
                    width: 0.0,
                    height: 2.587,
                    last_descent: 0.0426,
                },
                [0.0, 1.2713, 1.2935, 1.3157, 2.587],
            ),
        ];
        for (what, measured, autocad) in cases {
            for (attachment, expected) in LeaderAttachment::ALL.into_iter().zip(autocad) {
                let got = attachment.below_top(h, measured);
                assert!(
                    (got - expected).abs() < 1e-4,
                    "{what} {}: {got} against AutoCAD's {expected}",
                    attachment.name()
                );
            }
        }
    }

    fn point(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    #[test]
    fn the_text_sits_beyond_the_landing_on_the_side_the_leader_runs_to() {
        let style = MLeaderStyle {
            landing: 5.0,
            landing_gap: 1.0,
            text_height: 2.5,
            attachment: LeaderAttachment::TopOfTopLine,
            ..MLeaderStyle::standard(Unit::Mm)
        };
        let mut mleader = MLeader {
            leaders: vec![vec![point(0.0, 0.0), point(20.0, 10.0)]],
            content: MLeaderContent {
                text: "NOTE".to_owned(),
                style: None,
                height: None,
                attachment: None,
            },
            landing: None,
            arrowhead: None,
            arrow_size: None,
            mleader_style: STANDARD_MLEADER_STYLE.to_owned(),
            layer: "0".to_owned(),
            properties: Properties::default(),
        };
        let measure = |_: &MText| MTextBox {
            width: 9.6905,
            height: 2.5,
            last_descent: 0.0,
        };
        // As AutoCAD 2026 placed NOTE in romans: at (26, 10), top left.
        let laid = mleader.laid_out(&style, &measure);
        assert_eq!(laid.text.position, point(26.0, 10.0));
        assert_eq!(laid.text.justify, Attachment::TopLeft);
        assert_eq!(laid.content_base(), point(25.0, 10.0));
        // And to the left: at (-26, 10), top right, its base a gap short
        // of the text's left side.
        mleader.leaders = vec![vec![point(0.0, 0.0), point(-20.0, 10.0)]];
        let laid = mleader.laid_out(&style, &measure);
        assert_eq!(laid.text.position, point(-26.0, 10.0));
        assert_eq!(laid.text.justify, Attachment::TopRight);
        assert!((laid.content_base().x + 36.6905).abs() < 1e-9);
    }

    #[test]
    fn a_closed_filled_arrowhead_is_the_triangle_autocad_exploded() {
        // AutoCAD 2026 exploded a 2.5 arrow at the origin on a leader from
        // (20, 10) into a SOLID with these corners.
        let shapes =
            arrowhead_shapes(Arrowhead::ClosedFilled, [0.0, 0.0], [20.0, 10.0], 2.5).unwrap();
        let triangle = &shapes.fills[0];
        for corner in [[2.42241, 0.745356], [2.04973, 1.49071]] {
            assert!(
                triangle
                    .iter()
                    .any(|p| (p[0] - corner[0]).abs() < 1e-5 && (p[1] - corner[1]).abs() < 1e-5),
                "{triangle:?}"
            );
        }
        // And its leader line started an arrow's length up from the point.
        let line = trimmed(&[[0.0, 0.0], [20.0, 10.0]], Arrowhead::ClosedFilled, 2.5);
        assert!((line[0][0] - 2.23607).abs() < 1e-5 && (line[0][1] - 1.11803).abs() < 1e-5);
        assert_eq!(
            trimmed(&[[0.0, 0.0], [20.0, 10.0]], Arrowhead::None, 2.5)[0],
            [0.0, 0.0]
        );
    }

    #[test]
    fn a_steep_leader_with_an_annotation_is_hooked_as_autocad_hooks_it() {
        // AutoCAD 2026's own LEADER from (200, 100) to (230, 120), with a
        // 0.18 arrow, drew its hook from (229.82, 120).
        let mut leader = Leader {
            points: vec![point(200.0, 100.0), point(230.0, 120.0)],
            arrowhead: Arrowhead::ClosedFilled,
            arrow_size: 0.18,
            gap: 0.09,
            annotation: Some(LeaderAnnotation {
                text: "LEGACY NOTE".to_owned(),
                style: STANDARD.to_owned(),
                height: 0.18,
            }),
            layer: "0".to_owned(),
            properties: Properties::default(),
        };
        let path = leader.path();
        assert_eq!(path.len(), 3);
        assert!((path[1].x - 229.82).abs() < 1e-9 && path[1].y == 120.0);
        let annotation = leader.annotation_text().unwrap();
        assert!((annotation.position.x - 230.09).abs() < 1e-9);
        assert_eq!(annotation.justify, Attachment::MiddleLeft);
        // 15° from the horizontal or less is not hooked; nor is a leader
        // with no annotation.
        leader.points[1] = point(230.0, 100.0 + 30.0 * 15f64.to_radians().tan());
        assert!(!leader.hooked());
        leader.points[1] = point(230.0, 120.0);
        leader.annotation = None;
        assert!(!leader.hooked());
    }
}
