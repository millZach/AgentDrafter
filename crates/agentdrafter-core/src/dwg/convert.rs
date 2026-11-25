//! DWG and DXF version conversion behind the `dwg` seam (ADR-0004, ADR-0006,
//! ADR-0009).
//!
//! Converts an open `CadDocument` to any DWG version acadrust writes
//! (`AC1009`..`AC1032`), clears version-specific raw bitstreams so every
//! record is cleanly re-encoded for the target version, analyzes features
//! unsupported by older target versions, and re-reads the written bytes to
//! report any structural losses.

use std::collections::HashSet;
use std::path::Path;

use acadrust::entities::EntityType;
use acadrust::objects::ObjectType;
use acadrust::types::{Color, DxfVersion, LineWeight};
use acadrust::CadDocument;

use super::revise::{check_decode_regression, snapshot_drawing, DrawingSnapshot};
use super::{
    contents, detect_drawing_format, handle_text, kind_of, raw_dwg_version,
    read_drawing_bytes_with_stats, write_document, BUILD_MARK,
};
use crate::build::DrawingFormat;
use crate::convert::{ConversionLoss, ConversionLossKind, DwgVersion};
use crate::error::{Error, ErrorCode};
use crate::model::DecodeReport;

/// Converts [`DwgVersion`] to acadrust's [`DxfVersion`].
pub(super) const fn to_dxf_version(version: DwgVersion) -> DxfVersion {
    match version {
        DwgVersion::R12 => DxfVersion::AC1009,
        DwgVersion::R13 => DxfVersion::AC1012,
        DwgVersion::R14 => DxfVersion::AC1014,
        DwgVersion::R2000 => DxfVersion::AC1015,
        DwgVersion::R2004 => DxfVersion::AC1018,
        DwgVersion::R2007 => DxfVersion::AC1021,
        DwgVersion::R2010 => DxfVersion::AC1024,
        DwgVersion::R2013 => DxfVersion::AC1027,
        DwgVersion::R2018 => DxfVersion::AC1032,
    }
}

/// Result of converting a drawing in memory, including post-write verification.
pub(crate) struct ConvertedBytes {
    pub bytes: Vec<u8>,
    pub source_format: DrawingFormat,
    pub source_version: String,
    pub total_objects: usize,
    pub unchanged_objects: usize,
    pub losses: Vec<ConversionLoss>,
    pub decode: DecodeReport,
    pub output_decode: DecodeReport,
}

