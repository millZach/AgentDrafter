//! Queries: what an agent asks of a drawing set, or of one sheet
//! description, to check its work. Nothing is selected by coordinates.
//!
//! A query reads the records, the sheet descriptions and templates filled
//! in from their rows, not the built DWGs. The records are each sheet's
//! truth (ADR-0003): a query needs no build, cannot read a stale DWG, and
//! answers in the set's declared unit, exactly as the record gives it.
//!
//! - **Find** objects by ID, name, block, attribute or text, the text an
//!   insert's block draws among it.
//! - **Blocks**: the blocks each sheet inserts, and the IDs of the inserts.
//! - **Attributes**: the attributes of inserts, as JSON from tag to value.
//! - **Measure** the distance between points of two objects given by ID or
//!   name: an insert's or a text's position, a line's ends, a circle's
//!   centre.
//! - **Extents**: the box round everything each sheet draws, drawn through
//!   the one drawing pipeline (ADR-0011) with every layer continuous, as
//!   AutoCAD's extents are.
//!
//! A query that matches nothing says so and is not an error, a distance
//! whose end names an ID or a name no object carries among them. A request
//! that cannot be answered as asked is an error, and so is a sheet that
//! cannot be read: an answer must not leave a sheet out.

use std::path::{Component, Path, PathBuf};

use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};

use crate::build::is_same_file;
use crate::description::{read_sheet_record, SavedSet, SheetRecord};
use crate::draw::{self, Bounds, DisplayList, MOST_HATCH_DASHES};
use crate::error::{Error, ErrorCode};
use crate::geometry::snap::{touches, SnapPoint, Touch};
use crate::geometry::{ObjectRef, SnapRef};
use crate::model::{Attribute, Layer, Measured, Object, Point3, RecordsOfKind, Sheet, LAYER_ZERO};
use crate::original::{is_drawing_file, kinds_in_words, plural, read_dwg_record};
use crate::settings::Settings;
use crate::text::fonts::FontSubstitution;
use crate::text::placement::SheetFonts;
use crate::units::Unit;

mod layout;
pub(crate) mod snap;

pub use layout::{PageSetupUsed, SheetLayout, ViewportFound};
pub use snap::{ObjectGeometry, SnapObject, SnappedFrom};

/// How finely a sheet's curves are drawn to find its extents: within this
/// share of the sheet's larger side.
pub const EXTENTS_TOLERANCE: f64 = 1e-6;

/// A request to query a drawing set, or one sheet description.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryRequest {
    /// A drawing set's folder, which holds its settings, or one sheet
    /// description.
    pub source: PathBuf,
    /// Only the sheet of this name, of the set's sheets. Without it, every
    /// sheet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<String>,
    /// What to ask.
    pub query: Query,
}

/// What a query asks. Written as `{"kind": "find", ...}` in JSON, and read
/// refusing any key its kind does not take.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", from = "QueryInput")]
pub enum Query {
    /// The objects that match every part of the selection.
    Find(Selection),
    /// The blocks each sheet inserts.
    Blocks,
    /// The attributes of each insert that matches every part of the
    /// selection.
    Attributes(Selection),
    /// The distance between a point of one object and a point of another.
    Measure(Measure),
    /// The box round everything each sheet draws.
    Extents,
    /// The layers of each sheet, with their states and how many of its
    /// objects sit on each.
    Layers,
    /// The points AutoCAD's object snaps find on an object, or between it
    /// and another.
    Snap(SnapRef),
    /// The length, area, extents and angles of each object that matches
    /// every part of the selection.
    Geometry(Selection),
    /// Each sheet's paper-space layout, its page setup and its viewports.
    Layout,
    /// The groups of each sheet, with their descriptions, selectability and
    /// member objects.
    Groups,
    /// Each sheet's active annotation scale (`CANNOSCALE`), `ANNOALLVISIBLE`,
    /// scale list (`ACAD_SCALELIST`), and every annotative object with its
    /// supported scales.
    AnnotationScales,
}

/// A [`Query`] as it is read. A kind that takes nothing is an empty struct
/// here, not a unit variant, because serde lets a unit variant of a tagged
/// enum take any key without a word: `{"kind": "blocks", "name": "x"}`
/// would be answered as if `name` were not there, where it is refused.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum QueryInput {
    Find(Selection),
    Blocks {},
    Attributes(Selection),
    Measure(Measure),
    Extents {},
    Layers {},
    Snap(SnapRef),
    Geometry(Selection),
    Layout {},
    Groups {},
    AnnotationScales {},
}

impl From<QueryInput> for Query {
    fn from(input: QueryInput) -> Self {
        match input {
            QueryInput::Find(selection) => Self::Find(selection),
            QueryInput::Blocks {} => Self::Blocks,
            QueryInput::Attributes(selection) => Self::Attributes(selection),
            QueryInput::Measure(measure) => Self::Measure(measure),
            QueryInput::Extents {} => Self::Extents,
            QueryInput::Layers {} => Self::Layers,
            QueryInput::Snap(snap) => Self::Snap(snap),
            QueryInput::Geometry(selection) => Self::Geometry(selection),
            QueryInput::Layout {} => Self::Layout,
            QueryInput::Groups {} => Self::Groups,
            QueryInput::AnnotationScales {} => Self::AnnotationScales,
        }
    }
}

/// Which objects a find or an attributes query selects: those that match
/// every part given. With no part given, every object.
///
/// IDs, names, blocks and attribute tags are identifiers, and no two of
/// one kind differ only in case, so an exact match takes them in any case.
/// Text and attribute values match exactly, case and all. With `match:
/// contains`, every part matches when the object's value holds it, in any
/// case.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    /// The object's ID, such as `DEMO-001/border`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The object's name, such as `title-block`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The block an insert places.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<String>,
    /// An attribute's tag: an insert carrying that attribute, or, with
    /// `value`, the attribute the value is looked for in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// An attribute's value, in any of an insert's attributes, or in the
    /// one `tag` names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// What a text or an mtext says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// How the parts match. Without it, exactly.
    #[serde(default, rename = "match")]
    pub matching: Matching,
    /// Objects wholly within this box, as AutoCAD's selection window takes
    /// them: everything they draw inside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<SelectionBox>,
    /// Objects within this box or crossing it, as AutoCAD's crossing
    /// selection takes them: something they draw inside it or across its
    /// edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crossing: Option<SelectionBox>,
    /// Objects whose geometry meets this object's, on each sheet it is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub touching: Option<ObjectRef>,
    /// Objects in this space, on a sheet drawn in a layout: `model`, or a
    /// layout's paper space by its name, or `paper` on a sheet drawn in
    /// one. A box looks at each object in its own space, so with several
    /// layouts it names the one whose paper it is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,
    /// Objects belonging to the named group (`GROUP`), matched in any case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

/// A box on the sheet, seen from above, by two opposite corners in the
/// declared unit: `[[x1, y1], [x2, y2]]`, in either order. A corner may
/// carry a height, `[x, y, z]`, which it does not look at.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct SelectionBox(pub [[f64; 2]; 2]);

impl<'de> Deserialize<'de> for SelectionBox {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let corners: Vec<Vec<f64>> = Vec::deserialize(deserializer)?;
        let corner = |c: &[f64]| match c {
            [x, y] | [x, y, _] if x.is_finite() && y.is_finite() => Ok([*x, *y]),
            _ => Err(serde::de::Error::custom(
                "a box's corner is [x, y], or [x, y, z]: numbers in the declared unit",
            )),
        };
        match corners.as_slice() {
            [a, b] => Ok(SelectionBox([corner(a)?, corner(b)?])),
            _ => Err(serde::de::Error::custom(
                "a box is two opposite corners, [[x1, y1], [x2, y2]]",
            )),
        }
    }
}

impl SelectionBox {
    /// Its lower left and upper right corners.
    fn bounds(&self) -> ([f64; 2], [f64; 2]) {
        let [a, b] = self.0;
        (
            [a[0].min(b[0]), a[1].min(b[1])],
            [a[0].max(b[0]), a[1].max(b[1])],
        )
    }

    /// "from [0, 0] to [10, 5]".
    fn words(&self) -> String {
        let [a, b] = self.0;
        format!("from [{}, {}] to [{}, {}]", a[0], a[1], b[0], b[1])
    }
}

/// How the parts of a selection match an object's values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Matching {
    /// The whole value: identifiers in any case, text and attribute values
    /// case and all.
    #[default]
    Exact,
    /// The value holds what is given, in any case.
    Contains,
}

/// A distance to measure, from a point of one object to a point of another
/// on the same sheet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Measure {
    pub from: End,
    pub to: End,
}

/// One end of a distance: an object, by its ID or its name, and which of
/// its points.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct End {
    /// The object's ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The object's name, which only one sheet of those looked at may give.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Which of the object's points, named as its key in the sheet
    /// description: `position`, `from`, `to`, `center`, `points[2]`. Without
    /// it, the object's first: an insert's or a text's position, a line's
    /// `from`, a circle's, an arc's or an ellipse's centre, the first point
    /// of a polyline or a spline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point: Option<String>,
}

