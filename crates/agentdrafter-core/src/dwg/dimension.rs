//! Dimension styles and dimensions in a DWG.
//!
//! Part of the seam ([`super`]): only this module and its siblings name
//! acadrust. A dimension is AutoCAD's DIMENSION, written with its DIMSTYLE
//! and its own anonymous block, `*D1` and on, holding what it draws, as
//! AutoCAD 2026 wrote its own: the extension lines, the dimension line and
//! any tails and landing, `ByBlock` in colour and lineweight on the
//! dimension's own layer, an angular dimension's arcs as ARCs;
//! a closed filled arrowhead as a SOLID and any other as an insert of the
//! block AutoCAD draws it from, at its point, as long as the arrow and
//! turned along it; the text as MTEXT attached at its middle, a stacked
//! fraction in it as AutoCAD codes it, `{\H1x;\S1/2;}`; and a POINT on
//! layer `Defpoints` at each definition point. So AutoCAD shows it as
//! the pipeline draws it without laying it out again, and lays it out to
//! the same place when it does.
//!
//! A dimension style is written as AutoCAD 2026's own `Standard` or
//! `ISO-25` with what the model holds of it set: AutoCAD's extras of the
//! style of the sheet's unit, which draw nothing here, are as AutoCAD has
//! them. The drawing gets a `Defpoints` layer as AutoCAD makes it, when it
//! has none and it holds a dimension.
//!
//! A dimension and its style are read back when the model draws them: one
//! of the seven kinds, flat, with no override of its style but AutoCAD's
//! own, its text where AutoCAD keeps it, an angular dimension's arc point a
//! third of the way round its arc and an ordinate dimension's origin the
//! world's, and a style whose every value is one the pipeline draws. Its block is read with it, never as a block of
//! the drawing's; the pipeline draws it again from the dimension, as AutoCAD
//! does.

use acadrust::entities::{
    Arc as DwgArc, AttachmentPoint, Dimension as DwgDimension, DimensionAligned,
    DimensionAngular2Ln, DimensionAngular3Pt, DimensionBase, DimensionDiameter, DimensionLinear,
    DimensionOrdinate, DimensionRadius, EntityCommon, EntityType, Insert as DwgInsert,
    Line as DwgLine, MText as DwgMText, Point as DwgPoint, Solid as DwgSolid,
};
use acadrust::tables::{AppId, BlockRecord, DimStyle, Layer as DwgLayer, TableEntry};
use acadrust::types::{Color, Handle, LineWeight, Transparency, Vector3};
use acadrust::xdata::{ExtendedDataRecord, XDataValue};
use acadrust::CadDocument;

use super::leader::{add_arrow_block, arrow_block, arrowhead_of, check_library_arrowheads};
use super::text::{mtext_code, style_name};
use super::{point, vector};
use crate::error::{Error, ErrorCode};
use crate::model::{
    AngularUnit, Arrowhead, DecimalSeparator, Dimension, DimensionPart, DimensionStyle,
    FractionFormat, LaidDimension, MTextBox, Measured, Object, OrdinateAxis, Point3, Sheet,
    TextPiece, TextPlacement, UnitFormat, DEGREE_SIGN, DIAMETER_SIGN, MEASURED,
    STANDARD_DIMENSION_STYLE,
};
use crate::text::MText;
use crate::units::Unit;

/// The layer AutoCAD keeps a dimension's definition points on, which never
/// plots.
pub(super) const DEFPOINTS: &str = "Defpoints";

/// The application AutoCAD keeps a radius's or diameter's arc extension
/// under.
const RADIAL_EXTENSION: &str = "ACAD_DSTYLE_DIMRADIAL_EXTENSION";

/// The application AutoCAD keeps dimension style overrides under, and the
/// record of them.
const ACAD: &str = "ACAD";
const DSTYLE: &str = "DSTYLE";

/// AutoCAD's DIMUPT, which it overrides on every radius and diameter
/// dimension it makes.
const DIMUPT: i16 = 288;

/// AutoCAD's DIMTMOVE, which it overrides to 1 on every dimension its
/// DIMCONTINUE makes.
const DIMTMOVE: i16 = 279;

/// The flags byte AutoCAD 2026 wrote on its own dimensions: with the text
/// at its default place on a linear or aligned one, and placed on a radius
/// or diameter one.
const FLAGS_DEFAULT_TEXT: u8 = 0x0B;
const FLAGS_PLACED_TEXT: u8 = 0x0A;

/// AutoCAD's code that aligns a line of multiline text at its middle,
/// which it writes before the text of a dimension whose text is centred.
const ALIGN_MIDDLE: &str = "\\A1;";

/// AutoCAD's code for the diameter sign in multiline text.
const DIAMETER_CODE: &str = "\\U+2205";

/// AutoCAD's code for the degree sign in multiline text.
const DEGREE_CODE: &str = "\\U+00B0";

/// Whether the sheet holds a dimension.
fn has_dimensions(sheet: &Sheet) -> bool {
    sheet
        .objects
        .iter()
        .any(|o| matches!(o, Object::Dimension(_)))
}

fn added(what: &str) -> impl FnOnce(acadrust::error::DxfError) -> Error + '_ {
    move |e| {
        Error::new(
            ErrorCode::DwgWrite,
            format!("could not add {what} to the drawing: {e}"),
        )
    }
}

/// Before the drawing's table entries take their handles: the `Defpoints`
/// layer, as AutoCAD makes it, and the application of a radius's or a
/// diameter's arc extension, when the sheet holds a dimension.
pub(super) fn prepare(drawing: &mut CadDocument, sheet: &Sheet) -> Result<(), Error> {
    if !has_dimensions(sheet) {
        return Ok(());
    }
    if !drawing
        .layers
        .iter()
        .any(|l| l.name.eq_ignore_ascii_case(DEFPOINTS))
    {
        let mut layer = DwgLayer::new(DEFPOINTS);
        layer.is_plottable = false;
        drawing
            .layers
            .add(layer)
            .map_err(added("the layer `Defpoints`"))?;
    }
    let radial = sheet
        .objects
        .iter()
        .any(|o| matches!(o, Object::Dimension(d) if d.kind().is_radial()));
    if radial && drawing.app_ids.get(RADIAL_EXTENSION).is_none() {
        drawing
            .app_ids
            .add(AppId::new(RADIAL_EXTENSION))
            .map_err(added("the application of a radius's arc extension"))?;
    }
    Ok(())
}

