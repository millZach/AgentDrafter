//! Multileader styles, multileaders and older leaders in a DWG.
//!
//! Part of the seam ([`super`]): only this module and its siblings name
//! acadrust. A multileader is AutoCAD's MULTILEADER, written with its
//! MLEADERSTYLE, its one leader root, its landing and its multiline text,
//! each value as AutoCAD 2026 wrote made-up multileaders: the text's
//! location where its layout puts it, which AutoCAD keeps until it lays the
//! multileader out again, and then puts in the same place. What the
//! multileader gives of its own, its arrowhead, its size, its landing, its
//! text style, height and attachment, is written as an override of its
//! style, flagged as AutoCAD flags one.
//!
//! An older leader is AutoCAD's LEADER, with its annotation an MTEXT of its
//! own that the leader points at and that names the leader among its
//! reactors, as AutoCAD writes one. A LEADER draws with a dimension style:
//! it names `Standard`, which every drawing has, and carries what it draws
//! with as overrides in AutoCAD's extended data, `DSTYLE`, as AutoCAD writes
//! a leader's own arrowhead and size: the arrow's size, the gap to the
//! annotation, a scale of 1 and, but for a closed filled arrowhead, the
//! arrowhead's block. So it draws the same whatever `Standard` holds.
//!
//! An arrowhead other than closed filled is a block AutoCAD names, such as
//! `_Dot`; the drawing holds it once, with what AutoCAD 2026 put in it.
//!
//! Block content is written as AutoCAD 2026 wrote its own: the block's
//! record, where the block's base point sits, its scale, rotation and
//! connection, the matrix that places it, and a value for each of the
//! block's attribute definitions, by the definition's handle and its place
//! among them, from 1.
//!
//! A multileader, an older leader and a multileader style are read back
//! when the core has a model for them: text content, or a block the model
//! places, sitting where the model places it, straight leader lines
//! and a horizontal landing, one of the arrowheads and attachments the
//! pipeline draws, and the pens AutoCAD's own Standard style gives, or
//! they are counted not read. An older leader's annotation is read with it,
//! and not as an mtext of its own.

use acadrust::entities::multileader::{
    BlockAttribute, BlockContentConnectionType, FlowDirectionType, LeaderLine, LeaderRoot,
    LineSpacingStyle, MultiLeaderPathType, MultiLeaderPropertyOverrideFlags as Override,
    TextAlignmentType, TextAngleType, TextAttachmentDirectionType, TextAttachmentPointType,
    TextAttachmentType,
};
use acadrust::entities::{
    EntityType, HooklineDirection, Leader as DwgLeader, LeaderCreationType, LeaderPathType,
    Line as DwgLine, LwPolyline, LwVertex, MText as DwgMText, MultiLeader,
};
use acadrust::objects::{
    LeaderContentType as StyleContent, MultiLeaderDrawOrderType, MultiLeaderPathType as StylePath,
    MultiLeaderStyle as DwgStyle, ObjectType, TextAngleType as StyleAngle,
    TextAttachmentDirectionType as StyleDirection, TextAttachmentType as StyleAttachment,
};
use acadrust::tables::{BlockRecord, TableEntry};
use acadrust::types::{Color, Handle, LineWeight, Vector2, Vector3};
use acadrust::xdata::{ExtendedDataRecord, XDataValue};
use acadrust::CadDocument;

use super::text::{dwg_mtext, mtext_code, mtext_contents, style_name};
use super::{point, vector};
use crate::error::{Error, ErrorCode};
use crate::model::{
    Arrowhead, Attribute, Block, BlockConnection, Insert, LaidContent, Leader, LeaderAnnotation,
    LeaderAttachment, Lineweight, MLeader, MLeaderBlock, MLeaderContent, MLeaderStyle, MLeaderText,
    MTextBox, Object, ObjectColor, ObjectLineweight, Outline, Point3, Sheet,
    STANDARD_MLEADER_STYLE,
};
use crate::text::MText;

/// The application AutoCAD keeps dimension style overrides under.
const ACAD: &str = "ACAD";

/// The record of AutoCAD's dimension style overrides in extended data.
const DSTYLE: &str = "DSTYLE";

/// The dimension style an older leader names, which every drawing has.
const DIMENSION_STYLE: &str = "Standard";

/// AutoCAD's numbers of the dimension variables an older leader draws with.
const DIMSCALE: i16 = 40;
const DIMASZ: i16 = 41;
const DIMGAP: i16 = 147;
const DIMTAD: i16 = 77;
const DIMCLRD: i16 = 176;
const DIMLWD: i16 = 371;
const DIMLDRBLK: i16 = 341;

/// What AutoCAD flags on every multileader it makes with text: its content
/// type, its text's alignment and its use of default text.
fn base_overrides() -> Override {
    Override::CONTENT_TYPE | Override::TEXT_ALIGNMENT | Override::ENABLE_USE_DEFAULT_MTEXT
}

/// What a multileader may override of its style and still be read: what
/// AutoCAD flags on every one, and what the model holds of its own.
fn overrides_read() -> Override {
    base_overrides()
        | Override::ARROWHEAD
        | Override::ARROWHEAD_SIZE
        | Override::LANDING_DISTANCE
        | Override::ENABLE_DOGLEG
        | Override::LANDING_GAP
        | Override::TEXT_STYLE
        | Override::TEXT_HEIGHT
        | Override::TEXT_LEFT_ATTACHMENT
        | Override::TEXT_RIGHT_ATTACHMENT
}

fn dwg_color(color: ObjectColor) -> Color {
    match color {
        ObjectColor::ByLayer => Color::ByLayer,
        ObjectColor::ByBlock => Color::ByBlock,
        ObjectColor::Index(index) => Color::Index(index),
    }
}

fn dwg_lineweight(lineweight: ObjectLineweight) -> LineWeight {
    match lineweight {
        ObjectLineweight::ByLayer => LineWeight::ByLayer,
        ObjectLineweight::ByBlock => LineWeight::ByBlock,
        ObjectLineweight::Default => LineWeight::Default,
        ObjectLineweight::Width(width) => LineWeight::Value(width.hundredths()),
    }
}

fn object_color(color: Color) -> Option<ObjectColor> {
    match color {
        Color::ByLayer => Some(ObjectColor::ByLayer),
        Color::ByBlock => Some(ObjectColor::ByBlock),
        Color::Index(index) => Some(ObjectColor::Index(index)),
        _ => None,
    }
}

fn object_lineweight(lineweight: LineWeight) -> Option<ObjectLineweight> {
    match lineweight {
        LineWeight::ByLayer => Some(ObjectLineweight::ByLayer),
        LineWeight::ByBlock => Some(ObjectLineweight::ByBlock),
        LineWeight::Default => Some(ObjectLineweight::Default),
        LineWeight::Value(hundredths) => {
            Lineweight::from_hundredths(hundredths).map(ObjectLineweight::Width)
        }
    }
}

/// The drawing's dictionary of multileader styles.
fn styles_dictionary(drawing: &CadDocument) -> Option<Handle> {
    drawing
        .objects
        .iter()
        .find_map(|(handle, object)| match object {
            ObjectType::Dictionary(d) => d
                .entries
                .iter()
                .any(|(name, entry)| {
                    name == STANDARD_MLEADER_STYLE
                        && matches!(
                            drawing.objects.get(entry),
                            Some(ObjectType::MultiLeaderStyle(_))
                        )
                })
                .then_some(*handle),
            _ => None,
        })
}

