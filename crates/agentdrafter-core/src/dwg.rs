//! The seam between AgentDrafter and its DWG library.
//!
//! This module, with its submodules under `dwg/`, is the only code that
//! names acadrust (ADR-0009). Everything else in the core works on its own
//! model ([`crate::model`]), so moving the acadrust pin, or the library
//! itself, changes these files and `Cargo.toml` only. Moving the pin means
//! running the judge again. [`hatch`] carries hatches through the seam,
//! [`text`] text styles and text, [`block`] blocks, attribute
//! definitions and inserts, [`table`] table styles and tables, [`leader`]
//! multileader styles, multileaders and older leaders, [`ids`]
//! each object's ID, and [`import`] a DWG's or a DXF's blocks for an
//! import into a block library.
//!
//! Our own names here follow `CONTEXT.md`: a DWG file's contents are a
//! drawing and the things in it are objects. acadrust's names, such as
//! `CadDocument` and `EntityType`, stay as acadrust spells them.

use std::any::Any;
use std::f64::consts::TAU;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use acadrust::entities::{
    Arc as DwgArc, Circle as DwgCircle, Ellipse as DwgEllipse, EntityCommon, EntityType,
    Line as DwgLine, LwPolyline, LwVertex, Polyline3D, Solid as DwgSolid, Spline as DwgSpline,
};
use acadrust::tables::{BlockRecord, Layer as DwgLayer, LineType, LineTypeElement};
use acadrust::types::{Color, DxfVersion, LineWeight, Vector2, Vector3};
use acadrust::xdata::{ExtendedDataRecord, XDataValue};
use acadrust::{CadDocument, DwgReader, DwgWriter, DxfReader, DxfReaderConfiguration, DxfWriter};

use crate::build::DrawingFormat;

use crate::description::{normalize_degrees, SheetRecord};
use crate::error::{Error, ErrorCode};
use crate::model::MTextBox;
use crate::model::{
    is_one, Arc, Block, Circle, DecodeReport, DrawingContents, Ellipse, IdAndName, Layer, Line,
    Linetype, Lineweight, Object, ObjectColor, ObjectLinetype, ObjectLineweight, Point3, Polyline,
    Properties, RecordsOfKind, RevisionCloud, Solid, Spline, Xref, XrefKind, BUILT_IN_LINETYPES,
    BY_BLOCK, BY_LAYER, CONTINUOUS, FIT_DEGREE, LAYER_ZERO, REVCLOUD_APPLICATION,
};
use crate::text::{MText as MTextObject, Text as TextObject};
use crate::units::Unit;

mod annotation_scale;
mod block;
mod convert;
mod dimension;
mod dynamic_block;
mod extra;
mod field;
mod group;
mod hatch;
mod ids;
mod import;
mod layout;
mod leader;
mod media;
mod page_setup;
mod revise;
mod solid3d;
mod table;
mod text;
mod wipeout;

pub(crate) use convert::convert_drawing_bytes;
pub(crate) use import::read_blocks;
pub(crate) use layout::{PageSetup, PLOTTER, PLOT_STYLE};
#[cfg(test)]
pub(crate) use revise::{
    arc_world_center_and_normal, as_original_of_version, block_definition_entity_handle,
    carried_id_and_name_of, layer_details_of, minsert_grid_positions_of, polyline3d_details_of,
    raw_record_bytes_of, with_asymmetric_minsert_original, with_constant_and_locked_attrs,
    with_custom_header, with_neg_z_arc_original, with_polyline3d_original,
    with_true_color_locked_layer,
};
pub(crate) use revise::{
    LockedLayerHit, MutationHandles, RevisionSession, RevisionVerification, TargetSpace,
};
pub use revise::{UnexpectedChange, UnexpectedChangeKind};

/// The DWG version a new sheet is written in: the 2018 format, AC1032 (ADR-0006).
const NEW_SHEET_VERSION: DxfVersion = DxfVersion::AC1032;

/// The build mark: a custom drawing property, as AutoCAD's DWGPROPS shows
/// it, written into every DWG AgentDrafter builds. A build replaces an
/// existing DWG only when it carries it. Its value is [`BUILT`] for a sheet
/// built on its own, and [`BUILT_FROM_SET`] followed by the drawing set's
/// folder for a sheet of a set, such as `build; set=..`. A sheet of a set
/// whose rows are in more than one rows file names its own after
/// [`ROWS_IN_MARK`], before the set: `build; rows=rows/areas.yaml; set=..`.
const BUILD_MARK: &str = "AgentDrafter";

/// The build mark's value for a sheet built on its own.
const BUILT: &str = "build";

/// How the build mark's value starts for a sheet of a drawing set: the
/// set's folder follows, relative to the DWG's folder, with `/` between
/// folders, to the end of the value.
const BUILT_FROM_SET: &str = "build; set=";

/// How the build mark's value starts for a sheet of a drawing set that
/// names the sheet's rows file: the rows file follows, relative to the
/// set's folder, up to the first `;`, which it never holds, then `; set=`
/// and the set's folder, as after [`BUILT_FROM_SET`].
const ROWS_IN_MARK: &str = "build; rows=";

/// The section of a DWG that holds its drawing properties, DWGPROPS.
const SUMMARY_SECTION: &str = "AcDb:SummaryInfo";

/// What a DWG's build mark says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BuildMark {
    /// The folder of the drawing set the DWG was built from, relative to
    /// the DWG's folder, as the mark writes it: `/` between folders, such
    /// as `..` or `../demo-plant`. None for a sheet built on its own.
    pub set: Option<String>,
    /// The rows file the sheet's row is in, relative to the set's folder,
    /// as the set reaches it, such as `rows/areas.yaml`: named only for a
    /// sheet of a set whose rows are in more than one rows file, and only
    /// when the path holds no `;`. None otherwise.
    pub rows: Option<String>,
}

/// AutoCAD's own knot spacing for a curve through fit points: by the chord
/// length between them.
const CHORD_KNOTS: i32 = 0;

/// How far a knot or an ellipse's end parameter may sit from where it
/// belongs and still be read as there.
const TOLERANCE: f64 = 1e-12;

/// The sentinel bytes at the start of every AutoCAD Binary DXF file.
const BINARY_DXF_SENTINEL: &[u8] = b"AutoCAD Binary DXF";

/// Whether `path` has a `.dxf` extension, in any case.
pub(crate) fn is_dxf_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("dxf"))
}

/// Detects the drawing format of `bytes` read from `path`: binary DXF by its
/// sentinel, ASCII DXF by a `.dxf` extension, and otherwise DWG.
pub(crate) fn detect_drawing_format(bytes: &[u8], path: &Path) -> DrawingFormat {
    if bytes.starts_with(BINARY_DXF_SENTINEL) {
        DrawingFormat::DxfBinary
    } else if is_dxf_path(path) {
        DrawingFormat::Dxf
    } else {
        DrawingFormat::Dwg
    }
}

/// Writes a new sheet as the bytes of a DWG in the 2018 format, carrying the
/// build mark `mark`.
#[cfg(test)]
pub(crate) fn write_new_sheet(
    record: &SheetRecord,
    start_of: &dyn Fn(&TextObject) -> Point3,
    measure: &dyn Fn(&MTextObject) -> MTextBox,
    mark: &BuildMark,
) -> Result<Vec<u8>, Error> {
    write_new_sheet_as(record, start_of, measure, mark, DrawingFormat::Dwg)
}

/// Writes a new sheet as the bytes of a DWG, ASCII DXF or binary DXF in the
/// 2018 format (`AC1032`), carrying the build mark `mark`.
#[cfg(test)]
pub(crate) fn write_new_sheet_as(
    record: &SheetRecord,
    start_of: &dyn Fn(&TextObject) -> Point3,
    measure: &dyn Fn(&MTextObject) -> MTextBox,
    mark: &BuildMark,
    format: DrawingFormat,
) -> Result<Vec<u8>, Error> {
    write_new_sheet_in_version(
        record,
        start_of,
        measure,
        mark,
        format,
        crate::convert::DwgVersion::R2018,
    )
    .map(|(bytes, _)| bytes)
}

/// Writes a new sheet as the bytes of a DWG, ASCII DXF or binary DXF in
/// `version`, carrying the build mark `mark`, and returns any version
/// conversion losses when `version` is older than 2018 (`AC1032`). `record`
/// holds the sheet with the record of each of its objects, and each object is
/// written with its ID and name from its record. `start_of` gives where each
/// single line of text starts, which the drawing records beside the point it is
/// justified at, and `measure` the box multiline text lays out in, which
/// places a multileader's text.
pub(crate) fn write_new_sheet_in_version(
    record: &SheetRecord,
    start_of: &dyn Fn(&TextObject) -> Point3,
    measure: &dyn Fn(&MTextObject) -> MTextBox,
    mark: &BuildMark,
    format: DrawingFormat,
    version: crate::convert::DwgVersion,
) -> Result<(Vec<u8>, Vec<crate::convert::ConversionLoss>), Error> {
    let sheet = &record.sheet;
    if record.objects.len() != sheet.objects.len() {
        return Err(Error::new(
            ErrorCode::DwgWrite,
            format!(
                "sheet `{}` holds {} objects and the records of {}; every object is written \
                 with its ID",
                record.name,
                sheet.objects.len(),
                record.objects.len()
            ),
        ));
    }
    let mut drawing = CadDocument::with_version(NEW_SHEET_VERSION);
    drawing.header.insertion_units = insertion_units(sheet.units);
    if !is_one(&sheet.ltscale) {
        drawing.header.linetype_scale = sheet.ltscale;
    }
    drawing.header.measurement = i16::from(sheet.units.is_metric());
    drawing
        .summary_info
        .custom_properties
        .push((BUILD_MARK.to_owned(), mark_value(mark)));
    field::write_custom_properties(&mut drawing, &sheet.custom_properties);
    for linetype in &sheet.linetypes {
        drawing
            .line_types
            .add(dwg_linetype(linetype))
            .map_err(|e| {
                Error::new(
                    ErrorCode::DwgWrite,
                    format!(
                        "could not add linetype `{}` to the drawing: {e}",
                        linetype.name
                    ),
                )
            })?;
    }
    for layer in &sheet.layers {
        match drawing.layers.get_mut(&layer.name) {
            // Layer 0 is in every drawing: the sheet sets its properties.
            Some(existing) => set_layer_properties(existing, layer),
            None => {
                let mut dwg_layer = DwgLayer::new(layer.name.clone());
                set_layer_properties(&mut dwg_layer, layer);
                drawing.layers.add(dwg_layer).map_err(|e| {
                    Error::new(
                        ErrorCode::DwgWrite,
                        format!("could not add layer `{}` to the drawing: {e}", layer.name),
                    )
                })?;
            }
        }
    }
    text::add_text_styles(&mut drawing, &sheet.text_styles)?;
    // Every object carries its ID in AgentDrafter's extended data.
    if !sheet.objects.is_empty() {
        block::register_application(&mut drawing)?;
    }
    let has_revcloud = sheet
        .objects
        .iter()
        .chain(sheet.blocks.iter().flat_map(|b| b.objects.iter()))
        .any(|o| matches!(o, Object::RevisionCloud(_)));
    if has_revcloud {
        block::register_named_application(&mut drawing, REVCLOUD_APPLICATION)?;
    }
    let has_layer_transparency = sheet.layers.iter().any(|l| l.transparency > 0);
    if has_layer_transparency {
        block::register_named_application(&mut drawing, page_setup::LAYER_TRANSPARENCY_APP)?;
    }
    let uses_annotation_scales = annotation_scale::sheet_uses_annotation_scales(sheet);
    if uses_annotation_scales {
        block::register_named_application(&mut drawing, annotation_scale::ACAD_ANNOTATIVE)?;
    }
    let has_center_line = sheet
        .objects
        .iter()
        .chain(sheet.blocks.iter().flat_map(|b| b.objects.iter()))
        .any(|o| matches!(o, Object::CenterLine(_)));
    if has_center_line {
        extra::ensure_app_id(
            &mut drawing,
            acadrust::entities::CENTERLINE_XDATA_APPLICATION,
        );
    }
    let has_center_mark = sheet
        .objects
        .iter()
        .chain(sheet.blocks.iter().flat_map(|b| b.objects.iter()))
        .any(|o| matches!(o, Object::CenterMark(_)));
    if has_center_mark {
        extra::ensure_app_id(
            &mut drawing,
            acadrust::entities::CENTERMARK_XDATA_APPLICATION,
        );
    }
    for obj in sheet
        .objects
        .iter()
        .chain(sheet.blocks.iter().flat_map(|b| b.objects.iter()))
    {
        if let Object::Shape(sh) = obj {
            extra::ensure_shape_style(&mut drawing, &sh.file);
        }
    }
    extra::write_point_style(&mut drawing, sheet.pdmode, sheet.pdsize);
    dimension::prepare(&mut drawing, sheet)?;
    // Layers, linetypes, text styles and applications take their handles
    // before the blocks and objects take theirs.
    drawing.assign_table_entry_handles();
    let mline_style_handles = extra::add_mline_styles(&mut drawing, &sheet.mline_styles);
    let mut center_mark_counter = 0usize;
    let scale_handles = if uses_annotation_scales {
        annotation_scale::write_scales_and_variables(&mut drawing, sheet)
    } else {
        std::collections::HashMap::new()
    };
    let named_plot_styles = sheet.uses_named_plot_styles();
    if named_plot_styles {
        drawing.header.plotstyle_mode = false;
        page_setup::assign_layer_plot_styles(&mut drawing, &sheet.layers);
    }
    let has_wipeout = sheet
        .objects
        .iter()
        .chain(sheet.blocks.iter().flat_map(|b| b.objects.iter()))
        .any(|o| matches!(o, Object::Wipeout(_)));
    if has_wipeout || !sheet.wipeout_frame.is_off() {
        wipeout::write_wipeout_frame(&mut drawing, sheet.wipeout_frame);
    }
    let has_raster_image = sheet
        .objects
        .iter()
        .chain(sheet.blocks.iter().flat_map(|b| b.objects.iter()))
        .any(|o| matches!(o, Object::RasterImage(_)));
    if has_raster_image || !sheet.image_frame.is_off() {
        media::write_image_frame(&mut drawing, sheet.image_frame, sheet.units);
    }
    table::add_table_styles(&mut drawing, sheet)?;
    let units = insertion_units(sheet.units);
    let mut media_dirs: Vec<std::path::PathBuf> = Vec::new();
    if let Some(dir) = record.file.parent().filter(|p| !p.as_os_str().is_empty()) {
        media_dirs.push(dir.to_path_buf());
        if let Some(lib) = &sheet.block_library {
            if let Some(lib_dir) = dir.join(lib).parent() {
                if lib_dir != dir {
                    media_dirs.push(lib_dir.to_path_buf());
                }
            }
        }
    }
    let media_dir_refs: Vec<&Path> = media_dirs.iter().map(std::path::PathBuf::as_path).collect();
    for definition in &sheet.blocks {
        // Dynamic blocks write their default-evaluated geometry into the named
        // block record so default-state inserts draw cleanly without overlapping
        // hidden visibility states.
        let master = definition.evaluated(&std::collections::BTreeMap::new());
        let owner = block::add_block_record(&mut drawing, master.as_ref(), units)?;
        for object in &master.objects {
            let mut dwg_object = match object {
                Object::RasterImage(img) => {
                    let h = drawing.allocate_handle();
                    let px = sheet
                        .resolved_media_for(crate::model::MediaKind::Image, &img.path, None)
                        .map(|m| (m.width_px, m.height_px))
                        .or_else(|| {
                            crate::media::probe_image_dimensions(&img.path, &media_dir_refs)
                        });
                    let (mut e, _) = media::dwg_raster_image(&mut drawing, img, px, h);
                    img.layer.clone_into(&mut e.common_mut().layer);
                    set_properties(e.common_mut(), &img.properties);
                    e
                }
                Object::PdfUnderlay(pdf) => {
                    let h = drawing.allocate_handle();
                    let (mut e, _) = media::dwg_pdf_underlay(&mut drawing, pdf, h);
                    pdf.layer.clone_into(&mut e.common_mut().layer);
                    set_properties(e.common_mut(), &pdf.properties);
                    e
                }
                Object::MLine(ml) => {
                    let style = sheet
                        .mline_style(&ml.style)
                        .unwrap_or_else(crate::model::MLineStyle::standard);
                    let sh = mline_style_handles.get(&style.name.to_lowercase()).copied();
                    let mut e = EntityType::MLine(extra::dwg_mline(ml, &style, sh));
                    ml.layer.clone_into(&mut e.common_mut().layer);
                    set_properties(e.common_mut(), &ml.properties);
                    e
                }
                Object::Tolerance(tol) => {
                    let mut e =
                        EntityType::Tolerance(extra::dwg_tolerance(&drawing, tol, Some(sheet)));
                    tol.layer.clone_into(&mut e.common_mut().layer);
                    set_properties(e.common_mut(), &tol.properties);
                    e
                }
                Object::CenterMark(cm) => {
                    let mut e = EntityType::Insert(extra::dwg_center_mark(
                        &mut drawing,
                        cm,
                        &mut center_mark_counter,
                    ));
                    cm.layer.clone_into(&mut e.common_mut().layer);
                    set_properties(e.common_mut(), &cm.properties);
                    e
                }
                Object::CenterLine(cl) => {
                    let mut e = EntityType::Line(extra::dwg_center_line(&mut drawing, cl));
                    cl.layer.clone_into(&mut e.common_mut().layer);
                    set_properties(e.common_mut(), &cl.properties);
                    e
                }
                Object::Shape(sh) => {
                    let mut e = EntityType::Shape(extra::dwg_shape(&mut drawing, sh, Some(sheet)));
                    sh.layer.clone_into(&mut e.common_mut().layer);
                    set_properties(e.common_mut(), &sh.properties);
                    e
                }
                _ => dwg_object(object, start_of, &sheet.blocks)?,
            };
            dwg_object.common_mut().owner_handle = owner;
            if named_plot_styles {
                page_setup::set_entity_plot_style(
                    &mut drawing,
                    dwg_object.common_mut(),
                    &object.properties().plot_style,
                );
            }
            drawing.add_entity(dwg_object).map_err(|e| {
                Error::new(
                    ErrorCode::DwgWrite,
                    format!(
                        "could not add a {} to block `{}`: {e}",
                        object.type_name(),
                        definition.name
                    ),
                )
            })?;
        }
    }
    for xref in &sheet.xrefs {
        block::add_xref_record(&mut drawing, xref, units)?;
    }
    let mut dynamic_writer = dynamic_block::DynamicBlockWriter::new();
    // After the sheet's blocks, so that a block of its library drawn as
    // AutoCAD's arrowhead block is the one a leader draws with.
    leader::add_leader_styles(&mut drawing, sheet)?;
    dimension::add_dimension_styles(&mut drawing, sheet)?;
    // A sheet drawn in a layout names AutoCAD's `Layout1` after it, sets
    // its page setup and adds its overall viewport before its objects.
    layout::prepare(&mut drawing, sheet)?;
    let mut dimensions = 0;
    let mut id_to_handle: std::collections::HashMap<String, acadrust::types::Handle> =
        std::collections::HashMap::with_capacity(sheet.objects.len());
    // Deferred fields to attach once every sheet object has its handle (so
    // `object_property` can reference any object regardless of order).
    struct PendingField<'a> {
        host: acadrust::types::Handle,
        field: &'a crate::model::TextField,
        display_text: &'a str,
        width_factor: Option<f64>,
        is_mtext: bool,
        layout_name: Option<&'a str>,
    }
    let mut pending_fields: Vec<PendingField<'_>> = Vec::new();
    let default_layout_name = sheet.layout.as_ref().map(|l| l.name.as_str());
    for (i, (object, written)) in sheet.objects.iter().zip(&record.objects).enumerate() {
        // The space it is in: model space, the first layout's paper space,
        // AutoCAD's `*Paper_Space`, or another layout's, by its name.
        let layout_name = sheet
            .layout_of(i)
            .map(|l| l.name.as_str())
            .or(default_layout_name);
        let paper = match sheet.layout_of(i) {
            None => Space::Model,
            Some(_) if sheet.in_paper_space(i) => Space::Paper,
            Some(other) => Space::Layout(&other.name),
        };
        let is_model_annotative =
            matches!(paper, Space::Model) && sheet.object_is_annotative(object);
        let obj_scales = if is_model_annotative {
            sheet.object_annotation_scales(object)
        } else {
            Vec::new()
        };
        let default_scale = if is_model_annotative {
            Some(annotation_scale::default_scale_for(sheet, &obj_scales))
        } else {
            None
        };
        let anno_inv = default_scale.as_ref().map_or(1.0, |s| s.inverse_factor());
        if let (Object::Viewport(v), Some(on)) = (object, sheet.layout_of(i)) {
            let handle = add_viewport(
                &mut drawing,
                sheet,
                v,
                &on.name,
                (&record.name, &written.id, written.name.as_deref()),
            )?;
            id_to_handle.insert(written.id.to_lowercase(), handle);
            continue;
        }
        if let Object::RasterImage(img) = object {
            let h = drawing.allocate_handle();
            let px = sheet
                .resolved_media_for(crate::model::MediaKind::Image, &img.path, None)
                .map(|m| (m.width_px, m.height_px))
                .or_else(|| crate::media::probe_image_dimensions(&img.path, &media_dir_refs));
            let (mut dwg_img, _) = media::dwg_raster_image(&mut drawing, img, px, h);
            let common = dwg_img.common_mut();
            img.layer.clone_into(&mut common.layer);
            set_properties(common, &img.properties);
            ids::write_id_and_name(common, &record.name, &written.id, written.name.as_deref());
            if named_plot_styles {
                page_setup::set_entity_plot_style(
                    &mut drawing,
                    dwg_img.common_mut(),
                    &img.properties.plot_style,
                );
            }
            let handle = add_to(&mut drawing, dwg_img, paper).map_err(|e| {
                Error::new(
                    ErrorCode::DwgWrite,
                    format!("could not add a raster image to the drawing: {e}"),
                )
            })?;
            id_to_handle.insert(written.id.to_lowercase(), handle);
            continue;
        }
        if let Object::PdfUnderlay(pdf) = object {
            let h = drawing.allocate_handle();
            let (mut dwg_pdf, _) = media::dwg_pdf_underlay(&mut drawing, pdf, h);
            let common = dwg_pdf.common_mut();
            pdf.layer.clone_into(&mut common.layer);
            set_properties(common, &pdf.properties);
            ids::write_id_and_name(common, &record.name, &written.id, written.name.as_deref());
            if named_plot_styles {
                page_setup::set_entity_plot_style(
                    &mut drawing,
                    dwg_pdf.common_mut(),
                    &pdf.properties.plot_style,
                );
            }
            let handle = add_to(&mut drawing, dwg_pdf, paper).map_err(|e| {
                Error::new(
                    ErrorCode::DwgWrite,
                    format!("could not add a PDF underlay to the drawing: {e}"),
                )
            })?;
            id_to_handle.insert(written.id.to_lowercase(), handle);
            continue;
        }
        if let Object::MLine(ml) = object {
            let style = sheet
                .mline_style(&ml.style)
                .unwrap_or_else(crate::model::MLineStyle::standard);
            let sh = mline_style_handles.get(&style.name.to_lowercase()).copied();
            let mut dwg_ml = EntityType::MLine(extra::dwg_mline(ml, &style, sh));
            let common = dwg_ml.common_mut();
            ml.layer.clone_into(&mut common.layer);
            set_properties(common, &ml.properties);
            ids::write_id_and_name(common, &record.name, &written.id, written.name.as_deref());
            if named_plot_styles {
                page_setup::set_entity_plot_style(
                    &mut drawing,
                    dwg_ml.common_mut(),
                    &ml.properties.plot_style,
                );
            }
            let handle = add_to(&mut drawing, dwg_ml, paper).map_err(|e| {
                Error::new(
                    ErrorCode::DwgWrite,
                    format!("could not add a multiline to the drawing: {e}"),
                )
            })?;
            id_to_handle.insert(written.id.to_lowercase(), handle);
            continue;
        }
        if let Object::Tolerance(tol) = object {
            let mut dwg_tol =
                EntityType::Tolerance(extra::dwg_tolerance(&drawing, tol, Some(sheet)));
            let common = dwg_tol.common_mut();
            tol.layer.clone_into(&mut common.layer);
            set_properties(common, &tol.properties);
            ids::write_id_and_name(common, &record.name, &written.id, written.name.as_deref());
            if named_plot_styles {
                page_setup::set_entity_plot_style(
                    &mut drawing,
                    dwg_tol.common_mut(),
                    &tol.properties.plot_style,
                );
            }
            let handle = add_to(&mut drawing, dwg_tol, paper).map_err(|e| {
                Error::new(
                    ErrorCode::DwgWrite,
                    format!("could not add a tolerance to the drawing: {e}"),
                )
            })?;
            id_to_handle.insert(written.id.to_lowercase(), handle);
            continue;
        }
        if let Object::CenterMark(cm) = object {
            let mut dwg_cm = EntityType::Insert(extra::dwg_center_mark(
                &mut drawing,
                cm,
                &mut center_mark_counter,
            ));
            let common = dwg_cm.common_mut();
            cm.layer.clone_into(&mut common.layer);
            set_properties(common, &cm.properties);
            ids::write_id_and_name(common, &record.name, &written.id, written.name.as_deref());
            if named_plot_styles {
                page_setup::set_entity_plot_style(
                    &mut drawing,
                    dwg_cm.common_mut(),
                    &cm.properties.plot_style,
                );
            }
            let handle = add_to(&mut drawing, dwg_cm, paper).map_err(|e| {
                Error::new(
                    ErrorCode::DwgWrite,
                    format!("could not add a centre mark to the drawing: {e}"),
                )
            })?;
            id_to_handle.insert(written.id.to_lowercase(), handle);
            continue;
        }
        if let Object::CenterLine(cl) = object {
            let mut dwg_cl = EntityType::Line(extra::dwg_center_line(&mut drawing, cl));
            let common = dwg_cl.common_mut();
            cl.layer.clone_into(&mut common.layer);
            set_properties(common, &cl.properties);
            ids::write_id_and_name(common, &record.name, &written.id, written.name.as_deref());
            if named_plot_styles {
                page_setup::set_entity_plot_style(
                    &mut drawing,
                    dwg_cl.common_mut(),
                    &cl.properties.plot_style,
                );
            }
            let handle = add_to(&mut drawing, dwg_cl, paper).map_err(|e| {
                Error::new(
                    ErrorCode::DwgWrite,
                    format!("could not add a centre line to the drawing: {e}"),
                )
            })?;
            id_to_handle.insert(written.id.to_lowercase(), handle);
            continue;
        }
        if let Object::Shape(sh) = object {
            let mut dwg_sh = EntityType::Shape(extra::dwg_shape(&mut drawing, sh, Some(sheet)));
            let common = dwg_sh.common_mut();
            sh.layer.clone_into(&mut common.layer);
            set_properties(common, &sh.properties);
            ids::write_id_and_name(common, &record.name, &written.id, written.name.as_deref());
            if named_plot_styles {
                page_setup::set_entity_plot_style(
                    &mut drawing,
                    dwg_sh.common_mut(),
                    &sh.properties.plot_style,
                );
            }
            let handle = add_to(&mut drawing, dwg_sh, paper).map_err(|e| {
                Error::new(
                    ErrorCode::DwgWrite,
                    format!("could not add a shape to the drawing: {e}"),
                )
            })?;
            id_to_handle.insert(written.id.to_lowercase(), handle);
            continue;
        }
        if let Object::Table(t) = object {
            let style = sheet.table_style(&t.table_style).ok_or_else(|| {
                Error::new(
                    ErrorCode::DwgWrite,
                    format!(
                        "a table draws with table style `{}`, which the sheet does not define",
                        t.table_style
                    ),
                )
            })?;
            let mut dwg_table = EntityType::Table(Box::new(table::dwg_table(&drawing, t, &style)?));
            let common = dwg_table.common_mut();
            t.layer.clone_into(&mut common.layer);
            set_properties(common, &t.properties);
            ids::write_id_and_name(common, &record.name, &written.id, written.name.as_deref());
            if named_plot_styles {
                page_setup::set_entity_plot_style(
                    &mut drawing,
                    dwg_table.common_mut(),
                    &t.properties.plot_style,
                );
            }
            let handle = add_to(&mut drawing, dwg_table, paper).map_err(|e| {
                Error::new(
                    ErrorCode::DwgWrite,
                    format!("could not add a table to the drawing: {e}"),
                )
            })?;
            table::fill_block(&mut drawing, handle, t, &style)?;
            id_to_handle.insert(written.id.to_lowercase(), handle);
            continue;
        }
        if let Object::MLeader(m) = object {
            let style = sheet.mleader_style(&m.mleader_style).ok_or_else(|| {
                Error::new(
                    ErrorCode::DwgWrite,
                    format!(
                        "a multileader draws with multileader style `{}`, which the sheet does not define",
                        m.mleader_style
                    ),
                )
            })?;
            let scaled_ml = (anno_inv != 1.0).then(|| m.scaled_for_annotation(&style, anno_inv));
            let (eff_m, eff_s) = match &scaled_ml {
                Some((sm, ss)) => (sm, ss),
                None => (m, &style),
            };
            let mut dwg_mleader = EntityType::MultiLeader(Box::new(leader::dwg_mleader(
                &drawing,
                eff_m,
                eff_s,
                measure,
                &sheet.blocks,
            )?));
            let common = dwg_mleader.common_mut();
            m.layer.clone_into(&mut common.layer);
            set_properties(common, &m.properties);
            ids::write_id_and_name(common, &record.name, &written.id, written.name.as_deref());
            if named_plot_styles {
                page_setup::set_entity_plot_style(
                    &mut drawing,
                    dwg_mleader.common_mut(),
                    &m.properties.plot_style,
                );
            }
            let handle = add_to(&mut drawing, dwg_mleader, paper).map_err(|e| {
                Error::new(
                    ErrorCode::DwgWrite,
                    format!("could not add a multileader to the drawing: {e}"),
                )
            })?;
            if let Some(def_sc) = &default_scale {
                annotation_scale::attach_entity_annotation_scales(
                    &mut drawing,
                    handle,
                    &obj_scales,
                    def_sc,
                    &scale_handles,
                    !format.is_dwg(),
                );
            }
            id_to_handle.insert(written.id.to_lowercase(), handle);
            continue;
        }
        if let Object::Dimension(d) = object {
            let mut style = sheet.dimension_style(&d.dimension_style).ok_or_else(|| {
                Error::new(
                    ErrorCode::DwgWrite,
                    format!(
                        "a dimension draws with dimension style `{}`, which the sheet does not define",
                        d.dimension_style
                    ),
                )
            })?;
            if anno_inv != 1.0 {
                style.scale *= anno_inv;
            }
            dimensions += 1;
            let mut dwg_dimension = dimension::dwg_dimension(
                &mut drawing,
                d,
                &style,
                measure,
                dimensions,
                sheet.units,
            )?;
            let common = dwg_dimension.common_mut();
            d.layer.clone_into(&mut common.layer);
            set_properties(common, &d.properties);
            ids::write_id_and_name(common, &record.name, &written.id, written.name.as_deref());
            if named_plot_styles {
                page_setup::set_entity_plot_style(
                    &mut drawing,
                    dwg_dimension.common_mut(),
                    &d.properties.plot_style,
                );
            }
            let handle = add_to(&mut drawing, dwg_dimension, paper).map_err(|e| {
                Error::new(
                    ErrorCode::DwgWrite,
                    format!("could not add a dimension to the drawing: {e}"),
                )
            })?;
            if let Some(def_sc) = &default_scale {
                annotation_scale::attach_entity_annotation_scales(
                    &mut drawing,
                    handle,
                    &obj_scales,
                    def_sc,
                    &scale_handles,
                    !format.is_dwg(),
                );
            }
            id_to_handle.insert(written.id.to_lowercase(), handle);
            continue;
        }
        if let Object::Leader(l) = object {
            let (dwg_leader, annotation) = leader::dwg_leader(&drawing, l);
            let mut dwg_leader = EntityType::Leader(dwg_leader);
            let common = dwg_leader.common_mut();
            l.layer.clone_into(&mut common.layer);
            set_properties(common, &l.properties);
            ids::write_id_and_name(common, &record.name, &written.id, written.name.as_deref());
            if named_plot_styles {
                page_setup::set_entity_plot_style(
                    &mut drawing,
                    dwg_leader.common_mut(),
                    &l.properties.plot_style,
                );
            }
            let added = |e| {
                Error::new(
                    ErrorCode::DwgWrite,
                    format!("could not add a leader to the drawing: {e}"),
                )
            };
            let handle = add_to(&mut drawing, dwg_leader, paper).map_err(added)?;
            if let Some(mut text) = annotation {
                l.layer.clone_into(&mut text.common.layer);
                set_properties(&mut text.common, &l.properties);
                if named_plot_styles {
                    page_setup::set_entity_plot_style(
                        &mut drawing,
                        &mut text.common,
                        &l.properties.plot_style,
                    );
                }
                let text = add_to(&mut drawing, EntityType::MText(Box::new(text)), paper)
                    .map_err(added)?;
                leader::link_annotation(&mut drawing, handle, text);
            }
            id_to_handle.insert(written.id.to_lowercase(), handle);
            continue;
        }
        if let Object::Insert(insert) = object {
            if sheet
                .xrefs
                .iter()
                .any(|x| x.name == insert.block || x.name.eq_ignore_ascii_case(&insert.block))
            {
                let mut dwg_ins = block::dwg_xref_insert(insert);
                insert.layer.clone_into(&mut dwg_ins.common.layer);
                set_properties(&mut dwg_ins.common, &insert.properties);
                ids::write_id_and_name(
                    &mut dwg_ins.common,
                    &record.name,
                    &written.id,
                    written.name.as_deref(),
                );
                if named_plot_styles {
                    page_setup::set_entity_plot_style(
                        &mut drawing,
                        &mut dwg_ins.common,
                        &insert.properties.plot_style,
                    );
                }
                let handle =
                    add_to(&mut drawing, EntityType::Insert(dwg_ins), paper).map_err(|e| {
                        Error::new(
                            ErrorCode::DwgWrite,
                            format!(
                                "could not add an external reference insert to the drawing: {e}"
                            ),
                        )
                    })?;
                id_to_handle.insert(written.id.to_lowercase(), handle);
                continue;
            }
            let Some(definition) = sheet.blocks.iter().find(|b| b.name == insert.block) else {
                return Err(Error::new(
                    ErrorCode::DwgWrite,
                    format!(
                        "an insert places block `{}`, which the sheet does not define",
                        insert.block
                    ),
                ));
            };
            let scaled_ins = (anno_inv != 1.0).then(|| {
                let mut s = insert.clone();
                s.scale *= anno_inv;
                s
            });
            let eff_insert = scaled_ins.as_ref().unwrap_or(insert);
            let mut dwg_ins = dynamic_writer.dwg_insert(
                &mut drawing,
                eff_insert,
                definition,
                &sheet.blocks,
                units,
                start_of,
            )?;
            insert.layer.clone_into(&mut dwg_ins.common.layer);
            set_properties(&mut dwg_ins.common, &insert.properties);
            ids::write_id_and_name(
                &mut dwg_ins.common,
                &record.name,
                &written.id,
                written.name.as_deref(),
            );
            if named_plot_styles {
                page_setup::set_entity_plot_style(
                    &mut drawing,
                    &mut dwg_ins.common,
                    &insert.properties.plot_style,
                );
            }
            if definition.is_dynamic()
                && (!insert.state.is_empty() || dwg_ins.block_name != insert.block)
            {
                ids::write_dynamic_block(&mut dwg_ins.common, &insert.block, &insert.state);
            }
            let has_attr_fields = insert.attributes.iter().any(|a| a.field.is_some());
            let handle = add_to(&mut drawing, EntityType::Insert(dwg_ins), paper).map_err(|e| {
                Error::new(
                    ErrorCode::DwgWrite,
                    format!("could not add an insert to the drawing: {e}"),
                )
            })?;
            if let Some(def_sc) = &default_scale {
                annotation_scale::attach_entity_annotation_scales(
                    &mut drawing,
                    handle,
                    &obj_scales,
                    def_sc,
                    &scale_handles,
                    !format.is_dwg(),
                );
            }
            id_to_handle.insert(written.id.to_lowercase(), handle);
            if has_attr_fields {
                // Allocate handles for attributes and seqend so `set_text_field`
                // can attach ACAD_FIELD directly onto each AttributeEntity.
                let attr_count = match drawing.get_entity(handle) {
                    Some(EntityType::Insert(ins)) => ins.attributes.len(),
                    _ => 0,
                };
                let mut attr_handles = Vec::with_capacity(attr_count);
                for _ in 0..attr_count {
                    attr_handles.push(drawing.allocate_handle());
                }
                let seqend_handle = drawing.allocate_handle();
                if let Some(EntityType::Insert(ins)) = drawing.get_entity_mut(handle) {
                    for (att, h) in ins.attributes.iter_mut().zip(&attr_handles) {
                        att.common.handle = *h;
                        att.common.owner_handle = handle;
                    }
                    ins.seqend_handle = Some(seqend_handle);
                }
                for (att, &att_handle) in insert.attributes.iter().zip(&attr_handles) {
                    if let Some(tf) = &att.field {
                        pending_fields.push(PendingField {
                            host: att_handle,
                            field: tf,
                            display_text: &att.value,
                            width_factor: None,
                            is_mtext: false,
                            layout_name,
                        });
                    }
                }
            }
            continue;
        }
        let scaled_generic = (anno_inv != 1.0)
            .then(|| match object {
                Object::Text(t) => {
                    let mut s = t.clone();
                    s.height *= anno_inv;
                    Some(Object::Text(s))
                }
                Object::MText(m) => {
                    let mut s = m.clone();
                    s.height *= anno_inv;
                    s.width *= anno_inv;
                    Some(Object::MText(s))
                }
                Object::Hatch(h) if !h.is_solid() => {
                    let mut s = h.clone();
                    s.scale *= anno_inv;
                    Some(Object::Hatch(s))
                }
                _ => None,
            })
            .flatten();
        let eff_object = scaled_generic.as_ref().unwrap_or(object);
        let mut dwg_object = dwg_object(eff_object, start_of, &sheet.blocks)?;
        ids::write_id_and_name(
            dwg_object.common_mut(),
            &record.name,
            &written.id,
            written.name.as_deref(),
        );
        if named_plot_styles {
            page_setup::set_entity_plot_style(
                &mut drawing,
                dwg_object.common_mut(),
                &object.properties().plot_style,
            );
        }
        let handle = add_to(&mut drawing, dwg_object, paper).map_err(|e| {
            Error::new(
                ErrorCode::DwgWrite,
                format!("could not add a {} to the drawing: {e}", object.type_name()),
            )
        })?;
        if let Some(def_sc) = &default_scale {
            annotation_scale::attach_entity_annotation_scales(
                &mut drawing,
                handle,
                &obj_scales,
                def_sc,
                &scale_handles,
                !format.is_dwg(),
            );
        }
        id_to_handle.insert(written.id.to_lowercase(), handle);
        match object {
            Object::Text(t) => {
                if let Some(tf) = &t.field {
                    pending_fields.push(PendingField {
                        host: handle,
                        field: tf,
                        display_text: &t.contents,
                        width_factor: None,
                        is_mtext: false,
                        layout_name,
                    });
                }
            }
            Object::MText(t) => {
                if let Some(tf) = &t.field {
                    pending_fields.push(PendingField {
                        host: handle,
                        field: tf,
                        display_text: &t.contents,
                        width_factor: t.width_factor,
                        is_mtext: true,
                        layout_name,
                    });
                }
            }
            _ => {}
        }
    }
    if !pending_fields.is_empty() {
        let local_ids: Vec<String> = record.objects.iter().map(|r| r.id.clone()).collect();
        for pending in pending_fields {
            let ctx = crate::model::field::EvalContext {
                sheet_name: Some(&record.name),
                layout_name: pending.layout_name,
                custom_properties: &sheet.custom_properties,
                objects: &sheet.objects,
                object_ids: &local_ids,
                object_handles: &[],
            };
            let (mut slots, _) = crate::model::field::evaluate_text_field(pending.field, &ctx);
            if pending.field.items.len() == 1
                && crate::model::field::is_default_field_template(&pending.field.template)
                && !pending.display_text.is_empty()
            {
                slots[0] = pending.display_text.to_owned();
            }
            field::attach_text_field(
                &mut drawing,
                pending.host,
                pending.field,
                &slots,
                &id_to_handle,
                pending.width_factor,
                pending.is_mtext,
            )?;
        }
    }
    if !sheet.groups.is_empty() {
        let index_to_handle: Vec<Option<acadrust::types::Handle>> = record
            .objects
            .iter()
            .map(|w| id_to_handle.get(&w.id.to_lowercase()).copied())
            .collect();
        group::write_groups(&mut drawing, &sheet.groups, &index_to_handle)?;
    }
    convert::finish_new_sheet_in_version(drawing, version, format, &mark_value(mark))
}

