//! Imports the blocks of a DWG or a DXF into a block library, as AutoCAD's
//! DesignCenter (ADCENTER) drags a drawing's blocks into another.
//!
//! The drawing is only read, through the seam ([`crate::dwg::read_blocks`]),
//! into the core's own blocks: what the core has no model for, such as a
//! dimension or an object in a true colour, is not carried, and the reply
//! counts it for each block. The library is written through the record
//! editor ([`RecordFile`]), so it keeps every byte it had: each new block is
//! added after its last, and each layer, linetype, hatch pattern and text
//! style the blocks need and the library lacks is added to its table. What
//! the library defines already it keeps as it defines it.
//!
//! `agentdrafter import-blocks` writes a library of its own
//! ([`import_blocks`]); a drawing set's library is changed only by a change
//! set's `import_blocks` operation (`change/apply.rs`), so its journal and
//! undo hold the change. Both plan the import here, with [`plan`], and write
//! it with [`write_plan`].

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::build::{is_same_file, write_beside};
use crate::change::{fit_library_inserts, AttributeRemoved};
use crate::description::blocks::{library_hatch_patterns, read_block_library, tag_in_capitals};
use crate::description::{
    library_named, one_name, read_file, read_rows, read_set_settings, template_path, RecordFile,
    LAYER_KEYS, LINETYPE_KEYS, TEXT_STYLE_KEYS,
};
use crate::error::{Error, ErrorCode};
use crate::model::{
    Block, BlockLibrary, Donut, HatchEdge, HatchLoop, HatchPattern, HatchPatternLine, Layer,
    Linetype, Object, ObjectColor, ObjectLinetype, ObjectLineweight, Point3, Properties,
    CONTINUOUS, LAYER_ZERO, WHITE,
};
use crate::set::SET_SETTINGS_FILE;
use crate::text::{Attachment, Justify, TextStyle, STANDARD};
use crate::units::Unit;

/// The keys a hatch pattern takes, in the order a library writes them.
const HATCH_PATTERN_KEYS: [&str; 1] = ["lines"];

/// The font of every drawing's own `Standard` text style, as AutoCAD sets it.
const STANDARD_FONT: &str = "txt.shx";

/// How far two numbers may differ, as a share of the larger, or of 1 if
/// that is more, and still be the same: a DWG keeps angles in radians, so a
/// block read back is a little off in degrees.
const SAME: f64 = 1e-9;

/// What `agentdrafter import-blocks` is asked to do.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportBlocksRequest {
    /// The DWG or DXF to read the blocks of, by its extension.
    pub drawing: PathBuf,
    /// The block library to write: made when it is not there.
    pub library: PathBuf,
    /// Only these blocks, with every block each nests. Without them, every
    /// named block of the drawing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<String>,
    /// Replace a block the library holds already, rather than refuse.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub replace: bool,
    /// The drawing's unit, for a drawing that states none the library
    /// takes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub units: Option<Unit>,
    /// Do everything but write.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dry_run: bool,
}

/// What `agentdrafter import-blocks` did, or would do in a dry run.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ImportBlocksReport {
    /// The drawing read, as the request names it.
    pub drawing: String,
    /// The library written, as the request names it.
    pub library: String,
    /// Whether the library was made, or a dry run would make it.
    pub created: bool,
    /// The library's unit, and the drawing's.
    pub units: Unit,
    /// Whether this was a dry run, which wrote nothing.
    pub dry_run: bool,
    #[serde(flatten)]
    pub imported: BlocksImported,
    /// Each attribute value removed from an insert in another block of the
    /// library, because a block it replaced no longer carries the tag.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attributes_removed: Vec<AttributeRemoved>,
}

/// What an import wrote into a block library, for the command's reply and
/// the `import_blocks` operation's.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct BlocksImported {
    /// The blocks added, in the drawing's order.
    pub blocks_added: Vec<ImportedBlock>,
    /// The blocks whose whole definition was replaced.
    pub blocks_replaced: Vec<ImportedBlock>,
    /// The entries added to the library's tables.
    pub tables_added: TablesAdded,
    /// What the core could not carry, block by block.
    pub not_carried: Vec<NotCarried>,
    /// The blocks asked for that were not imported, and why.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<SkippedBlock>,
    pub notices: Vec<ImportNotice>,
}

/// A block an import wrote.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ImportedBlock {
    pub name: String,
    /// How many objects it holds.
    pub objects: usize,
    /// Every block it nests, however deep, in the drawing's order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub nests: Vec<String>,
}

/// The names an import added to each of the library's tables.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct TablesAdded {
    pub linetypes: Vec<String>,
    pub layers: Vec<String>,
    pub hatch_patterns: Vec<String>,
    pub text_styles: Vec<String>,
}

impl TablesAdded {
    fn names_mut(&mut self, table: &str) -> &mut Vec<String> {
        match table {
            "linetypes" => &mut self.linetypes,
            "layers" => &mut self.layers,
            "hatch_patterns" => &mut self.hatch_patterns,
            _ => &mut self.text_styles,
        }
    }
}

/// Objects of one type in one block that the core could not carry, such
/// as a dimension, an object in a true colour or an annotative attribute.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NotCarried {
    pub block: String,
    /// The object's type as a DXF names it, such as `DIMENSION`.
    pub object_type: String,
    pub count: usize,
}

/// A block asked for that was not imported.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SkippedBlock {
    pub block: String,
    pub why: String,
}

/// Something an import noticed that did not stop it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ImportNotice {
    pub code: ImportNoticeCode,
    pub message: String,
}

/// What kind of notice it is. Written in `snake_case` in JSON.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportNoticeCode {
    /// The library defines a layer, linetype, hatch pattern or text style
    /// the blocks use, differently from the drawing: the library's is kept.
    DefinitionKept,
    /// The blocks' text draws in `Standard`, which the drawing defines
    /// otherwise than AutoCAD's own; the library leaves `Standard` to each
    /// sheet.
    StandardStyle,
    /// A block replaced had cells, which a DWG does not hold, so the
    /// replacement has none: they are added by hand.
    CellsDropped,
    /// A hatch pattern of the drawing has the name of another, in the
    /// drawing or the library, defined otherwise: it is carried under a new
    /// name, its own with `_2`, `_3` and so on, and its hatches name that.
    PatternRenamed,
    /// The command replaced a block of a library that no drawing set it
    /// found names; a set elsewhere that names it would not see the change
    /// in its journal.
    NotJournaled,
}

