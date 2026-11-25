//! Blocks, their attribute definitions, and inserts of them.
//!
//! A block is a named group of objects defined once, in the block library a
//! sheet draws on, and inserted many times. Among its objects are attribute
//! definitions: labelled text fields each insert fills in. An insert places
//! the block at a position, scale and rotation, and carries a value for each
//! of the block's attributes, as AutoCAD's INSERT does.
//!
//! An attribute on an insert is drawn as its definition is, moved, scaled
//! and turned with the insert ([`attribute_text`]); the DWG holds it so, and
//! a reader checks it is so before it reads the insert back.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::dynamic_block::{DynamicParameter, DynamicValue};
use super::{is_false, layer_zero, Layer, Linetype, Object, Point3, LAYER_ZERO};
use crate::description::normalize_degrees;
use crate::text::{Attachment, Justify, MText, Text, TextStyle, STANDARD};
use crate::units::Unit;

fn standard() -> String {
    STANDARD.to_owned()
}

fn one() -> f64 {
    1.0
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(value: &f64) -> bool {
    *value == 0.0
}

/// A named group of objects defined once and inserted many times, as in
/// AutoCAD.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub name: String,
    /// The point of the block that sits at an insert's position, in the
    /// block's own coordinates.
    pub base_point: Point3,
    /// The block's objects in drawing order, its attribute definitions
    /// among them. A block holds no inserts.
    pub objects: Vec<Object>,
    /// The cells its texts and attribute definitions declare, in the order
    /// of `objects`. Only the checker reads them: a DWG holds none, and a
    /// block read from a DWG has none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cells: Vec<Cell>,
    /// Dynamic block parameters (visibility states, flips, linear distances,
    /// rotations, and lookup tables). Empty for an ordinary static block.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<DynamicParameter>,
    /// Whether this block definition is annotative (`ANNOTATIVE`): inserts of
    /// it draw scaled by the inverse of the active annotation scale so the
    /// symbol lands on paper at its declared paper size.
    #[serde(default, skip_serializing_if = "is_false")]
    pub annotative: bool,
}

/// The box a block's text or attribute definition draws within, such as
/// its title-block cell: in the block's own coordinates, seen from above.
/// On each insert it is moved, scaled and turned with the insert.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    /// The text's or attribute definition's place in the block's objects,
    /// counted from 0.
    pub object: usize,
    /// The cell's lower left corner, `[x, y]`.
    pub min: [f64; 2],
    /// The cell's upper right corner, `[x, y]`.
    pub max: [f64; 2],
}

impl Block {
    /// The block's attribute definitions, in the order it defines them.
    pub fn attribute_definitions(&self) -> impl Iterator<Item = &AttributeDefinition> {
        self.objects.iter().filter_map(|object| match object {
            Object::AttributeDefinition(definition) => Some(definition),
            _ => None,
        })
    }

    /// The attribute definitions an insert carries a value for, in the
    /// order it carries them: every one but the constant ones, whose value
    /// the block's definition holds, as in AutoCAD.
    pub fn carried_attributes(&self) -> impl Iterator<Item = &AttributeDefinition> {
        self.attribute_definitions().filter(|d| !d.constant)
    }

    /// The value the attribute definition at `index` among the block's
    /// objects shows on `insert`: a constant one's default, or the value
    /// the insert carries for it, matched by its place among the carried
    /// definitions, since a block may define a tag more than once. None
    /// when the object there is not an attribute definition.
    pub fn attribute_value<'a>(&'a self, insert: &'a Insert, index: usize) -> Option<&'a str> {
        let Some(Object::AttributeDefinition(definition)) = self.objects.get(index) else {
            return None;
        };
        if definition.constant {
            return Some(&definition.default);
        }
        let carried = self.objects[..index]
            .iter()
            .filter(|o| matches!(o, Object::AttributeDefinition(d) if !d.constant))
            .count();
        Some(
            insert
                .attributes
                .get(carried)
                .map_or(definition.default.as_str(), |a| a.value.as_str()),
        )
    }

    /// Evaluates this block at the dynamic `state` of an insert, returning a
    /// borrowed reference when the block has no local dynamic actions.
    pub fn evaluated<'a>(
        &'a self,
        state: &BTreeMap<String, DynamicValue>,
    ) -> std::borrow::Cow<'a, Block> {
        super::dynamic_block::evaluate_block(self, state)
    }

    /// Whether this block declares any dynamic parameters.
    pub fn is_dynamic(&self) -> bool {
        !self.parameters.is_empty()
    }
}