/// Serializes `drawing` as a DWG, ASCII DXF or binary DXF.
pub(crate) fn write_document(
    drawing: &CadDocument,
    format: DrawingFormat,
) -> Result<Vec<u8>, Error> {
    let outcome = catch_unwind(AssertUnwindSafe(|| match format {
        DrawingFormat::Dwg => DwgWriter::write_to_vec(drawing),
        DrawingFormat::Dxf => DxfWriter::new(drawing).write_to_vec(),
        DrawingFormat::DxfBinary => DxfWriter::new_binary(drawing).write_to_vec(),
    }));
    match outcome {
        Ok(Ok(bytes)) => Ok(bytes),
        Ok(Err(e)) => Err(Error::new(
            ErrorCode::DwgWrite,
            format!("the DWG library could not write the drawing: {e}"),
        )),
        Err(panic) => Err(Error::new(
            ErrorCode::DwgWrite,
            format!(
                "the DWG library failed while writing the drawing: {}",
                panic_message(panic)
            ),
        )),
    }
}

/// Adds `object` to paper space when `paper` says so, and otherwise to
/// model space.
/// The space an object of a sheet is added to.
#[derive(Clone, Copy)]
enum Space<'a> {
    /// Model space.
    Model,
    /// The paper space of the sheet's first layout, AutoCAD's
    /// `*Paper_Space`.
    Paper,
    /// The paper space of another of its layouts, by its name.
    Layout(&'a str),
}

fn add_to(
    drawing: &mut CadDocument,
    object: EntityType,
    space: Space,
) -> Result<acadrust::types::Handle, acadrust::DxfError> {
    match space {
        Space::Model => drawing.add_entity(object),
        Space::Paper => drawing.add_paper_space_entity(object),
        Space::Layout(name) => drawing.add_entity_to_layout(object, name),
    }
}

