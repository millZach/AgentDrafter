//! Block definitions, attribute definitions and inserts in a DWG.
//!
//! Part of the seam ([`super`]): only this module and its parent name
//! acadrust. A block is written as AutoCAD writes one: a block record
//! holding the block's objects, its attribute definitions among them, with
//! the drawing's unit so an insert is not scaled again. An insert carries
//! one attribute for each attribute definition of its block, in the order
//! the block defines them, drawn where the definition is, moved, scaled and
//! turned with the insert ([`attribute_text`]). An insert's name, as every
//! object's ID and name, is kept in AgentDrafter's extended data ([`super::ids`]).
//!
//! An insert reads back only when its attributes are where its block puts
//! them, so the core's model, which holds their values, is the whole truth.

use acadrust::entities::{
    AttributeDefinition as DwgAttributeDefinition, AttributeEntity, AttributeFlags,
    DrawingDirection, HorizontalAlignment as Horizontal, Insert as DwgInsert, LineSpacingStyle,
    MText as DwgMText, MTextFlag, VerticalAlignment as Vertical,
};
use acadrust::tables::{AppId, BlockRecord, TableEntry};
use acadrust::types::{Handle, Vector3};
use acadrust::CadDocument;

use super::text::{
    attachment_of, dwg_mtext, mtext_code, mtext_styled_contents, style_name, text_code,
    text_contents,
};
use super::{point, vector};
use crate::description::normalize_degrees;
use crate::error::{Error, ErrorCode};
use crate::model::block::{
    attachment_to_justify, definition_mtext, definition_text, justify_to_attachment,
};
use crate::model::{
    attribute_mtext, attribute_text, Attribute, AttributeDefinition, Block, Insert, Point3, Xref,
    XrefKind, LAYER_ZERO,
};
use crate::text::{Justify, MText, Text};

/// The registered application whose extended data on an object holds its
/// ID, and on an insert its name as well, each as a key followed by its
/// value ([`super::ids`]).
pub(super) const APPLICATION: &str = "AGENTDRAFTER";

/// Each justification, with AutoCAD's horizontal and vertical alignment of
/// an attribute for it.
const JUSTIFICATIONS: [(Justify, Horizontal, Vertical); 13] = [
    (Justify::Left, Horizontal::Left, Vertical::Baseline),
    (Justify::Center, Horizontal::Center, Vertical::Baseline),
    (Justify::Right, Horizontal::Right, Vertical::Baseline),
    (Justify::Middle, Horizontal::Middle, Vertical::Baseline),
    (Justify::TopLeft, Horizontal::Left, Vertical::Top),
    (Justify::TopCenter, Horizontal::Center, Vertical::Top),
    (Justify::TopRight, Horizontal::Right, Vertical::Top),
    (Justify::MiddleLeft, Horizontal::Left, Vertical::Middle),
    (Justify::MiddleCenter, Horizontal::Center, Vertical::Middle),
    (Justify::MiddleRight, Horizontal::Right, Vertical::Middle),
    (Justify::BottomLeft, Horizontal::Left, Vertical::Bottom),
    (Justify::BottomCenter, Horizontal::Center, Vertical::Bottom),
    (Justify::BottomRight, Horizontal::Right, Vertical::Bottom),
];

/// How far a point, height or angle of an attribute read back may sit from
/// where its block puts it, relative to its size, and still be read as
/// there. A DWG holds angles in radians, so an insert's rotation comes back
/// a little off in degrees, and so do its attributes' places.
const TOLERANCE: f64 = 1e-9;

/// Registers AgentDrafter's application, whose extended data holds the IDs
/// of objects and the names of inserts.
pub(super) fn register_application(drawing: &mut CadDocument) -> Result<(), Error> {
    register_named_application(drawing, APPLICATION)
}

/// Registers an application name in the drawing's APPID table if absent.
pub(super) fn register_named_application(
    drawing: &mut CadDocument,
    name: &str,
) -> Result<(), Error> {
    if drawing
        .app_ids
        .iter()
        .any(|a| a.name.eq_ignore_ascii_case(name))
    {
        return Ok(());
    }
    drawing.app_ids.add(AppId::new(name)).map_err(|e| {
        Error::new(
            ErrorCode::DwgWrite,
            format!("could not register the application `{name}` in the drawing: {e}"),
        )
    })
}

/// Adds an empty block record for `block`, in the drawing's `units` code,
/// and returns its handle, which the block's objects take as their owner.
pub(super) fn add_block_record(
    drawing: &mut CadDocument,
    block: &Block,
    units: i16,
) -> Result<Handle, Error> {
    let mut record = BlockRecord::new(block.name.clone());
    record.set_handle(drawing.allocate_handle());
    record.block_entity_handle = drawing.allocate_handle();
    record.block_end_handle = drawing.allocate_handle();
    record.base_point = vector(block.base_point);
    record.units = units;
    let handle = record.handle;
    drawing.block_records.add(record).map_err(|e| {
        Error::new(
            ErrorCode::DwgWrite,
            format!("could not add block `{}` to the drawing: {e}", block.name),
        )
    })?;
    if block.annotative {
        super::annotation_scale::mark_block_annotative(drawing, handle);
    }
    Ok(handle)
}