/// The names of the dimension styles the sheet writes: those it lists and
/// those its dimensions draw with.
fn written_styles(sheet: &Sheet) -> Vec<DimensionStyle> {
    let mut styles: Vec<DimensionStyle> = sheet.dimension_styles.clone();
    for object in &sheet.objects {
        if let Object::Dimension(d) = object {
            if !styles.iter().any(|s| s.name == d.dimension_style) {
                if let Some(style) = sheet.dimension_style(&d.dimension_style) {
                    styles.push(style);
                }
            }
        }
    }
    styles
}

/// Adds the sheet's dimension styles, and the blocks of their arrowheads,
/// after the sheet's own blocks, so that a block of the library drawn as
/// AutoCAD's arrowhead is the one a dimension draws with.
pub(super) fn add_dimension_styles(drawing: &mut CadDocument, sheet: &Sheet) -> Result<(), Error> {
    let styles = written_styles(sheet);
    let arrowheads: Vec<Arrowhead> = styles.iter().map(|s| s.arrowhead).collect();
    check_library_arrowheads(sheet, &arrowheads)?;
    for style in styles {
        add_arrow_block(drawing, style.arrowhead)?;
        let mut written = dwg_style(drawing, &style, sheet.units)?;
        match drawing.dim_styles.get_mut(&style.name) {
            Some(existing) => {
                written.handle = existing.handle;
                *existing = written;
            }
            None => {
                written.set_handle(drawing.allocate_handle());
                drawing
                    .dim_styles
                    .add(written)
                    .map_err(added("a dimension style"))?;
            }
        }
    }
    Ok(())
}

/// The DIMSTYLE for one of ours: AutoCAD 2026's own of the sheet's unit,
/// with what the model holds set.
fn dwg_style(
    drawing: &CadDocument,
    style: &DimensionStyle,
    units: Unit,
) -> Result<DimStyle, Error> {
    let mut s = DimStyle::dxf_defaults(style.name.clone());
    // What AutoCAD 2026 wrote and acadrust does not make.
    s.dimaltmzf = 100.0;
    s.dimmzf = 100.0;
    s.dimpost = String::new();
    if units.is_metric() {
        // ISO-25's extras, which draw nothing the model holds.
        s.dimaltf = 0.039_370_078_740_16;
        s.dimaltd = 3;
        s.dimtdec = 2;
        s.dimalttd = 3;
        s.dimtolj = 0;
        s.dimtzin = 8;
    }
    s.dimasz = style.arrow_size;
    s.dimtxt = style.text_height;
    s.dimgap = style.gap;
    s.dimexo = style.extension_offset;
    s.dimexe = style.extension_beyond;
    s.dimdle = style.line_extension;
    s.dimcen = style.center_mark;
    s.dimscale = style.scale;
    s.dimdli = style.baseline_spacing;
    s.dimlunit = style.unit_format.code();
    s.dimdec = i16::from(style.precision);
    s.dimfrac = style.fraction_format.code();
    s.dimdsep = i16::try_from(u32::from(style.decimal_separator.character())).unwrap_or(46);
    let feet_and_inches = match (style.suppress_zero_feet, style.suppress_zero_inches) {
        (true, true) => 0,
        (false, false) => 1,
        (false, true) => 2,
        (true, false) => 3,
    };
    s.dimzin = feet_and_inches
        + 4 * i16::from(style.suppress_leading_zeros)
        + 8 * i16::from(style.suppress_trailing_zeros);
    s.dimaunit = style.angular_unit.code();
    s.dimadec = i16::from(style.angular_precision);
    s.dimazin = i16::from(style.suppress_angular_leading_zeros)
        + 2 * i16::from(style.suppress_angular_trailing_zeros);
    if !style.prefix.is_empty() || !style.suffix.is_empty() {
        s.dimpost = format!("{}{MEASURED}{}", style.prefix, style.suffix);
    }
    style.text_style.clone_into(&mut s.dimtxsty);
    s.dimtxsty_handle = drawing
        .text_styles
        .get(&style.text_style)
        .map(|t| t.handle)
        .ok_or_else(|| {
            Error::new(
                ErrorCode::DwgWrite,
                format!(
                    "dimension style `{}` draws its text in text style `{}`, which the drawing does not have",
                    style.name, style.text_style
                ),
            )
        })?;
    s.dimblk = arrow_block(drawing, style.arrowhead).unwrap_or(Handle::NULL);
    match style.placement {
        TextPlacement::Centered => {
            s.dimtad = 0;
            s.dimtih = true;
            s.dimtoh = true;
            s.dimtofl = false;
        }
        TextPlacement::Above => {
            s.dimtad = 1;
            s.dimtih = false;
            s.dimtoh = false;
            s.dimtofl = true;
        }
    }
    Ok(s)
}

/// What AutoCAD 2026 set on each object of a dimension's block: the
/// dimension's own layer, and its colour and transparency, and for a line
/// its lineweight, `ByBlock`.
fn by_block(common: &mut EntityCommon, layer: &str, lineweight: bool) {
    layer.clone_into(&mut common.layer);
    common.color = Color::ByBlock;
    common.transparency = Transparency::ByBlock;
    if lineweight {
        common.line_weight = LineWeight::ByBlock;
    }
}

type V = [f64; 2];