/// The handle of the multileader style named `name` in the drawing.
fn style_handle(drawing: &CadDocument, name: &str) -> Option<Handle> {
    drawing
        .objects
        .iter()
        .find_map(|(handle, object)| match object {
            ObjectType::MultiLeaderStyle(style) if style.name == name => Some(*handle),
            _ => None,
        })
}

/// The handle of the block AutoCAD draws `arrowhead` from, which the
/// drawing holds once it is added: none for closed filled.
fn arrow_block(drawing: &CadDocument, arrowhead: Arrowhead) -> Option<Handle> {
    let name = arrowhead.block()?;
    drawing
        .block_records
        .iter()
        .find(|r| r.name.eq_ignore_ascii_case(name))
        .map(|r| r.handle)
}

/// Adds the block AutoCAD draws `arrowhead` from, unless the drawing holds
/// it, with what AutoCAD 2026 put in it: for `_Dot`, a closed polyline 0.5
/// wide round a circle of radius 0.25, and a line from 0.5 to 1 behind the
/// point; for `_Open`, the triangle's two sides and its axis; for `_None`,
/// nothing. Each is on layer `0`, `ByBlock` in colour and linetype.
fn add_arrow_block(drawing: &mut CadDocument, arrowhead: Arrowhead) -> Result<(), Error> {
    let Some(name) = arrowhead.block() else {
        return Ok(());
    };
    if arrow_block(drawing, arrowhead).is_some() {
        return Ok(());
    }
    let mut record = BlockRecord::new(name);
    record.set_handle(drawing.allocate_handle());
    record.block_entity_handle = drawing.allocate_handle();
    record.block_end_handle = drawing.allocate_handle();
    let owner = record.handle;
    drawing.block_records.add(record).map_err(|e| {
        Error::new(
            ErrorCode::DwgWrite,
            format!("could not add the arrowhead block `{name}` to the drawing: {e}"),
        )
    })?;
    for mut entity in arrow_entities(arrowhead) {
        entity.common_mut().owner_handle = owner;
        drawing.add_entity(entity).map_err(|e| {
            Error::new(
                ErrorCode::DwgWrite,
                format!("could not fill the arrowhead block `{name}`: {e}"),
            )
        })?;
    }
    Ok(())
}

/// What AutoCAD 2026 put in the block of `arrowhead`, before it has an
/// owner.
fn arrow_entities(arrowhead: Arrowhead) -> Vec<EntityType> {
    let by_block = |common: &mut acadrust::entities::EntityCommon, weight: bool| {
        common.color = Color::ByBlock;
        "ByBlock".clone_into(&mut common.linetype);
        if weight {
            common.line_weight = LineWeight::ByBlock;
        }
    };
    let line = |from: (f64, f64), to: (f64, f64)| {
        let mut line = DwgLine::from_points(
            Vector3::new(from.0, from.1, 0.0),
            Vector3::new(to.0, to.1, 0.0),
        );
        by_block(&mut line.common, true);
        EntityType::Line(line)
    };
    let wing = 1.0 / 6.0;
    match arrowhead {
        Arrowhead::Dot => {
            let mut dot = LwPolyline::new();
            dot.is_closed = true;
            dot.constant_width = 0.5;
            for x in [-0.25, 0.25] {
                let mut vertex = LwVertex::new(Vector2::new(x, 0.0));
                vertex.bulge = 1.0;
                vertex.start_width = 0.5;
                vertex.end_width = 0.5;
                dot.vertices.push(vertex);
            }
            by_block(&mut dot.common, false);
            vec![EntityType::LwPolyline(dot), line((-0.5, 0.0), (-1.0, 0.0))]
        }
        Arrowhead::Open => vec![
            line((-1.0, wing), (0.0, 0.0)),
            line((-1.0, -wing), (0.0, 0.0)),
            line((0.0, 0.0), (-1.0, 0.0)),
        ],
        Arrowhead::ClosedFilled | Arrowhead::None => Vec::new(),
    }
}

/// Numbers rounded to a billionth, so that two objects a rounding apart
/// compare the same.
fn rounded(value: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::Number(n) => n
            .as_f64()
            .and_then(|x| serde_json::Number::from_f64((x * 1e9).round() / 1e9 + 0.0))
            .map_or(Value::Number(n), Value::Number),
        Value::Array(items) => Value::Array(items.into_iter().map(rounded).collect()),
        Value::Object(keys) => {
            Value::Object(keys.into_iter().map(|(k, v)| (k, rounded(v))).collect())
        }
        other => other,
    }
}

/// What `objects` draw, in an order of their own: each object as its
/// rounded JSON, a line's two ends in an order of their own too, sorted,
/// so that two blocks drawn alike compare equal however their objects, or
/// a line's ends, are written in turn.
fn drawn_alike(objects: &[Object]) -> Vec<String> {
    fn ends_in_order(value: &mut serde_json::Value) {
        if let serde_json::Value::Object(keys) = value {
            if let (Some(from), Some(to)) = (keys.get("from"), keys.get("to")) {
                if serde_json::to_string(to).ok() < serde_json::to_string(from).ok() {
                    let (from, to) = (from.clone(), to.clone());
                    keys.insert("from".to_owned(), to);
                    keys.insert("to".to_owned(), from);
                }
            }
            for inner in keys.values_mut() {
                ends_in_order(inner);
            }
        }
    }
    let mut drawn: Vec<String> = objects
        .iter()
        .map(|object| {
            let mut value = rounded(serde_json::to_value(object).unwrap_or_default());
            ends_in_order(&mut value);
            value.to_string()
        })
        .collect();
    drawn.sort();
    drawn
}

/// Refuses a block of the sheet's, from its block library, that has the
/// name of an arrowhead block a leader of the sheet needs but is drawn
/// otherwise than AutoCAD's: the drawing holds one block of a name. One
/// drawn as AutoCAD's is written once and shared.
fn check_library_arrowheads(sheet: &Sheet, arrowheads: &[Arrowhead]) -> Result<(), Error> {
    for arrowhead in arrowheads {
        let Some(name) = arrowhead.block() else {
            continue;
        };
        let Some(block) = sheet
            .blocks
            .iter()
            .find(|b| b.name.eq_ignore_ascii_case(name))
        else {
            continue;
        };
        let expected: Option<Vec<Object>> = arrow_entities(*arrowhead)
            .iter()
            .map(|e| super::read_object(e, &[]))
            .collect();
        let same = expected.is_some_and(|expected| {
            block.base_point.x == 0.0
                && block.base_point.y == 0.0
                && block.base_point.z == 0.0
                && drawn_alike(&block.objects) == drawn_alike(&expected)
        });
        if !same {
            return Err(Error::new(
                ErrorCode::DuplicateName,
                format!(
                    "the block library's block `{}` differs from AutoCAD's arrowhead block that a leader needs, `{name}`, and a drawing holds one block of a name; rename the library's block, or draw it as AutoCAD's",
                    block.name
                ),
            ));
        }
    }
    Ok(())
}