/// Adds an external reference block record for `xref`, in the drawing's
/// `units` code, and returns its handle. No objects are owned by an xref
/// block record.
pub(super) fn add_xref_record(
    drawing: &mut CadDocument,
    xref: &Xref,
    units: i16,
) -> Result<Handle, Error> {
    let mut record = BlockRecord::new(xref.name.clone());
    record.set_handle(drawing.allocate_handle());
    record.block_entity_handle = drawing.allocate_handle();
    record.block_end_handle = drawing.allocate_handle();
    record.flags.is_xref = true;
    record.flags.is_xref_overlay = xref.kind == XrefKind::Overlay;
    record.flags.is_xref_unloaded = xref.unloaded;
    record.xref_path = xref.path.clone();
    record.units = units;
    let handle = record.handle;
    drawing.block_records.add(record).map_err(|e| {
        Error::new(
            ErrorCode::DwgWrite,
            format!(
                "could not add external reference `{}` to the drawing: {e}",
                xref.name
            ),
        )
    })?;
    Ok(handle)
}

/// The DWG insert for an external reference placement (carrying no attributes).
pub(super) fn dwg_xref_insert(insert: &Insert) -> DwgInsert {
    DwgInsert::new(insert.block.clone(), vector(insert.position))
        .with_scale(
            if insert.mirrored {
                -insert.scale
            } else {
                insert.scale
            },
            insert.scale,
            insert.scale,
        )
        .with_rotation(insert.rotation.to_radians())
}

/// The DWG attribute definition for one of ours, which starts at `start`:
/// its position when it is left-justified, and otherwise where AutoCAD
/// would start its tag. A multi-line definition (#117) carries an embedded
/// MTEXT payload as AutoCAD R2018+ writes it.
pub(super) fn dwg_attribute_definition(
    definition: &AttributeDefinition,
    start_of: &dyn Fn(&Text) -> Point3,
) -> DwgAttributeDefinition {
    let (_, horizontal, vertical) = alignment(definition.justify);
    if definition.multiline {
        let coded = mtext_code(&definition.default, None);
        let mut dwg =
            DwgAttributeDefinition::new(definition.tag.clone(), definition.prompt.clone(), coded);
        dwg.insertion_point = vector(definition.position);
        dwg.alignment_point = if definition.justify == Justify::Left {
            Vector3::ZERO
        } else {
            vector(definition.position)
        };
        dwg.height = definition.height;
        dwg.rotation = definition.rotation.to_radians();
        dwg.width_factor = definition.width_factor;
        dwg.text_style.clone_from(&definition.style);
        dwg.horizontal_alignment = horizontal;
        dwg.vertical_alignment = vertical;
        dwg.flags = AttributeFlags {
            invisible: definition.invisible,
            constant: definition.constant,
            verify: definition.verify,
            preset: definition.preset,
            ..AttributeFlags::default()
        };
        dwg.lock_position = definition.lock_position;
        dwg.is_multiline = true;
        dwg.mtext_flag = if definition.constant {
            MTextFlag::ConstantMultiLine
        } else {
            MTextFlag::MultiLine
        };
        dwg.line_count = definition.default.split('\n').count() as i16;
        let emb_model = MText {
            contents: definition.default.clone(),
            width_factor: None,
            ..definition_mtext(definition)
        };
        let mut emb = dwg_mtext(&emb_model);
        definition.layer.clone_into(&mut emb.common.layer);
        super::set_properties(&mut emb.common, &definition.properties);
        dwg.embedded_mtext = Some(Box::new(emb));
        definition.layer.clone_into(&mut dwg.common.layer);
        super::set_properties(&mut dwg.common, &definition.properties);
        return dwg;
    }
    let text = definition_text(definition);
    let mut dwg = DwgAttributeDefinition::new(
        definition.tag.clone(),
        definition.prompt.clone(),
        text_code(&definition.default),
    );
    dwg.insertion_point = vector(start_of(&text));
    dwg.alignment_point = alignment_point(&text);
    dwg.height = definition.height;
    dwg.rotation = definition.rotation.to_radians();
    dwg.width_factor = definition.width_factor;
    dwg.text_style.clone_from(&definition.style);
    dwg.horizontal_alignment = horizontal;
    dwg.vertical_alignment = vertical;
    dwg.flags = AttributeFlags {
        invisible: definition.invisible,
        constant: definition.constant,
        verify: definition.verify,
        preset: definition.preset,
        ..AttributeFlags::default()
    };
    // AutoCAD keeps Lock position in a bit of its own, not among the flags.
    dwg.lock_position = definition.lock_position;
    definition.layer.clone_into(&mut dwg.common.layer);
    super::set_properties(&mut dwg.common, &definition.properties);
    dwg
}