/// An angle in degrees turned into `(-90, 90]`, as AutoCAD turns it.
fn readable(degrees: f64) -> f64 {
    let mut a = degrees.rem_euclid(360.0);
    if a > 270.0 {
        a -= 360.0;
    } else if a > 90.0 {
        a -= 180.0;
    }
    a
}

/// What the dimension's block holds, in AutoCAD's order, laid out as
/// `laid`, its text coded for MTEXT.
fn block_entities(laid: &LaidDimension, middle_aligned: bool) -> Vec<EntityType> {
    let mut entities = Vec::new();
    let layer = laid.text.layer.as_str();
    let size = laid.arrow_size;
    for part in &laid.parts {
        match part {
            DimensionPart::Line([a, b]) => {
                let mut line = DwgLine::from_points(vector(*a), vector(*b));
                by_block(&mut line.common, layer, true);
                entities.push(EntityType::Line(line));
            }
            DimensionPart::Arc {
                center,
                radius,
                start,
                end,
            } => {
                let mut arc = DwgArc::new();
                arc.center = vector(*center);
                arc.radius = *radius;
                arc.start_angle = start.rem_euclid(360.0).to_radians();
                arc.end_angle = end.rem_euclid(360.0).to_radians();
                by_block(&mut arc.common, layer, true);
                entities.push(EntityType::Arc(arc));
            }
            DimensionPart::Arrow { tip, base } => {
                let back: V = [base.x - tip.x, base.y - tip.y];
                match laid.arrowhead {
                    Arrowhead::ClosedFilled => {
                        // The corners on the left of the line's readable
                        // direction first, as AutoCAD writes them; on an
                        // arc, those on the right of the body.
                        let degrees = back[1].atan2(back[0]).to_degrees();
                        let angle = if laid.arrows_on_arc {
                            degrees - 180.0
                        } else {
                            readable(degrees)
                        }
                        .to_radians();
                        let across = [-angle.sin() * size / 6.0, angle.cos() * size / 6.0];
                        let corner = |k: f64| {
                            Vector3::new(base.x + k * across[0], base.y + k * across[1], base.z)
                        };
                        let mut solid =
                            DwgSolid::new(corner(1.0), corner(-1.0), vector(*tip), vector(*tip));
                        by_block(&mut solid.common, layer, false);
                        entities.push(EntityType::Solid(solid));
                    }
                    other => {
                        let Some(name) = other.block() else { continue };
                        let mut insert =
                            DwgInsert::new(name, vector(*tip)).with_uniform_scale(size);
                        insert.rotation = (-back[1]).atan2(-back[0]);
                        by_block(&mut insert.common, layer, true);
                        entities.push(EntityType::Insert(insert));
                    }
                }
            }
            DimensionPart::Text => entities.push(EntityType::MText(Box::new(block_text(
                &laid.text,
                &laid.pieces,
                middle_aligned,
            )))),
        }
    }
    for p in &laid.defpoints {
        let mut dot = DwgPoint::new();
        dot.location = vector(*p);
        by_block(&mut dot.common, layer, false);
        DEFPOINTS.clone_into(&mut dot.common.layer);
        entities.push(EntityType::Point(dot));
    }
    entities
}

/// What a dimension's text says, coded as AutoCAD codes it in its MTEXT:
/// the diameter and degree signs as their codes, and a stacked fraction as
/// `{\H1x;\S1/2;}`, `#` for its slash when it is diagonal.
pub(super) fn coded_pieces(pieces: &[TextPiece]) -> String {
    pieces
        .iter()
        .map(|piece| match piece {
            TextPiece::Plain(s) => mtext_code(s, None)
                .replace(DIAMETER_SIGN, DIAMETER_CODE)
                .replace(DEGREE_SIGN, DEGREE_CODE),
            TextPiece::Fraction {
                numerator,
                denominator,
                diagonal,
            } => format!(
                "{{\\H1x;\\S{numerator}{}{denominator};}}",
                if *diagonal { '#' } else { '/' }
            ),
        })
        .collect()
}

/// The MTEXT of a dimension's text, as AutoCAD writes it in the block:
/// coded as AutoCAD codes it, and, in a style whose tolerances align at
/// their middle, as Standard's do, aligned at its middle.
fn block_text(text: &MText, pieces: &[TextPiece], middle_aligned: bool) -> DwgMText {
    let coded = coded_pieces(pieces);
    let value = if middle_aligned {
        format!("{ALIGN_MIDDLE}{coded}")
    } else {
        coded
    };
    let mut written = DwgMText::with_value(value, vector(text.position));
    written.height = text.height;
    written.rectangle_width = 0.0;
    let radians = text.rotation.to_radians();
    written.rotation = radians;
    written.dwg_x_direction = Some(Vector3::new(radians.cos(), radians.sin(), 0.0));
    text.style.clone_into(&mut written.style);
    written.attachment_point = AttachmentPoint::MiddleCenter;
    by_block(&mut written.common, &text.layer, false);
    written
}