/// Converts raw DWG or DXF bytes to `target_version` in `target_format`,
/// checking decode completeness (refusing unless `allow_loss` is true),
/// analyzing version-capability downgrades, re-reading the written bytes, and
/// reporting any dropped, downgraded, or changed objects.
pub(crate) fn convert_drawing_bytes(
    bytes: &[u8],
    source_path: &Path,
    target_version: DwgVersion,
    target_format: DrawingFormat,
    allow_loss: bool,
) -> Result<ConvertedBytes, Error> {
    let source_format = detect_drawing_format(bytes, source_path);
    let raw_version = if source_format.is_dwg() {
        raw_dwg_version(bytes)
    } else {
        None
    };

    let (mut drawing, stats, read_past_damage) =
        match read_drawing_bytes_with_stats(bytes, source_path, false) {
            Ok((drawing, stats)) => (drawing, stats, None),
            Err(strict_err) => {
                if raw_version.is_none() && !source_format.is_dxf() {
                    return Err(strict_err);
                }
                match read_drawing_bytes_with_stats(bytes, source_path, true) {
                    Ok((drawing, stats)) => (drawing, stats, Some(strict_err.message)),
                    Err(_) => return Err(strict_err),
                }
            }
        };

    let mut parsed_contents = contents(&drawing, &stats, raw_version.clone())?;
    if let Some(reason) = read_past_damage {
        parsed_contents.decode.decoded = false;
        parsed_contents.decode.read_past_damage = Some(reason);
    }
    let decode = parsed_contents.decode;
    let source_version = parsed_contents.dwg_version;

    if !decode.decoded && !allow_loss {
        let skipped = decode.skipped;
        let recovered = decode.recovered_errors;
        let not_decoded = decode.not_decoded.len();
        return Err(Error::new(
            ErrorCode::IncompleteDecode,
            format!(
                "refusing to convert `{}` because the DWG library did not decode every record \
                 cleanly (skipped: {skipped}, recovered errors: {recovered}, undecoded kinds: \
                 {not_decoded}); pass `allow_loss` (`--allow-loss`) to convert anyway (ADR-0004)",
                source_path.display()
            ),
        ));
    }

    let before = snapshot_drawing(&drawing, &source_version, &decode);
    let mut losses = analyze_version_losses(&drawing, target_version, target_format);

    let target_dxf = to_dxf_version(target_version);
    let version_changed = drawing.version != target_dxf || source_format != target_format;
    prepare_for_version(&mut drawing, target_version, version_changed);

    let out_bytes = write_document(&drawing, target_format)?;
    if target_format.is_dwg() {
        let expected = target_version.code().as_bytes();
        if out_bytes.get(..6) != Some(expected) {
            let actual = out_bytes
                .get(..6)
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .unwrap_or_else(|| "<truncated>".to_owned());
            return Err(Error::new(
                ErrorCode::DwgWrite,
                format!(
                    "expected converted DWG header `{}` but writer produced `{actual}`",
                    target_version.code()
                ),
            ));
        }
    }

    // Re-read the converted output to verify decode health and structural fidelity.
    let virtual_out = Path::new(if target_format.is_dxf() {
        "converted.dxf"
    } else {
        "converted.dwg"
    });
    let out_raw_ver = if target_format.is_dwg() {
        raw_dwg_version(&out_bytes)
    } else {
        None
    };
    let (after_doc, after_stats, out_damage) =
        match read_drawing_bytes_with_stats(&out_bytes, virtual_out, false) {
            Ok((doc, st)) => (doc, st, None),
            Err(err) => match read_drawing_bytes_with_stats(&out_bytes, virtual_out, true) {
                Ok((doc, st)) => (doc, st, Some(err.message)),
                Err(_) => {
                    return Err(Error::new(
                        ErrorCode::DwgWrite,
                        format!(
                            "the converted {} ({}) could not be read back: {}",
                            if target_format.is_dxf() { "DXF" } else { "DWG" },
                            target_version.code(),
                            err.message
                        ),
                    ));
                }
            },
        };
    let mut after_contents = contents(&after_doc, &after_stats, out_raw_ver)?;
    if let Some(reason) = out_damage {
        after_contents.decode.decoded = false;
        after_contents.decode.read_past_damage = Some(reason);
    }
    let output_decode = after_contents.decode;
    let after = snapshot_drawing(&after_doc, target_version.code(), &output_decode);

    let (total_objects, unchanged_objects) =
        diff_converted_snapshots(&before, &after, target_version, &mut losses);

    Ok(ConvertedBytes {
        bytes: out_bytes,
        source_format,
        source_version,
        total_objects,
        unchanged_objects,
        losses,
        decode,
        output_decode,
    })
}

