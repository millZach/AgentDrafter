//! Dynamic block reading, writing, and visibility-state revision behind the
//! `dwg` seam (ADR-0004, ADR-0009).

use std::collections::{BTreeMap, HashMap, HashSet};

use acadrust::entities::{EntityType, Insert as DwgInsert};
use acadrust::objects::{BlockEvalValue, ObjectType, XRecordEntry};
use acadrust::tables::{BlockRecord, TableEntry};
use acadrust::types::Handle;
use acadrust::CadDocument;

use super::block::{add_block_record, dwg_insert};
use super::{dwg_object, ids, point, vector};
use crate::description::normalize_degrees;
use crate::error::{Error, ErrorCode};
use crate::model::{
    is_default_state, Block, DynamicParameter, DynamicParameterKind, DynamicValue, Insert, Point3,
};
use crate::text::Text as TextObject;

/// Collects anonymous block names referenced by inserts that carry AgentDrafter's
/// `dyn_block` extended data, so `read_block` knows to read those evaluated `*U<n>`
/// blocks back when reading a DWG built by AgentDrafter.
pub(super) fn agentdrafter_evaluated_blocks(drawing: &CadDocument) -> HashSet<String> {
    let mut names = HashSet::new();
    for entity in super::in_every_space(drawing) {
        if let EntityType::Insert(ins) = entity {
            if ids::read_dynamic_block(&ins.common).is_some() && ins.block_name.starts_with('*') {
                names.insert(ins.block_name.clone());
            }
        }
    }
    names
}

/// Whether `record` is a dynamic block definition or an evaluated anonymous
/// block (`*U<n>`) belonging to a dynamic block (either AutoCAD's via
/// `ACAD_ENHANCEDBLOCK` / `AcDbBlockRepBTag` or AgentDrafter's via XDATA).
pub(super) fn is_dynamic_or_evaluated(
    drawing: &CadDocument,
    record: &BlockRecord,
    ad_evaluated: &HashSet<String>,
) -> bool {
    drawing.is_dynamic_block(record.handle)
        || drawing.dynamic_block_definition(record.handle).is_some()
        || ad_evaluated.contains(&record.name)
}

/// Projects `acadrust`'s read-only dynamic block properties for a master dynamic
/// block definition into core [`DynamicParameter`] records.
pub(super) fn read_block_parameters(
    drawing: &CadDocument,
    record: &BlockRecord,
) -> Vec<DynamicParameter> {
    if !drawing.is_dynamic_block(record.handle) {
        return Vec::new();
    }
    let Ok(props) = drawing.dynamic_block_properties(record.handle, None) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(props.len());
    for prop in props {
        if !prop.show || prop.name.is_empty() {
            continue;
        }
        let parameter_type = match (prop.type_code, prop.units) {
            (3, _) => "flip",
            (5, _) => "visibility_or_lookup",
            (_, 1) => "rotation",
            (_, 2) => "linear",
            (_, 3) => "area",
            _ => "dynamic",
        }
        .to_owned();
        let default = eval_value_to_dynamic(&prop.value, prop.type_code, prop.units);
        let allowed = prop
            .allowed_values
            .iter()
            .filter_map(|v| eval_value_to_dynamic(v, prop.type_code, prop.units))
            .collect();
        out.push(DynamicParameter {
            name: prop.name,
            kind: DynamicParameterKind::External {
                parameter_type,
                default,
                allowed,
                read_only: prop.read_only,
            },
        });
    }
    out
}

