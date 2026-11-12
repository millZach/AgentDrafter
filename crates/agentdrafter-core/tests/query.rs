//! Every object keeps its ID: the ID its record gives it, or its place in
//! the file, joined to its sheet's name, written into the DWG with it and
//! the same after a rebuild. A query asks the demo set's records for
//! objects by ID, name, block, attribute and text on every sheet, lists
//! blocks, reads attributes as JSON, and measures in the declared unit; a
//! query that matches nothing says so and succeeds.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_set_with, query_with, read_dwg, Answer, End, ErrorCode, Matching, Measure, Object, Query,
    QueryReport, QueryRequest, Selection, SetBuildRequest, Settings, Unit,
};
use serde_json::json;

#[path = "support/demo_set.rs"]
mod demo_set;

const SHEETS: [&str; 3] = ["DEMO-001", "DEMO-002", "DEMO-003"];

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-query")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A copy of the demo plant's drawing set in a folder of its own.
fn demo_set(test: &str) -> PathBuf {
    let set = scratch(test).join("demo-set");
    demo_set::copy_demo_set(&set);
    set
}

fn build(set: &Path, overwrite: bool) {
    let reply = build_set_with(
        &SetBuildRequest {
            set: set.to_owned(),
            overwrite,
        },
        &Settings::default(),
    );
    assert!(reply.ok, "{:#?}", reply.error);
}

/// Each object a built sheet's DWG holds, with the ID it carries.
fn built(set: &Path, sheet: &str) -> Vec<(String, Object)> {
    let read = read_dwg(&set.join("dwg").join(format!("{sheet}.dwg"))).unwrap();
    assert_eq!(read.ids_and_names.len(), read.objects.len());
    read.ids_and_names
        .into_iter()
        .map(|i| i.id.expect("every object a build writes carries its ID"))
        .zip(read.objects)
        .collect()
}

fn ask_on(source: &Path, sheet: Option<&str>, query: Query) -> QueryReport {
    query_with(
        &QueryRequest {
            source: source.to_owned(),
            sheet: sheet.map(str::to_owned),
            query,
        },
        &Settings::default(),
    )
    .unwrap_or_else(|e| panic!("{e:#?}"))
}

fn ask(source: &Path, query: Query) -> QueryReport {
    ask_on(source, None, query)
}

fn find(source: &Path, selection: Selection) -> QueryReport {
    ask(source, Query::Find(selection))
}

/// The sheet and ID of each object a find matched.
fn found(report: &QueryReport) -> Vec<(String, String)> {
    let Answer::Find { count, objects } = &report.answer else {
        panic!("a find's answer: {report:#?}");
    };
    assert_eq!(*count, objects.len());
    objects
        .iter()
        .map(|o| (o.sheet.clone(), o.id.clone()))
        .collect()
}

fn on_every_sheet(own: &str) -> Vec<(String, String)> {
    SHEETS
        .iter()
        .map(|s| ((*s).to_owned(), format!("{s}/{own}")))
        .collect()
}

fn by_id(id: &str) -> End {
    End {
        id: Some(id.to_owned()),
        ..End::default()
    }
}

fn measure(source: &Path, sheet: Option<&str>, from: End, to: End) -> QueryReport {
    ask_on(source, sheet, Query::Measure(Measure { from, to }))
}

fn distance(report: &QueryReport) -> (f64, [f64; 3]) {
    match &report.answer {
        Answer::Measure {
            distance: Some(distance),
            delta: Some(delta),
            ..
        } => (*distance, *delta),
        other => panic!("a measure's answer: {other:#?}"),
    }
}

#[track_caller]
fn close(a: f64, b: f64, within: f64) {
    assert!((a - b).abs() <= within, "{a} is not {b}, to {within}");
}

