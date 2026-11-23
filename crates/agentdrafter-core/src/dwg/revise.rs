//! Direct in-place revision of an existing DWG behind the `dwg` seam (ADR-0004,
//! ADR-0006, ADR-0009).
//!
//! Only this module and its siblings in `dwg/` touch acadrust. Untouched
//! objects keep their `raw_record` bytes, handles, extended data, tables,
//! dictionaries and custom records verbatim; `raw_record` is cleared only on
//! objects actually edited, and the revision is always written back in the
//! exact DWG version the original arrived in.

use std::collections::{BTreeMap, HashSet};
use std::f64::consts::TAU;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use acadrust::entities::{
    AttributeEntity, BoundaryEdge, EntityCommon, EntityType, HorizontalAlignment,
    Insert as DwgInsert, LwVertex, MText as DwgMText, MirrorOptions, Text as DwgText,
    TextHorizontalAlignment, TextVerticalAlignment, Vertex2D, Vertex3DPolyline, VerticalAlignment,
};
use acadrust::objects::ObjectType;
use acadrust::tables::{Layer as DwgLayer, TableEntry};
use acadrust::types::{
    Color, DxfVersion, Handle, LineWeight, Transform as DwgTransform, Vector2, Vector3,
};
use acadrust::{CadDocument, DwgWriter};
use serde::Serialize;
use serde_json::{Map, Value};

use super::{
    block, contents, dwg_object, handle_text, ids, kind_of, layer_of, panic_message, point,
    raw_dwg_version, read_drawing_bytes_with_stats, read_layer, read_properties,
    set_layer_properties, set_properties, text, vector,
};
use crate::description::{
    normalize_degrees, parse_angle_json, parse_length_json, parse_linetype_scale_json,
    parse_object_color_json, parse_object_lineweight_json, parse_plain_number_json,
    parse_point_json, sin_cos_degrees,
};
use crate::error::{Error, ErrorCode};
use crate::model::block::{attribute_text, readable};
use crate::model::{
    Block, DecodeReport, DrawingContents, Layer, Linetype, Object, ObjectColor, ObjectLineweight,
    Point3, Properties, BY_BLOCK, BY_LAYER, CONTINUOUS, LAYER_ZERO,
};
use crate::text::{Attachment, Justify, Text as TextObject, TextStyle, STANDARD, WIDTH_FACTORS};
use crate::units::Unit;

/// One change found by comparing the original drawing against the re-read
/// revision drawing outside the handles the change set requested.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UnexpectedChange {
    /// The record's kind as the DWG names it, such as `LINE`, `INSERT`, or `LAYER`.
    pub kind: String,
    /// The record's handle in hexadecimal (or name for a table entry).
    pub handle: String,
    /// What happened outside the requested operations.
    pub change: UnexpectedChangeKind,
    /// Short human-readable explanation of what differed.
    pub detail: String,
}

/// Kind of unexpected difference detected by the structural self-check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnexpectedChangeKind {
    Added,
    Removed,
    Changed,
}

/// All table names and block definitions present in the open drawing so
/// `add_*` and `change` can target any existing layer, linetype, text style,
/// layout, or block even when the core cannot model every object inside it.
pub(crate) struct RevisionTables {
    pub layers: Vec<Layer>,
    pub linetypes: Vec<Linetype>,
    pub styles: Vec<TextStyle>,
    pub blocks: Vec<Block>,
    pub layouts: Vec<String>,
}

/// An entity's space in the drawing: model space or a named layout's paper space.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TargetSpace {
    Model,
    Layout(String),
}

/// What a single direct mutation touched in the drawing, by hex handles.
#[derive(Clone, Debug, Default)]
pub(crate) struct MutationHandles {
    pub touched: Vec<String>,
    pub added: Vec<String>,
    pub deleted: Vec<String>,
}

/// Structural digest of one drawing object for the pre-/post-revision diff check.
#[derive(Clone, Debug, PartialEq)]
struct ObjectDigest {
    kind: String,
    owner: u64,
    /// Exact raw bytes for R2004+ non-compound passthrough objects.
    raw_bytes: Option<Vec<u8>>,
    /// Field-complete logical digest for pre-R2004 or compound objects that
    /// acadrust re-encodes instead of streaming verbatim, and for diagnostics.
    logical: String,
}

/// Structural digest of one non-graphical object in `drawing.objects`.
#[derive(Clone, Debug, PartialEq)]
struct NonGraphicalDigest {
    kind: String,
    owner: u64,
    summary: String,
}

/// Structural digest of one layer table entry.
#[derive(Clone, Debug, PartialEq)]
struct LayerDigest {
    handle: u64,
    name: String,
    color: String,
    color_name: Option<String>,
    book_name: Option<String>,
    linetype: String,
    lineweight: String,
    flags: (bool, bool, bool, bool, bool, bool),
    transparency: String,
    description: String,
}

/// Structural digest of one block record table entry.
#[derive(Clone, Debug, PartialEq)]
struct BlockRecordDigest {
    handle: u64,
    name: String,
    flags: String,
    layout_handle: u64,
    entity_handles: Vec<u64>,
}

/// Pre-edit structural snapshot of an original drawing.
struct DrawingSnapshot {
    dwg_version: String,
    decode: DecodeReport,
    objects: BTreeMap<u64, ObjectDigest>,
    /// Handles of owned sub-entities (Insert SEQENDs, Polyline3D vertices)
    /// that live inside their parent entity rather than top-level `entities`.
    sub_handles: HashSet<u64>,
    non_graphical: BTreeMap<u64, NonGraphicalDigest>,
    layers: BTreeMap<String, LayerDigest>,
    line_types: BTreeMap<String, String>,
    text_styles: BTreeMap<String, String>,
    dim_styles: BTreeMap<String, String>,
    block_records: BTreeMap<String, BlockRecordDigest>,
}

/// An open original DWG ready for in-place revision, strictly behind the `dwg`
/// seam.
pub(crate) struct RevisionSession {
    drawing: CadDocument,
    original_header: Option<[u8; 6]>,
    raw_version: Option<String>,
    dwg_version: String,
    decode: DecodeReport,
    initial_contents: DrawingContents,
    snapshot: DrawingSnapshot,
    expected_touched: HashSet<u64>,
    expected_added: HashSet<u64>,
    expected_deleted: HashSet<u64>,
    expected_spaces_mutated: HashSet<u64>,
    expected_layers_added: HashSet<String>,
    expected_layers_changed: HashSet<String>,
    app_registered: bool,
}

impl RevisionSession {
    /// Opens an original DWG from `bytes` already read from `path`, trying
    /// strict reading first and falling back to read-past-damage when a damaged
    /// DWG still has recoverable records (so `--allow-loss` can revise it and
    /// record `read_past_damage`, while without `--allow-loss` it is refused
    /// with `incomplete_decode`).
    pub(crate) fn open(bytes: &[u8], path: &Path) -> Result<Self, Error> {
        let original_header: Option<[u8; 6]> = bytes.get(..6).and_then(|s| s.try_into().ok());
        let raw_version = raw_dwg_version(bytes);
        let (drawing, stats, read_past_damage) =
            match read_drawing_bytes_with_stats(bytes, path, false) {
                Ok((drawing, stats)) => (drawing, stats, None),
                Err(strict_err) => {
                    // Only attempt read-past-damage on files with a DWG header.
                    if raw_version.is_none() {
                        return Err(strict_err);
                    }
                    match read_drawing_bytes_with_stats(bytes, path, true) {
                        Ok((drawing, stats)) => (drawing, stats, Some(strict_err.message)),
                        Err(_) => return Err(strict_err),
                    }
                }
            };
        let mut initial_contents = contents(&drawing, &stats, raw_version.clone())?;
        if let Some(reason) = read_past_damage {
            initial_contents.decode.decoded = false;
            initial_contents.decode.read_past_damage = Some(reason);
        }
        let decode = initial_contents.decode.clone();
        let dwg_version = initial_contents.dwg_version.clone();
        let snapshot = snapshot_drawing(&drawing, &dwg_version, &decode);
        Ok(Self {
            drawing,
            original_header,
            raw_version,
            dwg_version,
            decode,
            initial_contents,
            snapshot,
            expected_touched: HashSet::new(),
            expected_added: HashSet::new(),
            expected_deleted: HashSet::new(),
            expected_spaces_mutated: HashSet::new(),
            expected_layers_added: HashSet::new(),
            expected_layers_changed: HashSet::new(),
            app_registered: false,
        })
    }

    pub(crate) fn dwg_version(&self) -> &str {
        &self.dwg_version
    }

    pub(crate) fn decode_report(&self) -> &DecodeReport {
        &self.decode
    }

    pub(crate) fn initial_contents(&self) -> &DrawingContents {
        &self.initial_contents
    }

    /// Re-reads the current in-memory drawing into [`DrawingContents`] so snaps
    /// and target lookups see objects added, moved, or deleted by earlier
    /// operations in the same change set.
    pub(crate) fn current_contents(&self) -> Result<DrawingContents, Error> {
        let empty_stats = acadrust::ReadStats::default();
        contents(&self.drawing, &empty_stats, self.raw_version.clone())
    }

    /// Verifies that pinned acadrust can write this drawing back with the exact
    /// 6-byte DWG version header the original arrived in (ADR-0006), refusing
    /// cleanly rather than silently converting versions (such as `AC1006` ->
    /// `AC1009` or `AD1032` -> `AC1032`).
    pub(crate) fn check_writable_version(&self) -> Result<(), Error> {
        let Some(expected_header) = writable_header_bytes(self.drawing.version) else {
            return Err(Error::new(
                ErrorCode::DwgWrite,
                format!(
                    "the DWG library cannot write DWG version `{}` without converting it",
                    self.dwg_version
                ),
            ));
        };
        if let Some(orig) = self.original_header {
            if orig != expected_header {
                let raw = String::from_utf8_lossy(&orig);
                let target = std::str::from_utf8(&expected_header).unwrap_or("?");
                return Err(Error::new(
                    ErrorCode::DwgWrite,
                    format!(
                        "the DWG library reads `{raw}` as `{target}` and cannot write `{raw}` \
                         without changing its 6-byte DWG version header; revising is refused to \
                         preserve the original version (ADR-0006)"
                    ),
                ));
            }
        }
        Ok(())
    }

    /// Returns all table entries and block definitions present in the drawing
    /// (including entries the core cannot fully model) so `add_*` and `change`
    /// can reference any existing layer, linetype, text style, layout, or block.
    pub(crate) fn tables_for_parsing(&self) -> RevisionTables {
        let layers: Vec<Layer> = self
            .drawing
            .layers
            .iter()
            .map(|l| {
                read_layer(l).unwrap_or_else(|| {
                    let mut fallback = Layer::new(l.name.clone());
                    fallback.on = !l.flags.off;
                    fallback.frozen = l.flags.frozen;
                    fallback.locked = l.flags.locked;
                    fallback.plot = l.is_plottable;
                    fallback.description = l.description.clone();
                    fallback
                })
            })
            .collect();
        let linetypes: Vec<Linetype> = self
            .drawing
            .line_types
            .iter()
            .filter(|lt| {
                !crate::model::BUILT_IN_LINETYPES
                    .iter()
                    .any(|b| b.eq_ignore_ascii_case(&lt.name))
            })
            .map(|lt| Linetype {
                name: lt.name.clone(),
                description: lt.description.clone(),
                pattern: lt.elements.iter().map(|e| e.length).collect(),
            })
            .collect();
        let styles: Vec<TextStyle> = self
            .drawing
            .text_styles
            .iter()
            .filter(|s| !s.is_shape_file && !s.name.is_empty())
            .map(|s| TextStyle {
                name: s.name.clone(),
                font: if s.font_file.trim().is_empty() {
                    "txt.shx".to_owned()
                } else if s.font_file.contains('.') {
                    s.font_file.clone()
                } else {
                    format!("{}.shx", s.font_file)
                },
                width_factor: if (WIDTH_FACTORS.0..=WIDTH_FACTORS.1).contains(&s.width_factor) {
                    s.width_factor
                } else {
                    1.0
                },
            })
            .collect();
        let blocks = self.all_insertable_blocks();
        let layouts = self.layout_names();
        RevisionTables {
            layers,
            linetypes,
            styles,
            blocks,
            layouts,
        }
    }

    /// All non-anonymous, non-xref block definitions in the drawing, extracting
    /// each block's attribute definitions even if the block also holds objects
    /// the core has no model for.
    fn all_insertable_blocks(&self) -> Vec<Block> {
        self.drawing
            .block_records
            .iter()
            .filter(|r| {
                !(r.is_model_space()
                    || r.is_paper_space()
                    || r.is_layout()
                    || r.is_anonymous()
                    || r.flags.is_xref
                    || r.flags.is_xref_overlay
                    || !r.xref_path.is_empty())
            })
            .map(|record| {
                let mut objects = Vec::new();
                for dwg_obj in self.drawing.entities_in_block(&record.name) {
                    if let EntityType::AttributeDefinition(def) = dwg_obj {
                        if let Some(parsed) =
                            block::read_attribute_definition(def, layer_of(&def.common))
                        {
                            objects.push(Object::AttributeDefinition(parsed));
                        }
                    }
                }
                Block {
                    name: record.name.clone(),
                    base_point: point(record.base_point),
                    objects,
                    cells: Vec::new(),
                }
            })
            .collect()
    }

    /// All paper-space layout names in tab order.
    pub(crate) fn layout_names(&self) -> Vec<String> {
        let model = self.drawing.header.model_space_block_handle;
        let mut layouts: Vec<(&acadrust::objects::Layout, u64)> = self
            .drawing
            .objects
            .values()
            .filter_map(|o| match o {
                ObjectType::Layout(l) if l.block_record != model && !l.block_record.is_null() => {
                    Some((l, l.handle.value()))
                }
                _ => None,
            })
            .collect();
        layouts.sort_by_key(|(l, h)| (l.tab_order, *h));
        layouts.into_iter().map(|(l, _)| l.name.clone()).collect()
    }

    fn is_layer_locked(&self, layer_name: &str) -> bool {
        self.drawing
            .layers
            .iter()
            .find(|l| l.name.eq_ignore_ascii_case(layer_name))
            .is_some_and(|l| l.flags.locked)
    }

    /// Returns whether the sheet object at hex handle `hex` exists, and its
    /// layer name if locked. Refuses handles belonging to block definitions.
    pub(crate) fn locked_layer_of(&self, hex: &str) -> Result<Option<String>, Error> {
        let handle = parse_hex_handle(hex)?;
        let _ = self.ensure_sheet_entity(handle, hex)?;
        let Some(object) = self.drawing.get_entity(handle) else {
            return Err(Error::new(
                ErrorCode::UnknownName,
                format!("no object with handle `{hex}` in the drawing"),
            ));
        };
        let layer = layer_of(object.common());
        Ok(self.is_layer_locked(&layer).then_some(layer))
    }

    fn sheet_space_of_owner(&self, owner: Handle) -> Option<TargetSpace> {
        if owner.is_null() || owner == self.drawing.header.model_space_block_handle {
            return Some(TargetSpace::Model);
        }
        for obj in self.drawing.objects.values() {
            if let ObjectType::Layout(layout) = obj {
                if layout.block_record == owner {
                    return Some(TargetSpace::Layout(layout.name.clone()));
                }
            }
        }
        if !self.drawing.header.paper_space_block_handle.is_null()
            && owner == self.drawing.header.paper_space_block_handle
        {
            if let Some(name) = self.drawing.objects.values().find_map(|obj| {
                if let ObjectType::Layout(l) = obj {
                    if !l.name.eq_ignore_ascii_case("Model")
                        && l.block_record == self.drawing.header.paper_space_block_handle
                    {
                        return Some(l.name.clone());
                    }
                }
                None
            }) {
                return Some(TargetSpace::Layout(name));
            }
        }
        None
    }