/// Dynamic block info resolved for one `INSERT` when reading a DWG:
/// `(effective_block_name, dynamic_state, evaluated_anonymous_block_name)`.
pub(super) fn read_insert_dynamic_info(
    drawing: &CadDocument,
    insert: &DwgInsert,
) -> Option<(String, BTreeMap<String, DynamicValue>, Option<String>)> {
    // 1. Native AutoCAD dynamic block (plant DWGs via `ACAD_ENHANCEDBLOCK`).
    let def_handle = drawing
        .dynamic_definition_for_insert(insert.common.handle)
        .or_else(|| {
            drawing
                .block_records
                .get(&insert.block_name)
                .and_then(|r| drawing.dynamic_block_definition(r.handle))
        });
    if let Some(def_handle) = def_handle {
        if drawing.is_dynamic_block(def_handle) {
            let effective_name = drawing
                .block_records
                .iter()
                .find(|r| r.handle == def_handle)
                .map(|r| r.name.clone())
                .unwrap_or_else(|| insert.block_name.clone());
            let mut state = BTreeMap::new();
            if let Ok(props) =
                drawing.dynamic_block_properties(def_handle, Some(insert.common.handle))
            {
                for prop in props {
                    if !prop.show || !prop.visible || prop.name.is_empty() {
                        continue;
                    }
                    if let Some(val) =
                        eval_value_to_dynamic(&prop.value, prop.type_code, prop.units)
                    {
                        state.insert(prop.name, val);
                    }
                }
            }
            let evaluated_block =
                (insert.block_name != effective_name).then(|| insert.block_name.clone());
            return Some((effective_name, state, evaluated_block));
        }
    }

    // 2. AgentDrafter-built dynamic block insert (`AGENTDRAFTER` XDATA).
    if let Some((dyn_block, dyn_state)) = ids::read_dynamic_block(&insert.common) {
        let evaluated_block = (insert.block_name != dyn_block).then(|| insert.block_name.clone());
        return Some((dyn_block, dyn_state, evaluated_block));
    }

    None
}

fn eval_value_to_dynamic(
    value: &BlockEvalValue,
    type_code: i16,
    units: i16,
) -> Option<DynamicValue> {
    match value {
        BlockEvalValue::Text(s) => Some(DynamicValue::Text(s.clone())),
        BlockEvalValue::Short(s) if type_code == 3 => Some(DynamicValue::Bool(*s != 0)),
        BlockEvalValue::Short(s) => Some(DynamicValue::Number(f64::from(*s))),
        BlockEvalValue::Long(l) => Some(DynamicValue::Number(f64::from(*l))),
        BlockEvalValue::Real(r) if r.is_finite() => {
            let val = if units == 1 {
                normalize_degrees(r.to_degrees())
            } else {
                *r
            };
            Some(DynamicValue::Number(val))
        }
        _ => None,
    }
}

/// Caches evaluated anonymous `*U<n>` blocks written during `dwg::write` so
/// identical dynamic block states share one `*U<n>` block record.
pub(super) struct DynamicBlockWriter {
    next_index: u32,
    cache: HashMap<(String, String), (String, Block)>,
}

impl DynamicBlockWriter {
    pub(super) fn new() -> Self {
        Self {
            next_index: 1,
            cache: HashMap::new(),
        }
    }

    /// Writes `insert` to a `DwgInsert`, allocating (or reusing) an evaluated
    /// anonymous `*U<n>` block when `definition` is dynamic and `insert.state`
    /// is non-default.
    pub(super) fn dwg_insert(
        &mut self,
        drawing: &mut CadDocument,
        insert: &Insert,
        definition: &Block,
        all_blocks: &[Block],
        units: i16,
        start_of: &dyn Fn(&TextObject) -> Point3,
    ) -> Result<DwgInsert, Error> {
        if !definition.is_dynamic() || is_default_state(definition, &insert.state) {
            let eval = definition.evaluated(&insert.state);
            return dwg_insert(insert, eval.as_ref(), start_of);
        }
        let state_key = canonical_state_key(&insert.state);
        let cache_key = (definition.name.clone(), state_key);
        let (anon_name, eval_block) = if let Some(hit) = self.cache.get(&cache_key) {
            hit.clone()
        } else {
            let eval = definition.evaluated(&insert.state).into_owned();
            let anon_name = allocate_anon_name(drawing, &mut self.next_index);
            let mut anon_block = eval.clone();
            anon_block.name.clone_from(&anon_name);
            let owner = add_anonymous_block_record(drawing, &anon_block, units)?;
            for obj in &anon_block.objects {
                let mut dwg_obj = dwg_object(obj, start_of, all_blocks)?;
                dwg_obj.common_mut().owner_handle = owner;
                drawing.add_entity(dwg_obj).map_err(|e| {
                    Error::new(
                        ErrorCode::DwgWrite,
                        format!(
                            "could not add a {} to evaluated dynamic block `{anon_name}`: {e}",
                            obj.type_name()
                        ),
                    )
                })?;
            }
            self.cache
                .insert(cache_key, (anon_name.clone(), eval.clone()));
            (anon_name, eval)
        };
        let mut routed = insert.clone();
        routed.block = anon_name;
        dwg_insert(&routed, &eval_block, start_of)
    }
}