#[test]
fn every_object_carries_its_id_into_the_dwg_and_keeps_it_through_a_rebuild() {
    let set = demo_set("rebuild");
    build(&set, false);
    let first: Vec<Vec<(String, Object)>> = SHEETS.iter().map(|s| built(&set, s)).collect();

    for (sheet, objects) in SHEETS.iter().zip(&first) {
        let ids: Vec<&str> = objects.iter().map(|(id, _)| id.as_str()).collect();
        // The template gives the border, the heading and the notes an `id`;
        // the trim line and the title block take their places.
        let expected: Vec<String> = ["objects[0]", "border", "objects[2]", "heading", "notes"]
            .iter()
            .map(|own| format!("{sheet}/{own}"))
            .collect();
        assert_eq!(ids, expected);
    }
    // The query reads the same IDs from the records.
    let everything = found(&find(&set, Selection::default()));
    let in_dwgs: Vec<(String, String)> = SHEETS
        .iter()
        .zip(&first)
        .flat_map(|(s, objects)| objects.iter().map(|(id, _)| ((*s).to_owned(), id.clone())))
        .collect();
    assert_eq!(everything, in_dwgs);

    build(&set, true);
    let again: Vec<Vec<(String, Object)>> = SHEETS.iter().map(|s| built(&set, s)).collect();
    assert_eq!(again, first, "a rebuild gives every object the same ID");
}

#[test]
fn an_id_the_record_gives_survives_an_object_added_before_it() {
    let set = demo_set("added");
    build(&set, false);
    let before = built(&set, "DEMO-002");

    let template = set.join("templates/ansi-b.yaml");
    let text = fs::read_to_string(&template).unwrap();
    let text = text.replacen(
        "objects:\n",
        "objects:\n  - line: {from: [1, 1, 0], to: [2, 2, 0], layer: notes}\n",
        1,
    );
    fs::write(&template, text).unwrap();
    build(&set, true);
    let after = built(&set, "DEMO-002");

    assert_eq!(after.len(), before.len() + 1);
    for own in ["border", "heading", "notes"] {
        let id = format!("DEMO-002/{own}");
        let was = before.iter().position(|(i, _)| *i == id).unwrap();
        let is = after.iter().position(|(i, _)| *i == id).unwrap();
        assert_eq!(is, was + 1, "{id} moved down one place in the file");
        assert_eq!(after[is].1, before[was].1, "{id} is the same object");
    }
    // An object with no `id` takes its place, which moved.
    assert!(matches!(after[0], (ref id, Object::Line(_)) if id == "DEMO-002/objects[0]"));
    assert_eq!(after[1].1, before[0].1);
    assert_eq!(after[1].0, "DEMO-002/objects[1]");
}