/// What a query found.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct QueryReport {
    /// The drawing set's folder, sheet description, or DWG queried.
    pub source: String,
    /// The declared unit every length in the answer is in: the set's, the
    /// sheet description's, or the DWG's; none when a DWG states no unit the
    /// core knows.
    pub units: Option<Unit>,
    /// The sheets looked at, by name, in the set's order.
    pub sheets: Vec<String>,
    /// What was found, in words. When nothing matched, it says so.
    pub message: String,
    #[serde(flatten)]
    pub answer: Answer,
    /// How many objects of a DWG the core has no model for yet, which are
    /// not queried. Left out when 0.
    #[serde(skip_serializing_if = "is_zero_usize")]
    pub objects_not_read: usize,
    /// Those objects of a DWG, by kind, such as `ACAD_PROXY_ENTITY`, with
    /// the handle of each, as `read` names them. Left out when empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub not_read: Vec<RecordsOfKind>,
    /// External references that could not be found or loaded from disk. Left
    /// out when every reference resolved cleanly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_xrefs: Vec<crate::model::UnresolvedXref>,
    /// Raster images and PDF underlays that could not be found or decoded
    /// from disk. Left out when every attachment resolved cleanly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_media: Vec<crate::model::UnresolvedMedia>,
    /// The drawing set's inconsistent mark, when a change set landed on
    /// some of its sheets and failed on others: the answer reads the
    /// records as they are.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inconsistent: Option<crate::change::Inconsistent>,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero_usize(n: &usize) -> bool {
    *n == 0
}

/// How `units` reads in a message: its symbol, or `"drawing units"` when a
/// DWG declares none.
pub(super) fn units_words(units: Option<Unit>) -> &'static str {
    units.map_or("drawing units", Unit::symbol)
}

/// The answer to each kind of query. Written with the query's `kind`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Answer {
    Find {
        /// How many objects matched; 0 when none did.
        count: usize,
        objects: Vec<FoundObject>,
    },
    Blocks {
        /// How many inserts the sheets hold; 0 when none.
        count: usize,
        /// Each sheet looked at, with the blocks it inserts.
        blocks: Vec<SheetBlocks>,
    },
    Attributes {
        /// How many inserts matched; 0 when none did.
        count: usize,
        inserts: Vec<InsertAttributes>,
    },
    /// A distance; when an end matches no object, no distance and only
    /// the end that matched.
    Measure {
        /// The distance from one point to the other, in the declared unit.
        #[serde(skip_serializing_if = "Option::is_none")]
        distance: Option<f64>,
        /// How far the second point is from the first along X, Y and Z.
        #[serde(skip_serializing_if = "Option::is_none")]
        delta: Option<[f64; 3]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        from: Option<Box<MeasuredPoint>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        to: Option<Box<MeasuredPoint>>,
    },
    Extents {
        /// How many sheets draw something.
        count: usize,
        /// Each sheet looked at, with its extents.
        extents: Vec<SheetExtents>,
        /// The text styles whose fonts are not in the font folders, and the
        /// look-alike each was measured with: text's share of the extents
        /// rests on it.
        font_substitutions: Vec<FontSubstitution>,
    },
    Layers {
        /// How many layers the sheets hold, layer `0` of each among them.
        count: usize,
        /// Each sheet looked at, with its layers.
        layers: Vec<SheetLayers>,
    },
    /// The points a snap found; when an object it names matches none, no
    /// points and only the objects that matched.
    Snap {
        /// How many points were found; 0 when none were.
        count: usize,
        /// The object snapped to.
        #[serde(skip_serializing_if = "Option::is_none")]
        object: Option<SnapObject>,
        /// The other object of an intersection.
        #[serde(skip_serializing_if = "Option::is_none")]
        with: Option<SnapObject>,
        /// The point a perpendicular, a tangent or a nearest point is
        /// taken from.
        #[serde(skip_serializing_if = "Option::is_none")]
        from: Option<Box<SnappedFrom>>,
        points: Vec<SnapPoint>,
    },
    Geometry {
        /// How many objects matched; 0 when none did.
        count: usize,
        objects: Vec<ObjectGeometry>,
    },
    Layout {
        /// How many viewports the sheets' layouts hold; 0 when none.
        count: usize,
        /// Each sheet looked at, with its layout, if it is drawn in one.
        layouts: Vec<SheetLayout>,
    },
    Groups {
        /// How many groups the sheets hold; 0 when none.
        count: usize,
        /// Each sheet looked at, with its groups.
        groups: Vec<SheetGroups>,
    },
    AnnotationScales {
        /// How many annotative objects the sheets hold; 0 when none.
        count: usize,
        /// Each sheet looked at, with its active scale, scale list and
        /// annotative objects.
        annotation_scales: Vec<SheetAnnotationScales>,
    },
}

/// One sheet's annotation scale state (`CANNOSCALE`, `ACAD_SCALELIST`,
/// `ANNOALLVISIBLE`) and annotative objects.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SheetAnnotationScales {
    pub sheet: String,
    /// Active annotation scale name (`CANNOSCALE`), such as `1:1` or `1:50`.
    pub current: String,
    /// Active annotation scale factor (`paper_units / drawing_units`).
    pub factor: f64,
    /// Explicit `ANNOALLVISIBLE` override when set on the sheet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anno_all_visible: Option<bool>,
    /// Defined and active annotation scales on the sheet (`ACAD_SCALELIST`).
    pub scales: Vec<AnnotationScaleFound>,
    /// Annotative objects on the sheet, in order.
    pub objects: Vec<AnnotativeObjectFound>,
}

/// One annotation scale entry listed by `query annotation-scales`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AnnotationScaleFound {
    pub name: String,
    pub paper_units: f64,
    pub drawing_units: f64,
    pub factor: f64,
}

/// One annotative object listed by `query annotation-scales`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AnnotativeObjectFound {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    pub scales: Vec<String>,
}

/// One sheet's groups (`GROUP`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SheetGroups {
    pub sheet: String,
    /// Each group on the sheet, in the order defined.
    pub groups: Vec<GroupFound>,
}

/// One group on a sheet, as `query groups` reports it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GroupFound {
    /// The group's name (such as `FT-101`, or `*A1` for an unnamed group).
    pub name: String,
    /// Optional human-readable description.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Whether AutoCAD selects the whole group when any member is picked.
    pub selectable: bool,
    /// Whether the group is an AutoCAD unnamed (`*A...`) group.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unnamed: bool,
    /// Number of member objects in the group.
    pub count: usize,
    /// Member objects in the group, in order.
    pub objects: Vec<GroupMember>,
}

/// One member object listed inside a [`GroupFound`].
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GroupMember {
    /// The object's full ID on its sheet.
    pub id: String,
    /// Its optional name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Its object type, such as `line` or `insert`.
    #[serde(rename = "type")]
    pub kind: String,
}

/// One sheet's layers.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SheetLayers {
    pub sheet: String,
    /// Layer `0` first, as every drawing has it, then the sheet's own in
    /// order, those its block library brings among them.
    pub layers: Vec<LayerUse>,
}

/// A layer of a sheet: its colour, linetype, lineweight and states, and how
/// many of the sheet's objects sit on it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LayerUse {
    #[serde(flatten)]
    pub layer: Layer,
    /// How many of the sheet's objects sit on it, its inserts among them;
    /// the objects of a block are not counted.
    pub objects: usize,
}

/// Where an object is written: its file and its place in it, or its handle
/// in a DWG.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WrittenAt {
    /// The sheet description, the sheet's template, the rows file, or the DWG.
    pub file: String,
    /// The object's place in the file, such as `objects[3]`. Absent for a DWG.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// The line the object starts on, counted from 1. Absent for a DWG.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    /// The column it starts at, counted from 1 in characters. Absent for a DWG.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
    /// The object's handle in a DWG, in hexadecimal as AutoCAD lists it, such
    /// as `2F`. Absent for a sheet description or drawing set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
}

/// One object a find matched.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FoundObject {
    /// The sheet the object is on.
    pub sheet: String,
    /// The object's ID.
    pub id: String,
    /// Whether the record gives the ID under `id`. One it does not give is
    /// the object's place in its file, which moves when an object is added
    /// or taken away before it.
    pub id_written: bool,
    #[serde(flatten)]
    pub written_at: WrittenAt,
    /// The space it is in, on a sheet drawn in a layout: `model`, or the
    /// name of the layout whose paper space holds it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,
    /// The object, as the sheet holds it, in the declared unit.
    pub object: Object,
    /// When an insert is found by text its block draws: each text or
    /// multiline text of the block that matched.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub block_text: Vec<BlockText>,
    /// When it is found touching another object: where it meets it, and
    /// the part of each that meets there.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub touches: Vec<Touch>,
}

/// A text or a multiline text of an insert's block that a query's `text`
/// matched.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BlockText {
    /// The block library that defines the block, or the DWG.
    pub file: String,
    /// Its place in the block definition, such as
    /// `blocks.TITLE_BLOCK.objects[7]`. Absent for a DWG.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// Its handle in the DWG, in hexadecimal. Absent for a block library.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    /// What it says.
    pub contents: String,
}

/// The blocks one sheet inserts.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SheetBlocks {
    pub sheet: String,
    /// Each block the sheet inserts, in the order it first inserts it.
    pub blocks: Vec<BlockInserts>,
}

/// One block a sheet inserts, and the IDs of its inserts there.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BlockInserts {
    pub block: String,
    pub inserts: Vec<String>,
}

/// The attributes of one insert.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InsertAttributes {
    pub sheet: String,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub block: String,
    /// From each attribute's tag to its value, in the order the block
    /// defines them.
    pub attributes: AttributeValues,
    #[serde(flatten)]
    pub written_at: WrittenAt,
    /// When the insert is found by text its block draws: each text or
    /// multiline text of the block that matched.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub block_text: Vec<BlockText>,
}

/// An insert's attributes, written in JSON as a mapping from each tag to
/// its value, in the order the block defines them. A tag the block defines
/// more than once maps to a list of its values, in that order, the first
/// being occurrence 1, as an insert gives them.
#[derive(Clone, Debug, PartialEq)]
pub struct AttributeValues(pub Vec<Attribute>);