/// Writes every multileader style the sheet's multileaders draw with, and
/// every one it lists, into the drawing, and the blocks of every arrowhead
/// its leaders draw. `Standard` is in every drawing. The drawing's text
/// styles have their handles already.
pub(super) fn add_leader_styles(drawing: &mut CadDocument, sheet: &Sheet) -> Result<(), Error> {
    let mut wanted: Vec<MLeaderStyle> = sheet.mleader_styles.clone();
    let mut arrowheads = Vec::new();
    for object in &sheet.objects {
        match object {
            Object::MLeader(m) => {
                if !wanted.iter().any(|s| s.name == m.mleader_style) {
                    if let Some(style) = sheet.mleader_style(&m.mleader_style) {
                        wanted.push(style);
                    }
                }
                arrowheads.extend(m.arrowhead);
            }
            Object::Leader(l) => arrowheads.push(l.arrowhead),
            _ => {}
        }
    }
    arrowheads.extend(wanted.iter().map(|s| s.arrowhead));
    check_library_arrowheads(sheet, &arrowheads)?;
    for arrowhead in arrowheads {
        add_arrow_block(drawing, arrowhead)?;
    }
    if wanted.is_empty() {
        return Ok(());
    }
    let dictionary = styles_dictionary(drawing).ok_or_else(|| {
        Error::new(
            ErrorCode::DwgWrite,
            "the drawing has no ACAD_MLEADERSTYLE dictionary to add multileader styles to",
        )
    })?;
    for style in &wanted {
        let (handle, new) = match style_handle(drawing, &style.name) {
            Some(handle) => (handle, false),
            None => (drawing.allocate_handle(), true),
        };
        let written = dwg_style(drawing, sheet, style, handle, dictionary);
        drawing
            .objects
            .insert(handle, ObjectType::MultiLeaderStyle(written));
        if new {
            if let Some(ObjectType::Dictionary(d)) = drawing.objects.get_mut(&dictionary) {
                d.add_entry(&style.name, handle);
            }
        }
    }
    Ok(())
}

fn style_attachment(attachment: LeaderAttachment) -> StyleAttachment {
    StyleAttachment::from(attachment.code())
}

fn dwg_style(
    drawing: &CadDocument,
    sheet: &Sheet,
    style: &MLeaderStyle,
    handle: Handle,
    dictionary: Handle,
) -> DwgStyle {
    let mut written = DwgStyle::new(&style.name);
    written.handle = handle;
    written.owner_handle = dictionary;
    if style.name == STANDARD_MLEADER_STYLE {
        STANDARD_MLEADER_STYLE.clone_into(&mut written.description);
    }
    written.content_type = StyleContent::MText;
    written.multileader_draw_order = MultiLeaderDrawOrderType::LeaderFirst;
    written.path_type = StylePath::StraightLineSegments;
    written.line_color = dwg_color(style.leader_color);
    written.line_type_handle = drawing.line_types.get("ByBlock").map(|l| l.handle);
    written.line_weight = dwg_lineweight(style.leader_lineweight);
    written.enable_landing = true;
    written.enable_dogleg = style.landing > 0.0;
    written.landing_distance = style.landing;
    written.landing_gap = style.landing_gap;
    written.arrowhead_handle = arrow_block(drawing, style.arrowhead);
    written.arrowhead_size = style.arrow_size;
    written.text_style_handle = drawing.text_styles.get(&style.text_style).map(|s| s.handle);
    written.text_left_attachment = style_attachment(style.attachment);
    written.text_right_attachment = style_attachment(style.attachment);
    written.text_top_attachment = StyleAttachment::CenterOfText;
    written.text_bottom_attachment = StyleAttachment::CenterOfText;
    written.text_attachment_direction = StyleDirection::Horizontal;
    written.text_angle_type = StyleAngle::Horizontal;
    written.text_color = Color::ByBlock;
    written.text_height = style.text_height;
    written.align_space = style.text_height;
    written.enable_block_scale = true;
    written.enable_block_rotation = true;
    written.break_gap_size = if sheet.units.is_metric() {
        crate::units::Unit::Mm.convert(3.75, sheet.units)
    } else {
        crate::units::Unit::In.convert(0.125, sheet.units)
    };
    written
}

fn attachment_type(attachment: LeaderAttachment) -> TextAttachmentType {
    TextAttachmentType::from(attachment.code())
}

/// What AutoCAD 2026 flagged on every multileader it made with block
/// content: its content type, its block, and the block's scale and
/// rotation.
fn block_overrides() -> Override {
    Override::CONTENT_TYPE
        | Override::BLOCK_CONTENT
        | Override::BLOCK_CONTENT_SCALE
        | Override::BLOCK_CONTENT_ROTATION
}