/// Adds the sheet's viewport `viewport` to its layout, named `layout`,
/// with its ID and name, as AutoCAD's MVIEW makes one: a clipped one after
/// its boundary, an object of the layout's paper space on the viewport's
/// layer, with its pen, whose reactor is the viewport, as AutoCAD 2026's
/// VPCLIP and MVIEW left made-up ones; and freezing the layers it lists,
/// by their handles, as VPLAYER does.
fn add_viewport(
    drawing: &mut CadDocument,
    sheet: &crate::model::Sheet,
    viewport: &crate::model::Viewport,
    layout: &str,
    (sheet_name, id, name): (&str, &str, Option<&str>),
) -> Result<acadrust::types::Handle, Error> {
    let failed = |e: acadrust::DxfError| {
        Error::new(
            ErrorCode::DwgWrite,
            format!("could not add a viewport to layout `{layout}`: {e}"),
        )
    };
    let named_plot_styles = !drawing.header.plotstyle_mode;
    let boundary = match &viewport.clip {
        Some(clip) => {
            let mut boundary = layout::dwg_boundary(clip);
            let common = boundary.common_mut();
            viewport.layer.clone_into(&mut common.layer);
            set_properties(common, &viewport.properties);
            if named_plot_styles {
                page_setup::set_entity_plot_style(
                    drawing,
                    boundary.common_mut(),
                    &viewport.properties.plot_style,
                );
            }
            Some(
                drawing
                    .add_entity_to_layout(boundary, layout)
                    .map_err(failed)?,
            )
        }
        None => None,
    };
    let frozen = viewport
        .frozen_layers
        .iter()
        .map(|name| {
            drawing
                .layers
                .iter()
                .find(|l| l.name.eq_ignore_ascii_case(name))
                .map(|l| l.handle)
                .ok_or_else(|| {
                    Error::new(
                        ErrorCode::DwgWrite,
                        format!("a viewport freezes layer `{name}`, which the drawing lacks"),
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut dwg_viewport = layout::dwg_viewport(viewport, sheet.units, boundary, frozen);
    let common = dwg_viewport.common_mut();
    viewport.layer.clone_into(&mut common.layer);
    set_properties(common, &viewport.properties);
    ids::write_id_and_name(common, sheet_name, id, name);
    if let Some(sc) = &viewport.annotation_scale {
        ids::write_viewport_annoscale(common, sc);
    }
    if named_plot_styles {
        page_setup::set_entity_plot_style(
            drawing,
            dwg_viewport.common_mut(),
            &viewport.properties.plot_style,
        );
    }
    let handle = drawing
        .add_entity_to_layout(dwg_viewport, layout)
        .map_err(failed)?;
    if let Some(boundary) = boundary {
        if let Some(object) = drawing.get_entity_mut(boundary) {
            object.common_mut().reactors = vec![handle];
        }
    }
    Ok(handle)
}

fn dwg_linetype(linetype: &Linetype) -> LineType {
    let mut dwg_linetype = LineType::new(linetype.name.clone());
    dwg_linetype.description = linetype.description.clone();
    dwg_linetype.elements = linetype
        .pattern
        .iter()
        .map(|&length| LineTypeElement {
            length,
            complex: None,
        })
        .collect();
    dwg_linetype.pattern_length = linetype.pattern.iter().map(|l| l.abs()).sum();
    dwg_linetype
}

fn set_layer_properties(dwg_layer: &mut DwgLayer, layer: &Layer) {
    dwg_layer.flags.off = !layer.on;
    dwg_layer.flags.frozen = layer.frozen;
    dwg_layer.flags.locked = layer.locked;
    dwg_layer.is_plottable = layer.plot;
    layer.description.clone_into(&mut dwg_layer.description);
    dwg_layer.color = Color::Index(layer.color);
    dwg_layer.line_type = layer.linetype.clone();
    dwg_layer.line_weight = match layer.lineweight {
        Some(weight) => LineWeight::Value(weight.hundredths()),
        None => LineWeight::Default,
    };
    if layer.transparency > 0 {
        dwg_layer.transparency = page_setup::dwg_transparency_from_percent(layer.transparency);
    }
}

/// The DWG object for one of ours, on its layer, with its own colour,
/// linetype, lineweight and linetype scale where it gives them. An
/// insert's block is one of `blocks`.
fn dwg_object(
    object: &Object,
    start_of: &dyn Fn(&TextObject) -> Point3,
    blocks: &[Block],
) -> Result<EntityType, Error> {
    let mut dwg_object = match object {
        Object::Line(line) => {
            EntityType::Line(DwgLine::from_points(vector(line.from), vector(line.to)))
        }
        Object::Polyline(polyline) => dwg_polyline(polyline),
        Object::Arc(arc) => EntityType::Arc(DwgArc::from_center_radius_angles(
            vector(arc.center),
            arc.radius,
            arc.start_angle.to_radians(),
            arc.end_angle.to_radians(),
        )),
        Object::Circle(circle) => EntityType::Circle(DwgCircle::from_center_radius(
            vector(circle.center),
            circle.radius,
        )),
        Object::Ellipse(ellipse) => EntityType::Ellipse(dwg_ellipse(ellipse)),
        Object::Spline(spline) => EntityType::Spline(dwg_spline(spline)),
        Object::Hatch(hatch) => EntityType::Hatch(Box::new(hatch::dwg_hatch(hatch))),
        Object::Donut(donut) => dwg_polyline(&donut.polyline()),
        Object::Solid(solid) => EntityType::Solid(dwg_solid(solid)),
        Object::RevisionCloud(cloud) => dwg_revision_cloud(cloud),
        Object::Wipeout(w) => wipeout::dwg_wipeout(w),
        Object::Text(t) => EntityType::Text(text::dwg_text(t, start_of(t))),
        Object::MText(t) => EntityType::MText(Box::new(text::dwg_mtext(t))),
        Object::AttributeDefinition(definition) => {
            EntityType::AttributeDefinition(block::dwg_attribute_definition(definition, start_of))
        }
        Object::Table(_) => {
            return Err(Error::new(
                ErrorCode::DwgWrite,
                "a table belongs on a sheet, not in a block",
            ))
        }
        Object::MLeader(_) | Object::Leader(_) => {
            return Err(Error::new(
                ErrorCode::DwgWrite,
                "a leader belongs on a sheet, not in a block",
            ))
        }
        Object::Dimension(_) => {
            return Err(Error::new(
                ErrorCode::DwgWrite,
                "a dimension belongs on a sheet, not in a block",
            ))
        }
        Object::Viewport(_) => {
            return Err(Error::new(
                ErrorCode::DwgWrite,
                "a viewport belongs in a sheet's paper space",
            ))
        }
        Object::RasterImage(_)
        | Object::PdfUnderlay(_)
        | Object::MLine(_)
        | Object::Tolerance(_)
        | Object::CenterMark(_)
        | Object::CenterLine(_)
        | Object::Shape(_) => {
            return Err(Error::new(
                ErrorCode::DwgWrite,
                "this object requires drawing-level table or dictionary wiring",
            ))
        }
        Object::Point(pt) => EntityType::Point(extra::dwg_point(pt)),
        Object::XLine(xl) => EntityType::XLine(extra::dwg_xline(xl)),
        Object::Ray(ray) => EntityType::Ray(extra::dwg_ray(ray)),
        Object::Solid3D(solid) => EntityType::Solid3D(solid3d::dwg_solid3d(solid)?),
        Object::Insert(insert) => {
            let Some(definition) = blocks.iter().find(|b| b.name == insert.block) else {
                return Err(Error::new(
                    ErrorCode::DwgWrite,
                    format!(
                        "an insert places block `{}`, which the sheet does not define",
                        insert.block
                    ),
                ));
            };
            EntityType::Insert(block::dwg_insert(insert, definition, start_of)?)
        }
    };
    let common = dwg_object.common_mut();
    object.layer().clone_into(&mut common.layer);
    set_properties(common, object.properties());
    Ok(dwg_object)
}

/// A revision cloud is written as a closed lightweight polyline with
/// AutoCAD's `RevcloudProps` extended data (`1070 = 0`, `1040 = arc_length`)
/// so AutoCAD recognizes and edits it as a revision cloud (#72).
fn dwg_revision_cloud(cloud: &RevisionCloud) -> EntityType {
    let mut entity = dwg_polyline(&cloud.polyline());
    let mut record = ExtendedDataRecord::new(REVCLOUD_APPLICATION);
    record.add_value(XDataValue::Integer16(0));
    record.add_value(XDataValue::Real(cloud.arc_length));
    entity.common_mut().extended_data.upsert_record(record);
    entity
}

/// Writes an object's own colour, linetype, lineweight and linetype scale
/// on its DWG object. What is `ByLayer`, or a scale of 1, is left as
/// acadrust makes it, so an object that gives none is written as before.
pub(super) fn set_properties(common: &mut EntityCommon, properties: &Properties) {
    match properties.color {
        ObjectColor::ByLayer => {}
        ObjectColor::ByBlock => common.color = Color::ByBlock,
        ObjectColor::Index(index) => common.color = Color::Index(index),
    }
    match &properties.linetype {
        ObjectLinetype::ByLayer => {}
        ObjectLinetype::ByBlock => BY_BLOCK.clone_into(&mut common.linetype),
        ObjectLinetype::Named(name) => name.clone_into(&mut common.linetype),
    }
    match properties.lineweight {
        ObjectLineweight::ByLayer => {}
        ObjectLineweight::ByBlock => common.line_weight = LineWeight::ByBlock,
        ObjectLineweight::Default => common.line_weight = LineWeight::Default,
        ObjectLineweight::Width(width) => {
            common.line_weight = LineWeight::Value(width.hundredths());
        }
    }
    if !is_one(&properties.linetype_scale) {
        common.linetype_scale = properties.linetype_scale;
    }
    if !properties.transparency.is_by_layer() {
        common.transparency = page_setup::dwg_entity_transparency(properties.transparency);
    }
}

/// An object's own colour, linetype, lineweight, linetype scale, named plot
/// style and transparency as its DWG object holds them, if the core has a
/// model for them: an index colour rather than a true colour, and a standard
/// lineweight.
pub(super) fn read_properties(common: &EntityCommon) -> Option<Properties> {
    read_properties_in(common, false, &std::collections::HashMap::new())
}

pub(super) fn read_properties_in(
    common: &EntityCommon,
    named_mode: bool,
    ps_map: &std::collections::HashMap<acadrust::types::Handle, String>,
) -> Option<Properties> {
    let color = match common.color {
        Color::ByLayer => ObjectColor::ByLayer,
        Color::ByBlock => ObjectColor::ByBlock,
        Color::Index(index) if index >= 1 => ObjectColor::Index(index),
        _ => return None,
    };
    let linetype = if common.linetype.is_empty() || common.linetype.eq_ignore_ascii_case(BY_LAYER) {
        ObjectLinetype::ByLayer
    } else if common.linetype.eq_ignore_ascii_case(BY_BLOCK) {
        ObjectLinetype::ByBlock
    } else if common.linetype.eq_ignore_ascii_case(CONTINUOUS) {
        ObjectLinetype::Named(CONTINUOUS.to_owned())
    } else {
        ObjectLinetype::Named(common.linetype.clone())
    };
    let lineweight = match common.line_weight {
        LineWeight::ByLayer => ObjectLineweight::ByLayer,
        LineWeight::ByBlock => ObjectLineweight::ByBlock,
        LineWeight::Default => ObjectLineweight::Default,
        LineWeight::Value(hundredths) => {
            ObjectLineweight::Width(Lineweight::from_hundredths(hundredths)?)
        }
    };
    let linetype_scale = common.linetype_scale;
    if !(linetype_scale.is_finite() && linetype_scale > 0.0) {
        return None;
    }
    let plot_style = page_setup::read_entity_plot_style(common, named_mode, ps_map);
    let transparency = page_setup::read_entity_transparency(common.transparency);
    Some(Properties {
        color,
        linetype,
        lineweight,
        linetype_scale,
        plot_style,
        transparency,
    })
}

/// A polyline whose points share one `z` is a lightweight polyline at that
/// elevation, as AutoCAD's PLINE draws it, with each point's bulge and
/// widths, or its constant width; any other is a 3D polyline.
fn dwg_polyline(polyline: &Polyline) -> EntityType {
    let elevation = polyline.points.first().map_or(0.0, |p| p.z);
    if polyline.is_flat() {
        let mut flat = LwPolyline::from_points(
            polyline
                .points
                .iter()
                .map(|p| Vector2::new(p.x, p.y))
                .collect(),
        );
        for (i, vertex) in flat.vertices.iter_mut().enumerate() {
            vertex.bulge = polyline.bulge(i);
            if !polyline.widths.is_empty() {
                [vertex.start_width, vertex.end_width] = polyline.widths_at(i);
            }
        }
        flat.constant_width = polyline.width;
        flat.elevation = elevation;
        flat.is_closed = polyline.closed;
        EntityType::LwPolyline(flat)
    } else {
        let mut spatial =
            Polyline3D::from_points(polyline.points.iter().map(|&p| vector(p)).collect());
        spatial.flags.closed = polyline.closed;
        EntityType::Polyline3D(spatial)
    }
}

/// A 2D solid, its corners round its outline written in AutoCAD's order:
/// the third and fourth crossed, and a triangle's last corner twice.
fn dwg_solid(solid: &Solid) -> DwgSolid {
    let c = |i: usize| vector(solid.corners[i.min(solid.corners.len() - 1)]);
    match solid.corners.len() {
        3 => DwgSolid::new(c(0), c(1), c(2), c(2)),
        _ => DwgSolid::new(c(0), c(1), c(3), c(2)),
    }
}

fn dwg_ellipse(ellipse: &Ellipse) -> DwgEllipse {
    let mut dwg_ellipse = DwgEllipse::from_center_axes(
        vector(ellipse.center),
        vector(ellipse.major_axis),
        ellipse.ratio,
    );
    if !ellipse.is_whole() {
        let start = parameter(ellipse.start_angle, ellipse.ratio);
        let end = parameter(ellipse.end_angle, ellipse.ratio);
        dwg_ellipse.start_parameter = start;
        // The curve runs anticlockwise from start to end. AutoCAD keeps the
        // end parameter above the start, so an arc that crosses the major
        // axis ends past 2π; written that way, AutoCAD reads back the bytes
        // as written.
        dwg_ellipse.end_parameter = if end > start { end } else { end + TAU };
    }
    dwg_ellipse
}

/// The DWG's parameter, in `[0, 2π)`, for the point of an ellipse at
/// `degrees` from its major axis. The point at parameter `t` is
/// `centre + cos t · major + sin t · minor`, so its angle `θ` from the major
/// axis has `tan θ = ratio · tan t`.
fn parameter(degrees: f64, ratio: f64) -> f64 {
    let theta = degrees.to_radians();
    theta.sin().atan2(ratio * theta.cos()).rem_euclid(TAU)
}

/// The angle from the major axis, in degrees, of the point of an ellipse at
/// parameter `t`.
fn ellipse_angle(t: f64, ratio: f64) -> f64 {
    normalize_degrees((ratio * t.sin()).atan2(t.cos()).to_degrees())
}

fn dwg_spline(spline: &Spline) -> DwgSpline {
    let mut dwg_spline = if spline.fit_points.is_empty() {
        DwgSpline::from_control_points(
            i32::from(spline.degree),
            spline.control_points.iter().map(|&p| vector(p)).collect(),
        )
    } else {
        DwgSpline::from_fit_points(spline.fit_points.iter().map(|&p| vector(p)).collect())
    };
    dwg_spline.knot_parameterization = CHORD_KNOTS;
    dwg_spline
}

/// Reads the drawing in the DWG at `path`: its linetypes, its layers and
/// the objects in its model space.
pub(crate) fn read(path: &Path) -> Result<DrawingContents, Error> {
    let bytes = read_file_bytes(path)?;
    read_bytes(&bytes, path)
}

/// Reads the drawing from `bytes`, already read from `path`, so that a
/// fingerprint and the geometry come from the exact same bytes.
pub(crate) fn read_bytes(bytes: &[u8], path: &Path) -> Result<DrawingContents, Error> {
    let (drawing, stats) = read_drawing_bytes_with_stats(bytes, path, false)?;
    contents(&drawing, &stats, raw_dwg_version(bytes))
}

/// Reads the drawing in the DWG at `path` as [`read`] does; when the DWG
/// library cannot read the file whole, reads it again past the records it
/// cannot read, as AutoCAD's RECOVER reads a damaged drawing, though
/// repairing nothing, and the decode report says so and names each record
/// passed. Refused when even that read fails or yields no usable drawing.
#[cfg(test)]
pub(crate) fn read_past_damage(path: &Path) -> Result<DrawingContents, Error> {
    let bytes = read_file_bytes(path)?;
    read_bytes_past_damage(&bytes, path)
}

/// Reads the drawing from `bytes`, already read from `path`, past damaged
/// records as [`read_past_damage`] does.
pub(crate) fn read_bytes_past_damage(bytes: &[u8], path: &Path) -> Result<DrawingContents, Error> {
    let raw_version = raw_dwg_version(bytes);
    let strict = match read_drawing_bytes_with_stats(bytes, path, false) {
        Ok((drawing, stats)) => return contents(&drawing, &stats, raw_version),
        Err(error) => error,
    };
    let Ok((drawing, stats)) = read_drawing_bytes_with_stats(bytes, path, true) else {
        return Err(strict);
    };
    let mut read = contents(&drawing, &stats, raw_version)?;
    read.decode.decoded = false;
    read.decode.read_past_damage = Some(strict.message);
    Ok(read)
}

/// The 6-byte ASCII version tag from the start of a DWG file (such as
/// `AC1006`, `AC1021`, `AC1032`, or `AD1021`), preserving the exact version
/// the original arrived in even when the DWG library normalizes older
/// versions internally (such as `AC1006` -> `AC1009`).
fn raw_dwg_version(bytes: &[u8]) -> Option<String> {
    let header = bytes.get(..6)?;
    let text = std::str::from_utf8(header).ok()?;
    let digits = text
        .strip_prefix("AC")
        .or_else(|| text.strip_prefix("AD"))?;
    (digits.len() == 4 && digits.bytes().all(|b| b.is_ascii_digit())).then(|| text.to_owned())
}

/// What the core reads of `drawing`, read with `stats`.
fn contents(
    drawing: &CadDocument,
    stats: &acadrust::ReadStats,
    raw_version: Option<String>,
) -> Result<DrawingContents, Error> {
    let mut linetypes = Vec::new();
    let mut linetypes_not_read = 0;
    for dwg_linetype in drawing.line_types.iter() {
        if BUILT_IN_LINETYPES
            .iter()
            .any(|built_in| built_in.eq_ignore_ascii_case(&dwg_linetype.name))
        {
            continue;
        }
        match read_linetype(dwg_linetype) {
            Some(linetype) => linetypes.push(linetype),
            None => linetypes_not_read += 1,
        }
    }
    let named_mode = !drawing.header.plotstyle_mode;
    let ps_map = page_setup::plot_style_handle_map(drawing);
    let mut layers = Vec::new();
    let mut layers_not_read = 0;
    for dwg_layer in drawing.layers.iter() {
        // AutoCAD's own `Defpoints` is read with the dimensions.
        if dimension::is_own_defpoints(dwg_layer) {
            continue;
        }
        match read_layer(dwg_layer, named_mode, &ps_map) {
            Some(layer) => layers.push(layer),
            None => layers_not_read += 1,
        }
    }
    let (text_styles, text_styles_not_read) = text::read_text_styles(drawing);
    let (table_styles, table_styles_not_read) = table::read_table_styles(drawing);
    let (mleader_styles, mleader_styles_not_read) = leader::read_mleader_styles(drawing);
    let (dimension_styles, dimension_styles_not_read) = dimension::read_dimension_styles(drawing);
    // A leader's or a dimension style's arrowhead block is read with them.
    let mut arrow_blocks = leader::arrow_blocks(drawing);
    arrow_blocks.extend(dimension::style_arrow_blocks(drawing));
    // A dimension's own block holds what it draws, and is read with it.
    let dimension_blocks = dimension::dimension_blocks(drawing);
    // The sheet the drawing holds: model space, and the paper space of
    // each layout the core reads, whatever tab it was saved on.
    let units = unit(drawing.header.insertion_units);
    let (read_layouts, layouts_not_read, extra_layout_setups) =
        layout::read_layouts(drawing, units);
    let blocks_of: Vec<acadrust::types::Handle> = read_layouts.iter().map(|l| l.block).collect();
    let on_sheet = sheet_objects(drawing, &blocks_of);
    // A table's own block holds what it draws, and is read with the table.
    let table_blocks: Vec<(Option<acadrust::types::Handle>, String)> = on_sheet
        .iter()
        .map(|(e, _)| *e)
        .filter_map(|e| match e {
            EntityType::Table(t) => {
                let (handle, name) = table::block_of(t);
                Some((handle, name.to_owned()))
            }
            _ => None,
        })
        .collect();
    // A block that inserts another is read once that one is: each round
    // reads every block it can with the blocks read before it, until a
    // round reads none.
    let records: Vec<&BlockRecord> = drawing
        .block_records
        .iter()
        .filter(|r| !(r.is_model_space() || r.is_paper_space() || r.is_layout()))
        .filter(|r| {
            !table_blocks.iter().any(|(handle, name)| {
                *handle == Some(r.handle) || (!name.is_empty() && *name == r.name)
            })
        })
        .filter(|r| {
            !dimension_blocks
                .iter()
                .any(|n| n.eq_ignore_ascii_case(&r.name))
        })
        .collect();
    // An arrowhead block that only leaders draw is read with them, and is
    // neither a block of the drawing's nor counted; one an insert or a
    // multileader's block content places is a block like any other.
    let content_blocks = leader::content_blocks(drawing);
    let inserted: Vec<String> = drawing
        .model_space_entities()
        .chain(
            // A dimension's own block places its arrowheads, which are
            // read with it, not as inserts of the drawing's.
            drawing
                .block_records
                .iter()
                .filter(|r| {
                    !dimension_blocks
                        .iter()
                        .any(|n| n.eq_ignore_ascii_case(&r.name))
                })
                .flat_map(|r| drawing.entities_in_block(&r.name)),
        )
        .filter_map(|e| match e {
            EntityType::Insert(i) => Some(i.block_name.to_lowercase()),
            _ => None,
        })
        .collect();
    let leaders_only = |r: &BlockRecord| {
        arrow_blocks.contains(&r.handle)
            && !inserted.contains(&r.name.to_lowercase())
            && !content_blocks.contains(&r.handle)
    };
    let is_xref_record =
        |r: &BlockRecord| r.flags.is_xref || r.flags.is_xref_overlay || !r.xref_path.is_empty();
    let xrefs: Vec<Xref> = records
        .iter()
        .filter(|r| is_xref_record(r))
        .map(|r| Xref {
            name: r.name.clone(),
            path: r.xref_path.clone(),
            kind: if r.flags.is_xref_overlay {
                XrefKind::Overlay
            } else {
                XrefKind::Attach
            },
            unloaded: r.flags.is_xref_unloaded,
        })
        .collect();
    let ad_evaluated_blocks = dynamic_block::agentdrafter_evaluated_blocks(drawing);
    let mut read: Vec<Option<ReadBlock>> = records.iter().map(|_| None).collect();
    loop {
        let known: Vec<Block> = read.iter().flatten().map(|r| r.block.clone()).collect();
        let mut more = false;
        for (slot, record) in read.iter_mut().zip(&records) {
            if slot.is_none() {
                if let Some(definition) = read_block(
                    drawing,
                    record,
                    &known,
                    &ad_evaluated_blocks,
                    named_mode,
                    &ps_map,
                ) {
                    *slot = Some(definition);
                    more = true;
                }
            }
        }
        if !more {
            break;
        }
    }
    let blocks_not_read = read
        .iter()
        .zip(&records)
        .filter(|(b, r)| {
            b.is_none()
                && !leaders_only(r)
                && !is_xref_record(r)
                && !extra::is_center_mark_block(drawing, &r.name)
        })
        .count();
    let mut blocks = Vec::new();
    let mut block_handles = Vec::new();
    let mut block_ids_and_names = Vec::new();
    for (read_block, _) in read
        .into_iter()
        .zip(&records)
        .filter(|(_, r)| !leaders_only(r))
    {
        if let Some(read_block) = read_block {
            blocks.push(read_block.block);
            block_handles.push(read_block.handles);
            block_ids_and_names.push(read_block.ids_and_names);
        }
    }
    let mut objects = Vec::new();
    let mut ids_and_names = Vec::new();
    let mut handles = Vec::new();
    let mut not_read: Vec<(String, Option<String>)> = Vec::new();
    // An older leader's annotation is read with the leader, not on its own:
    // when the leader is not read, nor is its annotation.
    let mut leaders = Vec::new();
    let mut annotations = Vec::new();
    for &(dwg_object, _) in &on_sheet {
        if let EntityType::Leader(l) = dwg_object {
            let read = read_properties_in(&l.common, named_mode, &ps_map)
                .filter(|_| !l.common.invisible)
                .and_then(|properties| {
                    let (mut read, _) = leader::read_leader(drawing, l, layer_of(&l.common))?;
                    read.properties = properties;
                    Some(read)
                });
            if let Some(annotation) = leader::annotation_of(drawing, l) {
                annotations.push((annotation, read.is_some()));
            }
            leaders.push((l.common.handle, read));
        }
    }
    // A viewport is read before its boundary is met, so that a boundary
    // whose viewport is read is read with it, not on its own.
    let mut viewports: Vec<(acadrust::types::Handle, Option<Object>)> = Vec::new();
    let mut boundaries = Vec::new();
    for &(dwg_object, space) in &on_sheet {
        if let (EntityType::Viewport(v), Some(_)) = (dwg_object, space) {
            let read = read_viewport_of(drawing, v, named_mode, &ps_map);
            if read.is_some() && !v.clip_boundary_handle.is_null() {
                boundaries.push(v.clip_boundary_handle);
            }
            viewports.push((v.common.handle, read));
        }
    }
    let mut spaces = Vec::new();
    for &(dwg_object, space) in &on_sheet {
        let handle = dwg_object.common().handle;
        if boundaries.contains(&handle) {
            continue;
        }
        if let Some((_, read_with_its_leader)) = annotations.iter().find(|(a, _)| *a == handle) {
            if !read_with_its_leader {
                not_read.push((kind_of(dwg_object), Some(handle_text(handle))));
            }
            continue;
        }
        let read = match dwg_object {
            EntityType::Leader(l) => leaders
                .iter()
                .find(|(handle, _)| *handle == l.common.handle)
                .and_then(|(_, read)| read.clone())
                .map(Object::Leader),
            EntityType::MultiLeader(m) => read_properties_in(&m.common, named_mode, &ps_map)
                .filter(|_| !m.common.invisible)
                .and_then(|properties| {
                    let mut read = leader::read_mleader(
                        drawing,
                        m,
                        &mleader_styles,
                        &blocks,
                        layer_of(&m.common),
                    )?;
                    read.properties = properties;
                    Some(Object::MLeader(read))
                }),
            EntityType::Dimension(d) => read_properties_in(&d.base().common, named_mode, &ps_map)
                .filter(|_| !d.base().common.invisible)
                .and_then(|properties| {
                    let mut read = dimension::read_dimension(
                        d,
                        &dimension_styles,
                        layer_of(&d.base().common),
                    )?;
                    read.properties = properties;
                    Some(Object::Dimension(read))
                }),
            EntityType::Table(t) => read_properties_in(&t.common, named_mode, &ps_map)
                .filter(|_| !t.common.invisible)
                .and_then(|properties| {
                    let mut read =
                        table::read_table(drawing, t, &table_styles, layer_of(&t.common))?;
                    read.properties = properties;
                    Some(Object::Table(read))
                }),
            EntityType::Viewport(v) if space.is_some() => viewports
                .iter()
                .find(|(h, _)| *h == v.common.handle)
                .and_then(|(_, read)| read.clone()),
            _ => read_object_in(
                Some(drawing),
                dwg_object,
                &blocks,
                &xrefs,
                named_mode,
                &ps_map,
            ),
        };
        match read {
            Some(object) => {
                objects.push(object);
                spaces.push(space);
                handles.push(handle_text(handle));
                ids_and_names.push(IdAndName {
                    id: ids::read_id(dwg_object.common()),
                    name: ids::read_name(dwg_object.common()),
                });
            }
            None => not_read.push((kind_of(dwg_object), Some(handle_text(handle)))),
        }
    }
    // Attach AutoCAD FIELD definitions read from extension dictionaries onto
    // Text, MText, and Insert attributes, mapping referenced entity handles
    // back to their local IDs (or `#<HEX>` when the target has no ID).
    if !drawing.objects.is_empty() {
        let mut handle_to_id: std::collections::HashMap<u64, String> =
            std::collections::HashMap::with_capacity(objects.len());
        for (h_hex, id_name) in handles.iter().zip(&ids_and_names) {
            if let Ok(h_val) = u64::from_str_radix(h_hex, 16) {
                let label = id_name
                    .id
                    .as_deref()
                    .map(|full| {
                        full.split_once(crate::description::ID_SEPARATOR)
                            .map_or(full, |(_, local)| local)
                            .to_owned()
                    })
                    .unwrap_or_else(|| format!("#{h_hex}"));
                handle_to_id.insert(h_val, label);
            }
        }
        for (obj, h_hex) in objects.iter_mut().zip(&handles) {
            let Ok(h_val) = u64::from_str_radix(h_hex, 16) else {
                continue;
            };
            let Some(dwg_ent) = drawing.get_entity(acadrust::types::Handle::new(h_val)) else {
                continue;
            };
            match (obj, dwg_ent) {
                (Object::Text(t), EntityType::Text(dt)) => {
                    t.field = field::read_host_field(
                        drawing,
                        dt.common.xdictionary_handle,
                        &handle_to_id,
                        false,
                    );
                }
                (Object::MText(t), EntityType::MText(dt)) => {
                    t.field = field::read_host_field(
                        drawing,
                        dt.common.xdictionary_handle,
                        &handle_to_id,
                        true,
                    );
                }
                (Object::Insert(ins), EntityType::Insert(di)) => {
                    for (att, dwg_att) in ins.attributes.iter_mut().zip(&di.attributes) {
                        att.field = field::read_host_field(
                            drawing,
                            dwg_att.common.xdictionary_handle,
                            &handle_to_id,
                            false,
                        );
                    }
                }
                _ => {}
            }
        }
    }
    let (decode, skipped_for_sheet) = decode_report(drawing, stats);
    for (kind, handle) in skipped_for_sheet {
        let already = handle.as_ref().is_some_and(|h| {
            not_read
                .iter()
                .any(|(_, existing)| existing.as_ref() == Some(h))
        });
        if !already {
            not_read.push((kind, handle));
        }
    }
    let mut layout_setups = Vec::with_capacity(read_layouts.len());
    let mut layouts = read_layouts.into_iter().enumerate().map(|(k, read)| {
        layout_setups.push(read.setup);
        crate::model::Layout {
            name: read.name,
            paper: read.paper,
            page_setup: read.page_setup,
            paper_space: spaces.iter().map(|s| *s == Some(k)).collect(),
        }
    });
    let layout = layouts.next();
    let other_layouts: Vec<crate::model::Layout> = layouts.collect();
    let paper = layout.as_ref().and_then(|l| l.paper.clone());
    let page_setup = layout::read_model_page_setup(drawing, units);
    let plot_style_mode = if named_mode {
        crate::model::PlotStyleMode::Named
    } else {
        crate::model::PlotStyleMode::ColorDependent
    };
    let (annotation_scales, annotation_scale, anno_all_visible, scale_handle_map) =
        annotation_scale::read_scales_and_variables(drawing);
    let active_scale_factor = annotation_scale
        .as_deref()
        .and_then(|name| {
            annotation_scales
                .iter()
                .find(|s| s.name.eq_ignore_ascii_case(name))
                .cloned()
                .or_else(|| crate::model::AnnotationScale::from_name(name))
        })
        .map_or(1.0, |s| s.factor());
    for ((obj, h_hex), space) in objects.iter_mut().zip(&handles).zip(&spaces) {
        if !obj.supports_annotation_scale() {
            continue;
        }
        let Ok(h_val) = u64::from_str_radix(h_hex, 16) else {
            continue;
        };
        let Some(dwg_ent) = drawing.get_entity(acadrust::types::Handle::new(h_val)) else {
            continue;
        };
        let (is_anno, scale_names, default_factor) =
            annotation_scale::read_entity_annotation(drawing, dwg_ent, &scale_handle_map);
        let style_or_block_anno = match obj {
            Object::Text(t) => text_styles
                .iter()
                .any(|s| s.name.eq_ignore_ascii_case(&t.style) && s.annotative),
            Object::MText(m) => text_styles
                .iter()
                .any(|s| s.name.eq_ignore_ascii_case(&m.style) && s.annotative),
            Object::Dimension(d) => dimension_styles
                .iter()
                .any(|s| s.name.eq_ignore_ascii_case(&d.dimension_style) && s.annotative),
            Object::MLeader(m) => mleader_styles
                .iter()
                .any(|s| s.name.eq_ignore_ascii_case(&m.mleader_style) && s.annotative),
            Object::Insert(i) => blocks
                .iter()
                .any(|b| b.name.eq_ignore_ascii_case(&i.block) && b.annotative),
            _ => false,
        };
        if !is_anno && scale_names.is_empty() && !style_or_block_anno {
            continue;
        }
        let eff_factor = default_factor.unwrap_or(active_scale_factor);
        let unscale = space.is_none() && (eff_factor - 1.0).abs() > 1e-12 && eff_factor > 0.0;
        match obj {
            Object::Text(t) => {
                t.annotative = is_anno || !scale_names.is_empty();
                t.annotation_scales = scale_names;
                if unscale {
                    t.height *= eff_factor;
                }
            }
            Object::MText(m) => {
                m.annotative = is_anno || !scale_names.is_empty();
                m.annotation_scales = scale_names;
                if unscale {
                    m.height *= eff_factor;
                    m.width *= eff_factor;
                }
            }
            Object::Dimension(d) => {
                d.annotative = is_anno || !scale_names.is_empty();
                d.annotation_scales = scale_names;
            }
            Object::MLeader(m) => {
                m.annotative = is_anno || !scale_names.is_empty();
                m.annotation_scales = scale_names;
                if unscale {
                    let style = mleader_styles
                        .iter()
                        .find(|s| s.name.eq_ignore_ascii_case(&m.mleader_style));
                    let near_style = |v: f64, sv: f64| (v - sv).abs() <= 1e-9 * sv.abs().max(1.0);
                    m.landing = m.landing.and_then(|v| {
                        let u = v * eff_factor;
                        if style.is_some_and(|s| near_style(u, s.landing)) {
                            None
                        } else {
                            Some(u)
                        }
                    });
                    m.gap = m.gap.and_then(|v| {
                        let u = v * eff_factor;
                        if style.is_some_and(|s| near_style(u, s.landing_gap)) {
                            None
                        } else {
                            Some(u)
                        }
                    });
                    m.arrow_size = m.arrow_size.and_then(|v| {
                        let u = v * eff_factor;
                        if style.is_some_and(|s| near_style(u, s.arrow_size)) {
                            None
                        } else {
                            Some(u)
                        }
                    });
                    match &mut m.content {
                        crate::model::MLeaderContent::Text(tc) => {
                            tc.height = tc.height.and_then(|v| {
                                let u = v * eff_factor;
                                if style.is_some_and(|s| near_style(u, s.text_height)) {
                                    None
                                } else {
                                    Some(u)
                                }
                            });
                        }
                        crate::model::MLeaderContent::Block(bc) => {
                            bc.scale *= eff_factor;
                        }
                    }
                }
            }
            Object::Hatch(h) => {
                h.annotative = is_anno || !scale_names.is_empty();
                h.annotation_scales = scale_names;
                if unscale && !h.is_solid() {
                    h.scale *= eff_factor;
                }
            }
            Object::Insert(ins) => {
                ins.annotative = is_anno || !scale_names.is_empty();
                ins.annotation_scales = scale_names;
                if unscale {
                    ins.scale *= eff_factor;
                }
            }
            _ => {}
        }
    }
    let groups = group::read_groups(drawing, &handles);
    Ok(DrawingContents {
        dwg_version: raw_version.unwrap_or_else(|| drawing.version.to_string()),
        units,
        linetypes,
        linetypes_not_read,
        layers,
        layers_not_read,
        text_styles,
        text_styles_not_read,
        table_styles,
        table_styles_not_read,
        mleader_styles,
        mleader_styles_not_read,
        dimension_styles,
        dimension_styles_not_read,
        mline_styles: extra::read_mline_styles(drawing),
        pdmode: drawing.header.point_display_mode,
        pdsize: drawing.header.point_display_size,
        blocks,
        xrefs,
        block_handles,
        block_ids_and_names,
        blocks_not_read,
        objects,
        ids_and_names,
        objects_not_read: not_read.len(),
        ltscale: drawing.header.linetype_scale,
        wipeout_frame: wipeout::read_wipeout_frame(drawing),
        image_frame: media::read_image_frame(drawing),
        custom_properties: field::read_custom_properties(drawing),
        groups,
        annotation_scales,
        annotation_scale,
        anno_all_visible,
        layout,
        other_layouts,
        opens_on: layout::opened_on(drawing).map(|(name, _)| name),
        paper,
        page_setup,
        plot_style_mode,
        layouts_not_read,
        handles,
        not_read: RecordsOfKind::group(not_read),
        decode,
        layout_setups,
        extra_layout_setups,
    })
}

/// The handle of a DWG's record in hexadecimal, as AutoCAD lists it, such
/// as `2F`.
fn handle_text(handle: acadrust::types::Handle) -> String {
    format!("{:X}", handle.value())
}

/// Returns `name` when non-empty, otherwise computes `fallback`.
fn non_empty(name: &str, fallback: impl FnOnce() -> String) -> String {
    if name.is_empty() {
        fallback()
    } else {
        name.to_owned()
    }
}

/// The kind of a DWG's object, as the DWG names it, such as `LINE`: a
/// custom object's own name for one the DWG library does not decode.
fn kind_of(object: &EntityType) -> String {
    use acadrust::entities::{Dimension as DwgDimension, ExtendedEntityData, SurfaceKind};
    match object {
        EntityType::Unknown(unknown) => non_empty(&unknown.dxf_name, || {
            format!("DWG_TYPE_{}", unknown.dwg_type_code)
        }),
        EntityType::Extended(extended) => match &extended.data {
            ExtendedEntityData::RegisteredClass(data) => non_empty(&data.dxf_name, || {
                non_empty(&data.cpp_class_name, || "REGISTERED_CLASS".to_owned())
            }),
            ExtendedEntityData::Arch(data) => non_empty(&data.dxf_name, || {
                format!("DWG_TYPE_{}", data.dwg_type_code)
            }),
            _ => non_empty(extended.class_name(), || "EXTENDED_ENTITY".to_owned()),
        },
        EntityType::Solid(solid) if solid.is_trace => "TRACE".to_owned(),
        EntityType::Surface(surface) => match surface.kind {
            SurfaceKind::Generic => "SURFACE",
            SurfaceKind::Plane => "PLANESURFACE",
            SurfaceKind::Extruded => "EXTRUDEDSURFACE",
            SurfaceKind::Lofted => "LOFTEDSURFACE",
            SurfaceKind::Revolved => "REVOLVEDSURFACE",
            SurfaceKind::Swept => "SWEPTSURFACE",
            SurfaceKind::Nurb => "NURBSURFACE",
        }
        .to_owned(),
        // A fixed dimension's own kind, such as `DIMENSION_LINEAR`, is the
        // DWG's `DIMENSION`; class-based arc and large-radial dimensions keep
        // their own DXF names.
        EntityType::Dimension(dimension) => match dimension.as_ref() {
            DwgDimension::Arc(_) => "ARC_DIMENSION",
            DwgDimension::LargeRadial(_) => "LARGE_RADIAL_DIMENSION",
            _ => "DIMENSION",
        }
        .to_owned(),
        other => other.as_entity().entity_type().to_owned(),
    }
}

/// The kind of an object the DWG library kept only as raw, proxy or opaque
/// bytes rather than decoding into a typed model.
fn undecoded_object_kind(object: &EntityType) -> Option<String> {
    use acadrust::entities::ExtendedEntityData;
    match object {
        EntityType::Unknown(_) => Some(kind_of(object)),
        EntityType::Extended(extended)
            if matches!(
                extended.data,
                ExtendedEntityData::Proxy(_)
                    | ExtendedEntityData::Arch(_)
                    | ExtendedEntityData::Format(_)
                    | ExtendedEntityData::LayoutPrintConfig(_)
                    | ExtendedEntityData::RegisteredClass(_)
            ) =>
        {
            Some(kind_of(object))
        }
        _ => None,
    }
}

/// The handle and kind of a non-graphical object the DWG library kept only
/// as raw, proxy or opaque bytes rather than decoding whole.
fn undecoded_non_graphical(
    object: &acadrust::objects::ObjectType,
) -> Option<(acadrust::types::Handle, String)> {
    use acadrust::objects::{ClassObjectData, DgnLineStyleData, ObjectType};
    match object {
        ObjectType::Unknown {
            type_name, handle, ..
        } => Some((*handle, non_empty(type_name, || UNREADABLE.to_owned()))),
        ObjectType::Arch(arch) => Some((
            arch.handle,
            non_empty(&arch.dxf_name, || "AEC_OBJECT".to_owned()),
        )),
        ObjectType::ProxyObject(proxy) => Some((proxy.handle, "ACAD_PROXY_OBJECT".to_owned())),
        ObjectType::RegisteredClass(data) => Some((
            data.handle,
            non_empty(&data.dxf_name, || {
                non_empty(&data.cpp_class_name, || "REGISTERED_CLASS".to_owned())
            }),
        )),
        ObjectType::DgnLineStyle(dgn) => match &dgn.data {
            DgnLineStyleData::Definition {
                undecoded_fields: Some(_),
                ..
            } => Some((dgn.handle, "LSDEFINITION".to_owned())),
            DgnLineStyleData::Registered {
                dxf_name, payload, ..
            } if !payload.records.is_empty() => Some((
                dgn.handle,
                non_empty(dxf_name, || "DGN_LINESTYLE".to_owned()),
            )),
            _ => None,
        },
        ObjectType::ClassObject(data) => match &data.data {
            ClassObjectData::CsacDocumentOptions(options) if options.raw_dwg_data.is_some() => {
                Some((data.handle, "CSACDOCUMENTOPTIONS".to_owned()))
            }
            ClassObjectData::VbaProject(vba) if vba.storage.compound_file.is_none() => {
                Some((data.handle, "VBA_PROJECT".to_owned()))
            }
            _ => None,
        },
        ObjectType::XRecord(xrecord)
            if !xrecord.entries_complete && !xrecord.raw_data.is_empty() =>
        {
            Some((xrecord.handle, "XRECORD".to_owned()))
        }
        _ => None,
    }
}

/// The DXF name of a DWG's fixed or sentinel record type code, when known.
fn fixed_type_name(code: i16) -> Option<&'static str> {
    Some(match code {
        1 => "TEXT",
        2 => "ATTRIB",
        3 => "ATTDEF",
        4 => "BLOCK",
        5 => "ENDBLK",
        6 => "SEQEND",
        7 => "INSERT",
        8 => "MINSERT",
        10..=14 => "VERTEX",
        15 | 16 | 29 | 30 => "POLYLINE",
        17 => "ARC",
        18 => "CIRCLE",
        19 => "LINE",
        20..=26 => "DIMENSION",
        27 => "POINT",
        28 => "3DFACE",
        31 => "SOLID",
        32 => "TRACE",
        33 => "SHAPE",
        34 => "VIEWPORT",
        35 => "ELLIPSE",
        36 => "SPLINE",
        37 => "REGION",
        38 => "3DSOLID",
        39 => "BODY",
        40 => "RAY",
        41 => "XLINE",
        42 => "DICTIONARY",
        43 => "OLEFRAME",
        44 => "MTEXT",
        45 => "LEADER",
        46 => "TOLERANCE",
        47 => "MLINE",
        48 => "BLOCK_CONTROL",
        49 => "BLOCK_RECORD",
        50 => "LAYER_CONTROL",
        51 => "LAYER",
        52 => "STYLE_CONTROL",
        53 => "STYLE",
        56 => "LTYPE_CONTROL",
        57 => "LTYPE",
        60 => "VIEW_CONTROL",
        61 => "VIEW",
        62 => "UCS_CONTROL",
        63 => "UCS",
        64 => "VPORT_CONTROL",
        65 => "VPORT",
        66 => "APPID_CONTROL",
        67 => "APPID",
        68 => "DIMSTYLE_CONTROL",
        69 => "DIMSTYLE",
        70 => "VPENT_HDR_CONTROL",
        71 => "VPENT_HDR",
        72 => "GROUP",
        73 => "MLINESTYLE",
        74 => "OLE2FRAME",
        75 => "DUMMY",
        76 => "LONG_TRANSACTION",
        77 => "LWPOLYLINE",
        78 => "HATCH",
        79 => "XRECORD",
        80 => "ACDBPLACEHOLDER",
        81 => "VBA_PROJECT",
        82 => "LAYOUT",
        105 => "TABLECONTENT",
        106 => "TABLESTYLE",
        120 => "ACDBDICTIONARYWDFLT",
        121 => "DICTIONARYVAR",
        122 => "PLOTSETTINGS",
        123 => "MLEADERSTYLE",
        124 => "IMAGEDEF",
        125 => "IMAGEDEF_REACTOR",
        126 => "SCALE",
        127 => "SORTENTSTABLE",
        128 => "RASTERVARIABLES",
        129 => "DBCOLOR",
        130 => "WIPEOUTVARIABLES",
        131 => "GEODATA",
        132 => "BLOCKVISIBILITYPARAMETER",
        133 => "ACDB_BLOCKREPRESENTATION_DATA",
        134 => "SPATIAL_FILTER",
        135 => "PDFDEFINITION",
        136 => "DWFDEFINITION",
        137 => "DGNDEFINITION",
        138 => "FIELD",
        139 => "FIELDLIST",
        140 => "VISUALSTYLE",
        141 => "MATERIAL",
        142 => "OBJECT_PTR",
        143 => "TABLETEMPLATE",
        498 => "ACAD_PROXY_ENTITY",
        499 => "ACAD_PROXY_OBJECT",
        -1 => "IMAGE",
        -2 => "MESH",
        -3 => "MULTILEADER",
        -4 => "WIPEOUT",
        -5 => "SURFACE",
        -6 => "PLANESURFACE",
        -7 => "EXTRUDEDSURFACE",
        -8 => "LOFTEDSURFACE",
        -9 => "REVOLVEDSURFACE",
        -10 => "SWEPTSURFACE",
        -11 => "NURBSURFACE",
        -12 => "PDFUNDERLAY",
        -13 => "DWFUNDERLAY",
        -14 => "DGNUNDERLAY",
        -15 => "HELIX",
        -16 => "ACAD_TABLE",
        -17 => "LIGHT",
        -18 => "ARC_DIMENSION",
        -19 => "LARGE_RADIAL_DIMENSION",
        -20 => "CAMERA",
        -21 => "SECTIONOBJECT",
        -22 => "ARCALIGNEDTEXT",
        -23 => "RTEXT",
        -24 => "POSITIONMARKER",
        -25 => "COORDINATION_MODEL",
        -26 => "ACDBPOINTCLOUD",
        -27 => "ACDBPOINTCLOUDEX",
        -28 => "MPOLYGON",
        _ => return None,
    })
}

/// The name of a DWG record type code from a diagnostic: fixed code, class
/// name, or `DWG_TYPE_<code>` when unknown.
fn type_code_name(code: i16, drawing: &CadDocument) -> String {
    if let Some(fixed) = fixed_type_name(code) {
        return fixed.to_owned();
    }
    drawing
        .classes
        .iter()
        .find(|c| c.class_number == code)
        .filter(|c| !c.dxf_name.is_empty())
        .map_or_else(|| format!("DWG_TYPE_{code}"), |c| c.dxf_name.clone())
}

/// Whether a DWG record type code from a skipped-record diagnostic is, or
/// could be, a graphical sheet object rather than a table entry or
/// non-graphical object.
fn is_graphical_or_unknown_type(code: i16, drawing: &CadDocument) -> bool {
    if matches!(code, -28..=-1 | 1..=41 | 43..=47 | 74 | 77 | 78 | 498) {
        return true;
    }
    if matches!(
        code,
        42 | 48..=73 | 75 | 76 | 79..=82 | 105 | 106 | 120..=143 | 499
    ) {
        return false;
    }
    if code >= 500 {
        return drawing
            .classes
            .iter()
            .find(|c| c.class_number == code)
            .is_none_or(|c| c.is_an_entity);
    }
    true
}

/// Resolves a diagnostic's `record_type` to `(kind, could_be_sheet_object)`.
fn diagnostic_record_kind(record_type: Option<&str>, drawing: &CadDocument) -> (String, bool) {
    match record_type {
        None | Some("") => (UNREADABLE.to_owned(), true),
        Some(text) => match text.parse::<i16>() {
            Ok(code) => (
                type_code_name(code, drawing),
                is_graphical_or_unknown_type(code, drawing),
            ),
            Err(_) => (text.to_owned(), true),
        },
    }
}

/// The `not_decoded` kind for a section-level extraction or decode failure
/// diagnostic from the DWG library, when `code` names one.
fn failed_section_kind(code: &str) -> Option<&'static str> {
    Some(match code {
        "classes-decode-failed" | "classes-extract-failed" => "FAILED_CLASSES_SECTION",
        "header-decode-failed" | "header-extract-failed" => "FAILED_HEADER_SECTION",
        "template-decode-failed" => "FAILED_TEMPLATE_SECTION",
        "handles-decode-failed" | "handles-extract-failed" => "FAILED_HANDLES_SECTION",
        "objects-init-failed" | "objects-extract-failed" => "FAILED_OBJECTS_SECTION",
        "decryption-failed" => "FAILED_SECURITY_SECTION",
        "file-header-read-failed" => "FAILED_FILE_HEADER",
        "legacy-read-failed" => "FAILED_LEGACY_READ",
        _ => return None,
    })
}

/// Whether `stats` carries any section extraction/decode failure or
/// truncation diagnostic.
fn has_section_failure_or_truncation(stats: &acadrust::ReadStats) -> bool {
    stats.diagnostics.iter().any(|d| {
        matches!(
            d.code.as_str(),
            "section-read-truncated" | "section-page-skipped"
        ) || failed_section_kind(&d.code).is_some()
    })
}

/// Whether a `NotificationType::Warning` message from acadrust is an
/// informational trace emitted on healthy DWG reads (or a failsafe summary
/// already recorded under `not_decoded`), rather than a genuine reader
/// warning.
fn is_informational_trace_warning(message: &str) -> bool {
    [
        "Reading DWG file version: ",
        "AC15 file header: ",
        "AC18 inner header: ",
        "AC18: Read ",
        "AC1021 header: ",
        "AC1021 Header CRC-64 extracted: ",
        "AC1021 CRC Seeds: ",
        "AC1021 Pages Map CRC: ",
        "AC1021 Sections Map CRC: ",
        "AC1021: Read ",
        "  Section '",
        "AcDs: attached ",
        "DWG build summary: ",
        "Failsafe: ",
    ]
    .iter()
    .any(|prefix| message.starts_with(prefix))
}

/// Whether the DWG library decoded every record of `drawing`, read with
/// `stats`, and the records it did not: each object and non-graphical
/// object it keeps only as raw, proxy or opaque bytes, each record cut
/// short, each failed or truncated section/page, and each damaged record
/// skipped, by the kind the file gives it, with its handle. Also returns
/// skipped graphical or unidentified records so `not_read` reflects what
/// no output can draw.
fn decode_report(
    drawing: &CadDocument,
    stats: &acadrust::ReadStats,
) -> (DecodeReport, Vec<(String, Option<String>)>) {
    let mut not_decoded: Vec<(u64, String)> = drawing
        .entities()
        .filter_map(|e| undecoded_object_kind(e).map(|kind| (e.common().handle.value(), kind)))
        .collect();
    not_decoded.extend(
        drawing
            .objects
            .values()
            .filter_map(undecoded_non_graphical)
            .map(|(handle, kind)| (handle.value(), kind)),
    );
    let mut handleless_not_decoded: Vec<String> = Vec::new();
    let mut skipped_for_sheet: Vec<(u64, String)> = Vec::new();
    let mut skipped_handleless: Vec<String> = Vec::new();
    let mut diagnostics_truncated = drawing.notifications.omitted_count() > 0;
    let mut warnings = 0usize;
    let diagnostic_messages: std::collections::HashSet<&str> = stats
        .diagnostics
        .iter()
        .map(|d| d.message.as_str())
        .collect();

    for diagnostic in &stats.diagnostics {
        if let Some(section_kind) = failed_section_kind(diagnostic.code.as_str()) {
            handleless_not_decoded.push(section_kind.to_owned());
            continue;
        }
        match diagnostic.code.as_str() {
            "diagnostic-limit-reached" => {
                diagnostics_truncated = true;
            }
            "mesh-face-budget" => match diagnostic.record_handle {
                Some(handle) => {
                    if !not_decoded.iter().any(|(h, _)| *h == handle) {
                        not_decoded.push((handle, "MESH".to_owned()));
                    }
                }
                None => handleless_not_decoded.push("MESH".to_owned()),
            },
            "section-read-truncated" => {
                handleless_not_decoded.push("TRUNCATED_SECTION".to_owned());
            }
            "section-page-skipped" => {
                handleless_not_decoded.push("SKIPPED_SECTION_PAGE".to_owned());
            }
            "record-catalog-failed" | "record-read-failed" | "record-decode-failed" => {
                let (kind, could_be_sheet) =
                    diagnostic_record_kind(diagnostic.record_type.as_deref(), drawing);
                match diagnostic.record_handle {
                    Some(handle) => {
                        if !not_decoded.iter().any(|(h, _)| *h == handle) {
                            not_decoded.push((handle, kind.clone()));
                        }
                        if could_be_sheet && !skipped_for_sheet.iter().any(|(h, _)| *h == handle) {
                            skipped_for_sheet.push((handle, kind));
                        }
                    }
                    None => {
                        handleless_not_decoded.push(kind.clone());
                        if could_be_sheet {
                            skipped_handleless.push(kind);
                        }
                    }
                }
            }
            "auxiliary-object-preserved" => {
                if let Some(handle) = diagnostic.record_handle {
                    if !not_decoded.iter().any(|(h, _)| *h == handle) {
                        let (kind, _) =
                            diagnostic_record_kind(diagnostic.record_type.as_deref(), drawing);
                        not_decoded.push((handle, kind));
                    }
                }
            }
            _ => {
                warnings += 1;
            }
        }
    }
    for notification in drawing.notifications.iter() {
        use acadrust::notification::NotificationType;
        match notification.notification_type {
            // AC1021 zero-filled failed page decodes emit an Error
            // notification without pushing a `section-page-skipped`
            // ReadDiagnostic.
            NotificationType::Error
                if notification
                    .message
                    .starts_with("Failsafe: skipped corrupt page ") =>
            {
                handleless_not_decoded.push("SKIPPED_SECTION_PAGE".to_owned());
            }
            NotificationType::NotImplemented | NotificationType::NotSupported => {
                warnings += 1;
            }
            NotificationType::Warning
                if !is_informational_trace_warning(&notification.message)
                    && !diagnostic_messages.contains(notification.message.as_str()) =>
            {
                warnings += 1;
            }
            _ => {}
        }
    }
    // In the order of the file's handles, so that two reads of one file
    // report the same.
    not_decoded.sort();
    handleless_not_decoded.sort();
    let not_decoded = RecordsOfKind::group(
        not_decoded
            .into_iter()
            .map(|(handle, kind)| (kind, Some(format!("{handle:X}"))))
            .chain(handleless_not_decoded.into_iter().map(|kind| (kind, None))),
    );
    skipped_for_sheet.sort();
    skipped_handleless.sort();
    let skipped_sheet: Vec<(String, Option<String>)> = skipped_for_sheet
        .into_iter()
        .map(|(handle, kind)| (kind, Some(format!("{handle:X}"))))
        .chain(skipped_handleless.into_iter().map(|kind| (kind, None)))
        .collect();
    let recovered_errors = stats
        .recovered_errors
        .max(drawing.notifications.error_count());
    let report = DecodeReport {
        decoded: not_decoded.is_empty()
            && stats.skipped_source_records == 0
            && recovered_errors == 0
            && stats.stream_completed
            && !diagnostics_truncated,
        not_decoded,
        skipped: stats.skipped_source_records,
        recovered_errors,
        warnings,
        diagnostics_truncated,
        read_past_damage: None,
    };
    (report, skipped_sheet)
}

/// The kind a decode report gives a record the DWG library could not read
/// at all and whose type it could not tell.
const UNREADABLE: &str = "UNREADABLE_RECORD";

/// One of our viewports, if the DWG viewport `v` is one the core draws,
/// with its pen: its boundary, when it is clipped, one the core reads,
/// drawn with the viewport's pen as a build writes it, and every layer it
/// freezes one of the drawing's.
fn read_viewport_of(
    drawing: &CadDocument,
    v: &acadrust::entities::Viewport,
    named_mode: bool,
    ps_map: &std::collections::HashMap<acadrust::types::Handle, String>,
) -> Option<Object> {
    let properties =
        read_properties_in(&v.common, named_mode, ps_map).filter(|_| !v.common.invisible)?;
    let layer = layer_of(&v.common);
    let clip = if v.clip_boundary_handle.is_null() {
        None
    } else {
        let boundary = drawing.get_entity(v.clip_boundary_handle)?;
        let common = boundary.common();
        let same_pen = layer_of(common) == layer
            && !common.invisible
            && read_properties_in(common, named_mode, ps_map).as_ref() == Some(&properties);
        if !same_pen {
            return None;
        }
        Some(layout::read_boundary(boundary)?)
    };
    let frozen = v
        .frozen_layers
        .iter()
        .map(|h| {
            drawing
                .layers
                .iter()
                .find(|l| l.handle == *h)
                .map(|l| l.name.clone())
        })
        .collect::<Option<Vec<_>>>()?;
    let mut read = layout::read_viewport(v, layer, clip, frozen)?;
    read.properties = properties;
    Some(Object::Viewport(read))
}

/// The objects of model space and of every layout's paper space, in no
/// order: what owns the blocks a dimension draws from and the blocks a
/// leader draws with, wherever it is.
pub(super) fn in_every_space(drawing: &CadDocument) -> Vec<&EntityType> {
    let mut all: Vec<&EntityType> = drawing.model_space_entities().collect();
    for record in drawing.block_records.iter().filter(|r| r.is_paper_space()) {
        all.extend(drawing.entities_in_block(&record.name));
    }
    all
}

/// The objects of the sheet a drawing opens on, each with the place of
/// the layout whose paper space it is in, none for model space: model
/// space's in drawing order, and the paper space of each layout whose
/// paper-space block is among `layouts`, but its own overall viewport, all
/// merged in the order they were made, by handle, as a build writes a
/// sheet's objects in its order.
fn sheet_objects<'a>(
    drawing: &'a CadDocument,
    layouts: &[acadrust::types::Handle],
) -> Vec<(&'a EntityType, Option<usize>)> {
    let model_record = drawing
        .block_records
        .iter()
        .find(|r| r.is_model_space())
        .map(|r| r.handle);
    let model_sorted = match model_record {
        Some(h) => {
            wipeout::sorted_block_objects(drawing, h, drawing.model_space_entities().collect())
        }
        None => drawing.model_space_entities().collect(),
    };
    let model_table = model_record.and_then(|h| wipeout::sort_table_for_block(drawing, h));
    let mut lists: Vec<Vec<(&EntityType, Option<usize>, u64)>> = vec![model_sorted
        .into_iter()
        .map(|e| (e, None, wipeout::sort_key(model_table, e.common().handle)))
        .collect()];
    for (k, block) in layouts.iter().enumerate() {
        if let Some(record) = drawing.block_records.iter().find(|r| r.handle == *block) {
            let sorted = wipeout::sorted_block_objects(
                drawing,
                record.handle,
                drawing
                    .entities_in_block(&record.name)
                    .filter(|e| !layout::is_overall(e))
                    .collect(),
            );
            let table = wipeout::sort_table_for_block(drawing, record.handle);
            lists.push(
                sorted
                    .into_iter()
                    .map(|e| (e, Some(k), wipeout::sort_key(table, e.common().handle)))
                    .collect(),
            );
        }
    }
    let mut merged = Vec::with_capacity(lists.iter().map(Vec::len).sum());
    let mut heads = vec![0_usize; lists.len()];
    loop {
        // The list whose next object draws first; model space's on a tie.
        let next = (0..lists.len())
            .filter(|&l| heads[l] < lists[l].len())
            .min_by_key(|&l| lists[l][heads[l]].2);
        let Some(l) = next else { break };
        let (obj, space, _) = lists[l][heads[l]];
        merged.push((obj, space));
        heads[l] += 1;
    }
    merged
}

/// A block definition read back from a DWG, with each of its objects' handle
/// and any carried ID and name.
struct ReadBlock {
    block: Block,
    handles: Vec<String>,
    ids_and_names: Vec<IdAndName>,
}

/// A block definition the core can read back whole: named (or an evaluated
/// anonymous `*U<n>` block belonging to a dynamic block), not an external
/// reference, and holding only objects the core has a model for, an insert
/// among them only of one of `blocks`. Inside dynamic blocks, entities marked
/// invisible (such as objects hidden by a visibility state) are skipped rather
/// than failing the whole block.
fn read_block(
    drawing: &CadDocument,
    record: &BlockRecord,
    blocks: &[Block],
    ad_evaluated: &std::collections::HashSet<String>,
    named_mode: bool,
    ps_map: &std::collections::HashMap<acadrust::types::Handle, String>,
) -> Option<ReadBlock> {
    let is_dynamic_block = dynamic_block::is_dynamic_or_evaluated(drawing, record, ad_evaluated);
    if (record.is_anonymous() && !is_dynamic_block)
        || record.flags.is_xref
        || record.flags.is_xref_overlay
        || !record.xref_path.is_empty()
    {
        return None;
    }
    let mut objects = Vec::new();
    let mut handles = Vec::new();
    let mut ids_and_names = Vec::new();
    let block_objects = wipeout::sorted_block_objects(
        drawing,
        record.handle,
        drawing.entities_in_block(&record.name).collect(),
    );
    for dwg_object in block_objects {
        if matches!(dwg_object, EntityType::Block(_) | EntityType::BlockEnd(_)) {
            continue;
        }
        // Inside dynamic blocks (both master definitions and evaluated `*U<n>`
        // blocks), AutoCAD hides inactive visibility-state objects with
        // `common.invisible = true`; skip them cleanly so the visible state
        // reads and draws faithfully.
        if is_dynamic_block && dwg_object.common().invisible {
            continue;
        }
        objects.push(read_object_in(
            Some(drawing),
            dwg_object,
            blocks,
            &[],
            named_mode,
            ps_map,
        )?);
        let common = dwg_object.common();
        handles.push(handle_text(common.handle));
        ids_and_names.push(IdAndName {
            id: ids::read_id(common),
            name: ids::read_name(common),
        });
    }
    let parameters = dynamic_block::read_block_parameters(drawing, record);
    let annotative = annotation_scale::block_is_annotative(drawing, record.handle);
    Some(ReadBlock {
        block: Block {
            name: record.name.clone(),
            base_point: point(record.base_point),
            objects,
            cells: Vec::new(),
            parameters,
            annotative,
        },
        handles,
        ids_and_names,
    })
}

/// A linetype of dashes, gaps and dots. One that draws text or shapes has
/// no model yet.
fn read_linetype(dwg_linetype: &LineType) -> Option<Linetype> {
    if dwg_linetype.is_complex() {
        return None;
    }
    Some(Linetype {
        name: dwg_linetype.name.clone(),
        description: dwg_linetype.description.clone(),
        pattern: dwg_linetype.elements.iter().map(|e| e.length).collect(),
    })
}

/// The layer an object sits on: layer `0` when it names none.
pub(super) fn layer_of(common: &EntityCommon) -> String {
    if common.layer.is_empty() {
        LAYER_ZERO.to_owned()
    } else {
        common.layer.clone()
    }
}

/// A layer with an index colour and a standard or default lineweight.
fn read_layer(
    dwg_layer: &DwgLayer,
    named_mode: bool,
    ps_map: &std::collections::HashMap<acadrust::types::Handle, String>,
) -> Option<Layer> {
    let Color::Index(color) = dwg_layer.color else {
        return None;
    };
    let lineweight = match dwg_layer.line_weight {
        LineWeight::Default => None,
        LineWeight::Value(hundredths) => Some(Lineweight::from_hundredths(hundredths)?),
        LineWeight::ByLayer | LineWeight::ByBlock => return None,
    };
    Some(Layer {
        name: dwg_layer.name.clone(),
        color,
        linetype: dwg_layer.line_type.clone(),
        lineweight,
        plot_style: page_setup::read_layer_plot_style(dwg_layer, named_mode, ps_map),
        transparency: page_setup::read_layer_transparency(dwg_layer.transparency),
        on: !dwg_layer.flags.off,
        frozen: dwg_layer.flags.frozen,
        locked: dwg_layer.flags.locked,
        plot: dwg_layer.is_plottable,
        description: dwg_layer.description.clone(),
    })
}

/// One of our objects, if the DWG object and its own colour, linetype,
/// lineweight and linetype scale have a model in the core. An insert is
/// read only when its block is one of `blocks`.
pub(super) fn read_object(
    drawing: Option<&CadDocument>,
    dwg_object: &EntityType,
    blocks: &[Block],
) -> Option<Object> {
    read_object_in(
        drawing,
        dwg_object,
        blocks,
        &[],
        false,
        &std::collections::HashMap::new(),
    )
}

fn read_object_in(
    drawing: Option<&CadDocument>,
    dwg_object: &EntityType,
    blocks: &[Block],
    xrefs: &[Xref],
    named_mode: bool,
    ps_map: &std::collections::HashMap<acadrust::types::Handle, String>,
) -> Option<Object> {
    let common = dwg_object.common();
    if common.invisible {
        return None;
    }
    let properties = read_properties_in(common, named_mode, ps_map)?;
    let layer = layer_of(common);
    let object = match dwg_object {
        EntityType::Line(line) => {
            if let Some(cl) = extra::try_read_center_line(line, layer.clone(), Default::default()) {
                Object::CenterLine(cl)
            } else {
                Object::Line(Line {
                    from: point(line.start),
                    to: point(line.end),
                    layer,
                    properties: Default::default(),
                })
            }
        }
        EntityType::Point(p) => Object::Point(extra::read_point(p, layer, Default::default())),
        EntityType::XLine(xl) => Object::XLine(extra::read_xline(xl, layer, Default::default())),
        EntityType::Ray(r) => Object::Ray(extra::read_ray(r, layer, Default::default())),
        EntityType::MLine(ml) => Object::MLine(extra::read_mline(ml, layer, Default::default())),
        EntityType::Tolerance(tol) => Object::Tolerance(extra::read_tolerance(
            drawing?,
            tol,
            layer,
            Default::default(),
        )),
        EntityType::Shape(sh) => {
            Object::Shape(extra::read_shape(drawing?, sh, layer, Default::default()))
        }
        EntityType::LwPolyline(flat) if flat.normal == Vector3::UNIT_Z => {
            Object::Polyline(read_flat_polyline(flat, layer)?)
        }
        EntityType::Solid(solid)
            if solid.normal == Vector3::UNIT_Z && solid.thickness == 0.0 && !solid.is_trace =>
        {
            let [first, second, third, fourth] = [
                solid.first_corner,
                solid.second_corner,
                solid.third_corner,
                solid.fourth_corner,
            ]
            .map(point);
            let corners = if fourth == third {
                vec![first, second, third]
            } else {
                vec![first, second, fourth, third]
            };
            Object::Solid(Solid {
                corners,
                layer,
                properties: Default::default(),
            })
        }
        EntityType::Wipeout(w) => Object::Wipeout(wipeout::read_wipeout(w, layer)?),
        EntityType::RasterImage(img) => {
            Object::RasterImage(media::read_raster_image(drawing?, img, layer)?)
        }
        EntityType::Underlay(u) => {
            Object::PdfUnderlay(media::read_pdf_underlay(drawing?, u, layer)?)
        }
        EntityType::Solid3D(solid) => Object::Solid3D(solid3d::read_solid3d(solid, layer)?),
        EntityType::Polyline3D(spatial)
            if !spatial.flags.spline_fit
                && !spatial.flags.is_3d_mesh
                && !spatial.flags.is_polyface_mesh =>
        {
            Object::Polyline(Polyline {
                points: spatial.vertices.iter().map(|v| point(v.position)).collect(),
                closed: spatial.flags.closed,
                layer,
                properties: Default::default(),
                bulges: Vec::new(),
                width: 0.0,
                widths: Vec::new(),
            })
        }
        EntityType::Arc(arc) if arc.normal == Vector3::UNIT_Z => Object::Arc(Arc {
            center: point(arc.center),
            radius: arc.radius,
            start_angle: normalize_degrees(arc.start_angle.to_degrees()),
            end_angle: normalize_degrees(arc.end_angle.to_degrees()),
            layer,
            properties: Default::default(),
        }),
        EntityType::Circle(circle) if circle.normal == Vector3::UNIT_Z => Object::Circle(Circle {
            center: point(circle.center),
            radius: circle.radius,
            layer,
            properties: Default::default(),
        }),
        EntityType::Ellipse(ellipse)
            if ellipse.normal == Vector3::UNIT_Z && ellipse.major_axis.z == 0.0 =>
        {
            let whole =
                ellipse.start_parameter == 0.0 && (ellipse.end_parameter - TAU).abs() <= TOLERANCE;
            let (start_angle, end_angle) = if whole {
                (0.0, 360.0)
            } else {
                (
                    ellipse_angle(ellipse.start_parameter, ellipse.minor_axis_ratio),
                    ellipse_angle(ellipse.end_parameter, ellipse.minor_axis_ratio),
                )
            };
            Object::Ellipse(Ellipse {
                center: point(ellipse.center),
                major_axis: point(ellipse.major_axis),
                ratio: ellipse.minor_axis_ratio,
                start_angle,
                end_angle,
                layer,
                properties: Default::default(),
            })
        }
        EntityType::Spline(spline) => Object::Spline(read_spline(spline, layer)?),
        EntityType::Hatch(dwg_hatch) => Object::Hatch(hatch::read_hatch(dwg_hatch, layer)?),
        EntityType::Text(t) => Object::Text(text::read_text(t, layer)?),
        EntityType::MText(t) => Object::MText(text::read_mtext(t, layer)?),
        EntityType::AttributeDefinition(definition) => {
            Object::AttributeDefinition(block::read_attribute_definition(definition, layer)?)
        }
        EntityType::Insert(insert) => {
            if let Some(cm) = extra::try_read_center_mark(insert, layer.clone(), Default::default())
            {
                Object::CenterMark(cm)
            } else {
                Object::Insert(block::read_insert_in(
                    drawing, insert, layer, blocks, xrefs,
                )?)
            }
        }
        _ => return None,
    };
    let mut object = object;
    *object.properties_mut() = properties;
    Some(object)
}

/// A lightweight polyline: its points at its elevation, each one's bulge,
/// and its widths: each point's when any gives one, else its constant
/// width. One whose points give widths and a different constant width as
/// well, or a width less than 0, has no model.
fn read_flat_polyline(flat: &LwPolyline, layer: String) -> Option<Polyline> {
    let vertices = &flat.vertices;
    if vertices.iter().any(|v| {
        !v.bulge.is_finite()
            || !(v.start_width >= 0.0 && v.end_width >= 0.0)
            || !v.start_width.is_finite()
            || !v.end_width.is_finite()
    }) || !(flat.constant_width >= 0.0 && flat.constant_width.is_finite())
    {
        return None;
    }
    let own_widths = vertices.iter().any(has_width);
    let width = flat.constant_width;
    if own_widths
        && width != 0.0
        && vertices
            .iter()
            .any(|v| v.start_width != width || v.end_width != width)
    {
        return None;
    }
    let bulges: Vec<f64> = if vertices.iter().any(|v| v.bulge != 0.0) {
        vertices.iter().map(|v| v.bulge).collect()
    } else {
        Vec::new()
    };
    let (widths, width) = if own_widths && width == 0.0 {
        (
            vertices
                .iter()
                .map(|v| [v.start_width, v.end_width])
                .collect(),
            0.0,
        )
    } else {
        (Vec::new(), width)
    };
    Some(Polyline {
        points: vertices
            .iter()
            .map(|v| Point3::new(v.location.x, v.location.y, flat.elevation))
            .collect(),
        bulges,
        widths,
        width,
        closed: flat.is_closed,
        layer,
        properties: Properties::default(),
    })
}

fn has_width(vertex: &LwVertex) -> bool {
    vertex.start_width != 0.0 || vertex.end_width != 0.0
}

/// A spline through fit points with AutoCAD's default fit, or a
/// non-rational clamped B-spline with uniform knots. Any other has no model
/// yet.
fn read_spline(spline: &DwgSpline, layer: String) -> Option<Spline> {
    let points = |list: &[Vector3]| list.iter().map(|&v| point(v)).collect::<Vec<_>>();
    if spline.control_points.is_empty() {
        let default_fit = !spline.fit_points.is_empty()
            && spline.degree == i32::from(FIT_DEGREE)
            && spline.knot_parameterization == CHORD_KNOTS
            && spline.fit_tolerance == 0.0
            && spline.begin_tangent == Vector3::ZERO
            && spline.end_tangent == Vector3::ZERO;
        return default_fit.then(|| Spline {
            fit_points: points(&spline.fit_points),
            control_points: Vec::new(),
            degree: FIT_DEGREE,
            layer,
            properties: Default::default(),
        });
    }
    let degree = u8::try_from(spline.degree).ok()?;
    let (clamped_degree, uniform_knots) =
        DwgSpline::clamped_knots_with_degree(spline.degree, spline.control_points.len());
    let plain = spline.fit_points.is_empty()
        && !spline.flags.closed
        && !spline.flags.periodic
        && spline.weights.iter().all(|&w| w == 1.0)
        && clamped_degree == spline.degree
        && spline.knots.len() == uniform_knots.len()
        && spline
            .knots
            .iter()
            .zip(&uniform_knots)
            .all(|(k, u)| (k - u).abs() <= TOLERANCE);
    plain.then(|| Spline {
        fit_points: Vec::new(),
        control_points: points(&spline.control_points),
        degree,
        layer,
        properties: Default::default(),
    })
}

/// Whether the DWG at `path` carries the build mark, that is, whether
/// AgentDrafter built it. A file that cannot be read as a DWG is an error.
/// Reads directly through `DwgReader` without the strict decode-report gate
/// so `build --overwrite` on an existing AgentDrafter DWG is never refused
/// for non-fatal reader diagnostics.
pub(crate) fn carries_build_mark(path: &Path) -> Result<bool, Error> {
    read_drawing(path).map(|drawing| {
        mark_in(&drawing.summary_info.custom_properties).is_some()
            || convert::read_nod_build_mark(&drawing)
                .and_then(|val| mark_in(&[(BUILD_MARK.to_owned(), val)]))
                .is_some()
    })
}

/// What the build mark of the DWG at `path` says, read from its drawing
/// properties alone, without reading the drawing: none when the file
/// carries no build mark, or cannot be read as a DWG in a format
/// AgentDrafter builds in, the 2007 format or later.
pub(crate) fn build_mark(path: &Path) -> Option<BuildMark> {
    read_build_mark(path).ok().flatten()
}

/// What the build mark of the DWG in `bytes` says, read from its summary
/// section without reopening the file on disk.
pub(crate) fn build_mark_bytes(bytes: &[u8]) -> Option<BuildMark> {
    match catch_unwind(AssertUnwindSafe(|| drawing_properties_bytes(bytes))) {
        Ok(Ok(Some(properties))) => mark_in(&properties),
        _ => None,
    }
}

/// What the build mark of the DWG at `path` says, as [`build_mark`] reads
/// it, telling a file that carries no mark from one whose mark could not
/// be read:
///
/// - `Ok(None)`: the file was read and carries no build mark. That is a
///   file that is not a DWG, one whose first bytes name no DWG version; a
///   DWG older than the 2007 format, which AgentDrafter never builds; and
///   a DWG whose drawing properties hold no mark.
/// - `Err`: the file could not be opened or read, or it starts as a DWG of
///   the 2007 format or later but its header or its drawing properties
///   could not be read, as a file cut short is, so whether it carries the
///   mark cannot be told. The words say why.
pub(crate) fn read_build_mark(path: &Path) -> Result<Option<BuildMark>, String> {
    match catch_unwind(AssertUnwindSafe(|| drawing_properties(path))) {
        Ok(Ok(Some(properties))) => Ok(mark_in(&properties)),
        Ok(Ok(None)) => Ok(None),
        Ok(Err(why)) => Err(why),
        Err(_) => Err("the DWG library stopped reading its drawing properties".to_owned()),
    }
}

/// The custom drawing properties of the DWG or DXF at `path`, each its name
/// and its value. None for a file that is neither a DWG of the 2007 format or
/// later nor a DXF; an error when the file, or such a drawing's header or
/// summary, cannot be read.
fn drawing_properties(path: &Path) -> Result<Option<Vec<(String, String)>>, String> {
    let mut start = [0u8; 22];
    let read = std::fs::File::open(path)
        .and_then(|mut file| {
            use std::io::Read;
            let mut read = 0;
            while read < start.len() {
                match file.read(&mut start[read..])? {
                    0 => break,
                    n => read += n,
                }
            }
            Ok(read)
        })
        .map_err(|e| format!("could not read the file: {e}"))?;
    let prefix = &start[..read];
    // A DWG starts with its version, such as `AC1032` for the 2018 format;
    // the 2007 format is `AC1021`. The DWG library reads `AD` for `AC` too.
    if let Some(version) = dwg_header_version_number(prefix) {
        if version < 1021 {
            return Ok(None);
        }
        let mut reader = DwgReader::from_file(path)
            .map_err(|e| format!("the DWG library could not open it: {e}"))?;
        return summary_properties(&mut reader);
    }
    if prefix.starts_with(BINARY_DXF_SENTINEL) {
        let reader = DxfReader::from_file(path)
            .map_err(|e| format!("the DWG library could not open the DXF: {e}"))?;
        let drawing = reader
            .read()
            .map_err(|e| format!("the DWG library could not read the DXF: {e}"))?;
        return Ok(Some(drawing.summary_info.custom_properties));
    }
    if is_dxf_path(path) {
        let file =
            std::fs::File::open(path).map_err(|e| format!("could not read the file: {e}"))?;
        return ascii_dxf_header_custom_properties(std::io::BufReader::new(file));
    }
    Ok(None)
}

/// Same as [`drawing_properties`], reading from `bytes` already in memory
/// so the file is never opened a second time.
fn drawing_properties_bytes(bytes: &[u8]) -> Result<Option<Vec<(String, String)>>, String> {
    if let Some(version) = dwg_header_version_number(bytes) {
        if version < 1021 {
            return Ok(None);
        }
        let mut reader = DwgReader::from_stream(std::io::Cursor::new(bytes));
        return summary_properties(&mut reader);
    }
    if bytes.starts_with(BINARY_DXF_SENTINEL) {
        let reader = DxfReader::from_reader(std::io::Cursor::new(bytes.to_vec()))
            .map_err(|e| format!("the DWG library could not open the DXF: {e}"))?;
        let drawing = reader
            .read()
            .map_err(|e| format!("the DWG library could not read the DXF: {e}"))?;
        return Ok(Some(drawing.summary_info.custom_properties));
    }
    ascii_dxf_header_custom_properties(std::io::Cursor::new(bytes))
}

fn dwg_header_version_number(bytes: &[u8]) -> Option<u32> {
    let start = bytes.get(..6)?;
    let text = std::str::from_utf8(start).ok()?;
    let digits = text
        .strip_prefix("AC")
        .or_else(|| text.strip_prefix("AD"))?;
    (digits.len() == 4)
        .then(|| digits.parse::<u32>().ok())
        .flatten()
}

/// Reads `$CUSTOMPROPERTYTAG` / `$CUSTOMPROPERTY` pairs from an ASCII DXF's
/// `HEADER` section alone, stopping as soon as `HEADER` ends.
///
/// Returns `Ok(None)` when the stream does not start as an ASCII DXF;
/// `Ok(Some(props))` when it does and its header (or lack of one) was read;
/// and `Err` when a file that starts as an ASCII DXF has a truncated or
/// malformed header.
fn ascii_dxf_header_custom_properties<R: std::io::BufRead>(
    mut reader: R,
) -> Result<Option<Vec<(String, String)>>, String> {
    let mut line = String::new();
    let mut first_line = true;
    let mut read_pair = |reader: &mut R| -> Result<Option<(i32, String)>, String> {
        loop {
            line.clear();
            let n = reader
                .read_line(&mut line)
                .map_err(|e| format!("could not read the DXF: {e}"))?;
            if n == 0 {
                return Ok(None);
            }
            let code_text = if first_line {
                first_line = false;
                line.trim_start_matches('\u{feff}').trim()
            } else {
                line.trim()
            };
            if code_text.is_empty() {
                continue;
            }
            let Ok(code) = code_text.parse::<i32>() else {
                return Err("its DXF header has a malformed group code".to_owned());
            };
            let mut val = String::new();
            let vn = reader
                .read_line(&mut val)
                .map_err(|e| format!("could not read the DXF: {e}"))?;
            if vn == 0 {
                return Err("its DXF header is cut short".to_owned());
            }
            let val = val.trim_end_matches(&['\r', '\n'][..]).to_owned();
            return Ok(Some((code, val)));
        }
    };

    // Skip leading 999 comments and verify the stream starts as an ASCII DXF.
    let mut saw_comment = false;
    let first = loop {
        match read_pair(&mut reader) {
            Ok(Some((999, _))) => {
                saw_comment = true;
            }
            Ok(Some(pair)) => break pair,
            Ok(None) => {
                return if saw_comment {
                    Err("its DXF header is cut short".to_owned())
                } else {
                    Ok(None)
                };
            }
            Err(why) => {
                return if saw_comment { Err(why) } else { Ok(None) };
            }
        }
    };
    if first.0 != 0 || first.1.trim() != "SECTION" {
        return Ok(None);
    }
    let Some((2, section_name)) = read_pair(&mut reader)? else {
        return Err("its DXF header is cut short".to_owned());
    };
    if section_name.trim() != "HEADER" {
        return Ok(Some(Vec::new()));
    }
    let mut properties: Vec<(String, String)> = Vec::new();
    let mut pending = false;
    while let Some((code, value)) = read_pair(&mut reader)? {
        if code == 0 && value.trim() == "ENDSEC" {
            return Ok(Some(properties));
        }
        if code == 9 {
            let var = value.trim();
            if var == "$CUSTOMPROPERTYTAG" {
                let Some((_, tag)) = read_pair(&mut reader)? else {
                    return Err("its DXF header is cut short".to_owned());
                };
                properties.push((tag, String::new()));
                pending = true;
            } else if var == "$CUSTOMPROPERTY" {
                let Some((_, prop_val)) = read_pair(&mut reader)? else {
                    return Err("its DXF header is cut short".to_owned());
                };
                if pending {
                    if let Some((_, slot)) = properties.last_mut() {
                        *slot = prop_val;
                    }
                    pending = false;
                }
            }
        }
    }
    Err("its DXF header is cut short before ENDSEC".to_owned())
}

fn summary_properties<R: std::io::Read + std::io::Seek>(
    reader: &mut DwgReader<R>,
) -> Result<Option<Vec<(String, String)>>, String> {
    let header = reader
        .read_file_header()
        .map_err(|e| format!("the DWG library could not read its header: {e}"))?;
    if !header.version.r2007_plus() {
        return Ok(None);
    }
    // The summary section is optional: a DWG without one has no drawing
    // properties, so no build mark, which is not a failed read.
    let has_summary = header
        .section_descriptors
        .iter()
        .any(|section| section.name == SUMMARY_SECTION)
        || header.section_locators.contains_key(SUMMARY_SECTION);
    if !has_summary {
        return Ok(Some(Vec::new()));
    }
    let section = reader
        .get_section_buffer(SUMMARY_SECTION, &header)
        .map_err(|e| format!("the DWG library could not read its drawing properties: {e}"))?;
    custom_properties(&section)
        .map(Some)
        .ok_or_else(|| "its drawing properties are cut short or malformed".to_owned())
}

/// The custom properties in a summary section of the 2007 format or later:
/// eight texts (the title, the subject, the author, the keywords, the
/// comments, who saved it last, the revision number and the hyperlink
/// base), three times of 8 bytes each, then the count of custom properties
/// and each one's name and value. Each text is a count of UTF-16 units,
/// its closing nul among them, then the units.
fn custom_properties(section: &[u8]) -> Option<Vec<(String, String)>> {
    fn count(rest: &mut &[u8]) -> Option<usize> {
        let bytes = rest.get(..2)?;
        let count = usize::from(u16::from_le_bytes([bytes[0], bytes[1]]));
        *rest = &rest[2..];
        Some(count)
    }
    fn text(rest: &mut &[u8]) -> Option<String> {
        let length = count(rest)? * 2;
        let units: Vec<u16> = rest
            .get(..length)?
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| u16::from_le_bytes(pair))
            .collect();
        *rest = &rest[length..];
        Some(
            String::from_utf16_lossy(&units)
                .trim_end_matches('\0')
                .to_owned(),
        )
    }
    let mut rest = section;
    for _ in 0..8 {
        text(&mut rest)?;
    }
    rest = rest.get(24..)?;
    let properties = count(&mut rest)?;
    (0..properties)
        .map(|_| Some((text(&mut rest)?, text(&mut rest)?)))
        .collect()
}

/// The build mark's value for `mark`.
fn mark_value(mark: &BuildMark) -> String {
    match (&mark.set, &mark.rows) {
        (Some(set), Some(rows)) => format!("{ROWS_IN_MARK}{rows}; set={set}"),
        (Some(set), None) => format!("{BUILT_FROM_SET}{set}"),
        (None, _) => BUILT.to_owned(),
    }
}

/// The build mark among a drawing's custom properties, if one of them is
/// it.
fn mark_in(properties: &[(String, String)]) -> Option<BuildMark> {
    properties
        .iter()
        .filter(|(name, _)| name == BUILD_MARK)
        .find_map(|(_, value)| {
            if value == BUILT {
                return Some(BuildMark {
                    set: None,
                    rows: None,
                });
            }
            let (rows, set) = match value.strip_prefix(ROWS_IN_MARK) {
                Some(rest) => {
                    let (rows, set) = rest.split_once(';')?;
                    (Some(rows), set.strip_prefix(" set=")?)
                }
                None => (None, value.strip_prefix(BUILT_FROM_SET)?),
            };
            (!set.is_empty() && rows.is_none_or(|r| !r.is_empty())).then(|| BuildMark {
                set: Some(set.to_owned()),
                rows: rows.map(str::to_owned),
            })
        })
}

fn read_file_bytes(path: &Path) -> Result<Vec<u8>, Error> {
    std::fs::read(path).map_err(|e| {
        Error::new(
            ErrorCode::DwgRead,
            format!("the DWG library could not read the drawing: {e}"),
        )
        .in_file(path)
    })
}

/// Reads the DWG or DXF at `path` directly through `DwgReader` or `DxfReader`
/// without the strict decode-report gate (used by `carries_build_mark` and
/// `import-blocks`).
fn read_drawing(path: &Path) -> Result<CadDocument, Error> {
    let is_dxf = is_dxf_path(path);
    match catch_unwind(AssertUnwindSafe(|| {
        if is_dxf {
            DxfReader::from_file(path)?.read()
        } else {
            let mut reader = DwgReader::from_file(path)?;
            reader.read()
        }
    })) {
        Ok(Ok(drawing)) => Ok(drawing),
        Ok(Err(e)) => Err(Error::new(
            ErrorCode::DwgRead,
            format!("the DWG library could not read the drawing: {e}"),
        )
        .in_file(path)),
        Err(panic) => Err(Error::new(
            ErrorCode::DwgRead,
            format!(
                "the DWG library failed while reading the drawing: {}",
                panic_message(panic)
            ),
        )
        .in_file(path)),
    }
}

/// Whether `drawing` carries an AC1021 zero-filled corrupt-page error
/// notification (which acadrust emits without pushing a `ReadDiagnostic`).
fn has_ac21_skipped_page_notification(drawing: &CadDocument) -> bool {
    drawing.notifications.iter().any(|n| {
        n.notification_type == acadrust::notification::NotificationType::Error
            && n.message.starts_with("Failsafe: skipped corrupt page ")
    })
}

/// Whether an error notification message from acadrust is an intentionally
/// preserved auxiliary record (`auxiliary-object-preserved`) rather than
/// corruption.
fn is_auxiliary_preserved_error(
    message: &str,
    auxiliary_messages: &std::collections::HashSet<&str>,
) -> bool {
    auxiliary_messages.contains(message) || message.starts_with("Preserved auxiliary object ")
}

/// Why a strict read must refuse `(drawing, stats)`, if at all.
///
/// Acadrust intentionally preserves unsupported auxiliary payloads as raw
/// bytes in strict mode (`auxiliary-object-preserved`), emitting an error
/// notification without skipping the record; those stay readable in strict
/// mode (while still setting `decoded: false` and listing the record in
/// `not_decoded`), even when 1,000+ preserved auxiliary records hit the 999
/// diagnostic cap (`diagnostic-limit-reached`). Actual corruption—unreadable
/// object streams, skipped records, failed/truncated sections, AC1021
/// zero-filled corrupt pages, or error notifications/diagnostics beyond
/// preserved auxiliary objects—is refused.
fn strict_read_rejection(drawing: &CadDocument, stats: &acadrust::ReadStats) -> Option<String> {
    let auxiliary_messages: std::collections::HashSet<&str> = stats
        .diagnostics
        .iter()
        .filter(|d| d.code == "auxiliary-object-preserved")
        .map(|d| d.message.as_str())
        .collect();
    let auxiliary_preserved = stats
        .diagnostics
        .iter()
        .filter(|d| d.code == "auxiliary-object-preserved")
        .count();
    let diagnostics_capped = stats
        .diagnostics
        .iter()
        .any(|d| d.code == "diagnostic-limit-reached");
    let has_other_record_error_diag = stats.diagnostics.iter().any(|d| {
        matches!(
            d.code.as_str(),
            "record-catalog-failed" | "record-read-failed" | "record-decode-failed"
        )
    });
    let has_non_auxiliary_error_notification = drawing.notifications.iter().any(|n| {
        n.notification_type == acadrust::notification::NotificationType::Error
            && !is_auxiliary_preserved_error(&n.message, &auxiliary_messages)
    });
    let uncounted_errors = !(diagnostics_capped && auxiliary_preserved > 0)
        && (drawing.notifications.error_count() > auxiliary_preserved
            || stats.recovered_errors > auxiliary_preserved);
    let corrupt = stats.skipped_source_records > 0
        || has_section_failure_or_truncation(stats)
        || has_ac21_skipped_page_notification(drawing)
        || has_other_record_error_diag
        || has_non_auxiliary_error_notification
        || uncounted_errors;
    if !corrupt {
        return None;
    }
    Some(
        first_read_error(drawing, stats)
            .unwrap_or_else(|| "the DWG had damaged sections or pages".to_owned()),
    )
}

/// The first error message reported by the DWG library on `drawing` or
/// `stats`, if any.
fn first_read_error(drawing: &CadDocument, stats: &acadrust::ReadStats) -> Option<String> {
    let auxiliary_messages: std::collections::HashSet<&str> = stats
        .diagnostics
        .iter()
        .filter(|d| d.code == "auxiliary-object-preserved")
        .map(|d| d.message.as_str())
        .collect();
    drawing
        .notifications
        .iter()
        .find(|n| {
            n.notification_type == acadrust::notification::NotificationType::Error
                && !is_auxiliary_preserved_error(&n.message, &auxiliary_messages)
        })
        .map(|n| n.message.clone())
        .or_else(|| {
            stats
                .diagnostics
                .iter()
                .find(|d| {
                    failed_section_kind(&d.code).is_some()
                        || matches!(
                            d.code.as_str(),
                            "section-read-truncated"
                                | "section-page-skipped"
                                | "record-catalog-failed"
                                | "record-read-failed"
                                | "record-decode-failed"
                        )
                })
                .map(|d| d.message.clone())
        })
        .or_else(|| {
            drawing
                .notifications
                .iter()
                .find(|n| n.notification_type == acadrust::notification::NotificationType::Error)
                .map(|n| n.message.clone())
        })
}

/// Reads the DWG or DXF from `bytes`, already read from `path`, with what the
/// DWG library says of how it read each record: whole, or with `past_damage`
/// past the records it cannot read, which it reports.
fn read_drawing_bytes_with_stats(
    bytes: &[u8],
    path: &Path,
    past_damage: bool,
) -> Result<(CadDocument, acadrust::ReadStats), Error> {
    let as_dxf = is_dxf_path(path) || bytes.starts_with(BINARY_DXF_SENTINEL);
    let outcome = if as_dxf {
        let config = DxfReaderConfiguration {
            failsafe: past_damage,
            default_encoding: None,
        };
        match catch_unwind(AssertUnwindSafe(|| {
            let reader = DxfReader::from_reader(std::io::Cursor::new(bytes.to_vec()))?;
            reader.with_configuration(config).read_with_stats()
        })) {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(e)) => {
                return Err(Error::new(
                    ErrorCode::DwgRead,
                    format!("the DWG library could not read the DXF: {e}"),
                )
                .in_file(path))
            }
            Err(panic) => {
                return Err(Error::new(
                    ErrorCode::DwgRead,
                    format!(
                        "the DWG library failed while reading the DXF: {}",
                        panic_message(panic)
                    ),
                )
                .in_file(path))
            }
        }
    } else {
        let mut options = if past_damage {
            acadrust::DwgReadOptions::failsafe()
        } else {
            acadrust::DwgReadOptions::default()
        };
        // Pin the mesh face budget to the library's deterministic baseline cap
        // rather than probing host RAM, so reads and mirrors stay byte-identical
        // across machines and memory loads (and match WebAssembly).
        options.max_mesh_faces = Some(acadrust::MeshFaceBudget::BASELINE.max_faces);
        options.max_mesh_face_indices = Some(acadrust::MeshFaceBudget::BASELINE.max_face_indices);
        match catch_unwind(AssertUnwindSafe(|| {
            let mut reader =
                DwgReader::from_stream_with_options(std::io::Cursor::new(bytes), options);
            reader.read_with_stats()
        })) {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(e)) => {
                return Err(Error::new(
                    ErrorCode::DwgRead,
                    format!("the DWG library could not read the drawing: {e}"),
                )
                .in_file(path))
            }
            Err(panic) => {
                return Err(Error::new(
                    ErrorCode::DwgRead,
                    format!(
                        "the DWG library failed while reading the drawing: {}",
                        panic_message(panic)
                    ),
                )
                .in_file(path))
            }
        }
    };
    // Reject unreadable object streams on both strict and failsafe paths
    // (valid empty drawings still have `record_stream_read == true`, and
    // either `source_records == 0` for R10/R12 or `decoded_source_records > 0`
    // for R13+).
    if !outcome.stats.record_stream_read
        || (outcome.stats.source_records > 0 && outcome.stats.decoded_source_records == 0)
    {
        let why = first_read_error(&outcome.document, &outcome.stats).unwrap_or_else(|| {
            if as_dxf {
                "the DXF holds no drawing sections".to_owned()
            } else {
                "the DWG's object record stream could not be read".to_owned()
            }
        });
        return Err(Error::new(
            ErrorCode::DwgRead,
            format!(
                "the DWG library could not read the {}: {why}",
                if as_dxf { "DXF" } else { "drawing" }
            ),
        )
        .in_file(path));
    }
    // On strict reads, refuse files where the library hit actual section,
    // page or record corruption (such as AC1021 zero-filled corrupt pages or
    // failed named sections), while still allowing intentionally preserved
    // auxiliary payloads (`auxiliary-object-preserved`).
    if !past_damage {
        if let Some(why) = strict_read_rejection(&outcome.document, &outcome.stats) {
            return Err(Error::new(
                ErrorCode::DwgRead,
                format!("the DWG library could not read the drawing whole: {why}"),
            )
            .in_file(path));
        }
    }
    Ok((outcome.document, outcome.stats))
}