#[test]
fn a_query_finds_objects_by_id_name_block_attribute_and_text_on_every_sheet() {
    let set = demo_set("find");

    // By ID: the whole ID is one object; part of it finds one per sheet.
    let one = find(
        &set,
        Selection {
            id: Some("demo-002/BORDER".into()),
            ..Selection::default()
        },
    );
    assert_eq!(
        found(&one),
        [("DEMO-002".to_owned(), "DEMO-002/border".to_owned())]
    );
    let Answer::Find { objects, .. } = &one.answer else {
        unreachable!()
    };
    assert!(objects[0].id_written);
    assert_eq!(objects[0].written_at.field, "objects[1]");
    assert!(objects[0].written_at.file.ends_with("ansi-b.yaml"));
    assert!(matches!(objects[0].object, Object::Polyline(_)));
    assert_eq!(one.units, Unit::In);
    assert_eq!(one.sheets, SHEETS);

    let part = Selection {
        id: Some("Border".into()),
        matching: Matching::Contains,
        ..Selection::default()
    };
    assert_eq!(found(&find(&set, part)), on_every_sheet("border"));

    // By name, in any case, and by block.
    let named = Selection {
        name: Some("TITLE-BLOCK".into()),
        ..Selection::default()
    };
    assert_eq!(found(&find(&set, named)), on_every_sheet("objects[2]"));
    let of_block = Selection {
        block: Some("title_block".into()),
        ..Selection::default()
    };
    assert_eq!(found(&find(&set, of_block)), on_every_sheet("objects[2]"));

    // By an attribute's value: exactly, case and all, in any attribute or the one tagged.
    let value = |tag: Option<&str>, value: &str, matching: Matching| Selection {
        tag: tag.map(str::to_owned),
        value: Some(value.to_owned()),
        matching,
        ..Selection::default()
    };
    let area_200 = "AREA 200 GENERAL ARRANGEMENT";
    let on_002 = [("DEMO-002".to_owned(), "DEMO-002/objects[2]".to_owned())];
    assert_eq!(
        found(&find(&set, value(None, area_200, Matching::Exact))),
        on_002
    );
    assert_eq!(
        found(&find(&set, value(Some("title"), area_200, Matching::Exact))),
        on_002
    );
    assert!(found(&find(
        &set,
        value(Some("DWG_NO"), area_200, Matching::Exact)
    ))
    .is_empty());
    assert!(found(&find(&set, value(None, "ad", Matching::Exact))).is_empty());
    assert_eq!(
        found(&find(&set, value(Some("DRAWN"), "ad", Matching::Contains))),
        on_every_sheet("objects[2]")
    );

    // By text: a text's or an mtext's contents.
    let text = |text: &str, matching: Matching| Selection {
        text: Some(text.to_owned()),
        matching,
        ..Selection::default()
    };
    assert_eq!(
        found(&find(
            &set,
            text("AREA 300 GENERAL ARRANGEMENT", Matching::Exact)
        )),
        [("DEMO-003".to_owned(), "DEMO-003/heading".to_owned())]
    );
    assert_eq!(
        found(&find(&set, text("made-up drawing", Matching::Contains))),
        on_every_sheet("notes")
    );

    // Parts together must all match; one sheet can be asked alone.
    let both = Selection {
        block: Some("TITLE_BLOCK".into()),
        tag: Some("REV".into()),
        value: Some("A".into()),
        ..Selection::default()
    };
    assert_eq!(
        found(&find(&set, both)),
        [("DEMO-003".to_owned(), "DEMO-003/objects[2]".to_owned())]
    );
    let alone = ask_on(&set, Some("demo-001"), Query::Find(Selection::default()));
    assert_eq!(alone.sheets, ["DEMO-001"]);
    assert_eq!(found(&alone).len(), 5);
    let e = query_with(
        &QueryRequest {
            source: set.clone(),
            sheet: Some("DEMO-009".into()),
            query: Query::Blocks,
        },
        &Settings::default(),
    )
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::UnknownName);
    assert_eq!(
        e.message,
        "there is no sheet `DEMO-009` to query; the sheets: DEMO-001, DEMO-002, DEMO-003"
    );
}

#[test]
fn a_query_lists_the_blocks_on_each_sheet_and_reads_attributes_as_json() {
    let set = demo_set("blocks");

    let blocks = ask(&set, Query::Blocks);
    let listed = serde_json::to_value(&blocks).unwrap();
    assert_eq!(listed["kind"], "blocks");
    assert_eq!(listed["count"], 3);
    for (i, sheet) in SHEETS.iter().enumerate() {
        assert_eq!(
            listed["blocks"][i],
            json!({"sheet": sheet, "blocks": [{"block": "TITLE_BLOCK", "inserts": [format!("{sheet}/objects[2]")]}]})
        );
    }

    let attributes = ask(
        &set,
        Query::Attributes(Selection {
            name: Some("title-block".into()),
            ..Selection::default()
        }),
    );
    let read = serde_json::to_value(&attributes).unwrap();
    assert_eq!(read["kind"], "attributes");
    assert_eq!(read["count"], 3);
    assert_eq!(read["inserts"][1]["sheet"], "DEMO-002");
    assert_eq!(read["inserts"][1]["id"], "DEMO-002/objects[2]");
    assert_eq!(read["inserts"][1]["block"], "TITLE_BLOCK");
    assert_eq!(
        read["inserts"][1]["attributes"],
        json!({
            "TITLE": "AREA 200 GENERAL ARRANGEMENT",
            "DWG_NO": "DEMO-002",
            "SHEET": "2 OF 3",
            "REV": "0",
            "DATE": "2026-11-11",
            "DRAWN": "AD",
        })
    );
    // In the order the block defines them.
    let Answer::Attributes { inserts, .. } = &attributes.answer else {
        unreachable!()
    };
    let line = serde_json::to_string(&inserts[2].attributes).unwrap();
    let mut keys = Vec::new();
    for tag in ["TITLE", "DWG_NO", "SHEET", "REV", "DATE", "DRAWN"] {
        keys.push(line.find(&format!("\"{tag}\"")).unwrap());
    }
    assert!(keys.windows(2).all(|w| w[0] < w[1]), "{line}");
}