fn canonical_state_key(state: &BTreeMap<String, DynamicValue>) -> String {
    state
        .iter()
        .map(|(k, v)| format!("{k}={}", v.encode_xdata()))
        .collect::<Vec<_>>()
        .join("|")
}

fn allocate_anon_name(drawing: &CadDocument, next_index: &mut u32) -> String {
    loop {
        let candidate = format!("*U{}", *next_index);
        *next_index += 1;
        if drawing.block_records.get(&candidate).is_none() {
            return candidate;
        }
    }
}

fn add_anonymous_block_record(
    drawing: &mut CadDocument,
    block: &Block,
    units: i16,
) -> Result<Handle, Error> {
    let handle = add_block_record(drawing, block, units)?;
    if let Some(record) = drawing.block_records.get_mut(&block.name) {
        record.flags.anonymous = true;
    }
    Ok(handle)
}

/// Result of switching a visibility state on an original DWG's dynamic block
/// insert in place.
pub(super) struct VisibilitySwitchResult {
    pub message: String,
    pub added_block_name: String,
    pub added_entity_handles: Vec<Handle>,
    pub updated_xrecord_handle: Option<Handle>,
}

/// Switches the visibility state of an existing `INSERT` backed by an AutoCAD
/// `ACAD_ENHANCEDBLOCK` `BlockVisibilityParameter`, creating a fresh evaluated
/// anonymous `*U<n>` block with the chosen visibility state's entities marked
/// visible/invisible and updating the insert's `ACAD_ENHANCEDBLOCKDATA` cache.
pub(super) fn switch_original_visibility(
    drawing: &mut CadDocument,
    insert_handle: Handle,
    requested_state: &BTreeMap<String, DynamicValue>,
) -> Result<VisibilitySwitchResult, Error> {
    let Some(EntityType::Insert(ins)) = drawing.get_entity(insert_handle) else {
        return Err(Error::new(
            ErrorCode::BadValue,
            "set_block_state works only on an insert",
        ));
    };
    let current_block_name = ins.block_name.clone();
    let current_block_handle = drawing
        .block_records
        .get(&current_block_name)
        .map(|r| r.handle);

    let def_handle = drawing
        .dynamic_definition_for_insert(insert_handle)
        .or_else(|| current_block_handle.and_then(|h| drawing.dynamic_block_definition(h)))
        .ok_or_else(|| {
            Error::new(
                ErrorCode::BadValue,
                format!(
                    "insert `{current_block_name}` is not a dynamic block with an \
                     `ACAD_ENHANCEDBLOCK` evaluation graph"
                ),
            )
        })?;

    let vis = drawing
        .block_visibility_param_for_def(def_handle)
        .cloned()
        .ok_or_else(|| {
            Error::new(
                ErrorCode::BadValue,
                "this dynamic block in the original DWG has no visibility parameter; revising \
                 other dynamic action graphs in place on an external DWG is not supported yet",
            )
        })?;

    if requested_state.len() != 1 {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "revising an original DWG supports setting its visibility parameter `{}` alone",
                vis.name
            ),
        ));
    }
    let (param_name, raw_value) = requested_state.iter().next().expect("len == 1");
    if !param_name.eq_ignore_ascii_case(&vis.name) {
        return Err(Error::new(
            ErrorCode::UnknownName,
            format!(
                "unknown or unsupported dynamic parameter `{param_name}` on this original DWG \
                 insert; its visibility parameter is `{}`",
                vis.name
            ),
        ));
    }
    let DynamicValue::Text(target_state_name) = raw_value else {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "visibility parameter `{}` takes a state name string, not `{}`",
                vis.name,
                raw_value.display_value()
            ),
        ));
    };
    let chosen_state = vis
        .states
        .iter()
        .find(|s| s.name == *target_state_name)
        .ok_or_else(|| {
            let names: Vec<&str> = vis.states.iter().map(|s| s.name.as_str()).collect();
            Error::new(
                ErrorCode::BadValue,
                format!(
                    "unknown visibility state `{target_state_name}` for `{}`; allowed states: {}",
                    vis.name,
                    names.join(", ")
                ),
            )
        })?;

    let def_record = drawing
        .block_records
        .iter()
        .find(|r| r.handle == def_handle)
        .ok_or_else(|| {
            Error::new(
                ErrorCode::UnknownName,
                "could not find the master dynamic block record",
            )
        })?;
    let def_name = def_record.name.clone();
    let def_base = def_record.base_point;
    let def_units = def_record.units;

    let all_controlled: HashSet<Handle> = vis.all_blocks.iter().copied().collect();
    let visible_in_state: HashSet<Handle> = chosen_state.visible_blocks.iter().copied().collect();

    // Clone the master block's entities, flipping `common.invisible` per the
    // chosen visibility state.
    let mut cloned_entities: Vec<EntityType> = Vec::new();
    for entity in drawing.entities_in_block(&def_name) {
        if matches!(entity, EntityType::Block(_) | EntityType::BlockEnd(_)) {
            continue;
        }
        let orig_handle = entity.common().handle;
        let mut copy = entity.clone();
        copy.common_mut().handle = Handle::NULL;
        copy.common_mut().raw_record = None;
        copy.common_mut().reactors.clear();
        copy.common_mut().xdictionary_handle = None;
        if all_controlled.contains(&orig_handle) {
            copy.common_mut().invisible = !visible_in_state.contains(&orig_handle);
        }
        cloned_entities.push(copy);
    }

    // Allocate a brand-new anonymous `*U<n>` block record.
    let mut next_idx = 1_u32;
    let anon_name = allocate_anon_name(drawing, &mut next_idx);
    let mut record = BlockRecord::new(anon_name.clone());
    let anon_handle = drawing.allocate_handle();
    record.set_handle(anon_handle);
    record.block_entity_handle = drawing.allocate_handle();
    record.block_end_handle = drawing.allocate_handle();
    record.base_point = def_base;
    record.units = def_units;
    record.flags.anonymous = true;
    drawing.block_records.add(record).map_err(|e| {
        Error::new(
            ErrorCode::DwgWrite,
            format!("could not add evaluated block `{anon_name}`: {e}"),
        )
    })?;

    // Copy evaluated block metadata (`AcDbBlockRepBTag` XDATA) if the insert's
    // previous block was an evaluated `*U<n>` block so `dynamic_block_definition`
    // continues to resolve the new `*U<n>` back to the master block.
    if let Some(prev_handle) = current_block_handle {
        if prev_handle != def_handle {
            let _ = drawing.copy_evaluated_block_metadata(prev_handle, anon_handle);
        }
    }

    let mut added_entity_handles = Vec::with_capacity(cloned_entities.len());
    for mut entity in cloned_entities {
        entity.common_mut().owner_handle = anon_handle;
        let h = drawing.add_entity(entity).map_err(|e| {
            Error::new(
                ErrorCode::DwgWrite,
                format!("could not add evaluated entity to `{anon_name}`: {e}"),
            )
        })?;
        added_entity_handles.push(h);
    }

    // Point the INSERT at the new evaluated block.
    let has_rep = current_block_handle.is_some_and(|h| h != def_handle)
        && drawing.dynamic_definition_for_insert(insert_handle) == Some(def_handle);
    if !has_rep {
        let _ = super::block::register_application(drawing);
    }
    if let Some(EntityType::Insert(ins)) = drawing.get_entity_mut(insert_handle) {
        ins.block_name.clone_from(&anon_name);
        ins.common.raw_record = None;
        if !has_rep {
            let mut state_map = BTreeMap::new();
            state_map.insert(
                vis.name.clone(),
                DynamicValue::Text(target_state_name.clone()),
            );
            ids::write_dynamic_block(&mut ins.common, &def_name, &state_map);
        }
    }

    // Update the INSERT's saved visibility XRecord in `ACAD_ENHANCEDBLOCKDATA`
    // if present, so AutoCAD and `dynamic_block_properties` report the new state.
    let updated_xrecord_handle = update_saved_visibility_xrecord(
        drawing,
        insert_handle,
        vis.eval_node_id,
        target_state_name,
    );

    Ok(VisibilitySwitchResult {
        message: format!(
            "sets visibility `{}` to `{target_state_name}` (via `{anon_name}`)",
            vis.name
        ),
        added_block_name: anon_name,
        added_entity_handles,
        updated_xrecord_handle,
    })
}