/// A labelled text field on a block, filled in at each insert: AutoCAD's
/// ATTDEF.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AttributeDefinition {
    /// The attribute's tag: the label an insert fills it in by, in capitals.
    /// A block may define a tag more than once, as AutoCAD allows.
    pub tag: String,
    /// What AutoCAD asks for when a person inserts the block; empty for the tag.
    #[serde(default)]
    pub prompt: String,
    /// The value an insert takes when it gives none; a constant
    /// attribute's value on every insert.
    #[serde(default)]
    pub default: String,
    /// The point the attribute's text is justified at, in the block's own
    /// coordinates.
    pub position: Point3,
    /// The height of its capitals, in the declared unit.
    pub height: f64,
    /// The text style it draws with.
    #[serde(default = "standard")]
    pub style: String,
    /// How wide its characters are, as a share of their normal width.
    #[serde(default = "one")]
    pub width_factor: f64,
    /// Degrees anticlockwise from the block's X axis, in `[0, 360)`.
    #[serde(default)]
    pub rotation: f64,
    /// Where the text sits against its position.
    #[serde(default)]
    pub justify: Justify,
    /// Whether the attribute is hidden on every insert, as AutoCAD's
    /// Invisible mode hides it.
    #[serde(default)]
    pub invisible: bool,
    /// Whether the attribute has the same value, its `default`, on every
    /// insert, as AutoCAD's Constant mode: no insert carries a value for it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub constant: bool,
    /// Whether AutoCAD asks a person to confirm the value they type, as its
    /// Verify mode does. It changes nothing that is drawn.
    #[serde(default, skip_serializing_if = "is_false")]
    pub verify: bool,
    /// Whether AutoCAD takes the default without asking a person when the
    /// block is inserted, as its Preset mode does. It changes nothing that
    /// is drawn.
    #[serde(default, skip_serializing_if = "is_false")]
    pub preset: bool,
    /// Whether a person in AutoCAD cannot move the attribute apart from its
    /// insert, as its Lock position mode keeps it. It changes nothing that
    /// is drawn.
    #[serde(default, skip_serializing_if = "is_false")]
    pub lock_position: bool,
    /// Whether the attribute is multi-line (AutoCAD's ATTDEF Multiple lines
    /// mode, #117): its value draws as multiline text, wrapped in `width`
    /// with line breaks.
    #[serde(default, skip_serializing_if = "is_false")]
    pub multiline: bool,
    /// For a multi-line attribute, the boundary width lines wrap in, in the
    /// block's declared unit (0 for lines that break only at explicit line
    /// breaks). Always 0 for a single-line attribute.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub width: f64,
    #[serde(default = "layer_zero")]
    pub layer: String,
    #[serde(flatten)]
    pub properties: crate::model::Properties,
}

impl AttributeDefinition {
    /// The multiline text attachment corresponding to this definition's
    /// justification.
    pub fn attachment(&self) -> Attachment {
        justify_to_attachment(self.justify)
    }
}

/// Maps a [`Justify`] alignment to an mtext [`Attachment`].
pub(crate) fn justify_to_attachment(justify: Justify) -> Attachment {
    match justify {
        Justify::Left | Justify::TopLeft => Attachment::TopLeft,
        Justify::Center | Justify::TopCenter => Attachment::TopCenter,
        Justify::Right | Justify::TopRight => Attachment::TopRight,
        Justify::MiddleLeft => Attachment::MiddleLeft,
        Justify::Middle | Justify::MiddleCenter => Attachment::MiddleCenter,
        Justify::MiddleRight => Attachment::MiddleRight,
        Justify::BottomLeft => Attachment::BottomLeft,
        Justify::BottomCenter => Attachment::BottomCenter,
        Justify::BottomRight => Attachment::BottomRight,
    }
}

/// Maps an mtext [`Attachment`] to the matching [`Justify`] value.
pub(crate) fn attachment_to_justify(attachment: Attachment) -> Justify {
    match attachment {
        Attachment::TopLeft => Justify::TopLeft,
        Attachment::TopCenter => Justify::TopCenter,
        Attachment::TopRight => Justify::TopRight,
        Attachment::MiddleLeft => Justify::MiddleLeft,
        Attachment::MiddleCenter => Justify::MiddleCenter,
        Attachment::MiddleRight => Justify::MiddleRight,
        Attachment::BottomLeft => Justify::BottomLeft,
        Attachment::BottomCenter => Justify::BottomCenter,
        Attachment::BottomRight => Justify::BottomRight,
    }
}

