//! Queries: what an agent asks of a drawing set, or of one sheet
//! description, to check its work. Nothing is selected by coordinates.
//!
//! A query reads the records, the sheet descriptions and templates filled
//! in from their rows, not the built DWGs. The records are each sheet's
//! truth (ADR-0003): a query needs no build, cannot read a stale DWG, and
//! answers in the set's declared unit, exactly as the record gives it.
//!
//! - **Find** objects by ID, name, block, attribute or text.
//! - **Blocks**: the blocks each sheet inserts, and the IDs of the inserts.
//! - **Attributes**: the attributes of inserts, as JSON from tag to value.
//! - **Measure** the distance between points of two objects given by ID or
//!   name: an insert's or a text's position, a line's ends, a circle's
//!   centre.
//! - **Extents**: the box round everything each sheet draws, drawn through
//!   the one drawing pipeline (ADR-0011), as AutoCAD's extents are.
//!
//! A query that matches nothing says so and is not an error. A distance to
//! measure that names an ID or a name no object carries is an error, as is
//! a sheet that cannot be read: an answer must not leave a sheet out.

use std::path::{Path, PathBuf};

use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};

use crate::description::{
    read_rows, read_set_settings, read_sheet_record, read_template, SheetRecord,
};
use crate::draw::{self, MOST_HATCH_DASHES};
use crate::error::{Error, ErrorCode};
use crate::model::{Attribute, Object, Point3};
use crate::set::SET_SETTINGS_FILE;
use crate::settings::Settings;
use crate::text::fonts::FontSubstitution;
use crate::text::placement::SheetFonts;
use crate::units::Unit;

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

/// What a query asks. Written as `{"kind": "find", ...}` in JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
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
    /// The drawing set's folder, or the sheet description, queried.
    pub source: String,
    /// The declared unit every length in the answer is in: the set's, or
    /// the sheet description's.
    pub units: Unit,
    /// The sheets looked at, by name, in the set's order.
    pub sheets: Vec<String>,
    /// What was found, in words. When nothing matched, it says so.
    pub message: String,
    #[serde(flatten)]
    pub answer: Answer,
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
    Measure {
        /// The distance from one point to the other, in the declared unit.
        distance: f64,
        /// How far the second point is from the first along X, Y and Z.
        delta: [f64; 3],
        from: Box<MeasuredPoint>,
        to: Box<MeasuredPoint>,
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
}

/// Where an object is written: its file and its place in it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WrittenAt {
    /// The sheet description, or the sheet's template.
    pub file: String,
    /// The object's place in the file, such as `objects[3]`.
    pub field: String,
    /// The line the object starts on, counted from 1.
    pub line: usize,
    /// The column it starts at, counted from 1 in characters.
    pub column: usize,
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
    /// The object, as the sheet holds it, in the declared unit.
    pub object: Object,
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
}

/// An insert's attributes, written in JSON as a mapping from each tag to
/// its value, in the order the block defines them.
#[derive(Clone, Debug, PartialEq)]
pub struct AttributeValues(pub Vec<Attribute>);