/// A drawing's blocks as the seam reads them, with the tables they draw on.
pub(crate) struct DrawingBlocks {
    /// The drawing's `INSUNITS` code, and the unit it names, if the core
    /// knows it.
    pub units_code: i16,
    pub units: Option<Unit>,
    /// The linetypes, layers and text styles the core can read, each a
    /// block library can hold.
    pub linetypes: Vec<Linetype>,
    pub layers: Vec<Layer>,
    pub text_styles: Vec<TextStyle>,
    /// Every named block, in the drawing's order, holding the objects the
    /// core carries, which may be none.
    pub blocks: Vec<ReadBlock>,
    /// The blocks never offered: model and paper space, layouts, anonymous
    /// blocks and external references, each with why.
    pub skipped: Vec<(String, &'static str)>,
}

/// One named block of a drawing, as the seam reads it.
#[derive(Clone)]
pub(crate) struct ReadBlock {
    pub block: Block,
    /// Each type of object the core could not carry, as a DXF names it,
    /// and how many, in the order first met.
    pub not_carried: Vec<(&'static str, usize)>,
}

/// Whether an object read from a drawing can go into a block library
/// whose tables are those of the drawing: its layer, linetype and text
/// style are the drawing's own or every drawing's, and its text is text a
/// library takes.
pub(crate) fn carries(
    object: &Object,
    linetypes: &[Linetype],
    layers: &[Layer],
    styles: &[TextStyle],
) -> bool {
    let layer = object.layer();
    let layer_known = layer == LAYER_ZERO || layers.iter().any(|l| l.name == layer);
    let linetype_known = match &object.properties().linetype {
        ObjectLinetype::Named(name) => {
            name == CONTINUOUS || linetypes.iter().any(|l| l.name == *name)
        }
        ObjectLinetype::ByLayer | ObjectLinetype::ByBlock => true,
    };
    let style_known = |name: &str| name == STANDARD || styles.iter().any(|s| s.name == name);
    let one_line = |text: &str| !text.chars().any(char::is_control);
    let shows = |text: &str| !text.trim().is_empty();
    layer_known
        && linetype_known
        && match object {
            Object::Text(t) => style_known(&t.style) && shows(&t.contents) && one_line(&t.contents),
            Object::MText(t) => {
                style_known(&t.style)
                    && shows(&t.contents)
                    && !t.contents.chars().any(|c| c.is_control() && c != '\n')
            }
            Object::AttributeDefinition(d) => {
                style_known(&d.style)
                    && one_line(&d.default)
                    && one_line(&d.prompt)
                    && !d.tag.is_empty()
                    && !d.tag.chars().any(|c| c.is_whitespace() || c.is_control())
            }
            Object::Insert(insert) => insert.attributes.iter().all(|a| one_line(&a.value)),
            Object::Polyline(p) => p.points.len() >= if p.closed { 3 } else { 2 },
            _ => true,
        }
}

/// `object` as a donut when it is a polyline of the exact shape a donut is
/// written as ([`Donut::polyline`]): closed, through two points on one
/// level line, the first right of the second, each starting a half circle
/// anticlockwise (a bulge of 1), with a constant width that fills a ring,
/// or a disc when the ring has no hole. Anything else is left as it is.
pub(crate) fn as_donut(object: Object) -> Object {
    let Object::Polyline(polyline) = &object else {
        return object;
    };
    let shaped = polyline.closed
        && polyline.points.len() == 2
        && polyline.bulges == [1.0, 1.0]
        && polyline.widths.is_empty()
        && polyline.width > 0.0;
    if !shaped {
        return object;
    }
    let (right, left) = (polyline.points[0], polyline.points[1]);
    if right.y != left.y || right.z != left.z || right.x <= left.x {
        return object;
    }
    let middle = right.x - left.x;
    let outside_diameter = middle + polyline.width;
    let mut inside_diameter = middle - polyline.width;
    if inside_diameter.abs() <= SAME * outside_diameter {
        inside_diameter = 0.0;
    }
    if inside_diameter < 0.0 {
        return object;
    }
    Object::Donut(Donut {
        center: Point3::new((right.x + left.x) / 2.0, right.y, right.z),
        inside_diameter,
        outside_diameter,
        layer: polyline.layer.clone(),
        properties: polyline.properties.clone(),
    })
}

/// `block` with each attribute tag its attribute definitions and nested
/// inserts write taken in capitals, as AutoCAD keeps a tag and a library
/// reads one.
fn tags_in_capitals(mut block: Block) -> Block {
    for object in &mut block.objects {
        match object {
            Object::AttributeDefinition(definition) => {
                definition.tag = tag_in_capitals(&definition.tag);
            }
            Object::Insert(insert) => {
                for attribute in &mut insert.attributes {
                    attribute.tag = tag_in_capitals(&attribute.tag);
                }
            }
            _ => {}
        }
    }
    block
}

/// The library an import writes into, as it is before.
pub(crate) struct Existing {
    pub library: BlockLibrary,
    pub hatch_patterns: Vec<HatchPattern>,
}

impl Existing {
    /// The library whose text is `text`.
    pub(crate) fn read(text: &str) -> Result<Self, Error> {
        let (library, _) = read_block_library(text)?;
        let hatch_patterns = library_hatch_patterns(text)?;
        Ok(Self {
            library,
            hatch_patterns,
        })
    }

    /// A library that is not there yet, in `units`.
    fn none(units: Unit) -> Self {
        Self {
            library: BlockLibrary {
                units,
                linetypes: Vec::new(),
                layers: Vec::new(),
                text_styles: Vec::new(),
                blocks: Vec::new(),
            },
            hatch_patterns: Vec::new(),
        }
    }
}

/// What an import will write: each block, whole, and each table entry.
pub(crate) struct Plan {
    pub blocks: Vec<PlannedBlock>,
    pub entries: Vec<PlannedEntry>,
    pub imported: BlocksImported,
}

impl Plan {
    /// The blocks it replaces.
    pub(crate) fn replaced(&self) -> Vec<String> {
        self.blocks
            .iter()
            .filter(|b| b.replaces)
            .map(|b| b.name.clone())
            .collect()
    }
}

/// A block an import writes: its base point, unless it is `[0, 0, 0]`,
/// and its objects, each as a block library writes it.
pub(crate) struct PlannedBlock {
    pub name: String,
    pub base_point: Option<Value>,
    pub objects: Vec<Value>,
    pub replaces: bool,
}

/// An entry an import adds to one of the library's tables.
pub(crate) struct PlannedEntry {
    pub table: &'static str,
    pub order: &'static [&'static str],
    pub name: String,
    pub keys: Map<String, Value>,
}

/// What becomes of one block of the drawing an import takes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fate {
    Add,
    Replace,
    /// The library holds it already, the same: an insert of it names the
    /// library's.
    Reuse,
}