#[test]
fn a_query_measures_a_distance_and_the_extents_of_an_inch_set_in_inches() {
    let set = demo_set("inches");

    // From the heading's position to the title block's, named on one sheet.
    let named = End {
        name: Some("title-block".into()),
        ..End::default()
    };
    let report = measure(&set, Some("DEMO-001"), by_id("DEMO-001/heading"), named);
    assert_eq!(report.units, Unit::In);
    let (d, delta) = distance(&report);
    close(d, 330.5f64.sqrt(), 1e-12);
    assert_eq!(delta, [15.5, -9.5, 0.0]);
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["from"]["point"], "position");
    assert_eq!(json["to"]["type"], "insert");
    assert_eq!(json["to"]["at"], json!([16.5, 0.5, 0.0]));

    // Between two corners of the border, which is a polyline.
    let corner = |point: &str| End {
        point: Some(point.to_owned()),
        ..by_id("DEMO-002/border")
    };
    let (diagonal, _) = distance(&measure(
        &set,
        None,
        corner("points[0]"),
        corner("points[2]"),
    ));
    close(diagonal, (16f64 * 16.0 + 10.0 * 10.0).sqrt(), 1e-12);

    // Every sheet draws within its trim line, 17 by 11 inches.
    let extents = ask(&set, Query::Extents);
    let Answer::Extents { count, extents, .. } = &extents.answer else {
        panic!("an extents answer: {extents:#?}");
    };
    assert_eq!(*count, 3);
    for (sheet, found) in SHEETS.iter().zip(extents) {
        assert_eq!(found.sheet, *sheet);
        let e = found.extents.unwrap();
        assert_eq!((e.min, e.max), ([0.0, 0.0], [17.0, 11.0]), "{sheet}");
        assert_eq!((e.width, e.height), (17.0, 11.0));
    }
}

#[test]
fn a_query_measures_a_millimetre_sheet_in_millimetres() {
    // The block example's first sheet, in millimetres, read where it is.
    let sheet = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/blocks/sheet-1.yaml");
    let name = |name: &str| End {
        name: Some(name.to_owned()),
        ..End::default()
    };
    let report = measure(&sheet, None, name("marker-a1"), name("marker-a2"));
    assert_eq!(report.units, Unit::Mm);
    assert_eq!(report.sheets, ["sheet-1"]);
    let (d, delta) = distance(&report);
    close(d, 2600f64.sqrt(), 1e-12);
    assert_eq!(delta, [50.0, 10.0, 0.0]);

    // A made-up sheet whose extents are known: a line and a circle, one
    // length given in inches and converted to the declared unit.
    let dir = scratch("millimetres");
    let made_up = dir.join("plate.yaml");
    fs::write(
        &made_up,
        "units: mm\nobjects:\n  - line: {from: [10, 20, 0], to: ['4 in', 20, 0], id: base}\n  \
         - circle: {center: [60, 50, 0], radius: 25, id: hole}\n",
    )
    .unwrap();
    let report = ask(&made_up, Query::Extents);
    assert_eq!(report.units, Unit::Mm);
    let Answer::Extents { extents, .. } = &report.answer else {
        panic!("an extents answer: {report:#?}");
    };
    let e = extents[0].extents.unwrap();
    // Curves are drawn within a millionth of the sheet's larger side.
    let within = 101.6 * 1e-6;
    assert_eq!(e.min[0], 10.0);
    assert_eq!(e.max[0], 101.6);
    close(e.min[1], 20.0, within);
    close(e.max[1], 75.0, within);
    let (d, _) = distance(&measure(
        &made_up,
        None,
        by_id("plate/base"),
        by_id("plate/hole"),
    ));
    close(d, (50f64 * 50.0 + 30.0 * 30.0).sqrt(), 1e-12);
}