    fn ensure_sheet_entity(&self, handle: Handle, hex: &str) -> Result<TargetSpace, Error> {
        let Some(object) = self.drawing.get_entity(handle) else {
            return Err(Error::new(
                ErrorCode::UnknownName,
                format!("no object with handle `{hex}` in the drawing"),
            ));
        };
        self.sheet_space_of_owner(object.common().owner_handle)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::BadValue,
                    format!(
                        "object `{hex}` belongs to a block definition, not a sheet space, so it \
                         cannot be targeted as a sheet object"
                    ),
                )
            })
    }

    fn agentdrafter_app_handle(&self) -> Option<u64> {
        self.drawing
            .app_ids
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(block::APPLICATION))
            .map(|a| a.handle.value())
    }

    /// Adds a newly parsed [`Object`] directly to model space or `space`,
    /// allocating a fresh handle. Writes AgentDrafter xdata only when an
    /// explicit `id` or `name` was requested, and never writes an empty `id`.
    pub(crate) fn add_object(
        &mut self,
        object: &Object,
        space: &TargetSpace,
        sheet_name: &str,
        explicit_id: Option<&str>,
        explicit_name: Option<&str>,
        start_of: &dyn Fn(&TextObject) -> Point3,
    ) -> Result<(String, MutationHandles), Error> {
        let blocks = self.all_insertable_blocks();
        let mut dwg_obj = dwg_object(object, start_of, &blocks)?;
        match (explicit_id, explicit_name) {
            (Some(id_part), name_part) => {
                self.ensure_application_registered()?;
                ids::write_id_and_name(dwg_obj.common_mut(), sheet_name, id_part, name_part);
            }
            (None, Some(name_part)) => {
                self.ensure_application_registered()?;
                ids::write_name_only(dwg_obj.common_mut(), name_part);
            }
            (None, None) => {}
        }
        let handle = self.insert_into_space(dwg_obj, space)?;
        let hex = handle_text(handle);
        let mut record = MutationHandles::default();
        self.expected_added.insert(handle.value());
        record.added.push(hex.clone());
        if let Some(owner) = self
            .drawing
            .get_entity(handle)
            .map(|e| e.common().owner_handle.value())
        {
            if owner != 0 {
                self.expected_spaces_mutated.insert(owner);
            }
        }
        self.assign_child_handles(handle, &mut record);
        Ok((hex, record))
    }

    /// Assigns concrete handles to any unallocated owned sub-entities on an
    /// added, edited, or cloned `Insert` (attributes + seqend) or `Polyline3D`
    /// (vertices) so every handle written is unique and tracked.
    fn assign_child_handles(&mut self, entity_handle: Handle, record: &mut MutationHandles) {
        enum Need {
            None,
            Insert { need_att: Vec<bool>, need_seq: bool },
            Polyline3D { need_verts: Vec<bool> },
        }
        let need = match self.drawing.get_entity(entity_handle) {
            Some(EntityType::Insert(ins)) if !ins.attributes.is_empty() => Need::Insert {
                need_att: ins
                    .attributes
                    .iter()
                    .map(|a| a.common.handle.is_null())
                    .collect(),
                need_seq: ins.seqend_handle.is_none_or(|h| h.is_null()),
            },
            Some(EntityType::Polyline3D(p3)) if !p3.vertices.is_empty() => Need::Polyline3D {
                need_verts: p3.vertices.iter().map(|v| v.handle.is_null()).collect(),
            },
            _ => Need::None,
        };
        match need {
            Need::None => {}
            Need::Insert { need_att, need_seq } => {
                let allocated_atts: Vec<Option<Handle>> = need_att
                    .into_iter()
                    .map(|n| n.then(|| self.drawing.allocate_handle()))
                    .collect();
                let allocated_seq = need_seq.then(|| self.drawing.allocate_handle());
                if let Some(EntityType::Insert(ins)) = self.drawing.get_entity_mut(entity_handle) {
                    ins.common.raw_record = None;
                    for (att, maybe_h) in ins.attributes.iter_mut().zip(allocated_atts) {
                        att.common.owner_handle = entity_handle;
                        att.common.raw_record = None;
                        if let Some(h) = maybe_h {
                            att.common.handle = h;
                            self.expected_added.insert(h.value());
                            record.added.push(handle_text(h));
                        }
                    }
                    if let Some(seq_h) = allocated_seq {
                        ins.seqend_handle = Some(seq_h);
                        self.expected_added.insert(seq_h.value());
                        record.added.push(handle_text(seq_h));
                    }
                }
            }
            Need::Polyline3D { need_verts } => {
                let allocated_verts: Vec<Option<Handle>> = need_verts
                    .into_iter()
                    .map(|n| n.then(|| self.drawing.allocate_handle()))
                    .collect();
                if let Some(EntityType::Polyline3D(p3)) = self.drawing.get_entity_mut(entity_handle)
                {
                    p3.common.raw_record = None;
                    for (v, maybe_h) in p3.vertices.iter_mut().zip(allocated_verts) {
                        if let Some(h) = maybe_h {
                            v.handle = h;
                            self.expected_added.insert(h.value());
                            record.added.push(handle_text(h));
                        }
                    }
                }
            }
        }
    }

    fn insert_into_space(
        &mut self,
        dwg_obj: EntityType,
        space: &TargetSpace,
    ) -> Result<Handle, Error> {
        let res = match space {
            TargetSpace::Model => self.drawing.add_entity(dwg_obj),
            TargetSpace::Layout(layout_name) => {
                self.drawing.add_entity_to_layout(dwg_obj, layout_name)
            }
        };
        res.map_err(|e| {
            Error::new(
                ErrorCode::DwgWrite,
                format!("could not add object to the drawing: {e}"),
            )
        })
    }

    fn ensure_application_registered(&mut self) -> Result<(), Error> {
        if self.drawing.app_ids.get(block::APPLICATION).is_none() {
            block::register_application(&mut self.drawing)?;
            self.drawing.assign_table_entry_handles();
            self.app_registered = true;
        }
        Ok(())
    }

    /// Sets one attribute value on the `Insert` at `hex`, preserving its
    /// handle, style, position, alignment, xdata, flags, and sibling
    /// attributes. Checks both the insert's and the attribute's own layer for
    /// lock, and refuses constant attributes without counting them toward
    /// `occurrence`.
    pub(crate) fn set_attribute(
        &mut self,
        hex: &str,
        tag: &str,
        occurrence: Option<usize>,
        value: &str,
    ) -> Result<(String, MutationHandles), Error> {
        let handle = parse_hex_handle(hex)?;
        let _ = self.ensure_sheet_entity(handle, hex)?;
        let (matching_indices, has_constant_match, insert_layer) =
            match self.drawing.get_entity(handle) {
                Some(EntityType::Insert(ins)) => {
                    let mut editable = Vec::new();
                    let mut saw_constant = false;
                    for (i, a) in ins.attributes.iter().enumerate() {
                        if a.tag.eq_ignore_ascii_case(tag) {
                            if a.flags.constant {
                                saw_constant = true;
                            } else {
                                editable.push(i);
                            }
                        }
                    }
                    (editable, saw_constant, layer_of(&ins.common))
                }
                Some(other) => {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        format!("set_attribute works on an insert, not a {}", kind_of(other)),
                    ))
                }
                None => {
                    return Err(Error::new(
                        ErrorCode::UnknownName,
                        format!("no object with handle `{hex}` in the drawing"),
                    ))
                }
            };
        if self.is_layer_locked(&insert_layer) {
            return Err(Error::new(
                ErrorCode::Locked,
                format!(
                    "insert `{hex}` sits on layer `{insert_layer}`, which is locked; unlock it \
                     first with change_layer, set: {{\"locked\": false}}"
                ),
            ));
        }
        let chosen = match (matching_indices.as_slice(), occurrence) {
            ([], _) => {
                if has_constant_match {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        format!(
                            "attribute `{tag}` on insert `{hex}` is constant and cannot be edited"
                        ),
                    ));
                }
                let available: Vec<String> = match self.drawing.get_entity(handle) {
                    Some(EntityType::Insert(ins)) => ins
                        .attributes
                        .iter()
                        .filter(|a| !a.flags.constant)
                        .map(|a| a.tag.clone())
                        .collect(),
                    _ => Vec::new(),
                };
                return Err(Error::new(
                    ErrorCode::UnknownName,
                    if available.is_empty() {
                        format!("insert `{hex}` carries no editable attributes; cannot set `{tag}`")
                    } else {
                        format!(
                            "insert `{hex}` carries no editable attribute `{tag}`; its editable \
                             attributes: {}",
                            available.join(", ")
                        )
                    },
                ));
            }
            ([only], None | Some(1)) => *only,
            ([_], Some(n)) => {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    format!("insert `{hex}` carries `{tag}` once, not {n} times"),
                ));
            }
            (many, None) => {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    format!(
                        "insert `{hex}` carries `{tag}` {} times; pass `occurrence` (1 to {})",
                        many.len(),
                        many.len()
                    ),
                ));
            }
            (many, Some(n)) if n >= 1 && n <= many.len() => many[n - 1],
            (many, Some(n)) => {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    format!(
                        "insert `{hex}` carries `{tag}` {} times, not {n}",
                        many.len()
                    ),
                ));
            }
        };
        let att_layer = match self.drawing.get_entity(handle) {
            Some(EntityType::Insert(ins)) => layer_of(&ins.attributes[chosen].common),
            _ => unreachable!("checked above"),
        };
        if self.is_layer_locked(&att_layer) {
            return Err(Error::new(
                ErrorCode::Locked,
                format!(
                    "attribute `{tag}` on insert `{hex}` sits on layer `{att_layer}`, which is \
                     locked; unlock it first with change_layer, set: {{\"locked\": false}}"
                ),
            ));
        }
        let mut record = MutationHandles::default();
        let coded = text::text_code(value);
        let ins = match self.drawing.get_entity_mut(handle) {
            Some(EntityType::Insert(ins)) => ins,
            _ => unreachable!("checked above"),
        };
        ins.common.raw_record = None;
        let att = &mut ins.attributes[chosen];
        let old = att.value.clone();
        att.value = coded;
        att.common.raw_record = None;
        if let Some(mtext) = att.embedded_mtext.as_mut() {
            mtext.value = text::mtext_code(value, None);
            mtext.common.raw_record = None;
        }
        let att_handle = att.common.handle;
        self.expected_touched.insert(handle.value());
        record.touched.push(handle_text(handle));
        if !att_handle.is_null() {
            self.expected_touched.insert(att_handle.value());
            record.touched.push(handle_text(att_handle));
        }
        Ok((
            format!("set attribute `{}` from `{old}` to `{value}`", att.tag),
            record,
        ))
    }

    /// Changes keys on the object at `hex` directly in `CadDocument`,
    /// validating every key before mutating anything.
    pub(crate) fn change_object(
        &mut self,
        hex: &str,
        set: &Map<String, Value>,
        units: Unit,
        sheet_name: &str,
        existing_names: &HashSet<String>,
        start_of: &dyn Fn(&TextObject) -> Point3,
    ) -> Result<(String, MutationHandles), Error> {
        let handle = parse_hex_handle(hex)?;
        let _ = self.ensure_sheet_entity(handle, hex)?;
        let app_handle = self.agentdrafter_app_handle();
        // Validate completely on a cloned copy of the target entity before
        // touching the live drawing (all-or-nothing, and preserves raw_record
        // on failure!).
        let Some(original) = self.drawing.get_entity(handle).cloned() else {
            return Err(Error::new(
                ErrorCode::UnknownName,
                format!("no object with handle `{hex}` in the drawing"),
            ));
        };
        let old_p3_handles: Vec<u64> = match &original {
            EntityType::Polyline3D(p3) => p3
                .vertices
                .iter()
                .filter(|v| !v.handle.is_null())
                .map(|v| v.handle.value())
                .collect(),
            _ => Vec::new(),
        };
        let mut updated = original;
        let mut register_app = false;
        let mut changed_keys: Vec<String> = Vec::new();
        for (key, value) in set {
            match key.as_str() {
                "id" | "copy_of" => {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        format!("`{key}` cannot be changed; an object's ID is fixed for its life"),
                    )
                    .at_field(format!("set.{key}")));
                }
                "attributes" => {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        "use `set_attribute` to change an insert's attributes",
                    )
                    .at_field("set.attributes"));
                }
                "layer" => {
                    let layer_name = if value.is_null() {
                        LAYER_ZERO.to_owned()
                    } else {
                        let Some(name) = value.as_str() else {
                            return Err(Error::new(
                                ErrorCode::WrongType,
                                "expected a layer's name",
                            )
                            .at_field("set.layer"));
                        };
                        let Some(found) = self
                            .drawing
                            .layers
                            .iter()
                            .find(|l| l.name.eq_ignore_ascii_case(name))
                        else {
                            let choices: Vec<&str> = self
                                .drawing
                                .layers
                                .iter()
                                .map(|l| l.name.as_str())
                                .collect();
                            return Err(Error::new(
                                ErrorCode::UnknownName,
                                format!(
                                    "unknown layer `{name}`; the drawing's layers: {}",
                                    choices.join(", ")
                                ),
                            )
                            .at_field("set.layer"));
                        };
                        found.name.clone()
                    };
                    let common = updated.common_mut();
                    common.layer = layer_name;
                    common.layer_handle = None;
                    changed_keys.push("layer".to_owned());
                }
                "color" => {
                    let color = parse_object_color_json(value, "set.color")?;
                    let common = updated.common_mut();
                    common.color = match color {
                        ObjectColor::ByLayer => Color::ByLayer,
                        ObjectColor::ByBlock => Color::ByBlock,
                        ObjectColor::Index(index) => Color::Index(index),
                    };
                    common.color_name = None;
                    common.color_book_handle = None;
                    changed_keys.push("color".to_owned());
                }
                "linetype" => {
                    let lt_name = if value.is_null() {
                        BY_LAYER.to_owned()
                    } else {
                        let Some(name) = value.as_str() else {
                            return Err(Error::new(
                                ErrorCode::WrongType,
                                "expected a linetype's name",
                            )
                            .at_field("set.linetype"));
                        };
                        if name.eq_ignore_ascii_case(BY_LAYER) {
                            BY_LAYER.to_owned()
                        } else if name.eq_ignore_ascii_case(BY_BLOCK) {
                            BY_BLOCK.to_owned()
                        } else if name.eq_ignore_ascii_case(CONTINUOUS) {
                            CONTINUOUS.to_owned()
                        } else if let Some(found) = self
                            .drawing
                            .line_types
                            .iter()
                            .find(|l| l.name.eq_ignore_ascii_case(name))
                        {
                            found.name.clone()
                        } else {
                            let choices: Vec<&str> = self
                                .drawing
                                .line_types
                                .iter()
                                .map(|l| l.name.as_str())
                                .collect();
                            return Err(Error::new(
                                ErrorCode::UnknownName,
                                format!(
                                    "unknown linetype `{name}`; the drawing's linetypes: {}",
                                    choices.join(", ")
                                ),
                            )
                            .at_field("set.linetype"));
                        }
                    };
                    let common = updated.common_mut();
                    common.linetype = lt_name;
                    common.linetype_handle = None;
                    changed_keys.push("linetype".to_owned());
                }
                "lineweight" => {
                    let lw = parse_object_lineweight_json(value, "set.lineweight")?;
                    updated.common_mut().line_weight = match lw {
                        ObjectLineweight::ByLayer => LineWeight::ByLayer,
                        ObjectLineweight::ByBlock => LineWeight::ByBlock,
                        ObjectLineweight::Default => LineWeight::Default,
                        ObjectLineweight::Width(w) => LineWeight::Value(w.hundredths()),
                    };
                    changed_keys.push("lineweight".to_owned());
                }
                "linetype_scale" => {
                    let scale = parse_linetype_scale_json(value, "set.linetype_scale")?;
                    updated.common_mut().linetype_scale = scale;
                    changed_keys.push("linetype_scale".to_owned());
                }
                "name" => {
                    let new_name = if value.is_null() {
                        None
                    } else {
                        let Some(name) = value.as_str() else {
                            return Err(Error::new(
                                ErrorCode::WrongType,
                                "expected text for `name`, or null to remove it",
                            )
                            .at_field("set.name"));
                        };
                        if name.trim().is_empty() {
                            return Err(Error::new(
                                ErrorCode::BadValue,
                                "an object's name cannot be empty",
                            )
                            .at_field("set.name"));
                        }
                        let current_name = ids::read_name(updated.common());
                        let same_as_own = current_name
                            .as_deref()
                            .is_some_and(|c| c.eq_ignore_ascii_case(name));
                        if !same_as_own && existing_names.contains(&name.to_lowercase()) {
                            return Err(Error::new(
                                ErrorCode::DuplicateName,
                                format!("another object on the sheet is already named `{name}`"),
                            )
                            .at_field("set.name"));
                        }
                        Some(name.to_owned())
                    };
                    let carried_id = ids::read_id(updated.common());
                    ids::strip_agentdrafter_xdata(updated.common_mut(), app_handle);
                    match (carried_id.as_deref(), new_name.as_deref()) {
                        (Some(full), name_opt) => {
                            register_app = true;
                            let own_id = full.split_once('/').map(|(_, rest)| rest).unwrap_or(full);
                            ids::write_id_and_name(
                                updated.common_mut(),
                                sheet_name,
                                own_id,
                                name_opt,
                            );
                        }
                        (None, Some(name_str)) => {
                            register_app = true;
                            ids::write_name_only(updated.common_mut(), name_str);
                        }
                        (None, None) => {}
                    }
                    changed_keys.push("name".to_owned());
                }
                other => {
                    apply_type_change(&mut updated, other, value, units, &self.drawing, start_of)?;
                    changed_keys.push(other.to_owned());
                }
            }
        }
        updated.common_mut().raw_record = None;
        if let EntityType::Insert(ins) = &mut updated {
            for att in &mut ins.attributes {
                att.common.raw_record = None;
            }
        }
        if register_app {
            self.ensure_application_registered()?;
        }
        let mut record = MutationHandles::default();
        self.expected_touched.insert(handle.value());
        record.touched.push(handle_text(handle));
        if let EntityType::Insert(ins) = &updated {
            for att in &ins.attributes {
                if !att.common.handle.is_null() {
                    self.expected_touched.insert(att.common.handle.value());
                    record.touched.push(handle_text(att.common.handle));
                }
            }
        }
        if let EntityType::Polyline3D(p3) = &updated {
            let kept: HashSet<u64> = p3
                .vertices
                .iter()
                .filter(|v| !v.handle.is_null())
                .map(|v| v.handle.value())
                .collect();
            for old_h in &old_p3_handles {
                if !kept.contains(old_h) {
                    self.expected_deleted.insert(*old_h);
                    record.deleted.push(format!("{old_h:X}"));
                } else {
                    self.expected_touched.insert(*old_h);
                }
            }
        }
        *self
            .drawing
            .get_entity_mut(handle)
            .expect("target verified above") = updated;
        self.assign_child_handles(handle, &mut record);
        Ok((format!("changed {}", changed_keys.join(", ")), record))
    }

    /// Translates the object at `hex` in place by `by` (`[dx, dy, dz]`).
    pub(crate) fn move_object(
        &mut self,
        hex: &str,
        by: [f64; 3],
    ) -> Result<MutationHandles, Error> {
        let handle = parse_hex_handle(hex)?;
        let _ = self.ensure_sheet_entity(handle, hex)?;
        let Some(original) = self.drawing.get_entity(handle).cloned() else {
            return Err(Error::new(
                ErrorCode::UnknownName,
                format!("no object with handle `{hex}` in the drawing"),
            ));
        };
        let mut moved = original;
        translate_entity(&mut moved, Vector3::new(by[0], by[1], by[2]))?;
        clear_raw_record(&mut moved);
        let record = self.commit_edited_entity(handle, moved);
        Ok(record)
    }

    /// Rotates the object at `hex` in place by `angle` degrees anticlockwise
    /// about `about`.
    pub(crate) fn rotate_object(
        &mut self,
        hex: &str,
        about: [f64; 3],
        angle: f64,
    ) -> Result<MutationHandles, Error> {
        let handle = parse_hex_handle(hex)?;
        let _ = self.ensure_sheet_entity(handle, hex)?;
        let Some(original) = self.drawing.get_entity(handle).cloned() else {
            return Err(Error::new(
                ErrorCode::UnknownName,
                format!("no object with handle `{hex}` in the drawing"),
            ));
        };
        let mut turned = original;
        rotate_entity(&mut turned, about, angle)?;
        clear_raw_record(&mut turned);
        Ok(self.commit_edited_entity(handle, turned))
    }

    /// Scales the object at `hex` in place by `factor` about `about`.
    pub(crate) fn scale_object(
        &mut self,
        hex: &str,
        about: [f64; 3],
        factor: f64,
    ) -> Result<MutationHandles, Error> {
        let handle = parse_hex_handle(hex)?;
        let _ = self.ensure_sheet_entity(handle, hex)?;
        let Some(original) = self.drawing.get_entity(handle).cloned() else {
            return Err(Error::new(
                ErrorCode::UnknownName,
                format!("no object with handle `{hex}` in the drawing"),
            ));
        };
        let mut scaled = original;
        scale_entity(&mut scaled, about, factor)?;
        clear_raw_record(&mut scaled);
        Ok(self.commit_edited_entity(handle, scaled))
    }

    /// Mirrors the object at `hex` across the line through `first` and
    /// `second`: in place when `erase_source` is true, or as a cloned copy with
    /// freshly allocated handle(s) when false.
    pub(crate) fn mirror_object(
        &mut self,
        hex: &str,
        first: [f64; 3],
        second: [f64; 3],
        erase_source: bool,
    ) -> Result<(String, MutationHandles), Error> {
        let handle = parse_hex_handle(hex)?;
        let space = self.ensure_sheet_entity(handle, hex)?;
        let app_handle = self.agentdrafter_app_handle();
        let Some(original) = self.drawing.get_entity(handle).cloned() else {
            return Err(Error::new(
                ErrorCode::UnknownName,
                format!("no object with handle `{hex}` in the drawing"),
            ));
        };
        let mut mirrored = original;
        mirror_entity(&mut mirrored, first, second)?;
        if erase_source {
            clear_raw_record(&mut mirrored);
            let record = self.commit_edited_entity(handle, mirrored);
            Ok((handle_text(handle), record))
        } else {
            sanitize_clone(&mut mirrored, app_handle);
            let new_handle = self.insert_into_space(mirrored, &space)?;
            let new_hex = handle_text(new_handle);
            let mut record = MutationHandles::default();
            self.expected_added.insert(new_handle.value());
            record.added.push(new_hex.clone());
            if let Some(owner) = self
                .drawing
                .get_entity(new_handle)
                .map(|e| e.common().owner_handle.value())
            {
                if owner != 0 {
                    self.expected_spaces_mutated.insert(owner);
                }
            }
            self.assign_child_handles(new_handle, &mut record);
            Ok((new_hex, record))
        }
    }

    /// Copies the object at `hex` into the same space, translated by `by` and
    /// optionally rotated by `turn` degrees about `about`.
    pub(crate) fn copy_object(
        &mut self,
        hex: &str,
        by: [f64; 3],
        turn: f64,
        about: [f64; 3],
    ) -> Result<(String, MutationHandles), Error> {
        let handle = parse_hex_handle(hex)?;
        let space = self.ensure_sheet_entity(handle, hex)?;
        let app_handle = self.agentdrafter_app_handle();
        let Some(original) = self.drawing.get_entity(handle).cloned() else {
            return Err(Error::new(
                ErrorCode::UnknownName,
                format!("no object with handle `{hex}` in the drawing"),
            ));
        };
        let mut copy = original;
        sanitize_clone(&mut copy, app_handle);
        if by != [0.0; 3] {
            translate_entity(&mut copy, Vector3::new(by[0], by[1], by[2]))?;
        }
        if turn != 0.0 {
            rotate_entity(&mut copy, about, turn)?;
        }
        let new_handle = self.insert_into_space(copy, &space)?;
        let new_hex = handle_text(new_handle);
        let mut record = MutationHandles::default();
        self.expected_added.insert(new_handle.value());
        record.added.push(new_hex.clone());
        if let Some(owner) = self
            .drawing
            .get_entity(new_handle)
            .map(|e| e.common().owner_handle.value())
        {
            if owner != 0 {
                self.expected_spaces_mutated.insert(owner);
            }
        }
        self.assign_child_handles(new_handle, &mut record);
        Ok((new_hex, record))
    }

    /// Deletes the object at `hex` (and its owned attributes/seqend if an
    /// `Insert`, or owned vertices if a `Polyline3D`) from `CadDocument`.
    pub(crate) fn delete_object(&mut self, hex: &str) -> Result<MutationHandles, Error> {
        let handle = parse_hex_handle(hex)?;
        let _ = self.ensure_sheet_entity(handle, hex)?;
        let mut record = MutationHandles::default();
        let Some(removed) = self.drawing.remove_entity(handle) else {
            return Err(Error::new(
                ErrorCode::UnknownName,
                format!("no object with handle `{hex}` in the drawing"),
            ));
        };
        // Also strip the handle from its owning block record's entity_handles
        // so in-memory queries see the deletion immediately.
        let owner = removed.common().owner_handle;
        if !owner.is_null() {
            self.expected_spaces_mutated.insert(owner.value());
        }
        for br in self.drawing.block_records.iter_mut() {
            if owner.is_null() || br.handle == owner {
                br.entity_handles.retain(|h| *h != handle);
            }
        }
        self.expected_deleted.insert(handle.value());
        record.deleted.push(handle_text(handle));
        if let EntityType::Insert(ins) = &removed {
            for att in &ins.attributes {
                if !att.common.handle.is_null() {
                    self.expected_deleted.insert(att.common.handle.value());
                    record.deleted.push(handle_text(att.common.handle));
                }
            }
            if let Some(seq) = ins.seqend_handle {
                if !seq.is_null() {
                    self.expected_deleted.insert(seq.value());
                    record.deleted.push(handle_text(seq));
                }
            }
        } else if let EntityType::Polyline3D(p3) = &removed {
            for v in &p3.vertices {
                if !v.handle.is_null() {
                    self.expected_deleted.insert(v.handle.value());
                    record.deleted.push(handle_text(v.handle));
                }
            }
        }
        Ok(record)
    }

    /// Adds a new layer to `drawing.layers`, refusing duplicates (case-insensitively)
    /// and never dropping existing layers.
    pub(crate) fn add_layer(&mut self, layer: &Layer) -> Result<MutationHandles, Error> {
        if self
            .drawing
            .layers
            .iter()
            .any(|l| l.name.eq_ignore_ascii_case(&layer.name))
        {
            return Err(Error::new(
                ErrorCode::DuplicateName,
                format!("the drawing already has a layer named `{}`", layer.name),
            ));
        }
        let mut dwg_layer = DwgLayer::new(layer.name.clone());
        set_layer_properties(&mut dwg_layer, layer);
        dwg_layer.set_handle(self.drawing.allocate_handle());
        let handle = dwg_layer.handle;
        self.drawing.layers.add(dwg_layer).map_err(|e| {
            Error::new(
                ErrorCode::DwgWrite,
                format!("could not add layer `{}` to the drawing: {e}", layer.name),
            )
        })?;
        self.expected_layers_added.insert(layer.name.to_lowercase());
        Ok(MutationHandles {
            added: vec![handle_text(handle)],
            ..Default::default()
        })
    }

    /// Returns the existing [`Layer`] named `name` (case-insensitively) for
    /// `change_layer`, or an error if it does not exist.
    pub(crate) fn existing_layer(&self, name: &str) -> Result<Layer, Error> {
        let Some(found) = self
            .drawing
            .layers
            .iter()
            .find(|l| l.name.eq_ignore_ascii_case(name))
        else {
            let choices: Vec<&str> = self
                .drawing
                .layers
                .iter()
                .map(|l| l.name.as_str())
                .collect();
            return Err(Error::new(
                ErrorCode::UnknownName,
                format!(
                    "unknown layer `{name}`; the drawing's layers: {}",
                    choices.join(", ")
                ),
            ));
        };
        Ok(read_layer(found).unwrap_or_else(|| {
            let mut fallback = Layer::new(found.name.clone());
            fallback.on = !found.flags.off;
            fallback.frozen = found.flags.frozen;
            fallback.locked = found.flags.locked;
            fallback.plot = found.is_plottable;
            fallback.description = found.description.clone();
            fallback
        }))
    }

    /// Modifies ONLY the fields named in `keys` on an existing layer in
    /// `drawing.layers`, preserving unmodeled true colors, color books,
    /// lineweights, transparency, plot styles, and handles verbatim.
    pub(crate) fn change_layer_fields(
        &mut self,
        updated: &Layer,
        keys: &Map<String, Value>,
    ) -> Result<MutationHandles, Error> {
        let Some(existing) = self
            .drawing
            .layers
            .iter_mut()
            .find(|l| l.name.eq_ignore_ascii_case(&updated.name))
        else {
            return Err(Error::new(
                ErrorCode::UnknownName,
                format!("unknown layer `{}`", updated.name),
            ));
        };
        for key in keys.keys() {
            match key.as_str() {
                "color" => {
                    existing.color = Color::Index(updated.color);
                    existing.color_name = None;
                    existing.book_name = None;
                }
                "linetype" => existing.line_type = updated.linetype.clone(),
                "lineweight" => {
                    existing.line_weight = match updated.lineweight {
                        Some(w) => LineWeight::Value(w.hundredths()),
                        None => LineWeight::Default,
                    };
                }
                "on" => existing.flags.off = !updated.on,
                "frozen" => existing.flags.frozen = updated.frozen,
                "locked" => existing.flags.locked = updated.locked,
                "plot" => existing.is_plottable = updated.plot,
                "description" => existing.description = updated.description.clone(),
                _ => {}
            }
        }
        let handle = existing.handle;
        self.expected_layers_changed
            .insert(updated.name.to_lowercase());
        Ok(MutationHandles {
            touched: vec![handle_text(handle)],
            ..Default::default()
        })
    }

    fn commit_edited_entity(&mut self, handle: Handle, updated: EntityType) -> MutationHandles {
        let mut record = MutationHandles::default();
        self.expected_touched.insert(handle.value());
        record.touched.push(handle_text(handle));
        if let EntityType::Insert(ins) = &updated {
            for att in &ins.attributes {
                if !att.common.handle.is_null() {
                    self.expected_touched.insert(att.common.handle.value());
                    record.touched.push(handle_text(att.common.handle));
                }
            }
        }
        *self
            .drawing
            .get_entity_mut(handle)
            .expect("target verified above") = updated;
        record
    }

    /// Serializes the revised drawing to DWG bytes in the original's exact 6-byte
    /// DWG version header, strictly re-reads those bytes, and diffs every
    /// entity, non-graphical object, symbol table, requested add/delete, and
    /// decode health metric against the pre-edit snapshot.
    pub(crate) fn serialize_and_verify(
        &self,
        revision_path: &Path,
    ) -> Result<(Vec<u8>, Vec<UnexpectedChange>), Error> {
        self.check_writable_version()?;
        let bytes = match catch_unwind(AssertUnwindSafe(|| DwgWriter::write_to_vec(&self.drawing)))
        {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(e)) => {
                return Err(Error::new(
                    ErrorCode::DwgWrite,
                    format!("the DWG library could not write the revision: {e}"),
                ))
            }
            Err(panic) => {
                return Err(Error::new(
                    ErrorCode::DwgWrite,
                    format!(
                        "the DWG library failed while writing the revision: {}",
                        panic_message(panic)
                    ),
                ))
            }
        };
        if let Some(orig) = self.original_header {
            if bytes.get(..6) != Some(&orig[..]) {
                let expected = String::from_utf8_lossy(&orig);
                let got = bytes
                    .get(..6)
                    .map(|b| String::from_utf8_lossy(b).into_owned())
                    .unwrap_or_else(|| "<short>".to_owned());
                return Err(Error::new(
                    ErrorCode::DwgWrite,
                    format!(
                        "expected the revision to keep the exact 6-byte DWG header `{expected}`, \
                         but the DWG library wrote `{got}` (ADR-0006)"
                    ),
                ));
            }
        }
        let written_version =
            raw_dwg_version(&bytes).unwrap_or_else(|| self.drawing.version.to_string());
        let (re_read, re_stats) = read_drawing_bytes_with_stats(&bytes, revision_path, false)?;
        let re_contents = contents(&re_read, &re_stats, Some(written_version.clone()))?;
        let after = snapshot_drawing(&re_read, &written_version, &re_contents.decode);
        let unexpected = self.diff_snapshots(&after);
        Ok((bytes, unexpected))
    }

    fn diff_snapshots(&self, after: &DrawingSnapshot) -> Vec<UnexpectedChange> {
        let mut unexpected = Vec::new();
        let before = &self.snapshot;
        if before.dwg_version != after.dwg_version {
            unexpected.push(UnexpectedChange {
                kind: "HEADER".to_owned(),
                handle: "0".to_owned(),
                change: UnexpectedChangeKind::Changed,
                detail: format!(
                    "DWG version changed from `{}` to `{}`",
                    before.dwg_version, after.dwg_version
                ),
            });
        }
        check_decode_regression(&before.decode, &after.decode, &mut unexpected);

        // 1. Untouched entities must survive with identical owner, raw bytes (when
        //    passthrough-eligible), and field-complete logical digest.
        for (&handle, orig) in &before.objects {
            if self.expected_deleted.contains(&handle) {
                if after.objects.contains_key(&handle) {
                    unexpected.push(UnexpectedChange {
                        kind: orig.kind.clone(),
                        handle: format!("{handle:X}"),
                        change: UnexpectedChangeKind::Added,
                        detail: format!(
                            "deleted {} was still present in the revision after writing",
                            orig.kind
                        ),
                    });
                }
                continue;
            }
            match after.objects.get(&handle) {
                None => unexpected.push(UnexpectedChange {
                    kind: orig.kind.clone(),
                    handle: format!("{handle:X}"),
                    change: UnexpectedChangeKind::Removed,
                    detail: format!("untouched {} was missing from the revision", orig.kind),
                }),
                Some(now) if !self.expected_touched.contains(&handle) => {
                    let owner_changed = orig.owner != now.owner;
                    let raw_changed = match (&orig.raw_bytes, &now.raw_bytes) {
                        (Some(a), Some(b)) => a != b,
                        _ => false,
                    };
                    let logical_changed = orig.kind != now.kind || orig.logical != now.logical;
                    if owner_changed || raw_changed || logical_changed {
                        let reason = if owner_changed {
                            format!("owner changed from {:X} to {:X}", orig.owner, now.owner)
                        } else if logical_changed {
                            format!("before: {}; after: {}", orig.logical, now.logical)
                        } else {
                            "verbatim raw record bytes changed".to_owned()
                        };
                        unexpected.push(UnexpectedChange {
                            kind: orig.kind.clone(),
                            handle: format!("{handle:X}"),
                            change: UnexpectedChangeKind::Changed,
                            detail: format!(
                                "untouched {} changed in the revision ({reason})",
                                orig.kind
                            ),
                        });
                    }
                }
                Some(_) => {}
            }
        }
        for (&handle, now) in &after.objects {
            if !before.objects.contains_key(&handle) && !self.expected_added.contains(&handle) {
                unexpected.push(UnexpectedChange {
                    kind: now.kind.clone(),
                    handle: format!("{handle:X}"),
                    change: UnexpectedChangeKind::Added,
                    detail: format!("unexpected {} appeared in the revision", now.kind),
                });
            }
        }
        // Verify every requested addition actually appeared in the written DWG.
        for &handle in &self.expected_added {
            if self.expected_deleted.contains(&handle) {
                continue;
            }
            if !after.objects.contains_key(&handle) && !after.sub_handles.contains(&handle) {
                unexpected.push(UnexpectedChange {
                    kind: "ADDED_OBJECT".to_owned(),
                    handle: format!("{handle:X}"),
                    change: UnexpectedChangeKind::Removed,
                    detail: "requested added object was missing after writing the revision"
                        .to_owned(),
                });
            }
        }

        // 2. Non-graphical objects (dictionaries, layouts, xrecords, custom objects).
        for (&handle, orig) in &before.non_graphical {
            match after.non_graphical.get(&handle) {
                None => unexpected.push(UnexpectedChange {
                    kind: orig.kind.clone(),
                    handle: format!("{handle:X}"),
                    change: UnexpectedChangeKind::Removed,
                    detail: format!(
                        "untouched non-graphical {} was missing from the revision",
                        orig.kind
                    ),
                }),
                Some(now) if orig != now => unexpected.push(UnexpectedChange {
                    kind: orig.kind.clone(),
                    handle: format!("{handle:X}"),
                    change: UnexpectedChangeKind::Changed,
                    detail: format!(
                        "untouched non-graphical {} changed in the revision",
                        orig.kind
                    ),
                }),
                Some(_) => {}
            }
        }
        for (&handle, now) in &after.non_graphical {
            if !before.non_graphical.contains_key(&handle) {
                unexpected.push(UnexpectedChange {
                    kind: now.kind.clone(),
                    handle: format!("{handle:X}"),
                    change: UnexpectedChangeKind::Added,
                    detail: format!(
                        "unexpected non-graphical {} appeared in the revision",
                        now.kind
                    ),
                });
            }
        }

        // 3. Symbol tables: layers, linetypes, text styles, dim styles, block records.
        for (key, orig) in &before.layers {
            match after.layers.get(key) {
                None => unexpected.push(UnexpectedChange {
                    kind: "LAYER".to_owned(),
                    handle: orig.name.clone(),
                    change: UnexpectedChangeKind::Removed,
                    detail: format!("untouched layer `{}` was removed", orig.name),
                }),
                Some(now) if !self.expected_layers_changed.contains(key) && now != orig => {
                    unexpected.push(UnexpectedChange {
                        kind: "LAYER".to_owned(),
                        handle: orig.name.clone(),
                        change: UnexpectedChangeKind::Changed,
                        detail: format!("untouched layer `{}` changed", orig.name),
                    });
                }
                Some(_) => {}
            }
        }
        for (key, now) in &after.layers {
            if !before.layers.contains_key(key) && !self.expected_layers_added.contains(key) {
                unexpected.push(UnexpectedChange {
                    kind: "LAYER".to_owned(),
                    handle: now.name.clone(),
                    change: UnexpectedChangeKind::Added,
                    detail: format!("unexpected layer `{}` appeared in the revision", now.name),
                });
            }
        }
        for key in &self.expected_layers_added {
            if !after.layers.contains_key(key) {
                unexpected.push(UnexpectedChange {
                    kind: "LAYER".to_owned(),
                    handle: key.clone(),
                    change: UnexpectedChangeKind::Removed,
                    detail: format!("requested added layer `{key}` was missing after writing"),
                });
            }
        }
        diff_named_table(
            "LTYPE",
            &before.line_types,
            &after.line_types,
            &mut unexpected,
        );
        diff_named_table(
            "STYLE",
            &before.text_styles,
            &after.text_styles,
            &mut unexpected,
        );
        diff_named_table(
            "DIMSTYLE",
            &before.dim_styles,
            &after.dim_styles,
            &mut unexpected,
        );
        for (key, orig) in &before.block_records {
            match after.block_records.get(key) {
                None => unexpected.push(UnexpectedChange {
                    kind: "BLOCK_RECORD".to_owned(),
                    handle: orig.name.clone(),
                    change: UnexpectedChangeKind::Removed,
                    detail: format!("untouched block record `{}` was removed", orig.name),
                }),
                Some(now) => {
                    let meta_changed = orig.handle != now.handle
                        || orig.flags != now.flags
                        || orig.layout_handle != now.layout_handle;
                    let members_changed = !self.expected_spaces_mutated.contains(&orig.handle)
                        && orig.entity_handles != now.entity_handles;
                    if meta_changed || members_changed {
                        unexpected.push(UnexpectedChange {
                            kind: "BLOCK_RECORD".to_owned(),
                            handle: orig.name.clone(),
                            change: UnexpectedChangeKind::Changed,
                            detail: format!(
                                "untouched block record `{}` changed in the revision",
                                orig.name
                            ),
                        });
                    }
                }
            }
        }
        for (key, now) in &after.block_records {
            if !before.block_records.contains_key(key) {
                unexpected.push(UnexpectedChange {
                    kind: "BLOCK_RECORD".to_owned(),
                    handle: now.name.clone(),
                    change: UnexpectedChangeKind::Added,
                    detail: format!(
                        "unexpected block record `{}` appeared in the revision",
                        now.name
                    ),
                });
            }
        }
        unexpected
    }
}

