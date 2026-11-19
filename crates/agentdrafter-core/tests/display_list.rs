//! A sheet's display list as plain data, for the viewer: a drawing set's
//! sheet by name, a sheet description or a DWG, with the set's sheets, its
//! paper and its scale, written nowhere; the set's sheet draws what its
//! DWG draws.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_set_with, change_with, display_list_with, ChangeRequest, DisplayListReport,
    DisplayListRequest, ErrorCode, ObjectValues, Origin, SetBuildRequest, Settings, Shape,
};
use serde_json::{json, Value};

#[path = "support/demo_set.rs"]
mod demo_set;
#[path = "support/snapshot.rs"]
mod snapshot;

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-display-list")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn draw(source: &Path, sheet: Option<&str>) -> DisplayListReport {
    let mut request = DisplayListRequest::new(source);
    request.sheet = sheet.map(str::to_owned);
    display_list_with(&request, &Settings::default()).unwrap()
}

/// Every point the display list draws through, by colour, rounded to a
/// nanometre of the drawing's unit, in order.
fn points(report: &DisplayListReport) -> Vec<(u8, i64, i64)> {
    let mut points: Vec<_> = report
        .display_list
        .items
        .iter()
        .flat_map(|item| item.points().map(move |p| (item.color.0, p)))
        .map(|(color, [x, y])| (color, (x * 1e9).round() as i64, (y * 1e9).round() as i64))
        .collect();
    points.sort_unstable();
    points
}

#[test]
fn a_drawing_sets_sheet_is_drawn_by_name_with_the_sets_sheets() {
    let dir = scratch("set");
    let set = dir.join("demo-set");
    demo_set::copy_demo_set(&set);
    let before = snapshot::snapshot(&set);

    let first = draw(&set, None);
    assert_eq!(first.sheet, "DEMO-001", "the set's first sheet");
    assert_eq!(first.sheets, ["DEMO-001", "DEMO-002", "DEMO-003"]);
    assert_eq!(first.name, "demo-set");
    assert_eq!(first.millimetres_per_unit, 25.4, "the set is in inches");
    let paper = first.paper.as_ref().unwrap();
    assert_eq!((paper.width, paper.height), (17.0, 11.0));
    assert!(!first.display_list.items.is_empty());
    assert!(first
        .display_list
        .items
        .iter()
        .any(|item| matches!(item.shape, Shape::Stroke { .. })));

    let named = draw(&set, Some("demo-002"));
    assert_eq!(named.sheet, "DEMO-002", "a sheet is named in any case");
    assert_ne!(points(&named), points(&first));

    // `.` has no name of its own: the folder's is found.
    let here = draw(&set.join("."), None);
    assert_eq!(here.name, "demo-set");

    assert_eq!(snapshot::snapshot(&set), before, "nothing is written");
}

#[test]
fn a_sheet_that_is_not_there_or_a_wrong_tolerance_is_refused_by_field() {
    let dir = scratch("refused");
    let set = dir.join("demo-set");
    demo_set::copy_demo_set(&set);
    let mut request = DisplayListRequest::new(&set);
    request.sheet = Some("NOPE".to_owned());
    let error = display_list_with(&request, &Settings::default()).unwrap_err();
    assert_eq!(error.code, ErrorCode::UnknownName);
    assert_eq!(error.field.as_deref(), Some("sheet"));
    assert!(
        error.message.contains("DEMO-001, DEMO-002, DEMO-003"),
        "{}",
        error.message
    );

    for tolerance in [0.0, -1.0, f64::NAN] {
        let mut request = DisplayListRequest::new(&set);
        request.tolerance = Some(tolerance);
        let error = display_list_with(&request, &Settings::default()).unwrap_err();
        assert_eq!(error.code, ErrorCode::BadValue, "{tolerance}");
        assert_eq!(error.field.as_deref(), Some("tolerance"));
    }

    let missing = display_list_with(
        &DisplayListRequest::new(dir.join("none.yaml")),
        &Settings::default(),
    )
    .unwrap_err();
    assert_eq!(missing.code, ErrorCode::ReadFailed);
}