impl Serialize for AttributeValues {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut tags: Vec<(&str, Vec<&str>)> = Vec::new();
        for attribute in &self.0 {
            match tags.iter_mut().find(|(tag, _)| *tag == attribute.tag) {
                Some((_, values)) => values.push(&attribute.value),
                None => tags.push((&attribute.tag, vec![&attribute.value])),
            }
        }
        let mut map = serializer.serialize_map(Some(tags.len()))?;
        for (tag, values) in &tags {
            match values.as_slice() {
                [value] => map.serialize_entry(tag, value)?,
                values => map.serialize_entry(tag, values)?,
            }
        }
        map.end()
    }
}

/// One end of a measured distance.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MeasuredPoint {
    pub sheet: String,
    /// The object's ID.
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The object's type, such as `insert`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Which of its points, such as `position`.
    pub point: String,
    /// Where the point is, in the declared unit.
    pub at: Point3,
}

/// One sheet's extents.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SheetExtents {
    pub sheet: String,
    /// The box round everything the sheet draws; none when it draws nothing.
    pub extents: Option<Extents>,
}

/// The box round everything a sheet draws, seen from above, as AutoCAD's
/// extents are: the geometry itself, not the width of its pens.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Extents {
    /// The lower left corner, `[x, y]`.
    pub min: [f64; 2],
    /// The upper right corner, `[x, y]`.
    pub max: [f64; 2],
    pub width: f64,
    pub height: f64,
}

/// Queries a drawing set or a sheet description, reading fonts from the
/// font folders the environment names ([`Settings::from_env`]). See
/// [`query_with`].
pub fn query(request: &QueryRequest) -> Result<QueryReport, Error> {
    query_with(request, &Settings::from_env())
}

/// Queries a drawing set, one sheet description, or a DWG, from its records,
/// reading fonts from the font folders `settings` names to find extents.
pub fn query_with(request: &QueryRequest, settings: &Settings) -> Result<QueryReport, Error> {
    let source = request.source.as_path();
    let need_set_paper = matches!(request.query, Query::Layout);
    let Loaded {
        units,
        records,
        objects_not_read,
        not_read,
        unresolved_xrefs,
        unresolved_media,
    } = load(source, request.sheet.as_deref(), need_set_paper)?;
    let looked_at: Vec<&SheetRecord> = records.iter().collect();
    let looked = LookedAt(&looked_at, request.sheet.is_some());
    let (mut message, answer) = match &request.query {
        Query::Find(selection) => {
            check_touching(&looked_at, selection)?;
            find(&looked_at, selection, settings, &looked)
        }
        Query::Blocks => blocks(&looked_at, &looked),
        Query::Attributes(selection) => {
            check_touching(&looked_at, selection)?;
            attributes(&looked_at, selection, settings, &looked)
        }
        Query::Measure(measure) => self::measure(&looked_at, measure, units, &looked)?,
        Query::Extents => extents(&looked_at, settings, units, &looked),
        Query::Layers => layers(&looked_at, &looked),
        Query::Snap(request) => snap::snap(&looked_at, request, settings, &looked)?,
        Query::Geometry(selection) => {
            check_touching(&looked_at, selection)?;
            snap::geometry(&looked_at, selection, settings, units, &looked)
        }
        Query::Layout => layout::layout(&looked_at, settings, units, &looked),
        Query::Groups => groups(&looked_at, &looked),
        Query::AnnotationScales => annotation_scales(&looked_at, &looked),
    };
    if objects_not_read > 0 {
        message.push_str(&format!(
            "; {objects_not_read} object{} the core has no model for not read: {}",
            plural(objects_not_read),
            kinds_in_words(&not_read)
        ));
    }
    Ok(QueryReport {
        source: source.display().to_string(),
        units,
        sheets: looked_at.iter().map(|r| r.name.clone()).collect(),
        message,
        answer,
        objects_not_read,
        not_read,
        unresolved_xrefs,
        unresolved_media,
        inconsistent: if source.is_dir() {
            crate::change::inconsistent(source)
        } else {
            None
        },
    })
}

/// The sheets loaded for a query, with their declared unit and any objects of
/// a DWG the core has no model for.
struct Loaded {
    units: Option<Unit>,
    records: Vec<SheetRecord>,
    objects_not_read: usize,
    not_read: Vec<RecordsOfKind>,
    unresolved_xrefs: Vec<crate::model::UnresolvedXref>,
    unresolved_media: Vec<crate::model::UnresolvedMedia>,
}

/// Reads the sheets a query looks at, with the declared unit: every sheet
/// of the drawing set whose folder is `source`, the one sheet description
/// `source` is, or the DWG `source` is; or only the one named `sheet`, in
/// any case. A sheet that cannot be read stops the query.
fn load(source: &Path, sheet: Option<&str>, need_set_paper: bool) -> Result<Loaded, Error> {
    let named = |name: &str| sheet.is_none_or(|s| s.to_lowercase() == name.to_lowercase());
    if !source.is_dir() {
        if is_drawing_file(source) {
            let loaded = read_dwg_record(source, need_set_paper)?;
            if !named(&loaded.record.name) {
                return Err(no_such_sheet(sheet, &[loaded.record.name.as_str()]));
            }
            return Ok(Loaded {
                units: loaded.units,
                records: vec![loaded.record],
                objects_not_read: loaded.objects_not_read,
                not_read: loaded.not_read,
                unresolved_xrefs: loaded.unresolved_xrefs,
                unresolved_media: loaded.unresolved_media,
            });
        }
        let mut record = read_sheet_record(source)?;
        if !named(&record.name) {
            return Err(no_such_sheet(sheet, &[record.name.as_str()]));
        }
        let unresolved_xrefs =
            crate::xref::resolve_sheet_xrefs(&mut record.sheet, source).unresolved;
        let unresolved_media =
            crate::media::resolve_sheet_media_for_host(&mut record.sheet, source).unresolved;
        return Ok(Loaded {
            units: Some(record.sheet.units),
            records: vec![record],
            objects_not_read: 0,
            not_read: Vec::new(),
            unresolved_xrefs,
            unresolved_media,
        });
    }
    let set = SavedSet::read(source)?;
    let looked_at: Vec<_> = set.rows.iter().filter(|row| named(&row.sheet)).collect();
    if looked_at.is_empty() {
        return Err(no_such_sheet(sheet, &set.names()));
    }
    let mut unresolved_xrefs = Vec::new();
    let mut unresolved_media = Vec::new();
    let records = looked_at
        .into_iter()
        .map(|row| {
            let mut record = set.sheet(row).map_err(|e| Error {
                message: format!(
                    "sheet `{}` cannot be read, and a query answers for every sheet it looks \
                     at: {}",
                    row.sheet, e.message
                ),
                ..e
            })?;
            let host = record.file.clone();
            for u in crate::xref::resolve_sheet_xrefs(&mut record.sheet, &host).unresolved {
                if !unresolved_xrefs.contains(&u) {
                    unresolved_xrefs.push(u);
                }
            }
            for u in crate::media::resolve_sheet_media_for_host(&mut record.sheet, &host).unresolved
            {
                if !unresolved_media.contains(&u) {
                    unresolved_media.push(u);
                }
            }
            Ok(record)
        })
        .collect::<Result<Vec<_>, Error>>()?;
    Ok(Loaded {
        units: Some(set.units),
        records,
        objects_not_read: 0,
        not_read: Vec::new(),
        unresolved_xrefs,
        unresolved_media,
    })
}

/// The error for a sheet to look at that is not among `names`, the sheets
/// there are.
fn no_such_sheet(sheet: Option<&str>, names: &[&str]) -> Error {
    Error::new(
        ErrorCode::UnknownName,
        format!(
            "there is no sheet `{}` to query; the sheets: {}",
            sheet.unwrap_or_default(),
            names.join(", ")
        ),
    )
    .at_field("sheet")
}

/// The sheets a query looks at, for its message, and whether one was named.
struct LookedAt<'a>(&'a [&'a SheetRecord], bool);

impl LookedAt<'_> {
    /// "on sheet `DEMO-001`", or "on the 3 sheets looked at".
    fn on(&self) -> String {
        match self.0 {
            [one] if self.1 => format!("on sheet `{}`", one.name),
            [one] => format!("on the sheet `{}`", one.name),
            all => format!("on the {} sheets looked at", all.len()),
        }
    }
}

impl Matching {
    /// Whether an identifier, such as an ID or a name, matches `wanted`.
    fn identifier(self, have: &str, wanted: &str) -> bool {
        match self {
            Matching::Exact => have.to_lowercase() == wanted.to_lowercase(),
            Matching::Contains => contains(have, wanted),
        }
    }

    /// Whether text, such as an attribute's value, matches `wanted`.
    fn text(self, have: &str, wanted: &str) -> bool {
        match self {
            Matching::Exact => have == wanted,
            Matching::Contains => contains(have, wanted),
        }
    }
}

fn contains(have: &str, wanted: &str) -> bool {
    have.to_lowercase().contains(&wanted.to_lowercase())
}