/// The DWG insert for one of ours, of `block`, with an attribute for each
/// of the block's attribute definitions but the constant ones, which the
/// block's definition draws, as AutoCAD writes an insert.
pub(super) fn dwg_insert(
    insert: &Insert,
    block: &Block,
    start_of: &dyn Fn(&Text) -> Point3,
) -> Result<DwgInsert, Error> {
    let mut dwg = DwgInsert::new(insert.block.clone(), vector(insert.position))
        .with_scale(
            if insert.mirrored {
                -insert.scale
            } else {
                insert.scale
            },
            insert.scale,
            insert.scale,
        )
        .with_rotation(insert.rotation.to_radians());
    let definitions: Vec<&AttributeDefinition> = block.carried_attributes().collect();
    let tags: Vec<&str> = insert.attributes.iter().map(|a| a.tag.as_str()).collect();
    if tags
        != definitions
            .iter()
            .map(|d| d.tag.as_str())
            .collect::<Vec<_>>()
    {
        return Err(Error::new(
            ErrorCode::DwgWrite,
            format!(
                "an insert of block `{}` carries the attributes {tags:?}, not one for each of the block's attribute definitions in order",
                block.name
            ),
        ));
    }
    for (definition, attribute) in definitions.into_iter().zip(&insert.attributes) {
        if definition.multiline {
            let mtext = attribute_mtext(insert, block, definition, &attribute.value);
            let mtext_justify = attachment_to_justify(mtext.justify);
            let (_, horizontal, vertical) = alignment(mtext_justify);
            let coded = mtext_code(&attribute.value, None);
            let mut dwg_attribute = AttributeEntity::new(attribute.tag.clone(), coded);
            dwg_attribute.insertion_point = vector(mtext.position);
            dwg_attribute.alignment_point = if mtext_justify == Justify::Left {
                Vector3::ZERO
            } else {
                vector(mtext.position)
            };
            dwg_attribute.height = mtext.height;
            dwg_attribute.rotation = mtext.rotation.to_radians();
            dwg_attribute.width_factor = definition.width_factor;
            dwg_attribute.text_style.clone_from(&mtext.style);
            dwg_attribute.horizontal_alignment = horizontal;
            dwg_attribute.vertical_alignment = vertical;
            dwg_attribute.flags = attribute_flags(definition);
            dwg_attribute.lock_position = definition.lock_position;
            dwg_attribute.is_multiline = true;
            // On an ATTRIB, AutoCAD always uses MultiLine (2), never ConstantMultiLine (4).
            dwg_attribute.mtext_flag = MTextFlag::MultiLine;
            dwg_attribute.line_count = attribute.value.split('\n').count() as i16;
            let emb_model = MText {
                width_factor: None,
                ..mtext.clone()
            };
            let mut emb = dwg_mtext(&emb_model);
            emb.common.layer = mtext.layer.clone();
            super::set_properties(&mut emb.common, &mtext.properties);
            dwg_attribute.embedded_mtext = Some(Box::new(emb));
            super::set_properties(&mut dwg_attribute.common, &mtext.properties);
            dwg_attribute.common.layer = mtext.layer;
            dwg.attributes.push(dwg_attribute);
            continue;
        }
        let text = attribute_text(insert, block, definition, &attribute.value);
        let (_, horizontal, vertical) = alignment(text.justify);
        let mut dwg_attribute =
            AttributeEntity::new(attribute.tag.clone(), text_code(&attribute.value));
        dwg_attribute.insertion_point = vector(start_of(&text));
        dwg_attribute.alignment_point = alignment_point(&text);
        dwg_attribute.height = text.height;
        dwg_attribute.rotation = text.rotation.to_radians();
        dwg_attribute.width_factor = text.width_factor;
        dwg_attribute.text_style.clone_from(&text.style);
        dwg_attribute.horizontal_alignment = horizontal;
        dwg_attribute.vertical_alignment = vertical;
        dwg_attribute.flags = attribute_flags(definition);
        dwg_attribute.lock_position = definition.lock_position;
        super::set_properties(&mut dwg_attribute.common, &text.properties);
        dwg_attribute.common.layer = text.layer;
        dwg.attributes.push(dwg_attribute);
    }
    Ok(dwg)
}

fn alignment(justify: Justify) -> (Justify, Horizontal, Vertical) {
    JUSTIFICATIONS
        .into_iter()
        .find(|(j, _, _)| *j == justify)
        .unwrap_or(JUSTIFICATIONS[0])
}

/// The point a justified attribute is aligned at; AutoCAD keeps none for a
/// left-justified one.
fn alignment_point(text: &Text) -> Vector3 {
    if text.justify == Justify::Left {
        Vector3::ZERO
    } else {
        vector(text.position)
    }
}