fn update_saved_visibility_xrecord(
    drawing: &mut CadDocument,
    insert_handle: Handle,
    node_id: i32,
    new_state: &str,
) -> Option<Handle> {
    let mut dict = drawing.extension_dictionary_handle(insert_handle)?;
    for key in [
        "AcDbBlockRepresentation",
        "AppDataCache",
        "ACAD_ENHANCEDBLOCKDATA",
    ] {
        let ObjectType::Dictionary(d) = drawing.objects.get(&dict)? else {
            return None;
        };
        dict = d
            .entries
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, h)| *h)?;
    }
    let node_key = node_id.to_string();
    let xrec_handle = {
        let ObjectType::Dictionary(d) = drawing.objects.get(&dict)? else {
            return None;
        };
        d.entries
            .iter()
            .find(|(name, _)| name == &node_key)
            .map(|(_, h)| *h)?
    };
    if let Some(ObjectType::XRecord(xrec)) = drawing.objects.get_mut(&xrec_handle) {
        let mut updated = false;
        for entry in &mut xrec.entries {
            if entry.code == 1 {
                entry.value = acadrust::objects::XRecordValue::String(new_state.to_owned());
                updated = true;
                break;
            }
        }
        if !updated {
            xrec.entries.push(XRecordEntry {
                code: 1,
                value: acadrust::objects::XRecordValue::String(new_state.to_owned()),
            });
        }
        return Some(xrec_handle);
    }
    None
}