/// The DWG multileader for one of ours, laid out with `style`, which the
/// drawing holds, its block content's block one of `blocks`, which the
/// drawing holds too.
pub(super) fn dwg_mleader(
    drawing: &CadDocument,
    mleader: &MLeader,
    style: &MLeaderStyle,
    measure: &dyn Fn(&MText) -> MTextBox,
    blocks: &[Block],
) -> Result<MultiLeader, Error> {
    let style_handle = style_handle(drawing, &style.name).ok_or_else(|| {
        Error::new(
            ErrorCode::DwgWrite,
            format!(
                "a multileader draws with multileader style `{}`, which the drawing does not hold",
                style.name
            ),
        )
    })?;
    let laid = mleader.laid_out(style, measure, blocks);
    let right = laid.side > 0.0;
    let mut overrides = Override::empty();
    if laid.arrowhead != style.arrowhead {
        overrides |= Override::ARROWHEAD;
    }
    if laid.arrow_size != style.arrow_size {
        overrides |= Override::ARROWHEAD_SIZE;
    }
    if laid.landing != style.landing {
        overrides |= Override::LANDING_DISTANCE;
        if (laid.landing > 0.0) != (style.landing > 0.0) {
            overrides |= Override::ENABLE_DOGLEG;
        }
    }
    if laid.landing_gap != style.landing_gap {
        overrides |= Override::LANDING_GAP;
    }
    let mut written = MultiLeader::new();
    written.style_handle = Some(style_handle);
    written.path_type = MultiLeaderPathType::StraightLineSegments;
    written.line_color = dwg_color(style.leader_color);
    written.line_type_handle = drawing.line_types.get("ByBlock").map(|l| l.handle);
    written.line_weight = dwg_lineweight(style.leader_lineweight);
    written.enable_landing = true;
    written.enable_dogleg = laid.landing > 0.0;
    written.dogleg_length = laid.landing;
    written.arrowhead_handle = arrow_block(drawing, laid.arrowhead);
    written.arrowhead_size = laid.arrow_size;
    written.text_color = Color::ByBlock;
    written.text_frame = false;
    written.text_top_attachment = TextAttachmentType::CenterOfText;
    written.text_bottom_attachment = TextAttachmentType::CenterOfText;
    written.text_attachment_direction = TextAttachmentDirectionType::Horizontal;
    written.text_angle_type = TextAngleType::Horizontal;
    let context = &mut written.context;
    context.scale_factor = 1.0;
    context.content_base_point = vector(laid.content_base());
    context.arrowhead_size = laid.arrow_size;
    context.landing_gap = laid.landing_gap;
    context.text_top_attachment = TextAttachmentType::CenterOfText;
    context.text_bottom_attachment = TextAttachmentType::CenterOfText;
    context.base_point = laid
        .lines
        .first()
        .and_then(|l| l.first())
        .map_or(Vector3::ZERO, |p| vector(*p));
    match &laid.content {
        LaidContent::Text {
            text,
            attachment: laid_attachment,
            ..
        } => {
            overrides |= base_overrides();
            let text_style_handle = drawing.text_styles.get(&text.style).map(|s| s.handle);
            if text.style != style.text_style {
                overrides |= Override::TEXT_STYLE;
            }
            if text.height != style.text_height {
                overrides |= Override::TEXT_HEIGHT;
            }
            if *laid_attachment != style.attachment {
                overrides |= Override::TEXT_LEFT_ATTACHMENT | Override::TEXT_RIGHT_ATTACHMENT;
            }
            let attachment = attachment_type(*laid_attachment);
            let (alignment, attached) = if right {
                (TextAlignmentType::Left, TextAttachmentPointType::Left)
            } else {
                (TextAlignmentType::Right, TextAttachmentPointType::Right)
            };
            written.text_style_handle = text_style_handle;
            written.text_height = text.height;
            written.text_left_attachment = attachment;
            written.text_right_attachment = attachment;
            written.text_attachment_point = attached;
            written.text_alignment = alignment;
            let context = &mut written.context;
            context.text_height = text.height;
            context.text_left_attachment = attachment;
            context.text_right_attachment = attachment;
            context.text_alignment = alignment;
            context.has_text_contents = true;
            context.text_string = mtext_code(&text.contents, None);
            context.text_style_handle = text_style_handle;
            context.text_location = vector(text.position);
            context.text_direction = Vector3::UNIT_X;
            context.text_attachment_point = attached;
            context.text_flow_direction = FlowDirectionType::ByStyle;
            context.line_spacing_factor = 1.0;
            context.line_spacing_style = LineSpacingStyle::AtLeast;
            context.text_color = Color::ByBlock;
            context.background_fill_color = Color::ByLayer;
            context.background_scale_factor = 0.0;
            context.word_break = false;
        }
        LaidContent::Block { insert, .. } => {
            let connection = mleader
                .content
                .block()
                .map_or(BlockConnection::Extents, |b| b.connection);
            overrides |= block_overrides();
            if connection != BlockConnection::Extents {
                overrides |= Override::BLOCK_CONTENT_CONNECTION;
            }
            let record = drawing
                .block_records
                .iter()
                .find(|r| r.name == insert.block)
                .ok_or_else(|| {
                    Error::new(
                        ErrorCode::DwgWrite,
                        format!(
                            "a multileader's content is block `{}`, which the drawing does not hold",
                            insert.block
                        ),
                    )
                })?;
            let block = blocks.iter().find(|b| b.name == insert.block);
            // Each of the block's attribute definitions, in order, with the
            // value the content gives it, as AutoCAD 2026 wrote them: by its
            // place among the block's attribute definitions, from 1.
            let definitions: Vec<Handle> = drawing
                .entities_in_block(&record.name)
                .filter(|e| matches!(e, EntityType::AttributeDefinition(_)))
                .map(|e| e.common().handle)
                .collect();
            let places: Vec<usize> = block.map_or(Vec::new(), |b| {
                b.objects
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| matches!(o, Object::AttributeDefinition(_)))
                    .map(|(p, _)| p)
                    .collect()
            });
            written.block_attributes = definitions
                .iter()
                .zip(&places)
                .enumerate()
                .map(|(k, (handle, &p))| {
                    let value = block
                        .and_then(|b| b.attribute_value(insert, p))
                        .unwrap_or_default();
                    let mut attribute = BlockAttribute::new(value);
                    attribute.attribute_definition_handle = Some(*handle);
                    attribute.index = i16::try_from(k + 1).unwrap_or(i16::MAX);
                    attribute
                })
                .collect();
            let attachment = attachment_type(style.attachment);
            written.content_type = acadrust::entities::LeaderContentType::Block;
            written.text_style_handle =
                drawing.text_styles.get(&style.text_style).map(|s| s.handle);
            written.text_left_attachment = attachment;
            written.text_right_attachment = attachment;
            written.text_attachment_point = TextAttachmentPointType::Left;
            written.text_alignment = TextAlignmentType::Left;
            written.block_content_handle = Some(record.handle);
            written.block_content_color = Color::ByBlock;
            written.block_connection_type = block_connection_type(connection);
            written.block_rotation = insert.rotation.to_radians();
            written.block_scale = Vector3::new(insert.scale, insert.scale, insert.scale);
            let context = &mut written.context;
            context.text_height = style.text_height;
            context.text_left_attachment = attachment;
            context.text_right_attachment = attachment;
            context.text_alignment = TextAlignmentType::Left;
            context.text_normal = Vector3::ZERO;
            context.text_flow_direction = FlowDirectionType::Horizontal;
            context.text_color = Color::ByLayer;
            context.word_break = false;
            context.background_fill_color = Color::ByLayer;
            context.background_scale_factor = 1.5;
            context.has_text_contents = false;
            context.has_block_contents = true;
            context.block_content_handle = Some(record.handle);
            context.block_content_normal = Vector3::UNIT_Z;
            context.block_content_location = vector(insert.position);
            context.block_content_scale = Vector3::new(insert.scale, insert.scale, insert.scale);
            context.block_rotation = insert.rotation.to_radians();
            context.block_content_color = Color::ByLayer;
            context.block_connection_type = block_connection_type(connection);
            context.transform_matrix = block_transform(
                insert,
                block.map_or(Point3::new(0.0, 0.0, 0.0), |b| b.base_point),
            );
        }
    }
    written.property_override_flags = overrides;
    let mut root = LeaderRoot::new(0);
    root.content_valid = true;
    root.unknown = true;
    root.connection_point = vector(laid.connection);
    root.direction = Vector3::new(laid.side, 0.0, 0.0);
    root.landing_distance = laid.landing;
    root.text_attachment_direction = TextAttachmentDirectionType::Horizontal;
    for (i, points) in laid.lines.iter().enumerate() {
        let before: Vec<Vector3> = points[..points.len() - 1]
            .iter()
            .map(|p| vector(*p))
            .collect();
        let mut line = LeaderLine::from_points(i as i32, before);
        line.path_type = MultiLeaderPathType::StraightLineSegments;
        line.line_color = Color::ByBlock;
        line.line_weight = LineWeight::ByBlock;
        line.arrowhead_size = 0.0;
        root.lines.push(line);
    }
    written.context.leader_roots.push(root);
    Ok(written)
}

fn block_connection_type(connection: BlockConnection) -> BlockContentConnectionType {
    match connection {
        BlockConnection::Extents => BlockContentConnectionType::BlockExtents,
        BlockConnection::Insertion => BlockContentConnectionType::BasePoint,
    }
}