/// Plans importing `wanted` of `drawing`'s blocks, or every named block
/// when it names none, each with every block it nests, into `existing`.
/// A block the library holds already is refused, unless `replace`; a
/// nested block it holds the same is used as it is. `replace_key` names
/// how the caller asks to replace, such as `--replace`, for the error.
pub(crate) fn plan(
    drawing: &DrawingBlocks,
    existing: &Existing,
    wanted: &[String],
    replace: bool,
    replace_key: &str,
) -> Result<Plan, Error> {
    let library = &existing.library;
    let mut imported = BlocksImported::default();
    let owned: Vec<ReadBlock> = drawing
        .blocks
        .iter()
        .map(|read| ReadBlock {
            block: tags_in_capitals(read.block.clone()),
            not_carried: read.not_carried.clone(),
        })
        .collect();
    let named: Vec<&ReadBlock> = owned.iter().collect();
    let carried = |b: &ReadBlock| !b.block.objects.is_empty();
    // A block skipped whole still says what it could not carry.
    let nothing_carried = |b: &ReadBlock, imported: &mut BlocksImported| {
        imported.skipped.push(SkippedBlock {
            block: b.block.name.clone(),
            why: "none of its objects can be carried".to_owned(),
        });
        imported
            .not_carried
            .extend(b.not_carried.iter().map(|(kind, count)| NotCarried {
                block: b.block.name.clone(),
                object_type: (*kind).to_owned(),
                count: *count,
            }));
    };

    // The blocks asked for.
    let mut top: Vec<usize> = Vec::new();
    if wanted.is_empty() {
        for (i, b) in named.iter().enumerate() {
            if carried(b) {
                top.push(i);
            } else {
                nothing_carried(b, &mut imported);
            }
        }
    } else {
        let mut unknown = Vec::new();
        for name in wanted {
            if let Some(i) = named.iter().position(|b| one_name(&b.block.name, name)) {
                if carried(named[i]) {
                    if !top.contains(&i) {
                        top.push(i);
                    }
                } else if !imported
                    .skipped
                    .iter()
                    .any(|s| s.block == named[i].block.name)
                {
                    nothing_carried(named[i], &mut imported);
                }
            } else if let Some((found, why)) =
                drawing.skipped.iter().find(|(n, _)| one_name(n, name))
            {
                imported.skipped.push(SkippedBlock {
                    block: found.clone(),
                    why: (*why).to_owned(),
                });
            } else {
                unknown.push(name.as_str());
            }
        }
        if !unknown.is_empty() {
            let blocks: Vec<&str> = named.iter().map(|b| b.block.name.as_str()).collect();
            return Err(Error::new(
                ErrorCode::UnknownName,
                format!(
                    "the drawing has no block {}; its blocks: {}",
                    quoted(&unknown),
                    if blocks.is_empty() {
                        "none".to_owned()
                    } else {
                        blocks.join(", ")
                    }
                ),
            )
            .at_field("blocks"));
        }
    }

    // Every block they nest, however deep.
    let index_of = |name: &str| named.iter().position(|b| b.block.name == name);
    let nests_of = |i: usize| -> Vec<usize> {
        let mut found: Vec<usize> = Vec::new();
        let mut queue = vec![i];
        while let Some(at) = queue.pop() {
            for object in &named[at].block.objects {
                if let Object::Insert(insert) = object {
                    if let Some(j) = index_of(&insert.block) {
                        if j != i && !found.contains(&j) {
                            found.push(j);
                            queue.push(j);
                        }
                    }
                }
            }
        }
        found.sort_unstable();
        found
    };
    let mut taken: Vec<usize> = top.clone();
    for &i in &top {
        for j in nests_of(i) {
            if !taken.contains(&j) {
                taken.push(j);
            }
        }
    }
    taken.sort_unstable();

    // What becomes of each.
    let mut fates: BTreeMap<usize, Fate> = BTreeMap::new();
    let mut names = Names::default();
    let mut refused: Vec<String> = Vec::new();
    for &i in &taken {
        let block = &named[i].block;
        let asked = top.contains(&i);
        let fate = match library
            .blocks
            .iter()
            .find(|b| one_name(&b.name, &block.name))
        {
            None => Fate::Add,
            Some(held) if !asked && same_block(held, block) => {
                names.blocks.insert(block.name.clone(), held.name.clone());
                Fate::Reuse
            }
            Some(held) if held.name != block.name => {
                refused.push(format!(
                    "`{}`, which the library holds as `{}`: AutoCAD ignores case in names, so \
                     only a block of the same name is replaced",
                    block.name, held.name
                ));
                continue;
            }
            Some(_) if replace => Fate::Replace,
            Some(_) if asked => {
                refused.push(format!("`{}`", block.name));
                continue;
            }
            Some(_) => {
                let outer: Vec<&str> = taken
                    .iter()
                    .filter(|&&o| nests_of(o).contains(&i))
                    .map(|&o| named[o].block.name.as_str())
                    .collect();
                refused.push(format!(
                    "`{}`, which {} nests and the library defines otherwise",
                    block.name,
                    quoted(&outer)
                ));
                continue;
            }
        };
        fates.insert(i, fate);
    }
    if !refused.is_empty() {
        return Err(Error::new(
            ErrorCode::DuplicateName,
            format!(
                "the block library holds {} already; nothing was imported. Give {replace_key} to \
                 replace each block's whole definition with the drawing's, or leave the block out",
                refused.join(", ")
            ),
        ));
    }
    let written: Vec<usize> = fates
        .iter()
        .filter(|(_, f)| **f != Fate::Reuse)
        .map(|(i, _)| *i)
        .collect();

    // The tables the blocks written draw on.
    let mut used = Used::default();
    for &i in &written {
        for object in &named[i].block.objects {
            used.note(object);
        }
    }
    for name in used.layers.clone() {
        if let Some(layer) = drawing.layers.iter().find(|l| l.name == name) {
            if !layer.linetype.eq_ignore_ascii_case(CONTINUOUS) {
                used.linetype(&layer.linetype);
            }
        }
    }
    let mut entries: Vec<PlannedEntry> = Vec::new();
    let mut notices: Vec<ImportNotice> = Vec::new();
    let mut kept = |what: &str, name: &str, held: &str| {
        notices.push(ImportNotice {
            code: ImportNoticeCode::DefinitionKept,
            message: format!(
                "the library defines the {what} `{held}` otherwise than the drawing defines \
                 `{name}`; the library's definition is kept, so the blocks draw with it"
            ),
        });
    };
    for name in &used.linetypes {
        let Some(linetype) = drawing.linetypes.iter().find(|l| l.name == *name) else {
            continue;
        };
        match library.linetypes.iter().find(|l| one_name(&l.name, name)) {
            Some(held) => {
                names.linetypes.insert(name.clone(), held.name.clone());
                let mut ours = linetype.clone();
                ours.name.clone_from(&held.name);
                if !same(&ours, held) {
                    kept("linetype", name, &held.name);
                }
            }
            None => entries.push(PlannedEntry {
                table: "linetypes",
                order: &LINETYPE_KEYS,
                name: name.clone(),
                keys: whole_keys(linetype_keys(linetype)),
            }),
        }
    }
    for name in &used.layers {
        let Some(layer) = drawing.layers.iter().find(|l| l.name == *name) else {
            continue;
        };
        let mut ours = layer.clone();
        ours.linetype = names.linetype(&layer.linetype);
        match library.layers.iter().find(|l| one_name(&l.name, name)) {
            Some(held) => {
                names.layers.insert(name.clone(), held.name.clone());
                ours.name.clone_from(&held.name);
                if !same(&ours, held) {
                    kept("layer", name, &held.name);
                }
            }
            None => entries.push(PlannedEntry {
                table: "layers",
                order: &LAYER_KEYS,
                name: name.clone(),
                keys: whole_keys(layer_keys(&ours)),
            }),
        }
    }
    // A hatch pattern is shared by name only where its definition is the
    // same: one of the drawing's that has the name of another, in the
    // drawing or the library, defined otherwise is carried under a name of
    // its own.
    let mut known: Vec<(String, Vec<HatchPatternLine>)> = existing
        .hatch_patterns
        .iter()
        .map(|p| (p.name.clone(), p.lines.clone()))
        .collect();
    let mut renamed: Vec<(String, String, String)> = Vec::new();
    for &i in &written {
        for object in &named[i].block.objects {
            let Object::Hatch(hatch) = object else {
                continue;
            };
            if hatch.is_solid() {
                continue;
            }
            let pattern = &hatch.pattern;
            let resolved = names
                .hatch_patterns
                .iter()
                .find(|(p, _)| same_pattern(p, pattern))
                .map(|(_, name)| name.clone());
            let name = match resolved {
                Some(name) => name,
                None => {
                    let mut candidate = pattern.name.clone();
                    let mut n = 1;
                    let name = loop {
                        match known.iter().find(|(k, _)| one_name(k, &candidate)) {
                            None => {
                                known.push((candidate.clone(), pattern.lines.clone()));
                                entries.push(PlannedEntry {
                                    table: "hatch_patterns",
                                    order: &HATCH_PATTERN_KEYS,
                                    name: candidate.clone(),
                                    keys: whole_keys(hatch_pattern_keys(pattern)),
                                });
                                break candidate;
                            }
                            Some((k, lines)) if same_pattern_lines(lines, &pattern.lines) => {
                                break k.clone()
                            }
                            Some(_) => {
                                n += 1;
                                candidate = format!("{}_{n}", pattern.name);
                            }
                        }
                    };
                    names.hatch_patterns.push((pattern.clone(), name.clone()));
                    name
                }
            };
            let block = &named[i].block.name;
            let note = (block.clone(), pattern.name.clone(), name.clone());
            if !one_name(&name, &pattern.name) && !renamed.contains(&note) {
                renamed.push(note);
            }
        }
    }
    let drawing_style = |name: &str| drawing.text_styles.iter().find(|s| s.name == name);
    for name in &used.text_styles {
        let Some(style) = drawing_style(name) else {
            continue;
        };
        match library.text_styles.iter().find(|s| one_name(&s.name, name)) {
            Some(held) => {
                names.text_styles.insert(name.clone(), held.name.clone());
                names
                    .width_factors
                    .insert(held.name.clone(), held.width_factor);
                let mut ours = style.clone();
                ours.name.clone_from(&held.name);
                if !same(&ours, held) {
                    kept("text style", name, &held.name);
                }
            }
            None => {
                names.width_factors.insert(name.clone(), style.width_factor);
                entries.push(PlannedEntry {
                    table: "text_styles",
                    order: &TEXT_STYLE_KEYS,
                    name: name.clone(),
                    keys: whole_keys(text_style_keys(style)),
                });
            }
        }
    }
    // `Standard` is every drawing's own: a library that does not list it
    // leaves it to each sheet, which AutoCAD sets to txt.shx.
    let standard = library
        .text_styles
        .iter()
        .find(|s| s.name == STANDARD)
        .cloned()
        .unwrap_or_else(|| TextStyle {
            name: STANDARD.to_owned(),
            font: STANDARD_FONT.to_owned(),
            width_factor: 1.0,
        });
    names
        .width_factors
        .insert(STANDARD.to_owned(), standard.width_factor);
    for (block, old, new) in renamed {
        notices.push(ImportNotice {
            code: ImportNoticeCode::PatternRenamed,
            message: format!(
                "the block `{block}` hatches with the pattern `{old}`, whose name another \
                 pattern defined otherwise has in the drawing or the library; it is carried as \
                 `{new}`, and the block's hatches name that"
            ),
        });
    }
    if used.standard {
        if let Some(theirs) = drawing_style(STANDARD) {
            let differs = !theirs.font.eq_ignore_ascii_case(&standard.font)
                || !close(theirs.width_factor, standard.width_factor);
            if differs {
                notices.push(ImportNotice {
                    code: ImportNoticeCode::StandardStyle,
                    message: format!(
                        "the blocks' text in the text style Standard draws with {} at a width \
                         factor of {} in the drawing; Standard is every drawing's own, so the \
                         library {}, and the text draws with {} at {} on a sheet that leaves \
                         Standard as it is",
                        theirs.font,
                        theirs.width_factor,
                        if library.text_styles.iter().any(|s| s.name == STANDARD) {
                            "keeps its own"
                        } else {
                            "leaves it to each sheet"
                        },
                        standard.font,
                        standard.width_factor
                    ),
                });
            }
        }
    }

    // Each block, as the library writes it.
    let mut blocks = Vec::new();
    for &i in &written {
        let read = named[i];
        let block = &read.block;
        let replaces = fates[&i] == Fate::Replace;
        let objects: Vec<Value> = block
            .objects
            .iter()
            .map(|o| object_value(o, &names))
            .collect();
        let base_point =
            (block.base_point != Point3::new(0.0, 0.0, 0.0)).then(|| point_value(block.base_point));
        let entry = ImportedBlock {
            name: block.name.clone(),
            objects: objects.len(),
            nests: nests_of(i)
                .into_iter()
                .map(|j| named[j].block.name.clone())
                .collect(),
        };
        if replaces {
            if let Some(held) = library.blocks.iter().find(|b| b.name == block.name) {
                if !held.cells.is_empty() {
                    notices.push(ImportNotice {
                        code: ImportNoticeCode::CellsDropped,
                        message: format!(
                            "the block `{}` the library held declared {} cell{}, which a \
                             drawing does not hold, so its replacement declares none; add them \
                             by hand",
                            block.name,
                            held.cells.len(),
                            if held.cells.len() == 1 { "" } else { "s" }
                        ),
                    });
                }
            }
            imported.blocks_replaced.push(entry);
        } else {
            imported.blocks_added.push(entry);
        }
        for (object_type, count) in &read.not_carried {
            imported.not_carried.push(NotCarried {
                block: block.name.clone(),
                object_type: (*object_type).to_owned(),
                count: *count,
            });
        }
        blocks.push(PlannedBlock {
            name: block.name.clone(),
            base_point,
            objects,
            replaces,
        });
    }
    for entry in &entries {
        imported
            .tables_added
            .names_mut(entry.table)
            .push(entry.name.clone());
    }
    imported.notices = notices;
    Ok(Plan {
        blocks,
        entries,
        imported,
    })
}