#[test]
fn a_query_that_matches_nothing_succeeds_and_says_so() {
    let set = demo_set("nothing");

    let report = find(
        &set,
        Selection {
            text: Some("NO SUCH TEXT".into()),
            ..Selection::default()
        },
    );
    assert!(found(&report).is_empty());
    assert_eq!(
        report.message,
        "nothing matches: no object on the 3 sheets looked at has the text `NO SUCH TEXT`"
    );
    let line = serde_json::to_value(&report).unwrap();
    assert_eq!(line["count"], 0);
    assert_eq!(line["objects"], json!([]));

    let attributes = ask(
        &set,
        Query::Attributes(Selection {
            block: Some("NO_SUCH_BLOCK".into()),
            ..Selection::default()
        }),
    );
    assert!(matches!(
        attributes.answer,
        Answer::Attributes { count: 0, .. }
    ));
    assert!(
        attributes.message.starts_with("nothing matches"),
        "{}",
        attributes.message
    );

    let dir = scratch("nothing-drawn");
    let empty = dir.join("empty.yaml");
    fs::write(&empty, "units: mm\nobjects: []\n").unwrap();
    let blocks = ask(&empty, Query::Blocks);
    assert!(matches!(blocks.answer, Answer::Blocks { count: 0, .. }));
    assert!(
        blocks.message.starts_with("nothing matches"),
        "{}",
        blocks.message
    );
    let extents = ask(&empty, Query::Extents);
    assert_eq!(
        extents.message,
        "nothing matches: sheet `empty` draws nothing"
    );
    assert!(
        matches!(&extents.answer, Answer::Extents { count: 0, extents, .. } if extents[0].extents.is_none())
    );
}

#[test]
fn a_distance_to_an_object_nobody_has_matches_nothing_and_is_not_an_error() {
    let set = demo_set("no-such");

    let report = measure(&set, None, by_id("DEMO-001/nope"), by_id("DEMO-001/border"));
    assert_eq!(
        report.message,
        "nothing matches: no object on the 3 sheets looked at has the ID `DEMO-001/nope`, the \
         end to measure from"
    );
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["kind"], "measure");
    assert!(json.get("distance").is_none(), "{json}");
    assert!(json.get("from").is_none(), "{json}");
    assert_eq!(json["to"]["id"], "DEMO-001/border");

    let nobody = End {
        name: Some("nobody".into()),
        ..End::default()
    };
    let report = measure(&set, Some("DEMO-002"), by_id("DEMO-002/nope"), nobody);
    assert_eq!(
        report.message,
        "nothing matches: no object on sheet `DEMO-002` has the ID `DEMO-002/nope`, the end to \
         measure from, nor the name `nobody`, the end to measure to"
    );
    assert!(matches!(
        report.answer,
        Answer::Measure {
            distance: None,
            from: None,
            to: None,
            ..
        }
    ));

    // A request that cannot be answered as asked is still an error.
    let ask_measure = |sheet: Option<&str>, from: End, to: End| {
        query_with(
            &QueryRequest {
                source: set.clone(),
                sheet: sheet.map(str::to_owned),
                query: Query::Measure(Measure { from, to }),
            },
            &Settings::default(),
        )
        .unwrap_err()
    };
    // A name every sheet gives needs its sheet.
    let everywhere = End {
        name: Some("title-block".into()),
        ..End::default()
    };
    let e = ask_measure(None, everywhere, by_id("DEMO-001/border"));
    assert_eq!(e.code, ErrorCode::BadValue);
    assert!(e.message.contains("DEMO-001, DEMO-002, DEMO-003"), "{e:?}");

    // Two sheets are two drawings; a point an object does not have lists its points.
    let e = ask_measure(None, by_id("DEMO-001/border"), by_id("DEMO-002/border"));
    assert_eq!(e.code, ErrorCode::BadValue);
    let middle = End {
        point: Some("middle".into()),
        ..by_id("DEMO-001/border")
    };
    let e = ask_measure(None, middle, by_id("DEMO-001/heading"));
    assert_eq!(e.code, ErrorCode::UnknownName);
    assert_eq!(
        e.message,
        "a polyline `DEMO-001/border` has no point `middle`; its points: points[0] to points[3]"
    );
}