/// The matrix that places block content, row by row, as AutoCAD 2026 wrote
/// it: what takes a point in the block's own coordinates to the sheet,
/// scaled, turned and moved so that the block's base point, `base`, sits
/// where the insert puts it.
#[rustfmt::skip]
fn block_transform(insert: &Insert, base: Point3) -> [f64; 16] {
    let (sin, cos) = insert.rotation.to_radians().sin_cos();
    let s = insert.scale;
    let p = insert.position;
    let tx = p.x - s * (base.x * cos - base.y * sin);
    let ty = p.y - s * (base.x * sin + base.y * cos);
    let tz = p.z - s * base.z;
    [
        s * cos, -s * sin, 0.0, tx,
        s * sin, s * cos, 0.0, ty,
        0.0, 0.0, s, tz,
        0.0, 0.0, 0.0, 1.0,
    ]
}

/// The DWG leader for one of ours, and its annotation's MTEXT, if it has
/// one. Once both are in the drawing, [`link_annotation`] ties them.
pub(super) fn dwg_leader(drawing: &CadDocument, leader: &Leader) -> (DwgLeader, Option<DwgMText>) {
    let mut written = DwgLeader::new();
    DIMENSION_STYLE.clone_into(&mut written.dimension_style);
    written.vertices = leader.points.iter().map(|p| vector(*p)).collect();
    written.path_type = LeaderPathType::StraightLine;
    written.arrow_enabled = true;
    written.creation_type = if leader.annotation.is_some() {
        LeaderCreationType::WithText
    } else {
        LeaderCreationType::NoAnnotation
    };
    written.hookline_direction = if leader.side() > 0.0 {
        HooklineDirection::Opposite
    } else {
        HooklineDirection::Same
    };
    written.hookline_enabled = leader.hooked();
    written.horizontal_direction = Vector3::UNIT_X;
    written.normal = Vector3::UNIT_Z;
    let mut overrides = ExtendedDataRecord::new(ACAD);
    overrides.add_value(XDataValue::String(DSTYLE.to_owned()));
    overrides.add_value(XDataValue::ControlString("{".to_owned()));
    overrides.add_value(XDataValue::Integer16(DIMSCALE));
    overrides.add_value(XDataValue::Real(1.0));
    overrides.add_value(XDataValue::Integer16(DIMASZ));
    overrides.add_value(XDataValue::Real(leader.arrow_size));
    overrides.add_value(XDataValue::Integer16(DIMGAP));
    overrides.add_value(XDataValue::Real(leader.gap));
    // The annotation sits level with the leader's end, not above it: what
    // AutoCAD's own Standard has, and not acadrust's.
    overrides.add_value(XDataValue::Integer16(DIMTAD));
    overrides.add_value(XDataValue::Integer16(0));
    if let Some(block) = arrow_block(drawing, leader.arrowhead) {
        overrides.add_value(XDataValue::Integer16(DIMLDRBLK));
        overrides.add_value(XDataValue::Handle(block));
    }
    overrides.add_value(XDataValue::ControlString("}".to_owned()));
    written.common.extended_data.upsert_record(overrides);
    let annotation = leader.annotation_text().map(|t| dwg_mtext(&t));
    (written, annotation)
}

/// Ties an older leader and its annotation together, as AutoCAD does: the
/// leader points at the MTEXT, and the MTEXT names the leader among its
/// reactors.
pub(super) fn link_annotation(drawing: &mut CadDocument, leader: Handle, annotation: Handle) {
    if let Some(EntityType::Leader(l)) = drawing.get_entity_mut(leader) {
        l.annotation_handle = annotation;
    }
    if let Some(EntityType::MText(t)) = drawing.get_entity_mut(annotation) {
        if !t.common.reactors.contains(&leader) {
            t.common.reactors.push(leader);
        }
    }
}

/// The name of the block with handle `handle`, if the drawing holds one.
fn block_name(drawing: &CadDocument, handle: Handle) -> Option<&str> {
    drawing
        .block_records
        .iter()
        .find(|r| r.handle == handle)
        .map(|r| r.name.as_str())
}

/// The arrowhead of the block `handle` names: closed filled for none.
fn arrowhead_of(drawing: &CadDocument, handle: Option<Handle>) -> Option<Arrowhead> {
    match handle.filter(|h| !h.is_null()) {
        None => Some(Arrowhead::ClosedFilled),
        Some(handle) => Arrowhead::of_block(block_name(drawing, handle)?),
    }
}

/// The name of the text style with handle `handle`.
fn text_style_name(drawing: &CadDocument, handle: Option<Handle>) -> Option<String> {
    let handle = handle?;
    drawing
        .text_styles
        .iter()
        .find(|s| s.handle == handle)
        .map(|s| style_name(&s.name))
}

/// The drawing's multileader styles the core has a model for, and how many
/// it has not.
pub(super) fn read_mleader_styles(drawing: &CadDocument) -> (Vec<MLeaderStyle>, usize) {
    let mut styles = Vec::new();
    let mut not_read = 0;
    for object in drawing.objects.values() {
        if let ObjectType::MultiLeaderStyle(written) = object {
            match read_style(drawing, written) {
                Some(style) => styles.push(style),
                None => not_read += 1,
            }
        }
    }
    (styles, not_read)
}

fn read_style(drawing: &CadDocument, written: &DwgStyle) -> Option<MLeaderStyle> {
    let left = written.text_left_attachment as i16;
    let plain = written.content_type == StyleContent::MText
        && written.path_type == StylePath::StraightLineSegments
        && written.enable_landing
        && left == written.text_right_attachment as i16
        && written.text_attachment_direction == StyleDirection::Horizontal
        && written.text_angle_type == StyleAngle::Horizontal
        && written.text_color == Color::ByBlock
        && !written.text_frame
        && written.scale_factor == 1.0
        && !written.is_annotative;
    if !plain {
        return None;
    }
    Some(MLeaderStyle {
        name: written.name.clone(),
        text_style: text_style_name(drawing, written.text_style_handle)
            .unwrap_or_else(|| crate::text::STANDARD.to_owned()),
        text_height: written.text_height,
        arrowhead: arrowhead_of(drawing, written.arrowhead_handle)?,
        arrow_size: written.arrowhead_size,
        landing: if written.enable_dogleg {
            written.landing_distance
        } else {
            0.0
        },
        landing_gap: written.landing_gap,
        attachment: LeaderAttachment::of_code(left)?,
        leader_color: object_color(written.line_color)?,
        leader_lineweight: object_lineweight(written.line_weight)?,
    })
}

/// What a multileader with block content may override of its style and
/// still be read: what AutoCAD flags on every one, its connection, what the
/// model holds of its own, and its text's alignment, which AutoCAD 2026
/// flagged once it had mirrored one, and which block content does not use.
fn block_overrides_read() -> Override {
    block_overrides()
        | Override::BLOCK_CONTENT_CONNECTION
        | Override::TEXT_ALIGNMENT
        | Override::ARROWHEAD
        | Override::ARROWHEAD_SIZE
        | Override::LANDING_DISTANCE
        | Override::ENABLE_DOGLEG
}