/// An attribute definition the core can read back exactly: single-line or
/// multi-line (#117), flat, upright and unslanted.
pub(super) fn read_attribute_definition(
    dwg: &DwgAttributeDefinition,
    layer: String,
) -> Option<AttributeDefinition> {
    let base_plain = dwg.normal == Vector3::UNIT_Z
        && dwg.oblique_angle == 0.0
        && dwg.thickness == 0.0
        && dwg.text_generation_flags == 0
        && plain_flags(dwg.flags);
    if !base_plain {
        return None;
    }
    if single_line(
        dwg.mtext_flag,
        dwg.is_multiline,
        dwg.embedded_mtext.is_some(),
    ) {
        let justify = justification(dwg.horizontal_alignment, dwg.vertical_alignment)?;
        let position = if justify == Justify::Left {
            dwg.insertion_point
        } else {
            dwg.alignment_point
        };
        return Some(AttributeDefinition {
            tag: dwg.tag.clone(),
            prompt: dwg.prompt.clone(),
            default: text_contents(&dwg.default_value)?,
            position: point(position),
            height: dwg.height,
            style: style_name(&dwg.text_style),
            width_factor: dwg.width_factor,
            rotation: normalize_degrees(dwg.rotation.to_degrees()),
            justify,
            invisible: dwg.flags.invisible,
            constant: dwg.flags.constant,
            verify: dwg.flags.verify,
            preset: dwg.flags.preset,
            lock_position: dwg.lock_position,
            multiline: false,
            width: 0.0,
            layer,
            properties: Default::default(),
        });
    }
    // Multi-line attribute definition (#117).
    if let Some(emb) = dwg.embedded_mtext.as_deref() {
        if !plain_embedded_mtext(emb) {
            return None;
        }
        let (default, wf, u, o) = mtext_styled_contents(&emb.value)?;
        if u || o {
            return None;
        }
        let justify = attachment_to_justify(attachment_of(emb.attachment_point)?);
        let height = if emb.height > 0.0 {
            emb.height
        } else {
            dwg.height
        };
        let style = if !emb.style.is_empty() {
            style_name(&emb.style)
        } else {
            style_name(&dwg.text_style)
        };
        return Some(AttributeDefinition {
            tag: dwg.tag.clone(),
            prompt: dwg.prompt.clone(),
            default,
            position: point(emb.insertion_point),
            height,
            style,
            width_factor: wf.unwrap_or(dwg.width_factor),
            rotation: normalize_degrees(emb.rotation.to_degrees()),
            justify,
            invisible: dwg.flags.invisible,
            constant: dwg.flags.constant,
            verify: dwg.flags.verify,
            preset: dwg.flags.preset,
            lock_position: dwg.lock_position,
            multiline: true,
            width: emb.rectangle_width,
            layer,
            properties: Default::default(),
        });
    }
    // Multi-line without an embedded MTEXT payload (e.g. older DWG/DXF).
    let (default, wf, u, o) = mtext_styled_contents(&dwg.default_value)?;
    if u || o {
        return None;
    }
    let raw_justify = justification(dwg.horizontal_alignment, dwg.vertical_alignment)?;
    let position = if raw_justify == Justify::Left {
        dwg.insertion_point
    } else {
        dwg.alignment_point
    };
    Some(AttributeDefinition {
        tag: dwg.tag.clone(),
        prompt: dwg.prompt.clone(),
        default,
        position: point(position),
        height: dwg.height,
        style: style_name(&dwg.text_style),
        width_factor: wf.unwrap_or(dwg.width_factor),
        rotation: normalize_degrees(dwg.rotation.to_degrees()),
        justify: attachment_to_justify(justify_to_attachment(raw_justify)),
        invisible: dwg.flags.invisible,
        constant: dwg.flags.constant,
        verify: dwg.flags.verify,
        preset: dwg.flags.preset,
        lock_position: dwg.lock_position,
        multiline: true,
        width: 0.0,
        layer,
        properties: Default::default(),
    })
}

/// An insert the core can read back exactly: of a block it read, flat, not
/// an array, scaled the same along every axis, or mirrored as AutoCAD's
/// MIRROR leaves one, its X scale the negative of the others (#242), with
/// one attribute for each of the block's attribute definitions but the
/// constant ones, each where the block puts it. One not mirrored whose
/// attributes read forwards where its turn alone would not leave them so
/// reads back with [`Insert::readable_attributes`].
#[cfg(test)]
pub(super) fn read_insert(dwg: &DwgInsert, layer: String, blocks: &[Block]) -> Option<Insert> {
    read_insert_in(None, dwg, layer, blocks, &[])
}