/// Writes `plan` into the library `record`: each table entry, then each
/// block, whole.
pub(crate) fn write_plan(record: &mut RecordFile, plan: &Plan) -> Result<(), Error> {
    for entry in &plan.entries {
        record.table_add(entry.table, entry.order, &entry.name, &entry.keys)?;
    }
    for block in &plan.blocks {
        record.block_set(&block.name, block.base_point.as_ref(), &block.objects)?;
    }
    Ok(())
}

/// Whether `path` is a DXF, rather than a DWG, by its extension, in any
/// case; any other is a usage error.
pub(crate) fn is_dxf(path: &Path) -> Result<bool, Error> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("dwg") => Ok(false),
        Some("dxf") => Ok(true),
        _ => Err(Error::usage(format!(
            "blocks are imported from a DWG or a DXF, told apart by the extension, .dwg or .dxf; \
             `{}` has neither",
            path.display()
        ))),
    }
}

/// The unit the drawing's blocks are imported in: the one the drawing
/// states, or, when it states none a block library takes, the one `given`
/// names. Converting from one unit to another is not offered yet.
pub(crate) fn drawing_unit(
    drawing: &DrawingBlocks,
    given: Option<Unit>,
    units_key: &str,
) -> Result<Unit, Error> {
    match (drawing.units, given) {
        (Some(unit), None) => Ok(unit),
        (Some(unit), Some(given)) if unit == given => Ok(unit),
        (Some(unit), Some(given)) => Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "the drawing is in {unit}, and {units_key} names {given}; a drawing that states \
                 its unit is imported in it, since converting its blocks from one unit to \
                 another is not offered yet"
            ),
        )
        .at_field("units")),
        (None, Some(given)) => Ok(given),
        (None, None) => {
            let states = if drawing.units_code == 0 {
                "states no unit (its INSUNITS is 0)".to_owned()
            } else {
                format!(
                    "states a unit a block library does not take (its INSUNITS is {})",
                    drawing.units_code
                )
            };
            Err(Error::new(
                ErrorCode::MissingUnit,
                format!(
                    "the drawing {states}; name the unit its blocks are drawn in with \
                     {units_key}, one of {}",
                    Unit::symbols()
                ),
            )
            .at_field("units"))
        }
    }
}

/// Imports a drawing's blocks into a block library of its own, as
/// `agentdrafter import-blocks` does.
///
/// The library is made in the drawing's unit when it is not there; one
/// that is must be in the drawing's unit. A library a drawing set's
/// templates name is refused: it is changed only by a change set's
/// `import_blocks` operation, so the set's journal and undo hold it.
pub fn import_blocks(request: &ImportBlocksRequest) -> Result<ImportBlocksReport, Error> {
    let here = std::env::current_dir().unwrap_or_default();
    import_blocks_in(request, &here)
}

/// Imports as [`import_blocks`] does, looking for the drawing sets that may
/// name the library above it and above `here`, the folder the command runs
/// in.
fn import_blocks_in(
    request: &ImportBlocksRequest,
    here: &Path,
) -> Result<ImportBlocksReport, Error> {
    let dxf = is_dxf(&request.drawing)?;
    if let Some(set) = set_naming(&request.library, here) {
        return Err(Error::usage(format!(
            "the block library is the drawing set's in `{}`, whose templates name it; a set's \
             library is changed only by a change set, so that its journal and undo hold the \
             change: send `agentdrafter change` an import_blocks operation instead, such as \
             {{\"operations\": [{{\"op\": \"import_blocks\", \"from\": \"{}\"}}]}}",
            set.display(),
            request.drawing.display()
        ))
        .in_file(&request.library));
    }
    let drawing = crate::dwg::read_blocks(&request.drawing, dxf)?;
    let units = drawing_unit(&drawing, request.units, "--units")?;
    let old_text = match fs::read_to_string(&request.library) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(_) => Some(read_file(&request.library, "the block library")?),
    };
    let existing = match &old_text {
        Some(text) => {
            let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
            let existing = Existing::read(text).map_err(|e| e.or_in_file(&request.library))?;
            if existing.library.units != units {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    format!(
                        "the block library is in {}, and the drawing in {units}; blocks are \
                         imported into a library in the drawing's unit, since converting them \
                         from one unit to another is not offered yet",
                        existing.library.units
                    ),
                )
                .in_file(&request.library));
            }
            existing
        }
        None => Existing::none(units),
    };
    let mut plan = plan(
        &drawing,
        &existing,
        &request.blocks,
        request.replace,
        "--replace",
    )?;
    // No set the command can see names the library, but one elsewhere on
    // the disk may, and would not see a block replaced in its journal.
    if old_text.is_some() && !plan.replaced().is_empty() {
        plan.imported.notices.push(ImportNotice {
            code: ImportNoticeCode::NotJournaled,
            message: "no drawing set above the block library or above the folder the command \
                      runs in names it, so its blocks were replaced here; a drawing set \
                      elsewhere that names this library would not see the change in its \
                      journal, so a set's library is changed with a change set's import_blocks \
                      operation"
                .to_owned(),
        });
    }
    let created = old_text.is_none() && !plan.blocks.is_empty();
    let mut report = ImportBlocksReport {
        drawing: request.drawing.display().to_string(),
        library: request.library.display().to_string(),
        created,
        units,
        dry_run: request.dry_run,
        imported: BlocksImported::default(),
        attributes_removed: Vec::new(),
    };
    if plan.blocks.is_empty() {
        report.imported = plan.imported;
        return Ok(report);
    }
    let text = match old_text {
        Some(text) => text,
        None => new_library_text(units, &plan),
    };
    let mut record = RecordFile::from_text(&request.library, text)?;
    write_plan(&mut record, &plan)?;
    report.attributes_removed = fit_library_inserts(&mut record, &plan.replaced())?;
    let Some(new_text) = record.new_text()? else {
        report.imported = plan.imported;
        return Ok(report);
    };
    let bare = new_text.strip_prefix('\u{FEFF}').unwrap_or(&new_text);
    read_block_library(bare).map_err(|e| {
        Error::new(
            e.code,
            format!(
                "the block library it would write does not read back, so nothing was written: {}",
                e.message
            ),
        )
        .in_file(&request.library)
    })?;
    if !request.dry_run {
        let temporary = write_beside(&request.library, new_text.as_bytes(), "the block library")?;
        fs::rename(&temporary, &request.library).map_err(|e| {
            let _ = fs::remove_file(&temporary);
            Error::new(
                ErrorCode::WriteFailed,
                format!("could not write the block library: {e}"),
            )
            .in_file(&request.library)
        })?;
    }
    report.imported = plan.imported;
    Ok(report)
}

/// The text a new library starts as: its unit, each table the plan adds to
/// and its blocks, all empty, for the record editor to fill in.
fn new_library_text(units: Unit, plan: &Plan) -> String {
    let mut text = format!("units: {}\n", units.symbol());
    for table in ["linetypes", "layers", "hatch_patterns", "text_styles"] {
        if plan.entries.iter().any(|e| e.table == table) {
            text.push_str(&format!("{table}: {{}}\n"));
        }
    }
    text.push_str("blocks: {}\n");
    text
}

/// The folder of a drawing set whose templates name the block library at
/// `library`, if one does, among the sets the command can find: those in
/// the library's folder or any folder above it, and those in `here`, the
/// folder the command runs in, or any folder above it. A set elsewhere on
/// the disk is not seen. A set whose files cannot be read is not taken to
/// name it. A relative `library` is taken from `here`.
fn set_naming(library: &Path, here: &Path) -> Option<PathBuf> {
    let library = plain_path(&here.join(library));
    let here = plain_path(here);
    let mut folders: Vec<&Path> = library.parent()?.ancestors().collect();
    for folder in here.ancestors() {
        if !folders.contains(&folder) {
            folders.push(folder);
        }
    }
    folders
        .into_iter()
        .filter(|f| f.join(SET_SETTINGS_FILE).is_file())
        .find(|set| names_library(set, &library))
        .map(Path::to_owned)
}