/// Serializes a freshly constructed `CadDocument` (built at 2018) in
/// `target_version` and `target_format`, returning its bytes and any version
/// conversion losses when `target_version != DwgVersion::R2018`.
pub(super) fn finish_new_sheet_in_version(
    mut drawing: CadDocument,
    target_version: DwgVersion,
    target_format: DrawingFormat,
    build_mark_value: &str,
) -> Result<(Vec<u8>, Vec<ConversionLoss>), Error> {
    if target_version == DwgVersion::R2018 {
        let bytes = write_document(&drawing, target_format)?;
        return Ok((bytes, Vec::new()));
    }

    let clean_decode = DecodeReport {
        decoded: true,
        not_decoded: Vec::new(),
        skipped: 0,
        recovered_errors: 0,
        warnings: 0,
        diagnostics_truncated: false,
        read_past_damage: None,
    };
    let before = snapshot_drawing(&drawing, "AC1032", &clean_decode);
    let mut losses = analyze_version_losses(&drawing, target_version, target_format);
    prepare_for_version(&mut drawing, target_version, false);

    // Pre-R2004 DWG files have no `AcDb:SummaryInfo` section; record the build
    // mark in an `XRecord` under the Named Object Dictionary on R13/R14/2000 so
    // `build --overwrite` and `change` still recognize AgentDrafter's own builds.
    if target_format.is_dwg()
        && target_version < DwgVersion::R2004
        && target_version >= DwgVersion::R13
    {
        write_nod_build_mark(&mut drawing, build_mark_value);
    }

    let out_bytes = write_document(&drawing, target_format)?;
    if target_format.is_dwg() {
        let expected = target_version.code().as_bytes();
        if out_bytes.get(..6) != Some(expected) {
            let actual = out_bytes
                .get(..6)
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .unwrap_or_else(|| "<truncated>".to_owned());
            return Err(Error::new(
                ErrorCode::DwgWrite,
                format!(
                    "expected built DWG header `{}` but writer produced `{actual}`",
                    target_version.code()
                ),
            ));
        }
    }

    // Re-read the built output to catch any entities/tables dropped or altered
    // by the older format.
    let virtual_out = Path::new(if target_format.is_dxf() {
        "built.dxf"
    } else {
        "built.dwg"
    });
    let out_raw_ver = if target_format.is_dwg() {
        raw_dwg_version(&out_bytes)
    } else {
        None
    };
    if let Ok((after_doc, after_stats)) =
        read_drawing_bytes_with_stats(&out_bytes, virtual_out, false)
    {
        if let Ok(after_contents) = contents(&after_doc, &after_stats, out_raw_ver) {
            let after = snapshot_drawing(&after_doc, target_version.code(), &after_contents.decode);
            let _ = diff_converted_snapshots(&before, &after, target_version, &mut losses);
        }
    }

    Ok((out_bytes, losses))
}

/// Named Object Dictionary key for the fallback build mark on pre-R2004 DWGs
/// (which lack `AcDb:SummaryInfo`).
pub(super) const NOD_BUILD_MARK_KEY: &str = "AGENTDRAFTER_BUILD";

fn root_dict_handle(drawing: &CadDocument) -> Option<acadrust::types::Handle> {
    let root = drawing.header.named_objects_dict_handle;
    if !root.is_null() && matches!(drawing.objects.get(&root), Some(ObjectType::Dictionary(_))) {
        return Some(root);
    }
    drawing.objects.iter().find_map(|(&h, obj)| match obj {
        ObjectType::Dictionary(d) if d.owner.is_null() => Some(h),
        _ => None,
    })
}

fn write_nod_build_mark(drawing: &mut CadDocument, value: &str) {
    use acadrust::objects::{XRecord, XRecordEntry, XRecordValue};

    let Some(nod_handle) = root_dict_handle(drawing) else {
        return;
    };
    let handle = drawing.allocate_handle();
    let mut xrec = XRecord::new();
    xrec.handle = handle;
    xrec.owner = nod_handle;
    xrec.entries.push(XRecordEntry {
        code: 1,
        value: XRecordValue::String(value.to_owned()),
    });
    drawing.objects.insert(handle, ObjectType::XRecord(xrec));
    if let Some(ObjectType::Dictionary(nod)) = drawing.objects.get_mut(&nod_handle) {
        nod.entries.retain(|(k, _)| k != NOD_BUILD_MARK_KEY);
        nod.entries.push((NOD_BUILD_MARK_KEY.to_owned(), handle));
    }
}

/// Reads the fallback NOD build mark written for pre-R2004 built DWGs.
pub(super) fn read_nod_build_mark(drawing: &CadDocument) -> Option<String> {
    use acadrust::objects::XRecordValue;

    let nod_handle = root_dict_handle(drawing)?;
    let ObjectType::Dictionary(nod) = drawing.objects.get(&nod_handle)? else {
        return None;
    };
    let xrec_handle = nod
        .entries
        .iter()
        .find(|(k, _)| k == NOD_BUILD_MARK_KEY)
        .map(|(_, h)| *h)?;
    let ObjectType::XRecord(xrec) = drawing.objects.get(&xrec_handle)? else {
        return None;
    };
    xrec.entries.iter().find_map(|entry| match &entry.value {
        XRecordValue::String(s) => Some(s.clone()),
        _ => None,
    })
}