impl Selection {
    /// Whether the object at `index` of `record` matches every part given:
    /// none when it does not; when it does, the text and multiline text of
    /// an insert's block that `text` matched, empty when the object matched
    /// in itself.
    fn matches(&self, record: &SheetRecord, index: usize) -> Option<Vec<BlockText>> {
        let m = self.matching;
        let object = &record.sheet.objects[index];
        let insert = match object {
            Object::Insert(insert) => Some(insert),
            _ => None,
        };
        if let Some(id) = &self.id {
            if !m.identifier(&record.id(index), id) {
                return None;
            }
        }
        if let Some(space) = &self.space {
            let sheet = &record.sheet;
            let here = if space == crate::description::layout::MODEL {
                sheet.in_model_space(index)
            } else {
                let names: Vec<String> = sheet.layouts().map(|l| l.name.clone()).collect();
                crate::description::layout::layout_named(space, &names)
                    .is_some_and(|k| sheet.layouts().nth(k).is_some_and(|l| l.holds(index)))
            };
            if !here {
                return None;
            }
        }
        if let Some(name) = &self.name {
            match record.objects[index].name.as_deref() {
                Some(have) if m.identifier(have, name) => {}
                _ => return None,
            }
        }
        if let Some(group) = &self.group {
            let in_group = record
                .sheet
                .groups
                .iter()
                .any(|g| m.identifier(&g.name, group) && g.objects.contains(&index));
            if !in_group {
                return None;
            }
        }
        if let Some(block) = &self.block {
            match insert {
                Some(insert) if m.identifier(&insert.block, block) => {}
                _ => return None,
            }
        }
        if self.tag.is_some() || self.value.is_some() {
            let found = insert?.attributes.iter().any(|a| {
                self.tag.as_deref().is_none_or(|t| m.identifier(&a.tag, t))
                    && self.value.as_deref().is_none_or(|v| m.text(&a.value, v))
            });
            if !found {
                return None;
            }
        }
        let mut in_block = Vec::new();
        if let Some(text) = &self.text {
            match object {
                Object::Text(t) if m.text(&t.contents, text) => {}
                Object::MText(t) if m.text(&t.contents, text) => {}
                Object::Table(t) if t.cells.iter().flatten().any(|c| m.text(&c.text, text)) => {}
                Object::MLeader(l) if l.content.text().is_some_and(|t| m.text(&t.text, text)) => {}
                Object::Leader(l)
                    if l.annotation.as_ref().is_some_and(|a| m.text(&a.text, text)) => {}
                Object::Insert(insert) => {
                    in_block = block_text(record, insert, |c| m.text(c, text));
                    if in_block.is_empty() {
                        return None;
                    }
                }
                _ => return None,
            }
        }
        Some(in_block)
    }

    /// Whether the selection looks at where objects are: a box, or an
    /// object they touch.
    fn is_spatial(&self) -> bool {
        self.window.is_some() || self.crossing.is_some() || self.touching.is_some()
    }

    /// Where the selection looks, in words, such as "within the window from
    /// [0, 0] to [10, 5]"; none when it does not look at where objects are.
    fn where_words(&self) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(window) = &self.window {
            parts.push(format!("within the window {}", window.words()));
        }
        if let Some(crossing) = &self.crossing {
            parts.push(format!("within or crossing the box {}", crossing.words()));
        }
        if let Some(space) = &self.space {
            parts.push(if space == crate::description::layout::MODEL {
                "in model space".to_owned()
            } else {
                format!("in the paper space of `{space}`")
            });
        }
        if let Some(touching) = &self.touching {
            parts.push(match (&touching.id, &touching.name) {
                (Some(id), _) => format!("touching `{id}`"),
                (None, Some(name)) => format!("touching the object named `{name}`"),
                (None, None) => "touching no object".to_owned(),
            });
        }
        (!parts.is_empty()).then(|| parts.join(" and "))
    }

    /// The selection in words, such as "the text `NOTES`"; none when it
    /// gives no part but where objects are.
    fn words(&self) -> Option<String> {
        let contains = self.matching == Matching::Contains;
        let mut parts = Vec::new();
        let say = |exact: String, containing: String| if contains { containing } else { exact };
        if let Some(id) = &self.id {
            parts.push(say(
                format!("the ID `{id}`"),
                format!("an ID containing `{id}`"),
            ));
        }
        if let Some(name) = &self.name {
            parts.push(say(
                format!("the name `{name}`"),
                format!("a name containing `{name}`"),
            ));
        }
        if let Some(block) = &self.block {
            parts.push(say(
                format!("an insert of block `{block}`"),
                format!("an insert of a block whose name contains `{block}`"),
            ));
        }
        match (&self.tag, &self.value) {
            (Some(tag), Some(value)) => parts.push(say(
                format!("the attribute `{tag}` with the value `{value}`"),
                format!("an attribute whose tag contains `{tag}` and value contains `{value}`"),
            )),
            (Some(tag), None) => parts.push(say(
                format!("the attribute `{tag}`"),
                format!("an attribute whose tag contains `{tag}`"),
            )),
            (None, Some(value)) => parts.push(say(
                format!("an attribute with the value `{value}`"),
                format!("an attribute whose value contains `{value}`"),
            )),
            (None, None) => {}
        }
        if let Some(text) = &self.text {
            parts.push(say(
                format!("the text `{text}`"),
                format!("text containing `{text}`"),
            ));
        }
        if let Some(group) = &self.group {
            parts.push(say(
                format!("membership in group `{group}`"),
                format!("membership in a group whose name contains `{group}`"),
            ));
        }
        if parts.is_empty() {
            return None;
        }
        let joined = parts.join(" and ");
        Some(if contains {
            format!("{joined}, in any case")
        } else {
            joined
        })
    }
}

/// The space the sheet's object at `index` is in, on a sheet drawn in a
/// layout: `model`, or the name of the layout whose paper space holds it.
/// None on a sheet drawn in model space alone.
pub(crate) fn space_of(sheet: &Sheet, index: usize) -> Option<String> {
    sheet.layout.as_ref()?;
    Some(sheet.layout_of(index).map_or_else(
        || crate::description::layout::MODEL.to_owned(),
        |l| l.name.clone(),
    ))
}

/// The text and multiline text of `insert`'s block (evaluated at `insert.state`),
/// as the block library or DWG of `record`'s sheet defines it, and the values
/// of its constant attributes, which its definition holds, whose contents
/// `wanted` takes.
fn block_text(
    record: &SheetRecord,
    insert: &crate::model::Insert,
    wanted: impl Fn(&str) -> bool,
) -> Vec<BlockText> {
    let block_name = insert.evaluated_block.as_deref().unwrap_or(&insert.block);
    let bi = record
        .sheet
        .blocks
        .iter()
        .position(|b| b.name == block_name)
        .unwrap_or(0);
    let Some(definition) = record.sheet.block_for_insert(insert) else {
        return Vec::new();
    };
    let file = if record.is_dwg() {
        record.file.display().to_string()
    } else {
        record
            .sheet
            .block_library
            .as_ref()
            .map(|written| {
                let read = record.file.parent().unwrap_or(Path::new("")).join(written);
                worked_out(&read).display().to_string()
            })
            .unwrap_or_default()
    };
    definition
        .objects
        .iter()
        .enumerate()
        .filter_map(|(i, object)| {
            let contents = match object {
                Object::Text(t) => &t.contents,
                Object::MText(t) => &t.contents,
                Object::AttributeDefinition(d) if d.constant => &d.default,
                _ => return None,
            };
            wanted(contents).then(|| {
                if record.is_dwg() {
                    BlockText {
                        file: file.clone(),
                        field: None,
                        handle: record.block_handle_of(bi, i).map(str::to_owned),
                        contents: contents.clone(),
                    }
                } else {
                    BlockText {
                        file: file.clone(),
                        field: Some(format!("blocks.{}.objects[{i}]", definition.name)),
                        handle: None,
                        contents: contents.clone(),
                    }
                }
            })
        })
        .collect()
}

/// `path`, the file a sheet's block library was read from, with its `.`
/// and `..` worked out as written, so that the block library a template in
/// `templates/` names as `../blocks.yaml` is `blocks.yaml` in the set's
/// folder, as the set's other files are given. Worked out so, a path can
/// name another file than the one read, as when `templates/` is a link to
/// another folder; then `path` is given as it was read.
fn worked_out(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir
                if matches!(out.components().next_back(), Some(Component::Normal(_))) =>
            {
                out.pop();
            }
            other => out.push(other),
        }
    }
    if is_same_file(&out, path) {
        out
    } else {
        path.to_owned()
    }
}

/// One object a selection matched, on one of the sheets looked at.
struct Matched<'a> {
    record: &'a SheetRecord,
    index: usize,
    /// The text of its block that matched, when it is an insert found by
    /// the text its block draws.
    block_text: Vec<BlockText>,
    /// Where it meets the object the selection's `touching` names.
    touches: Vec<Touch>,
}

/// Each object on `sheets` that `selection` matches, in order. A box and
/// `touching` look at each sheet as it draws and as its geometry is read,
/// with its text measured in the fonts `settings` names.
fn matched<'a>(
    sheets: &'a [&'a SheetRecord],
    selection: &'a Selection,
    settings: &'a Settings,
) -> impl Iterator<Item = Matched<'a>> + 'a {
    sheets.iter().flat_map(move |record| {
        let space = selection
            .is_spatial()
            .then(|| Space::of(record, selection, settings));
        (0..record.objects.len()).filter_map(move |index| {
            let block_text = selection.matches(record, index)?;
            let touches = match &space {
                Some(space) => space.matches(selection, index)?,
                None => Vec::new(),
            };
            Some(Matched {
                record,
                index,
                block_text,
                touches,
            })
        })
    })
}