#[test]
fn a_duplicate_id_is_refused_naming_both_places() {
    let set = demo_set("duplicate");
    let template = set.join("templates/ansi-b.yaml");
    let text = fs::read_to_string(&template).unwrap();
    // The trim line takes the border's ID, in another case.
    let text = text.replacen(
        "closed: true, layer: trim}",
        "closed: true, layer: trim, id: BORDER}",
        1,
    );
    fs::write(&template, text).unwrap();

    let e = query_with(
        &QueryRequest {
            source: set.clone(),
            sheet: None,
            query: Query::Blocks,
        },
        &Settings::default(),
    )
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::DuplicateId);
    assert_eq!(e.field.as_deref(), Some("objects[1].polyline.id"));
    assert!(e.file.as_deref().unwrap().ends_with("ansi-b.yaml"), "{e:?}");
    let trim_line = 17;
    assert_eq!(e.line, Some(trim_line + 1));
    assert!(
        e.message.contains(&format!(
            "the ID `border` is given at line {trim_line} already, to the object at `objects[0]`"
        )),
        "{e:?}"
    );

    // Nothing is built from a set whose template gives an ID twice.
    let reply = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: false,
        },
        &Settings::default(),
    );
    let report = reply.result.unwrap();
    assert_eq!(report.built, 0);
    for sheet in &report.sheets {
        assert_eq!(sheet.error.as_ref().unwrap().code, ErrorCode::DuplicateId);
    }
    assert!(!set.join("dwg").join("DEMO-001.dwg").exists());
}

#[test]
fn extents_follow_the_geometry_not_the_dashes_of_its_linetype() {
    // A circle of radius 10 whose linetype draws a dash of 1 and a gap of
    // 20: the dashes cover a little of it, its extents all of it.
    let dir = scratch("dashed");
    let dashed = dir.join("dashed.yaml");
    fs::write(
        &dashed,
        "units: mm\nlinetypes:\n  SPARSE: {pattern: [1, -20]}\nlayers:\n  dashed: {linetype: SPARSE}\n\
         objects:\n  - circle: {center: [50, 50, 0], radius: 10, layer: dashed}\n",
    )
    .unwrap();
    let report = ask(&dashed, Query::Extents);
    let Answer::Extents { extents, .. } = &report.answer else {
        panic!("an extents answer: {report:#?}");
    };
    let e = extents[0].extents.unwrap();
    let within = 20.0 * 1e-6;
    close(e.width, 20.0, within);
    close(e.height, 20.0, within);
    close(e.min[0], 40.0, within);
    close(e.max[1], 60.0, within);
}