/// Reads an insert, resolving dynamic block effective names and saved parameter
/// states when `drawing` is provided, and accepting inserts of declared
/// external references in `xrefs`.
pub(super) fn read_insert_in(
    drawing: Option<&CadDocument>,
    dwg: &DwgInsert,
    layer: String,
    blocks: &[Block],
    xrefs: &[Xref],
) -> Option<Insert> {
    let scale = dwg.y_scale();
    let mirrored = dwg.x_scale() == -scale;
    let plain = dwg.normal == Vector3::UNIT_Z
        && !dwg.is_array()
        && !dwg.is_minsert()
        && dwg.view_rep_handle.is_none()
        && scale > 0.0
        && (dwg.x_scale() == scale || mirrored)
        && dwg.z_scale() == scale;
    if !plain {
        return None;
    }
    let Some(block) = blocks.iter().find(|b| b.name == dwg.block_name) else {
        let xref = xrefs
            .iter()
            .find(|x| x.name == dwg.block_name || x.name.eq_ignore_ascii_case(&dwg.block_name))?;
        if !dwg.attributes.is_empty() {
            return None;
        }
        return Some(Insert {
            block: xref.name.clone(),
            position: point(dwg.insert_point),
            scale,
            rotation: normalize_degrees(dwg.rotation.to_degrees()),
            mirrored,
            readable_attributes: false,
            attributes: Vec::new(),
            state: std::collections::BTreeMap::new(),
            evaluated_block: None,
            name: super::ids::read_name(&dwg.common),
            layer,
            properties: Default::default(),
            annotative: false,
            annotation_scales: Vec::new(),
        });
    };
    let (effective_block, state, evaluated_block) = drawing
        .and_then(|d| super::dynamic_block::read_insert_dynamic_info(d, dwg))
        .unwrap_or_else(|| (block.name.clone(), std::collections::BTreeMap::new(), None));
    let definitions: Vec<&AttributeDefinition> = block.carried_attributes().collect();
    if definitions.len() != dwg.attributes.len() {
        return None;
    }
    let insert = Insert {
        block: effective_block,
        position: point(dwg.insert_point),
        scale,
        rotation: normalize_degrees(dwg.rotation.to_degrees()),
        mirrored,
        readable_attributes: false,
        attributes: Vec::with_capacity(definitions.len()),
        state,
        evaluated_block,
        name: super::ids::read_name(&dwg.common),
        annotative: false,
        annotation_scales: Vec::new(),
        layer,
        properties: Default::default(),
    };
    // An insert that is not mirrored turns its attributes with it, or, one
    // mirrored back, keeps them reading forwards.
    let readings: &[bool] = if mirrored { &[false] } else { &[false, true] };
    readings.iter().find_map(|&readable_attributes| {
        let mut insert = Insert {
            readable_attributes,
            ..insert.clone()
        };
        for (definition, dwg_attribute) in definitions.iter().zip(&dwg.attributes) {
            let value = read_attribute(dwg_attribute, &insert, block, definition)?;
            insert.attributes.push(Attribute {
                tag: definition.tag.clone(),
                value,
                field: None,
            });
        }
        Some(insert)
    })
}

/// The value of an attribute on `insert`, if it is the attribute
/// `definition` puts there: the same tag, mode, style and justification,
/// drawn where the definition is, moved, scaled and turned with the insert.
fn read_attribute(
    dwg: &AttributeEntity,
    insert: &Insert,
    block: &Block,
    definition: &AttributeDefinition,
) -> Option<String> {
    let layer = if dwg.common.layer.is_empty() {
        LAYER_ZERO
    } else {
        dwg.common.layer.as_str()
    };
    let common_same = !dwg.common.invisible
        && dwg.tag == definition.tag
        && dwg.normal == Vector3::UNIT_Z
        && dwg.oblique_angle == 0.0
        && dwg.thickness == 0.0
        && dwg.text_generation_flags == 0
        && dwg.flags == attribute_flags(definition)
        && dwg.lock_position == definition.lock_position;
    if !common_same {
        return None;
    }
    if definition.multiline {
        let is_ml = dwg.is_multiline
            || dwg.mtext_flag != MTextFlag::SingleLine
            || dwg.embedded_mtext.is_some();
        if !is_ml {
            return None;
        }
        if let Some(emb) = dwg.embedded_mtext.as_deref() {
            if !plain_embedded_mtext(emb) {
                return None;
            }
            let (value, wf, u, o) = mtext_styled_contents(&emb.value)?;
            if u || o {
                return None;
            }
            let expected = attribute_mtext(insert, block, definition, &value);
            let emb_style = if !emb.style.is_empty() {
                style_name(&emb.style)
            } else {
                style_name(&dwg.text_style)
            };
            let emb_height = if emb.height > 0.0 {
                emb.height
            } else {
                dwg.height
            };
            let same = super::read_properties(&dwg.common).as_ref() == Some(&expected.properties)
                && layer == expected.layer
                && attachment_of(emb.attachment_point) == Some(expected.justify)
                && emb_style == expected.style
                && wf.unwrap_or(dwg.width_factor) == definition.width_factor
                && close(emb_height, expected.height, expected.height)
                && close(
                    emb.rectangle_width,
                    expected.width,
                    expected.width.max(expected.height),
                )
                && same_angle(emb.rotation.to_degrees(), expected.rotation)
                && same_point(point(emb.insertion_point), expected.position);
            return same.then_some(value);
        }
        let (value, wf, u, o) = mtext_styled_contents(&dwg.value)?;
        if u || o {
            return None;
        }
        let expected = attribute_mtext(insert, block, definition, &value);
        let raw_justify = justification(dwg.horizontal_alignment, dwg.vertical_alignment)?;
        let position = if raw_justify == Justify::Left {
            dwg.insertion_point
        } else {
            dwg.alignment_point
        };
        let same = super::read_properties(&dwg.common).as_ref() == Some(&expected.properties)
            && layer == expected.layer
            && justify_to_attachment(raw_justify) == expected.justify
            && style_name(&dwg.text_style) == expected.style
            && wf.unwrap_or(dwg.width_factor) == definition.width_factor
            && close(dwg.height, expected.height, expected.height)
            && close(0.0, expected.width, expected.height)
            && same_angle(dwg.rotation.to_degrees(), expected.rotation)
            && same_point(point(position), expected.position);
        return same.then_some(value);
    }
    let value = text_contents(&dwg.value)?;
    let expected = attribute_text(insert, block, definition, &value);
    let position = if expected.justify == Justify::Left {
        dwg.insertion_point
    } else {
        dwg.alignment_point
    };
    let same = super::read_properties(&dwg.common).as_ref() == Some(&expected.properties)
        && single_line(
            dwg.mtext_flag,
            dwg.is_multiline,
            dwg.embedded_mtext.is_some(),
        )
        && justification(dwg.horizontal_alignment, dwg.vertical_alignment)
            == Some(expected.justify)
        && style_name(&dwg.text_style) == expected.style
        && layer == expected.layer
        && dwg.width_factor == expected.width_factor
        && close(dwg.height, expected.height, expected.height)
        && same_angle(dwg.rotation.to_degrees(), expected.rotation)
        && same_point(point(position), expected.position);
    same.then_some(value)
}