/// One attribute an insert carries: the value of the block's attribute
/// with this tag. An insert carries one for each of its block's carried
/// definitions, in order, so a tag the block defines more than once
/// appears more than once.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attribute {
    pub tag: String,
    pub value: String,
    /// Optional AutoCAD field attached to this insert attribute (`FIELD`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<super::TextField>,
}

/// A block placed on a sheet: AutoCAD's INSERT, a block reference.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Insert {
    /// The name of the block it places.
    pub block: String,
    /// Where the block's base point sits.
    pub position: Point3,
    /// How much the block is enlarged, the same along every axis: more than 0.
    #[serde(default = "one")]
    pub scale: f64,
    /// Degrees anticlockwise from the X axis, in `[0, 360)`.
    #[serde(default)]
    pub rotation: f64,
    /// Whether the block is mirrored: its X axis reversed before it is
    /// turned, as AutoCAD's MIRROR leaves an insert, which a DWG keeps as a
    /// negative X scale (#242).
    #[serde(default, skip_serializing_if = "is_false")]
    pub mirrored: bool,
    /// Whether its attributes read forwards, left to right or upwards,
    /// however it is turned, as a mirror leaves them: a mirrored insert's
    /// always do, and one mirrored back keeps them so, as AutoCAD's MIRRTEXT
    /// 0 does on every mirror. A constant attribute is part of the block,
    /// which the insert carries no attribute for, so it turns with the block.
    #[serde(default, skip_serializing_if = "is_false")]
    pub readable_attributes: bool,
    /// A value for each of the block's attributes, in the order the block
    /// defines them.
    #[serde(default)]
    pub attributes: Vec<Attribute>,
    /// Chosen dynamic block parameter values, from parameter name to value.
    /// Empty for an ordinary static block or a dynamic block in default state.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub state: BTreeMap<String, DynamicValue>,
    /// When read from a DWG whose dynamic insert references an anonymous
    /// evaluated block (`*U<n>`), the name of that evaluated block in
    /// `sheet.blocks` while `block` keeps the effective dynamic block name.
    #[serde(skip)]
    pub(crate) evaluated_block: Option<String>,
    /// The name an agent selects the insert by, if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Whether this insert is annotative (`ANNOTATIVE`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub annotative: bool,
    /// The annotation scales this annotative insert supports (`OBJECTSCALE`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotation_scales: Vec<String>,
    #[serde(default = "layer_zero")]
    pub layer: String,
    #[serde(flatten)]
    pub properties: crate::model::Properties,
}

/// The blocks a drawing set shares, each defined once, with the layers,
/// linetypes and text styles their objects use.
///
/// Every length is in `units`, the library's declared unit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlockLibrary {
    pub units: Unit,
    /// The linetypes the library's layers draw with, besides `Continuous`.
    #[serde(default)]
    pub linetypes: Vec<Linetype>,
    /// The layers the blocks' objects sit on. Layer `0` is every drawing's;
    /// listing it here sets its colour, linetype and lineweight.
    #[serde(default)]
    pub layers: Vec<Layer>,
    /// The text styles the blocks' text draws with. Style `Standard` is
    /// every drawing's; listing it here sets its font and width factor.
    #[serde(default)]
    pub text_styles: Vec<TextStyle>,
    pub blocks: Vec<Block>,
}

/// The text an attribute draws on an insert of `block`: the definition's,
/// moved, scaled and turned with the insert, saying `value`.
///
/// As in AutoCAD, an attribute defined on layer `0` sits on the insert's
/// layer, and one defined on any other layer on that layer.
///
/// On a mirrored insert the attribute sits where the mirror puts it but
/// still reads forwards, as AutoCAD's MIRRTEXT 0 leaves it: its direction is
/// the mirrored one, or that turned a half turn, whichever reads left to
/// right or upwards ([`readable`]), and its justification is its own. On an
/// insert mirrored back, with [`Insert::readable_attributes`], it reads
/// along its turned direction, or that turned a half turn, by the same rule,
/// but for a constant one, which is part of the block and turns with it.
pub fn attribute_text(
    insert: &Insert,
    block: &Block,
    definition: &AttributeDefinition,
    value: &str,
) -> Text {
    let layer = if definition.layer == LAYER_ZERO {
        insert.layer.clone()
    } else {
        definition.layer.clone()
    };
    Text {
        contents: value.to_owned(),
        position: place(insert, block.base_point, definition.position),
        height: definition.height * insert.scale,
        style: definition.style.clone(),
        width_factor: definition.width_factor,
        rotation: if insert.mirrored {
            readable(insert.rotation + 180.0 - definition.rotation)
        } else if insert.readable_attributes && !definition.constant {
            readable(definition.rotation + insert.rotation)
        } else {
            normalize_degrees(definition.rotation + insert.rotation)
        },
        justify: definition.justify,
        underline: false,
        overline: false,
        layer,
        field: None,
        properties: definition.properties.clone(),
        annotative: false,
        annotation_scales: Vec::new(),
    }
}