/// Whether a template of the drawing set in `set` names the block library
/// at `library`.
fn names_library(set: &Path, library: &Path) -> bool {
    let Ok(settings) = read_set_settings(&set.join(SET_SETTINGS_FILE)) else {
        return false;
    };
    let Ok(rows) = read_rows(&set.join(&settings.rows)) else {
        return false;
    };
    rows.iter().any(|row| {
        let template = template_path(set, row);
        let Some(named) = fs::read_to_string(&template)
            .ok()
            .and_then(|text| library_named(&text))
        else {
            return false;
        };
        let path = plain_path(&template.parent().unwrap_or(Path::new("")).join(named));
        path == library || is_same_file(&path, library)
    })
}

/// `path` made absolute, with `.` and `..` worked out as written, so two
/// paths to a file not there yet compare as the file system would.
fn plain_path(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_owned());
    let mut plain = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                plain.pop();
            }
            other => plain.push(other),
        }
    }
    plain
}

/// The names a block library spells the drawing's by, where it holds one
/// already in another case, and the width factor of each text style the
/// blocks' text will draw with.
#[derive(Default)]
struct Names {
    layers: BTreeMap<String, String>,
    linetypes: BTreeMap<String, String>,
    /// Each hatch pattern of the drawing, by its name and definition, with
    /// the name the library writes it by.
    hatch_patterns: Vec<(HatchPattern, String)>,
    text_styles: BTreeMap<String, String>,
    blocks: BTreeMap<String, String>,
    width_factors: BTreeMap<String, f64>,
}

impl Names {
    fn of(map: &BTreeMap<String, String>, name: &str) -> String {
        map.get(name).cloned().unwrap_or_else(|| name.to_owned())
    }

    fn linetype(&self, name: &str) -> String {
        if name.eq_ignore_ascii_case(CONTINUOUS) {
            CONTINUOUS.to_owned()
        } else {
            Self::of(&self.linetypes, name)
        }
    }

    fn style(&self, name: &str) -> String {
        Self::of(&self.text_styles, name)
    }

    /// The name the library writes the hatch pattern `pattern` by.
    fn hatch_pattern(&self, pattern: &HatchPattern) -> String {
        self.hatch_patterns
            .iter()
            .find(|(p, _)| same_pattern(p, pattern))
            .map_or_else(|| pattern.name.clone(), |(_, name)| name.clone())
    }

    /// The width factor text in the style `name`, as the library spells
    /// it, takes when it gives none.
    fn width_factor(&self, style: &str) -> f64 {
        self.width_factors.get(style).copied().unwrap_or(1.0)
    }
}

/// The layers, linetypes and text styles the blocks' objects name, each
/// once, in the order first met; and whether any text draws in `Standard`.
#[derive(Default)]
struct Used {
    layers: Vec<String>,
    linetypes: Vec<String>,
    text_styles: Vec<String>,
    standard: bool,
}

impl Used {
    fn note(&mut self, object: &Object) {
        let layer = object.layer();
        if layer != LAYER_ZERO && !self.layers.iter().any(|l| l == layer) {
            self.layers.push(layer.to_owned());
        }
        if let ObjectLinetype::Named(name) = &object.properties().linetype {
            if name != CONTINUOUS {
                self.linetype(name);
            }
        }
        let style = match object {
            Object::Text(t) => Some(&t.style),
            Object::MText(t) => Some(&t.style),
            Object::AttributeDefinition(d) => Some(&d.style),
            _ => None,
        };
        if let Some(style) = style {
            if style == STANDARD {
                self.standard = true;
            } else if !self.text_styles.contains(style) {
                self.text_styles.push(style.clone());
            }
        }
    }

    fn linetype(&mut self, name: &str) {
        if !self.linetypes.iter().any(|l| l == name) {
            self.linetypes.push(name.to_owned());
        }
    }
}

/// Whether two blocks hold the same, but for their cells, which a drawing
/// does not hold, and for what a DWG's radians leave a little off.
fn same_block(a: &Block, b: &Block) -> bool {
    let strip = |block: &Block| Block {
        name: String::new(),
        cells: Vec::new(),
        ..block.clone()
    };
    same(&strip(a), &strip(b))
}

/// Whether two hatch patterns of a drawing are one: the same name, and
/// lines the same as [`same_pattern_lines`] has them. Planning and writing
/// both match a hatch's pattern by it, so a hatch is written with the name
/// its own definition was given.
fn same_pattern(a: &HatchPattern, b: &HatchPattern) -> bool {
    a.name == b.name && same_pattern_lines(&a.lines, &b.lines)
}

/// Whether two hatch patterns' lines are one definition, numbers within
/// [`SAME`]: the one test of a pattern's definition, against the drawing's
/// others and the library's.
fn same_pattern_lines(a: &[HatchPatternLine], b: &[HatchPatternLine]) -> bool {
    same(&a, &b)
}

/// Whether `a` and `b` are the same as plain data, numbers within [`SAME`].
fn same<T: Serialize>(a: &T, b: &T) -> bool {
    match (serde_json::to_value(a), serde_json::to_value(b)) {
        (Ok(a), Ok(b)) => same_value(&a, &b),
        _ => false,
    }
}

fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) => close(x, y),
            _ => x == y,
        },
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(x, y)| same_value(x, y))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(key, value)| y.get(key).is_some_and(|other| same_value(value, other)))
        }
        _ => a == b,
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= SAME * a.abs().max(b.abs()).max(1.0)
}