/// Adds the dimension `dimension`, laid out with `style` and its text
/// measured by `measure`, and its block, `*D` and the `number`th, owned by
/// it. The dimension's own layer, pen and ID are set on what this returns
/// before it is added.
pub(super) fn dwg_dimension(
    drawing: &mut CadDocument,
    dimension: &Dimension,
    style: &DimensionStyle,
    measure: &dyn Fn(&MText) -> MTextBox,
    number: usize,
    units: Unit,
) -> Result<EntityType, Error> {
    let laid = dimension.laid_out(style, units, measure);
    let name = format!("*D{number}");
    let mut record = BlockRecord::new(name.clone());
    record.flags.anonymous = true;
    record.set_handle(drawing.allocate_handle());
    record.block_entity_handle = drawing.allocate_handle();
    record.block_end_handle = drawing.allocate_handle();
    let owner = record.handle;
    drawing
        .block_records
        .add(record)
        .map_err(added("a dimension's block"))?;
    // AutoCAD aligns the text at its middle in the block when the style's
    // tolerances do, DIMTOLJ 1, as Standard's on an imperial sheet; ISO-25
    // on a metric one aligns them at the bottom.
    for mut entity in block_entities(&laid, !units.is_metric()) {
        entity.common_mut().owner_handle = owner;
        drawing
            .add_entity(entity)
            .map_err(added("an object of a dimension's block"))?;
    }
    let base = |kind| {
        let mut base = DimensionBase::new(kind);
        base.style_name.clone_from(&dimension.dimension_style);
        base.block_name.clone_from(&name);
        base.text_middle_point = vector(laid.text_point);
        base.actual_measurement = dimension.measurement();
        base.set_text_override(dimension.text.clone());
        if dimension.continued {
            overridden(&mut base, DIMTMOVE);
        }
        base
    };
    use acadrust::entities::DimensionType;
    let written = match &dimension.measured {
        Measured::Linear { points, angle } => {
            let mut d = DimensionLinear::new(vector(points[0]), vector(points[1]));
            d.base = base(DimensionType::Linear);
            d.definition_point = vector(dimension.location);
            d.base.definition_point = d.definition_point;
            d.rotation = angle.to_radians();
            d.base.dwg_flags_byte = FLAGS_DEFAULT_TEXT;
            DwgDimension::Linear(d)
        }
        Measured::Aligned { points } => {
            let mut d = DimensionAligned::new(vector(points[0]), vector(points[1]));
            d.base = base(DimensionType::Aligned);
            d.definition_point = vector(dimension.location);
            d.base.definition_point = d.definition_point;
            d.base.dwg_flags_byte = FLAGS_DEFAULT_TEXT;
            DwgDimension::Aligned(d)
        }
        Measured::Radius { center, point } => {
            let mut d = DimensionRadius::new(vector(*center), vector(*point));
            d.base = base(DimensionType::Radius);
            d.base.definition_point = vector(*center);
            radial_flags(&mut d.base, laid.text_placed);
            DwgDimension::Radius(d)
        }
        Measured::Diameter { center, point } => {
            let far = Point3::new(2.0 * center.x - point.x, 2.0 * center.y - point.y, point.z);
            let mut d = DimensionDiameter::new(vector(*point), vector(far));
            d.base = base(DimensionType::Diameter);
            d.base.definition_point = vector(far);
            radial_flags(&mut d.base, laid.text_placed);
            DwgDimension::Diameter(d)
        }
        Measured::Angular { lines } => {
            let mut d = DimensionAngular2Ln::new(
                vector(lines[0][0]),
                vector(lines[1][0]),
                vector(lines[1][1]),
            );
            d.base = base(DimensionType::Angular);
            d.base.actual_measurement = dimension.measurement().to_radians();
            // AutoCAD keeps the second line by its 13 and 14, the first by
            // its 15 and 10, and the arc's point as its 16.
            d.dimension_arc = vector(laid.arc_point.unwrap_or(dimension.location));
            d.first_point = vector(lines[1][0]);
            d.second_point = vector(lines[1][1]);
            d.angle_vertex = vector(lines[0][0]);
            d.definition_point = vector(lines[0][1]);
            d.base.definition_point = d.definition_point;
            d.base.dwg_flags_byte = FLAGS_DEFAULT_TEXT;
            DwgDimension::Angular2Ln(d)
        }
        Measured::ThreePointAngular { center, points } => {
            let mut d =
                DimensionAngular3Pt::new(vector(*center), vector(points[0]), vector(points[1]));
            d.base = base(DimensionType::Angular3Point);
            d.base.actual_measurement = dimension.measurement().to_radians();
            d.definition_point = vector(laid.arc_point.unwrap_or(dimension.location));
            d.first_point = vector(points[0]);
            d.second_point = vector(points[1]);
            d.angle_vertex = vector(*center);
            d.base.definition_point = d.definition_point;
            d.base.dwg_flags_byte = FLAGS_DEFAULT_TEXT;
            DwgDimension::Angular3Pt(d)
        }
        Measured::Ordinate { feature, axis } => {
            let mut d = DimensionOrdinate::new(
                vector(*feature),
                vector(dimension.location),
                *axis == OrdinateAxis::X,
            );
            d.base = base(DimensionType::Ordinate);
            d.definition_point = Vector3::new(0.0, 0.0, feature.z);
            d.base.definition_point = d.definition_point;
            d.base.dwg_flags_byte = FLAGS_DEFAULT_TEXT;
            DwgDimension::Ordinate(d)
        }
    };
    Ok(EntityType::Dimension(Box::new(written)))
}

/// What AutoCAD 2026 set on every radius and diameter dimension it made:
/// the text placed, unless it moved it out from too near the circle,
/// DIMUPT overridden, and no arc extension.
fn radial_flags(base: &mut DimensionBase, placed: bool) {
    if placed {
        base.dwg_flags_byte = FLAGS_PLACED_TEXT;
        base.text_user_positioned = true;
    } else {
        base.dwg_flags_byte = FLAGS_DEFAULT_TEXT;
        base.text_user_positioned = false;
    }
    overridden(base, DIMUPT);
    let mut extension = ExtendedDataRecord::new(RADIAL_EXTENSION);
    extension.add_value(XDataValue::Integer16(387));
    extension.add_value(XDataValue::Integer16(1));
    extension.add_value(XDataValue::Integer16(388));
    extension.add_value(XDataValue::Real(0.0));
    extension.add_value(XDataValue::Integer16(390));
    extension.add_value(XDataValue::Real(0.0));
    base.common.extended_data.upsert_record(extension);
}

/// The dimension's style overridden, as AutoCAD keeps it on a dimension,
/// with `code` set to 1.
fn overridden(base: &mut DimensionBase, code: i16) {
    let mut overrides = ExtendedDataRecord::new(ACAD);
    overrides.add_value(XDataValue::String(DSTYLE.to_owned()));
    overrides.add_value(XDataValue::ControlString("{".to_owned()));
    overrides.add_value(XDataValue::Integer16(code));
    overrides.add_value(XDataValue::Integer16(1));
    overrides.add_value(XDataValue::ControlString("}".to_owned()));
    base.common.extended_data.upsert_record(overrides);
}