/// Whether an embedded MTEXT payload on a multi-line attribute has only the
/// layout properties the core models.
fn plain_embedded_mtext(emb: &DwgMText) -> bool {
    emb.normal == Vector3::UNIT_Z
        && emb.line_spacing_style == LineSpacingStyle::AtLeast
        && close(emb.line_spacing_factor, 1.0, 1.0)
        && matches!(
            emb.drawing_direction,
            DrawingDirection::LeftToRight | DrawingDirection::ByStyle
        )
        && emb.background_fill_flags == 0
        && emb.rectangle_width >= 0.0
}

/// Whether an attribute definition's modes are those the core models:
/// Invisible, Constant, Verify and Preset, with Lock position in a bit of
/// its own. Annotative attributes, and the Lock position flag AutoCAD keeps
/// in its own bit rather than here, are not read.
fn plain_flags(flags: AttributeFlags) -> bool {
    !flags.annotative && !flags.locked_position
}

/// The modes an insert's attribute carries: its definition's Invisible,
/// Verify and Preset, as AutoCAD gives an inserted attribute them.
fn attribute_flags(definition: &AttributeDefinition) -> AttributeFlags {
    AttributeFlags {
        invisible: definition.invisible,
        verify: definition.verify,
        preset: definition.preset,
        ..AttributeFlags::default()
    }
}

fn single_line(flag: MTextFlag, multiline: bool, embedded: bool) -> bool {
    flag == MTextFlag::SingleLine && !multiline && !embedded
}

fn justification(horizontal: Horizontal, vertical: Vertical) -> Option<Justify> {
    JUSTIFICATIONS
        .into_iter()
        .find(|(_, h, v)| *h == horizontal && *v == vertical)
        .map(|(j, _, _)| j)
}

/// Whether `a` and `b` agree to [`TOLERANCE`] of `size`, or of 1 if that is more.
fn close(a: f64, b: f64, size: f64) -> bool {
    (a - b).abs() <= TOLERANCE * size.abs().max(1.0)
}

fn same_point(a: Point3, b: Point3) -> bool {
    let size = b.x.abs().max(b.y.abs()).max(b.z.abs());
    close(a.x, b.x, size) && close(a.y, b.y, size) && close(a.z, b.z, size)
}

