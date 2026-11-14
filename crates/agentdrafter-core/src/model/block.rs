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

use serde::{Deserialize, Serialize};

use super::{is_false, layer_zero, Layer, Linetype, Object, Point3, LAYER_ZERO};
use crate::description::normalize_degrees;
use crate::text::{Justify, Text, TextStyle, STANDARD};
use crate::units::Unit;

fn standard() -> String {
    STANDARD.to_owned()
}

fn one() -> f64 {
    1.0
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
    #[serde(default = "layer_zero")]
    pub layer: String,
    #[serde(flatten)]
    pub properties: crate::model::Properties,
}

/// One attribute an insert carries: the value of the block's attribute
/// with this tag. An insert carries one for each of its block's carried
/// definitions, in order, so a tag the block defines more than once
/// appears more than once.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attribute {
    pub tag: String,
    pub value: String,
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
    /// A value for each of the block's attributes, in the order the block
    /// defines them.
    #[serde(default)]
    pub attributes: Vec<Attribute>,
    /// The name an agent selects the insert by, if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
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
        rotation: normalize_degrees(definition.rotation + insert.rotation),
        justify: definition.justify,
        layer,
        properties: definition.properties.clone(),
    }
}

/// Where `point`, in a block's own coordinates, lands on an insert of a
/// block whose base point is `base`: moved so the base point sits at the
/// insert's position, scaled about it and turned about the Z axis.
pub(crate) fn place(insert: &Insert, base: Point3, point: Point3) -> Point3 {
    Placement::of(insert, base).point(point)
}

/// Where an insert puts its block's points: moved so that the block's base
/// point sits at the insert's position, scaled about it and turned about the
/// Z axis. The one home of that rule, for the DWG and the drawing pipeline.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Placement {
    base: Point3,
    position: Point3,
    scale: f64,
    sin: f64,
    cos: f64,
}

impl Placement {
    /// How much the insert enlarges its block.
    pub(crate) fn scale(&self) -> f64 {
        self.scale
    }

    pub(crate) fn of(insert: &Insert, base: Point3) -> Placement {
        let (sin, cos) = insert.rotation.to_radians().sin_cos();
        Placement {
            base,
            position: insert.position,
            scale: insert.scale,
            sin,
            cos,
        }
    }

    /// Where a point in the block's own coordinates lands.
    pub(crate) fn point(&self, point: Point3) -> Point3 {
        let x = (point.x - self.base.x) * self.scale;
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
            self.base.x + (x * self.cos + y * self.sin) / self.scale,
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
        layer: definition.layer.clone(),
        properties: definition.properties.clone(),
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
            attributes: Vec::new(),
            name: None,
            layer: "symbols".into(),
            properties: Default::default(),
        }
    }

    fn block(base_point: Point3) -> Block {
        Block {
            name: "B".into(),
            base_point,
            objects: Vec::new(),
            cells: Vec::new(),
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