/// Parses a hexadecimal handle string such as `"2F"` into a [`Handle`].
fn parse_hex_handle(hex: &str) -> Result<Handle, Error> {
    let val = u64::from_str_radix(hex.trim(), 16).map_err(|_| {
        Error::new(
            ErrorCode::BadValue,
            format!("`{hex}` is not a valid hexadecimal handle"),
        )
    })?;
    Ok(Handle::new(val))
}

/// Clears `raw_record` on an edited object (and on its attributes if an Insert)
/// so acadrust re-encodes the edited fields.
fn clear_raw_record(object: &mut EntityType) {
    object.common_mut().raw_record = None;
    if let EntityType::Insert(ins) = object {
        for att in &mut ins.attributes {
            att.common.raw_record = None;
        }
    }
}

/// Resets handle, raw_record, extension dictionary, reactors, carried
/// AgentDrafter ID/name (both structured and raw DWG EED), and owned sub-entity
/// handles on a cloned object before adding it as a new copy.
fn sanitize_clone(object: &mut EntityType, app_handle: Option<u64>) {
    let common = object.common_mut();
    common.handle = Handle::NULL;
    common.owner_handle = Handle::NULL;
    common.raw_record = None;
    common.xdictionary_handle = None;
    common.reactors.clear();
    // Strip AgentDrafter ID/name xdata (structured AND raw_dwg_eed) so copies
    // never duplicate a name or ID.
    ids::strip_agentdrafter_xdata(common, app_handle);
    match object {
        EntityType::Insert(ins) => {
            ins.seqend_handle = None;
            for att in &mut ins.attributes {
                att.common.handle = Handle::NULL;
                att.common.owner_handle = Handle::NULL;
                att.common.raw_record = None;
                att.common.xdictionary_handle = None;
                att.common.reactors.clear();
                ids::strip_agentdrafter_xdata(&mut att.common, app_handle);
            }
        }
        EntityType::Polyline3D(p3) => {
            for v in &mut p3.vertices {
                v.handle = Handle::NULL;
            }
        }
        _ => {}
    }
}