/// Whether two angles in degrees agree, counting whole turns as none.
fn same_angle(a: f64, b: f64) -> bool {
    let turned = normalize_degrees(a - b);
    close(turned.min(360.0 - turned), 0.0, 360.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::block::place;
    use crate::text::STANDARD;

    fn definition(tag: &str, justify: Justify) -> AttributeDefinition {
        AttributeDefinition {
            tag: tag.into(),
            prompt: "Tag".into(),
            default: "?".into(),
            position: Point3::new(0.0, 6.0, 0.0),
            height: 2.5,
            style: STANDARD.into(),
            width_factor: 1.0,
            rotation: 0.0,
            justify,
            invisible: false,
            layer: LAYER_ZERO.into(),
            properties: Default::default(),
            constant: false,
            lock_position: false,
            multiline: false,
            width: 0.0,
            preset: false,
            verify: false,
        }
    }

    fn block() -> Block {
        Block {
            name: "MARK".into(),
            base_point: Point3::new(0.0, 0.0, 0.0),
            objects: vec![
                crate::model::Object::AttributeDefinition(definition("TAG", Justify::MiddleCenter)),
                crate::model::Object::AttributeDefinition(definition("NOTE", Justify::Left)),
            ],
            cells: Vec::new(),
            parameters: Vec::new(),
            annotative: false,
        }
    }

    fn insert() -> Insert {
        Insert {
            block: "MARK".into(),
            position: Point3::new(40.0, 30.0, 0.0),
            scale: 2.0,
            rotation: 33.3,
            mirrored: false,
            readable_attributes: false,
            attributes: vec![
                Attribute {
                    tag: "TAG".into(),
                    value: "A-1".into(),
                    field: None,
                },
                Attribute {
                    tag: "NOTE".into(),
                    value: String::new(),
                    field: None,
                },
            ],
            state: std::collections::BTreeMap::new(),
            evaluated_block: None,
            name: Some("mark-1".into()),
            annotative: false,
            annotation_scales: Vec::new(),
            layer: "symbols".into(),
            properties: Default::default(),
        }
    }

    #[test]
    fn an_insert_reads_back_with_its_attributes_and_name() {
        let block = block();
        let insert = insert();
        let mut written = dwg_insert(&insert, &block, &|t| t.position).unwrap();
        // A build writes every object's ID and name, an insert's among them.
        super::super::ids::write_id_and_name(
            &mut written.common,
            "S",
            "objects[0]",
            Some("mark-1"),
        );
        assert_eq!(written.attributes.len(), 2);
        let justified = &written.attributes[0];
        assert_eq!(
            point(justified.alignment_point),
            place(&insert, block.base_point, Point3::new(0.0, 6.0, 0.0))
        );
        assert_eq!(justified.common.layer, "symbols");
        assert_eq!(written.attributes[1].alignment_point, Vector3::ZERO);
        let read = read_insert(&written, "symbols".into(), &[block]).unwrap();
        assert!(same_angle(read.rotation, insert.rotation), "{read:?}");
        assert_eq!(
            Insert {
                rotation: insert.rotation,
                ..read
            },
            insert
        );
    }

    #[test]
    fn a_mirrored_insert_is_written_with_a_negative_x_scale_and_reads_back() {
        let block = block();
        let mut insert = insert();
        insert.mirrored = true;
        insert.rotation = 180.0;
        let written = dwg_insert(&insert, &block, &|t| t.position).unwrap();
        assert_eq!(
            (written.x_scale(), written.y_scale(), written.z_scale()),
            (-2.0, 2.0, 2.0)
        );
        // Its attributes read forwards: upright, though the insert is
        // mirrored and turned a half turn.
        assert_eq!(written.attributes[0].rotation, 0.0);
        let read = read_insert(&written, "symbols".into(), std::slice::from_ref(&block)).unwrap();
        assert!(read.mirrored);
        assert_eq!(read.scale, 2.0);
        assert!(same_angle(read.rotation, 180.0), "{read:?}");
        // Any other negative scale is not one the core holds.
        let mut stretched = written.clone();
        stretched.set_scale_unchecked(2.0, -2.0, 2.0);
        assert!(read_insert(&stretched, "symbols".into(), &[block]).is_none());
    }

    #[test]
    fn a_mirrored_insert_whose_attribute_reads_upright_reads_back_through_its_own_dwg() {
        // Turned 30 degrees with its attribute defined at 120 (inverted
        // half-plane), the raw reflection is 90 (on the cutoff), so AutoCAD's
        // MIRRTEXT 0 flips it to 270. Read back, 120 is 119.99999999999999,
        // which must stay on the same side of the cutoff so it reads back.
        let mut ours = definition("TAG", Justify::MiddleCenter);
        ours.rotation = 120.0;
        let block = Block {
            name: "MARK".into(),
            base_point: Point3::new(0.0, 0.0, 0.0),
            objects: vec![crate::model::Object::AttributeDefinition(ours.clone())],
            cells: Vec::new(),
            parameters: Vec::new(),
            annotative: false,
        };
        let written = dwg_attribute_definition(&ours, &|t| t.position);
        let mut read_block = block.clone();
        read_block.objects = vec![crate::model::Object::AttributeDefinition(
            read_attribute_definition(&written, LAYER_ZERO.into()).unwrap(),
        )];
        let mut insert = insert();
        insert.attributes.truncate(1);
        insert.rotation = 30.0;
        insert.mirrored = true;
        let dwg = dwg_insert(&insert, &block, &|t| t.position).unwrap();
        assert!(same_angle(dwg.attributes[0].rotation.to_degrees(), 270.0));
        let read = read_insert(&dwg, "symbols".into(), &[read_block]).expect("it reads back");
        assert!(read.mirrored);
        assert_eq!(read.attributes, insert.attributes);
        // And so at the cutoff's other side, where it reads straight down
        // and is turned to read up.
        for rotation in [210.0, 300.0, 120.0] {
            insert.rotation = rotation;
            let dwg = dwg_insert(&insert, &block, &|t| t.position).unwrap();
            let read_block = Block {
                objects: vec![crate::model::Object::AttributeDefinition(
                    read_attribute_definition(&written, LAYER_ZERO.into()).unwrap(),
                )],
                ..block.clone()
            };
            assert!(
                read_insert(&dwg, "symbols".into(), &[read_block]).is_some(),
                "turned {rotation}"
            );
        }
    }

    #[test]
    fn an_insert_mirrored_back_reads_back_with_its_attributes_reading_forwards() {
        let block = block();
        let mut insert = insert();
        insert.rotation = 240.0;
        insert.readable_attributes = true;
        let written = dwg_insert(&insert, &block, &|t| t.position).unwrap();
        // Its attributes read at 60, not at the 240 it is turned.
        assert!(same_angle(
            written.attributes[0].rotation.to_degrees(),
            60.0
        ));
        let read = read_insert(&written, "symbols".into(), std::slice::from_ref(&block)).unwrap();
        assert!(read.readable_attributes && !read.mirrored, "{read:?}");
        assert_eq!(read.attributes, insert.attributes);
        // Turned so that they read forwards anyway, it reads back as an
        // ordinary insert, which draws the same.
        insert.rotation = 30.0;
        let written = dwg_insert(&insert, &block, &|t| t.position).unwrap();
        let read = read_insert(&written, "symbols".into(), &[block]).unwrap();
        assert!(!read.readable_attributes);
    }

    #[test]
    fn an_insert_whose_attribute_was_moved_is_not_read() {
        let block = block();
        let mut written = dwg_insert(&insert(), &block, &|t| t.position).unwrap();
        written.attributes[0].alignment_point.x += 0.5;
        assert!(read_insert(&written, "symbols".into(), &[block]).is_none());
    }

    #[test]
    fn an_insert_must_carry_one_attribute_for_each_definition() {
        let mut insert = insert();
        insert.attributes.pop();
        let error = dwg_insert(&insert, &block(), &|t| t.position).unwrap_err();
        assert_eq!(error.code, ErrorCode::DwgWrite);
    }

    #[test]
    fn an_attribute_definition_reads_back() {
        for justify in [Justify::Left, Justify::BottomRight] {
            let mut ours = definition("TAG", justify);
            ours.invisible = true;
            ours.rotation = 90.0;
            let written = dwg_attribute_definition(&ours, &|t| t.position);
            let read = read_attribute_definition(&written, LAYER_ZERO.into()).unwrap();
            assert!(same_angle(read.rotation, 90.0));
            assert_eq!(
                AttributeDefinition {
                    rotation: 90.0,
                    ..read
                },
                ours
            );
        }
    }

    #[test]
    fn a_multiline_attribute_definition_and_insert_read_back() {
        let mut def = definition("NOTES", Justify::TopLeft);
        def.multiline = true;
        def.width = 40.0;
        def.default = "LINE ONE\nLINE TWO".into();
        let written_def = dwg_attribute_definition(&def, &|t| t.position);
        assert!(written_def.is_multiline);
        assert_eq!(written_def.mtext_flag, MTextFlag::MultiLine);
        assert!(written_def.embedded_mtext.is_some());
        let read_def = read_attribute_definition(&written_def, LAYER_ZERO.into()).unwrap();
        assert_eq!(read_def, def);

        let blk = Block {
            name: "NOTEBLK".into(),
            base_point: Point3::new(0.0, 0.0, 0.0),
            objects: vec![crate::model::Object::AttributeDefinition(def)],
            cells: Vec::new(),
            parameters: Vec::new(),
            annotative: false,
        };
        let ins = Insert {
            block: "NOTEBLK".into(),
            position: Point3::new(10.0, 20.0, 0.0),
            scale: 2.0,
            rotation: 0.0,
            mirrored: false,
            readable_attributes: false,
            attributes: vec![Attribute {
                tag: "NOTES".into(),
                value: "FIRST\nSECOND\nTHIRD".into(),
                field: None,
            }],
            state: std::collections::BTreeMap::new(),
            evaluated_block: None,
            name: None,
            annotative: false,
            annotation_scales: Vec::new(),
            layer: "0".into(),
            properties: Default::default(),
        };
        let written_ins = dwg_insert(&ins, &blk, &|t| t.position).unwrap();
        assert!(written_ins.attributes[0].is_multiline);
        assert_eq!(written_ins.attributes[0].mtext_flag, MTextFlag::MultiLine);
        let emb = written_ins.attributes[0].embedded_mtext.as_ref().unwrap();
        assert_eq!(emb.rectangle_width, 80.0); // 40.0 * scale 2.0
        assert_eq!(emb.value, "FIRST\\PSECOND\\PTHIRD");
        let read_ins = read_insert(&written_ins, "0".into(), &[blk]).unwrap();
        assert_eq!(read_ins.attributes[0].value, "FIRST\nSECOND\nTHIRD");
    }

    #[test]
    fn every_justification_has_one_autocad_alignment() {
        for justify in Justify::ALL {
            let found: Vec<_> = JUSTIFICATIONS
                .iter()
                .filter(|(j, _, _)| *j == justify)
                .collect();
            assert_eq!(found.len(), 1);
            let (_, h, v) = found[0];
            assert_eq!(justification(*h, *v), Some(justify));
        }
    }

    /// `CONTEXT.md` says drawing and object. acadrust's own names, which
    /// say document and entity, are allowed; ours are not.
    #[test]
    fn our_names_here_say_drawing_and_object() {
        let source = include_str!("block.rs");
        let (code, _tests) = source.split_once("#[cfg(test)]").unwrap();
        let mut ours = code.to_owned();
        for acadrust_name in [
            "CadDocument",
            "AttributeEntity",
            "acadrust::entities",
            "block_entity_handle",
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
