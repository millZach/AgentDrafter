//! External reference resolution: loading attached and overlaid DWG/DXF files
//! from disk so the one drawing pipeline (ADR-0011), queries and checks draw
//! and inspect them without embedding their geometry into built DWGs.
//!
//! Resolution follows AutoCAD's rules:
//! - Both `attach` and `overlay` xrefs declared on the host drawing are loaded
//!   unless marked `unloaded`.
//! - Inside a referenced drawing (depth > 0), only `attach` xrefs are followed;
//!   nested `overlay` xrefs are skipped.
//! - Paths are resolved relative to the host file's folder (normalising `\` to
//!   `/`), with a fallback to the plain file name beside the host file when a
//!   saved path moved.
//! - Circular reference chains are broken and reported as unresolved.
//! - Xref-dependent layers, linetypes, text styles, table styles, multileader
//!   styles, dimension styles and nested blocks are prefixed with `<xref>|`
//!   (leaving built-in `0`, `Continuous`, `ByLayer`, `ByBlock` and `Standard`
//!   unprefixed so layer `0` and `ByBlock` inherit from the host insert).
//!   Existing host entries with the prefixed name (AutoCAD `VISRETAIN`
//!   overrides) are preserved rather than overwritten.
//! - If the xref's declared unit differs from the host's unit, its model-space
//!   geometry is scaled by the unit conversion ratio.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::dwg;
use crate::model::{
    Block, DrawingContents, Insert, Layer, Linetype, Object, ObjectLinetype, Point3, Properties,
    Sheet, UnresolvedXref, Xref, XrefKind, BUILT_IN_LINETYPES, LAYER_ZERO,
};
use crate::stamp::SourceKind;
use crate::text::{TextStyle, STANDARD};
use crate::units::Unit;

/// Maximum nesting depth for chained attached xrefs before refusing as cyclic.
const MAX_XREF_DEPTH: usize = 32;

/// What [`resolve_sheet_xrefs`] found on disk.
#[derive(Clone, Debug, Default)]
pub(crate) struct ResolvedXrefs {
    /// Every external reference file read from disk, for output stamps so
    /// `stale` detects when any referenced drawing changes.
    pub files: Vec<(SourceKind, PathBuf, Vec<u8>)>,
    /// External references that could not be found, could not be read, or
    /// formed a circular reference.
    pub unresolved: Vec<UnresolvedXref>,
}

/// Resolves every non-unloaded external reference declared on `sheet`, looking
/// relative to `host_path`, populates `sheet.resolved_xrefs` and merges
/// xref-dependent layers, linetypes, text styles and styles into `sheet`.
///
/// Unresolved xrefs are collected in [`ResolvedXrefs::unresolved`] rather than
/// failing the load, so the rest of the host drawing still draws, queries and
/// reports clearly which references were missing or unreadable.
pub(crate) fn resolve_sheet_xrefs(sheet: &mut Sheet, host_path: &Path) -> ResolvedXrefs {
    let mut out = ResolvedXrefs::default();
    if sheet.xrefs.is_empty() {
        return out;
    }
    let mut visited = HashSet::new();
    visited.insert(canonical_key(host_path));
    let xrefs = sheet.xrefs.clone();
    let host_units = sheet.units;
    for xref in &xrefs {
        if xref.unloaded {
            continue;
        }
        resolve_one(
            sheet,
            host_path,
            host_units,
            xref,
            &xref.name,
            0,
            &mut visited,
            &mut out,
        );
    }
    out
}

/// Resolves the xrefs of a [`DrawingContents`] without mutating it (used by
/// `agentdrafter read` to report which xrefs can and cannot be found/loaded).
pub(crate) fn check_drawing_xrefs(
    drawing: &DrawingContents,
    host_path: &Path,
) -> Vec<UnresolvedXref> {
    if drawing.xrefs.is_empty() {
        return Vec::new();
    }
    let mut sheet = Sheet {
        units: drawing.units.unwrap_or(Unit::Mm),
        block_library: None,
        linetypes: drawing.linetypes.clone(),
        layers: drawing.layers.clone(),
        text_styles: drawing.text_styles.clone(),
        table_styles: drawing.table_styles.clone(),
        mleader_styles: drawing.mleader_styles.clone(),
        dimension_styles: drawing.dimension_styles.clone(),
        blocks: drawing.blocks.clone(),
        xrefs: drawing.xrefs.clone(),
        resolved_xrefs: Vec::new(),
        resolved_media: Vec::new(),
        objects: Vec::new(),
        paper: None,
        page_setup: None,
        ltscale: drawing.ltscale,
        layout: None,
        other_layouts: Vec::new(),
        wipeout_frame: drawing.wipeout_frame,
        image_frame: drawing.image_frame,
        custom_properties: drawing.custom_properties.clone(),
        groups: drawing.groups.clone(),
    };
    resolve_sheet_xrefs(&mut sheet, host_path).unresolved
}