/// The DWG's `INSUNITS` code for a unit.
fn insertion_units(unit: Unit) -> i16 {
    match unit {
        Unit::In => 1,
        Unit::Ft => 2,
        Unit::Mm => 4,
        Unit::Cm => 5,
        Unit::M => 6,
    }
}

/// The unit for a DWG's `INSUNITS` code, if the core knows it.
fn unit(code: i16) -> Option<Unit> {
    Unit::ALL.into_iter().find(|u| insertion_units(*u) == code)
}

fn vector(p: Point3) -> Vector3 {
    Vector3::new(p.x, p.y, p.z)
}

fn point(v: Vector3) -> Point3 {
    Point3::new(v.x, v.y, v.z)
}

fn panic_message(panic: Box<dyn Any + Send>) -> String {
    if let Some(text) = panic.downcast_ref::<String>() {
        text.clone()
    } else if let Some(text) = panic.downcast_ref::<&str>() {
        (*text).to_owned()
    } else {
        "a panic with no message".to_owned()
    }
}

/// Only for tests: writes to `out` the DWG at `dwg`, a DWG AgentDrafter
/// built, as if someone else had drawn it: without the build mark, and
/// without AgentDrafter's extended data on any object, so that its objects
/// carry no ID of AgentDrafter's.
#[cfg(test)]
pub(crate) fn as_someone_elses(dwg: &Path, out: &Path) {
    let mut drawing = read_drawing(dwg).expect("the DWG reads");
    drawing
        .summary_info
        .custom_properties
        .retain(|(name, _)| name != BUILD_MARK);
    for entity in drawing.entities_mut() {
        let common = entity.common_mut();
        // A drawing AgentDrafter built holds no extended data but its own,
        // kept as the bytes it was read in as well as decoded.
        common.extended_data = Default::default();
        // Its bytes as read hold the extended data taken off.
        common.raw_record = None;
    }
    let bytes = DwgWriter::write_to_vec(&drawing).expect("the DWG writes");
    std::fs::write(out, bytes).expect("the DWG is saved");
}

