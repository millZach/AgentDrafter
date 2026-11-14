//! The seam between AgentDrafter and its DWG library.
//!
//! This module, with its submodules under `dwg/`, is the only code that
//! names acadrust (ADR-0009). Everything else in the core works on its own
//! model ([`crate::model`]), so moving the acadrust pin, or the library
//! itself, changes these files and `Cargo.toml` only. Moving the pin means
//! running the judge again. [`hatch`] carries hatches through the seam,
//! [`text`] text styles and text, [`block`] blocks, attribute
//! definitions and inserts, and [`ids`] each object's ID.
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
use acadrust::{CadDocument, DwgReader, DwgWriter};

use crate::description::{normalize_degrees, SheetRecord};
use crate::error::{Error, ErrorCode};
use crate::model::{
    is_one, Arc, Block, Circle, DrawingContents, Ellipse, IdAndName, Layer, Line, Linetype,
    Lineweight, Object, ObjectColor, ObjectLinetype, ObjectLineweight, Point3, Polyline,
    Properties, Solid, Spline, BUILT_IN_LINETYPES, BY_BLOCK, BY_LAYER, CONTINUOUS, FIT_DEGREE,
    LAYER_ZERO,
};
use crate::text::Text as TextObject;
use crate::units::Unit;

mod block;
mod hatch;
mod ids;
mod text;

/// The DWG version a new sheet is written in: the 2018 format, AC1032 (ADR-0006).
const NEW_SHEET_VERSION: DxfVersion = DxfVersion::AC1032;

/// The build mark: a custom drawing property, as AutoCAD's DWGPROPS shows
/// it, written into every DWG AgentDrafter builds. A build replaces an
/// existing DWG only when it carries it. Its value is [`BUILT`] for a sheet
/// built on its own, and [`BUILT_FROM_SET`] followed by the drawing set's
/// folder for a sheet of a set, such as `build; set=..`.
const BUILD_MARK: &str = "AgentDrafter";

/// The build mark's value for a sheet built on its own.
const BUILT: &str = "build";

/// How the build mark's value starts for a sheet of a drawing set: the
/// set's folder follows, relative to the DWG's folder, with `/` between
/// folders, to the end of the value.
const BUILT_FROM_SET: &str = "build; set=";

/// The section of a DWG that holds its drawing properties, DWGPROPS.
const SUMMARY_SECTION: &str = "AcDb:SummaryInfo";

/// What a DWG's build mark says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BuildMark {
    /// The folder of the drawing set the DWG was built from, relative to
    /// the DWG's folder, as the mark writes it: `/` between folders, such
    /// as `..` or `../demo-plant`. None for a sheet built on its own.
    pub set: Option<String>,
}

/// AutoCAD's own knot spacing for a curve through fit points: by the chord
/// length between them.
const CHORD_KNOTS: i32 = 0;

/// How far a knot or an ellipse's end parameter may sit from where it
/// belongs and still be read as there.
const TOLERANCE: f64 = 1e-12;

/// The DWG version a new sheet is written in, as the DWG names it.
pub(crate) fn new_sheet_version() -> String {
    NEW_SHEET_VERSION.to_string()
}