/// Removes the fallback NOD build mark when stripping build metadata.
#[cfg(test)]
pub(super) fn strip_nod_build_mark(drawing: &mut CadDocument) {
    let Some(nod_handle) = root_dict_handle(drawing) else {
        return;
    };
    let mut removed = Vec::new();
    if let Some(ObjectType::Dictionary(nod)) = drawing.objects.get_mut(&nod_handle) {
        nod.entries.retain(|(k, h)| {
            if k == NOD_BUILD_MARK_KEY {
                removed.push(*h);
                false
            } else {
                true
            }
        });
    }
    for handle in removed {
        drawing.objects.remove(&handle);
    }
}

/// Prepares `drawing` for serialization in `target_version`.
///
/// Clears version-specific raw DWG records when the version or format changes
/// so acadrust re-encodes every object in the target version's format rather
/// than copying source-version bitstream bytes verbatim.
pub(super) fn prepare_for_version(
    drawing: &mut CadDocument,
    target: DwgVersion,
    clear_raw_records: bool,
) {
    let dxf_ver = to_dxf_version(target);
    drawing.version = dxf_ver;
    drawing.dwg_source_version = None;

    if clear_raw_records {
        for entity in drawing.entities_mut() {
            let common = entity.common_mut();
            common.raw_record = None;
            if !common.extended_data.records().is_empty() {
                common.extended_data.raw_dwg_eed.clear();
            }
            if let EntityType::Insert(ins) = entity {
                for att in &mut ins.attributes {
                    att.common.raw_record = None;
                    if !att.common.extended_data.records().is_empty() {
                        att.common.extended_data.raw_dwg_eed.clear();
                    }
                    if let Some(m) = att.embedded_mtext.as_mut() {
                        m.common.raw_record = None;
                        if !m.common.extended_data.records().is_empty() {
                            m.common.extended_data.raw_dwg_eed.clear();
                        }
                    }
                }
            }
        }
        for obj in drawing.objects.values_mut() {
            match obj {
                ObjectType::Layout(l) => {
                    l.raw_plot_settings_codes = None;
                }
                ObjectType::TableStyle(t) => {
                    t.raw_dxf_codes = None;
                }
                ObjectType::TableContent(t) => {
                    t.common.raw_record = None;
                }
                ObjectType::TableTemplate(t) => {
                    t.common.raw_record = None;
                }
                ObjectType::Arch(a) => {
                    a.raw_dwg_data = None;
                    a.raw_dwg_handle_bits = 0;
                    a.raw_dwg_version = None;
                    a.raw_dxf_codes = None;
                }
                _ => {}
            }
        }
    }
}