/// The names of the blocks the drawing's dimensions draw from, which are
/// read with them and never as blocks of the drawing's.
pub(super) fn dimension_blocks(drawing: &CadDocument) -> Vec<String> {
    super::in_every_space(drawing)
        .into_iter()
        .filter_map(|e| match e {
            EntityType::Dimension(d) => Some(d.base().block_name.clone()),
            _ => None,
        })
        .filter(|n| !n.is_empty())
        .collect()
}

/// The handles of the arrowhead blocks the drawing's dimension styles draw
/// with, which are read with them.
pub(super) fn style_arrow_blocks(drawing: &CadDocument) -> Vec<Handle> {
    drawing
        .dim_styles
        .iter()
        .flat_map(|s| [s.dimblk, s.dimblk1, s.dimblk2, s.dimldrblk])
        .filter(|h| !h.is_null())
        .collect()
}

/// Whether the drawing's layer is AutoCAD's own `Defpoints`, as AutoCAD
/// makes it and as a build writes it, which is read with the dimensions
/// and not as a layer of the sheet's.
pub(super) fn is_own_defpoints(layer: &DwgLayer) -> bool {
    layer.name.eq_ignore_ascii_case(DEFPOINTS)
        && !layer.is_plottable
        && layer.color == Color::WHITE
        && layer.line_type.eq_ignore_ascii_case("Continuous")
        && layer.line_weight == LineWeight::Default
        && !layer.flags.off
        && !layer.flags.frozen
        && !layer.flags.locked
        && layer.description.is_empty()
}

/// The drawing's dimension styles the core has a model for, in table
/// order, and how many it has not, a style a dimension names that the
/// drawing lacks among them. The drawing's own `Standard` that no
/// dimension draws with is counted only when the core can read it.
pub(super) fn read_dimension_styles(drawing: &CadDocument) -> (Vec<DimensionStyle>, usize) {
    let mut used: Vec<String> = Vec::new();
    for entity in super::in_every_space(drawing) {
        if let EntityType::Dimension(d) = entity {
            let name = &d.base().style_name;
            if !used.iter().any(|u| u.eq_ignore_ascii_case(name)) {
                used.push(name.clone());
            }
        }
    }
    let mut styles = Vec::new();
    let mut not_read = 0;
    for written in drawing.dim_styles.iter() {
        let drawn_with = used.iter().any(|u| u.eq_ignore_ascii_case(&written.name));
        match read_style(drawing, written) {
            Some(style) => styles.push(style),
            // Every drawing's own `Standard`, which the build writes as the
            // DWG library makes it when no dimension draws with it and the
            // sheet does not list it, is no style of the sheet's.
            None if !drawn_with && written.name.eq_ignore_ascii_case(STANDARD_DIMENSION_STYLE) => {}
            None => not_read += 1,
        }
    }
    // A style a dimension names that the drawing does not have.
    not_read += used
        .iter()
        .filter(|name| drawing.dim_styles.get(name).is_none())
        .count();
    (styles, not_read)
}

fn read_style(drawing: &CadDocument, s: &DimStyle) -> Option<DimensionStyle> {
    let plain = s.dimclrd == 0
        && s.dimclre == 0
        && s.dimclrt == 0
        && s.dimclrd_true_color.is_none()
        && s.dimclre_true_color.is_none()
        && s.dimclrt_true_color.is_none()
        && s.dimlwd == -2
        && s.dimlwe == -2
        && !s.dimsd1
        && !s.dimsd2
        && !s.dimse1
        && !s.dimse2
        && s.dimtsz == 0.0
        && s.dimtvp == 0.0
        && s.dimjust == 0
        && !s.dimtix
        && !s.dimsoxd
        && s.dimtmove == 0
        && s.dimatfit == 3
        && s.dimtfill == 0
        && !s.dimtxtdirection
        && !s.dimfxlon
        && s.dimlfac == 1.0
        && s.dimrnd == 0.0
        && !s.dimalt
        && !s.dimtol
        && !s.dimlim
        && !s.dimsah
        && s.dimltex_handle.is_null()
        && s.dimltex1_handle.is_null()
        && s.dimltex2_handle.is_null()
        && !s.annotative
        && s.dimscale > 0.0
        && s.dimasz > 0.0
        && s.dimtxt > 0.0
        && s.dimgap >= 0.0
        && s.dimexo >= 0.0
        && s.dimexe >= 0.0
        && s.dimdle >= 0.0
        && s.dimcen >= 0.0
        && (0..=8).contains(&s.dimdec)
        && (0..=15).contains(&s.dimzin)
        && (0..=8).contains(&s.dimadec)
        && (0..=3).contains(&s.dimazin)
        && s.dimdli >= 0.0
        && !s.xref_dependent;
    if !plain {
        return None;
    }
    let placement = match (s.dimtad, s.dimtih, s.dimtoh, s.dimtofl) {
        (0, true, true, false) => TextPlacement::Centered,
        (1, false, false, true) => TextPlacement::Above,
        _ => return None,
    };
    let arrowhead = arrowhead_of(drawing, Some(s.dimblk))?;
    let (prefix, suffix) = if s.dimpost.is_empty() {
        (String::new(), String::new())
    } else {
        let (before, after) = s.dimpost.split_once(MEASURED)?;
        let codes = |t: &str| t.contains(['<', '>', '\\', '{', '}', '\n']);
        // `%%` is a control code in DIMPOST, `%%c` a diameter sign, which
        // the model does not hold.
        if codes(before) || codes(after) || s.dimpost.contains("%%") {
            return None;
        }
        (before.to_owned(), after.to_owned())
    };
    let text_style = drawing
        .text_styles
        .iter()
        .find(|t| t.handle == s.dimtxsty_handle)
        .map(|t| style_name(&t.name))?;
    Some(DimensionStyle {
        name: s.name.clone(),
        arrowhead,
        arrow_size: s.dimasz,
        text_style,
        text_height: s.dimtxt,
        gap: s.dimgap,
        extension_offset: s.dimexo,
        extension_beyond: s.dimexe,
        line_extension: s.dimdle,
        center_mark: s.dimcen,
        placement,
        unit_format: UnitFormat::of_code(s.dimlunit)?,
        precision: u8::try_from(s.dimdec).ok()?,
        fraction_format: FractionFormat::of_code(s.dimfrac)?,
        decimal_separator: DecimalSeparator::of_code(s.dimdsep)?,
        suppress_leading_zeros: s.dimzin & 4 != 0,
        suppress_trailing_zeros: s.dimzin & 8 != 0,
        suppress_zero_feet: matches!(s.dimzin & 3, 0 | 3),
        suppress_zero_inches: matches!(s.dimzin & 3, 0 | 2),
        angular_unit: AngularUnit::of_code(s.dimaunit)?,
        angular_precision: u8::try_from(s.dimadec).ok()?,
        suppress_angular_leading_zeros: s.dimazin & 1 != 0,
        suppress_angular_trailing_zeros: s.dimazin & 2 != 0,
        prefix,
        suffix,
        scale: s.dimscale,
        baseline_spacing: s.dimdli,
    })
}