/// Names in backticks, joined, for a message.
fn quoted(names: &[&str]) -> String {
    names
        .iter()
        .map(|n| format!("`{n}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// An angle in degrees as an import writes it: snapped to the nearest
/// billionth of a degree, so that one a DWG kept in radians, such as
/// 29.999999999999996, is written as it was drawn, 30; a whole turn is
/// written 0.
fn degrees(angle: f64) -> f64 {
    const STEP: f64 = 1e9;
    let snapped = (angle * STEP).round() / STEP;
    let snapped = if (angle - snapped).abs() <= 1.0 / STEP {
        snapped
    } else {
        angle
    };
    if snapped == 360.0 {
        0.0
    } else {
        snapped
    }
}

/// The two end angles of an arc or an elliptical arc as an import writes
/// them: each snapped as [`degrees`] snaps it when the two stay apart, as
/// written, a whole turn taken off; otherwise both as read, so that a short
/// sweep is never closed up into nothing or into a whole curve.
fn sweep(start: f64, end: f64) -> (f64, f64) {
    let (snapped_start, snapped_end) = (degrees(start), degrees(end));
    if (snapped_end - snapped_start).rem_euclid(360.0) == 0.0 {
        (start, end)
    } else {
        (snapped_start, snapped_end)
    }
}

fn point_value(p: Point3) -> Value {
    json!([p.x, p.y, p.z])
}

/// An object as a block library writes it: `{type: {keys}}`, leaving out
/// each key at what a library takes when it is left out, with names
/// spelled as the library spells them.
fn object_value(object: &Object, names: &Names) -> Value {
    let mut keys = Map::new();
    let mut put = |key: &str, value: Value| {
        keys.insert(key.to_owned(), value);
    };
    match object {
        Object::Line(line) => {
            put("from", point_value(line.from));
            put("to", point_value(line.to));
        }
        Object::Polyline(polyline) => {
            let flat = polyline.is_flat();
            let last = polyline.points.len().saturating_sub(1);
            let points: Vec<Value> = polyline
                .points
                .iter()
                .enumerate()
                .map(|(i, &p)| {
                    // The last point of an open polyline starts no segment.
                    let starts = flat && (polyline.closed || i < last);
                    let bulge = if starts { polyline.bulge(i) } else { 0.0 };
                    let [start, end] = if starts && !polyline.widths.is_empty() {
                        polyline.widths_at(i)
                    } else {
                        [0.0, 0.0]
                    };
                    if bulge == 0.0 && start == 0.0 && end == 0.0 {
                        return point_value(p);
                    }
                    let mut vertex = Map::new();
                    vertex.insert("point".into(), point_value(p));
                    if bulge != 0.0 {
                        vertex.insert("bulge".into(), json!(bulge));
                    }
                    if start != 0.0 {
                        vertex.insert("start_width".into(), json!(start));
                    }
                    if end != 0.0 {
                        vertex.insert("end_width".into(), json!(end));
                    }
                    Value::Object(vertex)
                })
                .collect();
            put("points", Value::Array(points));
            if polyline.closed {
                put("closed", json!(true));
            }
            if flat && polyline.width != 0.0 {
                put("width", json!(polyline.width));
            }
        }
        Object::Arc(arc) => {
            put("center", point_value(arc.center));
            put("radius", json!(arc.radius));
            let (start, end) = sweep(arc.start_angle, arc.end_angle);
            put("start_angle", json!(start));
            put("end_angle", json!(end));
        }
        Object::Circle(circle) => {
            put("center", point_value(circle.center));
            put("radius", json!(circle.radius));
        }
        Object::Ellipse(ellipse) => {
            put("center", point_value(ellipse.center));
            put("major_axis", point_value(ellipse.major_axis));
            put("ratio", json!(ellipse.ratio));
            if !ellipse.is_whole() {
                let (start, end) = sweep(ellipse.start_angle, ellipse.end_angle);
                put("start_angle", json!(start));
                put("end_angle", json!(end));
            }
        }
        Object::Spline(spline) => {
            if spline.fit_points.is_empty() {
                let points: Vec<Value> = spline
                    .control_points
                    .iter()
                    .map(|&p| point_value(p))
                    .collect();
                put("control_points", Value::Array(points));
                put("degree", json!(spline.degree));
            } else {
                let points: Vec<Value> =
                    spline.fit_points.iter().map(|&p| point_value(p)).collect();
                put("fit_points", Value::Array(points));
            }
        }
        Object::Hatch(hatch) => {
            if hatch.is_solid() {
                put("pattern", json!(hatch.pattern.name));
            } else {
                put("pattern", json!(names.hatch_pattern(&hatch.pattern)));
                if hatch.scale != 1.0 {
                    put("scale", json!(hatch.scale));
                }
                if degrees(hatch.angle) != 0.0 {
                    put("angle", json!(degrees(hatch.angle)));
                }
            }
            let loops: Vec<Value> = hatch.boundary.iter().map(loop_value).collect();
            put("boundary", Value::Array(loops));
        }
        Object::Donut(donut) => {
            put("center", point_value(donut.center));
            if donut.inside_diameter != 0.0 {
                put("inside_diameter", json!(donut.inside_diameter));
            }
            put("outside_diameter", json!(donut.outside_diameter));
        }
        Object::Solid(solid) => {
            let corners: Vec<Value> = solid.corners.iter().map(|&p| point_value(p)).collect();
            put("corners", Value::Array(corners));
        }
        Object::Text(text) => {
            put("contents", json!(text.contents));
            put("position", point_value(text.position));
            put("height", json!(text.height));
            let style = names.style(&text.style);
            if text.width_factor != names.width_factor(&style) {
                put("width_factor", json!(text.width_factor));
            }
            if style != STANDARD {
                put("style", json!(style));
            }
            if degrees(text.rotation) != 0.0 {
                put("rotation", json!(degrees(text.rotation)));
            }
            if text.justify != Justify::Left {
                put("justify", json!(text.justify.name()));
            }
        }
        Object::MText(text) => {
            put("contents", json!(text.contents));
            put("position", point_value(text.position));
            put("height", json!(text.height));
            if text.width != 0.0 {
                put("width", json!(text.width));
            }
            let style = names.style(&text.style);
            if style != STANDARD {
                put("style", json!(style));
            }
            if let Some(factor) = text.width_factor {
                put("width_factor", json!(factor));
            }
            if degrees(text.rotation) != 0.0 {
                put("rotation", json!(degrees(text.rotation)));
            }
            if text.justify != Attachment::TopLeft {
                put("justify", json!(text.justify.name()));
            }
        }
        Object::AttributeDefinition(definition) => {
            put("tag", json!(definition.tag));
            if !definition.prompt.is_empty() {
                put("prompt", json!(definition.prompt));
            }
            if !definition.default.is_empty() {
                put("default", json!(definition.default));
            }
            put("position", point_value(definition.position));
            put("height", json!(definition.height));
            let style = names.style(&definition.style);
            if style != STANDARD {
                put("style", json!(style));
            }
            if definition.width_factor != names.width_factor(&style) {
                put("width_factor", json!(definition.width_factor));
            }
            if degrees(definition.rotation) != 0.0 {
                put("rotation", json!(degrees(definition.rotation)));
            }
            if definition.justify != Justify::Left {
                put("justify", json!(definition.justify.name()));
            }
            // A constant attribute asks nobody for its value, so AutoCAD's
            // ATTDEF gives it no Verify or Preset, and nor does a library.
            let modes = [
                ("invisible", definition.invisible),
                ("constant", definition.constant),
                ("verify", definition.verify && !definition.constant),
                ("preset", definition.preset && !definition.constant),
                ("lock_position", definition.lock_position),
            ];
            for (mode, on) in modes {
                if on {
                    put(mode, json!(true));
                }
            }
        }
        Object::Insert(insert) => {
            put("block", json!(Names::of(&names.blocks, &insert.block)));
            put("position", point_value(insert.position));
            if insert.scale != 1.0 {
                put("scale", json!(insert.scale));
            }
            if degrees(insert.rotation) != 0.0 {
                put("rotation", json!(degrees(insert.rotation)));
            }
            // A tag the block defines more than once takes its values as a
            // list, in the block's order.
            let mut values: Vec<(String, Vec<String>)> = Vec::new();
            for attribute in &insert.attributes {
                match values.iter_mut().find(|(tag, _)| *tag == attribute.tag) {
                    Some((_, list)) => list.push(attribute.value.clone()),
                    None => values.push((attribute.tag.clone(), vec![attribute.value.clone()])),
                }
            }
            if !values.is_empty() {
                let attributes: Map<String, Value> = values
                    .into_iter()
                    .map(|(tag, list)| match list.as_slice() {
                        [one] => (tag, json!(one)),
                        _ => (tag, json!(list)),
                    })
                    .collect();
                put("attributes", Value::Object(attributes));
            }
        }
    }
    let layer = object.layer();
    if layer != LAYER_ZERO {
        put("layer", json!(Names::of(&names.layers, layer)));
    }
    put_properties(&mut keys, object.properties(), names);
    let mut written = Map::new();
    written.insert(object.type_name().to_owned(), Value::Object(keys));
    whole_numbers(Value::Object(written))
}

/// `keys` with each number that is a whole number written as one, as
/// [`whole_numbers`] writes it.
fn whole_keys(keys: Map<String, Value>) -> Map<String, Value> {
    match whole_numbers(Value::Object(keys)) {
        Value::Object(keys) => keys,
        _ => Map::new(),
    }
}

/// `value` with each number that is a whole number, such as `6.0`, written
/// as one, `6`: the same number, as a person writes it.
fn whole_numbers(value: Value) -> Value {
    /// The largest whole number every larger one of which an f64 may not
    /// hold exactly: 2^53.
    const EXACT: f64 = 9_007_199_254_740_992.0;
    match value {
        Value::Number(n) => match n.as_f64() {
            #[allow(clippy::cast_possible_truncation)]
            Some(x) if n.is_f64() && x.fract() == 0.0 && x.abs() < EXACT => json!(x as i64),
            _ => Value::Number(n),
        },
        Value::Array(items) => Value::Array(items.into_iter().map(whole_numbers).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, item)| (key, whole_numbers(item)))
                .collect(),
        ),
        other => other,
    }
}

/// Writes an object's own colour, linetype, lineweight and linetype scale
/// where they are not its layer's.
fn put_properties(keys: &mut Map<String, Value>, properties: &Properties, names: &Names) {
    match properties.color {
        ObjectColor::ByLayer => {}
        ObjectColor::ByBlock => {
            keys.insert("color".into(), json!("ByBlock"));
        }
        ObjectColor::Index(index) => {
            keys.insert("color".into(), json!(index));
        }
    }
    match &properties.linetype {
        ObjectLinetype::ByLayer => {}
        ObjectLinetype::ByBlock => {
            keys.insert("linetype".into(), json!("ByBlock"));
        }
        ObjectLinetype::Named(name) => {
            keys.insert("linetype".into(), json!(names.linetype(name)));
        }
    }
    match properties.lineweight {
        ObjectLineweight::ByLayer => {}
        ObjectLineweight::ByBlock => {
            keys.insert("lineweight".into(), json!("ByBlock"));
        }
        ObjectLineweight::Default => {
            keys.insert("lineweight".into(), json!("Default"));
        }
        ObjectLineweight::Width(width) => {
            keys.insert("lineweight".into(), json!(width.to_string()));
        }
    }
    if properties.linetype_scale != 1.0 {
        keys.insert("linetype_scale".into(), json!(properties.linetype_scale));
    }
}

fn loop_value(hatch_loop: &HatchLoop) -> Value {
    match hatch_loop {
        HatchLoop::Polyline { points } => {
            let points: Vec<Value> = points.iter().map(|&p| point_value(p)).collect();
            json!({ "polyline": { "points": points } })
        }
        HatchLoop::Edges(edges) => {
            let edges: Vec<Value> = edges
                .iter()
                .map(|edge| match *edge {
                    HatchEdge::Line { from, to } => {
                        json!({ "line": { "from": point_value(from), "to": point_value(to) } })
                    }
                    HatchEdge::Arc {
                        center,
                        radius,
                        start_angle,
                        end_angle,
                    } => {
                        let (start_angle, end_angle) = sweep(start_angle, end_angle);
                        json!({ "arc": {
                            "center": point_value(center),
                            "radius": radius,
                            "start_angle": start_angle,
                            "end_angle": end_angle,
                        } })
                    }
                })
                .collect();
            json!({ "edges": edges })
        }
    }
}

fn linetype_keys(linetype: &Linetype) -> Map<String, Value> {
    let mut keys = Map::new();
    if !linetype.description.is_empty() {
        keys.insert("description".into(), json!(linetype.description));
    }
    keys.insert("pattern".into(), json!(linetype.pattern));
    keys
}

fn layer_keys(layer: &Layer) -> Map<String, Value> {
    let mut keys = Map::new();
    if layer.color != WHITE {
        keys.insert("color".into(), json!(layer.color));
    }
    if layer.linetype != CONTINUOUS {
        keys.insert("linetype".into(), json!(layer.linetype));
    }
    if let Some(width) = layer.lineweight {
        keys.insert("lineweight".into(), json!(width.to_string()));
    }
    for (key, value, default) in [
        ("on", layer.on, true),
        ("frozen", layer.frozen, false),
        ("locked", layer.locked, false),
        ("plot", layer.plot, true),
    ] {
        if value != default {
            keys.insert(key.into(), json!(value));
        }
    }
    if !layer.description.is_empty() {
        keys.insert("description".into(), json!(layer.description));
    }
    keys
}

fn text_style_keys(style: &TextStyle) -> Map<String, Value> {
    let mut keys = Map::new();
    keys.insert("font".into(), json!(style.font));
    if style.width_factor != 1.0 {
        keys.insert("width_factor".into(), json!(style.width_factor));
    }
    keys
}

fn hatch_pattern_keys(pattern: &HatchPattern) -> Map<String, Value> {
    let lines: Vec<Value> = pattern
        .lines
        .iter()
        .map(|line| {
            let mut keys = Map::new();
            if degrees(line.angle) != 0.0 {
                keys.insert("angle".into(), json!(degrees(line.angle)));
            }
            if line.origin != [0.0, 0.0] {
                keys.insert("origin".into(), json!(line.origin));
            }
            keys.insert("delta".into(), json!(line.delta));
            if !line.dashes.is_empty() {
                keys.insert("dashes".into(), json!(line.dashes));
            }
            Value::Object(keys)
        })
        .collect();
    let mut keys = Map::new();
    keys.insert("lines".into(), Value::Array(lines));
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{build, BuildRequest};
    use crate::description::parse_block_library;
    use crate::model::{Arc, Insert};

    /// A drawing whose blocks, layers, linetypes and text styles are those
    /// of the made-up block library `text`, each block holding all it holds.
    fn drawing(text: &str) -> DrawingBlocks {
        let library = parse_block_library(text).unwrap();
        DrawingBlocks {
            units_code: 4,
            units: Some(Unit::Mm),
            linetypes: library.linetypes,
            layers: library.layers,
            text_styles: library.text_styles,
            blocks: library
                .blocks
                .into_iter()
                .map(|block| ReadBlock {
                    block,
                    not_carried: Vec::new(),
                })
                .collect(),
            skipped: Vec::new(),
        }
    }

    /// What `plan` makes of `drawing` against the library `held`, and the
    /// library's text it writes.
    fn planned(drawing: &DrawingBlocks, held: &str, wanted: &[&str]) -> (Plan, String) {
        let existing = Existing::read(held).unwrap();
        let wanted: Vec<String> = wanted.iter().map(|w| (*w).to_owned()).collect();
        let plan = plan(drawing, &existing, &wanted, false, "--replace").unwrap();
        let mut record = RecordFile::from_text(Path::new("blocks.yaml"), held.to_owned()).unwrap();
        write_plan(&mut record, &plan).unwrap();
        let text = record.current_text().unwrap();
        (plan, text)
    }

    const EMPTY: &str = "units: mm\nblocks: {}\n";

    /// A made-up block hatched with GRID, defined by `delta`.
    fn hatched(block: &str, delta: &str) -> String {
        format!(
            "units: mm\nhatch_patterns:\n  GRID: {{lines: [{{delta: {delta}}}]}}\nblocks:\n  \
             {block}:\n    objects:\n      - hatch: {{pattern: GRID, boundary: [{{polyline: \
             {{points: [[0, 0, 0], [1, 0, 0], [1, 1, 0]]}}}}]}}\n"
        )
    }

    fn pattern_of<'a>(library: &'a BlockLibrary, block: &str) -> &'a HatchPattern {
        let block = library.blocks.iter().find(|b| b.name == block).unwrap();
        match &block.objects[0] {
            Object::Hatch(hatch) => &hatch.pattern,
            other => panic!("a hatch, not {other:?}"),
        }
    }

    #[test]
    fn a_hatch_pattern_named_as_another_defined_otherwise_is_carried_under_a_new_name() {
        // Two blocks of one drawing, each with its own GRID.
        let mut both = drawing(&hatched("A", "[0, 5]"));
        both.blocks.extend(drawing(&hatched("B", "[0, 7]")).blocks);
        let (plan, text) = planned(&both, EMPTY, &[]);
        let back = parse_block_library(&text).unwrap();
        assert_eq!(pattern_of(&back, "A").name, "GRID");
        assert_eq!(pattern_of(&back, "A").lines[0].delta, [0.0, 5.0]);
        assert_eq!(pattern_of(&back, "B").name, "GRID_2");
        assert_eq!(pattern_of(&back, "B").lines[0].delta, [0.0, 7.0]);
        let renamed: Vec<&ImportNotice> = plan
            .imported
            .notices
            .iter()
            .filter(|n| n.code == ImportNoticeCode::PatternRenamed)
            .collect();
        assert_eq!(renamed.len(), 1, "{:?}", plan.imported.notices);
        let message = &renamed[0].message;
        assert!(
            message.contains("`B`") && message.contains("`GRID`") && message.contains("`GRID_2`"),
            "{message}"
        );
        assert_eq!(
            plan.imported.tables_added.hatch_patterns,
            ["GRID", "GRID_2"]
        );

        // Against the library's own GRID: the same is shared, another renamed,
        // past a GRID_2 the library defines otherwise too.
        let held = "units: mm\nhatch_patterns:\n  GRID: {lines: [{delta: [0, 7]}]}\n  GRID_2: {lines: [{delta: [0, 9]}]}\nblocks:\n  HELD:\n    objects:\n      - circle: {center: [0, 0, 0], radius: 1}\n";
        let (plan, text) = planned(&both, held, &[]);
        let back = parse_block_library(&text).unwrap();
        assert_eq!(pattern_of(&back, "A").name, "GRID_3");
        assert_eq!(pattern_of(&back, "A").lines[0].delta, [0.0, 5.0]);
        assert_eq!(pattern_of(&back, "B").name, "GRID");
        assert_eq!(plan.imported.tables_added.hatch_patterns, ["GRID_3"]);
        assert!(!plan
            .imported
            .notices
            .iter()
            .any(|n| n.code == ImportNoticeCode::DefinitionKept));
    }

    #[test]
    fn a_hatch_is_written_with_the_name_its_own_definition_was_given() {
        // Two definitions the same to a billionth are one, and both differ
        // from the library's GRID.
        let mut both = drawing(&hatched("A", "[0, 5]"));
        both.blocks
            .extend(drawing(&hatched("B", "[0, 5.0000000001]")).blocks);
        let held = "units: mm
hatch_patterns:
  GRID: {lines: [{delta: [0, 7]}]}
blocks:
  HELD:
    objects:
      - circle: {center: [0, 0, 0], radius: 1}
";
        let (plan, text) = planned(&both, held, &[]);
        assert_eq!(plan.imported.tables_added.hatch_patterns, ["GRID_2"]);
        let back = parse_block_library(&text).unwrap();
        for block in ["A", "B"] {
            assert_eq!(pattern_of(&back, block).name, "GRID_2", "{block}: {text}");
            assert_eq!(
                pattern_of(&back, block).lines[0].delta,
                [0.0, 5.0],
                "{block}"
            );
        }
    }

    #[test]
    fn a_short_sweep_keeps_its_ends_as_read_rather_than_close_up() {
        let library = "units: mm
blocks:
  SWEEPS:
    objects:
      - arc: {center: [0, 0, 0], radius: 1, start_angle: 0, end_angle: 0.0000000004}
      - ellipse: {center: [0, 0, 0], major_axis: [2, 0, 0], ratio: 0.5, start_angle: 30, end_angle: 30.0000000004}
      - arc: {center: [0, 0, 0], radius: 1, start_angle: 29.999999999999996, end_angle: 90}
";
        let (_, text) = planned(&drawing(library), EMPTY, &[]);
        let back = parse_block_library(&text).unwrap();
        let objects = &back.blocks[0].objects;
        let Object::Arc(short) = &objects[0] else {
            panic!("an arc: {text}")
        };
        assert_eq!(
            (short.start_angle, short.end_angle),
            (0.0, 0.000_000_000_4),
            "{text}"
        );
        let Object::Ellipse(elliptical) = &objects[1] else {
            panic!("an ellipse: {text}")
        };
        assert!(!elliptical.is_whole(), "{text}");
        assert_eq!(
            (elliptical.start_angle, elliptical.end_angle),
            (30.0, 30.000_000_000_4)
        );
        // Ends that stay apart are still snapped.
        assert!(text.contains("start_angle: 30, end_angle: 90"), "{text}");
    }

    /// A made-up block that defines NOTE twice, the second in lowercase,
    /// and one that nests it, giving both values.
    const NESTED: &str = "units: mm