/// Only for tests: writes `dwg` to `out` as a unitless original (INSUNITS = 0,
/// no build mark, no AgentDrafter extended data).
#[cfg(test)]
pub(crate) fn as_unitless_original(dwg: &Path, out: &Path) {
    let mut drawing = read_drawing(dwg).expect("the DWG reads");
    drawing.header.insertion_units = 0;
    drawing
        .summary_info
        .custom_properties
        .retain(|(name, _)| name != BUILD_MARK);
    for entity in drawing.entities_mut() {
        let common = entity.common_mut();
        common.extended_data = Default::default();
        common.raw_record = None;
    }
    let bytes = DwgWriter::write_to_vec(&drawing).expect("the DWG writes");
    std::fs::write(out, bytes).expect("the DWG is saved");
}

/// Only for tests: plain-data overrides for one DWG layout's stored page setup,
/// keeping `acadrust` strictly behind the `dwg` seam (ADR-0009).
#[cfg(test)]
#[derive(Clone, Debug, Default)]
pub(crate) struct TestLayoutSetup {
    pub plotter: Option<String>,
    pub paper_size: Option<String>,
    pub paper_width_mm: Option<f64>,
    pub paper_height_mm: Option<f64>,
    pub plot_paper_units: Option<i16>,
    pub plot_rotation: Option<i16>,
    pub plot_type: Option<i16>,
    pub plot_scale: Option<(f64, f64)>,
    /// `(left, bottom, right, top)` printer margins in millimetres.
    pub margins_mm: Option<(f64, f64, f64, f64)>,
    /// `(x, y)` plot origin in millimetres.
    pub plot_origin_mm: Option<(f64, f64)>,
    pub plot_style_sheet: Option<String>,
}