/// Writes a new sheet as the bytes of a DWG in the 2018 format, carrying the
/// build mark `mark`. `record` holds the sheet with the record of each of
/// its objects, and each object is written with its ID and name from its
/// record. `start_of` gives where each single line of text starts, which
/// the DWG records beside the point it is justified at.
pub(crate) fn write_new_sheet(
    record: &SheetRecord,
    start_of: &dyn Fn(&TextObject) -> Point3,
    mark: &BuildMark,
) -> Result<Vec<u8>, Error> {
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
    // Layers, linetypes, text styles and applications take their handles
    // before the blocks and objects take theirs.
    drawing.assign_table_entry_handles();
    let units = insertion_units(sheet.units);
    for definition in &sheet.blocks {
        let owner = block::add_block_record(&mut drawing, definition, units)?;
        for object in &definition.objects {
            let mut dwg_object = dwg_object(object, start_of, &sheet.blocks)?;
            dwg_object.common_mut().owner_handle = owner;
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
    for (object, written) in sheet.objects.iter().zip(&record.objects) {
        let mut dwg_object = dwg_object(object, start_of, &sheet.blocks)?;
        ids::write_id_and_name(
            dwg_object.common_mut(),
            &record.name,
            &written.id,
            written.name.as_deref(),
        );
        drawing.add_entity(dwg_object).map_err(|e| {
            Error::new(
                ErrorCode::DwgWrite,
                format!("could not add a {} to the drawing: {e}", object.type_name()),
            )
        })?;
    }
    match catch_unwind(AssertUnwindSafe(|| DwgWriter::write_to_vec(&drawing))) {
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
        Object::Text(t) => EntityType::Text(text::dwg_text(t, start_of(t))),
        Object::MText(t) => EntityType::MText(Box::new(text::dwg_mtext(t))),
        Object::AttributeDefinition(definition) => {
            EntityType::AttributeDefinition(block::dwg_attribute_definition(definition, start_of))
        }
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
}

/// An object's own colour, linetype, lineweight and linetype scale as its
/// DWG object holds them, if the core has a model for them: an index colour
/// rather than a true colour, and a standard lineweight.
pub(super) fn read_properties(common: &EntityCommon) -> Option<Properties> {
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
    Some(Properties {
        color,
        linetype,
        lineweight,
        linetype_scale,
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
    let drawing = read_drawing(path)?;
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
    let mut layers = Vec::new();
    let mut layers_not_read = 0;
    for dwg_layer in drawing.layers.iter() {
        match read_layer(dwg_layer) {
            Some(layer) => layers.push(layer),
            None => layers_not_read += 1,
        }
    }
    let (text_styles, text_styles_not_read) = text::read_text_styles(&drawing);
    // A block that inserts another is read once that one is: each round
    // reads every block it can with the blocks read before it, until a
    // round reads none.
    let records: Vec<&BlockRecord> = drawing
        .block_records
        .iter()
        .filter(|r| !(r.is_model_space() || r.is_paper_space() || r.is_layout()))
        .collect();
    let mut read: Vec<Option<Block>> = records.iter().map(|_| None).collect();
    loop {
        let known: Vec<Block> = read.iter().flatten().cloned().collect();
        let mut more = false;
        for (slot, record) in read.iter_mut().zip(&records) {
            if slot.is_none() {
                if let Some(definition) = read_block(&drawing, record, &known) {
                    *slot = Some(definition);
                    more = true;
                }
            }
        }
        if !more {
            break;
        }
    }
    let blocks_not_read = read.iter().filter(|b| b.is_none()).count();
    let blocks: Vec<Block> = read.into_iter().flatten().collect();
    let mut objects = Vec::new();
    let mut ids_and_names = Vec::new();
    let mut objects_not_read = 0;
    for dwg_object in drawing.model_space_entities() {
        match read_object(dwg_object, &blocks) {
            Some(object) => {
                objects.push(object);
                ids_and_names.push(IdAndName {
                    id: ids::read_id(dwg_object.common()),
                    name: ids::read_name(dwg_object.common()),
                });
            }
            None => objects_not_read += 1,
        }
    }
    Ok(DrawingContents {
        dwg_version: drawing.version.to_string(),
        units: unit(drawing.header.insertion_units),
        linetypes,
        linetypes_not_read,
        layers,
        layers_not_read,
        text_styles,
        text_styles_not_read,
        blocks,
        blocks_not_read,
        objects,
        ids_and_names,
        objects_not_read,
        ltscale: drawing.header.linetype_scale,
    })
}

/// A block definition the core can read back whole: named, not an
/// external reference, and holding only objects the core has a model for,
/// an insert among them only of one of `blocks`.
fn read_block(drawing: &CadDocument, record: &BlockRecord, blocks: &[Block]) -> Option<Block> {
    if record.is_anonymous()
        || record.flags.is_xref
        || record.flags.is_xref_overlay
        || !record.xref_path.is_empty()
    {
        return None;
    }
    let mut objects = Vec::new();
    for dwg_object in drawing.entities_in_block(&record.name) {
        if matches!(dwg_object, EntityType::Block(_) | EntityType::BlockEnd(_)) {
            continue;
        }
        objects.push(read_object(dwg_object, blocks)?);
    }
    Some(Block {
        name: record.name.clone(),
        base_point: point(record.base_point),
        objects,
        cells: Vec::new(),
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

/// A layer with an index colour and a standard or default lineweight.
fn read_layer(dwg_layer: &DwgLayer) -> Option<Layer> {
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
fn read_object(dwg_object: &EntityType, blocks: &[Block]) -> Option<Object> {
    let common = dwg_object.common();
    if common.invisible {
        return None;
    }
    let properties = read_properties(common)?;
    let layer = if common.layer.is_empty() {
        LAYER_ZERO.to_owned()
    } else {
        common.layer.clone()
    };
    let object = match dwg_object {
        EntityType::Line(line) => Object::Line(Line {
            from: point(line.start),
            to: point(line.end),
            layer,
            properties: Default::default(),
        }),
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
        EntityType::Insert(insert) => Object::Insert(block::read_insert(insert, layer, blocks)?),
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
pub(crate) fn carries_build_mark(path: &Path) -> Result<bool, Error> {
    read_drawing(path).map(|drawing| mark_in(&drawing.summary_info.custom_properties).is_some())
}

/// What the build mark of the DWG at `path` says, read from its drawing
/// properties alone, without reading the drawing: none when the file
/// carries no build mark, or cannot be read as a DWG in a format
/// AgentDrafter builds in, the 2007 format or later.
pub(crate) fn build_mark(path: &Path) -> Option<BuildMark> {
    let properties = catch_unwind(AssertUnwindSafe(|| drawing_properties(path))).ok()??;
    mark_in(&properties)
}

/// The custom drawing properties of the DWG at `path`, each its name and
/// its value, read from its summary section alone.
fn drawing_properties(path: &Path) -> Option<Vec<(String, String)>> {
    let mut reader = DwgReader::from_file(path).ok()?;
    let header = reader.read_file_header().ok()?;
    if !header.version.r2007_plus() {
        return None;
    }
    let section = reader.get_section_buffer(SUMMARY_SECTION, &header).ok()?;
    custom_properties(&section)
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
    match &mark.set {
        Some(set) => format!("{BUILT_FROM_SET}{set}"),
        None => BUILT.to_owned(),
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
                return Some(BuildMark { set: None });
            }
            let set = value.strip_prefix(BUILT_FROM_SET)?;
            (!set.is_empty()).then(|| BuildMark {
                set: Some(set.to_owned()),
            })
        })
}

fn read_drawing(path: &Path) -> Result<CadDocument, Error> {
    match catch_unwind(AssertUnwindSafe(|| {
        DwgReader::from_file(path).and_then(|mut reader| reader.read())
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
            blocks: Vec::new(),
            objects: Vec::new(),
            paper: None,
            ltscale: 1.0,
        }
    }

    fn record(sheet: Sheet) -> SheetRecord {
        SheetRecord {
            name: "sheet".into(),
            file: PathBuf::from("sheet.yaml"),
            sheet,
            objects: Vec::new(),
            rows_file: None,
        }
    }

    const ON_ITS_OWN: BuildMark = BuildMark { set: None };

    #[test]
    fn a_built_dwg_carries_the_build_mark() {
        let dir = scratch("marked");
        for (name, mark, value) in [
            ("alone.dwg", ON_ITS_OWN, "build"),
            (
                "of-a-set.dwg",
                BuildMark {
                    set: Some("..".into()),
                },
                "build; set=..",
            ),
            (
                "far.dwg",
                BuildMark {
                    set: Some("../../plants/démo plant; 2".into()),
                },
                "build; set=../../plants/démo plant; 2",
            ),
        ] {
            let dwg = dir.join(name);
            fs::write(
                &dwg,
                write_new_sheet(&record(empty_sheet()), &|t| t.position, &mark).unwrap(),
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
            assert_eq!(build_mark(&dwg), Some(mark), "{name}");
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
                set: Some("../a b".into())
            })
        );
        for not_a_mark in [
            &[("AgentDrafter", "built")][..],
            &[("AgentDrafter", "build; set=")],
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
        assert_eq!(build_mark(&dir.join("gone.dwg")), None);
        // A DWG of the 2018 format with no drawing properties of its own.
        let bytes = DwgWriter::write_to_vec(&CadDocument::with_version(NEW_SHEET_VERSION)).unwrap();
        fs::write(&file, bytes).unwrap();
        assert_eq!(build_mark(&file), None);
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
        let error = write_new_sheet(&record(sheet), &|t| t.position, &ON_ITS_OWN).unwrap_err();
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
        let read = read_object(&EntityType::Line(line.clone()), &[]).unwrap();
        assert!(read.properties().is_by_layer());
        line.common.color = Color::Index(1);
        line.common.linetype = "Hidden".to_owned();
        line.common.line_weight = LineWeight::Value(35);
        line.common.linetype_scale = 0.5;
        let read = read_object(&EntityType::Line(line.clone()), &[]).unwrap();
        assert_eq!(
            *read.properties(),
            Properties {
                color: ObjectColor::Index(1),
                linetype: ObjectLinetype::Named("Hidden".to_owned()),
                lineweight: ObjectLineweight::Width(Lineweight::from_hundredths(35).unwrap()),
                linetype_scale: 0.5,
            }
        );
        line.common.color = Color::ByBlock;
        line.common.linetype = "BYBLOCK".to_owned();
        line.common.line_weight = LineWeight::ByBlock;
        let read = read_object(&EntityType::Line(line.clone()), &[]).unwrap();
        assert_eq!(read.properties().color, ObjectColor::ByBlock);
        assert_eq!(read.properties().linetype, ObjectLinetype::ByBlock);
        assert_eq!(read.properties().lineweight, ObjectLineweight::ByBlock);
        line.common.color = Color::Rgb { r: 1, g: 2, b: 3 };
        assert!(read_object(&EntityType::Line(line), &[]).is_none());
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
        };
        set_properties(&mut common, &own);
        assert_eq!(common.color, Color::ByBlock);
        assert_eq!(common.linetype, "Hidden");
        assert_eq!(common.line_weight, LineWeight::Default);
        assert_eq!(common.linetype_scale, 2.0);
        assert_eq!(read_properties(&common), Some(own));
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
            "add_entity",
            "entities_in_block",
            "model_space_entities",
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