/// A multileader the core can read back whole, drawing with one of
/// `styles`: text content, or a block among `blocks` placed where the
/// model places it; one leader root with straight leader lines and a
/// horizontal landing on the side its first leader line runs to; and no
/// pen of its own for its leader lines.
pub(super) fn read_mleader(
    drawing: &CadDocument,
    written: &MultiLeader,
    styles: &[MLeaderStyle],
    blocks: &[Block],
    layer: String,
) -> Option<MLeader> {
    let context = &written.context;
    let [root] = context.leader_roots.as_slice() else {
        return None;
    };
    let plain = written.path_type == MultiLeaderPathType::StraightLineSegments
        && written.enable_landing
        && written.scale_factor == 1.0
        && context.scale_factor == 1.0
        && !written.enable_annotation_scale
        && root.break_points.is_empty()
        && root.direction.y == 0.0
        && root.direction.x.abs() == 1.0
        && !root.lines.is_empty();
    if !plain {
        return None;
    }
    let style_name = written
        .style_handle
        .and_then(|h| match drawing.objects.get(&h) {
            Some(ObjectType::MultiLeaderStyle(s)) => Some(s.name.clone()),
            _ => None,
        })?;
    let style = styles.iter().find(|s| s.name == style_name)?;
    if object_color(written.line_color)? != style.leader_color
        || object_lineweight(written.line_weight)? != style.leader_lineweight
    {
        return None;
    }
    let connection = point(root.connection_point);
    let mut leaders = Vec::with_capacity(root.lines.len());
    for line in &root.lines {
        let straight = line.path_type == MultiLeaderPathType::StraightLineSegments
            && line.break_points.is_empty()
            && line.break_infos.is_empty()
            && line.override_flags.is_empty()
            && !line.points.is_empty();
        if !straight {
            return None;
        }
        let mut points: Vec<Point3> = line.points.iter().map(|p| point(*p)).collect();
        points.push(connection);
        if points.windows(2).any(|pair| pair[0] == pair[1]) {
            return None;
        }
        leaders.push(points);
    }
    let landing = if written.enable_dogleg {
        root.landing_distance
    } else {
        0.0
    };
    let arrowhead = arrowhead_of(drawing, written.arrowhead_handle)?;
    let differs = |same: bool| !same;
    let content = match written.content_type {
        acadrust::entities::LeaderContentType::MText => read_text_content(drawing, written, style)?,
        acadrust::entities::LeaderContentType::Block => {
            MLeaderContent::Block(read_block_content(drawing, written, blocks)?)
        }
        _ => return None,
    };
    // An override the model does not hold, such as the text's colour,
    // would be lost.
    let held = match content {
        MLeaderContent::Text(_) => overrides_read(),
        MLeaderContent::Block(_) => block_overrides_read(),
    };
    if !held.contains(written.property_override_flags) {
        return None;
    }
    let block_content = matches!(content, MLeaderContent::Block(_));
    let read = MLeader {
        leaders,
        content,
        landing: differs(landing == style.landing).then_some(landing),
        gap: differs(block_content || context.landing_gap == style.landing_gap)
            .then_some(context.landing_gap),
        arrowhead: differs(arrowhead == style.arrowhead).then_some(arrowhead),
        arrow_size: differs(written.arrowhead_size == style.arrow_size)
            .then_some(written.arrowhead_size),
        mleader_style: style.name.clone(),
        layer,
        properties: Default::default(),
    };
    // The landing runs to the side the model gives it, or it is not read.
    if read.side() != root.direction.x {
        return None;
    }
    // Block content sits where the model places it, or it is not read.
    if let MLeaderContent::Block(_) = &read.content {
        let laid = read.laid_out(style, &|_| MTextBox::default(), blocks);
        let LaidContent::Block { insert, base } = &laid.content else {
            return None;
        };
        let reach = read
            .leaders
            .iter()
            .flatten()
            .chain([&insert.position])
            .map(|p| p.x.abs().max(p.y.abs()))
            .fold(1.0, f64::max);
        let near = |a: Point3, b: Vector3| {
            (a.x - b.x).abs() <= reach * 1e-9 && (a.y - b.y).abs() <= reach * 1e-9
        };
        if !near(insert.position, context.block_content_location)
            || !near(*base, context.content_base_point)
        {
            return None;
        }
    }
    Some(read)
}

/// A multileader's text content, as the model holds it, or none.
fn read_text_content(
    drawing: &CadDocument,
    written: &MultiLeader,
    style: &MLeaderStyle,
) -> Option<MLeaderContent> {
    let context = &written.context;
    let plain = context.has_text_contents
        && !context.has_block_contents
        && !written.text_frame
        && written.text_color == Color::ByBlock
        && written.text_angle_type == TextAngleType::Horizontal
        && written.text_attachment_direction == TextAttachmentDirectionType::Horizontal
        && written.text_left_attachment as i16 == written.text_right_attachment as i16
        && context.text_rotation == 0.0
        && context.text_width == 0.0
        && context.line_spacing_factor == 1.0
        && context.column_type == 0
        && !context.background_fill_enabled;
    if !plain {
        return None;
    }
    let (text, width_factor) = mtext_contents(&context.text_string)?;
    if width_factor.is_some() {
        return None;
    }
    let attachment = LeaderAttachment::of_code(written.text_left_attachment as i16)?;
    let text_style = text_style_name(drawing, context.text_style_handle)?;
    let differs = |same: bool| !same;
    Some(MLeaderContent::Text(MLeaderText {
        text,
        style: differs(text_style == style.text_style).then_some(text_style),
        height: differs(context.text_height == style.text_height).then_some(context.text_height),
        attachment: differs(attachment == style.attachment).then_some(attachment),
    }))
}

/// A multileader's block content, as the model holds it, or none: a block
/// among `blocks` that the model places, upright on the sheet, scaled the
/// same along every axis, in the multileader's own colour, with a value
/// for each of its attribute definitions, by their handles.
fn read_block_content(
    drawing: &CadDocument,
    written: &MultiLeader,
    blocks: &[Block],
) -> Option<MLeaderBlock> {
    let context = &written.context;
    let handle = context.block_content_handle?;
    // The entity's scale and rotation, which the context repeats, within a
    // rounding: AutoCAD 2026 wrote them a last digit apart once it had
    // mirrored a multileader.
    let scale = written.block_scale;
    let within = |a: f64, b: f64| (a - b).abs() <= 1e-9 * a.abs().max(1.0);
    let plain = context.has_block_contents
        && !context.has_text_contents
        && written.block_content_handle == Some(handle)
        && written.block_content_color == Color::ByBlock
        && context.block_content_normal == Vector3::UNIT_Z
        && scale.x > 0.0
        && scale.x == scale.y
        && scale.y == scale.z
        && within(context.block_content_scale.x, scale.x)
        && within(context.block_content_scale.y, scale.y)
        && within(context.block_content_scale.z, scale.z)
        && within(written.block_rotation, context.block_rotation);
    if !plain {
        return None;
    }
    let record = drawing.block_records.iter().find(|r| r.handle == handle)?;
    let block = blocks.iter().find(|b| b.name == record.name)?;
    Outline::of(block, blocks).ok()?;
    let connection = match written.block_connection_type {
        BlockContentConnectionType::BlockExtents => BlockConnection::Extents,
        BlockContentConnectionType::BasePoint => BlockConnection::Insertion,
    };
    // Each attribute definition of the block, in order, and its value.
    let definitions: Vec<Handle> = drawing
        .entities_in_block(&record.name)
        .filter(|e| matches!(e, EntityType::AttributeDefinition(_)))
        .map(|e| e.common().handle)
        .collect();
    let carried: Vec<&crate::model::AttributeDefinition> = block.attribute_definitions().collect();
    if definitions.len() != carried.len() || written.block_attributes.len() > definitions.len() {
        return None;
    }
    let mut values: Vec<Option<String>> = vec![None; carried.len()];
    for attribute in &written.block_attributes {
        let k = definitions
            .iter()
            .position(|h| Some(*h) == attribute.attribute_definition_handle)?;
        if values[k].is_some() || attribute.width != 0.0 {
            return None;
        }
        values[k] = Some(attribute.text.clone());
    }
    let attributes = carried
        .iter()
        .zip(values)
        .map(|(definition, value)| Attribute {
            tag: definition.tag.clone(),
            value: value.unwrap_or_else(|| definition.default.clone()),
        })
        .collect();
    let degrees = crate::description::normalize_degrees(written.block_rotation.to_degrees());
    Some(MLeaderBlock {
        block: block.name.clone(),
        attributes,
        scale: scale.x,
        rotation: degrees,
        connection,
    })
}