#[cfg(test)]
fn apply_test_layout_setup(layout: &mut acadrust::objects::Layout, setup: &TestLayoutSetup) {
    if let Some(plotter) = &setup.plotter {
        layout.plot_printer_name = plotter.clone();
    }
    if let Some(paper_size) = &setup.paper_size {
        layout.paper_size = paper_size.clone();
    }
    if let Some(width) = setup.paper_width_mm {
        layout.paper_width = width;
    }
    if let Some(height) = setup.paper_height_mm {
        layout.paper_height = height;
    }
    if let Some(paper_units) = setup.plot_paper_units {
        layout.plot_paper_units = paper_units;
    }
    if let Some(rotation) = setup.plot_rotation {
        layout.plot_rotation = rotation;
    }
    if let Some(plot_type) = setup.plot_type {
        layout.plot_type = plot_type;
    }
    if let Some((num, den)) = setup.plot_scale {
        layout.plot_scale_numerator = num;
        layout.plot_scale_denominator = den;
    }
    if let Some((left, bottom, right, top)) = setup.margins_mm {
        layout.plot_margin_left = left;
        layout.plot_margin_bottom = bottom;
        layout.plot_margin_right = right;
        layout.plot_margin_top = top;
    }
    if let Some((x, y)) = setup.plot_origin_mm {
        layout.plot_origin_x = x;
        layout.plot_origin_y = y;
    }
    if let Some(style) = &setup.plot_style_sheet {
        layout.plot_style_sheet = style.clone();
    }
}

#[cfg(test)]
fn strip_build_metadata(drawing: &mut CadDocument) {
    drawing
        .summary_info
        .custom_properties
        .retain(|(name, _)| name != BUILD_MARK);
    convert::strip_nod_build_mark(drawing);
    for object in drawing.entities_mut() {
        let common = object.common_mut();
        common.extended_data = Default::default();
        common.raw_record = None;
    }
}

/// Only for tests: mutates a built DWG's named layout page setup using plain
/// data (as an original stripped of build mark and AgentDrafter extended data)
/// and saves it to `out`.
#[cfg(test)]
pub(crate) fn as_original_with_layout_setup(
    dwg: &Path,
    out: &Path,
    layout_name: &str,
    unitless: bool,
    setup: &TestLayoutSetup,
) {
    let mut drawing = read_drawing(dwg).expect("the DWG reads");
    if unitless {
        drawing.header.insertion_units = 0;
    }
    strip_build_metadata(&mut drawing);
    let layout = drawing
        .objects
        .values_mut()
        .find_map(|o| match o {
            acadrust::objects::ObjectType::Layout(l) if l.name == layout_name => Some(l),
            _ => None,
        })
        .expect("named layout exists");
    apply_test_layout_setup(layout, setup);
    let bytes = DwgWriter::write_to_vec(&drawing).expect("the DWG writes");
    std::fs::write(out, bytes).expect("the DWG is saved");
}

/// Only for tests: adds extra inactive empty layouts with plain-data page
/// setups to a built DWG (stripped of build mark and AgentDrafter extended
/// data) and saves it to `out`.
#[cfg(test)]
pub(crate) fn as_original_with_extra_layouts(
    dwg: &Path,
    out: &Path,
    extra_layouts: &[(&str, TestLayoutSetup)],
) {
    let mut drawing = read_drawing(dwg).expect("the DWG reads");
    strip_build_metadata(&mut drawing);
    for (name, setup) in extra_layouts {
        drawing.add_layout(name).expect("add extra layout");
        let layout = drawing
            .objects
            .values_mut()
            .find_map(|o| match o {
                acadrust::objects::ObjectType::Layout(l) if l.name == *name => Some(l),
                _ => None,
            })
            .expect("added layout exists");
        apply_test_layout_setup(layout, setup);
    }
    let bytes = DwgWriter::write_to_vec(&drawing).expect("the DWG writes");
    std::fs::write(out, bytes).expect("the DWG is saved");
}

/// Only for tests: writes `dwg` to `out` with the first two model-space
/// objects carrying the same AgentDrafter `name` (with distinct IDs).
#[cfg(test)]
pub(crate) fn with_duplicate_carried_name(dwg: &Path, out: &Path, duplicate_name: &str) {
    let mut drawing = read_drawing(dwg).expect("the DWG reads");
    block::register_application(&mut drawing).expect("register app");
    drawing.assign_table_entry_handles();
    drawing
        .summary_info
        .custom_properties
        .retain(|(name, _)| name != BUILD_MARK);
    let handles: Vec<_> = drawing
        .model_space_entities()
        .take(2)
        .map(|e| e.common().handle)
        .collect();
    assert_eq!(handles.len(), 2, "need at least two model-space entities");
    for (count, handle) in handles.into_iter().enumerate() {
        let entity = drawing.get_entity_mut(handle).expect("entity exists");
        let common = entity.common_mut();
        common.extended_data = Default::default();
        ids::write_id_and_name(common, "DUP", &format!("obj-{count}"), Some(duplicate_name));
        common.raw_record = None;
    }
    let bytes = DwgWriter::write_to_vec(&drawing).expect("the DWG writes");
    std::fs::write(out, bytes).expect("the DWG is saved");
}

/// Only for tests: writes to `out` the DWG at `dwg`, a 2018 DWG AgentDrafter
/// built, with its first line turned into an object of a made-up
/// application's custom class named `kind`, which the DWG library reads but
/// cannot decode: it keeps the object as the bytes it read. Gives the
/// object's handle, as [`handle_text`] writes it. Nothing in it comes from
/// any real drawing (ADR-0008).
#[cfg(test)]
pub(crate) fn with_a_made_up_custom_object(dwg: &Path, out: &Path, kind: &str) -> String {
    use acadrust::classes::DxfClass;
    use acadrust::entities::UnknownEntity;

    let mut drawing = read_drawing(dwg).expect("the DWG reads");
    let line = drawing
        .model_space_entities()
        .find(|e| matches!(e, EntityType::Line(_)))
        .expect("the DWG holds a line")
        .clone();
    let mut common = line.common().clone();
    let raw = common
        .raw_record
        .take()
        .expect("a line read from a DWG keeps its bytes");
    let handle = common.handle;
    drawing
        .classes
        .add_or_update(DxfClass::new_entity(kind, "AcDbAgentDrafterMadeUp"));
    // Classes are numbered by their place when the DWG is written, so the
    // number is read back from the written file until it holds.
    let mut number = drawing
        .classes
        .get_by_name(kind)
        .map(|c| c.class_number)
        .expect("the class was added");
    for _ in 0..3 {
        let data = of_type(&raw.data, number);
        let mut unknown = UnknownEntity::new(kind);
        unknown.common = common.clone();
        unknown.dwg_type_code = number;
        unknown.raw_dwg_data = Some(data);
        unknown.dwg_handle_bits = raw.handle_bits;
        unknown.dwg_source_version = Some(drawing.version);
        *drawing.get_entity_mut(handle).expect("the line") = EntityType::Unknown(unknown);
        let bytes = DwgWriter::write_to_vec(&drawing).expect("the DWG writes");
        std::fs::write(out, bytes).expect("the DWG is saved");
        let written = read_drawing(out).expect("the DWG written reads");
        let given = written
            .classes
            .get_by_name(kind)
            .map(|c| c.class_number)
            .expect("the class was written");
        if given == number {
            return handle_text(handle);
        }
        number = given;
    }
    panic!("the made-up class's number never held");
}

/// Only for tests: writes to `out` the DWG at `dwg`, a 2018 DWG AgentDrafter
/// built, with the record of its first line cut short, as a damaged file
/// holds it: its type and handle read, its geometry does not. Gives the
/// line's handle, as [`handle_text`] writes it.
#[cfg(test)]
pub(crate) fn with_a_damaged_line(dwg: &Path, out: &Path) -> String {
    use acadrust::entities::UnknownEntity;

    let mut drawing = read_drawing(dwg).expect("the DWG reads");
    let line = drawing
        .model_space_entities()
        .find(|e| matches!(e, EntityType::Line(_)))
        .expect("the DWG holds a line")
        .clone();
    let mut common = line.common().clone();
    let raw = common
        .raw_record
        .take()
        .expect("a line read from a DWG keeps its bytes");
    let handle = common.handle;
    let mut damaged = UnknownEntity::new("LINE");
    damaged.common = common;
    damaged.dwg_type_code = 19;
    damaged.raw_dwg_data = Some(raw.data[..raw.data.len() / 3].to_vec());
    damaged.dwg_handle_bits = 0;
    damaged.dwg_source_version = Some(drawing.version);
    *drawing.get_entity_mut(handle).expect("the line") = EntityType::Unknown(damaged);
    let bytes = DwgWriter::write_to_vec(&drawing).expect("the DWG writes");
    std::fs::write(out, bytes).expect("the DWG is saved");
    handle_text(handle)
}