/// Analyzes `drawing` for AutoCAD features introduced after `target` and
/// returns concrete per-object / per-feature downgrade notices.
pub(super) fn analyze_version_losses(
    drawing: &CadDocument,
    target: DwgVersion,
    target_format: DrawingFormat,
) -> Vec<ConversionLoss> {
    let mut losses = Vec::new();
    let target_label = format!("{} ({})", target.code(), target.release());

    for entity in drawing.entities() {
        let handle = handle_text(entity.common().handle);
        let kind = kind_of(entity);
        let common = entity.common();

        // Entity-kind introduction versions
        if target < DwgVersion::R2010 {
            match entity {
                EntityType::Underlay(_) => {
                    losses.push(ConversionLoss {
                        kind: kind.clone(),
                        handle: handle.clone(),
                        change: ConversionLossKind::Downgraded,
                        detail: format!(
                            "{kind} requires AC1024 (2010) or later and is not supported natively \
                             in {target_label}"
                        ),
                    });
                }
                EntityType::Mesh(_) | EntityType::Surface(_) => {
                    losses.push(ConversionLoss {
                        kind: kind.clone(),
                        handle: handle.clone(),
                        change: ConversionLossKind::Downgraded,
                        detail: format!(
                            "{kind} requires AC1024 (2010) or later and may not round-trip in \
                             {target_label}"
                        ),
                    });
                }
                _ => {}
            }
        }
        if target < DwgVersion::R2007 {
            if matches!(entity, EntityType::MultiLeader(_)) {
                losses.push(ConversionLoss {
                    kind: kind.clone(),
                    handle: handle.clone(),
                    change: ConversionLossKind::Downgraded,
                    detail: format!(
                        "MULTILEADER requires AC1021 (2007/2008) or later and is not supported \
                         natively in {target_label}"
                    ),
                });
            }
            if entity_has_non_ascii_text(entity) {
                losses.push(ConversionLoss {
                    kind: kind.clone(),
                    handle: handle.clone(),
                    change: ConversionLossKind::Downgraded,
                    detail: format!(
                        "non-ASCII Unicode text may lose characters in {target_label} (UTF-16 \
                         strings were introduced in AC1021 / 2007)"
                    ),
                });
            }
        }
        if target < DwgVersion::R2004 {
            if matches!(entity, EntityType::Table(_)) {
                losses.push(ConversionLoss {
                    kind: kind.clone(),
                    handle: handle.clone(),
                    change: ConversionLossKind::Downgraded,
                    detail: format!(
                        "ACAD_TABLE requires AC1018 (2004/2005) or later and is not supported \
                         natively in {target_label}"
                    ),
                });
            }
            if matches!(common.color, Color::Rgb { .. }) {
                losses.push(ConversionLoss {
                    kind: kind.clone(),
                    handle: handle.clone(),
                    change: ConversionLossKind::Downgraded,
                    detail: format!(
                        "24-bit TrueColor on {kind} is downgraded to an ACI color index in \
                         {target_label} (introduced in AC1018 / 2004)"
                    ),
                });
            }
            if !common.transparency.is_by_layer() && !common.transparency.is_opaque() {
                losses.push(ConversionLoss {
                    kind: kind.clone(),
                    handle: handle.clone(),
                    change: ConversionLossKind::Downgraded,
                    detail: format!(
                        "object transparency on {kind} is not supported in {target_label}"
                    ),
                });
            }
        }
        if target < DwgVersion::R2000 {
            if !matches!(
                common.line_weight,
                LineWeight::ByLayer | LineWeight::ByBlock | LineWeight::Default
            ) {
                losses.push(ConversionLoss {
                    kind: kind.clone(),
                    handle: handle.clone(),
                    change: ConversionLossKind::Downgraded,
                    detail: format!(
                        "explicit lineweight on {kind} is dropped in {target_label} (introduced \
                         in AC1015 / 2000)"
                    ),
                });
            }
            if matches!(entity, EntityType::Wipeout(_)) {
                losses.push(ConversionLoss {
                    kind: kind.clone(),
                    handle: handle.clone(),
                    change: ConversionLossKind::Downgraded,
                    detail: format!(
                        "WIPEOUT requires AC1015 (2000) or later and is not supported natively in \
                         {target_label}"
                    ),
                });
            }
        }
        if target < DwgVersion::R14
            && matches!(
                entity,
                EntityType::LwPolyline(_) | EntityType::Hatch(_) | EntityType::RasterImage(_)
            )
        {
            losses.push(ConversionLoss {
                kind: kind.clone(),
                handle: handle.clone(),
                change: ConversionLossKind::Downgraded,
                detail: format!(
                    "{kind} requires AC1014 (R14) or later and is not supported natively in \
                     {target_label}"
                ),
            });
        }
        if target < DwgVersion::R13
            && matches!(
                entity,
                EntityType::MText(_)
                    | EntityType::Spline(_)
                    | EntityType::Ellipse(_)
                    | EntityType::MLine(_)
                    | EntityType::Tolerance(_)
                    | EntityType::XLine(_)
                    | EntityType::Ray(_)
                    | EntityType::Leader(_)
                    | EntityType::Solid3D(_)
                    | EntityType::Region(_)
                    | EntityType::Body(_)
            )
        {
            losses.push(ConversionLoss {
                kind,
                handle,
                change: ConversionLossKind::Downgraded,
                detail: format!(
                    "object kind requires AC1012 (R13) or later and is not supported natively in \
                     {target_label}"
                ),
            });
        }
    }

    // Layer-level features
    for layer in drawing.layers.iter() {
        if target < DwgVersion::R2004 {
            if matches!(layer.color, Color::Rgb { .. }) {
                losses.push(ConversionLoss {
                    kind: "LAYER".to_owned(),
                    handle: layer.name.clone(),
                    change: ConversionLossKind::Downgraded,
                    detail: format!(
                        "24-bit TrueColor on layer `{}` is downgraded to an ACI color index in \
                         {target_label}",
                        layer.name
                    ),
                });
            }
            if !layer.transparency.is_by_layer() && !layer.transparency.is_opaque() {
                losses.push(ConversionLoss {
                    kind: "LAYER".to_owned(),
                    handle: layer.name.clone(),
                    change: ConversionLossKind::Downgraded,
                    detail: format!(
                        "transparency on layer `{}` is not supported in {target_label}",
                        layer.name
                    ),
                });
            }
        }
        if target < DwgVersion::R2000
            && !matches!(
                layer.line_weight,
                LineWeight::ByLayer | LineWeight::ByBlock | LineWeight::Default
            )
        {
            losses.push(ConversionLoss {
                kind: "LAYER".to_owned(),
                handle: layer.name.clone(),
                change: ConversionLossKind::Downgraded,
                detail: format!(
                    "lineweight on layer `{}` is dropped in {target_label} (introduced in AC1015 \
                     / 2000)",
                    layer.name
                ),
            });
        }
    }

    // Drawing-wide & non-graphical features
    if target < DwgVersion::R2007 {
        let mut scale_count = 0usize;
        for obj in drawing.objects.values() {
            if matches!(obj, ObjectType::Scale(_) | ObjectType::ObjectContextData(_)) {
                scale_count += 1;
            }
        }
        if scale_count > 0 {
            losses.push(ConversionLoss {
                kind: "ANNOTATION_SCALE".to_owned(),
                handle: "0".to_owned(),
                change: ConversionLossKind::Downgraded,
                detail: format!(
                    "{scale_count} annotative scale / context object(s) require AC1021 \
                     (2007/2008) or later and are not supported natively in {target_label}"
                ),
            });
        }
    }

    if target < DwgVersion::R2004 {
        let field_count = drawing
            .objects
            .values()
            .filter(|o| matches!(o, ObjectType::Field(_) | ObjectType::FieldList(_)))
            .count();
        if field_count > 0 {
            losses.push(ConversionLoss {
                kind: "FIELD".to_owned(),
                handle: "0".to_owned(),
                change: ConversionLossKind::Downgraded,
                detail: format!(
                    "{field_count} AutoCAD field object(s) require AC1018 (2004/2005) or later; \
                     evaluated text remains static in {target_label}"
                ),
            });
        }
        if target_format.is_dwg() {
            let user_props: Vec<&str> = drawing
                .summary_info
                .custom_properties
                .iter()
                .filter(|(k, _)| k != BUILD_MARK)
                .map(|(k, _)| k.as_str())
                .collect();
            if !user_props.is_empty() {
                losses.push(ConversionLoss {
                    kind: "SUMMARY_INFO".to_owned(),
                    handle: "0".to_owned(),
                    change: ConversionLossKind::Downgraded,
                    detail: format!(
                        "custom drawing properties ({}) require AC1018 (2004) or later in DWG \
                         format and are omitted in {target_label}",
                        user_props.join(", ")
                    ),
                });
            }
        }
    }

    if target < DwgVersion::R2000 {
        if !drawing.header.plotstyle_mode {
            losses.push(ConversionLoss {
                kind: "PLOT_STYLE_MODE".to_owned(),
                handle: "0".to_owned(),
                change: ConversionLossKind::Downgraded,
                detail: format!(
                    "named plot styles (STB, PSTYLEMODE=0) require AC1015 (2000) or later and are \
                     not supported in {target_label}"
                ),
            });
        }
        let paper_layouts = drawing
            .objects
            .values()
            .filter(|o| {
                matches!(
                    o,
                    ObjectType::Layout(l)
                        if l.block_record != drawing.header.model_space_block_handle
                            && !l.block_record.is_null()
                )
            })
            .count();
        if paper_layouts > 1 {
            losses.push(ConversionLoss {
                kind: "LAYOUT".to_owned(),
                handle: "0".to_owned(),
                change: ConversionLossKind::Downgraded,
                detail: format!(
                    "{paper_layouts} paper-space layouts require AC1015 (2000) or later; older \
                     formats support only one paper space ({target_label})"
                ),
            });
        }
    }

    losses
}