/// The multiline text a multi-line attribute draws on an insert of `block`:
/// its definition's geometry, moved, scaled (including its wrap `width`) and
/// turned with the insert, saying `value` (#117).
pub fn attribute_mtext(
    insert: &Insert,
    block: &Block,
    definition: &AttributeDefinition,
    value: &str,
) -> MText {
    let layer = if definition.layer == LAYER_ZERO {
        insert.layer.clone()
    } else {
        definition.layer.clone()
    };
    MText {
        contents: value.to_owned(),
        runs: Vec::new(),
        position: place(insert, block.base_point, definition.position),
        height: definition.height * insert.scale,
        width: definition.width * insert.scale,
        style: definition.style.clone(),
        width_factor: Some(definition.width_factor),
        rotation: if insert.mirrored {
            readable(insert.rotation + 180.0 - definition.rotation)
        } else if insert.readable_attributes && !definition.constant {
            readable(definition.rotation + insert.rotation)
        } else {
            normalize_degrees(definition.rotation + insert.rotation)
        },
        justify: definition.attachment(),
        underline: false,
        overline: false,
        layer,
        field: None,
        properties: definition.properties.clone(),
        annotative: false,
        annotation_scales: Vec::new(),
    }
}

/// Of a direction of text that a mirror leaves reading backwards, `degrees`,
/// and that turned a half turn, the one that reads left to right or
/// upwards: in `[0, 90]` or `(270, 360)`, as AutoCAD's MIRRTEXT 0 keeps
/// mirrored text reading forwards.
///
/// Text that reads straight up or down is on the cutoff, so a direction
/// within [`CUTOFF_NOISE`] of it counts as on it: one that reads up, at 90°,
/// is kept, and one that reads down, at 270°, is turned to read up. A
/// direction read back from a DWG, which keeps radians, lands a hair either
/// side of the degrees it was written in, and must give the same text.
pub(crate) fn readable(degrees: f64) -> f64 {
    let direction = normalize_degrees(degrees);
    if direction <= 90.0 + CUTOFF_NOISE || direction > 270.0 + CUTOFF_NOISE {
        direction
    } else {
        normalize_degrees(direction + 180.0)
    }
}

/// How far, in degrees, a direction may lie from the readable rule's
/// cutoff and count as on it: far more than the round trip through radians
/// moves one, far less than any direction a drawing means.
const CUTOFF_NOISE: f64 = 1e-9;

/// Where `point`, in a block's own coordinates, lands on an insert of a
/// block whose base point is `base`: moved so the base point sits at the
/// insert's position, scaled about it and turned about the Z axis.
pub(crate) fn place(insert: &Insert, base: Point3, point: Point3) -> Point3 {
    Placement::of(insert, base).point(point)
}

/// Where an insert puts its block's points: moved so that the block's base
/// point sits at the insert's position, its X reversed when it is mirrored,
/// scaled about it and turned about the Z axis. The one home of that rule,
/// for the DWG and the drawing pipeline.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Placement {
    base: Point3,
    position: Point3,
    scale: f64,
    mirrored: bool,
    rotation: f64,
    sin: f64,
    cos: f64,
}

impl Placement {
    /// How much the insert enlarges its block.
    pub(crate) fn scale(&self) -> f64 {
        self.scale
    }

    /// Whether the insert mirrors its block.
    pub(crate) fn mirrored(&self) -> bool {
        self.mirrored
    }

    /// The direction, in radians anticlockwise, that a direction of the
    /// block's own, `radians`, takes on the sheet.
    pub(crate) fn radians(&self, radians: f64) -> f64 {
        let rotation = self.rotation.to_radians();
        if self.mirrored {
            rotation + std::f64::consts::PI - radians
        } else {
            rotation + radians
        }
    }