impl Serialize for AttributeValues {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for attribute in &self.0 {
            map.serialize_entry(&attribute.tag, &attribute.value)?;
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

/// Queries a drawing set, or one sheet description, from its records,
/// reading fonts from the font folders `settings` names to find extents.
pub fn query_with(request: &QueryRequest, settings: &Settings) -> Result<QueryReport, Error> {
    let source = request.source.as_path();
    let (units, records) = load(source, request.sheet.as_deref())?;
    let looked_at: Vec<&SheetRecord> = records.iter().collect();
    let looked = LookedAt(&looked_at, request.sheet.is_some());
    let (message, answer) = match &request.query {
        Query::Find(selection) => find(&looked_at, selection, &looked),
        Query::Blocks => blocks(&looked_at, &looked),
        Query::Attributes(selection) => attributes(&looked_at, selection, &looked),
        Query::Measure(measure) => self::measure(&looked_at, measure, units, &looked)?,
        Query::Extents => extents(&looked_at, settings, units, &looked),
    };
    Ok(QueryReport {
        source: source.display().to_string(),
        units,
        sheets: looked_at.iter().map(|r| r.name.clone()).collect(),
        message,
        answer,
    })
}

/// Reads the sheets a query looks at, with the declared unit: every sheet
/// of the drawing set whose folder is `source`, or the one sheet
/// description `source` is; or only the one named `sheet`, in any case. A
/// sheet that cannot be read stops the query.
fn load(source: &Path, sheet: Option<&str>) -> Result<(Unit, Vec<SheetRecord>), Error> {
    let named = |name: &str| sheet.is_none_or(|s| s.to_lowercase() == name.to_lowercase());
    if !source.is_dir() {
        let record = read_sheet_record(source)?;
        if !named(&record.name) {
            return Err(no_such_sheet(sheet, &[record.name.as_str()]));
        }
        return Ok((record.sheet.units, vec![record]));
    }
    let set = read_set_settings(&source.join(SET_SETTINGS_FILE))?;
    let rows_file = source.join(&set.rows);
    let rows = read_rows(&rows_file)?;
    let looked_at: Vec<_> = rows.iter().filter(|row| named(&row.sheet)).collect();
    if looked_at.is_empty() {
        let names: Vec<&str> = rows.iter().map(|row| row.sheet.as_str()).collect();
        return Err(no_such_sheet(sheet, &names));
    }
    let records = looked_at
        .into_iter()
        .map(|row| {
            let template = source.join(&row.template);
            read_template(&template, row, &rows_file, set.units, &mut Vec::new()).map_err(|e| {
                Error {
                    message: format!(
                        "sheet `{}` cannot be read, and a query answers for every sheet it \
                         looks at: {}",
                        row.sheet, e.message
                    ),
                    ..e
                }
            })
        })
        .collect::<Result<_, _>>()?;
    Ok((set.units, records))
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
    /// Whether the object at `index` of `record` matches every part given.
    fn matches(&self, record: &SheetRecord, index: usize) -> bool {
        let m = self.matching;
        let object = &record.sheet.objects[index];
        let insert = match object {
            Object::Insert(insert) => Some(insert),
            _ => None,
        };
        if let Some(id) = &self.id {
            if !m.identifier(&record.id(index), id) {
                return false;
            }
        }
        if let Some(name) = &self.name {
            match insert.and_then(|i| i.name.as_deref()) {
                Some(have) if m.identifier(have, name) => {}
                _ => return false,
            }
        }
        if let Some(block) = &self.block {
            match insert {
                Some(insert) if m.identifier(&insert.block, block) => {}
                _ => return false,
            }
        }
        if self.tag.is_some() || self.value.is_some() {
            let Some(insert) = insert else {
                return false;
            };
            let found = insert.attributes.iter().any(|a| {
                self.tag.as_deref().is_none_or(|t| m.identifier(&a.tag, t))
                    && self.value.as_deref().is_none_or(|v| m.text(&a.value, v))
            });
            if !found {
                return false;
            }
        }
        if let Some(text) = &self.text {
            let contents = match object {
                Object::Text(t) => &t.contents,
                Object::MText(t) => &t.contents,
                _ => return false,
            };
            if !m.text(contents, text) {
                return false;
            }
        }
        true
    }

    /// The selection in words, such as "the text `NOTES`"; none when it
    /// has no parts.
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

fn written_at(record: &SheetRecord, index: usize) -> WrittenAt {
    let object = &record.objects[index].object;
    WrittenAt {
        file: record.file.display().to_string(),
        field: object.field.clone(),
        line: object.line,
        column: object.column,
    }
}

/// What a search found, in words: `count` of `thing` that match `words`,
/// the selection in words, on the sheets `on` names; or that none did.
fn found_message(count: usize, thing: &str, words: Option<String>, on: &str) -> String {
    match (count, words) {
        (0, Some(words)) => format!("nothing matches: no {thing} {on} has {words}"),
        (0, None) => format!("nothing matches: there is no {thing} {on}"),
        (1, Some(words)) => format!("1 {thing} has {words}, {on}"),
        (count, Some(words)) => format!("{count} {thing}s have {words}, {on}"),
        (count, None) => format!("{}, {on}", counted(count, thing)),
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

fn find(sheets: &[&SheetRecord], selection: &Selection, looked: &LookedAt) -> (String, Answer) {
    let mut objects = Vec::new();
    for record in sheets {
        for (index, object) in record.sheet.objects.iter().enumerate() {
            if selection.matches(record, index) {
                objects.push(FoundObject {
                    sheet: record.name.clone(),
                    id: record.id(index),
                    id_written: record.objects[index].written(),
                    written_at: written_at(record, index),
                    object: object.clone(),
                });
            }
        }
    }
    let count = objects.len();
    let message = found_message(count, "object", selection.words(), &looked.on());
    (message, Answer::Find { count, objects })
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
    looked: &LookedAt,
) -> (String, Answer) {
    let mut inserts = Vec::new();
    for record in sheets {
        for (index, object) in record.sheet.objects.iter().enumerate() {
            let Object::Insert(insert) = object else {
                continue;
            };
            if selection.matches(record, index) {
                inserts.push(InsertAttributes {
                    sheet: record.name.clone(),
                    id: record.id(index),
                    name: insert.name.clone(),
                    block: insert.block.clone(),
                    attributes: AttributeValues(insert.attributes.clone()),
                    written_at: written_at(record, index),
                });
            }
        }
    }
    let count = inserts.len();
    let message = found_message(count, "insert", selection.words(), &looked.on());
    (message, Answer::Attributes { count, inserts })
}

fn measure(
    sheets: &[&SheetRecord],
    measure: &Measure,
    units: Unit,
    looked: &LookedAt,
) -> Result<(String, Answer), Error> {
    let from = end(sheets, &measure.from, "query.from", looked)?;
    let to = end(sheets, &measure.to, "query.to", looked)?;
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
    let delta = [
        to.at.x - from.at.x,
        to.at.y - from.at.y,
        to.at.z - from.at.z,
    ];
    let distance = delta.iter().map(|d| d * d).sum::<f64>().sqrt();
    let message = format!(
        "{distance} {units} from the {} of `{}` to the {} of `{}`, on sheet `{}`",
        from.point, from.id, to.point, to.id, from.sheet
    );
    Ok((
        message,
        Answer::Measure {
            distance,
            delta,
            from: Box::new(from),
            to: Box::new(to),
        },
    ))
}

/// The point one end of a distance names. `field` is the end's place in
/// the request, for an error.
fn end(
    sheets: &[&SheetRecord],
    end: &End,
    field: &str,
    looked: &LookedAt,
) -> Result<MeasuredPoint, Error> {
    let (record, index) = match (&end.id, &end.name) {
        (Some(id), None) => {
            let found = sheets.iter().find_map(|record| {
                (0..record.objects.len())
                    .find(|&i| Matching::Exact.identifier(&record.id(i), id))
                    .map(|i| (*record, i))
            });
            found.ok_or_else(|| {
                Error::new(
                    ErrorCode::NoSuchObject,
                    format!("no object {} has the ID `{id}`", looked.on()),
                )
                .at_field(format!("{field}.id"))
            })?
        }
        (None, Some(name)) => {
            let found: Vec<(&SheetRecord, usize)> = sheets
                .iter()
                .filter_map(|record| {
                    record
                        .sheet
                        .objects
                        .iter()
                        .position(|object| match object {
                            Object::Insert(insert) => insert
                                .name
                                .as_deref()
                                .is_some_and(|have| Matching::Exact.identifier(have, name)),
                            _ => false,
                        })
                        .map(|i| (*record, i))
                })
                .collect();
            match found.as_slice() {
                [] => {
                    return Err(Error::new(
                        ErrorCode::NoSuchObject,
                        format!("no object {} has the name `{name}`", looked.on()),
                    )
                    .at_field(format!("{field}.name")))
                }
                [one] => *one,
                several => {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        format!(
                            "the name `{name}` is given on {} sheets, {}, and a name is given \
                             once on a sheet; name the sheet to measure on, or give the \
                             object's ID",
                            several.len(),
                            several
                                .iter()
                                .map(|(r, _)| r.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    )
                    .at_field(format!("{field}.name")))
                }
            }
        }
        _ => {
            return Err(Error::usage(
                "each end of a distance names its object by `id` or by `name`, and not both",
            )
            .at_field(field.to_owned()))
        }
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
    Ok(MeasuredPoint {
        sheet: record.name.clone(),
        id,
        name: match object {
            Object::Insert(insert) => insert.name.clone(),
            _ => None,
        },
        kind: object.type_name().to_owned(),
        point,
        at,
    })
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
fn article(kind: &str) -> String {
    if kind.starts_with(['a', 'e', 'i', 'o', 'u']) || kind == "mtext" {
        format!("an {kind}")
    } else {
        format!("a {kind}")
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
        Object::Text(text) => one("position", text.position),
        Object::MText(text) => one("position", text.position),
        Object::AttributeDefinition(definition) => one("position", definition.position),
        Object::Insert(insert) => one("position", insert.position),
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
    units: Unit,
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
    let message = match listed.as_slice() {
        [one] => match &one.extents {
            Some(e) => format!(
                "sheet `{}` draws within {} by {} {units}, from [{}, {}] to [{}, {}]",
                one.sheet, e.width, e.height, e.min[0], e.min[1], e.max[0], e.max[1]
            ),
            None => format!("nothing matches: sheet `{}` draws nothing", one.sheet),
        },
        _ if count == 0 => format!("nothing matches: no sheet draws anything {}", looked.on()),
        all => format!(
            "the extents of {} of the {} sheets looked at, in {units}",
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
fn sheet_extents(record: &SheetRecord, fonts: &SheetFonts) -> Option<Extents> {
    let sheet = &record.sheet;
    // A coarse drawing, curves to within a unit, for the sheet's size; then
    // one fine enough to measure it by.
    let coarse = draw::draw_sheet(sheet, fonts, 1.0, MOST_HATCH_DASHES).extents()?;
    let size = coarse.width().max(coarse.height());
    let tolerance = if size > 0.0 {
        size * EXTENTS_TOLERANCE
    } else {
        EXTENTS_TOLERANCE
    };
    let fine = draw::draw_sheet(sheet, fonts, tolerance, MOST_HATCH_DASHES).extents()?;
    Some(Extents {
        min: fine.min,
        max: fine.max,
        width: fine.width(),
        height: fine.height(),
    })
}