/// Selects matching object indices on a single sheet using the same rules as
/// `query find` / `query attributes` (including spatial `window`, `crossing`,
/// and `touching`).
pub(crate) fn select_on_sheet(
    selection: &Selection,
    record: &SheetRecord,
    settings: &Settings,
) -> Result<Vec<usize>, Error> {
    check_touching(&[record], selection)?;
    let space = selection
        .is_spatial()
        .then(|| Space::of(record, selection, settings));
    let mut out = Vec::new();
    for index in 0..record.objects.len() {
        if selection.matches(record, index).is_none() {
            continue;
        }
        if let Some(space) = &space {
            if space.matches(selection, index).is_none() {
                continue;
            }
        }
        out.push(index);
    }
    Ok(out)
}

/// Human-readable description of a [`Selection`] for batch operation messages.
pub(crate) fn selection_words(selection: &Selection) -> String {
    match (selection.words(), selection.where_words()) {
        (Some(w), Some(wh)) => format!("{w} {wh}"),
        (Some(w), None) => w,
        (None, Some(wh)) => wh,
        (None, None) => "every insert".to_owned(),
    }
}

/// Whether the object a selection's `touching` names is on one of
/// `sheets`, by its ID or by its name.
fn touching_found(sheets: &[&SheetRecord], selection: &Selection) -> bool {
    let Some(named) = &selection.touching else {
        return true;
    };
    sheets
        .iter()
        .any(|record| named_on(record, named).is_some())
}

/// Validates `selection.touching`: when it names an object by `name` and more
/// than one object on a single sheet carries that name (as can happen on a
/// DWG), returns an error listing all matching IDs so the caller can
/// disambiguate with `--id`.
fn check_touching(sheets: &[&SheetRecord], selection: &Selection) -> Result<(), Error> {
    let Some(named) = &selection.touching else {
        return Ok(());
    };
    if named.id.is_some() {
        return Ok(());
    }
    let Some(name) = named.name.as_deref() else {
        return Ok(());
    };
    for record in sheets {
        let matches: Vec<String> = (0..record.objects.len())
            .filter(|&i| {
                record.objects[i]
                    .name
                    .as_deref()
                    .is_some_and(|have| Matching::Exact.identifier(have, name))
            })
            .map(|i| record.id(i))
            .collect();
        if matches.len() > 1 {
            return Err(Error::new(
                ErrorCode::BadValue,
                format!(
                    "{} is ambiguous on sheet `{}`: {} objects carry it ({}); give one object's \
                     ID instead",
                    named.words(),
                    record.name,
                    matches.len(),
                    matches.join(", ")
                ),
            )
            .at_field("query.touching.name"));
        }
    }
    Ok(())
}

/// The place on `record`'s sheet of the object `named` names, by its ID or
/// by its name, in any case.
fn named_on(record: &SheetRecord, named: &ObjectRef) -> Option<usize> {
    (0..record.objects.len()).find(|&i| match (&named.id, &named.name) {
        (Some(id), _) => Matching::Exact.identifier(&record.id(i), id),
        (None, Some(name)) => record.objects[i]
            .name
            .as_deref()
            .is_some_and(|have| Matching::Exact.identifier(have, name)),
        (None, None) => false,
    })
}

/// What a selection's box and `touching` look at on one sheet: what each
/// of its objects draws, and where each meets the object `touching` names.
struct Space {
    /// For each of the sheet's objects, what it draws, when a box is given.
    drawn: Vec<Drawn>,
    /// For each of the sheet's objects, where it meets the object
    /// `touching` names; none when the sheet does not hold that object.
    touching: Option<Vec<Vec<Touch>>>,
}

/// What one object draws, as the one drawing pipeline draws it to measure
/// the sheet, seen from above.
#[derive(Default)]
struct Drawn {
    /// Its lines, each a run of points; a run of one point is a dot.
    runs: Vec<Vec<draw::Point2>>,
    /// Its filled areas' outlines, each with the rule of which points are
    /// inside.
    fills: Vec<(draw::FillRule, Vec<Vec<draw::Point2>>)>,
    /// Its hatches' boundary loops, each closed: a hatch is selected by its
    /// boundary as well as its pattern's lines, which may stop short of it.
    hatches: Vec<Vec<Vec<draw::Point2>>>,
    /// The band each wide arc segment of a polyline or a donut fills, top
    /// level, in a block or in a multileader's content, which a crossing
    /// takes it by anywhere inside, as AutoCAD 2026's did: it fills a wide
    /// arc in pieces too fine for a box inside it to miss their edges.
    arcs: Vec<Vec<draw::Point2>>,
}

impl Space {
    fn of(record: &SheetRecord, selection: &Selection, settings: &Settings) -> Space {
        let fonts = SheetFonts::find(&record.sheet, settings);
        let mut drawn: Vec<Drawn> = Vec::new();
        if selection.window.is_some() || selection.crossing.is_some() {
            drawn = (0..record.objects.len())
                .map(|_| Drawn::default())
                .collect();
            // Each object is selected where it is in its own space: on a
            // sheet drawn in a layout, model space's at full size and paper
            // space's on the paper.
            let list = measuring_objects(&record.sheet, &fonts);
            for item in list.items {
                // An older leader's annotation is an MTEXT of its own in
                // the DWG, which AutoCAD selects apart from its LEADER: the
                // leader is selected by its own lines and arrow.
                let annotation = matches!(
                    record.sheet.objects.get(item.object),
                    Some(Object::Leader(_))
                ) && item.part.is_some();
                let Some(d) = drawn.get_mut(item.object).filter(|_| !annotation) else {
                    continue;
                };
                match item.shape {
                    draw::Shape::Stroke { runs, .. } => d.runs.extend(runs),
                    draw::Shape::Fill { rule, contours } => d.fills.push((rule, contours)),
                    draw::Shape::Mask { contours } => {
                        d.fills.push((draw::FillRule::EvenOdd, contours))
                    }
                    draw::Shape::Image {
                        origin,
                        u_axis,
                        v_axis,
                        clip,
                        clip_inverted,
                        ..
                    } => {
                        let outline = draw::Shape::image_outline(
                            origin,
                            u_axis,
                            v_axis,
                            clip.as_deref(),
                            clip_inverted,
                        );
                        d.fills.push((draw::FillRule::EvenOdd, vec![outline]));
                    }
                }
            }
            for (object, loops) in list.hatch_loops {
                if let Some(d) = drawn.get_mut(object) {
                    d.hatches.push(loops);
                }
            }
            for (object, band) in list.wide_arcs {
                if let Some(d) = drawn.get_mut(object) {
                    d.arcs.push(band);
                }
            }
        }
        let touching = selection.touching.as_ref().and_then(|named| {
            let index = named_on(record, named)?;
            let reader = crate::geometry::Reader {
                sheet: &record.sheet,
                fonts: &fonts,
            };
            let theirs = reader.parts(&record.sheet.objects[index]);
            let met = record
                .sheet
                .objects
                .iter()
                .enumerate()
                .map(|(i, object)| {
                    if i == index || theirs.is_empty() || !record.sheet.same_space(i, index) {
                        return Vec::new();
                    }
                    touches(&reader.parts(object), &theirs)
                })
                .collect();
            Some(met)
        });
        Space { drawn, touching }
    }

    /// Whether the object at `index` lies where `selection` looks: none
    /// when it does not; when it does, where it meets the object the
    /// selection's `touching` names, empty without one.
    fn matches(&self, selection: &Selection, index: usize) -> Option<Vec<Touch>> {
        if let Some(window) = &selection.window {
            if !self.drawn[index].within(window.bounds()) {
                return None;
            }
        }
        if let Some(crossing) = &selection.crossing {
            if !self.drawn[index].reaches(crossing.bounds()) {
                return None;
            }
        }
        if selection.touching.is_some() {
            let met = &self.touching.as_ref()?[index];
            if met.is_empty() {
                return None;
            }
            return Some(met.clone());
        }
        Some(Vec::new())
    }
}

impl Drawn {
    /// Every point it draws through, its hatches' boundaries among them.
    fn points(&self) -> impl Iterator<Item = draw::Point2> + '_ {
        self.runs
            .iter()
            .flatten()
            .chain(self.fills.iter().flat_map(|(_, c)| c.iter().flatten()))
            .chain(self.hatches.iter().flatten().flatten())
            .copied()
    }

    /// Whether it draws something and everything it draws is within the
    /// box from `min` to `max`, edges included: AutoCAD's window.
    fn within(&self, (min, max): ([f64; 2], [f64; 2])) -> bool {
        let mut any = false;
        for p in self.points() {
            if !inside_box(p, min, max) {
                return false;
            }
            any = true;
        }
        any
    }

    /// Whether something it draws is within the box from `min` to `max` or
    /// crosses its edge: a line, a hatch's pattern line or boundary, or the
    /// outline of a filled area, each wide segment's band its own, so the
    /// seam where two meet is an edge, as AutoCAD 2026's crossing took
    /// them; or whether the box is inside a wide arc's band. Otherwise a box
    /// wholly inside a filled area crosses nothing: inside a solid, a
    /// straight band or a hatch, solid or patterned, AutoCAD took none but
    /// across the inner edges of the triangles it fills them with, its own.
    fn reaches(&self, (min, max): ([f64; 2], [f64; 2])) -> bool {
        if self.points().any(|p| inside_box(p, min, max)) {
            return true;
        }
        let crosses = |run: &[draw::Point2], closed: bool| {
            let n = run.len();
            let edges = if closed { n } else { n.saturating_sub(1) };
            (0..edges).any(|i| segment_meets_box(run[i], run[(i + 1) % n], min, max))
        };
        if self.runs.iter().any(|run| crosses(run, false)) {
            return true;
        }
        if self
            .fills
            .iter()
            .any(|(_, contours)| contours.iter().any(|c| crosses(c, true)))
        {
            return true;
        }
        if self
            .arcs
            .iter()
            .any(|band| draw::inside(min, draw::FillRule::NonZero, std::slice::from_ref(band)))
        {
            return true;
        }
        self.hatches
            .iter()
            .any(|loops| loops.iter().any(|l| crosses(l, true)))
    }
}