/// Applies one type-specific key in a `change` operation's `set` map directly
/// to `object`.
fn apply_type_change(
    object: &mut EntityType,
    key: &str,
    value: &Value,
    units: Unit,
    drawing: &CadDocument,
    start_of: &dyn Fn(&TextObject) -> Point3,
) -> Result<(), Error> {
    let field = format!("set.{key}");
    match object {
        EntityType::Line(line) => match key {
            "from" => line.start = vector(parse_point_json(value, units, &field)?),
            "to" => line.end = vector(parse_point_json(value, units, &field)?),
            _ => return Err(unknown_change_key("line", key)),
        },
        EntityType::Circle(circle) => match key {
            "center" => circle.center = vector(parse_point_json(value, units, &field)?),
            "radius" => {
                let r = parse_length_json(value, units, &field)?;
                if !r.is_finite() || r <= 0.0 {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        "a circle's radius must be greater than 0",
                    )
                    .at_field(field));
                }
                circle.radius = r;
            }
            _ => return Err(unknown_change_key("circle", key)),
        },
        EntityType::Arc(arc) => match key {
            "center" => arc.center = vector(parse_point_json(value, units, &field)?),
            "radius" => {
                let r = parse_length_json(value, units, &field)?;
                if !r.is_finite() || r <= 0.0 {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        "an arc's radius must be greater than 0",
                    )
                    .at_field(field));
                }
                arc.radius = r;
            }
            "start_angle" => arc.start_angle = parse_angle_json(value, &field)?.to_radians(),
            "end_angle" => arc.end_angle = parse_angle_json(value, &field)?.to_radians(),
            _ => return Err(unknown_change_key("arc", key)),
        },
        EntityType::Ellipse(ellipse) => match key {
            "center" => ellipse.center = vector(parse_point_json(value, units, &field)?),
            "major_axis" => {
                let axis = vector(parse_point_json(value, units, &field)?);
                if axis.x == 0.0 && axis.y == 0.0 && axis.z == 0.0 {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        "an ellipse's major_axis cannot be zero",
                    )
                    .at_field(field));
                }
                ellipse.major_axis = axis;
            }
            "ratio" => {
                let ratio = parse_plain_number_json(value, &field, "a ratio in (0, 1]")?;
                if !(ratio > 0.0 && ratio <= 1.0) {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        "an ellipse's ratio must be greater than 0 and at most 1",
                    )
                    .at_field(field));
                }
                ellipse.minor_axis_ratio = ratio;
            }
            "start_angle" => {
                let deg = parse_angle_json(value, &field)?;
                ellipse.start_parameter = super::parameter(deg, ellipse.minor_axis_ratio);
            }
            "end_angle" => {
                let deg = parse_angle_json(value, &field)?;
                let p = super::parameter(deg, ellipse.minor_axis_ratio);
                ellipse.end_parameter = if p > ellipse.start_parameter {
                    p
                } else {
                    p + TAU
                };
            }
            _ => return Err(unknown_change_key("ellipse", key)),
        },
        EntityType::LwPolyline(poly) => match key {
            "closed" => {
                let Some(b) = value.as_bool() else {
                    return Err(
                        Error::new(ErrorCode::WrongType, "expected true or false").at_field(field)
                    );
                };
                poly.is_closed = b;
            }
            "width" => {
                let w = if value.is_null() {
                    0.0
                } else {
                    parse_length_json(value, units, &field)?
                };
                if w < 0.0 {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        "a polyline's width cannot be negative",
                    )
                    .at_field(field));
                }
                poly.constant_width = w;
            }
            "points" => {
                let Some(arr) = value.as_array() else {
                    return Err(
                        Error::new(ErrorCode::WrongType, "expected a list of points")
                            .at_field(field),
                    );
                };
                if arr.len() < 2 {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        "a polyline needs at least 2 points",
                    )
                    .at_field(field));
                }
                let mut pts = Vec::with_capacity(arr.len());
                for (i, item) in arr.iter().enumerate() {
                    pts.push(parse_point_json(item, units, &format!("{field}[{i}]"))?);
                }
                let elev = pts[0].z;
                poly.elevation = elev;
                poly.vertices = pts
                    .into_iter()
                    .map(|p| LwVertex::new(Vector2::new(p.x, p.y)))
                    .collect();
            }
            _ => return Err(unknown_change_key("polyline", key)),
        },
        EntityType::Polyline2D(poly) => match key {
            "closed" => {
                let Some(b) = value.as_bool() else {
                    return Err(
                        Error::new(ErrorCode::WrongType, "expected true or false").at_field(field)
                    );
                };
                poly.flags.set_closed(b);
            }
            "points" => {
                let Some(arr) = value.as_array() else {
                    return Err(
                        Error::new(ErrorCode::WrongType, "expected a list of points")
                            .at_field(field),
                    );
                };
                if arr.len() < 2 {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        "a polyline needs at least 2 points",
                    )
                    .at_field(field));
                }
                let mut pts = Vec::with_capacity(arr.len());
                for (i, item) in arr.iter().enumerate() {
                    pts.push(vector(parse_point_json(
                        item,
                        units,
                        &format!("{field}[{i}]"),
                    )?));
                }
                // Mutate vertices in place, preserving entity common/handle/layer/normal.
                poly.elevation = pts[0].z;
                for (i, pt) in pts.iter().enumerate() {
                    if let Some(existing) = poly.vertices.get_mut(i) {
                        existing.location = *pt;
                    } else {
                        poly.vertices.push(Vertex2D::new(*pt));
                    }
                }
                poly.vertices.truncate(pts.len());
            }
            _ => return Err(unknown_change_key("polyline", key)),
        },
        EntityType::Polyline3D(poly) => match key {
            "closed" => {
                let Some(b) = value.as_bool() else {
                    return Err(
                        Error::new(ErrorCode::WrongType, "expected true or false").at_field(field)
                    );
                };
                poly.flags.closed = b;
            }
            "points" => {
                let Some(arr) = value.as_array() else {
                    return Err(
                        Error::new(ErrorCode::WrongType, "expected a list of points")
                            .at_field(field),
                    );
                };
                if arr.len() < 2 {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        "a polyline needs at least 2 points",
                    )
                    .at_field(field));
                }
                let mut pts = Vec::with_capacity(arr.len());
                for (i, item) in arr.iter().enumerate() {
                    pts.push(vector(parse_point_json(
                        item,
                        units,
                        &format!("{field}[{i}]"),
                    )?));
                }
                // Mutate vertices in place, preserving handle, owner, layer,
                // flags, xdata, and existing vertex handles. New vertices start
                // with `Handle::NULL` and receive fresh handles before commit.
                let parent_layer = poly.common.layer.clone();
                for (i, pt) in pts.iter().enumerate() {
                    if let Some(existing) = poly.vertices.get_mut(i) {
                        existing.position = *pt;
                    } else {
                        let mut v = Vertex3DPolyline::new(*pt);
                        v.layer = parent_layer.clone();
                        poly.vertices.push(v);
                    }
                }
                poly.vertices.truncate(pts.len());
            }
            _ => return Err(unknown_change_key("polyline", key)),
        },
        EntityType::Solid(solid) => match key {
            "corners" => {
                let Some(arr) = value.as_array() else {
                    return Err(
                        Error::new(ErrorCode::WrongType, "expected 3 or 4 corner points")
                            .at_field(field),
                    );
                };
                if !(3..=4).contains(&arr.len()) {
                    return Err(
                        Error::new(ErrorCode::BadValue, "a solid needs 3 or 4 corners")
                            .at_field(field),
                    );
                }
                let mut pts = Vec::with_capacity(arr.len());
                for (i, item) in arr.iter().enumerate() {
                    pts.push(vector(parse_point_json(
                        item,
                        units,
                        &format!("{field}[{i}]"),
                    )?));
                }
                solid.first_corner = pts[0];
                solid.second_corner = pts[1];
                if pts.len() == 3 {
                    solid.third_corner = pts[2];
                    solid.fourth_corner = pts[2];
                } else {
                    solid.third_corner = pts[3];
                    solid.fourth_corner = pts[2];
                }
            }
            _ => return Err(unknown_change_key("solid", key)),
        },
        EntityType::Text(dwg_text) => {
            apply_text_change(dwg_text, key, value, units, drawing, start_of)?
        }
        EntityType::MText(dwg_mtext) => apply_mtext_change(dwg_mtext, key, value, units, drawing)?,
        EntityType::Insert(ins) => apply_insert_change(ins, key, value, units)?,
        other => {
            return Err(Error::new(
                ErrorCode::BadValue,
                format!(
                    "changing `{key}` is not supported on {} in an original DWG",
                    kind_of(other)
                ),
            )
            .at_field(field))
        }
    }
    Ok(())
}

fn apply_text_change(
    dwg_text: &mut DwgText,
    key: &str,
    value: &Value,
    units: Unit,
    drawing: &CadDocument,
    start_of: &dyn Fn(&TextObject) -> Point3,
) -> Result<(), Error> {
    let field = format!("set.{key}");
    match key {
        "contents" | "text" => {
            let Some(s) = value.as_str() else {
                return Err(Error::new(ErrorCode::WrongType, "expected text").at_field(field));
            };
            if s.is_empty() || s.contains('\n') || s.contains('\r') {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    "single-line text contents must be one non-empty line",
                )
                .at_field(field));
            }
            dwg_text.value = text::text_code(s);
        }
        "position" => {
            let p = vector(parse_point_json(value, units, &field)?);
            if dwg_text.horizontal_alignment == TextHorizontalAlignment::Left
                && dwg_text.vertical_alignment == TextVerticalAlignment::Baseline
            {
                dwg_text.insertion_point = p;
            } else {
                let delta = p - dwg_text.alignment_point.unwrap_or(dwg_text.insertion_point);
                dwg_text.alignment_point = Some(p);
                dwg_text.insertion_point = dwg_text.insertion_point + delta;
            }
        }
        "height" => {
            let h = parse_length_json(value, units, &field)?;
            if !h.is_finite() || h <= 0.0 {
                return Err(
                    Error::new(ErrorCode::BadValue, "text height must be greater than 0")
                        .at_field(field),
                );
            }
            dwg_text.height = h;
        }
        "rotation" => {
            dwg_text.rotation = parse_angle_json(value, &field)?.to_radians();
        }
        "width_factor" => {
            let wf = parse_plain_number_json(value, &field, "a width factor")?;
            if !(WIDTH_FACTORS.0..=WIDTH_FACTORS.1).contains(&wf) {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    format!(
                        "width_factor must be between {} and {}",
                        WIDTH_FACTORS.0, WIDTH_FACTORS.1
                    ),
                )
                .at_field(field));
            }
            dwg_text.width_factor = wf;
        }
        "style" => {
            let style = if value.is_null() {
                STANDARD.to_owned()
            } else {
                let Some(name) = value.as_str() else {
                    return Err(
                        Error::new(ErrorCode::WrongType, "expected a text style name")
                            .at_field(field),
                    );
                };
                resolve_style_name(drawing, name, &field)?
            };
            dwg_text.style = style;
        }
        "justify" => {
            let justify = parse_justify_json(value, &field)?;
            let pos = if dwg_text.horizontal_alignment == TextHorizontalAlignment::Left
                && dwg_text.vertical_alignment == TextVerticalAlignment::Baseline
            {
                dwg_text.insertion_point
            } else {
                dwg_text.alignment_point.unwrap_or(dwg_text.insertion_point)
            };
            let (h, v) = text_alignment(justify);
            dwg_text.horizontal_alignment = h;
            dwg_text.vertical_alignment = v;
            if justify == Justify::Left {
                dwg_text.insertion_point = pos;
                dwg_text.alignment_point = None;
            } else {
                dwg_text.alignment_point = Some(pos);
            }
        }
        _ => return Err(unknown_change_key("text", key)),
    }
    // Recompute baseline start point for justified text when modeled.
    if let Some(modeled) = text::read_text(dwg_text, layer_of(&dwg_text.common)) {
        if modeled.justify != Justify::Left {
            dwg_text.insertion_point = vector(start_of(&modeled));
        }
    }
    Ok(())
}