blocks:
  CHILD:
    objects:
      - attdef: {tag: NOTE, position: [0, 0, 0], height: 1}
      - attdef: {tag: NOTE, position: [0, 2, 0], height: 1}
  PARENT:
    objects:
      - insert: {block: CHILD, position: [0, 0, 0], attributes: {NOTE: [A, B]}}
";

    /// The drawing of [`NESTED`] as a DWG may hold it, the child's second
    /// tag, and the parent's value for it, written in lowercase.
    fn lowercase_tags() -> DrawingBlocks {
        let mut read = drawing(NESTED);
        for block in &mut read.blocks {
            match block.block.objects.last_mut() {
                Some(Object::AttributeDefinition(d)) => d.tag = "note".into(),
                Some(Object::Insert(Insert { attributes, .. })) => {
                    attributes[1].tag = "note".into()
                }
                _ => {}
            }
        }
        read
    }

    #[test]
    fn a_tag_in_lowercase_is_taken_in_capitals_before_values_are_grouped_and_blocks_compared() {
        let (_, text) = planned(&lowercase_tags(), EMPTY, &[]);
        assert!(text.contains("attributes: {NOTE: [A, B]}"), "{text}");
        let back = parse_block_library(&text).unwrap();
        let original = parse_block_library(NESTED).unwrap();
        assert_eq!(back.blocks, original.blocks);

        // The child the library holds, the same but for the tag's case, is
        // used as it is when the parent is imported after it.
        let held = "units: mm\nblocks:\n  CHILD:\n    objects:\n      - attdef: {tag: NOTE, position: [0, 0, 0], height: 1}\n      - attdef: {tag: NOTE, position: [0, 2, 0], height: 1}\n";
        let (plan, text) = planned(&lowercase_tags(), held, &["PARENT"]);
        assert_eq!(plan.imported.blocks_added.len(), 1);
        assert_eq!(plan.imported.blocks_added[0].name, "PARENT");
        assert!(text.starts_with(held), "{text}");
    }

    #[test]
    fn a_block_skipped_whole_still_says_what_it_could_not_carry() {
        let mut read = drawing(&hatched("A", "[0, 5]"));
        read.blocks.push(ReadBlock {
            block: Block {
                name: "DIMS".into(),
                base_point: Point3::new(0.0, 0.0, 0.0),
                objects: Vec::new(),
                cells: Vec::new(),
            },
            not_carried: vec![("DIMENSION", 2), ("LEADER", 1)],
        });
        for wanted in [&[][..], &["dims", "A"]] {
            let existing = Existing::read(EMPTY).unwrap();
            let wanted: Vec<String> = wanted.iter().map(|w| (*w).to_owned()).collect();
            let plan = plan(&read, &existing, &wanted, false, "--replace").unwrap();
            assert_eq!(plan.imported.skipped.len(), 1, "{wanted:?}");
            assert_eq!(plan.imported.skipped[0].block, "DIMS");
            let lost: Vec<(&str, &str, usize)> = plan
                .imported
                .not_carried
                .iter()
                .map(|n| (n.block.as_str(), n.object_type.as_str(), n.count))
                .collect();
            assert_eq!(
                lost,
                [("DIMS", "DIMENSION", 2), ("DIMS", "LEADER", 1)],
                "{wanted:?}"
            );
        }
    }

    #[test]
    fn an_angle_read_from_radians_is_written_as_it_was_drawn() {
        let off = 29.999_999_999_999_996;
        let names = Names::default();
        let written = |object: Object| {
            let value = object_value(&object, &names);
            value[object.type_name()].clone()
        };
        let mut insert = match &drawing(NESTED).blocks[1].block.objects[0] {
            Object::Insert(insert) => insert.clone(),
            _ => unreachable!(),
        };
        insert.rotation = off;
        assert_eq!(written(Object::Insert(insert))["rotation"], json!(30));
        let arc = Arc {
            center: Point3::new(0.0, 0.0, 0.0),
            radius: 1.0,
            start_angle: off,
            end_angle: 359.999_999_999_999_94,
            layer: LAYER_ZERO.into(),
            properties: Properties::default(),
        };
        let arc = written(Object::Arc(arc));
        assert_eq!(
            (&arc["start_angle"], &arc["end_angle"]),
            (&json!(30), &json!(0)),
            "{arc}"
        );
        let mut pattern = parse_block_library(&hatched("A", "[0, 5]")).unwrap().blocks[0].clone();
        if let Object::Hatch(hatch) = &mut pattern.objects[0] {
            hatch.pattern.lines[0].angle = 44.999_999_999_999_99;
            let keys = hatch_pattern_keys(&hatch.pattern);
            assert_eq!(keys["lines"][0]["angle"], json!(45.0));
            hatch.angle = off;
            hatch.scale = 2.0;
            assert_eq!(written(Object::Hatch(hatch.clone()))["angle"], json!(30));
        }
        // An angle drawn with more digits keeps them, to a billionth.
        assert_eq!(degrees(12.345_678_9), 12.345_678_9);
        assert_eq!(degrees(0.1), 0.1);
    }

    /// A made-up drawing set in `job/` whose template names the library
    /// `shared/blocks.yaml`, outside the set's folder, and a made-up drawing
    /// whose block that library holds.
    fn shared_library(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("agentdrafter-import-unit-{}", std::process::id()))
            .join(test);
        let _ = fs::remove_dir_all(&dir);
        for folder in ["job/templates", "shared", "parts", "elsewhere"] {
            fs::create_dir_all(dir.join(folder)).unwrap();
        }
        fs::write(
            dir.join("job/set.yaml"),
            "units: mm\noutput: dwg\nrows: rows.yaml\n",
        )
        .unwrap();
        fs::write(
            dir.join("job/rows.yaml"),
            "rows:\n  - {sheet: S-1, template: templates/sheet.yaml}\n",
        )
        .unwrap();
        fs::write(
            dir.join("job/templates/sheet.yaml"),
            "units: mm\nblock_library: ../../shared/blocks.yaml\nobjects: []\n",
        )
        .unwrap();
        let library = "units: mm\nblocks:\n  MARK:\n    objects:\n      - circle: {center: [0, 0, 0], radius: 2}\n";
        fs::write(dir.join("shared/blocks.yaml"), library).unwrap();
        fs::write(
            dir.join("parts/blocks.yaml"),
            library.replace("radius: 2", "radius: 3"),
        )
        .unwrap();
        fs::write(
            dir.join("parts/sheet.yaml"),
            "units: mm\nblock_library: blocks.yaml\nobjects:\n  - insert: {block: MARK, position: [0, 0, 0]}\n",
        )
        .unwrap();
        build(&BuildRequest {
            description: dir.join("parts/sheet.yaml"),
            output: None,
            overwrite: false,
        })
        .unwrap();
        dir
    }

    #[test]
    fn a_set_s_library_outside_its_folder_is_found_from_the_folder_the_command_runs_in() {
        let dir = shared_library("found");
        let library = dir.join("shared/blocks.yaml");
        let before = fs::read(&library).unwrap();
        let request = ImportBlocksRequest {
            drawing: dir.join("parts/sheet.dwg"),
            library: library.clone(),
            blocks: Vec::new(),
            replace: true,
            units: None,
            dry_run: false,
        };
        // Run in the set's folder, or below it, the set is found.
        for here in ["job", "job/templates"] {
            let error = import_blocks_in(&request, &dir.join(here)).unwrap_err();
            assert_eq!(error.code, ErrorCode::Usage, "{here}: {error:?}");
            assert!(error.message.contains("import_blocks"), "{error:?}");
            assert_eq!(fs::read(&library).unwrap(), before);
        }
        // Run elsewhere, it is not, and replacing says what that misses.
        let report = import_blocks_in(&request, &dir.join("elsewhere")).unwrap();
        assert_eq!(report.imported.blocks_replaced.len(), 1);
        let notices = &report.imported.notices;
        assert!(
            notices
                .iter()
                .any(|n| n.code == ImportNoticeCode::NotJournaled
                    && n.message.contains("import_blocks")),
            "{notices:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
