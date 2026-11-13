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
    HorizontalAlignment as Horizontal, Insert as DwgInsert, MTextFlag,
    VerticalAlignment as Vertical,
};
use acadrust::tables::{AppId, BlockRecord, TableEntry};
use acadrust::types::{Handle, Vector3};
use acadrust::CadDocument;

use super::text::{style_name, text_code, text_contents};
use super::{point, vector};
use crate::description::normalize_degrees;
use crate::error::{Error, ErrorCode};
use crate::model::block::definition_text;
use crate::model::{
    attribute_text, Attribute, AttributeDefinition, Block, Insert, Point3, LAYER_ZERO,
};
use crate::text::{Justify, Text};

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
    if drawing.app_ids.get(APPLICATION).is_some() {
        return Ok(());
    }
    drawing.app_ids.add(AppId::new(APPLICATION)).map_err(|e| {
        Error::new(
            ErrorCode::DwgWrite,
            format!("could not register the application `{APPLICATION}` in the drawing: {e}"),
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
    Ok(handle)
}

/// The DWG attribute definition for one of ours, which starts at `start`:
/// its position when it is left-justified, and otherwise where AutoCAD
/// would start its tag.
pub(super) fn dwg_attribute_definition(
    definition: &AttributeDefinition,
    start_of: &dyn Fn(&Text) -> Point3,
) -> DwgAttributeDefinition {
    let text = definition_text(definition);
    let (_, horizontal, vertical) = alignment(definition.justify);
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
        ..AttributeFlags::default()
    };
    definition.layer.clone_into(&mut dwg.common.layer);
    super::set_properties(&mut dwg.common, &definition.properties);
    dwg
}

/// The DWG insert for one of ours, of `block`, with an attribute for each
/// of the block's attribute definitions.
pub(super) fn dwg_insert(
    insert: &Insert,
    block: &Block,
    start_of: &dyn Fn(&Text) -> Point3,
) -> Result<DwgInsert, Error> {
    let mut dwg = DwgInsert::new(insert.block.clone(), vector(insert.position))
        .with_uniform_scale(insert.scale)
        .with_rotation(insert.rotation.to_radians());
    let definitions: Vec<&AttributeDefinition> = block.attribute_definitions().collect();
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
        let text = attribute_text(insert, block, definition, &attribute.value);
        let (_, horizontal, vertical) = alignment(definition.justify);
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
        dwg_attribute.flags = AttributeFlags {
            invisible: definition.invisible,
            ..AttributeFlags::default()
        };
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

/// An attribute definition the core can read back exactly: a single line
/// of text, flat, upright and unslanted, at most invisible.
pub(super) fn read_attribute_definition(
    dwg: &DwgAttributeDefinition,
    layer: String,
) -> Option<AttributeDefinition> {
    let plain = dwg.normal == Vector3::UNIT_Z
        && dwg.oblique_angle == 0.0
        && dwg.thickness == 0.0
        && dwg.text_generation_flags == 0
        && plain_flags(dwg.flags)
        && single_line(
            dwg.mtext_flag,
            dwg.is_multiline,
            dwg.embedded_mtext.is_some(),
        );
    if !plain {
        return None;
    }
    let justify = justification(dwg.horizontal_alignment, dwg.vertical_alignment)?;
    let position = if justify == Justify::Left {
        dwg.insertion_point
    } else {
        dwg.alignment_point
    };
    Some(AttributeDefinition {
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
        layer,
        properties: Default::default(),
    })
}

/// An insert the core can read back exactly: of a block it read, flat, not
/// an array, scaled the same along every axis, with one attribute for each
/// of the block's attribute definitions, each where the block puts it.
pub(super) fn read_insert(dwg: &DwgInsert, layer: String, blocks: &[Block]) -> Option<Insert> {
    let scale = dwg.x_scale();
    let plain = dwg.normal == Vector3::UNIT_Z
        && !dwg.is_array()
        && !dwg.is_minsert()
        && dwg.view_rep_handle.is_none()
        && scale > 0.0
        && dwg.y_scale() == scale
        && dwg.z_scale() == scale;
    if !plain {
        return None;
    }
    let block = blocks.iter().find(|b| b.name == dwg.block_name)?;
    let definitions: Vec<&AttributeDefinition> = block.attribute_definitions().collect();
    if definitions.len() != dwg.attributes.len() {
        return None;
    }
    let mut insert = Insert {
        block: block.name.clone(),
        position: point(dwg.insert_point),
        scale,
        rotation: normalize_degrees(dwg.rotation.to_degrees()),
        attributes: Vec::with_capacity(definitions.len()),
        name: super::ids::read_name(&dwg.common),
        layer,
        properties: Default::default(),
    };
    for (definition, dwg_attribute) in definitions.into_iter().zip(&dwg.attributes) {
        let value = read_attribute(dwg_attribute, &insert, block, definition)?;
        insert.attributes.push(Attribute {
            tag: definition.tag.clone(),
            value,
        });
    }
    Some(insert)
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
    let value = text_contents(&dwg.value)?;
    let expected = attribute_text(insert, block, definition, &value);
    let layer = if dwg.common.layer.is_empty() {
        LAYER_ZERO
    } else {
        dwg.common.layer.as_str()
    };
    let position = if expected.justify == Justify::Left {
        dwg.insertion_point
    } else {
        dwg.alignment_point
    };
    let same = !dwg.common.invisible
        && super::read_properties(&dwg.common).as_ref() == Some(&expected.properties)
        && dwg.tag == definition.tag
        && dwg.normal == Vector3::UNIT_Z
        && dwg.oblique_angle == 0.0
        && dwg.thickness == 0.0
        && dwg.text_generation_flags == 0
        && plain_flags(dwg.flags)
        && dwg.flags.invisible == definition.invisible
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

/// Whether an attribute's modes are those the core models: none, or
/// Invisible. Constant, Verify and Preset attributes are not read.
fn plain_flags(flags: AttributeFlags) -> bool {
    !flags.constant && !flags.verify && !flags.preset && !flags.annotative
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
        }
    }

    fn insert() -> Insert {
        Insert {
            block: "MARK".into(),
            position: Point3::new(40.0, 30.0, 0.0),
            scale: 2.0,
            rotation: 33.3,
            attributes: vec![
                Attribute {
                    tag: "TAG".into(),
                    value: "A-1".into(),
                },
                Attribute {
                    tag: "NOTE".into(),
                    value: String::new(),
                },
            ],
            name: Some("mark-1".into()),
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