/// The overrides of its style AutoCAD keeps on a dimension, by their
/// numbers and values, or none when its record of them cannot be read.
fn overrides(common: &EntityCommon) -> Option<Vec<(i16, XDataValue)>> {
    let Some(record) = common
        .extended_data
        .records()
        .iter()
        .find(|r| r.application_name.eq_ignore_ascii_case(ACAD))
    else {
        return Some(Vec::new());
    };
    let values = &record.values;
    let Some(start) = values
        .iter()
        .position(|v| matches!(v, XDataValue::String(s) if s == DSTYLE))
    else {
        return Some(Vec::new());
    };
    let mut codes = Vec::new();
    let mut rest = values[start + 1..].iter();
    if !matches!(rest.next(), Some(XDataValue::ControlString(s)) if s == "{") {
        return None;
    }
    loop {
        match rest.next()? {
            XDataValue::Integer16(code) => {
                codes.push((*code, rest.next()?.clone()));
            }
            XDataValue::ControlString(s) if s == "}" => return Some(codes),
            _ => return None,
        }
    }
}

/// Whether a radius's or diameter's arc extension draws nothing: none, or
/// one from an angle to the same angle.
fn no_arc_extension(common: &EntityCommon) -> bool {
    let Some(record) = common
        .extended_data
        .records()
        .iter()
        .find(|r| r.application_name.eq_ignore_ascii_case(RADIAL_EXTENSION))
    else {
        return true;
    };
    let mut angles = Vec::new();
    let mut values = record.values.iter();
    while let Some(value) = values.next() {
        if let XDataValue::Integer16(388 | 390) = value {
            match values.next() {
                Some(XDataValue::Real(a)) => angles.push(*a),
                _ => return false,
            }
        }
    }
    matches!(angles.as_slice(), [a, b] if a == b)
}