fn entity_has_non_ascii_text(entity: &EntityType) -> bool {
    match entity {
        EntityType::Text(t) => !t.value.is_ascii(),
        EntityType::MText(m) => !m.value.is_ascii(),
        EntityType::AttributeDefinition(a) => !a.tag.is_ascii() || !a.default_value.is_ascii(),
        EntityType::AttributeEntity(a) => !a.tag.is_ascii() || !a.value.is_ascii(),
        EntityType::Insert(ins) => ins
            .attributes
            .iter()
            .any(|a| !a.tag.is_ascii() || !a.value.is_ascii()),
        _ => false,
    }
}

/// Compares pre-conversion and post-conversion structural snapshots and appends
/// any observed object drops, structural changes, table losses, or decode
/// regressions to `losses`.
///
/// Returns `(total_objects, unchanged_objects)`.
pub(super) fn diff_converted_snapshots(
    before: &DrawingSnapshot,
    after: &DrawingSnapshot,
    target: DwgVersion,
    losses: &mut Vec<ConversionLoss>,
) -> (usize, usize) {
    let mut already_noted: HashSet<(String, String)> = losses
        .iter()
        .map(|l| (l.kind.clone(), l.handle.clone()))
        .collect();

    let total_objects = before.objects.len();
    let mut unchanged_objects = 0usize;

    for (&handle, orig) in &before.objects {
        let hex = format!("{handle:X}");
        match after.objects.get(&handle) {
            None => {
                // On R12 / older formats acadrust may reassign handles or drop
                // unsupported entity kinds; report every missing handle clearly.
                losses.push(ConversionLoss {
                    kind: orig.kind.clone(),
                    handle: hex,
                    change: ConversionLossKind::Dropped,
                    detail: format!(
                        "{} (handle {:X}) was not present after converting to {} ({})",
                        orig.kind,
                        handle,
                        target.code(),
                        target.release()
                    ),
                });
            }
            Some(now) => {
                if now.kind == orig.kind && logical_matches(&orig.logical, &now.logical) {
                    unchanged_objects += 1;
                } else if !already_noted.contains(&(orig.kind.clone(), hex.clone())) {
                    already_noted.insert((orig.kind.clone(), hex.clone()));
                    let detail = if now.kind != orig.kind {
                        format!(
                            "object kind changed from {} to {} after converting to {} ({})",
                            orig.kind,
                            now.kind,
                            target.code(),
                            target.release()
                        )
                    } else {
                        format!(
                            "{} properties or geometry changed after converting to {} ({})",
                            orig.kind,
                            target.code(),
                            target.release()
                        )
                    };
                    losses.push(ConversionLoss {
                        kind: orig.kind.clone(),
                        handle: hex,
                        change: ConversionLossKind::Changed,
                        detail,
                    });
                }
            }
        }
    }

    // Layers: check dropped layers (ignoring case) and dropped/changed core attributes.
    for (key, orig) in &before.layers {
        match after.layers.get(key) {
            None => losses.push(ConversionLoss {
                kind: "LAYER".to_owned(),
                handle: orig.name.clone(),
                change: ConversionLossKind::Dropped,
                detail: format!(
                    "layer `{}` was absent after converting to {} ({})",
                    orig.name,
                    target.code(),
                    target.release()
                ),
            }),
            Some(now) => {
                // Compare layer properties expected to survive the target version.
                let color_changed = now.color != orig.color;
                let lt_changed = now.linetype != orig.linetype;
                let lw_changed = target >= DwgVersion::R2000 && now.lineweight != orig.lineweight;
                let tr_changed =
                    target >= DwgVersion::R2004 && now.transparency != orig.transparency;
                if (color_changed || lt_changed || lw_changed || tr_changed)
                    && !already_noted.contains(&("LAYER".to_owned(), orig.name.clone()))
                {
                    let mut diffs = Vec::new();
                    if color_changed {
                        diffs.push(format!("color {} -> {}", orig.color, now.color));
                    }
                    if lt_changed {
                        diffs.push(format!("linetype {} -> {}", orig.linetype, now.linetype));
                    }
                    if lw_changed {
                        diffs.push(format!(
                            "lineweight {} -> {}",
                            orig.lineweight, now.lineweight
                        ));
                    }
                    if tr_changed {
                        diffs.push(format!(
                            "transparency {} -> {}",
                            orig.transparency, now.transparency
                        ));
                    }
                    losses.push(ConversionLoss {
                        kind: "LAYER".to_owned(),
                        handle: orig.name.clone(),
                        change: ConversionLossKind::Changed,
                        detail: format!(
                            "layer `{}` changed after converting to {} ({}): {}",
                            orig.name,
                            target.code(),
                            target.release(),
                            diffs.join(", ")
                        ),
                    });
                }
            }
        }
    }

    // Block definitions: verify non-space blocks survived.
    for (key, orig) in &before.block_records {
        if key.starts_with("*model_space") || key.starts_with("*paper_space") {
            continue;
        }
        if !after.block_records.contains_key(key) {
            losses.push(ConversionLoss {
                kind: "BLOCK_RECORD".to_owned(),
                handle: orig.name.clone(),
                change: ConversionLossKind::Dropped,
                detail: format!(
                    "block `{}` was absent after converting to {} ({})",
                    orig.name,
                    target.code(),
                    target.release()
                ),
            });
        }
    }

    // Decode regressions on the converted output.
    let mut decode_changes = Vec::new();
    check_decode_regression(&before.decode, &after.decode, &mut decode_changes);
    for ch in decode_changes {
        losses.push(ConversionLoss {
            kind: ch.kind,
            handle: ch.handle,
            change: ConversionLossKind::Decode,
            detail: ch.detail,
        });
    }

    (total_objects, unchanged_objects)
}