/// The dimension style overrides AutoCAD keeps on an object, by their
/// numbers.
fn dimension_overrides(common: &acadrust::entities::EntityCommon) -> Vec<(i16, XDataValue)> {
    let Some(record) = common
        .extended_data
        .records()
        .iter()
        .find(|r| r.application_name.eq_ignore_ascii_case(ACAD))
    else {
        return Vec::new();
    };
    let values = &record.values;
    let Some(start) = values
        .iter()
        .position(|v| matches!(v, XDataValue::String(s) if s == DSTYLE))
    else {
        return Vec::new();
    };
    let mut overrides = Vec::new();
    let mut rest = values[start + 1..].iter();
    if !matches!(rest.next(), Some(XDataValue::ControlString(s)) if s == "{") {
        return Vec::new();
    }
    while let Some(value) = rest.next() {
        match value {
            XDataValue::Integer16(code) => {
                if let Some(v) = rest.next() {
                    overrides.push((*code, v.clone()));
                }
            }
            _ => break,
        }
    }
    overrides
}

fn real(value: &XDataValue) -> Option<f64> {
    match value {
        XDataValue::Real(x) | XDataValue::Distance(x) | XDataValue::ScaleFactor(x) => Some(*x),
        _ => None,
    }
}

fn whole(value: &XDataValue) -> Option<i16> {
    match value {
        XDataValue::Integer16(x) => Some(*x),
        XDataValue::Integer32(x) => i16::try_from(*x).ok(),
        _ => None,
    }
}

/// An older leader the core can read back whole, and the handle of the
/// MTEXT that is its annotation, read with it: straight, flat, its
/// annotation, when it has one, multiline text where the model puts it, and
/// drawn with no pen of its own from its dimension style.
pub(super) fn read_leader(
    drawing: &CadDocument,
    written: &DwgLeader,
    layer: String,
) -> Option<(Leader, Option<Handle>)> {
    let plain = written.path_type == LeaderPathType::StraightLine
        && written.normal == Vector3::UNIT_Z
        && written.horizontal_direction == Vector3::UNIT_X
        && written.vertices.len() >= 2
        && written
            .vertices
            .iter()
            .all(|v| v.z == written.vertices[0].z);
    if !plain {
        return None;
    }
    let style = drawing.dim_styles.get(&written.dimension_style)?;
    let overrides = dimension_overrides(&written.common);
    let over = |code: i16| overrides.iter().find(|(c, _)| *c == code).map(|(_, v)| v);
    let scale = match over(DIMSCALE) {
        Some(v) => real(v)?,
        None => style.dimscale,
    };
    let scale = if scale == 0.0 { 1.0 } else { scale };
    let size = match over(DIMASZ) {
        Some(v) => real(v)?,
        None => style.dimasz,
    } * scale;
    let gap = match over(DIMGAP) {
        Some(v) => real(v)?,
        None => style.dimgap,
    } * scale;
    let color = match over(DIMCLRD) {
        Some(v) => whole(v)?,
        None => style.dimclrd,
    };
    let lineweight = match over(DIMLWD) {
        Some(v) => whole(v)?,
        None => style.dimlwd,
    };
    let above = match over(DIMTAD) {
        Some(v) => whole(v)?,
        None => style.dimtad,
    };
    // Its line takes the leader's own pen, `ByBlock` in colour and
    // lineweight, and its annotation sits level with its end.
    if size <= 0.0 || gap < 0.0 || color != 0 || lineweight != -2 || above != 0 {
        return None;
    }
    let arrowhead = if written.arrow_enabled {
        let block = match over(DIMLDRBLK) {
            Some(XDataValue::Handle(h)) => Some(*h),
            Some(_) => return None,
            None => Some(style.dimldrblk),
        };
        arrowhead_of(drawing, block)?
    } else {
        Arrowhead::None
    };
    let points: Vec<Point3> = written.vertices.iter().map(|v| point(*v)).collect();
    if points.windows(2).any(|pair| pair[0] == pair[1]) {
        return None;
    }
    let mut read = Leader {
        points,
        arrowhead,
        arrow_size: size,
        gap,
        annotation: None,
        layer,
        properties: Default::default(),
    };
    let annotation = match written.creation_type {
        LeaderCreationType::NoAnnotation => None,
        LeaderCreationType::WithText if !written.annotation_handle.is_null() => {
            let Some(EntityType::MText(t)) = drawing.get_entity(written.annotation_handle) else {
                return None;
            };
            // The annotation draws as the model writes it: on the leader's
            // layer, in its pen, and shown.
            let pen = super::read_properties(&t.common);
            let presented = !t.common.invisible
                && pen.is_some()
                && pen == super::read_properties(&written.common)
                && super::layer_of(&t.common) == super::layer_of(&written.common);
            if !presented {
                return None;
            }
            let text = super::text::read_mtext(t, read.layer.clone())?;
            read.annotation = Some(LeaderAnnotation {
                text: text.contents.clone(),
                style: text.style.clone(),
                height: text.height,
            });
            // The annotation sits where the model puts it, or it is not read.
            let expected = read.annotation_text()?;
            let reach = read
                .points
                .iter()
                .map(|p| p.x.abs().max(p.y.abs()))
                .fold(1.0, f64::max);
            let near = |a: f64, b: f64| (a - b).abs() <= reach * 1e-9;
            let placed = near(text.position.x, expected.position.x)
                && near(text.position.y, expected.position.y)
                && text.justify == expected.justify
                && text.width == 0.0
                && text.rotation == 0.0
                && text.width_factor.is_none();
            if !placed {
                return None;
            }
            Some(written.annotation_handle)
        }
        _ => return None,
    };
    // AutoCAD hooks towards the side the model gives the annotation.
    if read.hooked() {
        let towards = if read.side() > 0.0 {
            HooklineDirection::Opposite
        } else {
            HooklineDirection::Same
        };
        if written.hookline_direction != towards {
            return None;
        }
    }
    Some((read, annotation))
}

/// The MTEXT that is an older leader's annotation, if it has one.
pub(super) fn annotation_of(drawing: &CadDocument, written: &DwgLeader) -> Option<Handle> {
    let handle = written.annotation_handle;
    (written.creation_type == LeaderCreationType::WithText
        && !handle.is_null()
        && matches!(drawing.get_entity(handle), Some(EntityType::MText(_))))
    .then_some(handle)
}