/// Whether `p` is within the box from `min` to `max`, edges included.
fn inside_box(p: draw::Point2, min: [f64; 2], max: [f64; 2]) -> bool {
    p[0] >= min[0] && p[0] <= max[0] && p[1] >= min[1] && p[1] <= max[1]
}

/// Whether the segment from `a` to `b` meets the box from `min` to `max`,
/// edges included: the segment clipped to the box, as Liang and Barsky clip
/// it, leaves something.
fn segment_meets_box(a: draw::Point2, b: draw::Point2, min: [f64; 2], max: [f64; 2]) -> bool {
    let d = [b[0] - a[0], b[1] - a[1]];
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    for axis in 0..2 {
        for (p, q) in [
            (-d[axis], a[axis] - min[axis]),
            (d[axis], max[axis] - a[axis]),
        ] {
            if p == 0.0 {
                if q < 0.0 {
                    return false;
                }
            } else {
                let r = q / p;
                if p < 0.0 {
                    t0 = t0.max(r);
                } else {
                    t1 = t1.min(r);
                }
                if t0 > t1 {
                    return false;
                }
            }
        }
    }
    true
}

/// Where the object at `index` of `record`'s sheet is written.
pub(crate) fn written_at(record: &SheetRecord, index: usize) -> WrittenAt {
    if record.is_dwg() {
        return WrittenAt {
            file: record.file.display().to_string(),
            field: None,
            line: None,
            column: None,
            handle: record.handle_of(index).map(str::to_owned),
        };
    }
    let object = &record.objects[index].object;
    WrittenAt {
        file: record.file_of(index).display().to_string(),
        field: Some(object.field.clone()),
        line: Some(object.line),
        column: Some(object.column),
        handle: None,
    }
}

/// What a query found, in words: `count` of `thing` that match
/// `selection` on `sheets`, which `on` names; or that none did.
fn found_message(
    count: usize,
    thing: &str,
    selection: &Selection,
    sheets: &[&SheetRecord],
    on: &str,
) -> String {
    if let (0, Some(touching)) = (count, &selection.touching) {
        if !touching_found(sheets, selection) {
            return format!(
                "nothing matches: no object {on} has {}, the object to touch",
                touching.words()
            );
        }
    }
    match (count, selection.words(), selection.where_words()) {
        (0, Some(has), Some(lies)) => {
            format!("nothing matches: no {thing} {on} has {has} and is {lies}")
        }
        (0, Some(has), None) => format!("nothing matches: no {thing} {on} has {has}"),
        (0, None, Some(lies)) => format!("nothing matches: no {thing} {on} is {lies}"),
        (0, None, None) => format!("nothing matches: there is no {thing} {on}"),
        (1, Some(has), Some(lies)) => format!("1 {thing} has {has} and is {lies}, {on}"),
        (1, Some(has), None) => format!("1 {thing} has {has}, {on}"),
        (1, None, Some(lies)) => format!("1 {thing} is {lies}, {on}"),
        (count, Some(has), Some(lies)) => {
            format!("{count} {thing}s have {has} and are {lies}, {on}")
        }
        (count, Some(has), None) => format!("{count} {thing}s have {has}, {on}"),
        (count, None, Some(lies)) => format!("{count} {thing}s are {lies}, {on}"),
        (count, None, None) => format!("{}, {on}", counted(count, thing)),
    }
}

/// `count` things, such as "3 objects" or "1 object".
fn counted(count: usize, thing: &str) -> String {
    if count == 1 {
        format!("1 {thing}")
    } else {
        format!("{count} {thing}s")
    }
}

fn find(
    sheets: &[&SheetRecord],
    selection: &Selection,
    settings: &Settings,
    looked: &LookedAt,
) -> (String, Answer) {
    let objects: Vec<FoundObject> = matched(sheets, selection, settings)
        .map(|m| FoundObject {
            sheet: m.record.name.clone(),
            id: m.record.id(m.index),
            id_written: m.record.id_written(m.index),
            written_at: written_at(m.record, m.index),
            space: space_of(&m.record.sheet, m.index),
            object: m.record.sheet.objects[m.index].clone(),
            block_text: m.block_text,
            touches: m.touches,
        })
        .collect();
    let count = objects.len();
    let message = found_message(count, "object", selection, sheets, &looked.on());
    (message, Answer::Find { count, objects })
}

/// Each sheet's layers, layer `0` first.
fn layers(sheets: &[&SheetRecord], looked: &LookedAt) -> (String, Answer) {
    let listed: Vec<SheetLayers> = sheets
        .iter()
        .map(|record| {
            let sheet = &record.sheet;
            let zero = sheet
                .layers
                .iter()
                .find(|l| l.name == LAYER_ZERO)
                .cloned()
                .unwrap_or_else(|| Layer::new(LAYER_ZERO));
            let layers = std::iter::once(zero)
                .chain(
                    sheet
                        .layers
                        .iter()
                        .filter(|l| l.name != LAYER_ZERO)
                        .cloned(),
                )
                .map(|layer| LayerUse {
                    objects: sheet
                        .objects
                        .iter()
                        .filter(|o| o.layer() == layer.name)
                        .count(),
                    layer,
                })
                .collect();
            SheetLayers {
                sheet: record.name.clone(),
                layers,
            }
        })
        .collect();
    let count: usize = listed.iter().map(|s| s.layers.len()).sum();
    let not_drawn: Vec<String> = listed
        .iter()
        .flat_map(|s| &s.layers)
        .filter(|l| !l.layer.on || l.layer.frozen || !l.layer.plot)
        .map(|l| l.layer.name.clone())
        .fold(Vec::new(), |mut names, name| {
            if !names.contains(&name) {
                names.push(name);
            }
            names
        });
    let message = format!(
        "{} {}{}",
        counted(count, "layer"),
        looked.on(),
        if not_drawn.is_empty() {
            String::new()
        } else {
            format!("; off, frozen or not plotting: {}", not_drawn.join(", "))
        }
    );
    (
        message,
        Answer::Layers {
            count,
            layers: listed,
        },
    )
}

fn groups(sheets: &[&SheetRecord], looked: &LookedAt) -> (String, Answer) {
    let listed: Vec<SheetGroups> = sheets
        .iter()
        .map(|record| {
            let groups = record
                .sheet
                .groups
                .iter()
                .map(|g| {
                    let objects: Vec<GroupMember> = g
                        .objects
                        .iter()
                        .filter_map(|&idx| {
                            let obj = record.sheet.objects.get(idx)?;
                            Some(GroupMember {
                                id: record.id(idx),
                                name: record.objects.get(idx).and_then(|o| o.name.clone()),
                                kind: obj.type_name().to_owned(),
                            })
                        })
                        .collect();
                    GroupFound {
                        name: g.name.clone(),
                        description: g.description.clone(),
                        selectable: g.selectable,
                        unnamed: g.unnamed,
                        count: objects.len(),
                        objects,
                    }
                })
                .collect();
            SheetGroups {
                sheet: record.name.clone(),
                groups,
            }
        })
        .collect();
    let count: usize = listed.iter().map(|s| s.groups.len()).sum();
    let message = if count == 0 {
        format!("nothing matches: no groups {}", looked.on())
    } else {
        let names: Vec<&str> = listed
            .iter()
            .flat_map(|s| s.groups.iter().map(|g| g.name.as_str()))
            .collect();
        format!(
            "{} {}: {}",
            counted(count, "group"),
            looked.on(),
            names.join(", ")
        )
    };
    (
        message,
        Answer::Groups {
            count,
            groups: listed,
        },
    )
}

fn annotation_scales(sheets: &[&SheetRecord], looked: &LookedAt) -> (String, Answer) {
    let listed: Vec<SheetAnnotationScales> = sheets
        .iter()
        .map(|record| {
            let sheet = &record.sheet;
            let active = sheet.active_annotation_scale();
            let raw_scales = if sheet.annotation_scales.is_empty() {
                vec![active.clone()]
            } else {
                let mut list = sheet.annotation_scales.clone();
                if !list
                    .iter()
                    .any(|s| s.name.eq_ignore_ascii_case(&active.name))
                {
                    list.insert(0, active.clone());
                }
                list
            };
            let scales: Vec<AnnotationScaleFound> = raw_scales
                .into_iter()
                .map(|s| AnnotationScaleFound {
                    factor: s.factor(),
                    name: s.name,
                    paper_units: s.paper_units,
                    drawing_units: s.drawing_units,
                })
                .collect();
            let objects: Vec<AnnotativeObjectFound> = sheet
                .objects
                .iter()
                .enumerate()
                .filter(|(_, obj)| sheet.object_is_annotative(obj))
                .map(|(idx, obj)| AnnotativeObjectFound {
                    id: record.id(idx),
                    name: record.objects.get(idx).and_then(|o| o.name.clone()),
                    kind: obj.type_name().to_owned(),
                    scales: sheet
                        .object_annotation_scales(obj)
                        .into_iter()
                        .map(|s| s.name)
                        .collect(),
                })
                .collect();
            let active_factor = active.factor();
            SheetAnnotationScales {
                sheet: record.name.clone(),
                current: active.name,
                factor: active_factor,
                anno_all_visible: sheet.anno_all_visible,
                scales,
                objects,
            }
        })
        .collect();
    let count: usize = listed.iter().map(|s| s.objects.len()).sum();
    let message = match listed.as_slice() {
        [one] => format!(
            "sheet `{}` active annotation scale is `{}` with {} and {}",
            one.sheet,
            one.current,
            counted(one.scales.len(), "scale"),
            counted(one.objects.len(), "annotative object"),
        ),
        _ => format!("{} {}", counted(count, "annotative object"), looked.on(),),
    };
    (
        message,
        Answer::AnnotationScales {
            count,
            annotation_scales: listed,
        },
    )
}