#[test]
fn every_object_may_have_a_name_carried_into_its_dwg_and_found_by_it() {
    let set = demo_set("names");
    let template = set.join("templates/ansi-b.yaml");
    let text = fs::read_to_string(&template).unwrap();
    let text = text.replacen(
        "layer: border, id: border}",
        "layer: border, id: border, name: frame}",
        1,
    );
    fs::write(&template, text).unwrap();
    build(&set, false);

    // The DWG carries the border's name, and the title block's as before.
    let read = read_dwg(&set.join("dwg/DEMO-002.dwg")).unwrap();
    let named: Vec<(Option<&str>, Option<&str>)> = read
        .ids_and_names
        .iter()
        .map(|i| (i.id.as_deref(), i.name.as_deref()))
        .filter(|(_, name)| name.is_some())
        .collect();
    assert_eq!(
        named,
        [
            (Some("DEMO-002/border"), Some("frame")),
            (Some("DEMO-002/objects[2]"), Some("title-block")),
        ]
    );

    // Found by its name, in any case, on every sheet.
    let by_name = Selection {
        name: Some("FRAME".into()),
        ..Selection::default()
    };
    assert_eq!(found(&find(&set, by_name)), on_every_sheet("border"));

    // And measured from by its name: the border's first corner to the title block.
    let named = |name: &str| End {
        name: Some(name.to_owned()),
        ..End::default()
    };
    let report = measure(&set, Some("DEMO-001"), named("frame"), named("title-block"));
    let (d, delta) = distance(&report);
    assert_eq!((d, delta), (16.0, [16.0, 0.0, 0.0]));
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["from"]["name"], "frame");
    assert_eq!(json["from"]["point"], "points[0]");

    // A name is given once on a sheet, whatever objects carry it.
    let text = fs::read_to_string(&template).unwrap();
    let text = text.replacen("layer: trim}", "layer: trim, name: Title-Block}", 1);
    fs::write(&template, text).unwrap();
    let e = query_with(
        &QueryRequest {
            source: set.clone(),
            sheet: None,
            query: Query::Blocks,
        },
        &Settings::default(),
    )
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::DuplicateName);
    assert_eq!(e.field.as_deref(), Some("objects[2].insert.name"));
    assert!(
        e.message.contains(
            "the name `title-block` is given at line 17 already, to the object at `objects[0]`"
        ),
        "{e:?}"
    );
}

#[test]
fn a_query_finds_text_an_inserted_block_draws() {
    let set = demo_set("block-text");
    let report = find(
        &set,
        Selection {
            text: Some("DEMO PLANT".into()),
            ..Selection::default()
        },
    );
    assert_eq!(found(&report), on_every_sheet("objects[2]"));
    let Answer::Find { objects, .. } = &report.answer else {
        unreachable!()
    };
    for object in objects {
        assert!(matches!(object.object, Object::Insert(_)));
        assert_eq!(object.block_text.len(), 1);
        let text = &object.block_text[0];
        assert_eq!(text.field, "blocks.TITLE_BLOCK.objects[7]");
        assert_eq!(text.contents, "DEMO PLANT");
        // The template names it as `../blocks.yaml`; it is the set's
        // `blocks.yaml`, worked out (#35).
        assert_eq!(text.file, set.join("blocks.yaml").display().to_string());
    }
    // Text on the sheet itself carries no block text.
    let heading = find(
        &set,
        Selection {
            text: Some("AREA 100 GENERAL ARRANGEMENT".into()),
            ..Selection::default()
        },
    );
    let line = serde_json::to_value(&heading).unwrap();
    assert_eq!(line["objects"][0]["id"], "DEMO-001/heading");
    assert!(line["objects"][0].get("block_text").is_none(), "{line}");
}

/// When the set's `templates/` is a link to a folder elsewhere, the
/// template's `../blocks.yaml` is read beside that folder, not in the set,
/// so the block library is given by the path it was read from.
#[cfg(unix)]
#[test]
fn block_text_names_the_block_library_read_through_a_linked_folder() {
    let dir = scratch("block-text-linked");
    let set = dir.join("set");
    let shared = dir.join("shared");
    demo_set::copy_demo_set(&set);
    fs::create_dir_all(&shared).unwrap();
    fs::rename(set.join("templates"), shared.join("templates")).unwrap();
    fs::rename(set.join("blocks.yaml"), shared.join("blocks.yaml")).unwrap();
    std::os::unix::fs::symlink("../shared/templates", set.join("templates")).unwrap();
    let report = find(
        &set,
        Selection {
            text: Some("DEMO PLANT".into()),
            ..Selection::default()
        },
    );
    let Answer::Find { objects, .. } = &report.answer else {
        unreachable!()
    };
    assert_eq!(objects.len(), SHEETS.len());
    let read = set.join("templates").join("../blocks.yaml");
    assert!(!set.join("blocks.yaml").exists());
    for object in objects {
        let text = &object.block_text[0];
        assert_eq!(text.file, read.display().to_string());
        assert!(Path::new(&text.file).is_file(), "{text:?}");
    }
}