/// A dimension the core can read back whole: of one of the four kinds,
/// flat, drawn with a style the core read and no override of it but
/// AutoCAD's own DIMUPT, its text where AutoCAD keeps it.
pub(super) fn read_dimension(
    written: &DwgDimension,
    styles: &[DimensionStyle],
    layer: String,
) -> Option<Dimension> {
    let base = written.base();
    let plain = base.normal == Vector3::UNIT_Z
        && base.text_rotation == 0.0
        && base.horizontal_direction == 0.0
        && base.insertion_point == Vector3::ZERO
        && base.insertion_scale == Vector3::new(1.0, 1.0, 1.0)
        && base.insertion_rotation == 0.0
        && !base.flip_arrow1
        && !base.flip_arrow2
        && base.attachment_point == acadrust::entities::AttachmentPointType::MiddleCenter
        && base.line_spacing_factor == 1.0;
    if !plain {
        return None;
    }
    // AutoCAD's own overrides: DIMUPT on a radius or a diameter, DIMTMOVE
    // 1 on a dimension DIMCONTINUE made, which marks it continued.
    let radial = matches!(written, DwgDimension::Radius(_) | DwgDimension::Diameter(_));
    let mut continued = false;
    let one = |v: &XDataValue| matches!(v, XDataValue::Integer16(1));
    for (code, value) in overrides(&base.common)? {
        match code {
            DIMUPT if radial => {}
            DIMTMOVE if !radial && one(&value) => continued = true,
            _ => return None,
        }
    }
    if !no_arc_extension(&base.common) {
        return None;
    }
    if continued && matches!(written, DwgDimension::Ordinate(_)) {
        return None;
    }
    let style = styles
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(&base.style_name))?;
    let text = match base.text_override() {
        None | Some("") => None,
        Some(own) => {
            let coded = own.contains(['\\', '{', '}', '\n']) || own.contains("%%");
            if coded || own.trim().is_empty() {
                return None;
            }
            Some(own.to_owned())
        }
    };
    let text_point = point(base.text_middle_point);
    let near = |a: f64, b: f64, reach: f64| (a - b).abs() <= 1e-9 * (1.0 + reach);
    let (measured, location) = match written {
        DwgDimension::Linear(d) => {
            if d.ext_line_rotation != 0.0 || base.text_user_positioned {
                return None;
            }
            let angle = crate::description::normalize_degrees(d.rotation.to_degrees());
            let points = [point(d.first_point), point(d.second_point)];
            let location = point(d.definition_point);
            let (s, c) = angle.to_radians().sin_cos();
            on_line(&points, [c, s], location)?;
            (Measured::Linear { points, angle }, location)
        }
        DwgDimension::Aligned(d) => {
            if d.ext_line_rotation != 0.0 || base.text_user_positioned {
                return None;
            }
            let points = [point(d.first_point), point(d.second_point)];
            let dx = points[1].x - points[0].x;
            let dy = points[1].y - points[0].y;
            let length = dx.hypot(dy);
            if length == 0.0 {
                return None;
            }
            let location = point(d.definition_point);
            on_line(&points, [dx / length, dy / length], location)?;
            (Measured::Aligned { points }, location)
        }
        DwgDimension::Radius(d) => {
            let center = point(d.angle_vertex);
            let on = point(d.definition_point);
            let location =
                radial_location(center, on, text_point, style, base.text_user_positioned)?;
            (Measured::Radius { center, point: on }, location)
        }
        DwgDimension::Diameter(d) => {
            let near_point = point(d.angle_vertex);
            let far = point(d.definition_point);
            let center = Point3::new(
                (near_point.x + far.x) / 2.0,
                (near_point.y + far.y) / 2.0,
                (near_point.z + far.z) / 2.0,
            );
            let location = radial_location(
                center,
                near_point,
                text_point,
                style,
                base.text_user_positioned,
            )?;
            (
                Measured::Diameter {
                    center,
                    point: near_point,
                },
                location,
            )
        }
        DwgDimension::Angular2Ln(d) => {
            if base.text_user_positioned {
                return None;
            }
            let lines = [
                [point(d.angle_vertex), point(d.definition_point)],
                [point(d.first_point), point(d.second_point)],
            ];
            (Measured::Angular { lines }, point(d.dimension_arc))
        }
        DwgDimension::Angular3Pt(d) => {
            if base.text_user_positioned {
                return None;
            }
            let measured = Measured::ThreePointAngular {
                center: point(d.angle_vertex),
                points: [point(d.first_point), point(d.second_point)],
            };
            (measured, point(d.definition_point))
        }
        DwgDimension::Ordinate(d) => {
            let origin = point(d.definition_point);
            if base.text_user_positioned || origin.x != 0.0 || origin.y != 0.0 {
                return None;
            }
            let feature = point(d.feature_location);
            let axis = if d.is_ordinate_type_x {
                OrdinateAxis::X
            } else {
                OrdinateAxis::Y
            };
            let end = point(d.leader_endpoint);
            if feature.x == end.x && feature.y == end.y {
                return None;
            }
            (Measured::Ordinate { feature, axis }, end)
        }
        _ => return None,
    };
    let dimension = Dimension {
        measured,
        location,
        dimension_style: style.name.clone(),
        text,
        continued,
        layer,
        properties: Default::default(),
    };
    let points = dimension.points();
    let reach = points
        .iter()
        .map(|p| p.x.abs().max(p.y.abs()))
        .fold(0.0, f64::max);
    if points.iter().any(|p| !near(p.z, points[0].z, reach)) {
        return None;
    }
    if dimension.kind().is_angular() {
        // AutoCAD keeps the arc's point a third of the way round it, or two
        // thirds when the text is outside: the model keeps it at a third,
        // and writes it where AutoCAD keeps it again.
        let sector = dimension.sector()?;
        let share = {
            let to = [
                dimension.location.x - sector.vertex.x,
                dimension.location.y - sector.vertex.y,
            ];
            let at = to[1].atan2(to[0]).to_degrees();
            (at - sector.start).rem_euclid(360.0) / sector.sweep
        };
        if (share - 1.0 / 3.0).abs() <= 1e-9 {
            return Some(dimension);
        }
        if (share - 2.0 / 3.0).abs() > 1e-9 {
            return None;
        }
        let mut dimension = dimension;
        dimension.location = sector.arc_point();
        return Some(dimension);
    } else if dimension.kind() != crate::model::DimensionKind::Ordinate
        && dimension.measurement() == 0.0
    {
        return None;
    }
    Some(dimension)
}

/// Checks a linear or aligned dimension's definition point where AutoCAD
/// keeps it: on the line through its second point across its direction
/// `u`, and with neither point on the dimension line.
fn on_line(points: &[Point3; 2], u: V, location: Point3) -> Option<()> {
    let reach = points
        .iter()
        .chain([&location])
        .map(|p| p.x.abs().max(p.y.abs()))
        .fold(1.0, f64::max);
    let along = (location.x - points[1].x) * u[0] + (location.y - points[1].y) * u[1];
    let n = [-u[1], u[0]];
    let across = |p: &Point3| (location.x - p.x) * n[0] + (location.y - p.y) * n[1];
    let measures = ((points[1].x - points[0].x) * u[0] + (points[1].y - points[0].y) * u[1]).abs();
    (along.abs() <= 1e-9 * reach
        && across(&points[0]) != 0.0
        && across(&points[1]) != 0.0
        && measures > 0.0)
        .then_some(())
}

/// Where a radius's or diameter's text was placed, from where AutoCAD
/// keeps it, `text_point`: for text above its line, the point itself,
/// along the line from the centre through the arrow's point `on`; for
/// centred text that lands, the point where that line meets the landing,
/// level with the text's middle; for centred text that does not, the
/// point itself.
fn radial_location(
    center: Point3,
    on: Point3,
    text_point: Point3,
    style: &DimensionStyle,
    placed: bool,
) -> Option<Point3> {
    let d = [on.x - center.x, on.y - center.y];
    let r = d[0].hypot(d[1]);
    if r == 0.0 {
        return None;
    }
    let u = [d[0] / r, d[1] / r];
    let reach = [center, on, text_point]
        .iter()
        .map(|p| p.x.abs().max(p.y.abs()))
        .fold(1.0, f64::max);
    let steep =
        u[1].abs().atan2(u[0].abs()).to_degrees() >= crate::model::dimension::LANDING_ANGLE - 1e-9;
    let location = if style.placement == TextPlacement::Centered && steep {
        let t = (text_point.y - center.y) / u[1];
        Point3::new(center.x + u[0] * t, center.y + u[1] * t, center.z)
    } else if style.placement == TextPlacement::Above && !placed {
        // Text AutoCAD placed itself, moved out from too near the circle,
        // is kept at its middle, above the line: its place along the line
        // is where the model moves it to again.
        let t = (text_point.x - center.x) * u[0] + (text_point.y - center.y) * u[1];
        Point3::new(center.x + u[0] * t, center.y + u[1] * t, center.z)
    } else {
        text_point
    };
    // On the line from the centre.
    let v = [location.x - center.x, location.y - center.y];
    let across = v[0] * u[1] - v[1] * u[0];
    let along = v[0] * u[0] + v[1] * u[1];
    if across.abs() > 1e-9 * reach {
        return None;
    }
    if along > 0.0 {
        return Some(location);
    }
    // Text inside a circle too small to hold it, which AutoCAD puts past
    // the centre, on the far side from the arrow: the location is on the
    // arrow's side, as the line runs from the centre through it, and any
    // point between the centre and the arrow puts the text there again.
    Some(Point3::new(
        center.x + u[0] * r / 2.0,
        center.y + u[1] * r / 2.0,
        center.z,
    ))
}