fn blocks(sheets: &[&SheetRecord], looked: &LookedAt) -> (String, Answer) {
    let mut count = 0;
    let mut names: Vec<String> = Vec::new();
    let listed: Vec<SheetBlocks> = sheets
        .iter()
        .map(|record| {
            let mut blocks: Vec<BlockInserts> = Vec::new();
            for (index, object) in record.sheet.objects.iter().enumerate() {
                let Object::Insert(insert) = object else {
                    continue;
                };
                count += 1;
                if !names.contains(&insert.block) {
                    names.push(insert.block.clone());
                }
                match blocks.iter_mut().find(|b| b.block == insert.block) {
                    Some(block) => block.inserts.push(record.id(index)),
                    None => blocks.push(BlockInserts {
                        block: insert.block.clone(),
                        inserts: vec![record.id(index)],
                    }),
                }
            }
            SheetBlocks {
                sheet: record.name.clone(),
                blocks,
            }
        })
        .collect();
    let message = if count == 0 {
        format!("nothing matches: no block is inserted {}", looked.on())
    } else {
        format!(
            "{} of {} {}: {}",
            counted(count, "insert"),
            counted(names.len(), "block"),
            looked.on(),
            names.join(", ")
        )
    };
    (
        message,
        Answer::Blocks {
            count,
            blocks: listed,
        },
    )
}

fn attributes(
    sheets: &[&SheetRecord],
    selection: &Selection,
    settings: &Settings,
    looked: &LookedAt,
) -> (String, Answer) {
    let inserts: Vec<InsertAttributes> = matched(sheets, selection, settings)
        .filter_map(|m| match &m.record.sheet.objects[m.index] {
            Object::Insert(insert) => Some(InsertAttributes {
                sheet: m.record.name.clone(),
                id: m.record.id(m.index),
                name: insert.name.clone(),
                block: insert.block.clone(),
                attributes: AttributeValues(insert.attributes.clone()),
                written_at: written_at(m.record, m.index),
                block_text: m.block_text,
            }),
            _ => None,
        })
        .collect();
    let count = inserts.len();
    let message = found_message(count, "insert", selection, sheets, &looked.on());
    (message, Answer::Attributes { count, inserts })
}

fn measure(
    sheets: &[&SheetRecord],
    measure: &Measure,
    units: Option<Unit>,
    looked: &LookedAt,
) -> Result<(String, Answer), Error> {
    let from = end(sheets, &measure.from, "query.from")?;
    let to = end(sheets, &measure.to, "query.to")?;
    let ((from_record, from_index, from), (to_record, to_index, to)) = match (from, to) {
        (Some(from), Some(to)) => (from, to),
        (from, to) => {
            let mut missing = Vec::new();
            if from.is_none() {
                missing.push(format!("{}, the end to measure from", measure.from.words()));
            }
            if to.is_none() {
                missing.push(format!("{}, the end to measure to", measure.to.words()));
            }
            let message = format!(
                "nothing matches: no object {} has {}",
                looked.on(),
                missing.join(", nor ")
            );
            let answer = Answer::Measure {
                distance: None,
                delta: None,
                from: from.map(|(_, _, p)| Box::new(p)),
                to: to.map(|(_, _, p)| Box::new(p)),
            };
            return Ok((message, answer));
        }
    };
    if from.sheet != to.sheet {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "`{}` is on sheet `{}` and `{}` on sheet `{}`; a distance is measured between \
                 two objects on one sheet",
                from.id, from.sheet, to.id, to.sheet
            ),
        )
        .at_field("query.to"));
    }
    if !from_record.sheet.same_space(from_index, to_index) {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "`{}` is in {} and `{}` is in {}; a distance is measured between two objects in \
                 the same space",
                from.id,
                from_record.sheet.space_words(from_index),
                to.id,
                to_record.sheet.space_words(to_index)
            ),
        )
        .at_field("query.to"));
    }
    let delta = [
        to.at.x - from.at.x,
        to.at.y - from.at.y,
        to.at.z - from.at.z,
    ];
    let distance = delta.iter().map(|d| d * d).sum::<f64>().sqrt();
    let u = units_words(units);
    let message = format!(
        "{distance} {u} from the {} of `{}` to the {} of `{}`, on sheet `{}`",
        from.point, from.id, to.point, to.id, from.sheet
    );
    Ok((
        message,
        Answer::Measure {
            distance: Some(distance),
            delta: Some(delta),
            from: Some(Box::new(from)),
            to: Some(Box::new(to)),
        },
    ))
}

impl End {
    /// The object the end names, in words, such as "the ID `DEMO-001/border`".
    fn words(&self) -> String {
        match (&self.id, &self.name) {
            (Some(id), _) => format!("the ID `{id}`"),
            (None, Some(name)) => format!("the name `{name}`"),
            (None, None) => "no object".to_owned(),
        }
    }
}

/// The point one end of a distance names, or none when no object on
/// `sheets` has the ID or name it gives. `field` is the end's place in the
/// request, for an error.
fn end<'a>(
    sheets: &[&'a SheetRecord],
    end: &End,
    field: &str,
) -> Result<Option<(&'a SheetRecord, usize, MeasuredPoint)>, Error> {
    let named = ObjectRef {
        id: end.id.clone(),
        name: end.name.clone(),
    };
    let Some((record, index)) =
        object_named(sheets, &named, field, "each end of a distance", "measure")?
    else {
        return Ok(None);
    };
    let object = &record.sheet.objects[index];
    let id = record.id(index);
    let points = points_of(object);
    let (point, at) = match &end.point {
        None => points
            .first()
            .cloned()
            .ok_or_else(|| no_point(object, &id, field))?,
        Some(wanted) => points
            .iter()
            .find(|(name, _)| name == wanted)
            .cloned()
            .ok_or_else(|| {
                if points.is_empty() {
                    return no_point(object, &id, field);
                }
                Error::new(
                    ErrorCode::UnknownName,
                    format!(
                        "{} `{id}` has no point `{wanted}`; its points: {}",
                        article(object.type_name()),
                        point_choices(&points)
                    ),
                )
                .at_field(format!("{field}.point"))
            })?,
    };
    Ok(Some((
        record,
        index,
        MeasuredPoint {
            sheet: record.name.clone(),
            id,
            name: record.objects[index].name.clone(),
            kind: object.type_name().to_owned(),
            point,
            at,
        },
    )))
}

/// The error for an object with no point of its own to measure from.
fn no_point(object: &Object, id: &str, field: &str) -> Error {
    Error::new(
        ErrorCode::BadValue,
        format!(
            "{} `{id}` has no point of its own to measure from; measure from an object at its \
             boundary",
            article(object.type_name())
        ),
    )
    .at_field(field.to_owned())
}

/// "a line", "an insert".
pub(crate) fn article(kind: &str) -> String {
    if kind.starts_with(['a', 'e', 'i', 'o', 'u']) || kind == "mtext" || kind == "mleader" {
        format!("an {kind}")
    } else {
        format!("a {kind}")
    }
}

/// The object `object` names on `sheets`, by its ID or by its name; none
/// when no object there has it. `field` is its place in the request, for an
/// error; `naming` says what names it, such as "each end of a distance",
/// and `doing` what is done on the sheet it is on, such as "measure".
pub(crate) fn object_named<'a>(
    sheets: &[&'a SheetRecord],
    object: &ObjectRef,
    field: &str,
    naming: &str,
    doing: &str,
) -> Result<Option<(&'a SheetRecord, usize)>, Error> {
    let found: Vec<(&SheetRecord, usize)> = match (&object.id, &object.name) {
        (Some(id), None) => sheets
            .iter()
            .flat_map(|record| {
                (0..record.objects.len())
                    .filter(|&i| Matching::Exact.identifier(&record.id(i), id))
                    .map(|i| (*record, i))
            })
            .take(1)
            .collect(),
        (None, Some(name)) => sheets
            .iter()
            .flat_map(|record| {
                record
                    .objects
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| {
                        r.name
                            .as_deref()
                            .is_some_and(|have| Matching::Exact.identifier(have, name))
                    })
                    .map(move |(i, _)| (*record, i))
            })
            .collect(),
        _ => {
            return Err(Error::usage(format!(
                "{naming} names its object by `id` or by `name`, and not both"
            ))
            .at_field(field.to_owned()))
        }
    };
    match found.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some(*one)),
        several if several.iter().all(|(r, _)| r.name == several[0].0.name) => {
            let ids: Vec<String> = several.iter().map(|(r, i)| r.id(*i)).collect();
            Err(Error::new(
                ErrorCode::BadValue,
                format!(
                    "{} is ambiguous on sheet `{}`: {} objects carry it ({}); give one object's \
                     ID instead",
                    object.words(),
                    several[0].0.name,
                    several.len(),
                    ids.join(", ")
                ),
            )
            .at_field(format!("{field}.name")))
        }
        several => Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "{} is given on {} sheets, {}, and a name is given once on a sheet; name the \
                 sheet to {doing} on, or give the object's ID",
                object.words(),
                several.len(),
                several
                    .iter()
                    .map(|(r, _)| r.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
        .at_field(format!("{field}.name"))),
    }
}