fn apply_mtext_change(
    dwg_mtext: &mut DwgMText,
    key: &str,
    value: &Value,
    units: Unit,
    drawing: &CadDocument,
) -> Result<(), Error> {
    let field = format!("set.{key}");
    match key {
        "contents" | "text" => {
            let Some(s) = value.as_str() else {
                return Err(Error::new(ErrorCode::WrongType, "expected text").at_field(field));
            };
            if s.is_empty() {
                return Err(
                    Error::new(ErrorCode::BadValue, "mtext contents cannot be empty")
                        .at_field(field),
                );
            }
            let existing_wf = text::mtext_contents(&dwg_mtext.value).and_then(|(_, wf)| wf);
            dwg_mtext.value = text::mtext_code(s, existing_wf);
        }
        "position" => {
            dwg_mtext.insertion_point = vector(parse_point_json(value, units, &field)?);
        }
        "height" => {
            let h = parse_length_json(value, units, &field)?;
            if !h.is_finite() || h <= 0.0 {
                return Err(
                    Error::new(ErrorCode::BadValue, "mtext height must be greater than 0")
                        .at_field(field),
                );
            }
            dwg_mtext.height = h;
        }
        "width" => {
            let w = if value.is_null() {
                0.0
            } else {
                parse_length_json(value, units, &field)?
            };
            if w < 0.0 {
                return Err(
                    Error::new(ErrorCode::BadValue, "mtext width cannot be negative")
                        .at_field(field),
                );
            }
            dwg_mtext.rectangle_width = w;
        }
        "rotation" => {
            dwg_mtext.rotation = parse_angle_json(value, &field)?.to_radians();
            dwg_mtext.dwg_x_direction = None;
        }
        "width_factor" => {
            let wf = if value.is_null() {
                None
            } else {
                let n = parse_plain_number_json(value, &field, "a width factor")?;
                if !(WIDTH_FACTORS.0..=WIDTH_FACTORS.1).contains(&n) {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        format!(
                            "width_factor must be between {} and {}",
                            WIDTH_FACTORS.0, WIDTH_FACTORS.1
                        ),
                    )
                    .at_field(field));
                }
                Some(n)
            };
            if let Some((contents, _)) = text::mtext_contents(&dwg_mtext.value) {
                dwg_mtext.value = text::mtext_code(&contents, wf);
            }
        }
        "style" => {
            let style = if value.is_null() {
                STANDARD.to_owned()
            } else {
                let Some(name) = value.as_str() else {
                    return Err(
                        Error::new(ErrorCode::WrongType, "expected a text style name")
                            .at_field(field),
                    );
                };
                resolve_style_name(drawing, name, &field)?
            };
            dwg_mtext.style = style;
        }
        "justify" => {
            let att = parse_attachment_json(value, &field)?;
            dwg_mtext.attachment_point = mtext_attachment(att);
        }
        _ => return Err(unknown_change_key("mtext", key)),
    }
    Ok(())
}

fn apply_insert_change(
    ins: &mut DwgInsert,
    key: &str,
    value: &Value,
    units: Unit,
) -> Result<(), Error> {
    let field = format!("set.{key}");
    match key {
        "position" => {
            let next = vector(parse_point_json(value, units, &field)?);
            let delta = next - ins.insert_point;
            ins.insert_point = next;
            for att in &mut ins.attributes {
                translate_attribute(att, delta);
            }
        }
        "rotation" => {
            let next_deg = parse_angle_json(value, &field)?;
            let old_deg = normalize_degrees(ins.rotation.to_degrees());
            let delta_deg = next_deg - old_deg;
            let about = [ins.insert_point.x, ins.insert_point.y, ins.insert_point.z];
            ins.rotation = next_deg.to_radians();
            for att in &mut ins.attributes {
                rotate_attribute(att, about, delta_deg);
            }
        }
        "scale" => {
            let next_scale = parse_plain_number_json(value, &field, "a positive scale factor")?;
            if !next_scale.is_finite() || next_scale <= 0.0 {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    "an insert's scale must be greater than 0",
                )
                .at_field(field));
            }
            let old_scale = ins.y_scale().abs().max(1e-12);
            let factor = next_scale / old_scale;
            let about = [ins.insert_point.x, ins.insert_point.y, ins.insert_point.z];
            let x_sign = if ins.x_scale() < 0.0 { -1.0 } else { 1.0 };
            ins.set_scale_unchecked(x_sign * next_scale, next_scale, next_scale);
            for att in &mut ins.attributes {
                scale_attribute(att, about, factor);
            }
        }
        _ => return Err(unknown_change_key("insert", key)),
    }
    Ok(())
}

fn resolve_style_name(drawing: &CadDocument, name: &str, field: &str) -> Result<String, Error> {
    if let Some(found) = drawing
        .text_styles
        .iter()
        .find(|s| !s.is_shape_file && s.name.eq_ignore_ascii_case(name))
    {
        Ok(found.name.clone())
    } else {
        let choices: Vec<&str> = drawing
            .text_styles
            .iter()
            .filter(|s| !s.is_shape_file)
            .map(|s| s.name.as_str())
            .collect();
        Err(Error::new(
            ErrorCode::UnknownName,
            format!(
                "unknown text style `{name}`; the drawing's text styles: {}",
                choices.join(", ")
            ),
        )
        .at_field(field))
    }
}

fn parse_justify_json(value: &Value, field: &str) -> Result<Justify, Error> {
    if value.is_null() {
        return Ok(Justify::Left);
    }
    let Some(s) = value.as_str() else {
        return Err(
            Error::new(ErrorCode::WrongType, "expected a justification name").at_field(field),
        );
    };
    Justify::ALL
        .into_iter()
        .find(|j| j.name() == s)
        .ok_or_else(|| {
            Error::new(ErrorCode::BadValue, format!("unknown justification `{s}`")).at_field(field)
        })
}

fn parse_attachment_json(value: &Value, field: &str) -> Result<Attachment, Error> {
    if value.is_null() {
        return Ok(Attachment::TopLeft);
    }
    let Some(s) = value.as_str() else {
        return Err(
            Error::new(ErrorCode::WrongType, "expected an attachment name").at_field(field),
        );
    };
    Attachment::ALL
        .into_iter()
        .find(|a| a.name() == s)
        .ok_or_else(|| {
            Error::new(ErrorCode::BadValue, format!("unknown attachment `{s}`")).at_field(field)
        })
}

fn text_alignment(j: Justify) -> (TextHorizontalAlignment, TextVerticalAlignment) {
    match j {
        Justify::Left => (
            TextHorizontalAlignment::Left,
            TextVerticalAlignment::Baseline,
        ),
        Justify::Center => (
            TextHorizontalAlignment::Center,
            TextVerticalAlignment::Baseline,
        ),
        Justify::Right => (
            TextHorizontalAlignment::Right,
            TextVerticalAlignment::Baseline,
        ),
        Justify::Middle => (
            TextHorizontalAlignment::Middle,
            TextVerticalAlignment::Baseline,
        ),
        Justify::TopLeft => (TextHorizontalAlignment::Left, TextVerticalAlignment::Top),
        Justify::TopCenter => (TextHorizontalAlignment::Center, TextVerticalAlignment::Top),
        Justify::TopRight => (TextHorizontalAlignment::Right, TextVerticalAlignment::Top),
        Justify::MiddleLeft => (TextHorizontalAlignment::Left, TextVerticalAlignment::Middle),
        Justify::MiddleCenter => (
            TextHorizontalAlignment::Center,
            TextVerticalAlignment::Middle,
        ),
        Justify::MiddleRight => (
            TextHorizontalAlignment::Right,
            TextVerticalAlignment::Middle,
        ),
        Justify::BottomLeft => (TextHorizontalAlignment::Left, TextVerticalAlignment::Bottom),
        Justify::BottomCenter => (
            TextHorizontalAlignment::Center,
            TextVerticalAlignment::Bottom,
        ),
        Justify::BottomRight => (
            TextHorizontalAlignment::Right,
            TextVerticalAlignment::Bottom,
        ),
    }
}

fn mtext_attachment(a: Attachment) -> acadrust::entities::AttachmentPoint {
    use acadrust::entities::AttachmentPoint as P;
    match a {
        Attachment::TopLeft => P::TopLeft,
        Attachment::TopCenter => P::TopCenter,
        Attachment::TopRight => P::TopRight,
        Attachment::MiddleLeft => P::MiddleLeft,
        Attachment::MiddleCenter => P::MiddleCenter,
        Attachment::MiddleRight => P::MiddleRight,
        Attachment::BottomLeft => P::BottomLeft,
        Attachment::BottomCenter => P::BottomCenter,
        Attachment::BottomRight => P::BottomRight,
    }
}

fn unknown_change_key(kind: &str, key: &str) -> Error {
    Error::new(
        ErrorCode::UnknownKey,
        format!("unknown or unsupported key `{key}` for changing a {kind} in an original DWG"),
    )
    .at_field(format!("set.{key}"))
}

// ── Direct transforms on acadrust EntityType ──────────────────────────────

/// True when `normal` is the standard +Z extrusion vector `(0, 0, 1)`, where
/// OCS coincides with WCS.
fn is_standard_z(normal: Vector3) -> bool {
    normal.x.abs() < 1e-9 && normal.y.abs() < 1e-9 && (normal.z - 1.0).abs() < 1e-9
}

/// Returns `true` if `object` is an OCS entity whose stored coordinates use a
/// non-standard extrusion normal (not `+Z`).
fn has_non_standard_normal(object: &EntityType) -> bool {
    let normal = match object {
        EntityType::Circle(e) => e.normal,
        EntityType::Arc(e) => e.normal,
        EntityType::Ellipse(e) => e.normal,
        EntityType::LwPolyline(e) => e.normal,
        EntityType::Polyline2D(e) => e.normal,
        EntityType::Solid(e) => e.normal,
        EntityType::Text(e) => e.normal,
        EntityType::MText(e) => e.normal,
        EntityType::Insert(e) => {
            if !is_standard_z(e.normal) {
                return true;
            }
            return e.attributes.iter().any(|a| !is_standard_z(a.normal));
        }
        EntityType::Hatch(e) => e.normal,
        EntityType::Point(e) => e.normal,
        _ => return false,
    };
    !is_standard_z(normal)
}

fn translate_entity(object: &mut EntityType, delta: Vector3) -> Result<(), Error> {
    match object {
        EntityType::Line(_)
        | EntityType::Circle(_)
        | EntityType::Arc(_)
        | EntityType::Ellipse(_)
        | EntityType::LwPolyline(_)
        | EntityType::Polyline2D(_)
        | EntityType::Polyline3D(_)
        | EntityType::Spline(_)
        | EntityType::Solid(_)
        | EntityType::Face3D(_)
        | EntityType::Point(_)
        | EntityType::Ray(_)
        | EntityType::XLine(_)
        | EntityType::Text(_)
        | EntityType::MText(_)
        | EntityType::Insert(_)
        | EntityType::Hatch(_) => {
            // acadrust's `Entity::translate` applies WCS->OCS arbitrary-axis
            // projection for all OCS entities (handling -Z and tilted normals
            // accurately) and translates world-space XDATA along with the entity.
            object.translate(delta);
            Ok(())
        }
        other => Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "move is not supported on {} in an original DWG",
                kind_of(other)
            ),
        )),
    }
}

fn translate_attribute(att: &mut AttributeEntity, delta: Vector3) {
    att.insertion_point = att.insertion_point + delta;
    if att.horizontal_alignment != HorizontalAlignment::Left
        || att.vertical_alignment != VerticalAlignment::Baseline
    {
        att.alignment_point = att.alignment_point + delta;
    }
    if let Some(m) = att.embedded_mtext.as_mut() {
        m.insertion_point = m.insertion_point + delta;
    }
}

fn rot_pt(p: Vector3, about: [f64; 3], sin: f64, cos: f64) -> Vector3 {
    let dx = p.x - about[0];
    let dy = p.y - about[1];
    Vector3::new(
        about[0] + dx * cos - dy * sin,
        about[1] + dx * sin + dy * cos,
        p.z,
    )
}

fn rot_v2(v: Vector2, about: [f64; 3], sin: f64, cos: f64) -> Vector2 {
    let dx = v.x - about[0];
    let dy = v.y - about[1];
    Vector2::new(
        about[0] + dx * cos - dy * sin,
        about[1] + dx * sin + dy * cos,
    )
}

fn rot_vec3(v: Vector3, sin: f64, cos: f64) -> Vector3 {
    Vector3::new(v.x * cos - v.y * sin, v.x * sin + v.y * cos, v.z)
}

fn rot_vec2(v: Vector2, sin: f64, cos: f64) -> Vector2 {
    Vector2::new(v.x * cos - v.y * sin, v.x * sin + v.y * cos)
}

fn rotate_entity(object: &mut EntityType, about: [f64; 3], degrees: f64) -> Result<(), Error> {
    if !matches!(
        object,
        EntityType::Line(_)
            | EntityType::Circle(_)
            | EntityType::Arc(_)
            | EntityType::Ellipse(_)
            | EntityType::LwPolyline(_)
            | EntityType::Polyline2D(_)
            | EntityType::Polyline3D(_)
            | EntityType::Spline(_)
            | EntityType::Solid(_)
            | EntityType::Text(_)
            | EntityType::MText(_)
            | EntityType::Insert(_)
            | EntityType::Hatch(_)
    ) {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "rotate is not supported on {} in an original DWG",
                kind_of(object)
            ),
        ));
    }
    if has_non_standard_normal(object) {
        let tf = DwgTransform::from_rotation_with_origin(
            Vector3::UNIT_Z,
            degrees.to_radians(),
            Vector3::new(about[0], about[1], about[2]),
        );
        object.apply_transform(&tf);
        return Ok(());
    }
    let (sin, cos) = sin_cos_degrees(degrees);
    let rad = degrees.to_radians();
    match object {
        EntityType::Line(e) => {
            e.start = rot_pt(e.start, about, sin, cos);
            e.end = rot_pt(e.end, about, sin, cos);
        }
        EntityType::Circle(e) => {
            e.center = rot_pt(e.center, about, sin, cos);
        }
        EntityType::Arc(e) => {
            e.center = rot_pt(e.center, about, sin, cos);
            e.start_angle = normalize_degrees(e.start_angle.to_degrees() + degrees).to_radians();
            e.end_angle = normalize_degrees(e.end_angle.to_degrees() + degrees).to_radians();
        }
        EntityType::Ellipse(e) => {
            e.center = rot_pt(e.center, about, sin, cos);
            e.major_axis = rot_vec3(e.major_axis, sin, cos);
        }
        EntityType::LwPolyline(e) => {
            for v in &mut e.vertices {
                v.location = rot_v2(v.location, about, sin, cos);
            }
        }
        EntityType::Polyline2D(e) => {
            for v in &mut e.vertices {
                v.location = rot_pt(v.location, about, sin, cos);
            }
        }
        EntityType::Polyline3D(e) => {
            for v in &mut e.vertices {
                v.position = rot_pt(v.position, about, sin, cos);
            }
        }
        EntityType::Spline(e) => {
            for p in &mut e.control_points {
                *p = rot_pt(*p, about, sin, cos);
            }
            for p in &mut e.fit_points {
                *p = rot_pt(*p, about, sin, cos);
            }
            e.begin_tangent = rot_vec3(e.begin_tangent, sin, cos);
            e.end_tangent = rot_vec3(e.end_tangent, sin, cos);
        }
        EntityType::Solid(e) => {
            e.first_corner = rot_pt(e.first_corner, about, sin, cos);
            e.second_corner = rot_pt(e.second_corner, about, sin, cos);
            e.third_corner = rot_pt(e.third_corner, about, sin, cos);
            e.fourth_corner = rot_pt(e.fourth_corner, about, sin, cos);
        }
        EntityType::Text(e) => {
            e.insertion_point = rot_pt(e.insertion_point, about, sin, cos);
            if let Some(a) = &mut e.alignment_point {
                *a = rot_pt(*a, about, sin, cos);
            }
            e.rotation = normalize_degrees(e.rotation.to_degrees() + degrees).to_radians();
        }
        EntityType::MText(e) => {
            e.insertion_point = rot_pt(e.insertion_point, about, sin, cos);
            e.rotation = normalize_degrees(e.rotation.to_degrees() + degrees).to_radians();
            e.dwg_x_direction = None;
        }
        EntityType::Insert(e) => {
            e.insert_point = rot_pt(e.insert_point, about, sin, cos);
            e.rotation = normalize_degrees(e.rotation.to_degrees() + degrees).to_radians();
            for att in &mut e.attributes {
                rotate_attribute(att, about, degrees);
            }
        }
        EntityType::Hatch(e) => {
            e.pattern_angle =
                normalize_degrees(e.pattern_angle.to_degrees() + degrees).to_radians();
            for p in &mut e.seed_points {
                *p = rot_v2(*p, about, sin, cos);
            }
            for line in &mut e.pattern.lines {
                line.base_point = rot_v2(line.base_point, about, sin, cos);
                line.offset = rot_vec2(line.offset, sin, cos);
                line.angle += rad;
            }
            for path in &mut e.paths {
                for edge in &mut path.edges {
                    match edge {
                        BoundaryEdge::Line(l) => {
                            l.start = rot_v2(l.start, about, sin, cos);
                            l.end = rot_v2(l.end, about, sin, cos);
                        }
                        BoundaryEdge::CircularArc(a) => {
                            a.center = rot_v2(a.center, about, sin, cos);
                            a.start_angle = normalize_degrees(a.start_angle.to_degrees() + degrees)
                                .to_radians();
                            a.end_angle =
                                normalize_degrees(a.end_angle.to_degrees() + degrees).to_radians();
                        }
                        BoundaryEdge::EllipticArc(ea) => {
                            ea.center = rot_v2(ea.center, about, sin, cos);
                            ea.major_axis_endpoint = rot_vec2(ea.major_axis_endpoint, sin, cos);
                        }
                        BoundaryEdge::Spline(s) => {
                            for cp in &mut s.control_points {
                                let p = rot_v2(Vector2::new(cp.x, cp.y), about, sin, cos);
                                cp.x = p.x;
                                cp.y = p.y;
                            }
                            for fp in &mut s.fit_points {
                                *fp = rot_v2(*fp, about, sin, cos);
                            }
                        }
                        BoundaryEdge::Polyline(p) => {
                            for v in &mut p.vertices {
                                let pt = rot_v2(Vector2::new(v.x, v.y), about, sin, cos);
                                v.x = pt.x;
                                v.y = pt.y;
                            }
                        }
                    }
                }
            }
        }
        other => {
            return Err(Error::new(
                ErrorCode::BadValue,
                format!(
                    "rotate is not supported on {} in an original DWG",
                    kind_of(other)
                ),
            ))
        }
    }
    Ok(())
}

fn rotate_attribute(att: &mut AttributeEntity, about: [f64; 3], degrees: f64) {
    let (sin, cos) = sin_cos_degrees(degrees);
    att.insertion_point = rot_pt(att.insertion_point, about, sin, cos);
    if att.horizontal_alignment != HorizontalAlignment::Left
        || att.vertical_alignment != VerticalAlignment::Baseline
    {
        att.alignment_point = rot_pt(att.alignment_point, about, sin, cos);
    }
    att.rotation = normalize_degrees(att.rotation.to_degrees() + degrees).to_radians();
    if let Some(m) = att.embedded_mtext.as_mut() {
        m.insertion_point = rot_pt(m.insertion_point, about, sin, cos);
        m.rotation = normalize_degrees(m.rotation.to_degrees() + degrees).to_radians();
        m.dwg_x_direction = None;
    }
}