#[cfg(test)]
mod tests {
    use acadrust::types::DxfVersion;

    use super::*;

    /// A drawing with the dimension style `style`, written as the build
    /// writes it, beside the DWG library's own `Standard`.
    fn drawing_with(style: &DimensionStyle) -> CadDocument {
        let mut drawing = CadDocument::with_version(DxfVersion::AC1032);
        drawing.assign_table_entry_handles();
        let mut written = dwg_style(&drawing, style, Unit::Mm).unwrap();
        written.set_handle(drawing.allocate_handle());
        drawing.dim_styles.add(written).unwrap();
        drawing
    }

    fn style(name: &str) -> DimensionStyle {
        DimensionStyle {
            name: name.to_owned(),
            ..DimensionStyle::iso_25(Unit::Mm)
        }
    }

    #[test]
    fn a_style_no_dimension_draws_with_is_read_and_the_drawings_own_standard_is_not() {
        let spare = DimensionStyle {
            precision: 4,
            ..style("spare")
        };
        let (read, not_read) = read_dimension_styles(&drawing_with(&spare));
        assert_eq!(read, vec![spare]);
        assert_eq!(not_read, 0);
    }

    #[test]
    fn a_style_the_model_cannot_hold_is_counted_whether_or_not_a_dimension_draws_with_it() {
        let mut drawing = drawing_with(&style("toleranced"));
        drawing.dim_styles.get_mut("toleranced").unwrap().dimtol = true;
        let (read, not_read) = read_dimension_styles(&drawing);
        assert!(read.is_empty());
        assert_eq!(not_read, 1);
    }

    /// A made-up right angle on an arc of radius 0.71, less across than an
    /// arrowhead is long: AutoCAD 2026, laying it out again, drew each
    /// arrowhead its full 2.5 from its tip through the vertex, which is
    /// what the drawing draws and the DWG holds.
    #[test]
    fn an_arrowhead_longer_than_its_arcs_circle_is_across_keeps_its_length() {
        let d = Dimension {
            measured: Measured::ThreePointAngular {
                center: Point3::new(50.0, 50.0, 0.0),
                points: [Point3::new(60.0, 50.0, 0.0), Point3::new(50.0, 60.0, 0.0)],
            },
            location: Point3::new(50.5, 50.5, 0.0),
            dimension_style: "ISO-25".to_owned(),
            text: None,
            continued: false,
            layer: "0".to_owned(),
            properties: Default::default(),
        };
        let measure = |t: &MText| MTextBox {
            width: 3.0,
            height: t.height,
            ..MTextBox::default()
        };
        let laid = d.laid_out(&DimensionStyle::iso_25(Unit::Mm), Unit::Mm, &measure);
        let solids: Vec<_> = block_entities(&laid, false)
            .into_iter()
            .filter_map(|e| match e {
                EntityType::Solid(s) => Some(s),
                _ => None,
            })
            .collect();
        assert_eq!(solids.len(), 2);
        // AutoCAD's tips and bases, the middles of the SOLIDs' backs.
        for (s, tip, base) in [
            (&solids[0], [50.7071, 50.0], [48.2071, 50.0]),
            (&solids[1], [50.0, 50.7071], [50.0, 48.2071]),
        ] {
            let back = [
                (s.first_corner.x + s.second_corner.x) / 2.0,
                (s.first_corner.y + s.second_corner.y) / 2.0,
            ];
            for (got, want) in [([s.third_corner.x, s.third_corner.y], tip), (back, base)] {
                assert!(
                    (got[0] - want[0]).abs() < 1e-4 && (got[1] - want[1]).abs() < 1e-4,
                    "{got:?} is not {want:?}"
                );
            }
            let drawn = crate::model::leader::arrowhead_shapes(
                Arrowhead::ClosedFilled,
                [s.third_corner.x, s.third_corner.y],
                back,
                laid.arrow_size,
            )
            .unwrap();
            let fill = &drawn.fills[0];
            let drawn_back = [
                (fill[1][0] + fill[2][0]) / 2.0,
                (fill[1][1] + fill[2][1]) / 2.0,
            ];
            assert!((drawn_back[0] - back[0]).hypot(drawn_back[1] - back[1]) < 1e-9);
        }
    }

    #[test]
    fn a_dimpost_holding_a_control_code_is_not_read() {
        let mut drawing = drawing_with(&style("coded"));
        let written = drawing.dim_styles.get("coded").unwrap().clone();
        assert!(read_style(&drawing, &written).is_some());
        for post in ["%%c<>", "<>%%d"] {
            post.clone_into(&mut drawing.dim_styles.get_mut("coded").unwrap().dimpost);
            let written = drawing.dim_styles.get("coded").unwrap().clone();
            assert!(read_style(&drawing, &written).is_none(), "{post}");
        }
    }
}
