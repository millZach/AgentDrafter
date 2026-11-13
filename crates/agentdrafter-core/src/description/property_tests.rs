//! An object's own colour, linetype, lineweight and linetype scale, and a
//! sheet's `ltscale`, in a sheet description.

use super::{parse_block_library, parse_sheet_description};
use crate::error::{Error, ErrorCode};
use crate::model::{
    Lineweight, Object, ObjectColor, ObjectLinetype, ObjectLineweight, Properties, Sheet,
};

fn sheet(text: &str) -> Sheet {
    parse_sheet_description(text).unwrap_or_else(|e| panic!("{e}"))
}

/// A sheet in mm with a linetype `DASHED` and a layer `outline`, holding
/// one line whose own keys are `keys`.
fn line_with(keys: &str) -> String {
    format!(
        "units: mm\nlinetypes:\n  DASHED: {{pattern: [5, -2.5]}}\nlayers:\n  outline: {{color: red}}\n\
         objects:\n  - line: {{from: [0, 0, 0], to: [1, 0, 0], layer: outline{keys}}}\n"
    )
}

fn properties_of(keys: &str) -> Properties {
    sheet(&line_with(keys)).objects[0].properties().clone()
}

fn error(text: &str) -> Error {
    match parse_sheet_description(text) {
        Ok(sheet) => panic!("expected an error, read {sheet:?}"),
        Err(error) => error,
    }
}

#[track_caller]
fn assert_names(e: &Error, code: ErrorCode, field: &str, words: &[&str]) {
    assert_eq!(e.code, code, "code of {e:?}");
    assert_eq!(e.field.as_deref(), Some(field), "field of {e:?}");
    for word in words {
        assert!(e.message.contains(word), "`{word}` in {:?}", e.message);
    }
}

#[test]
fn an_object_without_properties_of_its_own_takes_its_layers() {
    assert!(properties_of("").is_by_layer());
    assert_eq!(sheet(&line_with("")).ltscale, 1.0);
}

#[test]
fn an_object_takes_its_own_colour_linetype_lineweight_and_linetype_scale() {
    assert_eq!(
        properties_of(", color: 3, linetype: DASHED, lineweight: 0.35 mm, linetype_scale: 0.5"),
        Properties {
            color: ObjectColor::Index(3),
            linetype: ObjectLinetype::Named("DASHED".to_owned()),
            lineweight: ObjectLineweight::Width(Lineweight::from_millimetres(0.35).unwrap()),
            linetype_scale: 0.5,
        }
    );
    assert_eq!(
        properties_of(", color: magenta").color,
        ObjectColor::Index(6)
    );
    assert_eq!(
        properties_of(", color: '250'").color,
        ObjectColor::Index(250)
    );
    let by_block = properties_of(", color: ByBlock, linetype: ByBlock, lineweight: ByBlock");
    assert_eq!(by_block.color, ObjectColor::ByBlock);
    assert_eq!(by_block.linetype, ObjectLinetype::ByBlock);
    assert_eq!(by_block.lineweight, ObjectLineweight::ByBlock);
    let by_layer = properties_of(", color: ByLayer, linetype: ByLayer, lineweight: ByLayer");
    assert!(by_layer.is_by_layer());
    assert_eq!(
        properties_of(", linetype: Continuous").linetype,
        ObjectLinetype::Named("Continuous".to_owned())
    );
    assert_eq!(
        properties_of(", lineweight: Default").lineweight,
        ObjectLineweight::Default
    );
    // A bare null leaves the linetype out, as on a layer.
    assert!(properties_of(", linetype: null").is_by_layer());
}

#[test]
fn every_object_type_takes_properties_of_its_own() {
    let sheet = sheet(
        "units: mm\nobjects:\n\
         - polyline: {points: [[0, 0, 0], [1, 0, 0]], color: 1}\n\
         - arc: {center: [0, 0, 0], radius: 1, start_angle: 0, end_angle: 90, color: 2}\n\
         - circle: {center: [0, 0, 0], radius: 1, color: 3}\n\
         - ellipse: {center: [0, 0, 0], major_axis: [2, 0, 0], ratio: 0.5, color: 4}\n\
         - spline: {fit_points: [[0, 0, 0], [1, 1, 0], [2, 0, 0]], color: 5}\n\
         - hatch: {pattern: SOLID, boundary: [{polyline: {points: [[0, 0, 0], [1, 0, 0], [1, 1, 0]]}}], color: 6}\n\
         - text: {contents: A, position: [0, 0, 0], height: 1, color: 7}\n\
         - mtext: {contents: B, position: [0, 0, 0], height: 1, color: 8}\n",
    );
    let colors: Vec<ObjectColor> = sheet.objects.iter().map(|o| o.properties().color).collect();
    assert_eq!(colors, (1..=8).map(ObjectColor::Index).collect::<Vec<_>>());
}

#[test]
fn a_sheet_takes_its_ltscale() {
    let text = line_with("").replace("units: mm\n", "units: mm\nltscale: 2.5\n");
    assert_eq!(sheet(&text).ltscale, 2.5);
}

#[test]
fn a_wrong_property_is_refused_with_the_choices() {
    for (keys, code, field, words) in [
        (
            ", color: 0",
            ErrorCode::BadValue,
            "objects[0].line.color",
            &["ByLayer", "ByBlock", "1 to 255"][..],
        ),
        (
            ", color: purple",
            ErrorCode::BadValue,
            "objects[0].line.color",
            &["ByLayer", "red"],
        ),
        (
            ", color: bylayer",
            ErrorCode::BadValue,
            "objects[0].line.color",
            &["ByLayer"],
        ),
        (
            ", linetype: HIDDEN",
            ErrorCode::UnknownName,
            "objects[0].line.linetype",
            &["ByLayer, ByBlock, Continuous, DASHED"],
        ),
        (
            ", lineweight: 0.33",
            ErrorCode::BadValue,
            "objects[0].line.lineweight",
            &["0.35", "`ByLayer`, `ByBlock` or `Default`"],
        ),
        (
            ", linetype_scale: 0",
            ErrorCode::BadValue,
            "objects[0].line.linetype_scale",
            &["more than 0"],
        ),
        (
            ", linetype_scale: 2 mm",
            ErrorCode::BadNumber,
            "objects[0].line.linetype_scale",
            &["no unit"],
        ),
    ] {
        assert_names(&error(&line_with(keys)), code, field, words);
    }
    let e = error(&line_with("").replace("units: mm\n", "units: mm\nltscale: -1\n"));
    assert_names(&e, ErrorCode::BadValue, "ltscale", &["more than 0"]);
}

#[test]
fn a_blocks_objects_take_properties_of_their_own() {
    let library = parse_block_library(
        "units: mm\nlinetypes:\n  DOTS: {pattern: [0, -1]}\n  UNUSED: {pattern: [1, -1]}\nblocks:\n  B:\n    objects:\n\
         \x20     - line: {from: [0, 0, 0], to: [1, 0, 0], color: ByBlock, linetype: DOTS, lineweight: ByBlock}\n\
         \x20     - attdef: {tag: TAG, position: [0, 0, 0], height: 1, color: ByBlock}\n",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let block = &library.blocks[0];
    assert_eq!(block.objects[0].properties().color, ObjectColor::ByBlock);
    assert_eq!(
        block.objects[0].properties().linetype,
        ObjectLinetype::Named("DOTS".to_owned())
    );
    let Object::AttributeDefinition(definition) = &block.objects[1] else {
        panic!("{:?}", block.objects[1]);
    };
    assert_eq!(definition.properties.color, ObjectColor::ByBlock);
}