    pub(crate) fn of(insert: &Insert, base: Point3) -> Placement {
        let (sin, cos) = crate::description::sin_cos_degrees(insert.rotation);
        Placement {
            base,
            position: insert.position,
            scale: insert.scale,
            mirrored: insert.mirrored,
            rotation: insert.rotation,
            sin,
            cos,
        }
    }

    /// The X scale: negative when the insert is mirrored.
    fn x_scale(&self) -> f64 {
        if self.mirrored {
            -self.scale
        } else {
            self.scale
        }
    }

    /// Where a point in the block's own coordinates lands.
    pub(crate) fn point(&self, point: Point3) -> Point3 {
        let x = (point.x - self.base.x) * self.x_scale();
        let y = (point.y - self.base.y) * self.scale;
        let z = (point.z - self.base.z) * self.scale;
        Point3::new(
            self.position.x + x * self.cos - y * self.sin,
            self.position.y + x * self.sin + y * self.cos,
            self.position.z + z,
        )
    }

    /// Where a placed point `[x, y]` is in the block's own coordinates.
    pub(crate) fn back(&self, x: f64, y: f64) -> [f64; 2] {
        let x = x - self.position.x;
        let y = y - self.position.y;
        [
            self.base.x + (x * self.cos + y * self.sin) / self.x_scale(),
            self.base.y + (y * self.cos - x * self.sin) / self.scale,
        ]
    }
}

/// The text an attribute definition draws in its block: AutoCAD shows the
/// tag, so a justified definition starts where its tag would.
pub(crate) fn definition_text(definition: &AttributeDefinition) -> Text {
    Text {
        contents: definition.tag.clone(),
        position: definition.position,
        height: definition.height,
        style: definition.style.clone(),
        width_factor: definition.width_factor,
        rotation: definition.rotation,
        justify: definition.justify,
        underline: false,
        overline: false,
        layer: definition.layer.clone(),
        field: None,
        properties: definition.properties.clone(),
        annotative: false,
        annotation_scales: Vec::new(),
    }
}

