//! The `id` an object carries in a sheet description or template, and the
//! ID each object takes from it, or from its place in the file.

use std::fs;

use super::ids::{ObjectRecord, Written};
use super::strict_yaml;
use super::{parse_block_library, read_sheet_record, sheet_and_records};
use crate::error::{Error, ErrorCode};
use crate::model::BlockLibrary;

fn records(text: &str) -> Result<Vec<ObjectRecord>, Error> {
    let root = strict_yaml::read(text)?;
    let mut no_library = |_: &str| -> Result<BlockLibrary, Error> { unreachable!() };
    sheet_and_records(root, &mut no_library, None, None).map(|(_, records)| records)
}

#[test]
fn an_object_carries_the_id_its_record_gives_or_takes_its_place() {
    let read = records(
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [1, 0, 0]}\n  - circle: {center: [0, 0, 0], radius: 1, id: 'hole:A-1.2'}\n",
    )
    .unwrap();
    assert_eq!(
        read,
        [
            ObjectRecord {
                id: "objects[0]".into(),
                name: None,
                object: Written {
                    field: "objects[0]".into(),
                    line: 3,
                    column: 5
                },
                id_key: None,
                in_row: false,
            },
            ObjectRecord {
                id: "hole:A-1.2".into(),
                name: None,
                object: Written {
                    field: "objects[1]".into(),
                    line: 4,
                    column: 5
                },
                id_key: Some(Written {
                    field: "objects[1].circle.id".into(),
                    line: 4,
                    column: 48
                }),
                in_row: false,
            },
        ]
    );
}

#[test]
fn an_id_holds_ascii_letters_digits_and_four_marks() {
    for (id, why) in [
        ("''", "cannot be empty"),
        ("'a b'", "not ` `"),
        ("'S-1/a'", "`/` joins a sheet's name to the ID"),
        ("'objects[1'", "an object with no ID takes its place"),
        ("'objects[01]'", "a place is written as an ID only whole"),
        ("'rows[1].objects'", "an object with no ID takes its place"),
        ("état", "not `\\u{e9}`"),
    ] {
        let e = records(&format!(
            "units: mm\nobjects:\n  - circle: {{center: [0, 0, 0], radius: 1, id: {id}}}\n"
        ))
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::BadValue, "{id}: {e:?}");
        assert_eq!(e.field.as_deref(), Some("objects[0].circle.id"));
        assert!(e.message.contains(why), "{id}: {e:?}");
    }
    let longest = "a".repeat(255);
    let ok = format!(
        "units: mm\nobjects:\n  - circle: {{center: [0, 0, 0], radius: 1, id: {longest}}}\n"
    );
    assert!(records(&ok).is_ok());
    let e = records(&ok.replace(&longest, &format!("{longest}a"))).unwrap_err();
    assert!(e.message.contains("at most 255 characters"), "{e:?}");
}

#[test]
fn a_place_written_as_an_id_is_kept_and_never_taken_twice() {
    // An operation that deletes an object writes the place of each later
    // object without an `id` as its `id`, so its ID does not move.
    let read = records(
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [1, 0, 0], id: 'objects[2]'}\n  - line: {from: [0, 0, 0], to: [0, 1, 0], id: 'rows[3].objects[0]'}\n",
    )
    .unwrap();
    assert_eq!(read[0].id, "objects[2]");
    assert_eq!(read[1].id, "rows[3].objects[0]");
    let later = records(
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [1, 0, 0], id: 'objects[1]'}\n  - line: {from: [0, 0, 0], to: [0, 1, 0]}\n",
    )
    .unwrap_err();
    assert_eq!(later.code, ErrorCode::DuplicateId);
    assert_eq!(later.field.as_deref(), Some("objects[1]"));
    assert!(later.message.contains("give this one an `id`"), "{later:?}");
    let earlier = records(
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [1, 0, 0]}\n  - line: {from: [0, 0, 0], to: [0, 1, 0], id: 'OBJECTS[0]'}\n",
    )
    .unwrap_err();
    assert_eq!(earlier.code, ErrorCode::BadValue, "{earlier:?}");
    let earlier = records(
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [1, 0, 0]}\n  - line: {from: [0, 0, 0], to: [0, 1, 0], id: 'objects[0]'}\n",
    )
    .unwrap_err();
    assert_eq!(earlier.code, ErrorCode::DuplicateId);
    assert!(
        earlier.message.contains("is the place of the object"),
        "{earlier:?}"
    );
}

#[test]
fn an_id_is_given_once_in_a_file_in_any_case() {
    let e = records(
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [1, 0, 0], id: edge}\n  - line: {from: [0, 0, 0], to: [0, 1, 0]}\n  - line: {from: [0, 0, 0], to: [1, 1, 0], id: EDGE}\n",
    )
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::DuplicateId);
    assert_eq!(e.field.as_deref(), Some("objects[2].line.id"));
    assert_eq!((e.line, e.column), (Some(5), Some(48)));
    assert!(
        e.message
            .starts_with("the ID `EDGE` is given at line 3 already, to the object at `objects[0]`"),
        "{e:?}"
    );
}

#[test]
fn a_blocks_objects_take_no_id() {
    let e = parse_block_library(
        "units: mm\nblocks:\n  B: {objects: [{circle: {center: [0, 0, 0], radius: 1, id: a}}]}\n",
    )
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::UnknownKey);
    assert_eq!(e.field.as_deref(), Some("blocks.B.objects[0].circle.id"));
    assert!(
        e.message.ends_with("a circle takes center, radius, layer"),
        "{e:?}"
    );
}

#[test]
fn a_sheet_description_on_its_own_is_named_by_its_file() {
    let dir = std::env::temp_dir().join(format!("agentdrafter-ids-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("pump-skid.yaml");
    fs::write(
        &path,
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [1, 0, 0]}\n  - line: {from: [0, 0, 0], to: [0, 1, 0], id: edge}\n",
    )
    .unwrap();
    let record = read_sheet_record(&path).unwrap();
    assert_eq!(record.name, "pump-skid");
    assert_eq!(
        [record.id(0), record.id(1)],
        ["pump-skid/objects[0]", "pump-skid/edge"]
    );
    assert_eq!(record.file, path);
    let _ = fs::remove_dir_all(&dir);
}