fn scale_pt(p: Vector3, about: [f64; 3], factor: f64) -> Vector3 {
    Vector3::new(
        about[0] + (p.x - about[0]) * factor,
        about[1] + (p.y - about[1]) * factor,
        about[2] + (p.z - about[2]) * factor,
    )
}

fn scale_v2(v: Vector2, about: [f64; 3], factor: f64) -> Vector2 {
    Vector2::new(
        about[0] + (v.x - about[0]) * factor,
        about[1] + (v.y - about[1]) * factor,
    )
}

fn scale_entity(object: &mut EntityType, about: [f64; 3], factor: f64) -> Result<(), Error> {
    if !matches!(
        object,
        EntityType::Line(_)
            | EntityType::Circle(_)
            | EntityType::Arc(_)
            | EntityType::Ellipse(_)
            | EntityType::LwPolyline(_)
            | EntityType::Polyline2D(_)
            | EntityType::Polyline3D(_)
            | EntityType::Spline(_)
            | EntityType::Solid(_)
            | EntityType::Text(_)
            | EntityType::MText(_)
            | EntityType::Insert(_)
            | EntityType::Hatch(_)
    ) {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "scale is not supported on {} in an original DWG",
                kind_of(object)
            ),
        ));
    }
    if has_non_standard_normal(object) {
        let tf = DwgTransform::from_scaling_with_origin(
            Vector3::new(factor, factor, factor),
            Vector3::new(about[0], about[1], about[2]),
        );
        object.apply_transform(&tf);
        return Ok(());
    }
    match object {
        EntityType::Line(e) => {
            e.start = scale_pt(e.start, about, factor);
            e.end = scale_pt(e.end, about, factor);
        }
        EntityType::Circle(e) => {
            e.center = scale_pt(e.center, about, factor);
            e.radius *= factor;
        }
        EntityType::Arc(e) => {
            e.center = scale_pt(e.center, about, factor);
            e.radius *= factor;
        }
        EntityType::Ellipse(e) => {
            e.center = scale_pt(e.center, about, factor);
            e.major_axis = e.major_axis * factor;
        }
        EntityType::LwPolyline(e) => {
            for v in &mut e.vertices {
                v.location = scale_v2(v.location, about, factor);
                v.start_width *= factor;
                v.end_width *= factor;
            }
            e.constant_width *= factor;
            e.elevation = about[2] + (e.elevation - about[2]) * factor;
        }
        EntityType::Polyline2D(e) => {
            for v in &mut e.vertices {
                v.location = scale_pt(v.location, about, factor);
            }
            e.start_width *= factor;
            e.end_width *= factor;
            e.elevation = about[2] + (e.elevation - about[2]) * factor;
        }
        EntityType::Polyline3D(e) => {
            for v in &mut e.vertices {
                v.position = scale_pt(v.position, about, factor);
            }
        }
        EntityType::Spline(e) => {
            for p in &mut e.control_points {
                *p = scale_pt(*p, about, factor);
            }
            for p in &mut e.fit_points {
                *p = scale_pt(*p, about, factor);
            }
        }
        EntityType::Solid(e) => {
            e.first_corner = scale_pt(e.first_corner, about, factor);
            e.second_corner = scale_pt(e.second_corner, about, factor);
            e.third_corner = scale_pt(e.third_corner, about, factor);
            e.fourth_corner = scale_pt(e.fourth_corner, about, factor);
        }
        EntityType::Text(e) => {
            e.insertion_point = scale_pt(e.insertion_point, about, factor);
            if let Some(a) = &mut e.alignment_point {
                *a = scale_pt(*a, about, factor);
            }
            e.height *= factor;
        }
        EntityType::MText(e) => {
            e.insertion_point = scale_pt(e.insertion_point, about, factor);
            e.height *= factor;
            e.rectangle_width *= factor;
        }
        EntityType::Insert(e) => {
            e.insert_point = scale_pt(e.insert_point, about, factor);
            e.set_scale_unchecked(
                e.x_scale() * factor,
                e.y_scale() * factor,
                e.z_scale() * factor,
            );
            for att in &mut e.attributes {
                scale_attribute(att, about, factor);
            }
        }
        EntityType::Hatch(e) => {
            e.elevation = about[2] + (e.elevation - about[2]) * factor;
            e.pattern_scale *= factor;
            for p in &mut e.seed_points {
                *p = scale_v2(*p, about, factor);
            }
            for line in &mut e.pattern.lines {
                line.base_point = scale_v2(line.base_point, about, factor);
                line.offset = line.offset * factor;
                for dash in &mut line.dash_lengths {
                    *dash *= factor;
                }
            }
            for path in &mut e.paths {
                for edge in &mut path.edges {
                    match edge {
                        BoundaryEdge::Line(l) => {
                            l.start = scale_v2(l.start, about, factor);
                            l.end = scale_v2(l.end, about, factor);
                        }
                        BoundaryEdge::CircularArc(a) => {
                            a.center = scale_v2(a.center, about, factor);
                            a.radius *= factor;
                        }
                        BoundaryEdge::EllipticArc(ea) => {
                            ea.center = scale_v2(ea.center, about, factor);
                            ea.major_axis_endpoint = ea.major_axis_endpoint * factor;
                        }
                        BoundaryEdge::Spline(s) => {
                            for cp in &mut s.control_points {
                                let p = scale_v2(Vector2::new(cp.x, cp.y), about, factor);
                                cp.x = p.x;
                                cp.y = p.y;
                            }
                            for fp in &mut s.fit_points {
                                *fp = scale_v2(*fp, about, factor);
                            }
                        }
                        BoundaryEdge::Polyline(p) => {
                            for v in &mut p.vertices {
                                let pt = scale_v2(Vector2::new(v.x, v.y), about, factor);
                                v.x = pt.x;
                                v.y = pt.y;
                            }
                        }
                    }
                }
            }
        }
        other => {
            return Err(Error::new(
                ErrorCode::BadValue,
                format!(
                    "scale is not supported on {} in an original DWG",
                    kind_of(other)
                ),
            ))
        }
    }
    Ok(())
}

fn scale_attribute(att: &mut AttributeEntity, about: [f64; 3], factor: f64) {
    att.insertion_point = scale_pt(att.insertion_point, about, factor);
    if att.horizontal_alignment != HorizontalAlignment::Left
        || att.vertical_alignment != VerticalAlignment::Baseline
    {
        att.alignment_point = scale_pt(att.alignment_point, about, factor);
    }
    att.height *= factor;
    if let Some(m) = att.embedded_mtext.as_mut() {
        m.insertion_point = scale_pt(m.insertion_point, about, factor);
        m.height *= factor;
        m.rectangle_width *= factor;
    }
}

/// Mirror geometry across the 2D line through `first` and `second`, preserving
/// `MIRRTEXT 0` (text reads forwards) and `MIRRHATCH 0` (pattern angle stays).
fn mirror_entity(object: &mut EntityType, first: [f64; 3], second: [f64; 3]) -> Result<(), Error> {
    if !matches!(
        object,
        EntityType::Line(_)
            | EntityType::Circle(_)
            | EntityType::Arc(_)
            | EntityType::Ellipse(_)
            | EntityType::LwPolyline(_)
            | EntityType::Polyline2D(_)
            | EntityType::Polyline3D(_)
            | EntityType::Spline(_)
            | EntityType::Solid(_)
            | EntityType::Text(_)
            | EntityType::MText(_)
            | EntityType::Insert(_)
            | EntityType::Hatch(_)
    ) {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "mirror is not supported on {} in an original DWG",
                kind_of(object)
            ),
        ));
    }
    let dx = second[0] - first[0];
    let dy = second[1] - first[1];
    let sq = dx * dx + dy * dy;
    if sq == 0.0 || !sq.is_finite() {
        return Err(Error::new(
            ErrorCode::BadValue,
            "first_point and second_point are the same point seen from above, which names no \
             mirror line",
        ));
    }
    if has_non_standard_normal(object) {
        object.mirror_with_options(
            Vector3::new(first[0], first[1], first[2]),
            Vector3::new(second[0], second[1], second[2]),
            MirrorOptions {
                mirror_text: false,
                mirror_hatch: false,
                text_width: None,
            },
        );
        return Ok(());
    }
    let cos2 = (dx * dx - dy * dy) / sq;
    let sin2 = 2.0 * dx * dy / sq;
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
    let axis = if axis >= 180.0 { 0.0 } else { axis };
    let mp = |p: Vector3| {
        let rx = p.x - first[0];
        let ry = p.y - first[1];
        Vector3::new(
            first[0] + rx * cos2 + ry * sin2,
            first[1] + rx * sin2 - ry * cos2,
            p.z,
        )
    };
    let mv2 = |v: Vector2| {
        let rx = v.x - first[0];
        let ry = v.y - first[1];
        Vector2::new(
            first[0] + rx * cos2 + ry * sin2,
            first[1] + rx * sin2 - ry * cos2,
        )
    };
    let mdir3 = |v: Vector3| Vector3::new(v.x * cos2 + v.y * sin2, v.x * sin2 - v.y * cos2, v.z);
    let mdir2 = |v: Vector2| Vector2::new(v.x * cos2 + v.y * sin2, v.x * sin2 - v.y * cos2);
    let mdeg = |deg: f64| normalize_degrees(2.0 * axis - deg);

    match object {
        EntityType::Line(e) => {
            e.start = mp(e.start);
            e.end = mp(e.end);
        }
        EntityType::Circle(e) => {
            e.center = mp(e.center);
        }
        EntityType::Arc(e) => {
            e.center = mp(e.center);
            let new_start = mdeg(e.end_angle.to_degrees());
            let new_end = mdeg(e.start_angle.to_degrees());
            e.start_angle = new_start.to_radians();
            e.end_angle = new_end.to_radians();
        }
        EntityType::Ellipse(e) => {
            e.center = mp(e.center);
            e.major_axis = mdir3(e.major_axis);
            let whole = e.start_parameter == 0.0 && (e.end_parameter - TAU).abs() <= 1e-12;
            if !whole {
                let start = (-e.end_parameter).rem_euclid(TAU);
                let end = (-e.start_parameter).rem_euclid(TAU);
                e.start_parameter = start;
                e.end_parameter = if end > start { end } else { end + TAU };
            }
        }
        EntityType::LwPolyline(e) => {
            for v in &mut e.vertices {
                v.location = mv2(v.location);
                v.bulge = -v.bulge;
            }
        }
        EntityType::Polyline2D(e) => {
            for v in &mut e.vertices {
                v.location = mp(v.location);
                v.bulge = -v.bulge;
            }
        }
        EntityType::Polyline3D(e) => {
            for v in &mut e.vertices {
                v.position = mp(v.position);
            }
        }
        EntityType::Spline(e) => {
            for p in &mut e.control_points {
                *p = mp(*p);
            }
            for p in &mut e.fit_points {
                *p = mp(*p);
            }
            e.begin_tangent = mdir3(e.begin_tangent);
            e.end_tangent = mdir3(e.end_tangent);
        }
        EntityType::Solid(e) => {
            e.first_corner = mp(e.first_corner);
            e.second_corner = mp(e.second_corner);
            e.third_corner = mp(e.third_corner);
            e.fourth_corner = mp(e.fourth_corner);
        }
        EntityType::Text(e) => {
            // MIRRTEXT 0: text position mirrors and its direction stays reading
            // forwards (left-to-right or upwards).
            e.insertion_point = mp(e.insertion_point);
            if let Some(a) = &mut e.alignment_point {
                *a = mp(*a);
            }
            e.rotation = readable(mdeg(e.rotation.to_degrees())).to_radians();
            e.generation_flags = 0;
        }
        EntityType::MText(e) => {
            e.insertion_point = mp(e.insertion_point);
            e.rotation = readable(mdeg(e.rotation.to_degrees())).to_radians();
            e.dwg_x_direction = None;
        }
        EntityType::Insert(e) => {
            e.insert_point = mp(e.insert_point);
            // Reflecting an insert across a line at angle `axis` negates X scale
            // (reversing local +X to -X) and adds a compensating half turn (+180°)
            // to the mirrored rotation, matching AutoCAD and `transform.rs`.
            e.set_scale_unchecked(-e.x_scale(), e.y_scale(), e.z_scale());
            e.rotation = normalize_degrees(mdeg(e.rotation.to_degrees()) + 180.0).to_radians();
            for att in &mut e.attributes {
                att.insertion_point = mp(att.insertion_point);
                if att.horizontal_alignment != HorizontalAlignment::Left
                    || att.vertical_alignment != VerticalAlignment::Baseline
                {
                    att.alignment_point = mp(att.alignment_point);
                }
                att.rotation = readable(mdeg(att.rotation.to_degrees())).to_radians();
                att.text_generation_flags = 0;
                if let Some(m) = att.embedded_mtext.as_mut() {
                    m.insertion_point = mp(m.insertion_point);
                    m.rotation = readable(mdeg(m.rotation.to_degrees())).to_radians();
                    m.dwg_x_direction = None;
                }
            }
        }
        EntityType::Hatch(e) => {
            for p in &mut e.seed_points {
                *p = mv2(*p);
            }
            for line in &mut e.pattern.lines {
                line.base_point = mv2(line.base_point);
            }
            for path in &mut e.paths {
                for edge in &mut path.edges {
                    match edge {
                        BoundaryEdge::Line(l) => {
                            l.start = mv2(l.start);
                            l.end = mv2(l.end);
                        }
                        BoundaryEdge::CircularArc(a) => {
                            a.center = mv2(a.center);
                            let new_start = mdeg(a.end_angle.to_degrees());
                            let new_end = mdeg(a.start_angle.to_degrees());
                            a.start_angle = new_start.to_radians();
                            a.end_angle = new_end.to_radians();
                        }
                        BoundaryEdge::EllipticArc(ea) => {
                            ea.center = mv2(ea.center);
                            ea.major_axis_endpoint = mdir2(ea.major_axis_endpoint);
                            ea.counter_clockwise = !ea.counter_clockwise;
                        }
                        BoundaryEdge::Spline(s) => {
                            for cp in &mut s.control_points {
                                let p = mv2(Vector2::new(cp.x, cp.y));
                                cp.x = p.x;
                                cp.y = p.y;
                            }
                            for fp in &mut s.fit_points {
                                *fp = mv2(*fp);
                            }
                        }
                        BoundaryEdge::Polyline(p) => {
                            for v in &mut p.vertices {
                                let pt = mv2(Vector2::new(v.x, v.y));
                                v.x = pt.x;
                                v.y = pt.y;
                                v.z = -v.z; // bulge
                            }
                        }
                    }
                }
            }
        }
        other => {
            return Err(Error::new(
                ErrorCode::BadValue,
                format!(
                    "mirror is not supported on {} in an original DWG",
                    kind_of(other)
                ),
            ))
        }
    }
    Ok(())
}

// ── DWG version header check & structural snapshot for diff self-check ────

/// Returns the exact 6-byte DWG header `DwgWriter` emits for `version`, or
/// `None` if pinned acadrust cannot write that version without converting it.
fn writable_header_bytes(version: DxfVersion) -> Option<[u8; 6]> {
    match version {
        DxfVersion::AC1009 => Some(*b"AC1009"),
        DxfVersion::AC1012 => Some(*b"AC1012"),
        DxfVersion::AC1014 => Some(*b"AC1014"),
        DxfVersion::AC1015 => Some(*b"AC1015"),
        DxfVersion::AC1018 => Some(*b"AC1018"),
        DxfVersion::AC1021 => Some(*b"AC1021"),
        DxfVersion::AC1024 => Some(*b"AC1024"),
        DxfVersion::AC1027 => Some(*b"AC1027"),
        DxfVersion::AC1032 => Some(*b"AC1032"),
        _ => None,
    }
}

fn check_decode_regression(
    before: &DecodeReport,
    after: &DecodeReport,
    out: &mut Vec<UnexpectedChange>,
) {
    if before.decoded && !after.decoded {
        out.push(UnexpectedChange {
            kind: "DECODE".to_owned(),
            handle: "0".to_owned(),
            change: UnexpectedChangeKind::Changed,
            detail: "revision no longer decoded cleanly after writing".to_owned(),
        });
    }
    if after.skipped > before.skipped {
        out.push(UnexpectedChange {
            kind: "DECODE".to_owned(),
            handle: "0".to_owned(),
            change: UnexpectedChangeKind::Changed,
            detail: format!(
                "skipped records increased from {} to {} after writing",
                before.skipped, after.skipped
            ),
        });
    }
    if after.recovered_errors > before.recovered_errors {
        out.push(UnexpectedChange {
            kind: "DECODE".to_owned(),
            handle: "0".to_owned(),
            change: UnexpectedChangeKind::Changed,
            detail: format!(
                "recovered decode errors increased from {} to {} after writing",
                before.recovered_errors, after.recovered_errors
            ),
        });
    }
    if after.diagnostics_truncated && !before.diagnostics_truncated {
        out.push(UnexpectedChange {
            kind: "DECODE".to_owned(),
            handle: "0".to_owned(),
            change: UnexpectedChangeKind::Changed,
            detail: "revision hit the decode diagnostic limit after writing".to_owned(),
        });
    }
    for entry in &after.not_decoded {
        let prev = before
            .not_decoded
            .iter()
            .find(|b| b.kind == entry.kind)
            .map(|b| b.count)
            .unwrap_or(0);
        if entry.count > prev {
            out.push(UnexpectedChange {
                kind: "DECODE".to_owned(),
                handle: entry.kind.clone(),
                change: UnexpectedChangeKind::Changed,
                detail: format!(
                    "undecoded `{}` records increased from {prev} to {} after writing",
                    entry.kind, entry.count
                ),
            });
        }
    }
}