/// Compares logical entity digests across DWG versions, ignoring raw EED wire
/// byte representation differences (`reed=[...]`) when structured XData
/// (`xd=[...]`) was parsed and matches — because DWG switched XData string
/// encoding from ANSI codepage to UTF-16LE in R2007 (`AC1021`).
fn logical_matches(before: &str, after: &str) -> bool {
    if before == after {
        return true;
    }
    strip_parsed_raw_eed(before) == strip_parsed_raw_eed(after)
}

fn strip_parsed_raw_eed(summary: &str) -> String {
    // `common_summary` formats `xd=[...] reed=[...]` before geometry fields.
    // Only ignore `reed=[...]` when `xd=[...]` is non-empty (parsed records exist).
    if summary.contains("xd=[]") {
        return summary.to_owned();
    }
    let Some(start) = summary.find(" reed=[") else {
        return summary.to_owned();
    };
    let rest = &summary[start + " reed=[".len()..];
    let mut depth = 1usize;
    let mut end_rel = None;
    for (idx, ch) in rest.char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    end_rel = Some(idx);
                    break;
                }
            }
            _ => {}
        }
    }
    match end_rel {
        Some(rel) => format!("{}{}", &summary[..start], &rest[rel + 1..]),
        None => summary.to_owned(),
    }
}