/// Only for tests: the bytes of a record of a 2010 DWG or later, `line`, a
/// line's, written as a record of type `code` instead. Such a record starts
/// with its type: two bits that say how it is written, then a byte or two.
/// A line, 19, is `00` and 19; a type from 0x1F0 to 0x2EF is `01` and the
/// type less 0x1F0; any other is `10` and the type as two bytes, the low
/// one first. The rest of the record follows unchanged, its handles found
/// from its end, so it reads the same however long the type is.
#[cfg(test)]
fn of_type(line: &[u8], code: i16) -> Vec<u8> {
    let bits_of = |byte: u8| (0..8).rev().map(move |i| (byte >> i) & 1 == 1);
    let bits: Vec<bool> = line.iter().copied().flat_map(bits_of).collect();
    assert!(
        !bits[0] && !bits[1] && ((line[0] << 2) | (line[1] >> 6)) == 19,
        "a line's record starts with its type, 19"
    );
    let mut out: Vec<bool> = Vec::new();
    match code {
        0..=0xFF => {
            out.extend([false, false]);
            out.extend(bits_of(code as u8));
        }
        0x1F0..=0x2EF => {
            out.extend([false, true]);
            out.extend(bits_of((code - 0x1F0) as u8));
        }
        _ => {
            out.extend([true, false]);
            for byte in code.to_le_bytes() {
                out.extend(bits_of(byte));
            }
        }
    }
    out.extend_from_slice(&bits[10..]);
    out.chunks(8)
        .map(|chunk| {
            chunk
                .iter()
                .enumerate()
                .fold(0_u8, |byte, (i, &bit)| byte | (u8::from(bit) << (7 - i)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;
    use crate::build::{build, BuildRequest};
    use crate::model::Sheet;

    /// An empty folder for one test under the system's temporary folder.
    fn scratch(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("agentdrafter-dwg-{}", std::process::id()))
            .join(test);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn empty_sheet() -> Sheet {
        Sheet {
            units: Unit::Mm,
            block_library: None,
            linetypes: Vec::new(),
            layers: Vec::new(),
            text_styles: Vec::new(),
            table_styles: Vec::new(),
            mleader_styles: Vec::new(),
            dimension_styles: Vec::new(),
            mline_styles: Vec::new(),
            pdmode: 0,
            pdsize: 0.0,
            blocks: Vec::new(),
            xrefs: Vec::new(),
            resolved_xrefs: Vec::new(),
            resolved_media: Vec::new(),
            resolved_shapes: Vec::new(),
            objects: Vec::new(),
            paper: None,
            page_setup: None,
            ltscale: 1.0,
            layout: None,
            other_layouts: Vec::new(),
            wipeout_frame: crate::model::WipeoutFrame::Off,
            image_frame: crate::model::WipeoutFrame::Off,
            custom_properties: std::collections::BTreeMap::new(),
            groups: Vec::new(),
            annotation_scales: Vec::new(),
            annotation_scale: None,
            anno_all_visible: None,
        }
    }

    fn record(sheet: Sheet) -> SheetRecord {
        SheetRecord {
            name: "sheet".into(),
            file: PathBuf::from("sheet.yaml"),
            sheet,
            objects: Vec::new(),
            rows_file: None,
            dwg: None,
        }
    }

    const ON_ITS_OWN: BuildMark = BuildMark {
        set: None,
        rows: None,
    };

    #[test]
    fn a_record_the_dwg_library_cannot_decode_is_named_by_kind_and_handle() {
        let dir = scratch("not-decoded");
        let description = dir.join("sheet.yaml");
        fs::write(
            &description,
            "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [10, 0, 0]}\n  - circle: {center: [5, 5, 0], radius: 2}\n",
        )
        .unwrap();
        build(&BuildRequest {
            description: description.clone(),
            output: None,
            overwrite: false,
        })
        .unwrap();
        let whole = read(&dir.join("sheet.dwg")).unwrap();
        assert!(whole.decode.decoded, "{:?}", whole.decode);
        assert_eq!(whole.handles.len(), 2);
        assert!(whole.not_read.is_empty());

        let out = dir.join("custom.dwg");
        let handle = with_a_made_up_custom_object(&dir.join("sheet.dwg"), &out, "MADE_UP_DEVICE");
        let read = read(&out).unwrap();
        assert!(!read.decode.decoded, "{:?}", read.decode);
        assert_eq!(
            read.decode.not_decoded,
            vec![RecordsOfKind {
                kind: "MADE_UP_DEVICE".into(),
                count: 1,
                handles: vec![handle.clone()],
            }]
        );
        // Nothing draws it: it is the object of the sheet not read, and the
        // circle is read with the handle it had.
        assert_eq!(read.objects_not_read, 1);
        assert_eq!(read.not_read, read.decode.not_decoded);
        assert_eq!(read.objects.len(), 1);
        assert_eq!(read.handles, whole.handles[1..]);
        // Each read of the same file names the same.
        assert_eq!(super::read(&out).unwrap(), read);
    }

    #[test]
    fn a_damaged_record_is_read_past_only_when_asked_and_is_named() {
        let dir = scratch("damaged");
        let description = dir.join("sheet.yaml");
        fs::write(
            &description,
            "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [10, 0, 0]}\n  - circle: {center: [5, 5, 0], radius: 2}\n",
        )
        .unwrap();
        build(&BuildRequest {
            description,
            output: None,
            overwrite: false,
        })
        .unwrap();
        let out = dir.join("damaged.dwg");
        let handle = with_a_damaged_line(&dir.join("sheet.dwg"), &out);
        let refused = read(&out).unwrap_err();
        assert_eq!(refused.code, ErrorCode::DwgRead, "{refused:?}");
        let past = read_past_damage(&out).unwrap();
        assert!(!past.decode.decoded, "{:?}", past.decode);
        assert_eq!(past.decode.skipped, 1, "{:?}", past.decode);
        assert!(past.decode.recovered_errors >= 1, "{:?}", past.decode);
        assert_eq!(
            past.decode.read_past_damage.as_deref(),
            Some(refused.message.as_str())
        );
        assert_eq!(
            past.decode.not_decoded,
            vec![RecordsOfKind {
                kind: "LINE".into(),
                count: 1,
                handles: vec![handle.clone()],
            }],
            "the damaged line is named by its real kind, not DWG_TYPE_19: {:?}",
            past.decode
        );
        // The skipped graphical line is also in not_read and objects_not_read.
        assert_eq!(past.objects_not_read, 1, "{past:?}");
        assert_eq!(
            past.not_read,
            vec![RecordsOfKind {
                kind: "LINE".into(),
                count: 1,
                handles: vec![handle],
            }]
        );
        // The circle is read as it was.
        assert_eq!(past.objects.len(), 1, "{past:?}");
        assert!(matches!(past.objects[0], Object::Circle(_)));
    }

    #[test]
    fn non_dwg_files_always_fail_even_past_damage() {
        let dir = scratch("non-dwg-past-damage");
        for (name, bytes) in [
            ("empty.dwg", &b""[..]),
            ("text.dwg", b"not a dwg file at all\n"),
            (
                "garbage.dwg",
                &[0xde, 0xad, 0xbe, 0xef, 0x00, 0x11, 0x22, 0x33],
            ),
        ] {
            let path = dir.join(name);
            fs::write(&path, bytes).unwrap();
            assert_eq!(read(&path).unwrap_err().code, ErrorCode::DwgRead, "{name}");
            assert_eq!(
                read_past_damage(&path).unwrap_err().code,
                ErrorCode::DwgRead,
                "{name} must not succeed with 0 objects"
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn opaque_proxy_arch_and_raw_records_force_decoded_false() {
        use acadrust::classes::arch::{ArchEntityData, ArchObject};
        use acadrust::entities::{
            ExtendedEntity, ExtendedEntityData, FormatData, LayoutPrintConfigData, ProxyEntityData,
            RegisteredClassEntityData,
        };
        use acadrust::objects::{
            ClassObject, ClassObjectData, CsacDocumentOptions, ObjectType, ProxyObject,
            ProxyPayload, RegisteredClassObject, XRecord,
        };
        use acadrust::types::Handle;

        let mut drawing = CadDocument::with_version(NEW_SHEET_VERSION);
        let stats = acadrust::ReadStats {
            stream_completed: true,
            ..Default::default()
        };
        let (clean, _) = decode_report(&drawing, &stats);
        assert!(clean.decoded, "{clean:?}");

        // Opaque graphical entities: Proxy, Arch, Format, LayoutPrintConfig, RegisteredClass.
        let mut h = 0x100u64;
        let mut next_h = || {
            h += 1;
            Handle::new(h)
        };
        let cases: Vec<(&str, ExtendedEntityData)> = vec![
            (
                "ACAD_PROXY_ENTITY",
                ExtendedEntityData::Proxy(ProxyEntityData {
                    proxy_id: 498,
                    class_id: 500,
                    dxf_subclass: String::new(),
                    version: 0,
                    dwg_version: 0,
                    maintenance_version: 0,
                    from_dxf: false,
                    graphics: ProxyPayload::default(),
                    payload: ProxyPayload::from_bytes(b"opaque"),
                    text_payload: ProxyPayload::default(),
                    object_ids: Vec::new(),
                }),
            ),
            (
                "AEC_WALL",
                ExtendedEntityData::Arch(ArchEntityData::from_parts("AEC_WALL", "AecDbWall", None)),
            ),
            (
                "Format",
                ExtendedEntityData::Format(FormatData {
                    raw_dwg_data: Some(vec![1, 2, 3]),
                    ..Default::default()
                }),
            ),
            (
                "LAYOUTPRINTCONFIG",
                ExtendedEntityData::LayoutPrintConfig(LayoutPrintConfigData {
                    raw_dwg_data: Some(vec![1, 2, 3]),
                    ..Default::default()
                }),
            ),
            (
                "MADE_UP_REG_ENT",
                ExtendedEntityData::RegisteredClass(RegisteredClassEntityData {
                    dxf_name: "MADE_UP_REG_ENT".into(),
                    cpp_class_name: "AcDbMadeUp".into(),
                    ..Default::default()
                }),
            ),
        ];
        for (expected_kind, data) in cases {
            let mut single = CadDocument::with_version(NEW_SHEET_VERSION);
            let mut common = EntityCommon::new();
            let handle = next_h();
            common.handle = handle;
            let _ = single.add_entity(EntityType::Extended(Box::new(ExtendedEntity {
                common,
                data,
            })));
            let (rep, _) = decode_report(&single, &stats);
            assert!(!rep.decoded, "{expected_kind}: {rep:?}");
            assert_eq!(rep.not_decoded.len(), 1, "{expected_kind}: {rep:?}");
            assert_eq!(rep.not_decoded[0].kind, expected_kind);
            assert_eq!(rep.not_decoded[0].handles, vec![handle_text(handle)]);
        }

        // Opaque non-graphical objects: Arch, ProxyObject, RegisteredClass,
        // DgnLineStyle with undecoded_fields or Registered payload,
        // VbaProject with empty or raw-only (failed compound-file) storage,
        // CsacDocumentOptions with raw_dwg_data, and incomplete XRecord.
        use acadrust::compound_file::StructuredStoragePayload;
        use acadrust::objects::{DgnLineStyleData, DgnLineStyleObject, VbaProject};
        let h_arch = next_h();
        let h_proxy = next_h();
        let h_reg = next_h();
        let h_lsdef = next_h();
        let h_lsreg = next_h();
        let h_vba = next_h();
        let h_vba_raw = next_h();
        let h_csac = next_h();
        let h_xrec = next_h();
        let obj_cases: Vec<(&str, Handle, ObjectType)> = vec![
            (
                "AEC_WALL_STYLE",
                h_arch,
                ObjectType::Arch(ArchObject::new("AEC_WALL_STYLE", h_arch, Handle::NULL)),
            ),
            (
                "ACAD_PROXY_OBJECT",
                h_proxy,
                ObjectType::ProxyObject(ProxyObject {
                    handle: h_proxy,
                    ..Default::default()
                }),
            ),
            (
                "MADE_UP_REG_OBJ",
                h_reg,
                ObjectType::RegisteredClass(RegisteredClassObject {
                    handle: h_reg,
                    dxf_name: "MADE_UP_REG_OBJ".into(),
                    ..Default::default()
                }),
            ),
            (
                "LSDEFINITION",
                h_lsdef,
                ObjectType::DgnLineStyle(DgnLineStyleObject {
                    handle: h_lsdef,
                    owner: Handle::NULL,
                    reactors: Vec::new(),
                    xdictionary_handle: None,
                    data: DgnLineStyleData::Definition {
                        description: String::new(),
                        version: 1,
                        style_number: 1,
                        component_uid: [0; 16],
                        is_continuous: false,
                        unit_definition: 0.0,
                        unit_scale: 1.0,
                        units_type: 0,
                        is_element: false,
                        is_physical: false,
                        is_scale_independent: false,
                        is_snappable: false,
                        root_component: Handle::NULL,
                        undecoded_fields: Some(ProxyPayload::from_bytes(&[0xAB, 0xCD])),
                        properties: Vec::new(),
                    },
                }),
            ),
            (
                "LSUNKNOWNCOMPONENT",
                h_lsreg,
                ObjectType::DgnLineStyle(DgnLineStyleObject {
                    handle: h_lsreg,
                    owner: Handle::NULL,
                    reactors: Vec::new(),
                    xdictionary_handle: None,
                    data: DgnLineStyleData::Registered {
                        dxf_name: "LSUNKNOWNCOMPONENT".into(),
                        properties: Vec::new(),
                        payload: ProxyPayload::from_bytes(&[1, 2, 3]),
                        object_ids: Vec::new(),
                    },
                }),
            ),
            (
                "VBA_PROJECT",
                h_vba,
                ObjectType::ClassObject(ClassObject {
                    handle: h_vba,
                    owner: Handle::NULL,
                    reactors: Vec::new(),
                    xdictionary_handle: None,
                    data: ClassObjectData::VbaProject(VbaProject {
                        storage: Default::default(),
                    }),
                }),
            ),
            (
                "VBA_PROJECT",
                h_vba_raw,
                ObjectType::ClassObject(ClassObject {
                    handle: h_vba_raw,
                    owner: Handle::NULL,
                    reactors: Vec::new(),
                    xdictionary_handle: None,
                    data: ClassObjectData::VbaProject(VbaProject {
                        // Failed compound-file decode keeps raw bytes in
                        // `leading_records` with `compound_file: None`.
                        storage: StructuredStoragePayload::decode(&[0xDE, 0xAD, 0xBE, 0xEF]),
                    }),
                }),
            ),
            (
                "CSACDOCUMENTOPTIONS",
                h_csac,
                ObjectType::ClassObject(ClassObject {
                    handle: h_csac,
                    owner: Handle::NULL,
                    reactors: Vec::new(),
                    xdictionary_handle: None,
                    data: ClassObjectData::CsacDocumentOptions(CsacDocumentOptions {
                        class_version: 1,
                        raw_dwg_data: Some(vec![1, 2]),
                        raw_dwg_handle_bits: 0,
                        raw_dwg_version: Some(NEW_SHEET_VERSION),
                    }),
                }),
            ),
            (
                "XRECORD",
                h_xrec,
                ObjectType::XRecord({
                    let mut x = XRecord::new();
                    x.handle = h_xrec;
                    x.entries_complete = false;
                    x.raw_data = vec![1, 2, 3];
                    x
                }),
            ),
        ];
        for (expected_kind, handle, obj) in obj_cases {
            drawing.objects.clear();
            drawing.objects.insert(handle, obj);
            let (rep, _) = decode_report(&drawing, &stats);
            assert!(!rep.decoded, "{expected_kind}: {rep:?}");
            assert_eq!(rep.not_decoded.len(), 1, "{expected_kind}: {rep:?}");
            assert_eq!(rep.not_decoded[0].kind, expected_kind);
            assert_eq!(rep.not_decoded[0].handles, vec![handle_text(handle)]);
        }
    }

    #[test]
    fn truncated_mesh_warnings_and_diagnostic_cap_are_reported_truthfully() {
        use acadrust::io::read::{ReadDiagnostic, ReadStage};

        let drawing = CadDocument::with_version(NEW_SHEET_VERSION);

        // A mesh cut short at the face budget has 0 skipped and 0 errors,
        // yet MUST force decoded=false and list MESH in not_decoded.
        let mut mesh_diag = ReadDiagnostic::new(
            "mesh-face-budget",
            ReadStage::RecordStream,
            "MESH 0x2A exceeded budget",
        );
        mesh_diag.record_handle = Some(0x2A);
        let mesh_stats = acadrust::ReadStats {
            stream_completed: true,
            diagnostics: vec![mesh_diag],
            ..Default::default()
        };
        let (rep, _) = decode_report(&drawing, &mesh_stats);
        assert!(!rep.decoded, "{rep:?}");
        assert_eq!(rep.warnings, 0, "{rep:?}");
        assert_eq!(
            rep.not_decoded,
            vec![RecordsOfKind {
                kind: "MESH".into(),
                count: 1,
                handles: vec!["2A".into()],
            }]
        );

        // Non-fatal warning diagnostics are counted in warnings and NOT
        // dumped into not_decoded, even when another record failed.
        let mut warn_diag = ReadDiagnostic::new(
            "unresolved-layer-handle",
            ReadStage::Objects,
            "layer missing",
        );
        warn_diag.record_handle = Some(0x30);
        warn_diag.record_type = Some("LINE".into());
        let warn_stats = acadrust::ReadStats {
            stream_completed: true,
            recovered_errors: 1,
            diagnostics: vec![warn_diag],
            ..Default::default()
        };
        let (rep, _) = decode_report(&drawing, &warn_stats);
        assert!(!rep.decoded);
        assert!(
            rep.not_decoded.is_empty(),
            "warning must not enter not_decoded: {rep:?}"
        );
        assert_eq!(rep.warnings, 1, "{rep:?}");

        // Hitting the 999 diagnostic cap sets diagnostics_truncated=true and decoded=false.
        let cap_diag = ReadDiagnostic::new(
            "diagnostic-limit-reached",
            ReadStage::RecordStream,
            "Additional reader diagnostics were omitted",
        );
        let cap_stats = acadrust::ReadStats {
            stream_completed: true,
            diagnostics: vec![cap_diag],
            ..Default::default()
        };
        let (rep, _) = decode_report(&drawing, &cap_stats);
        assert!(rep.diagnostics_truncated, "{rep:?}");
        assert!(!rep.decoded, "{rep:?}");
        assert_eq!(rep.warnings, 0, "sentinel is not a warning: {rep:?}");
        assert!(rep.not_decoded.is_empty(), "{rep:?}");

        // AC1021 zero-filled failed page decodes emit an Error notification
        // without a `section-page-skipped` diagnostic; capture them in
        // `not_decoded`, count `recovered_errors`, and force `decoded: false`.
        let mut ac21_doc = CadDocument::with_version(NEW_SHEET_VERSION);
        ac21_doc.notifications.notify(
            acadrust::notification::NotificationType::Error,
            "Failsafe: skipped corrupt page 2 in section 'AcDb:AcDbObjects': crc mismatch",
        );
        ac21_doc.notifications.notify(
            acadrust::notification::NotificationType::Warning,
            "Failsafe: 1 of 4 pages skipped in section 'AcDb:AcDbObjects'",
        );
        let clean_stats = acadrust::ReadStats {
            stream_completed: true,
            ..Default::default()
        };
        let (rep, _) = decode_report(&ac21_doc, &clean_stats);
        assert!(!rep.decoded, "{rep:?}");
        assert_eq!(rep.recovered_errors, 1, "{rep:?}");
        assert_eq!(
            rep.warnings, 0,
            "failsafe summary is not a warning: {rep:?}"
        );
        assert_eq!(
            rep.not_decoded,
            vec![RecordsOfKind {
                kind: "SKIPPED_SECTION_PAGE".into(),
                count: 1,
                handles: Vec::new(),
            }]
        );

        // Named section extraction/decode failures are recorded in `not_decoded`
        // and force `decoded: false`.
        let sec_stats = acadrust::ReadStats {
            stream_completed: true,
            diagnostics: vec![
                ReadDiagnostic::new(
                    "classes-extract-failed",
                    ReadStage::Classes,
                    "Failed to extract classes section",
                ),
                ReadDiagnostic::new(
                    "header-decode-failed",
                    ReadStage::Header,
                    "Failed to read header",
                ),
            ],
            ..Default::default()
        };
        let (rep, _) = decode_report(&drawing, &sec_stats);
        assert!(!rep.decoded, "{rep:?}");
        assert_eq!(rep.warnings, 0, "{rep:?}");
        assert_eq!(
            rep.not_decoded,
            vec![
                RecordsOfKind {
                    kind: "FAILED_CLASSES_SECTION".into(),
                    count: 1,
                    handles: Vec::new(),
                },
                RecordsOfKind {
                    kind: "FAILED_HEADER_SECTION".into(),
                    count: 1,
                    handles: Vec::new(),
                },
            ]
        );

        // AC18 page-capacity warnings are counted in `warnings`, while normal
        // informational traces are ignored.
        let mut warn_doc = CadDocument::with_version(NEW_SHEET_VERSION);
        warn_doc.notifications.notify(
            acadrust::notification::NotificationType::Warning,
            "Reading DWG file version: AC1032 (AC1032)",
        );
        warn_doc.notifications.notify(
            acadrust::notification::NotificationType::Warning,
            "AC18 section 'AcDb:AcDbObjects' declared size 1024 is slightly larger than its 1 page(s) of 512 bytes; retaining the bounded section",
        );
        let (rep, _) = decode_report(&warn_doc, &clean_stats);
        assert!(rep.decoded, "{rep:?}");
        assert_eq!(rep.warnings, 1, "{rep:?}");
        assert!(rep.not_decoded.is_empty(), "{rep:?}");

        // An intentionally preserved auxiliary payload emits an Error
        // notification and `auxiliary-object-preserved` diagnostic without
        // skipping the record: strict mode MUST accept it (no
        // `read_past_damage`), while `decode_report` still marks it
        // `decoded: false` and lists it in `not_decoded`.
        let mut aux_doc = CadDocument::with_version(NEW_SHEET_VERSION);
        let aux_msg = "Preserved auxiliary object 0x7B as raw bytes";
        aux_doc
            .notifications
            .notify(acadrust::notification::NotificationType::Error, aux_msg);
        let mut aux_diag = ReadDiagnostic::new(
            "auxiliary-object-preserved",
            ReadStage::RecordStream,
            aux_msg,
        );
        aux_diag.record_handle = Some(0x7B);
        aux_diag.record_type = Some("ACDBASSOCDEPENDENCY".into());
        let aux_stats = acadrust::ReadStats {
            stream_completed: true,
            record_stream_read: true,
            source_records: 10,
            decoded_source_records: 10,
            recovered_errors: 1,
            diagnostics: vec![aux_diag],
            ..Default::default()
        };
        assert_eq!(
            strict_read_rejection(&aux_doc, &aux_stats),
            None,
            "auxiliary-object-preserved must not reject strict read"
        );
        let (rep, _) = decode_report(&aux_doc, &aux_stats);
        assert!(!rep.decoded, "{rep:?}");
        assert_eq!(
            rep.not_decoded,
            vec![RecordsOfKind {
                kind: "ACDBASSOCDEPENDENCY".into(),
                count: 1,
                handles: vec!["7B".into()],
            }]
        );
        // By contrast, an AC1021 zero-filled corrupt page IS rejected in strict mode.
        assert!(
            strict_read_rejection(&ac21_doc, &clean_stats).is_some(),
            "AC1021 skipped page must reject strict read"
        );
        assert!(
            strict_read_rejection(&drawing, &sec_stats).is_some(),
            "named section failure must reject strict read"
        );

        // 1,000 intentionally preserved auxiliary records hit the 999
        // diagnostic cap (`diagnostic-limit-reached`) while recording all
        // 1,000 in `notifications.error_count()` / `recovered_errors`: strict
        // mode MUST accept them (no false `read_past_damage`), while
        // `decode_report` still marks `decoded: false`,
        // `diagnostics_truncated: true`, and lists the 999 retained records in
        // `not_decoded`.
        let mut aux_cap_doc = CadDocument::with_version(NEW_SHEET_VERSION);
        let mut aux_cap_diags = Vec::with_capacity(1_000);
        for i in 0..1_000u64 {
            let handle = 0x1000 + i;
            let msg = format!(
                "Preserved auxiliary object ACDBASSOCDEPENDENCY at handle {handle:#X}, \
                 type_code=512 as raw DWG data after decode failure: test"
            );
            aux_cap_doc
                .notifications
                .notify(acadrust::notification::NotificationType::Error, msg.clone());
            if i < 999 {
                let mut d =
                    ReadDiagnostic::new("auxiliary-object-preserved", ReadStage::RecordStream, msg);
                d.record_handle = Some(handle);
                d.record_type = Some("ACDBASSOCDEPENDENCY".into());
                aux_cap_diags.push(d);
            }
        }
        aux_cap_diags.push(ReadDiagnostic::new(
            "diagnostic-limit-reached",
            ReadStage::RecordStream,
            "Additional reader diagnostics were omitted after the safety limit",
        ));
        let aux_cap_stats = acadrust::ReadStats {
            stream_completed: true,
            record_stream_read: true,
            source_records: 1_010,
            decoded_source_records: 1_010,
            recovered_errors: aux_cap_doc.notifications.error_count(),
            diagnostics: aux_cap_diags,
            ..Default::default()
        };
        assert_eq!(aux_cap_stats.recovered_errors, 1_000);
        assert_eq!(
            strict_read_rejection(&aux_cap_doc, &aux_cap_stats),
            None,
            "1,000 preserved auxiliary records hitting diagnostic cap must not reject strict read"
        );
        let (rep, _) = decode_report(&aux_cap_doc, &aux_cap_stats);
        assert!(!rep.decoded, "{rep:?}");
        assert!(rep.diagnostics_truncated, "{rep:?}");
        assert_eq!(rep.recovered_errors, 1_000, "{rep:?}");
        assert_eq!(rep.not_decoded.len(), 1, "{rep:?}");
        assert_eq!(rep.not_decoded[0].kind, "ACDBASSOCDEPENDENCY");
        assert_eq!(rep.not_decoded[0].count, 999);
        assert_eq!(rep.not_decoded[0].handles.len(), 999);

        // If a non-auxiliary error notification is also present beside capped
        // preserved auxiliary diagnostics, strict mode MUST still reject.
        let mut mixed_doc = CadDocument::with_version(NEW_SHEET_VERSION);
        mixed_doc.notifications.notify(
            acadrust::notification::NotificationType::Error,
            "Corrupt record payload",
        );
        for n in aux_cap_doc.notifications.iter().take(999) {
            mixed_doc
                .notifications
                .notify(n.notification_type, n.message.clone());
        }
        assert!(
            strict_read_rejection(&mixed_doc, &aux_cap_stats).is_some(),
            "non-auxiliary error beside capped preserved auxiliary diagnostics must reject strict read"
        );
    }

    #[test]
    fn empty_valid_dwg_reads_raw_version_preserved_and_unreadable_stream_rejected() {
        let empty_path = Path::new("empty.dwg");

        // A valid empty DWG (0 sheet objects) reads cleanly on both strict and
        // failsafe paths with `decoded: true`.
        let empty_doc = CadDocument::with_version(NEW_SHEET_VERSION);
        let bytes = acadrust::DwgWriter::write_to_vec(&empty_doc).unwrap();
        let strict = read_bytes(&bytes, empty_path).unwrap();
        assert!(strict.decode.decoded, "{:?}", strict.decode);
        assert_eq!(strict.dwg_version, "AC1032");
        assert!(strict.objects.is_empty());
        let past = read_bytes_past_damage(&bytes, empty_path).unwrap();
        assert!(past.decode.decoded, "{:?}", past.decode);

        // Raw 6-byte DWG version header is preserved even when acadrust
        // normalizes an older format internally (e.g. AC1006 -> AC1009).
        assert_eq!(raw_dwg_version(b"AC1006\x00\x00"), Some("AC1006".into()));
        assert_eq!(raw_dwg_version(b"AD1021\x00\x00"), Some("AD1021".into()));
        assert_eq!(raw_dwg_version(b"NOTDWG"), None);
        let r12_doc = CadDocument::with_version(acadrust::DxfVersion::AC1009);
        let stats = acadrust::ReadStats {
            stream_completed: true,
            record_stream_read: true,
            ..Default::default()
        };
        let c = contents(&r12_doc, &stats, raw_dwg_version(b"AC1006\x00\x00")).unwrap();
        assert_eq!(c.dwg_version, "AC1006");

        // Truncating an AC1032 file after the file header so the handles /
        // objects stream cannot be read MUST fail on BOTH strict and failsafe
        // paths (never returning an empty 0-object drawing).
        let truncated = &bytes[..bytes.len().min(512)];
        let err_strict = read_bytes(truncated, empty_path).unwrap_err();
        assert_eq!(err_strict.code, ErrorCode::DwgRead);
        let err_past = read_bytes_past_damage(truncated, empty_path).unwrap_err();
        assert_eq!(err_past.code, ErrorCode::DwgRead);
    }

    #[test]
    fn kinds_match_dxf_names_for_extended_trace_surface_and_dimensions() {
        use acadrust::classes::arch::ArchEntityData;
        use acadrust::entities::{
            Dimension as DwgDimension, DimensionArc, DimensionLargeRadial, ExtendedEntity,
            ExtendedEntityData, FormatData, LayoutPrintConfigData, PointCloudData,
            PointCloudExData, Surface, SurfaceKind, UnknownEntity,
        };

        let ext = |data| {
            EntityType::Extended(Box::new(ExtendedEntity {
                common: EntityCommon::new(),
                data,
            }))
        };
        assert_eq!(
            kind_of(&ext(ExtendedEntityData::PointCloud(PointCloudData {
                class_version: 0,
                origin: Vector3::ZERO,
                saved_filename: String::new(),
                source_files: Vec::new(),
                extents_min: Vector3::ZERO,
                extents_max: Vector3::ZERO,
                point_count: 0,
                ucs_name: String::new(),
                ucs_origin: Vector3::ZERO,
                ucs_x_direction: Vector3::UNIT_X,
                ucs_y_direction: Vector3::UNIT_Y,
                ucs_z_direction: Vector3::UNIT_Z,
                definition_handle: acadrust::types::Handle::NULL,
                reactor_handle: acadrust::types::Handle::NULL,
                show_intensity: false,
                intensity_scheme: 0,
                minimum_intensity: 0.0,
                maximum_intensity: 0.0,
                low_intensity_threshold: 0.0,
                high_intensity_threshold: 0.0,
                show_clipping: false,
                clippings: Vec::new(),
            }))),
            "ACDBPOINTCLOUD"
        );
        assert_eq!(
            kind_of(&ext(ExtendedEntityData::PointCloudEx(PointCloudExData {
                class_version: 0,
                extents_min: Vector3::ZERO,
                extents_max: Vector3::ZERO,
                ucs_origin: Vector3::ZERO,
                ucs_x_direction: Vector3::UNIT_X,
                ucs_y_direction: Vector3::UNIT_Y,
                ucs_z_direction: Vector3::UNIT_Z,
                locked: false,
                definition_handle: acadrust::types::Handle::NULL,
                reactor_handle: acadrust::types::Handle::NULL,
                name: String::new(),
                show_intensity: false,
                show_cropping: false,
                hidden_scans: Vec::new(),
                hidden_regions: Vec::new(),
                stylization_type: 0,
                intensity_color_scheme: String::new(),
                current_color_scheme: String::new(),
                classification_color_scheme: String::new(),
                elevation_min: 0.0,
                elevation_max: 0.0,
                intensity_min: 0,
                intensity_max: 0,
                intensity_out_of_range_behavior: 0,
                elevation_out_of_range_behavior: 0,
                elevation_apply_to_fixed_range: false,
                intensity_as_gradient: false,
                elevation_as_gradient: false,
                croppings: Vec::new(),
            }))),
            "ACDBPOINTCLOUDEX"
        );
        assert_eq!(
            kind_of(&ext(ExtendedEntityData::Arch(ArchEntityData::from_parts(
                "AEC_DOOR",
                "AecDbDoor",
                None
            )))),
            "AEC_DOOR"
        );
        assert_eq!(
            kind_of(&ext(ExtendedEntityData::Format(FormatData::default()))),
            "Format"
        );
        assert_eq!(
            kind_of(&ext(ExtendedEntityData::LayoutPrintConfig(
                LayoutPrintConfigData::default()
            ))),
            "LAYOUTPRINTCONFIG"
        );
        let mut trace = DwgSolid::new(
            Vector3::ZERO,
            Vector3::UNIT_X,
            Vector3::UNIT_Y,
            Vector3::UNIT_Y,
        );
        trace.is_trace = true;
        assert_eq!(kind_of(&EntityType::Solid(trace)), "TRACE");
        let surface = Surface::new(SurfaceKind::Plane);
        assert_eq!(
            kind_of(&EntityType::Surface(Box::new(surface))),
            "PLANESURFACE"
        );
        assert_eq!(
            kind_of(&EntityType::Dimension(Box::new(DwgDimension::Arc(
                DimensionArc::default()
            )))),
            "ARC_DIMENSION"
        );
        assert_eq!(
            kind_of(&EntityType::Dimension(Box::new(DwgDimension::LargeRadial(
                DimensionLargeRadial::default()
            )))),
            "LARGE_RADIAL_DIMENSION"
        );
        let mut empty_unknown = UnknownEntity::new("");
        empty_unknown.dwg_type_code = 512;
        assert_eq!(kind_of(&EntityType::Unknown(empty_unknown)), "DWG_TYPE_512");
    }

    #[test]
    fn a_built_dwg_carries_the_build_mark() {
        let dir = scratch("marked");
        for (name, mark, value) in [
            ("alone.dwg", ON_ITS_OWN, "build"),
            (
                "of-a-set.dwg",
                BuildMark {
                    set: Some("..".into()),
                    rows: None,
                },
                "build; set=..",
            ),
            (
                "of-a-set-of-rows-files.dwg",
                BuildMark {
                    set: Some("../a; set=b".into()),
                    rows: Some("rows/areas.yaml".into()),
                },
                "build; rows=rows/areas.yaml; set=../a; set=b",
            ),
            (
                "far.dwg",
                BuildMark {
                    set: Some("../../plants/démo plant; 2".into()),
                    rows: None,
                },
                "build; set=../../plants/démo plant; 2",
            ),
        ] {
            let dwg = dir.join(name);
            fs::write(
                &dwg,
                write_new_sheet(
                    &record(empty_sheet()),
                    &|t| t.position,
                    &|_| MTextBox::default(),
                    &mark,
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(carries_build_mark(&dwg), Ok(true), "{name}");
            // The mark is one drawing property, as AutoCAD's DWGPROPS shows it.
            let drawing = read_drawing(&dwg).unwrap();
            assert_eq!(
                drawing.summary_info.custom_properties,
                [("AgentDrafter".to_owned(), value.to_owned())],
                "{name}"
            );
            // Read from the drawing properties alone, as from the drawing.
            assert_eq!(build_mark(&dwg), Some(mark.clone()), "{name}");
            // The DWG library reads a version starting `AD` as `AC`, and so
            // does the mark's reader.
            let mut bytes = fs::read(&dwg).unwrap();
            bytes[..2].copy_from_slice(b"AD");
            fs::write(&dwg, &bytes).unwrap();
            assert_eq!(build_mark(&dwg), Some(mark), "{name}, as AD");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_the_agentdrafter_property_saying_build_is_the_build_mark() {
        let mark = |properties: &[(&str, &str)]| {
            let properties: Vec<(String, String)> = properties
                .iter()
                .map(|&(n, v)| (n.to_owned(), v.to_owned()))
                .collect();
            mark_in(&properties)
        };
        assert_eq!(mark(&[("AgentDrafter", "build")]), Some(ON_ITS_OWN));
        assert_eq!(
            mark(&[("Client", "x"), ("AgentDrafter", "build; set=../a b")]),
            Some(BuildMark {
                set: Some("../a b".into()),
                rows: None,
            })
        );
        assert_eq!(
            mark(&[("AgentDrafter", "build; rows=r/b.yaml; set=../a; rows=c")]),
            Some(BuildMark {
                set: Some("../a; rows=c".into()),
                rows: Some("r/b.yaml".into()),
            })
        );
        for not_a_mark in [
            &[("AgentDrafter", "built")][..],
            &[("AgentDrafter", "build; set=")],
            &[("AgentDrafter", "build; rows=r.yaml; set=")],
            &[("AgentDrafter", "build; rows=; set=..")],
            &[("AgentDrafter", "build; rows=r.yaml")],
            &[("AgentDrafter", "build; rows=r.yaml;set=..")],
            &[("AgentDrafter", "Build")],
            &[("agentdrafter", "build")],
            &[("Author", "build")],
            &[],
        ] {
            assert_eq!(mark(not_a_mark), None, "{not_a_mark:?}");
        }
    }

    #[test]
    fn a_file_that_is_not_a_dwg_carries_no_build_mark() {
        let dir = scratch("not-a-dwg");
        let file = dir.join("sheet.dwg");
        fs::write(&file, b"AC1032 but nothing after").unwrap();
        assert_eq!(build_mark(&file), None);
        // It starts as a 2018 DWG, so whether it carries the mark cannot be
        // told: it could not be read, which is not a DWG without one.
        assert!(read_build_mark(&file).is_err());
        assert_eq!(build_mark(&dir.join("gone.dwg")), None);
        assert!(read_build_mark(&dir.join("gone.dwg")).is_err());
        // Not a DWG at all, or one of a format AgentDrafter never builds,
        // carries no mark.
        for bytes in [&b"someone else's drawing"[..], b"", b"AC1018 of 2004"] {
            fs::write(&file, bytes).unwrap();
            assert_eq!(read_build_mark(&file), Ok(None));
        }
        // A DWG of the 2018 format with no drawing properties of its own.
        let bytes = DwgWriter::write_to_vec(&CadDocument::with_version(NEW_SHEET_VERSION)).unwrap();
        fs::write(&file, &bytes).unwrap();
        assert_eq!(build_mark(&file), None);
        assert_eq!(read_build_mark(&file), Ok(None));
        // The same DWG cut short after its first 256 bytes: its header
        // starts well, and its drawing properties cannot be read.
        fs::write(&file, &bytes[..256]).unwrap();
        assert!(read_build_mark(&file).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    /// Every object is written with its ID: a sheet whose objects and
    /// records disagree is refused, never written with an object unmarked.
    #[test]
    fn an_object_without_its_record_is_not_written() {
        let mut sheet = empty_sheet();
        sheet.objects.push(Object::Line(Line {
            from: Point3::new(0.0, 0.0, 0.0),
            to: Point3::new(1.0, 0.0, 0.0),
            layer: LAYER_ZERO.to_owned(),
            properties: Default::default(),
        }));
        let error = write_new_sheet(
            &record(sheet),
            &|t| t.position,
            &|_| MTextBox::default(),
            &ON_ITS_OWN,
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::DwgWrite);
        assert!(error.message.contains("with its ID"), "{error:?}");
    }

    #[test]
    fn a_dwg_someone_else_drew_is_not_replaced_even_when_asked() {
        let dir = scratch("unmarked");
        let description = dir.join("sheet.yaml");
        fs::write(&description, "units: mm\nobjects: []\n").unwrap();
        let original = dir.join("sheet.dwg");
        let bytes = DwgWriter::write_to_vec(&CadDocument::with_version(NEW_SHEET_VERSION)).unwrap();
        fs::write(&original, &bytes).unwrap();
        assert_eq!(carries_build_mark(&original), Ok(false));

        let error = build(&BuildRequest {
            description,
            output: None,
            overwrite: true,
        })
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::NotABuild, "{error:?}");
        assert!(
            error.message.contains("not built by AgentDrafter"),
            "{error:?}"
        );
        assert_eq!(fs::read(&original).unwrap(), bytes);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_ellipse_angle_and_its_parameter_agree() {
        for ratio in [1.0, 0.5, 0.125] {
            for degrees in [0.0, 30.0, 90.0, 135.0, 180.0, 200.0, 270.0, 315.0, 359.5] {
                let t = parameter(degrees, ratio);
                assert!((0.0..TAU).contains(&t), "{degrees} at {ratio}: {t}");
                let back = ellipse_angle(t, ratio);
                assert!(
                    (back - degrees).abs() < 1e-9,
                    "{degrees} at {ratio} came back as {back}"
                );
            }
        }
        // On a circle the parameter is the angle itself.
        assert!((parameter(60.0, 1.0) - 60f64.to_radians()).abs() < 1e-15);
    }

    /// AutoCAD keeps an elliptical arc's end parameter above its start, so
    /// one that crosses the major axis ends past 2π (seen in AutoCAD 2026's
    /// reading of a built DWG).
    #[test]
    fn an_elliptical_arc_across_the_major_axis_ends_past_two_pi() {
        let arc = |start_angle, end_angle| {
            dwg_ellipse(&Ellipse {
                center: Point3::new(0.0, 0.0, 0.0),
                major_axis: Point3::new(0.0, 8.0, 0.0),
                ratio: 0.75,
                start_angle,
                end_angle,
                layer: LAYER_ZERO.to_owned(),
                properties: Default::default(),
            })
        };
        let across = arc(300.0, 60.0);
        assert!(across.start_parameter < TAU, "{across:?}");
        assert!(across.end_parameter > TAU, "{across:?}");
        let within = arc(20.0, 200.0);
        assert!(within.start_parameter < within.end_parameter);
        assert!(within.end_parameter < TAU, "{within:?}");
        let whole = arc(0.0, 360.0);
        assert_eq!((whole.start_parameter, whole.end_parameter), (0.0, TAU));
    }

    #[test]
    fn a_flat_polyline_is_lightweight_and_any_other_is_3d() {
        let polyline = |points: Vec<Point3>| Polyline {
            points,
            closed: false,
            layer: LAYER_ZERO.to_owned(),
            properties: Default::default(),
            bulges: Vec::new(),
            width: 0.0,
            widths: Vec::new(),
        };
        let flat = polyline(vec![Point3::new(0.0, 0.0, 2.0), Point3::new(1.0, 0.0, 2.0)]);
        let EntityType::LwPolyline(written) = dwg_polyline(&flat) else {
            panic!("a flat polyline is written as a lightweight polyline");
        };
        assert_eq!(written.elevation, 2.0);
        let climbing = polyline(vec![Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 0.0, 2.0)]);
        assert!(matches!(dwg_polyline(&climbing), EntityType::Polyline3D(_)));
    }

    #[test]
    fn an_objects_own_properties_are_read_and_a_true_colour_is_not() {
        let mut line = DwgLine::from_points(Vector3::ZERO, Vector3::UNIT_X);
        let read = read_object(None, &EntityType::Line(line.clone()), &[]).unwrap();
        assert!(read.properties().is_by_layer());
        line.common.color = Color::Index(1);
        line.common.linetype = "Hidden".to_owned();
        line.common.line_weight = LineWeight::Value(35);
        line.common.linetype_scale = 0.5;
        let read = read_object(None, &EntityType::Line(line.clone()), &[]).unwrap();
        assert_eq!(
            *read.properties(),
            Properties {
                color: ObjectColor::Index(1),
                linetype: ObjectLinetype::Named("Hidden".to_owned()),
                lineweight: ObjectLineweight::Width(Lineweight::from_hundredths(35).unwrap()),
                linetype_scale: 0.5,
                ..Properties::default()
            }
        );
        line.common.color = Color::ByBlock;
        line.common.linetype = "BYBLOCK".to_owned();
        line.common.line_weight = LineWeight::ByBlock;
        let read = read_object(None, &EntityType::Line(line.clone()), &[]).unwrap();
        assert_eq!(read.properties().color, ObjectColor::ByBlock);
        assert_eq!(read.properties().linetype, ObjectLinetype::ByBlock);
        assert_eq!(read.properties().lineweight, ObjectLineweight::ByBlock);
        line.common.color = Color::Rgb { r: 1, g: 2, b: 3 };
        assert!(read_object(None, &EntityType::Line(line), &[]).is_none());
    }

    #[test]
    fn an_objects_own_properties_are_written_and_by_layer_is_left_as_it_was() {
        let plain = DwgLine::from_points(Vector3::ZERO, Vector3::UNIT_X).common;
        let mut common = plain.clone();
        set_properties(&mut common, &Properties::default());
        assert_eq!(common.color, plain.color);
        assert_eq!(common.linetype, plain.linetype);
        assert_eq!(common.line_weight, plain.line_weight);
        assert_eq!(common.linetype_scale, plain.linetype_scale);
        let own = Properties {
            color: ObjectColor::ByBlock,
            linetype: ObjectLinetype::Named("Hidden".to_owned()),
            lineweight: ObjectLineweight::Default,
            linetype_scale: 2.0,
            ..Properties::default()
        };
        set_properties(&mut common, &own);
        assert_eq!(common.color, Color::ByBlock);
        assert_eq!(common.linetype, "Hidden");
        assert_eq!(common.line_weight, LineWeight::Default);
        assert_eq!(common.linetype_scale, 2.0);
        assert_eq!(read_properties(&common), Some(own));
    }

    #[test]
    fn revision_cloud_dwg_carries_revcloud_props_bulge_and_calligraphy_widths() {
        use crate::model::{
            REVCLOUD_APPLICATION, REVCLOUD_BULGE, REVCLOUD_CALLIGRAPHY_WIDTH_FACTOR,
        };
        let dir =
            std::env::temp_dir().join(format!("agentdrafter-dwg-revcloud-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let yaml_path = dir.join("clouds.yaml");
        let dwg_path = dir.join("clouds.dwg");
        std::fs::write(
            &yaml_path,
            "\
units: in
objects:
  - revision_cloud:
      id: rc-normal
      rectangle: { from: [0, 0, 0], to: [4, 2, 0] }
      arc_length: 0.5
      style: normal
  - revision_cloud:
      id: rc-cal
      name: cal-cloud
      rectangle: { from: [5, 0, 0], to: [9, 2, 0] }
      arc_length: 0.5
      style: calligraphy
      reverse: true
",
        )
        .expect("write yaml");
        let settings = crate::Settings::default();
        crate::build_with(
            &crate::BuildRequest {
                description: yaml_path.clone(),
                output: Some(dwg_path.clone()),
                overwrite: true,
            },
            &settings,
        )
        .expect("build DWG");
        let drawing = acadrust::io::dwg::DwgReader::from_file(&dwg_path)
            .expect("open DWG")
            .read()
            .expect("read back DWG");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            drawing.app_ids.get(REVCLOUD_APPLICATION).is_some(),
            "RevcloudProps APPID must be registered"
        );
        let polylines: Vec<&LwPolyline> = drawing
            .model_space_entities()
            .filter_map(|e| match e {
                EntityType::LwPolyline(p) => Some(p),
                _ => None,
            })
            .collect();
        assert_eq!(polylines.len(), 2);

        let normal = polylines[0];
        assert!(normal.is_closed);
        assert_eq!(normal.vertices.len(), 24);
        assert_eq!(normal.constant_width, 0.0);
        for v in &normal.vertices {
            assert!((v.bulge - REVCLOUD_BULGE).abs() < 1e-9);
            assert_eq!(v.start_width, 0.0);
            assert_eq!(v.end_width, 0.0);
        }
        let rec = normal
            .common
            .extended_data
            .records()
            .iter()
            .find(|r| r.application_name == REVCLOUD_APPLICATION)
            .expect("normal cloud carries RevcloudProps XData");
        assert_eq!(
            rec.values,
            vec![XDataValue::Integer16(0), XDataValue::Real(0.5)]
        );

        let cal = polylines[1];
        assert!(cal.is_closed);
        assert_eq!(cal.vertices.len(), 24);
        assert_eq!(cal.constant_width, 0.0);
        let expected_end_width = 0.5 * REVCLOUD_CALLIGRAPHY_WIDTH_FACTOR;
        for v in &cal.vertices {
            assert!((v.bulge + REVCLOUD_BULGE).abs() < 1e-9);
            assert_eq!(v.start_width, 0.0);
            assert!((v.end_width - expected_end_width).abs() < 1e-9);
        }
        let cal_rec = cal
            .common
            .extended_data
            .records()
            .iter()
            .find(|r| r.application_name == REVCLOUD_APPLICATION)
            .expect("calligraphy cloud carries RevcloudProps XData");
        assert_eq!(
            cal_rec.values,
            vec![XDataValue::Integer16(0), XDataValue::Real(0.5)]
        );
    }

    #[test]
    fn wipeout_dwg_writes_closed_vertices_wipeout_vars_and_sortents_order() {
        let dir =
            std::env::temp_dir().join(format!("agentdrafter-dwg-wipeout-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let yaml_path = dir.join("wipeout.yaml");
        let dwg_path = dir.join("wipeout.dwg");
        std::fs::write(
            &yaml_path,
            "\
units: in
wipeout_frame: display
objects:
  - line:
      id: wire
      from: [0, 1, 0]
      to: [6, 1, 0]
  - wipeout:
      id: mask
      points:
        - [2, 0.5, 0]
        - [4, 0.5, 0]
        - [4, 1.5, 0]
        - [2, 1.5, 0]
  - text:
      id: label
      contents: TAG
      position: [3, 1, 0]
      height: 0.25
",
        )
        .expect("write yaml");
        let settings = crate::Settings::default();
        crate::build_with(
            &crate::BuildRequest {
                description: yaml_path,
                output: Some(dwg_path.clone()),
                overwrite: true,
            },
            &settings,
        )
        .expect("build DWG");

        let mut drawing = acadrust::io::dwg::DwgReader::from_file(&dwg_path)
            .expect("open DWG")
            .read()
            .expect("read back DWG");

        // 1. ACAD_WIPEOUT_VARS must be written with display_frame = Display (2).
        assert_eq!(
            wipeout::read_wipeout_frame(&drawing),
            crate::WipeoutFrame::Display
        );

        // 2. The Wipeout entity must repeat its first vertex to close the polygon.
        let wipeouts: Vec<&acadrust::entities::Wipeout> = drawing
            .model_space_entities()
            .filter_map(|e| match e {
                EntityType::Wipeout(w) => Some(w),
                _ => None,
            })
            .collect();
        assert_eq!(wipeouts.len(), 1);
        assert_eq!(wipeouts[0].clip_boundary_vertices.len(), 5);
        assert_eq!(
            wipeouts[0].clip_boundary_vertices.first(),
            wipeouts[0].clip_boundary_vertices.last()
        );

        // 3. Read back via our reader preserves Wipeout + WipeoutFrame.
        let read_back = crate::read_dwg(&dwg_path).expect("read_dwg");
        assert_eq!(read_back.wipeout_frame, crate::WipeoutFrame::Display);
        assert!(matches!(read_back.objects[1], Object::Wipeout(_)));

        // 4. Applying an ACAD_SORTENTS permutation reverses draw order on read-back!
        let ms_handle = drawing
            .block_records
            .get("*Model_Space")
            .expect("*Model_Space")
            .handle;
        let orig_handles: Vec<acadrust::types::Handle> = drawing
            .model_space_entities()
            .map(|e| e.common().handle)
            .collect();
        assert_eq!(orig_handles.len(), 3);
        let reversed = vec![orig_handles[2], orig_handles[1], orig_handles[0]];
        wipeout::apply_sort_order(&mut drawing, ms_handle, &reversed);
        let sorted_dwg = dir.join("sorted.dwg");
        let bytes = DwgWriter::write_to_vec(&drawing).expect("write sorted");
        std::fs::write(&sorted_dwg, &bytes).expect("save sorted");
        let after = crate::read_dwg(&sorted_dwg).expect("read sorted");
        let ids: Vec<String> = after
            .ids_and_names
            .iter()
            .filter_map(|i| i.id.clone())
            .collect();
        assert_eq!(ids, vec!["wipeout/label", "wipeout/mask", "wipeout/wire"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `CONTEXT.md` says drawing and object. acadrust's own names, which
    /// say document and entity, are allowed; ours are not.
    #[test]
    fn our_names_in_the_seam_say_drawing_and_object() {
        for source in [include_str!("dwg.rs"), include_str!("dwg/hatch.rs")] {
            assert_our_names_say_drawing_and_object(source);
        }
    }

    fn assert_our_names_say_drawing_and_object(source: &str) {
        let (code, _tests) = source.split_once("#[cfg(test)]").unwrap();
        let mut ours = code.to_owned();
        for acadrust_name in [
            "CadDocument",
            "EntityCommon",
            "EntityType",
            "acadrust::entities",
            "add_paper_space_entity",
            "add_entity_to_layout",
            "add_entity",
            "get_entity_mut",
            "get_entity",
            "entities_in_block",
            "model_space_entities",
            "ExtendedEntityData",
            ".entities()",
            "outcome.document",
            "as_entity().entity_type()",
            "CsacDocumentOptions",
            "CSACDOCUMENTOPTIONS",
            "is_an_entity",
            // The kinds a DWG names, which `read` reports as the DWG names them.
            "ACAD_PROXY_ENTITY",
            "EXTENDED_ENTITY",
        ] {
            ours = ours.replace(acadrust_name, "");
        }
        for word in ["document", "entit"] {
            let found: Vec<&str> = ours
                .lines()
                .filter(|line| line.to_lowercase().contains(word))
                .collect();
            assert!(found.is_empty(), "`{word}` in our own names: {found:#?}");
        }
    }
}