fn diff_named_table(
    kind: &str,
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
    out: &mut Vec<UnexpectedChange>,
) {
    for (key, orig) in before {
        match after.get(key) {
            None => out.push(UnexpectedChange {
                kind: kind.to_owned(),
                handle: key.clone(),
                change: UnexpectedChangeKind::Removed,
                detail: format!("untouched {kind} `{key}` was removed"),
            }),
            Some(now) if now != orig => out.push(UnexpectedChange {
                kind: kind.to_owned(),
                handle: key.clone(),
                change: UnexpectedChangeKind::Changed,
                detail: format!("untouched {kind} `{key}` changed in the revision"),
            }),
            Some(_) => {}
        }
    }
    for key in after.keys() {
        if !before.contains_key(key) {
            out.push(UnexpectedChange {
                kind: kind.to_owned(),
                handle: key.clone(),
                change: UnexpectedChangeKind::Added,
                detail: format!("unexpected {kind} `{key}` appeared in the revision"),
            });
        }
    }
}

fn snapshot_drawing(
    drawing: &CadDocument,
    dwg_version: &str,
    decode: &DecodeReport,
) -> DrawingSnapshot {
    let mut objects = BTreeMap::new();
    let mut sub_handles = HashSet::new();
    let r2004_plus = drawing.version >= DxfVersion::AC1018;
    for entity in drawing.entities() {
        let handle = entity.common().handle.value();
        let kind = kind_of(entity);
        let compound = is_compound(entity);
        let raw_bytes = if r2004_plus && !compound {
            entity.common().raw_record.as_ref().map(|r| r.data.clone())
        } else {
            None
        };
        objects.insert(
            handle,
            ObjectDigest {
                kind,
                owner: entity.common().owner_handle.value(),
                raw_bytes,
                logical: logical_summary(entity),
            },
        );
        match entity {
            EntityType::Insert(ins) => {
                for att in &ins.attributes {
                    if !att.common.handle.is_null() {
                        objects.insert(
                            att.common.handle.value(),
                            ObjectDigest {
                                kind: "ATTRIB".to_owned(),
                                owner: handle,
                                raw_bytes: None,
                                logical: attribute_logical_summary(att),
                            },
                        );
                    }
                }
                if let Some(seq) = ins.seqend_handle {
                    if !seq.is_null() {
                        sub_handles.insert(seq.value());
                    }
                }
            }
            EntityType::Polyline3D(p3) => {
                for v in &p3.vertices {
                    if !v.handle.is_null() {
                        sub_handles.insert(v.handle.value());
                    }
                }
            }
            _ => {}
        }
    }
    let non_graphical = drawing
        .objects
        .iter()
        .map(|(h, obj)| (h.value(), non_graphical_digest(obj)))
        .collect();
    let layers = drawing
        .layers
        .iter()
        .map(|l| {
            (
                l.name.to_lowercase(),
                LayerDigest {
                    handle: l.handle.value(),
                    name: l.name.clone(),
                    color: format!("{:?}", l.color),
                    color_name: l.color_name.clone(),
                    book_name: l.book_name.clone(),
                    linetype: l.line_type.to_lowercase(),
                    lineweight: format!("{:?}", l.line_weight),
                    flags: (
                        l.flags.off,
                        l.flags.frozen,
                        l.flags.locked,
                        l.flags.frozen_in_new_viewport,
                        l.flags.xref_dependent,
                        l.is_plottable,
                    ),
                    transparency: format!("{:?}", l.transparency),
                    description: l.description.clone(),
                },
            )
        })
        .collect();
    let line_types = drawing
        .line_types
        .iter()
        .map(|lt| (lt.name.to_lowercase(), format!("{lt:?}")))
        .collect();
    let text_styles = drawing
        .text_styles
        .iter()
        .map(|ts| (ts.name.to_lowercase(), format!("{ts:?}")))
        .collect();
    let dim_styles = drawing
        .dim_styles
        .iter()
        .map(|ds| (ds.name.to_lowercase(), format!("{ds:?}")))
        .collect();
    let block_records = drawing
        .block_records
        .iter()
        .map(|br| {
            (
                br.name.to_lowercase(),
                BlockRecordDigest {
                    handle: br.handle.value(),
                    name: br.name.clone(),
                    flags: format!("{:?}", br.flags),
                    layout_handle: br.layout.value(),
                    entity_handles: br.entity_handles.iter().map(|h| h.value()).collect(),
                },
            )
        })
        .collect();
    DrawingSnapshot {
        dwg_version: dwg_version.to_owned(),
        decode: decode.clone(),
        objects,
        sub_handles,
        non_graphical,
        layers,
        line_types,
        text_styles,
        dim_styles,
        block_records,
    }
}

fn non_graphical_digest(obj: &ObjectType) -> NonGraphicalDigest {
    let (kind, owner, summary) = match obj {
        ObjectType::Dictionary(d) => ("DICTIONARY".to_owned(), d.owner.value(), format!("{d:?}")),
        ObjectType::Layout(l) => {
            let mut clean = l.clone();
            clean.raw_plot_settings_codes = None;
            ("LAYOUT".to_owned(), l.owner.value(), format!("{clean:?}"))
        }
        ObjectType::XRecord(x) => {
            let summary = if x.entries_complete {
                format!(
                    "XRecord(h={:?},o={:?},r={:?},xd={:?},name={:?},cf={:?},e={:?},refs={:?})",
                    x.handle,
                    x.owner,
                    x.reactors,
                    x.xdictionary_handle,
                    x.name,
                    x.cloning_flags,
                    x.entries,
                    x.object_references
                )
            } else {
                format!(
                    "XRecordRaw(h={:?},o={:?},raw={:?},e={:?})",
                    x.handle, x.owner, x.raw_dwg_data, x.entries
                )
            };
            ("XRECORD".to_owned(), x.owner.value(), summary)
        }
        ObjectType::Group(g) => ("GROUP".to_owned(), g.owner.value(), format!("{g:?}")),
        ObjectType::MLineStyle(m) => ("MLINESTYLE".to_owned(), m.owner.value(), format!("{m:?}")),
        ObjectType::ImageDefinition(i) => {
            ("IMAGEDEF".to_owned(), i.owner.value(), format!("{i:?}"))
        }
        ObjectType::UnderlayDefinition(u) => (
            "UNDERLAYDEF".to_owned(),
            u.owner_handle.value(),
            format!("{u:?}"),
        ),
        ObjectType::PlotSettings(p) => (
            "PLOTSETTINGS".to_owned(),
            p.owner.value(),
            format!("{p:?}"),
        ),
        ObjectType::MultiLeaderStyle(m) => (
            "MLEADERSTYLE".to_owned(),
            m.owner_handle.value(),
            format!("{m:?}"),
        ),
        ObjectType::TableStyle(t) => {
            let mut clean = t.clone();
            clean.raw_dxf_codes = None;
            (
                "TABLESTYLE".to_owned(),
                t.owner_handle.value(),
                format!("{clean:?}"),
            )
        }
        ObjectType::TableContent(t) => {
            let mut clean = t.clone();
            clean.common.raw_record = None;
            (
                "TABLECONTENT".to_owned(),
                t.common.owner_handle.value(),
                format!("{clean:?}"),
            )
        }
        ObjectType::TableTemplate(t) => {
            let mut clean = t.clone();
            clean.common.raw_record = None;
            (
                "TABLETEMPLATE".to_owned(),
                t.common.owner_handle.value(),
                format!("{clean:?}"),
            )
        }
        ObjectType::Scale(s) => (
            "SCALE".to_owned(),
            s.owner_handle.value(),
            format!("{s:?}"),
        ),
        ObjectType::ObjectContextData(c) => (
            "OBJECTCONTEXTDATA".to_owned(),
            c.owner_handle.value(),
            format!("{c:?}"),
        ),
        ObjectType::SortEntitiesTable(s) => {
            let entries: Vec<(u64, u64)> = s
                .entries()
                .map(|e| (e.entity_handle.value(), e.sort_handle.value()))
                .collect();
            (
                "SORTENTSTABLE".to_owned(),
                s.owner_handle.value(),
                format!(
                    "SortEntitiesTable(h={:?},o={:?},bo={:?},e={entries:?})",
                    s.handle, s.owner_handle, s.block_owner_handle
                ),
            )
        }
        ObjectType::DictionaryVariable(d) => (
            "DICTIONARYVAR".to_owned(),
            d.owner_handle.value(),
            format!("{d:?}"),
        ),
        ObjectType::VisualStyle(v) => {
            ("VISUALSTYLE".to_owned(), v.owner.value(), format!("{v:?}"))
        }
        ObjectType::Material(m) => ("MATERIAL".to_owned(), m.owner.value(), format!("{m:?}")),
        ObjectType::ImageDefinitionReactor(r) => (
            "IMAGEDEF_REACTOR".to_owned(),
            r.owner.value(),
            format!("{r:?}"),
        ),
        ObjectType::GeoData(g) => ("GEODATA".to_owned(), g.owner.value(), format!("{g:?}")),
        ObjectType::SpatialFilter(s) => (
            "SPATIAL_FILTER".to_owned(),
            s.owner.value(),
            format!("{s:?}"),
        ),
        ObjectType::RasterVariables(r) => (
            "RASTERVARIABLES".to_owned(),
            r.owner.value(),
            format!("{r:?}"),
        ),
        ObjectType::BookColor(b) => ("DBCOLOR".to_owned(), b.owner.value(), format!("{b:?}")),
        ObjectType::PlaceHolder(p) => (
            "ACDBPLACEHOLDER".to_owned(),
            p.owner.value(),
            format!("{p:?}"),
        ),
        ObjectType::DictionaryWithDefault(d) => (
            "ACDBDICTIONARYWDFLT".to_owned(),
            d.owner.value(),
            format!("{d:?}"),
        ),
        ObjectType::WipeoutVariables(w) => (
            "WIPEOUTVARIABLES".to_owned(),
            w.owner.value(),
            format!("{w:?}"),
        ),
        ObjectType::BlockVisibilityParameter(p) => (
            "BLOCKVISIBILITYPARAMETER".to_owned(),
            p.owner.value(),
            format!("{p:?}"),
        ),
        ObjectType::DynamicBlock(d) => (
            "DYNAMICBLOCK".to_owned(),
            d.owner.value(),
            format!("{d:?}"),
        ),
        ObjectType::Associative(a) => (
            "ASSOCIATIVE".to_owned(),
            a.owner.value(),
            format!("{a:?}"),
        ),
        ObjectType::ClassObject(c) => (
            "CLASSOBJECT".to_owned(),
            c.owner.value(),
            format!("{c:?}"),
        ),
        ObjectType::DataObject(d) => (
            "DATAOBJECT".to_owned(),
            d.owner.value(),
            format!("{d:?}"),
        ),
        ObjectType::Field(f) => ("FIELD".to_owned(), f.owner.value(), format!("{f:?}")),
        ObjectType::FieldList(f) => ("FIELDLIST".to_owned(), f.owner.value(), format!("{f:?}")),
        ObjectType::RegisteredClass(r) => {
            let summary = if !r.properties.is_empty() {
                format!(
                    "RegisteredClass(h={:?},o={:?},r={:?},xd={:?},dxf={:?},cpp={:?},p={:?},pl={:?},ids={:?})",
                    r.handle,
                    r.owner,
                    r.reactors,
                    r.xdictionary_handle,
                    r.dxf_name,
                    r.cpp_class_name,
                    r.properties,
                    r.payload,
                    r.object_ids
                )
            } else {
                format!(
                    "RegisteredClassRaw(h={:?},o={:?},dxf={:?},raw={:?})",
                    r.handle, r.owner, r.dxf_name, r.raw_dwg_data
                )
            };
            ("REGISTEREDCLASS".to_owned(), r.owner.value(), summary)
        }
        ObjectType::DgnLineStyle(d) => (
            "DGNLINESTYLE".to_owned(),
            d.owner.value(),
            format!("{d:?}"),
        ),
        ObjectType::ProxyObject(p) => (
            "PROXYOBJECT".to_owned(),
            p.owner.value(),
            format!("{p:?}"),
        ),
        ObjectType::Arch(a) => {
            let mut clean = a.clone();
            clean.raw_dwg_data = None;
            clean.raw_dwg_handle_bits = 0;
            clean.raw_dwg_version = None;
            clean.raw_dxf_codes = None;
            ("ARCH".to_owned(), a.owner.value(), format!("{clean:?}"))
        }
        ObjectType::Unknown {
            type_name,
            handle,
            owner,
            raw_dwg_data,
            raw_dxf_codes,
            ..
        } => (
            type_name.clone(),
            owner.value(),
            format!(
                "Unknown(t={type_name:?},h={handle:?},o={owner:?},raw={raw_dwg_data:?},dxf={raw_dxf_codes:?})"
            ),
        ),
    };
    NonGraphicalDigest {
        kind,
        owner,
        summary,
    }
}

fn is_compound(entity: &EntityType) -> bool {
    matches!(
        entity,
        EntityType::Polyline(_)
            | EntityType::Polyline2D(_)
            | EntityType::Polyline3D(_)
            | EntityType::PolyfaceMesh(_)
            | EntityType::PolygonMesh(_)
    ) || matches!(entity, EntityType::Insert(i) if !i.attributes.is_empty() || i.seqend_handle.is_some())
        || matches!(
            entity,
            EntityType::Solid3D(_)
                | EntityType::Region(_)
                | EntityType::Body(_)
                | EntityType::Surface(_)
        )
        || matches!(entity, EntityType::MText(t) if t.background_fill_flags & 0x11 == 0x10)
}

fn xdata_summary(c: &EntityCommon) -> String {
    let mut parts: Vec<String> = c
        .extended_data
        .records()
        .iter()
        .map(|rec| format!("{rec:?}"))
        .collect();
    parts.sort();
    let raw_eed: Vec<String> = c
        .extended_data
        .raw_dwg_eed
        .iter()
        .map(|(h, bytes)| format!("raw({h:X}):{bytes:?}"))
        .collect();
    format!("xd=[{}] reed=[{}]", parts.join(","), raw_eed.join(","))
}

fn common_summary(kind: &str, c: &EntityCommon) -> String {
    format!(
        "{kind} layer={} color={:?} cname={:?} lt={} lts={:.6} lw={:?} inv={} tr={:?} {}",
        c.layer,
        c.color,
        c.color_name,
        c.linetype,
        c.linetype_scale,
        c.line_weight,
        c.invisible,
        c.transparency,
        xdata_summary(c),
    )
}

fn attribute_logical_summary(att: &AttributeEntity) -> String {
    format!(
        "{} tag={:?} value={:?} pos=({:.6},{:.6},{:.6}) align=({:.6},{:.6},{:.6}) \
         ha={:?} va={:?} h={:.6} r={:.6} wf={:.6} obl={:.6} style={} flags={:?} \
         n=({:.6},{:.6},{:.6}) mtext={:?}",
        common_summary("ATTRIB", &att.common),
        att.tag,
        att.value,
        att.insertion_point.x,
        att.insertion_point.y,
        att.insertion_point.z,
        att.alignment_point.x,
        att.alignment_point.y,
        att.alignment_point.z,
        att.horizontal_alignment,
        att.vertical_alignment,
        att.height,
        att.rotation,
        att.width_factor,
        att.oblique_angle,
        att.text_style,
        att.flags,
        att.normal.x,
        att.normal.y,
        att.normal.z,
        att.embedded_mtext.as_ref().map(|m| (
            &m.value,
            m.insertion_point,
            m.height,
            m.rotation,
            m.rectangle_width
        )),
    )
}