/// The multiline text a multi-line attribute definition draws in its block:
/// showing its tag at its attachment point and boundary width (#117).
pub(crate) fn definition_mtext(definition: &AttributeDefinition) -> MText {
    MText {
        contents: definition.tag.clone(),
        runs: Vec::new(),
        position: definition.position,
        height: definition.height,
        width: definition.width,
        style: definition.style.clone(),
        width_factor: Some(definition.width_factor),
        rotation: definition.rotation,
        justify: definition.attachment(),
        underline: false,
        overline: false,
        layer: definition.layer.clone(),
        field: None,
        properties: definition.properties.clone(),
        annotative: false,
        annotation_scales: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition(position: Point3, layer: &str) -> AttributeDefinition {
        AttributeDefinition {
            tag: "TAG".into(),
            prompt: String::new(),
            default: String::new(),
            position,
            height: 2.5,
            style: STANDARD.into(),
            width_factor: 0.8,
            rotation: 350.0,
            justify: Justify::MiddleCenter,
            invisible: false,
            layer: layer.into(),
            properties: Default::default(),
            constant: false,
            lock_position: false,
            multiline: false,
            width: 0.0,
            preset: false,
            verify: false,
        }
    }

    fn insert(rotation: f64, scale: f64) -> Insert {
        Insert {
            block: "B".into(),
            position: Point3::new(100.0, 50.0, 1.0),
            scale,
            rotation,
            mirrored: false,
            readable_attributes: false,
            attributes: Vec::new(),
            state: BTreeMap::new(),
            evaluated_block: None,
            name: None,
            layer: "symbols".into(),
            properties: Default::default(),
            annotative: false,
            annotation_scales: Vec::new(),
        }
    }

    fn block(base_point: Point3) -> Block {
        Block {
            name: "B".into(),
            base_point,
            objects: Vec::new(),
            cells: Vec::new(),
            parameters: Vec::new(),
            annotative: false,
        }
    }

    #[track_caller]
    fn close(p: Point3, x: f64, y: f64, z: f64) {
        assert!(
            (p.x - x).abs() < 1e-12 && (p.y - y).abs() < 1e-12 && (p.z - z).abs() < 1e-12,
            "{p:?} is not ({x}, {y}, {z})"
        );
    }

    #[test]
    fn an_attribute_moves_scales_and_turns_with_its_insert() {
        let base = Point3::new(2.0, 1.0, 0.0);
        let definition = definition(Point3::new(5.0, 1.0, 0.5), "tags");
        // 3 along the block's X from its base, doubled and turned a quarter.
        let text = attribute_text(&insert(90.0, 2.0), &block(base), &definition, "A-1");
        close(text.position, 100.0, 56.0, 2.0);
        assert_eq!(text.height, 5.0);
        assert_eq!(text.rotation, 80.0);
        assert_eq!(text.width_factor, 0.8);
        assert_eq!(text.contents, "A-1");
        assert_eq!(text.layer, "tags");
    }

    #[test]
    fn a_mirrored_inserts_attribute_sits_mirrored_and_reads_forwards() {
        let base = Point3::new(2.0, 1.0, 0.0);
        let mut definition = definition(Point3::new(5.0, 3.0, 0.5), "tags");
        definition.rotation = 0.0;
        // Mirrored across the block's Y axis: 3 along X lands 3 back, exactly.
        let mut mirrored = insert(0.0, 2.0);
        mirrored.mirrored = true;
        let text = attribute_text(&mirrored, &block(base), &definition, "A-1");
        assert_eq!(text.position, Point3::new(94.0, 54.0, 2.0));
        assert_eq!(text.rotation, 0.0, "it reads forwards, not backwards");
        assert_eq!(text.justify, Justify::MiddleCenter);
        // Mirrored and turned a half turn, as a mirror across a horizontal
        // line leaves an insert: the attribute lands below, still upright.
        let mut flipped = insert(180.0, 1.0);
        flipped.mirrored = true;
        let text = attribute_text(&flipped, &block(base), &definition, "A-1");
        assert_eq!(text.position, Point3::new(103.0, 48.0, 1.5));
        assert_eq!(text.rotation, 0.0);
        // A definition turned 30 degrees reads along its mirrored line.
        definition.rotation = 30.0;
        let text = attribute_text(&mirrored, &block(base), &definition, "A-1");
        assert_eq!(text.rotation, 330.0);
    }

    #[test]
    fn an_insert_mirrored_back_keeps_its_attributes_reading_forwards() {
        let base = Point3::new(0.0, 0.0, 0.0);
        let mut definition = definition(Point3::new(5.0, 0.0, 0.0), "tags");
        definition.rotation = 0.0;
        // Mirrored at 120, the attribute reads at 300.
        let mut once = insert(120.0, 1.0);
        once.mirrored = true;
        let text = attribute_text(&once, &block(base), &definition, "A-1");
        assert_eq!(text.rotation, 300.0);
        // Mirrored back across a line up the sheet, the insert is turned 240
        // and no longer mirrored: its attribute reads at 60, not 240.
        let mut twice = insert(240.0, 1.0);
        twice.readable_attributes = true;
        let text = attribute_text(&twice, &block(base), &definition, "A-1");
        assert!((text.rotation - 60.0).abs() < 1e-9, "{}", text.rotation);
        // A constant one is part of the block, so it turns with it, at 240,
        // as AutoCAD draws it from the DWG.
        let mut constant = definition.clone();
        constant.constant = true;
        let text = attribute_text(&twice, &block(base), &constant, "K");
        assert_eq!(text.rotation, 240.0);
        // Without the key, an insert turned 240 turns its attribute with it.
        twice.readable_attributes = false;
        let text = attribute_text(&twice, &block(base), &definition, "A-1");
        assert_eq!(text.rotation, 240.0);
    }

    #[test]
    fn readable_keeps_text_reading_left_to_right_or_upwards() {
        assert_eq!(readable(180.0), 0.0);
        assert_eq!(readable(150.0), 330.0);
        assert_eq!(readable(90.0), 90.0);
        assert_eq!(readable(270.0), 90.0);
        assert_eq!(readable(-30.0), 330.0);
        // A hair off the cutoff, as a DWG's radians bring a direction back,
        // reads as on it.
        assert_eq!(readable(90.00000000000001), 90.00000000000001);
        assert!((readable(269.99999999999994) - 90.0).abs() < 1e-12);
        assert!((readable(270.00000000000006) - 90.0).abs() < 1e-12);
        assert_eq!(readable(90.001), 270.001);
    }

    #[test]
    fn an_attribute_defined_on_layer_zero_sits_on_the_inserts_layer() {
        let definition = definition(Point3::new(0.0, 0.0, 0.0), LAYER_ZERO);
        let text = attribute_text(
            &insert(0.0, 1.0),
            &block(Point3::new(0.0, 0.0, 0.0)),
            &definition,
            "",
        );
        assert_eq!(text.layer, "symbols");
        close(text.position, 100.0, 50.0, 1.0);
    }
}