#[test]
fn a_sets_sheet_draws_what_its_dwg_draws() {
    let dir = scratch("dwg");
    let set = dir.join("demo-set");
    demo_set::copy_demo_set(&set);
    let built = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: false,
        },
        &Settings::default(),
    );
    assert!(built.ok, "{:?}", built.error);
    for sheet in ["DEMO-001", "DEMO-003"] {
        let from_set = draw(&set, Some(sheet));
        let dwg = set.join("dwg").join(format!("{sheet}.dwg"));
        let from_dwg = draw(&dwg, None);
        assert_eq!(
            from_dwg.sheet, sheet,
            "a DWG's one sheet is named by its file"
        );
        assert_eq!(from_dwg.sheets, [sheet]);
        // A DWG holds no paper; its set's sheet declares it (Zach, 2026-11-13).
        assert_eq!(
            from_dwg.paper, from_set.paper,
            "a DWG takes its set's paper"
        );
        assert!(from_dwg.paper.is_some());
        assert_eq!(from_dwg.objects_not_drawn, 0);
        assert_eq!(points(&from_dwg), points(&from_set), "{sheet}");
    }
}

#[test]
fn a_sheet_description_is_drawn_with_its_curves_as_fine_as_asked() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/geometry.yaml");
    let report = draw(&source, None);
    assert_eq!(report.sheet, "geometry");
    assert_eq!(report.name, "geometry.yaml");
    assert_eq!(report.millimetres_per_unit, 1.0);
    let extents = report.extents.unwrap();
    let larger = extents.width().max(extents.height());
    assert!(
        (report.tolerance - larger * 1e-5).abs() < 1e-12,
        "{}",
        report.tolerance
    );

    let mut coarse = DisplayListRequest::new(&source);
    coarse.tolerance = Some(1.0);
    let coarse = display_list_with(&coarse, &Settings::default()).unwrap();
    assert_eq!(coarse.tolerance, 1.0);
    assert!(
        points(&coarse).len() < points(&report).len(),
        "finer curves draw through more points"
    );
}

#[test]
fn with_origins_each_object_says_whether_its_template_or_its_row_writes_it() {
    let plant = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo-plant");
    let before = snapshot::snapshot(&plant);
    let mut request = DisplayListRequest::new(&plant);
    request.sheet = Some("DP-LD-101".to_owned());

    // Without the option, nothing is said, and the reply is as it was.
    let plain = display_list_with(&request, &Settings::default()).unwrap();
    assert!(plain.template.is_none());
    assert!(plain.objects.iter().all(|o| o.from.is_none()));
    let text = serde_json::to_string(&plain).unwrap();
    assert!(!text.contains("\"from\"") && !text.contains("\"template\""));

    request.origins = true;
    let told = display_list_with(&request, &Settings::default()).unwrap();
    let reach = told
        .template
        .as_ref()
        .expect("a set's sheet names its template");
    assert_eq!(reach.path, "templates/loop-2-wire.yaml");
    assert_eq!(
        reach.sheets,
        [
            "DP-LD-101",
            "DP-LD-102",
            "DP-LD-104",
            "DP-LD-201",
            "DP-LD-202",
            "DP-LD-301",
            "DP-LD-302"
        ],
        "every sheet built from the template, in the set's order, DP-LD-103 not among them"
    );
    // The row's own two objects, the device and the I/O channel, come last.
    let rows: Vec<&str> = told
        .objects
        .iter()
        .filter(|o| o.from == Some(Origin::Row))
        .filter_map(|o| o.id.as_deref())
        .collect();
    assert_eq!(rows, ["DP-LD-101/device", "DP-LD-101/io-channel"]);
    let border = told
        .objects
        .iter()
        .find(|o| o.id.as_deref() == Some("DP-LD-101/border"))
        .unwrap();
    assert_eq!(border.from, Some(Origin::Template));
    assert!(told.objects.iter().all(|o| o.from.is_some()));
    assert_eq!(
        told.display_list, plain.display_list,
        "what is drawn is the same"
    );

    // The other template's one sheet.
    request.sheet = Some("DP-LD-103".to_owned());
    let other = display_list_with(&request, &Settings::default()).unwrap();
    let reach = other.template.unwrap();
    assert_eq!(reach.path, "templates/loop-3-wire.yaml");
    assert_eq!(reach.sheets, ["DP-LD-103"]);

    // A sheet description on its own has no template and no row.
    let mut alone = DisplayListRequest::new(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/one-line.yaml"),
    );
    alone.origins = true;
    let alone = display_list_with(&alone, &Settings::default()).unwrap();
    assert!(alone.template.is_none());
    assert!(alone.objects.iter().all(|o| o.from.is_none()));

    assert_eq!(snapshot::snapshot(&plant), before, "nothing is written");
}