/// The points of an object a distance can be measured from, each named as
/// its key in the sheet description, the object's own point first.
fn points_of(object: &Object) -> Vec<(String, Point3)> {
    let listed = |key: &str, points: &[Point3]| -> Vec<(String, Point3)> {
        points
            .iter()
            .enumerate()
            .map(|(i, p)| (format!("{key}[{i}]"), *p))
            .collect()
    };
    let one = |key: &str, point: Point3| vec![(key.to_owned(), point)];
    match object {
        Object::Line(line) => vec![("from".to_owned(), line.from), ("to".to_owned(), line.to)],
        Object::Polyline(polyline) => listed("points", &polyline.points),
        Object::Arc(arc) => one("center", arc.center),
        Object::Circle(circle) => one("center", circle.center),
        Object::Ellipse(ellipse) => one("center", ellipse.center),
        Object::Spline(spline) if spline.fit_points.is_empty() => {
            listed("control_points", &spline.control_points)
        }
        Object::Spline(spline) => listed("fit_points", &spline.fit_points),
        Object::Hatch(_) => Vec::new(),
        Object::Donut(donut) => one("center", donut.center),
        Object::Solid(solid) => listed("corners", &solid.corners),
        Object::Wipeout(wipeout) => listed("points", &wipeout.points),
        Object::RasterImage(image) => one("position", image.position),
        Object::PdfUnderlay(underlay) => one("position", underlay.position),
        Object::Point(pt) => one("position", pt.position),
        Object::XLine(xl) => one("through", xl.through),
        Object::Ray(ray) => one("from", ray.from),
        Object::MLine(ml) => listed("points", &ml.points),
        Object::Tolerance(tol) => one("position", tol.position),
        Object::CenterMark(cm) => one("center", cm.center),
        Object::CenterLine(cl) => vec![("from".to_owned(), cl.from), ("to".to_owned(), cl.to)],
        Object::Shape(sh) => one("position", sh.position),
        Object::Solid3D(solid) => {
            let (_, center) = crate::geometry::solid3d::solid_key_points(&solid.body);
            one("center", center)
        }
        Object::RevisionCloud(cloud) => match &cloud.boundary {
            crate::model::RevisionCloudBoundary::Rectangle { from, to } => {
                vec![("from".to_owned(), *from), ("to".to_owned(), *to)]
            }
            crate::model::RevisionCloudBoundary::Polygon(points) => listed("polygon", points),
        },
        Object::Viewport(viewport) => one("center", viewport.center),
        Object::Text(text) => one("position", text.position),
        Object::MText(text) => one("position", text.position),
        Object::AttributeDefinition(definition) => one("position", definition.position),
        Object::Insert(insert) => one("position", insert.position),
        Object::Table(table) => one("position", table.position),
        Object::MLeader(mleader) => mleader
            .leaders
            .iter()
            .enumerate()
            .flat_map(|(i, points)| listed(&format!("leaders[{i}]"), points))
            .collect(),
        Object::Leader(leader) => listed("points", &leader.points),
        Object::Dimension(dimension) => {
            let mut points = match &dimension.measured {
                Measured::Linear { points, .. } | Measured::Aligned { points } => {
                    listed("points", points)
                }
                Measured::Radius { center, point } | Measured::Diameter { center, point } => {
                    vec![("center".to_owned(), *center), ("point".to_owned(), *point)]
                }
                Measured::Angular { lines } => {
                    let mut points = listed("lines[0]", &lines[0]);
                    points.extend(listed("lines[1]", &lines[1]));
                    points
                }
                Measured::ThreePointAngular { center, points } => {
                    let mut all = vec![("center".to_owned(), *center)];
                    all.extend(listed("points", points));
                    all
                }
                Measured::Ordinate { feature, .. } => vec![("feature".to_owned(), *feature)],
            };
            points.push(("location".to_owned(), dimension.location));
            points
        }
    }
}

/// An object's points, for an error: a list of points as its first and last.
fn point_choices(points: &[(String, Point3)]) -> String {
    match points {
        [first, .., last] if first.0.ends_with("[0]") => format!("{} to {}", first.0, last.0),
        _ => points
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    }
}

fn extents(
    sheets: &[&SheetRecord],
    settings: &Settings,
    units: Option<Unit>,
    looked: &LookedAt,
) -> (String, Answer) {
    let mut substitutions: Vec<FontSubstitution> = Vec::new();
    let listed: Vec<SheetExtents> = sheets
        .iter()
        .map(|record| {
            let fonts = SheetFonts::find(&record.sheet, settings);
            for substitution in fonts.substitutions() {
                if !substitutions.contains(&substitution) {
                    substitutions.push(substitution);
                }
            }
            SheetExtents {
                sheet: record.name.clone(),
                extents: sheet_extents(record, &fonts),
            }
        })
        .collect();
    let count = listed.iter().filter(|s| s.extents.is_some()).count();
    let u = units_words(units);
    let message = match listed.as_slice() {
        [one] => match &one.extents {
            Some(e) => format!(
                "sheet `{}` draws within {} by {} {u}, from [{}, {}] to [{}, {}]",
                one.sheet, e.width, e.height, e.min[0], e.min[1], e.max[0], e.max[1]
            ),
            None => format!("nothing matches: sheet `{}` draws nothing", one.sheet),
        },
        _ if count == 0 => format!("nothing matches: no sheet draws anything {}", looked.on()),
        all => format!(
            "the extents of {} of the {} sheets looked at, in {u}",
            count,
            all.len()
        ),
    };
    (
        message,
        Answer::Extents {
            count,
            extents: listed,
            font_substitutions: substitutions,
        },
    )
}

/// The box round everything a sheet draws, its curves drawn within
/// [`EXTENTS_TOLERANCE`] of its larger side; none when it draws nothing.
///
/// The extents follow the geometry, as AutoCAD's do, not the dashes a
/// linetype lays along it: the sheet is drawn through the one pipeline with
/// every layer and object continuous. Text still draws as its font draws it.
fn sheet_extents(record: &SheetRecord, fonts: &SheetFonts) -> Option<Extents> {
    measuring_drawing(&record.sheet, fonts)
        .extents()
        .map(|fine| Extents::of(&fine))
}

/// The sheet drawn to measure it: through the one pipeline with every
/// layer and object continuous, so that what it draws follows the geometry, not the
/// dashes a linetype lays along it, with its curves drawn within
/// [`EXTENTS_TOLERANCE`] of its larger side. Text draws as its font draws
/// it. Empty when the sheet draws nothing.
pub(crate) fn measuring_drawing(sheet: &Sheet, fonts: &SheetFonts) -> DisplayList {
    // With no linetype defined, every layer and object draws continuous.
    let mut sheet = sheet.clone();
    sheet.linetypes.clear();
    let sheet = &sheet;
    // A coarse drawing, curves to within a unit, for the sheet's size; then
    // one fine enough to measure it by.
    let coarse = draw::draw_sheet(sheet, fonts, 1.0, MOST_HATCH_DASHES);
    let Some(bounds) = coarse.extents() else {
        return coarse;
    };
    let size = bounds.width().max(bounds.height());
    let tolerance = if size > 0.0 {
        size * EXTENTS_TOLERANCE
    } else {
        EXTENTS_TOLERANCE
    };
    draw::draw_sheet(sheet, fonts, tolerance, MOST_HATCH_DASHES)
}

/// Each of the sheet's objects drawn in its own coordinates, as
/// [`measuring_drawing`] draws a sheet with no layout: on a sheet drawn in
/// a layout, model space's at full size and paper space's on the paper, as
/// a query selects them by a box.
pub(crate) fn measuring_objects(sheet: &Sheet, fonts: &SheetFonts) -> DisplayList {
    if sheet.layout.is_none() {
        return measuring_drawing(sheet, fonts);
    }
    let mut sheet = sheet.clone();
    sheet.linetypes.clear();
    let sheet = &sheet;
    let coarse = draw::draw_objects(sheet, fonts, 1.0, MOST_HATCH_DASHES);
    let Some(bounds) = coarse.extents() else {
        return coarse;
    };
    let size = bounds.width().max(bounds.height());
    let tolerance = if size > 0.0 {
        size * EXTENTS_TOLERANCE
    } else {
        EXTENTS_TOLERANCE
    };
    draw::draw_objects(sheet, fonts, tolerance, MOST_HATCH_DASHES)
}

/// The sheet drawn as it prints, to see what touches what: drawn as
/// [`measuring_drawing`] draws it, its curves as fine, but with its
/// linetypes laid out, so a line is its dashes, and with layers that do not
/// plot left out, as on paper.
pub(crate) fn printing_drawing(sheet: &Sheet, fonts: &SheetFonts) -> DisplayList {
    let coarse = draw::draw_sheet(sheet, fonts, 1.0, MOST_HATCH_DASHES);
    let size = coarse.extents().map_or(0.0, |b| b.width().max(b.height()));
    let tolerance = if size > 0.0 {
        size * EXTENTS_TOLERANCE
    } else {
        EXTENTS_TOLERANCE
    };
    draw::draw_sheet_for(
        sheet,
        fonts,
        tolerance,
        MOST_HATCH_DASHES,
        draw::Purpose::Plot,
    )
}

impl Extents {
    /// The box `bounds` draws within.
    pub(crate) fn of(bounds: &Bounds) -> Extents {
        Extents {
            min: bounds.min,
            max: bounds.max,
            width: bounds.width(),
            height: bounds.height(),
        }
    }
}