/// Helper keeping `vector` and `point` re-exports used across submodules.
#[allow(dead_code)]
fn _point_roundtrip(p: Point3) -> Point3 {
    point(vector(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use acadrust::classes::DxfClass;
    use acadrust::entities::{EntityType, Line as DwgLine};
    use acadrust::objects::{
        BlockEvaluationGraph, BlockEvaluationNode, BlockVisibilityParameter, BlockVisibilityState,
        Dictionary, DynamicBlockData, DynamicBlockObject, XRecord,
    };
    use acadrust::tables::AppId;
    use acadrust::types::{DxfVersion, Vector3};
    use acadrust::DwgWriter;

    /// Builds a genuine AC1032 DWG holding a dynamic block `VALVE` with an
    /// `ACAD_ENHANCEDBLOCK` evaluation graph, a `BlockVisibilityParameter`
    /// (`Visibility1`: `OPEN` vs `CLOSED`), and an `INSERT` with its
    /// `AcDbBlockRepresentation` / `ACAD_ENHANCEDBLOCKDATA` cache set to `OPEN`.
    fn synthetic_autocad_dynamic_dwg() -> Vec<u8> {
        let mut doc = CadDocument::with_version(DxfVersion::AC1032);
        doc.header.insertion_units = 4; // mm
        doc.app_ids.add(AppId::new("AGENTDRAFTER")).unwrap();
        doc.assign_table_entry_handles();

        // Master block `VALVE` holding two lines: line_open (0..10,0) and line_closed (0..0,10).
        let mut valve_rec = BlockRecord::new("VALVE");
        let valve_h = doc.allocate_handle();
        valve_rec.set_handle(valve_h);
        valve_rec.block_entity_handle = doc.allocate_handle();
        valve_rec.block_end_handle = doc.allocate_handle();
        valve_rec.units = 4;
        doc.block_records.add(valve_rec).unwrap();

        let mut line_open = DwgLine::from_points(Vector3::ZERO, Vector3::new(10.0, 0.0, 0.0));
        line_open.common.owner_handle = valve_h;
        let line_open_h = doc.add_entity(EntityType::Line(line_open)).unwrap();

        let mut line_closed = DwgLine::from_points(Vector3::ZERO, Vector3::new(0.0, 10.0, 0.0));
        line_closed.common.owner_handle = valve_h;
        line_closed.common.invisible = true;
        let line_closed_h = doc.add_entity(EntityType::Line(line_closed)).unwrap();

        // Extension dictionary on `VALVE` -> `ACAD_ENHANCEDBLOCK` -> `BlockEvaluationGraph` -> `BlockVisibilityParameter`.
        let xdic_h = doc.ensure_extension_dictionary(valve_h);
        let graph_h = doc.allocate_handle();
        let vis_h = doc.allocate_handle();

        let vis = BlockVisibilityParameter {
            handle: vis_h,
            owner: graph_h,
            eval_node_id: 1,
            eval_value_code: 1,
            eval_value: BlockEvalValue::Text("OPEN".into()),
            show_properties: true,
            name: "Visibility1".into(),
            all_blocks: vec![line_open_h, line_closed_h],
            states: vec![
                BlockVisibilityState {
                    name: "OPEN".into(),
                    visible_blocks: vec![line_open_h],
                    visible_params: Vec::new(),
                },
                BlockVisibilityState {
                    name: "CLOSED".into(),
                    visible_blocks: vec![line_closed_h],
                    visible_params: Vec::new(),
                },
            ],
            ..Default::default()
        };

        let mut graph_obj = DynamicBlockObject::new("ACAD_EVALUATION_GRAPH", "AcDbEvalGraph");
        graph_obj.handle = graph_h;
        graph_obj.owner = xdic_h;
        graph_obj.data = DynamicBlockData::EvaluationGraph(BlockEvaluationGraph {
            first_node_id: 1,
            first_node_id_copy: 1,
            nodes: vec![BlockEvaluationNode {
                id: 1,
                edge_flags: 32,
                next_id: -1,
                expression: vis_h,
                node_data: [0; 4],
                active_cycles: None,
            }],
            edges: Vec::new(),
        });

        if let Some(ObjectType::Dictionary(xdic)) = doc.objects.get_mut(&xdic_h) {
            xdic.entries.push(("ACAD_ENHANCEDBLOCK".into(), graph_h));
        }
        doc.objects
            .insert(graph_h, ObjectType::DynamicBlock(graph_obj));
        doc.objects
            .insert(vis_h, ObjectType::BlockVisibilityParameter(vis));

        // INSERT of `VALVE` at (50, 50, 0) with AcDbBlockRepresentation ->
        // AppDataCache -> ACAD_ENHANCEDBLOCKDATA -> "1" XRecord ("OPEN").
        let dwg_ins = DwgInsert::new("VALVE", Vector3::new(50.0, 50.0, 0.0));
        let ins_h = doc.add_entity(EntityType::Insert(dwg_ins)).unwrap();
        let ins_xdic_h = doc.ensure_extension_dictionary(ins_h);
        let rep_dict_h = doc.allocate_handle();
        let cache_dict_h = doc.allocate_handle();
        let data_dict_h = doc.allocate_handle();
        let xrec_h = doc.allocate_handle();

        if let Some(ObjectType::Dictionary(ins_xdic)) = doc.objects.get_mut(&ins_xdic_h) {
            ins_xdic
                .entries
                .push(("AcDbBlockRepresentation".into(), rep_dict_h));
        }

        let mut rep_dict = Dictionary::new();
        rep_dict.handle = rep_dict_h;
        rep_dict.owner = ins_xdic_h;
        rep_dict.entries.push(("AppDataCache".into(), cache_dict_h));

        let mut cache_dict = Dictionary::new();
        cache_dict.handle = cache_dict_h;
        cache_dict.owner = rep_dict_h;
        cache_dict
            .entries
            .push(("ACAD_ENHANCEDBLOCKDATA".into(), data_dict_h));

        let mut data_dict = Dictionary::new();
        data_dict.handle = data_dict_h;
        data_dict.owner = cache_dict_h;
        data_dict.entries.push(("1".into(), xrec_h));

        let mut xrec = XRecord::new();
        xrec.handle = xrec_h;
        xrec.owner = data_dict_h;
        xrec.entries.push(XRecordEntry {
            code: 1,
            value: acadrust::objects::XRecordValue::String("OPEN".into()),
        });

        doc.objects
            .insert(rep_dict_h, ObjectType::Dictionary(rep_dict));
        doc.objects
            .insert(cache_dict_h, ObjectType::Dictionary(cache_dict));
        doc.objects
            .insert(data_dict_h, ObjectType::Dictionary(data_dict));
        doc.objects.insert(xrec_h, ObjectType::XRecord(xrec));

        // Register required classes for DWG round-trip.
        doc.classes
            .add_or_update(DxfClass::new("ACAD_EVALUATION_GRAPH", "AcDbEvalGraph"));
        doc.classes.add_or_update(DxfClass::new(
            "BLOCKVISIBILITYPARAMETER",
            "AcDbBlockVisibilityParameter",
        ));

        DwgWriter::write_to_vec(&doc).expect("write synthetic dynamic DWG")
    }

    #[test]
    fn native_autocad_dynamic_block_reads_preserves_and_switches_visibility_state() {
        let bytes = synthetic_autocad_dynamic_dwg();
        let path = std::path::Path::new("plant-valve.dwg");

        // 1. Read back: dynamic block `VALVE` reads cleanly, skips hidden line,
        //    and exposes `Visibility1: OPEN` and its External parameter metadata.
        let contents = super::super::read_bytes(&bytes, path).expect("read dynamic DWG");
        assert_eq!(
            contents.objects_not_read, 0,
            "not_read: {:?}",
            contents.not_read
        );
        assert_eq!(contents.objects.len(), 1);
        let crate::model::Object::Insert(ins) = &contents.objects[0] else {
            panic!("expected insert");
        };
        assert_eq!(ins.block, "VALVE");
        assert_eq!(
            ins.state.get("Visibility1"),
            Some(&DynamicValue::Text("OPEN".into()))
        );
        let valve_block = contents
            .blocks
            .iter()
            .find(|b| b.name == "VALVE")
            .expect("VALVE read");
        assert_eq!(
            valve_block.objects.len(),
            1,
            "hidden CLOSED line inside dynamic VALVE must be skipped"
        );
        assert!(
            valve_block
                .parameters
                .iter()
                .any(|p| p.name == "Visibility1"),
            "master dynamic block should expose Visibility1 parameter: {:?}",
            valve_block.parameters
        );

        // 2. Switch visibility state from `OPEN` to `CLOSED` in place through
        //    RevisionSession: 0 unexpected changes and revised DWG reads `CLOSED`!
        let ins_hex = contents.handles[0].clone();
        let mut session = super::super::RevisionSession::open(&bytes, path).unwrap();
        let new_state = BTreeMap::from([(
            "Visibility1".to_owned(),
            DynamicValue::Text("CLOSED".into()),
        )]);
        let (msg, handles) = session.set_block_state(&ins_hex, &new_state).unwrap();
        assert!(msg.contains("CLOSED"), "{msg}");
        assert_eq!(handles.touched, vec![ins_hex]);
        let verified = session.serialize_and_verify(path).unwrap();
        assert!(
            verified.requested_failures.is_empty(),
            "requested_failures: {:#?}",
            verified.requested_failures
        );
        assert!(
            verified.unexpected_changes.is_empty(),
            "unexpected_changes: {:#?}",
            verified.unexpected_changes
        );

        // Re-read revised DWG and confirm visibility switched to CLOSED!
        let after = super::super::read_bytes(&verified.bytes, path).unwrap();
        let crate::model::Object::Insert(after_ins) = &after.objects[0] else {
            panic!("expected insert after revision");
        };
        assert_eq!(after_ins.block, "VALVE");
        assert_eq!(
            after_ins.state.get("Visibility1"),
            Some(&DynamicValue::Text("CLOSED".into()))
        );
        let eval_name = after_ins.evaluated_block.as_deref().unwrap();
        let eval_blk = after.blocks.iter().find(|b| b.name == eval_name).unwrap();
        assert_eq!(eval_blk.objects.len(), 1);
        let crate::model::Object::Line(l) = &eval_blk.objects[0] else {
            panic!("expected visible CLOSED line");
        };
        assert!(
            l.to.x.abs() < 1e-9 && (l.to.y - 10.0).abs() < 1e-9,
            "CLOSED line should point up to (0, 10): {l:?}"
        );
    }
}