#[test]
fn with_origins_a_sheet_drawn_in_a_layout_names_the_space_an_add_goes_in() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    // A made-up set of one sheet, its template one of the layout examples.
    let set_of = |name: &str, example: &str, units: &str| -> PathBuf {
        let set = scratch(name);
        fs::create_dir_all(set.join("templates")).unwrap();
        fs::copy(examples.join(example), set.join("templates").join(example)).unwrap();
        fs::write(
            set.join("set.yaml"),
            format!("units: {units}\noutput: dwg\nrows: rows.yaml\n"),
        )
        .unwrap();
        fs::write(
            set.join("rows.yaml"),
            format!("rows:\n  - sheet: LAY-001\n    template: templates/{example}\n"),
        )
        .unwrap();
        set
    };
    let space = |set: &Path, layout: Option<&str>, origins: bool| {
        let mut request = DisplayListRequest::new(set);
        request.layout = layout.map(str::to_owned);
        request.origins = origins;
        display_list_with(&request, &Settings::default())
            .unwrap()
            .space
    };

    // One layout: an add drawn on its page goes in paper space.
    let one = set_of("layout-space-one", "layout.yaml", "mm");
    assert_eq!(space(&one, None, true).as_deref(), Some("paper"));
    assert_eq!(space(&one, None, false), None, "without origins, nothing");

    // Several: an add names the layout drawn.
    let several = set_of("layout-space-several", "layouts.yaml", "mm");
    assert_eq!(space(&several, None, true).as_deref(), Some("Assembly"));
    assert_eq!(
        space(&several, Some("wiring"), true).as_deref(),
        Some("Wiring")
    );

    // A sheet drawn in model space names none.
    let plant = examples.join("demo-plant");
    assert_eq!(space(&plant, None, true), None);
}

/// An object's `values`, as JSON, by its ID.
fn values_of(report: &DisplayListReport, id: &str) -> Value {
    let object = report
        .objects
        .iter()
        .find(|o| o.id.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("no object {id}"));
    serde_json::to_value(object.values.as_ref()).unwrap()
}