/// The handles of the arrowhead blocks the drawing's leaders and
/// multileader styles draw with, which are read with them and not as
/// blocks of their own.
pub(super) fn arrow_blocks(drawing: &CadDocument) -> Vec<Handle> {
    let mut handles = Vec::new();
    for object in drawing.objects.values() {
        if let ObjectType::MultiLeaderStyle(s) = object {
            handles.extend(s.arrowhead_handle);
        }
    }
    for entity in drawing.model_space_entities() {
        match entity {
            EntityType::MultiLeader(m) => handles.extend(m.arrowhead_handle),
            EntityType::Leader(l) => {
                for (code, value) in dimension_overrides(&l.common) {
                    if let (DIMLDRBLK, XDataValue::Handle(h)) = (code, value) {
                        handles.push(h);
                    }
                }
            }
            _ => {}
        }
    }
    handles.retain(|h| !h.is_null());
    handles
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Cursor;
    use std::path::{Path, PathBuf};

    use acadrust::entities::multileader::MultiLeaderPropertyOverrideFlags as Override;
    use acadrust::entities::{EntityType, Insert as DwgInsert};
    use acadrust::types::{Color, Vector3};
    use acadrust::{CadDocument, DwgReader, DwgWriter};

    use crate::build::{build, BuildRequest};
    use crate::model::{DrawingContents, Object};

    /// An empty folder for one test under the system's temporary folder.
    fn scratch(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("agentdrafter-dwg-leader-{}", std::process::id()))
            .join(test);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A made-up sheet: a multileader with a dot, and an older leader with
    /// its annotation, on layer `notes`.
    const SHEET: &str = "units: mm
layers:
  notes: {color: yellow}
objects:
  - mleader: {leaders: [[[0, 0, 0], [20, 10, 0]]], content: {text: TB-1}, arrowhead: dot, layer: notes}
  - leader: {points: [[50, 0, 0], [60, 20, 0]], annotation: {text: SEE NOTE}, layer: notes}
";

    /// Builds `text` in a folder of its own, and gives the folder and the
    /// drawing the DWG holds.
    fn built(test: &str, text: &str) -> (PathBuf, CadDocument) {
        let dir = scratch(test);
        let description = dir.join("sheet.yaml");
        fs::write(&description, text).unwrap();
        build(&BuildRequest {
            description,
            output: None,
            overwrite: false,
        })
        .unwrap_or_else(|e| panic!("{e}"));
        let bytes = fs::read(dir.join("sheet.dwg")).unwrap();
        let drawing = DwgReader::from_stream(Cursor::new(bytes)).read().unwrap();
        (dir, drawing)
    }

    /// Writes `drawing` beside the built DWG and reads it as the core does.
    fn read_back(dir: &Path, drawing: &CadDocument) -> DrawingContents {
        let path = dir.join("changed.dwg");
        fs::write(&path, DwgWriter::write_to_vec(drawing).unwrap()).unwrap();
        super::super::read(&path).unwrap()
    }

    fn handle_of(drawing: &CadDocument, kind: &str) -> acadrust::types::Handle {
        drawing
            .model_space_entities()
            .find(|e| {
                matches!(
                    (kind, e),
                    ("mleader", EntityType::MultiLeader(_))
                        | ("leader", EntityType::Leader(_))
                        | ("mtext", EntityType::MText(_))
                )
            })
            .map(|e| e.common().handle)
            .unwrap()
    }

    #[test]
    fn the_sheet_reads_back_whole() {
        let (dir, drawing) = built("whole", SHEET);
        let read = read_back(&dir, &drawing);
        assert_eq!(read.objects_not_read, 0, "{read:?}");
        assert_eq!(read.objects.len(), 2);
    }

    #[test]
    fn a_multileader_with_an_override_the_model_does_not_hold_is_not_read() {
        let (dir, mut drawing) = built("override", SHEET);
        let handle = handle_of(&drawing, "mleader");
        if let Some(EntityType::MultiLeader(m)) = drawing.get_entity_mut(handle) {
            m.property_override_flags |= Override::TEXT_COLOR;
        }
        let read = read_back(&dir, &drawing);
        assert_eq!(read.objects_not_read, 1, "{read:?}");
        assert!(!read.objects.iter().any(|o| matches!(o, Object::MLeader(_))));
    }

    #[test]
    fn an_annotation_with_a_presentation_of_its_own_is_not_read_nor_its_leader() {
        type Change = fn(&mut acadrust::entities::EntityCommon);
        let changes: [(&str, Change); 5] = [
            ("colour", |c| c.color = Color::Index(1)),
            ("linetype", |c| "Continuous".clone_into(&mut c.linetype)),
            ("lineweight", |c| {
                c.line_weight = acadrust::types::LineWeight::Value(50)
            }),
            ("layer", |c| "0".clone_into(&mut c.layer)),
            ("visibility", |c| c.invisible = true),
        ];
        for (what, change) in changes {
            let (dir, mut drawing) = built(&format!("annotation-{what}"), SHEET);
            let handle = handle_of(&drawing, "mtext");
            if let Some(EntityType::MText(t)) = drawing.get_entity_mut(handle) {
                change(&mut t.common);
            }
            let read = read_back(&dir, &drawing);
            assert_eq!(read.objects.len(), 1, "{what}: {read:?}");
            assert!(
                matches!(read.objects[0], Object::MLeader(_)),
                "{what}: {read:?}"
            );
            assert_eq!(read.objects_not_read, 2, "{what}: {read:?}");
        }
    }

    #[test]
    fn an_ordinary_insert_of_an_arrowhead_block_reads_with_its_block() {
        let (dir, mut drawing) = built("insert-dot", SHEET);
        drawing
            .add_entity(EntityType::Insert(DwgInsert::new(
                "_Dot",
                Vector3::new(100.0, 50.0, 0.0),
            )))
            .unwrap();
        let read = read_back(&dir, &drawing);
        assert_eq!(read.objects_not_read, 0, "{read:?}");
        assert_eq!(read.blocks_not_read, 0, "{read:?}");
        assert!(read.objects.iter().any(|o| matches!(o, Object::MLeader(_))));
        assert!(read
            .objects
            .iter()
            .any(|o| matches!(o, Object::Insert(i) if i.block == "_Dot")));
        let names: Vec<&str> = read.blocks.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["_Dot"]);
    }

    /// AutoCAD 2026 laid a hexagon out again, its base point 2 below its
    /// own origin, at (x, 105), and wrote its matrix moving the origin to
    /// 107: the matrix takes the block's own points to the sheet.
    #[test]
    fn block_contents_matrix_places_the_blocks_own_origin() {
        let insert = crate::model::Insert {
            block: "HEX".into(),
            position: crate::model::Point3::new(62.0, 105.0, 0.0),
            scale: 1.0,
            rotation: 0.0,
            attributes: Vec::new(),
            name: None,
            layer: "0".into(),
            properties: Default::default(),
        };
        let m = super::block_transform(&insert, crate::model::Point3::new(0.0, -2.0, 0.0));
        assert_eq!((m[3], m[7]), (62.0, 107.0));
    }
}