/// Looks up the on-disk file for an xref's saved `saved_path` relative to
/// `host_path`.
///
/// Resolution order (matching AutoCAD):
/// 1. Direct path (if absolute and exists) or joined onto `host_path.parent()`
///    after normalising `\` separators to `/`.
/// 2. Plain file name of `saved_path` in `host_path.parent()` (when the saved
///    path came from another folder or machine).
pub(crate) fn find_xref_file(host_path: &Path, saved_path: &str) -> Option<PathBuf> {
    let trimmed = saved_path.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized = trimmed.replace('\\', "/");
    let host_dir = host_path.parent().unwrap_or_else(|| Path::new(""));

    // 1. Absolute path as written, or relative to host directory.
    let raw = Path::new(trimmed);
    if raw.is_absolute() && raw.is_file() {
        return Some(raw.to_path_buf());
    }
    let norm_path = Path::new(&normalized);
    if norm_path.is_absolute() && norm_path.is_file() {
        return Some(norm_path.to_path_buf());
    }
    let joined = host_dir.join(norm_path);
    if joined.is_file() {
        return Some(joined);
    }

    // 2. Fallback: file name beside the host drawing (e.g. saved Windows path
    //    `C:\Projects\grid.dwg` opened on Linux or moved into the same folder).
    let file_name = normalized
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())?;
    let beside = host_dir.join(file_name);
    if beside.is_file() {
        return Some(beside);
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn resolve_one(
    host: &mut Sheet,
    parent_file: &Path,
    host_units: Unit,
    xref: &Xref,
    block_name: &str,
    depth: usize,
    visited: &mut HashSet<PathBuf>,
    out: &mut ResolvedXrefs,
) {
    if depth >= MAX_XREF_DEPTH {
        out.unresolved.push(UnresolvedXref {
            name: xref.name.clone(),
            path: xref.path.clone(),
            kind: xref.kind,
            reason: format!("external reference nesting exceeded {MAX_XREF_DEPTH} levels"),
        });
        return;
    }
    let Some(resolved_path) = find_xref_file(parent_file, &xref.path) else {
        out.unresolved.push(UnresolvedXref {
            name: xref.name.clone(),
            path: xref.path.clone(),
            kind: xref.kind,
            reason: "file not found".to_owned(),
        });
        return;
    };
    let key = canonical_key(&resolved_path);
    if !visited.insert(key.clone()) {
        out.unresolved.push(UnresolvedXref {
            name: xref.name.clone(),
            path: xref.path.clone(),
            kind: xref.kind,
            reason: "circular external reference".to_owned(),
        });
        return;
    }
    let bytes = match fs::read(&resolved_path) {
        Ok(bytes) => bytes,
        Err(e) => {
            visited.remove(&key);
            out.unresolved.push(UnresolvedXref {
                name: xref.name.clone(),
                path: xref.path.clone(),
                kind: xref.kind,
                reason: format!("could not read file: {e}"),
            });
            return;
        }
    };
    let drawing = match dwg::read_bytes(&bytes, &resolved_path) {
        Ok(drawing) => drawing,
        Err(e) => {
            visited.remove(&key);
            out.unresolved.push(UnresolvedXref {
                name: xref.name.clone(),
                path: xref.path.clone(),
                kind: xref.kind,
                reason: format!("could not read drawing: {}", e.message),
            });
            return;
        }
    };

    // Record the file once in `out.files` for output stamping.
    if !out.files.iter().any(|(_, p, _)| canonical_key(p) == key) {
        out.files
            .push((SourceKind::Xref, resolved_path.clone(), bytes));
    }

    let prefix = format!("{block_name}|");
    let xref_units = drawing.units.unwrap_or(host_units);
    let unit_scale = xref_units.convert(1.0, host_units);

    // 1. Merge prefixed linetypes, layers, text styles, table styles,
    //    mleader styles and dimension styles into the host sheet, keeping any
    //    host override (`VISRETAIN`) that already exists.
    merge_xref_tables(host, &drawing, &prefix, unit_scale);

    // 2. Recursively resolve nested ATTACH xrefs (skipping nested OVERLAYs,
    //    exactly as AutoCAD does).
    let mut nested_xref_map: Vec<(String, String)> = Vec::new();
    for nested in &drawing.xrefs {
        if nested.unloaded || nested.kind == XrefKind::Overlay {
            continue;
        }
        let nested_block_name = format!("{prefix}{}", nested.name);
        nested_xref_map.push((nested.name.clone(), nested_block_name.clone()));
        resolve_one(
            host,
            &resolved_path,
            xref_units,
            nested,
            &nested_block_name,
            depth + 1,
            visited,
            out,
        );
    }

    // 3. Prefix and add the xref's own block definitions to `host.resolved_xrefs`.
    let block_renames: Vec<(String, String)> = drawing
        .blocks
        .iter()
        .map(|b| (b.name.clone(), format!("{prefix}{}", b.name)))
        .chain(nested_xref_map)
        .collect();
    let model_objects = model_space_objects_of(&drawing);
    for mut block in drawing.blocks {
        block.name = format!("{prefix}{}", block.name);
        for object in &mut block.objects {
            prefix_object(object, &prefix, &block_renames);
        }
        push_resolved_block(host, block);
    }

    // 4. Collect the xref's model-space objects (excluding paper-space objects
    //    and viewports, since AutoCAD xrefs display model space only).
    let mut prefixed_objects: Vec<Object> = model_objects
        .into_iter()
        .filter(|o| !matches!(o, Object::Viewport(_)))
        .collect();
    for object in &mut prefixed_objects {
        prefix_object(object, &prefix, &block_renames);
    }

    // 5. If the xref's unit differs from the host's unit, wrap its model-space
    //    objects in an inner block placed at the unit conversion scale so every
    //    object type (lines, arcs, text, hatches, nested blocks) scales cleanly.
    if (unit_scale - 1.0).abs() > 1e-12 && unit_scale.is_finite() && unit_scale > 0.0 {
        let inner_name = format!("{prefix}__model");
        push_resolved_block(
            host,
            Block {
                name: inner_name.clone(),
                base_point: Point3::new(0.0, 0.0, 0.0),
                objects: prefixed_objects,
                cells: Vec::new(),
                parameters: Vec::new(),
            },
        );
        let wrapper_insert = Object::Insert(Insert {
            block: inner_name,
            position: Point3::new(0.0, 0.0, 0.0),
            scale: unit_scale,
            rotation: 0.0,
            mirrored: false,
            readable_attributes: false,
            attributes: Vec::new(),
            state: std::collections::BTreeMap::new(),
            evaluated_block: None,
            name: None,
            layer: LAYER_ZERO.to_owned(),
            properties: Properties::default(),
        });
        push_resolved_block(
            host,
            Block {
                name: block_name.to_owned(),
                base_point: Point3::new(0.0, 0.0, 0.0),
                objects: vec![wrapper_insert],
                cells: Vec::new(),
                parameters: Vec::new(),
            },
        );
    } else {
        push_resolved_block(
            host,
            Block {
                name: block_name.to_owned(),
                base_point: Point3::new(0.0, 0.0, 0.0),
                objects: prefixed_objects,
                cells: Vec::new(),
                parameters: Vec::new(),
            },
        );
    }

    visited.remove(&key);
}

/// Extracts only the model-space objects from `drawing` (excluding objects
/// belonging to any paper-space layout).
pub(crate) fn model_space_objects_of(drawing: &DrawingContents) -> Vec<Object> {
    let layouts: Vec<&crate::model::Layout> = drawing
        .layout
        .iter()
        .chain(&drawing.other_layouts)
        .collect();
    drawing
        .objects
        .iter()
        .enumerate()
        .filter(|&(i, _)| !layouts.iter().any(|l| l.holds(i)))
        .map(|(_, o)| o.clone())
        .collect()
}

fn push_resolved_block(host: &mut Sheet, block: Block) {
    if !host
        .resolved_xrefs
        .iter()
        .any(|b| b.name.eq_ignore_ascii_case(&block.name))
    {
        host.resolved_xrefs.push(block);
    }
}

fn merge_xref_tables(host: &mut Sheet, drawing: &DrawingContents, prefix: &str, unit_scale: f64) {
    for lt in &drawing.linetypes {
        if is_built_in_linetype(&lt.name) {
            continue;
        }
        let name = format!("{prefix}{}", lt.name);
        if !host
            .linetypes
            .iter()
            .any(|existing| existing.name.eq_ignore_ascii_case(&name))
        {
            host.linetypes.push(Linetype {
                name,
                description: lt.description.clone(),
                pattern: lt.pattern.iter().map(|d| d * unit_scale).collect(),
            });
        }
    }
    for layer in &drawing.layers {
        if layer.name == LAYER_ZERO {
            continue;
        }
        let name = format!("{prefix}{}", layer.name);
        if !host
            .layers
            .iter()
            .any(|existing| existing.name.eq_ignore_ascii_case(&name))
        {
            let linetype = prefix_linetype_name(&layer.linetype, prefix);
            host.layers.push(Layer {
                name,
                color: layer.color,
                linetype,
                lineweight: layer.lineweight,
                on: layer.on,
                frozen: layer.frozen,
                locked: layer.locked,
                plot: layer.plot,
                description: layer.description.clone(),
                plot_style: layer.plot_style.clone(),
                transparency: layer.transparency,
            });
        }
    }
    for style in &drawing.text_styles {
        if style.name.eq_ignore_ascii_case(STANDARD) {
            continue;
        }
        let name = format!("{prefix}{}", style.name);
        if !host
            .text_styles
            .iter()
            .any(|existing| existing.name.eq_ignore_ascii_case(&name))
        {
            host.text_styles.push(TextStyle {
                name,
                font: style.font.clone(),
                width_factor: style.width_factor,
            });
        }
    }
    for ts in &drawing.table_styles {
        let name = format!("{prefix}{}", ts.name);
        if !host
            .table_styles
            .iter()
            .any(|existing| existing.name.eq_ignore_ascii_case(&name))
        {
            let mut cloned = ts.clone();
            cloned.name = name;
            cloned.title.text_style = prefix_style_name(&cloned.title.text_style, prefix);
            cloned.header.text_style = prefix_style_name(&cloned.header.text_style, prefix);
            cloned.data.text_style = prefix_style_name(&cloned.data.text_style, prefix);
            host.table_styles.push(cloned);
        }
    }
    for ms in &drawing.mleader_styles {
        let name = format!("{prefix}{}", ms.name);
        if !host
            .mleader_styles
            .iter()
            .any(|existing| existing.name.eq_ignore_ascii_case(&name))
        {
            let mut cloned = ms.clone();
            cloned.name = name;
            cloned.text_style = prefix_style_name(&cloned.text_style, prefix);
            host.mleader_styles.push(cloned);
        }
    }
    for ds in &drawing.dimension_styles {
        let name = format!("{prefix}{}", ds.name);
        if !host
            .dimension_styles
            .iter()
            .any(|existing| existing.name.eq_ignore_ascii_case(&name))
        {
            let mut cloned = ds.clone();
            cloned.name = name;
            cloned.text_style = prefix_style_name(&cloned.text_style, prefix);
            host.dimension_styles.push(cloned);
        }
    }
}

pub(crate) fn is_built_in_linetype(name: &str) -> bool {
    BUILT_IN_LINETYPES
        .iter()
        .any(|b| b.eq_ignore_ascii_case(name))
}

pub(crate) fn prefix_layer_name(layer: &str, prefix: &str) -> String {
    if layer == LAYER_ZERO {
        layer.to_owned()
    } else {
        format!("{prefix}{layer}")
    }
}

pub(crate) fn prefix_linetype_name(linetype: &str, prefix: &str) -> String {
    if is_built_in_linetype(linetype) {
        linetype.to_owned()
    } else {
        format!("{prefix}{linetype}")
    }
}

pub(crate) fn prefix_style_name(style: &str, prefix: &str) -> String {
    if style.eq_ignore_ascii_case(STANDARD) {
        style.to_owned()
    } else {
        format!("{prefix}{style}")
    }
}

fn prefix_properties(props: &mut Properties, prefix: &str) {
    if let ObjectLinetype::Named(name) = &mut props.linetype {
        if !is_built_in_linetype(name) {
            *name = format!("{prefix}{name}");
        }
    }
}

/// Rewrites layer, linetype, text style, style and block references on `object`
/// using `prefix` and `block_renames`. Used both for runtime xref resolution
/// (`<xref>|`) and for `bind_xref` (`<xref>$0$` in `Bind` mode).
pub(crate) fn prefix_object(object: &mut Object, prefix: &str, block_renames: &[(String, String)]) {
    if prefix.is_empty() && block_renames.is_empty() {
        return;
    }
    let rename_block = |name: &str| -> String {
        block_renames
            .iter()
            .find(|(old, _)| old.eq_ignore_ascii_case(name))
            .map(|(_, new)| new.clone())
            .unwrap_or_else(|| {
                if prefix.is_empty() {
                    name.to_owned()
                } else {
                    format!("{prefix}{name}")
                }
            })
    };
    let rewrite_layer = |layer: &mut String| {
        if !prefix.is_empty() {
            *layer = prefix_layer_name(layer, prefix);
        }
    };
    let rewrite_style = |style: &mut String| {
        if !prefix.is_empty() {
            *style = prefix_style_name(style, prefix);
        }
    };
    match object {
        Object::Line(o) => {
            rewrite_layer(&mut o.layer);
            prefix_properties(&mut o.properties, prefix);
        }
        Object::Polyline(o) => {
            rewrite_layer(&mut o.layer);
            prefix_properties(&mut o.properties, prefix);
        }
        Object::Arc(o) => {
            rewrite_layer(&mut o.layer);
            prefix_properties(&mut o.properties, prefix);
        }
        Object::Circle(o) => {
            rewrite_layer(&mut o.layer);
            prefix_properties(&mut o.properties, prefix);
        }
        Object::Ellipse(o) => {
            rewrite_layer(&mut o.layer);
            prefix_properties(&mut o.properties, prefix);
        }
        Object::Spline(o) => {
            rewrite_layer(&mut o.layer);
            prefix_properties(&mut o.properties, prefix);
        }
        Object::Hatch(o) => {
            rewrite_layer(&mut o.layer);
            prefix_properties(&mut o.properties, prefix);
        }
        Object::Text(o) => {
            rewrite_layer(&mut o.layer);
            rewrite_style(&mut o.style);
            prefix_properties(&mut o.properties, prefix);
        }
        Object::MText(o) => {
            rewrite_layer(&mut o.layer);
            rewrite_style(&mut o.style);
            prefix_properties(&mut o.properties, prefix);
        }
        Object::Donut(o) => {
            rewrite_layer(&mut o.layer);
            prefix_properties(&mut o.properties, prefix);
        }
        Object::Solid(o) => {
            rewrite_layer(&mut o.layer);
            prefix_properties(&mut o.properties, prefix);
        }
        Object::RevisionCloud(o) => {
            rewrite_layer(&mut o.layer);
            prefix_properties(&mut o.properties, prefix);
        }
        Object::Wipeout(o) => {
            rewrite_layer(&mut o.layer);
            prefix_properties(&mut o.properties, prefix);
        }
        Object::RasterImage(o) => {
            rewrite_layer(&mut o.layer);
            prefix_properties(&mut o.properties, prefix);
        }
        Object::PdfUnderlay(o) => {
            rewrite_layer(&mut o.layer);
            prefix_properties(&mut o.properties, prefix);
        }
        Object::AttributeDefinition(o) => {
            rewrite_layer(&mut o.layer);
            rewrite_style(&mut o.style);
            prefix_properties(&mut o.properties, prefix);
        }
        Object::Insert(o) => {
            rewrite_layer(&mut o.layer);
            prefix_properties(&mut o.properties, prefix);
            o.block = rename_block(&o.block);
            if let Some(eval) = &mut o.evaluated_block {
                *eval = rename_block(eval);
            }
        }
        Object::Table(o) => {
            rewrite_layer(&mut o.layer);
            prefix_properties(&mut o.properties, prefix);
            if !prefix.is_empty() {
                o.table_style = format!("{prefix}{}", o.table_style);
            }
        }
        Object::MLeader(o) => {
            rewrite_layer(&mut o.layer);
            prefix_properties(&mut o.properties, prefix);
            if !prefix.is_empty() {
                o.mleader_style = format!("{prefix}{}", o.mleader_style);
            }
            if let crate::model::MLeaderContent::Block(b) = &mut o.content {
                b.block = rename_block(&b.block);
            }
        }
        Object::Leader(o) => {
            rewrite_layer(&mut o.layer);
            prefix_properties(&mut o.properties, prefix);
            if let Some(ann) = &mut o.annotation {
                rewrite_style(&mut ann.style);
            }
        }
        Object::Dimension(o) => {
            rewrite_layer(&mut o.layer);
            prefix_properties(&mut o.properties, prefix);
            if !prefix.is_empty() {
                o.dimension_style = format!("{prefix}{}", o.dimension_style);
            }
        }
        Object::Viewport(o) => {
            rewrite_layer(&mut o.layer);
            prefix_properties(&mut o.properties, prefix);
        }
    }
}

fn canonical_key(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