#[test]
fn with_values_each_value_says_where_it_is_written_on_the_demo_plant() {
    let plant = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo-plant");
    let before = snapshot::snapshot(&plant);
    let mut request = DisplayListRequest::new(&plant);
    request.sheet = Some("DP-LD-101".to_owned());
    request.origins = true;

    // Without the option, nothing is said, and the reply is as it was.
    let plain = display_list_with(&request, &Settings::default()).unwrap();
    assert!(plain.objects.iter().all(|o| o.values.is_none()));
    assert!(!serde_json::to_string(&plain)
        .unwrap()
        .contains("\"values\""));

    request.values = true;
    let told = display_list_with(&request, &Settings::default()).unwrap();
    assert_eq!(
        told.display_list, plain.display_list,
        "what is drawn is the same"
    );

    // A text whose template takes its contents from the row: the row's
    // value, in its one place on the sheet.
    assert_eq!(
        values_of(&told, "DP-LD-101/service"),
        json!({"contents": {"value": "FT-101 RAW WATER INLET FLOW", "written": "row_value", "row_value": "SERVICE", "fills": 1}})
    );
    // A wire number, which the template writes in four places.
    let wire = told
        .objects
        .iter()
        .find(|o| {
            o.values
                .as_ref()
                .and_then(|v| v.contents.as_ref())
                .is_some_and(|c| c.row_value.as_deref() == Some("WIRE_1"))
        })
        .expect("a wire number from the row");
    let contents = wire.values.as_ref().unwrap().contents.as_ref().unwrap();
    assert_eq!(contents.fills, Some(4));
    // The template's own notes.
    let notes = values_of(&told, "DP-LD-101/notes");
    assert_eq!(notes["contents"]["written"], "own");
    assert!(notes["contents"]["value"]
        .as_str()
        .unwrap()
        .starts_with("NOTES:\n1. A MADE-UP LOOP SHEET"));
    // The title block: values from the row, its own, and those it leaves
    // unset, which show their defaults.
    let title = values_of(&told, "DP-LD-101/title-block");
    let by_tag = |tag: &str| {
        title["attributes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["tag"] == tag)
            .cloned()
            .unwrap()
    };
    assert_eq!(
        by_tag("REV"),
        json!({"tag": "REV", "value": "0", "written": "row_value", "row_value": "REV", "fills": 2}),
        "the revision block's REV_1 takes it too"
    );
    assert_eq!(
        by_tag("DRAWN"),
        json!({"tag": "DRAWN", "value": "AD", "written": "own"})
    );
    assert_eq!(
        by_tag("CHECKED"),
        json!({"tag": "CHECKED", "value": "", "written": "unset"})
    );
    assert_eq!(
        by_tag("SCALE"),
        json!({"tag": "SCALE", "value": "NONE", "written": "unset"})
    );
    // One value for each attribute, in their order.
    let object = told
        .objects
        .iter()
        .find(|o| o.id.as_deref() == Some("DP-LD-101/title-block"))
        .unwrap();
    let tags: Vec<&str> = object.attributes.iter().map(|a| a.tag.as_str()).collect();
    let valued: Vec<&str> = object
        .values
        .as_ref()
        .unwrap()
        .attributes
        .iter()
        .map(|a| a.tag.as_str())
        .collect();
    assert_eq!(tags, valued);
    // The row's own device writes its values itself.
    assert_eq!(
        values_of(&told, "DP-LD-101/device"),
        json!({"attributes": [
            {"tag": "FUNC", "value": "FT", "written": "own"},
            {"tag": "LOOP", "value": "101", "written": "own"}
        ]})
    );
    // A line has nothing quick properties edit.
    assert_eq!(values_of(&told, "DP-LD-101/border"), Value::Null);

    // A sheet description says what each value is, and not where it is
    // written: it has no row.
    let mut alone = DisplayListRequest::new(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/geometry.yaml"),
    );
    alone.values = true;
    let alone = display_list_with(&alone, &Settings::default()).unwrap();
    let given: Vec<&ObjectValues> = alone
        .objects
        .iter()
        .filter_map(|o| o.values.as_ref())
        .collect();
    assert!(
        given.iter().any(|v| v.radius == Some(40.0)),
        "the example's arc"
    );
    let text = serde_json::to_string(&alone).unwrap();
    assert!(!text.contains("\"written\""));

    assert_eq!(snapshot::snapshot(&plant), before, "nothing is written");
}

#[test]
fn with_values_a_made_up_set_says_where_each_value_is_written() {
    let set = scratch("values");
    fs::create_dir_all(set.join("templates")).unwrap();
    fs::write(
        set.join("set.yaml"),
        "units: mm\noutput: dwg\nrows: rows.yaml\n",
    )
    .unwrap();
    fs::write(
        set.join("blocks.yaml"),
        "units: mm\nblocks:\n  TAGGED:\n    objects:\n      - circle: {center: [0, 0, 0], radius: 5}\n      - attdef: {tag: TAG, position: [0, 0, 0], height: 2}\n      - attdef: {tag: NOTE, position: [0, -3, 0], height: 1}\n      - attdef: {tag: NOTE, position: [0, -6, 0], height: 1, default: SEE}\n      - attdef: {tag: BY, position: [0, -9, 0], height: 1, default: AD}\n",
    )
    .unwrap();
    fs::write(
        set.join("templates/t.yaml"),
        "units: mm\nblock_library: ../blocks.yaml\nobjects:\n  - circle: {center: [10, 10, 0], radius: 4, id: hole}\n  - arc: {center: [30, 10, 0], radius: 6, start_angle: 0, end_angle: 90, id: bend}\n  - text: {contents: {row: TITLE}, position: [0, 40, 0], height: 3, id: heading}\n  - text: {contents: FIXED, position: [0, 50, 0], height: 3, id: fixed}\n  - mtext: {contents: {row: TITLE}, position: [0, 60, 0], height: 3, width: 50, id: notes}\n  - insert: {block: TAGGED, position: [50, 50, 0], id: tagged, attributes: {tag: {row: TAG}, NOTE: [{row: TITLE}, null]}}\n",
    )
    .unwrap();
    fs::write(
        set.join("rows.yaml"),
        "rows:\n  - sheet: S-1\n    template: templates/t.yaml\n    values: {TITLE: FIRST, TAG: T-1}\n    objects:\n      - circle: {center: [70, 10, 0], radius: 2.5, id: washer}\n      - insert: {block: TAGGED, position: [80, 50, 0], id: own, attributes: {TAG: X-1}}\n  - sheet: S-2\n    template: templates/t.yaml\n    values: {TITLE: SECOND, TAG: T-2}\n",
    )
    .unwrap();
    let mut request = DisplayListRequest::new(&set);
    request.values = true;
    let told = display_list_with(&request, &Settings::default()).unwrap();

    assert_eq!(values_of(&told, "S-1/hole"), json!({"radius": 4.0}));
    assert_eq!(values_of(&told, "S-1/bend"), json!({"radius": 6.0}));
    assert_eq!(values_of(&told, "S-1/washer"), json!({"radius": 2.5}));
    // TITLE fills the heading, the notes and the first NOTE.
    let title = json!({"value": "FIRST", "written": "row_value", "row_value": "TITLE", "fills": 3});
    assert_eq!(values_of(&told, "S-1/heading"), json!({"contents": title}));
    assert_eq!(values_of(&told, "S-1/notes"), json!({"contents": title}));
    assert_eq!(
        values_of(&told, "S-1/fixed"),
        json!({"contents": {"value": "FIXED", "written": "own"}})
    );
    // A tag written in lowercase, a tag the block defines twice by its
    // occurrence, one item a bare null, and a tag the insert leaves out.
    assert_eq!(
        values_of(&told, "S-1/tagged"),
        json!({"attributes": [
            {"tag": "TAG", "value": "T-1", "written": "row_value", "row_value": "TAG", "fills": 1},
            {"tag": "NOTE", "occurrence": 1, "value": "FIRST", "written": "row_value", "row_value": "TITLE", "fills": 3},
            {"tag": "NOTE", "occurrence": 2, "value": "SEE", "written": "unset"},
            {"tag": "BY", "value": "AD", "written": "unset"}
        ]})
    );
    // The row's own insert.
    assert_eq!(
        values_of(&told, "S-1/own"),
        json!({"attributes": [
            {"tag": "TAG", "value": "X-1", "written": "own"},
            {"tag": "NOTE", "occurrence": 1, "value": "", "written": "unset"},
            {"tag": "NOTE", "occurrence": 2, "value": "SEE", "written": "unset"},
            {"tag": "BY", "value": "AD", "written": "unset"}
        ]})
    );

    // The other sheet, with its own row's values.
    request.sheet = Some("S-2".to_owned());
    let other = display_list_with(&request, &Settings::default()).unwrap();
    assert_eq!(
        values_of(&other, "S-2/heading")["contents"]["value"],
        "SECOND"
    );
}

#[test]
fn with_values_the_set_as_it_was_at_an_entry_says_where_each_value_was_written() {
    // A copy of the demo plant's records, to change.
    let set = scratch("values-entry");
    let plant = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo-plant");
    for file in ["set.yaml", "rows.yaml", "blocks.yaml"] {
        fs::copy(plant.join(file), set.join(file)).unwrap();
    }
    fs::create_dir_all(set.join("templates")).unwrap();
    for file in ["loop-2-wire.yaml", "loop-3-wire.yaml"] {
        fs::copy(
            plant.join("templates").join(file),
            set.join("templates").join(file),
        )
        .unwrap();
    }
    let apply = |change_set: Value| {
        let reply = change_with(
            &ChangeRequest {
                set: set.clone(),
                change_set,
                dry_run: false,
            },
            &Settings::default(),
        );
        reply.result.unwrap().journal_entry.unwrap()
    };
    // Entry 1 touches the row alone; entry 2 sets CHECKED, which the
    // template left unset, on the template.
    let first = apply(
        json!({"operations": [{"op": "set_attribute", "target": {"id": "DP-LD-101/title-block"}, "tag": "REV", "value": "A"}]}),
    );
    let second = apply(
        json!({"operations": [{"op": "set_attribute", "target": {"id": "DP-LD-101/title-block"}, "tag": "CHECKED", "value": "ZM"}]}),
    );
    assert_eq!((first, second), (1, 2));
    let checked = |entry: Option<u64>| {
        let mut request = DisplayListRequest::new(&set);
        request.sheet = Some("DP-LD-101".to_owned());
        request.values = true;
        request.entry = entry;
        let told = display_list_with(&request, &Settings::default()).unwrap();
        let title = values_of(&told, "DP-LD-101/title-block");
        let by_tag = |tag: &str| {
            title["attributes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["tag"] == tag)
                .cloned()
                .unwrap()
        };
        (by_tag("CHECKED"), by_tag("REV"))
    };
    let (was, rev_was) = checked(Some(first));
    assert_eq!(
        was,
        json!({"tag": "CHECKED", "value": "", "written": "unset"}),
        "as it was at entry 1, the template left it unset"
    );
    assert_eq!(rev_was["value"], "A");
    assert_eq!(rev_was["written"], "row_value");
    let (now, _) = checked(None);
    assert_eq!(
        now,
        json!({"tag": "CHECKED", "value": "ZM", "written": "own"}),
        "as it is, the template writes it"
    );
}