fn logical_summary(entity: &EntityType) -> String {
    let c = entity.common();
    let kind = kind_of(entity);
    let base = common_summary(&kind, c);
    match entity {
        EntityType::Line(e) => format!(
            "{base} ({:.6},{:.6},{:.6})->({:.6},{:.6},{:.6}) n=({:.6},{:.6},{:.6})",
            e.start.x, e.start.y, e.start.z, e.end.x, e.end.y, e.end.z, e.normal.x, e.normal.y, e.normal.z
        ),
        EntityType::Circle(e) => format!(
            "{base} ({:.6},{:.6},{:.6}) r={:.6} n=({:.6},{:.6},{:.6})",
            e.center.x, e.center.y, e.center.z, e.radius, e.normal.x, e.normal.y, e.normal.z
        ),
        EntityType::Arc(e) => format!(
            "{base} ({:.6},{:.6},{:.6}) r={:.6} a=({:.6},{:.6}) n=({:.6},{:.6},{:.6})",
            e.center.x,
            e.center.y,
            e.center.z,
            e.radius,
            e.start_angle,
            e.end_angle,
            e.normal.x,
            e.normal.y,
            e.normal.z
        ),
        EntityType::Ellipse(e) => format!(
            "{base} ({:.6},{:.6},{:.6}) maj=({:.6},{:.6},{:.6}) mr={:.6} p=({:.6},{:.6}) n=({:.6},{:.6},{:.6})",
            e.center.x,
            e.center.y,
            e.center.z,
            e.major_axis.x,
            e.major_axis.y,
            e.major_axis.z,
            e.minor_axis_ratio,
            e.start_parameter,
            e.end_parameter,
            e.normal.x,
            e.normal.y,
            e.normal.z
        ),
        EntityType::LwPolyline(e) => {
            let verts: Vec<String> = e
                .vertices
                .iter()
                .map(|v| {
                    format!(
                        "({:.6},{:.6},b={:.6},sw={:.6},ew={:.6})",
                        v.location.x, v.location.y, v.bulge, v.start_width, v.end_width
                    )
                })
                .collect();
            format!(
                "{base} closed={} cw={:.6} elev={:.6} n=({:.6},{:.6},{:.6}) v=[{}]",
                e.is_closed,
                e.constant_width,
                e.elevation,
                e.normal.x,
                e.normal.y,
                e.normal.z,
                verts.join(",")
            )
        }
        EntityType::Polyline2D(e) => {
            let verts: Vec<String> = e
                .vertices
                .iter()
                .map(|v| {
                    format!(
                        "({:.6},{:.6},{:.6},b={:.6},sw={:.6},ew={:.6})",
                        v.location.x, v.location.y, v.location.z, v.bulge, v.start_width, v.end_width
                    )
                })
                .collect();
            format!(
                "{base} flags={:?} sw={:.6} ew={:.6} elev={:.6} n=({:.6},{:.6},{:.6}) v=[{}]",
                e.flags,
                e.start_width,
                e.end_width,
                e.elevation,
                e.normal.x,
                e.normal.y,
                e.normal.z,
                verts.join(",")
            )
        }
        EntityType::Polyline3D(e) => {
            let verts: Vec<String> = e
                .vertices
                .iter()
                .map(|v| format!("({:.6},{:.6},{:.6})", v.position.x, v.position.y, v.position.z))
                .collect();
            format!("{base} flags={:?} v=[{}]", e.flags, verts.join(","))
        }
        EntityType::Spline(e) => format!(
            "{base} deg={} flags={:?} knots={:?} cp={:?} w={:?} fp={:?} bt={:?} et={:?}",
            e.degree,
            e.flags,
            e.knots,
            e.control_points,
            e.weights,
            e.fit_points,
            e.begin_tangent,
            e.end_tangent
        ),
        EntityType::Solid(e) => format!(
            "{base} c1={:?} c2={:?} c3={:?} c4={:?} n={:?}",
            e.first_corner, e.second_corner, e.third_corner, e.fourth_corner, e.normal
        ),
        EntityType::Text(e) => format!(
            "{base} {:?} pos=({:.6},{:.6},{:.6}) align={:?} ha={:?} va={:?} h={:.6} r={:.6} \
             wf={:.6} obl={:.6} style={} gen={} n=({:.6},{:.6},{:.6})",
            e.value,
            e.insertion_point.x,
            e.insertion_point.y,
            e.insertion_point.z,
            e.alignment_point,
            e.horizontal_alignment,
            e.vertical_alignment,
            e.height,
            e.rotation,
            e.width_factor,
            e.oblique_angle,
            e.style,
            e.generation_flags,
            e.normal.x,
            e.normal.y,
            e.normal.z
        ),
        EntityType::MText(e) => format!(
            "{base} {:?} pos=({:.6},{:.6},{:.6}) h={:.6} rw={:.6} r={:.6} att={:?} style={} \
             n=({:.6},{:.6},{:.6})",
            e.value,
            e.insertion_point.x,
            e.insertion_point.y,
            e.insertion_point.z,
            e.height,
            e.rectangle_width,
            e.rotation,
            e.attachment_point,
            e.style,
            e.normal.x,
            e.normal.y,
            e.normal.z
        ),
        EntityType::Insert(e) => format!(
            "{base} block={} pos=({:.6},{:.6},{:.6}) s=({:.6},{:.6},{:.6}) r={:.6} \
             n=({:.6},{:.6},{:.6}) grid=({},{},{:.6},{:.6}) atts={}",
            e.block_name,
            e.insert_point.x,
            e.insert_point.y,
            e.insert_point.z,
            e.x_scale(),
            e.y_scale(),
            e.z_scale(),
            e.rotation,
            e.normal.x,
            e.normal.y,
            e.normal.z,
            e.column_count,
            e.row_count,
            e.column_spacing,
            e.row_spacing,
            e.attributes.len()
        ),
        EntityType::Hatch(e) => format!(
            "{base} solid={} pat={} ang={:.6} scale={:.6} elev={:.6} n={:?} paths={:?} seeds={:?}",
            e.is_solid,
            e.pattern.name,
            e.pattern_angle,
            e.pattern_scale,
            e.elevation,
            e.normal,
            e.paths,
            e.seed_points
        ),
        EntityType::Unknown(u) => format!(
            "{base} code={} raw={:?}",
            u.dwg_type_code, u.raw_dwg_data
        ),
        _ => base,
    }
}

/// Only for tests: writes a made-up DWG at `dwg` out in a specific `DxfVersion`
/// (e.g. `"AC1015"`, `"AC1018"`, `"AC1021"`, `"AC1024"`, `"AC1027"`, `"AC1032"`),
/// stripped of AgentDrafter's build mark and extended data so it acts as an
/// original of that version (ADR-0006, ADR-0008).
#[cfg(test)]
pub(crate) fn as_original_of_version(dwg: &Path, out: &Path, version: &str) {
    let target = match version {
        "AC1009" => DxfVersion::AC1009,
        "AC1012" => DxfVersion::AC1012,
        "AC1014" => DxfVersion::AC1014,
        "AC1015" => DxfVersion::AC1015,
        "AC1018" => DxfVersion::AC1018,
        "AC1021" => DxfVersion::AC1021,
        "AC1024" => DxfVersion::AC1024,
        "AC1027" => DxfVersion::AC1027,
        "AC1032" => DxfVersion::AC1032,
        other => panic!("unsupported test version `{other}`"),
    };
    let mut drawing = super::read_drawing(dwg).expect("the DWG reads");
    drawing.version = target;
    drawing.dwg_source_version = None;
    super::strip_build_metadata(&mut drawing);
    let bytes = DwgWriter::write_to_vec(&drawing).expect("the DWG writes");
    std::fs::write(out, bytes).expect("the DWG is saved");
}

/// Only for tests: reads the raw record bytes for `hex` handle from a DWG file,
/// to verify byte-identical preservation of untouched or custom objects.
#[cfg(test)]
pub(crate) fn raw_record_bytes_of(dwg: &Path, hex: &str) -> Option<Vec<u8>> {
    let drawing = super::read_drawing(dwg).expect("the DWG reads");
    let handle = parse_hex_handle(hex).ok()?;
    let entity = drawing.get_entity(handle)?;
    if let EntityType::Unknown(u) = entity {
        return u.raw_dwg_data.clone();
    }
    entity.common().raw_record.as_ref().map(|r| r.data.clone())
}

/// Only for tests: adds a locked true-color layer with custom linetype &
/// lineweight to an original DWG so `change_layer` can prove it mutates only
/// requested fields in place.
#[cfg(test)]
pub(crate) fn with_true_color_locked_layer(
    dwg: &Path,
    out: &Path,
    layer_name: &str,
    r: u8,
    g: u8,
    b: u8,
) {
    use acadrust::tables::{LineType, LineTypeElement};

    let mut drawing = super::read_drawing(dwg).expect("the DWG reads");
    super::strip_build_metadata(&mut drawing);
    if !drawing
        .line_types
        .iter()
        .any(|lt| lt.name.eq_ignore_ascii_case("DASHED"))
    {
        let mut lt = LineType::new("DASHED");
        lt.elements.push(LineTypeElement::dash(5.0));
        lt.elements.push(LineTypeElement::space(2.5));
        lt.pattern_length = 7.5;
        lt.set_handle(drawing.allocate_handle());
        drawing.line_types.add(lt).expect("add DASHED");
    }
    let mut layer = DwgLayer::new(layer_name.to_owned());
    layer.color = Color::Rgb { r, g, b };
    layer.line_type = "DASHED".to_owned();
    layer.line_weight = LineWeight::Value(35);
    layer.flags.locked = true;
    layer.set_handle(drawing.allocate_handle());
    drawing.layers.add(layer).expect("add true-color layer");
    let bytes = DwgWriter::write_to_vec(&drawing).expect("write DWG");
    std::fs::write(out, bytes).expect("save DWG");
}

/// Only for tests: inspects a layer's raw color, linetype, lineweight, and
/// locked flag straight from a DWG.
#[cfg(test)]
pub(crate) fn layer_details_of(
    dwg: &Path,
    layer_name: &str,
) -> Option<(String, String, String, bool)> {
    let drawing = super::read_drawing(dwg).expect("the DWG reads");
    let l = drawing
        .layers
        .iter()
        .find(|l| l.name.eq_ignore_ascii_case(layer_name))?;
    Some((
        format!("{:?}", l.color),
        l.line_type.clone(),
        format!("{:?}", l.line_weight),
        l.flags.locked,
    ))
}

/// Only for tests: rewrites the first 6 bytes of a DWG copy to `header` (e.g.
/// `b"AD1032"` or `b"AC1006"`) to verify exact 6-byte version header checks.
#[cfg(test)]
pub(crate) fn with_custom_header(dwg: &Path, out: &Path, header: &[u8; 6]) {
    let mut bytes = std::fs::read(dwg).expect("read DWG");
    assert!(bytes.len() >= 6);
    bytes[..6].copy_from_slice(header);
    std::fs::write(out, bytes).expect("save DWG");
}

/// Only for tests: adds a 3-vertex `Polyline3D` with custom XDATA to an
/// original DWG and returns its hex handle.
#[cfg(test)]
pub(crate) fn with_polyline3d_original(dwg: &Path, out: &Path) -> String {
    use acadrust::entities::Polyline3D;
    use acadrust::tables::AppId;
    use acadrust::xdata::{ExtendedDataRecord, XDataValue};

    let mut drawing = super::read_drawing(dwg).expect("the DWG reads");
    super::strip_build_metadata(&mut drawing);
    if !drawing
        .app_ids
        .iter()
        .any(|a| a.name.eq_ignore_ascii_case("TESTAPP"))
    {
        let mut app = AppId::new("TESTAPP");
        app.set_handle(drawing.allocate_handle());
        drawing.app_ids.add(app).expect("add TESTAPP");
    }
    let mut p3 = Polyline3D::from_points(vec![
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(10.0, 0.0, 5.0),
        Vector3::new(10.0, 10.0, 10.0),
    ]);
    p3.common.layer = LAYER_ZERO.to_owned();
    let mut rec = ExtendedDataRecord::new("TESTAPP");
    rec.add_value(XDataValue::String("KEEP-ME".to_owned()));
    p3.common.extended_data.upsert_record(rec);
    let handle = drawing
        .add_entity(EntityType::Polyline3D(p3))
        .expect("add Polyline3D");
    let mut v_handles = Vec::new();
    for _ in 0..3 {
        v_handles.push(drawing.allocate_handle());
    }
    if let Some(EntityType::Polyline3D(added)) = drawing.get_entity_mut(handle) {
        for (v, vh) in added.vertices.iter_mut().zip(v_handles) {
            v.handle = vh;
        }
    }
    let bytes = DwgWriter::write_to_vec(&drawing).expect("write DWG");
    std::fs::write(out, bytes).expect("save DWG");
    handle_text(handle)
}

/// Only for tests: returns the vertex hex handles, positions, and whether
/// `"TESTAPP"` xdata is still present on a `Polyline3D`.
#[cfg(test)]
pub(crate) fn polyline3d_details_of(
    dwg: &Path,
    hex: &str,
) -> Option<(Vec<String>, Vec<[f64; 3]>, bool)> {
    let drawing = super::read_drawing(dwg).expect("the DWG reads");
    let handle = parse_hex_handle(hex).ok()?;
    let EntityType::Polyline3D(p3) = drawing.get_entity(handle)? else {
        return None;
    };
    let handles = p3.vertices.iter().map(|v| handle_text(v.handle)).collect();
    let pts = p3
        .vertices
        .iter()
        .map(|v| [v.position.x, v.position.y, v.position.z])
        .collect();
    let has_xdata = p3
        .common
        .extended_data
        .records()
        .iter()
        .any(|r| r.application_name.eq_ignore_ascii_case("TESTAPP"));
    Some((handles, pts, has_xdata))
}

/// Only for tests: creates an original DWG holding an insert on layer `0` with
/// three attributes: a constant `TAG` (`"CONST"`), an editable `TAG` on locked
/// layer `ATTR_LOCK` (`"LOCKED"`), and an editable `TAG` on layer `0` (`"FREE"`).
/// Returns the insert's hex handle.
#[cfg(test)]
pub(crate) fn with_constant_and_locked_attrs(dwg: &Path, out: &Path) -> String {
    use acadrust::tables::BlockRecord;

    let mut drawing = super::read_drawing(dwg).expect("the DWG reads");
    super::strip_build_metadata(&mut drawing);
    if !drawing
        .block_records
        .iter()
        .any(|b| b.name.eq_ignore_ascii_case("TESTBLK"))
    {
        let mut br = BlockRecord::new("TESTBLK");
        br.set_handle(drawing.allocate_handle());
        drawing.block_records.add(br).expect("add TESTBLK");
    }
    let mut locked = DwgLayer::new("ATTR_LOCK".to_owned());
    locked.flags.locked = true;
    locked.set_handle(drawing.allocate_handle());
    drawing.layers.add(locked).expect("add ATTR_LOCK");

    let mut ins = DwgInsert::new("TESTBLK", Vector3::new(50.0, 50.0, 0.0));
    ins.common.layer = LAYER_ZERO.to_owned();

    let mut a_const = AttributeEntity::new("TAG".to_owned(), "CONST".to_owned());
    a_const.insertion_point = Vector3::new(50.0, 55.0, 0.0);
    a_const.flags.constant = true;
    a_const.common.layer = LAYER_ZERO.to_owned();

    let mut a_locked = AttributeEntity::new("TAG".to_owned(), "LOCKED".to_owned());
    a_locked.insertion_point = Vector3::new(50.0, 50.0, 0.0);
    a_locked.common.layer = "ATTR_LOCK".to_owned();

    let mut a_free = AttributeEntity::new("TAG".to_owned(), "FREE".to_owned());
    a_free.insertion_point = Vector3::new(50.0, 45.0, 0.0);
    a_free.common.layer = LAYER_ZERO.to_owned();

    ins.attributes = vec![a_const, a_locked, a_free];
    let handle = drawing
        .add_entity(EntityType::Insert(ins))
        .expect("add insert");
    let att_handles = [
        drawing.allocate_handle(),
        drawing.allocate_handle(),
        drawing.allocate_handle(),
    ];
    let seq = drawing.allocate_handle();
    if let Some(EntityType::Insert(added)) = drawing.get_entity_mut(handle) {
        for (a, ah) in added.attributes.iter_mut().zip(att_handles) {
            a.common.handle = ah;
            a.common.owner_handle = handle;
        }
        added.seqend_handle = Some(seq);
    }
    let bytes = DwgWriter::write_to_vec(&drawing).expect("write DWG");
    std::fs::write(out, bytes).expect("save DWG");
    handle_text(handle)
}

/// Only for tests: adds an `Arc` with `-Z` normal `(0, 0, -1)` at OCS center
/// `(10, 20, 0)` (= WCS `(-10, 20, 0)`) to an original DWG and returns its hex
/// handle.
#[cfg(test)]
pub(crate) fn with_neg_z_arc_original(dwg: &Path, out: &Path) -> String {
    use acadrust::entities::Arc as DwgArc;

    let mut drawing = super::read_drawing(dwg).expect("the DWG reads");
    super::strip_build_metadata(&mut drawing);
    let mut arc = DwgArc::from_center_radius_angles(
        Vector3::new(10.0, 20.0, 0.0),
        5.0,
        0.0,
        std::f64::consts::FRAC_PI_2,
    );
    arc.normal = Vector3::new(0.0, 0.0, -1.0);
    arc.common.layer = LAYER_ZERO.to_owned();
    let handle = drawing.add_entity(EntityType::Arc(arc)).expect("add arc");
    let bytes = DwgWriter::write_to_vec(&drawing).expect("write DWG");
    std::fs::write(out, bytes).expect("save DWG");
    handle_text(handle)
}

/// Only for tests: returns the WCS center (via OCS->WCS arbitrary-axis matrix)
/// and normal of an `Arc` at `hex`.
#[cfg(test)]
pub(crate) fn arc_world_center_and_normal(dwg: &Path, hex: &str) -> Option<([f64; 3], [f64; 3])> {
    let drawing = super::read_drawing(dwg).expect("the DWG reads");
    let handle = parse_hex_handle(hex).ok()?;
    let EntityType::Arc(arc) = drawing.get_entity(handle)? else {
        return None;
    };
    let n = arc.normal.normalize();
    let ax = if n.x.abs() < 1.0 / 64.0 && n.y.abs() < 1.0 / 64.0 {
        Vector3::UNIT_Y.cross(&n).normalize()
    } else {
        Vector3::UNIT_Z.cross(&n).normalize()
    };
    let ay = n.cross(&ax).normalize();
    let wcs = ax * arc.center.x + ay * arc.center.y + n * arc.center.z;
    Some((
        [wcs.x, wcs.y, wcs.z],
        [arc.normal.x, arc.normal.y, arc.normal.z],
    ))
}

/// Only for tests: returns the hex handle of an entity owned by a non-space
/// block definition in `dwg`.
#[cfg(test)]
pub(crate) fn block_definition_entity_handle(dwg: &Path) -> Option<String> {
    let drawing = super::read_drawing(dwg).expect("the DWG reads");
    let block_owners: HashSet<Handle> = drawing
        .block_records
        .iter()
        .filter(|br| {
            !br.name.eq_ignore_ascii_case("*Model_Space")
                && !br.name.to_ascii_lowercase().starts_with("*paper_space")
        })
        .map(|br| br.handle)
        .collect();
    let found = drawing
        .entities()
        .find(|e| block_owners.contains(&e.common().owner_handle))
        .map(|e| handle_text(e.common().handle));
    found
}

/// Only for tests: reads AgentDrafter's carried ID and name directly from the
/// entity at `hex` in `dwg`.
#[cfg(test)]
pub(crate) fn carried_id_and_name_of(dwg: &Path, hex: &str) -> (Option<String>, Option<String>) {
    let drawing = super::read_drawing(dwg).expect("the DWG reads");
    let Ok(handle) = parse_hex_handle(hex) else {
        return (None, None);
    };
    let Some(entity) = drawing.get_entity(handle) else {
        return (None, None);
    };
    (
        ids::read_id(entity.common()),
        ids::read_name(entity.common()),
    )
}

// Keep unused imports clean; `attribute_text` / `read_properties` / `set_properties`
// are kept available via module imports if needed.
const _: fn(&crate::model::Insert, &Block, &crate::model::AttributeDefinition, &str) -> TextObject =
    attribute_text;
const _: fn(&EntityCommon) -> Option<Properties> = read_properties;
const _: fn(&mut EntityCommon, &Properties) = set_properties;
