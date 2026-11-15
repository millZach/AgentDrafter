//! Dimensions: the reader takes dimensions and their styles, puts their
//! locations where AutoCAD keeps them and refuses what the pipeline cannot
//! draw, naming the field; the one pipeline draws their lines, arrowheads
//! and text; the DWG reads back the same dimensions and styles, with its
//! `Defpoints` layer and its dimensions' blocks read with them; the checker
//! tells a dimension's lines from its letters; a query gives what each
//! measures and shows; and change sets add them by kind, with snapped
//! points, move them, change them, and edit a template's dimension styles.
//! Every dimension here is made up. AutoCAD 2026 opened, audited, laid out
//! again and plotted what these write (see the pull request).

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build, build_set_with, change_with, check_with, display_list_with, operations,
    parse_sheet_description, query_with, read_dwg, Answer, Arrowhead, BuildRequest, ChangeReport,
    ChangeRequest, CheckRequest, DecimalSeparator, Dimension, DimensionKind, DimensionStyle,
    DisplayListRequest, ErrorCode, Measured, Object, Point3, Query, QueryRequest, Reply, Rule,
    SetBuildRequest, Settings, Sheet, TextPlacement, Unit, DIAMETER_SIGN,
};
use serde_json::{json, Value};

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-dimension")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A made-up mounting plate: its outline, a hole and a slot's end,
/// dimensioned in ISO-25 and in two styles of its own.
const PLATE: &str = "units: mm
paper: [200, 120]
layers:
  dims: {color: green}
dimension_styles:
  holes:
    arrowhead: dot
    arrow_size: 2
    text_height: 3
    precision: 1
    suffix: ' mm'
  reference:
    placement: centered
    decimal_separator: period
    suppress_trailing_zeros: true
objects:
  - polyline: {id: plate, points: [[20, 20, 0], [180, 20, 0], [180, 100, 0], [20, 100, 0]], closed: true}
  - circle: {id: hole, center: [60, 60, 0], radius: 5}
  - dimension: {id: width, kind: linear, points: [[20, 20, 0], [180, 20, 0]], location: [100, 8, 0], layer: dims}
  - dimension: {id: height, kind: linear, angle: 90, points: [[180, 20, 0], [180, 100, 0]], location: [195, 50, 0]}
  - dimension: {id: diagonal, kind: aligned, points: [[20, 20, 0], [180, 100, 0]], location: [90, 70, 0], dimension_style: reference}
  - dimension: {id: hole-size, kind: diameter, center: [60, 60, 0], point: [65, 60, 0], location: [80, 80, 0], dimension_style: holes}
  - dimension: {id: hole-radius, kind: radius, center: [60, 60, 0], point: [55, 60, 0], location: [75, 45, 0], text: <> TYP}
  - polyline: {points: [[0, 0, 0], [200, 0, 0], [200, 120, 0], [0, 120, 0]], closed: true}
";

fn sheet() -> Sheet {
    parse_sheet_description(PLATE).unwrap()
}

fn dimension(sheet: &Sheet, index: usize) -> &Dimension {
    match &sheet.objects[index] {
        Object::Dimension(d) => d,
        other => panic!("{other:?}"),
    }
}

fn close(a: Point3, b: [f64; 3]) {
    assert!(
        (a.x - b[0]).abs() < 1e-9 && (a.y - b[1]).abs() < 1e-9 && (a.z - b[2]).abs() < 1e-9,
        "{a:?} is not {b:?}"
    );
}

#[test]
fn dimensions_and_their_styles_read_with_what_they_give() {
    let sheet = sheet();
    let width = dimension(&sheet, 2);
    assert_eq!(width.kind(), DimensionKind::Linear);
    assert_eq!(width.dimension_style, "ISO-25");
    assert_eq!(width.measurement(), 160.0);
    // Kept where the dimension line meets the second extension line.
    close(width.location, [180.0, 8.0, 0.0]);
    let height = dimension(&sheet, 3);
    assert_eq!(height.measurement(), 80.0);
    close(height.location, [195.0, 100.0, 0.0]);
    let diagonal = dimension(&sheet, 4);
    assert!((diagonal.measurement() - 160f64.hypot(80.0)).abs() < 1e-9);
    // A radius's arrow is where the line from the centre through its
    // location meets the circle, whatever point on it was given.
    let radius = dimension(&sheet, 6);
    let Measured::Radius { center, point } = radius.measured else {
        panic!()
    };
    close(center, [60.0, 60.0, 0.0]);
    let away = (point.x - 60.0).hypot(point.y - 60.0);
    assert!((away - 5.0).abs() < 1e-12);
    assert!(((point.y - 60.0) / (point.x - 60.0) + 1.0).abs() < 1e-12);
    // The metric sheet's own style is AutoCAD's ISO-25.
    let iso = sheet.dimension_style("ISO-25").unwrap();
    assert_eq!(iso, DimensionStyle::iso_25(Unit::Mm));
    assert_eq!(
        (iso.arrow_size, iso.text_height, iso.gap, iso.placement),
        (2.5, 2.5, 0.625, TextPlacement::Above)
    );
    assert!(sheet.dimension_style("Standard").is_none());
    let holes = sheet.dimension_style("holes").unwrap();
    assert_eq!(holes.arrowhead, Arrowhead::Dot);
    assert_eq!(holes.decimal_separator, DecimalSeparator::Comma);
    assert_eq!(holes.suffix, " mm");
    // What each shows.
    assert_eq!(width.displayed_text(&iso), "160");
    let reference = sheet.dimension_style("reference").unwrap();
    assert_eq!(diagonal.displayed_text(&reference), "178.89");
    assert_eq!(dimension(&sheet, 5).displayed_text(&holes), "10 mm");
    assert_eq!(radius.displayed_text(&iso), "R5 TYP");
    // An imperial sheet's own is AutoCAD's Standard.
    let imperial = parse_sheet_description(
        "units: in
objects:
  - dimension: {kind: diameter, center: [1, 1, 0], point: [1.25, 1, 0], location: [2, 2, 0]}
",
    )
    .unwrap();
    let d = dimension(&imperial, 0);
    assert_eq!(d.dimension_style, "Standard");
    let standard = imperial.dimension_style("Standard").unwrap();
    assert_eq!(standard, DimensionStyle::standard(Unit::In));
    assert_eq!(
        d.displayed_text(&standard),
        format!("{DIAMETER_SIGN}0.5000")
    );
}

fn refused(text: &str) -> agentdrafter_core::Error {
    parse_sheet_description(text).unwrap_err()
}

#[test]
fn the_reader_refuses_a_dimension_it_cannot_draw_and_names_the_field() {
    let on = |dimension: &str| format!("units: mm\nobjects:\n  - dimension: {dimension}\n");
    let cases = [
        (
            "{kind: angular, points: [[0, 0, 0], [1, 0, 0]], location: [0, 1, 0]}",
            "objects[0].dimension.kind",
            "angular, ordinate, baseline and continued dimensions wait",
        ),
        (
            "{kind: aligned, angle: 30, points: [[0, 0, 0], [1, 0, 0]], location: [0, 1, 0]}",
            "objects[0].dimension.angle",
            "an aligned dimension takes `points`, not `angle`",
        ),
        (
            "{kind: radius, points: [[0, 0, 0], [1, 0, 0]], location: [0, 1, 0]}",
            "objects[0].dimension.points",
            "takes `center` and `point`",
        ),
        (
            "{kind: linear, points: [[0, 0, 0], [0, 5, 0]], location: [3, 2, 0]}",
            "objects[0].dimension.points",
            "it measures 0",
        ),
        (
            "{kind: linear, points: [[0, 0, 0], [10, 0, 0]], location: [5, 0, 0]}",
            "objects[0].dimension.location",
            "passes through one of the dimension's points",
        ),
        (
            "{kind: linear, points: [[0, 0, 0], [10, 0, 2]], location: [5, 5, 0]}",
            "objects[0].dimension.points",
            "a dimension lies flat",
        ),
        (
            "{kind: radius, center: [0, 0, 0], point: [0, 0, 0], location: [5, 5, 0]}",
            "objects[0].dimension.point",
            "lies apart from its centre",
        ),
        (
            "{kind: radius, center: [0, 0, 0], point: [5, 0, 0], location: [0, 0, 0]}",
            "objects[0].dimension.location",
            "is the circle's centre",
        ),
        (
            "{kind: linear, points: [[0, 0, 0], [10, 0, 0]], location: [5, 5, 0], text: \"A\\\\B\"}",
            "objects[0].dimension.text",
            "holds no `\\`",
        ),
        (
            "{kind: linear, points: [[0, 0, 0], [10, 0, 0]], location: [5, 5, 0], dimension_style: Standard}",
            "objects[0].dimension.dimension_style",
            "unknown dimension style `Standard`; the sheet's dimension styles: ISO-25",
        ),
    ];
    for (dimension, field, says) in cases {
        let e = refused(&on(dimension));
        assert_eq!(e.field.as_deref(), Some(field), "{dimension}: {e:?}");
        assert!(e.message.contains(says), "{dimension}: {}", e.message);
    }
    let style = |keys: &str| format!("units: mm\ndimension_styles:\n  s: {keys}\nobjects: []\n");
    for (keys, field, says) in [
        ("{scale: 0}", "dimension_styles.s.scale", "more than 0"),
        (
            "{unit_format: architectural}",
            "dimension_styles.s.unit_format",
            "architectural",
        ),
        ("{precision: 9}", "dimension_styles.s.precision", "0 to 8"),
        (
            "{prefix: '<'}",
            "dimension_styles.s.prefix",
            "holds `<` or `>`",
        ),
        (
            "{placement: below}",
            "dimension_styles.s.placement",
            "not a placement",
        ),
    ] {
        let e = refused(&style(keys));
        assert_eq!(e.field.as_deref(), Some(field), "{keys}: {e:?}");
        assert!(e.message.contains(says), "{keys}: {}", e.message);
    }
    let e = refused("units: mm\ndimension_styles:\n  iso-25: {}\nobjects: []\n");
    assert!(e.message.contains("list it as `ISO-25`"), "{}", e.message);
}

#[test]
fn a_dimension_belongs_on_a_sheet_not_in_a_block() {
    let library = "units: mm
blocks:
  B:
    objects:
      - dimension: {kind: linear, points: [[0, 0, 0], [1, 0, 0]], location: [0, 1, 0]}
";
    let e = agentdrafter_core::parse_block_library(library).unwrap_err();
    assert_eq!(e.code, ErrorCode::UnknownKey);
    assert!(
        e.message
            .contains("a dimension belongs on a sheet, not in a block"),
        "{}",
        e.message
    );
}

fn built(dir: &Path, text: &str) -> PathBuf {
    let path = dir.join("sheet.yaml");
    fs::write(&path, text).unwrap();
    build(&BuildRequest {
        description: path,
        output: None,
        overwrite: false,
    })
    .unwrap();
    dir.join("sheet.dwg")
}

#[test]
fn dimensions_and_their_styles_read_back_from_the_dwg() {
    let dir = scratch("round-trip");
    let dwg = built(&dir, PLATE);
    let sheet = sheet();
    let read = read_dwg(&dwg).unwrap();
    assert_eq!(read.objects_not_read, 0, "{read:?}");
    assert_eq!(read.dimension_styles_not_read, 0, "{read:?}");
    // Each dimension's own block, and the dot's, are read with them.
    assert!(read.blocks.is_empty(), "{:?}", read.blocks);
    assert_eq!(read.blocks_not_read, 0, "{read:?}");
    // AutoCAD's Defpoints, which the build adds, is read with them too.
    assert!(read.layers.iter().all(|l| l.name != "Defpoints"));
    assert_eq!(read.layers_not_read, 0);
    assert_eq!(read.objects, sheet.objects);
    assert_eq!(read.ids_and_names[2].id.as_deref(), Some("sheet/width"));
    for name in ["ISO-25", "holes", "reference"] {
        let style = read
            .dimension_styles
            .iter()
            .find(|s| s.name == name)
            .unwrap();
        assert_eq!(*style, sheet.dimension_style(name).unwrap(), "{name}");
    }
}

#[test]
fn dimensions_on_an_imperial_sheet_read_back_with_its_style() {
    let dir = scratch("imperial");
    let text = "units: in
objects:
  - circle: {center: [2, 2, 0], radius: 0.25}
  - dimension: {kind: linear, points: [[1, 1, 0], [1.3, 1, 0]], location: [1.15, 0.5, 0]}
  - dimension: {kind: radius, center: [2, 2, 0], point: [2.25, 2, 0], location: [3, 3, 0]}
  - dimension: {kind: radius, center: [2, 2, 0], point: [2.25, 2, 0], location: [3, 2.1, 0]}
";
    let read = read_dwg(&built(&dir, text)).unwrap();
    assert_eq!(read.objects_not_read, 0, "{read:?}");
    assert_eq!(read.objects, parse_sheet_description(text).unwrap().objects);
    assert_eq!(
        read.dimension_styles,
        vec![DimensionStyle::standard(Unit::In)]
    );
}

#[test]
fn a_sheet_with_no_dimensions_gets_no_defpoints_layer() {
    let dir = scratch("none");
    let read = read_dwg(&built(
        &dir,
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [1, 0, 0]}\n",
    ))
    .unwrap();
    assert!(read.layers.iter().all(|l| l.name != "Defpoints"));
    assert!(read.dimension_styles.is_empty());
}

#[test]
fn the_pipeline_draws_a_dimensions_lines_and_its_text_as_its_part() {
    let dir = scratch("drawn");
    let path = dir.join("sheet.yaml");
    fs::write(&path, PLATE).unwrap();
    let report = display_list_with(
        &DisplayListRequest {
            source: path,
            sheet: None,
            tolerance: None,
            entry: None,
        },
        &Settings::default(),
    )
    .unwrap();
    let list = &report.display_list;
    for i in 2..7 {
        assert!(
            list.items
                .iter()
                .any(|item| item.object == i && item.part.is_none()),
            "the lines of object {i}"
        );
        assert!(
            list.text
                .iter()
                .any(|run| run.object == i && run.part == Some(0)),
            "the text of object {i}"
        );
    }
    // The width's text says its value, a gap above its line, at y = 8.
    let width: String = list
        .text
        .iter()
        .filter(|run| run.object == 2)
        .flat_map(|run| run.characters.iter().map(|(c, _)| *c))
        .collect();
    assert_eq!(width, "160");
}

fn check(path: &Path) -> agentdrafter_core::CheckReport {
    check_with(
        &CheckRequest {
            source: path.to_owned(),
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap()
}

#[test]
fn the_checker_counts_a_dimensions_lines_against_other_text_and_its_text_against_other_lines() {
    let dir = scratch("checked");
    let path = dir.join("sheet.yaml");
    // A dimension's own lines and arrowheads never touch its own text.
    fs::write(&path, PLATE).unwrap();
    let report = check(&path);
    assert!(report.passed, "{:#?}", report.findings);
    // A line through the width's text is found against its text.
    let crossed =
        format!("{PLATE}  - line: {{id: through, from: [100, 5, 0], to: [100, 15, 0]}}\n");
    fs::write(&path, &crossed).unwrap();
    let found: Vec<_> = check(&path)
        .findings
        .into_iter()
        .filter(|f| f.rule == Rule::TextOnLine)
        .collect();
    assert_eq!(found.len(), 1, "{found:#?}");
    let text = found[0].object.as_ref().unwrap();
    assert_eq!(text.id, "sheet/width");
    assert_eq!(text.kind, "dimension");
    assert_eq!(text.text.as_deref(), Some("160"));
    assert_eq!(found[0].other.as_ref().unwrap().id, "sheet/through");
    // Its lines through another text are found against that text.
    let under = format!(
        "{PLATE}  - text: {{id: label, contents: XXXXXXXXXX, position: [30, 7, 0], height: 3}}\n"
    );
    fs::write(&path, &under).unwrap();
    let found: Vec<_> = check(&path)
        .findings
        .into_iter()
        .filter(|f| f.rule == Rule::TextOnLine)
        .collect();
    assert!(!found.is_empty());
    assert!(found
        .iter()
        .all(|f| f.object.as_ref().unwrap().id == "sheet/label"
            && f.other.as_ref().unwrap().id == "sheet/width"));
}

#[test]
fn a_query_gives_what_a_dimension_measures_and_shows() {
    let dir = scratch("query");
    let path = dir.join("sheet.yaml");
    fs::write(&path, PLATE).unwrap();
    let report = query_with(
        &QueryRequest {
            source: path,
            sheet: None,
            query: Query::Geometry(agentdrafter_core::Selection {
                id: Some("sheet/hole-size".into()),
                ..Default::default()
            }),
        },
        &Settings::default(),
    )
    .unwrap();
    let Answer::Geometry { objects, .. } = report.answer else {
        panic!()
    };
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].measures.measurement, Some(10.0));
    assert_eq!(objects[0].measures.text.as_deref(), Some("10 mm"));
    assert_eq!(
        report.message,
        "a dimension `sheet/hole-size`, on sheet `sheet`, measures 10 mm and shows `10 mm`"
    );
}

/// A made-up drawing set of one sheet: a plate with a hole on its
/// template, built.
fn plate_set(test: &str) -> PathBuf {
    let set = scratch(test);
    fs::create_dir_all(set.join("templates")).unwrap();
    let write = |name: &str, text: &str| fs::write(set.join(name), text).unwrap();
    write("set.yaml", "units: mm\noutput: dwg\nrows: rows.yaml\n");
    write(
        "rows.yaml",
        "rows:\n  - sheet: PLATE-001\n    template: templates/plate.yaml\n",
    );
    write(
        "templates/plate.yaml",
        "units: mm
text_styles:
  notes: {font: romans.shx}
dimension_styles:
  callout: {text_style: notes, text_height: 3}
objects:
  - polyline: {id: plate, points: [[20, 20, 0], [180, 20, 0], [180, 100, 0], [20, 100, 0]], closed: true}
  - circle: {id: hole, center: [60, 60, 0], radius: 5}
  - dimension: {id: width, kind: linear, points: [[20, 20, 0], [180, 20, 0]], location: [100, 8, 0], dimension_style: callout}
",
    );
    let built = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: false,
        },
        &Settings::default(),
    );
    assert!(built.ok, "{:#?}", built.error);
    set
}

fn apply(set: &Path, operations: Value) -> Reply<ChangeReport> {
    change_with(
        &ChangeRequest {
            set: set.to_owned(),
            change_set: json!({ "operations": operations }),
            dry_run: false,
        },
        &Settings::default(),
    )
}

fn template(set: &Path) -> String {
    fs::read_to_string(set.join("templates/plate.yaml")).unwrap()
}

fn rows(set: &Path) -> String {
    fs::read_to_string(set.join("rows.yaml")).unwrap()
}

#[test]
fn each_kind_is_added_by_its_own_operation_with_autocads_alias() {
    let listed = operations();
    for (name, alias) in [
        ("add_linear_dimension", "DIMLINEAR"),
        ("add_aligned_dimension", "DIMALIGNED"),
        ("add_radius_dimension", "DIMRADIUS"),
        ("add_diameter_dimension", "DIMDIAMETER"),
        ("add_dimension_style", "DIMSTYLE"),
        ("change_dimension_style", ""),
        ("rename_dimension_style", ""),
        ("delete_dimension_style", ""),
    ] {
        let info = listed.operations.iter().find(|o| o.name == name).unwrap();
        assert_eq!(info.alias, alias, "{name}");
    }
    // An operation of a kind takes no kind of its own, and a snapped point
    // wherever it takes a point.
    let info = listed
        .operations
        .iter()
        .find(|o| o.name == "add_radius_dimension")
        .unwrap();
    let validator = jsonschema::validator_for(&info.schema).unwrap();
    let snap = json!({"snap": {"id": "S/hole", "snap": "center"}});
    assert!(validator.is_valid(&json!({
        "op": "DIMRADIUS", "sheet": "S", "center": snap, "point": [1, 0, 0], "location": [2, 2, 0]
    })));
    assert!(!validator.is_valid(&json!({
        "op": "DIMRADIUS", "sheet": "S", "kind": "radius", "center": [0, 0, 0], "point": [1, 0, 0], "location": [2, 2, 0]
    })));
}

#[test]
fn a_change_set_adds_dimensions_at_snapped_points_and_moves_and_changes_them() {
    let set = plate_set("added");
    let reply = apply(
        &set,
        json!([
            {
                "op": "dimlinear", "sheet": "PLATE-001", "id": "height", "angle": 90,
                "points": [
                    {"snap": {"id": "PLATE-001/plate", "snap": "endpoint", "which": "points[1]"}},
                    {"snap": {"id": "PLATE-001/plate", "snap": "endpoint", "which": "points[2]"}}
                ],
                "location": [195, 60, 0]
            },
            {
                "op": "add_diameter_dimension", "sheet": "PLATE-001", "id": "hole-size",
                "center": {"snap": {"id": "PLATE-001/hole", "snap": "center"}},
                "point": {"snap": {"id": "PLATE-001/hole", "snap": "quadrant", "which": "0"}},
                "location": [80, 80, 0], "text": "2X <>"
            },
            {"op": "move", "target": {"id": "PLATE-001/height"}, "by": [5, 0, 0]},
            {"op": "change", "target": {"id": "PLATE-001/width"}, "set": {"text": "<> OVERALL"}}
        ]),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    // The record holds the points the snaps name; the kind comes first.
    let rows = rows(&set);
    for written in [
        "kind: linear",
        "points: [[185, 20, 0], [185, 100, 0]]",
        "location: [200, 60, 0]",
        "kind: diameter",
        "center: [60, 60, 0]",
        "point: [65, 60, 0]",
        "text: 2X <>",
    ] {
        assert!(rows.contains(written), "{written}: {rows}");
    }
    assert!(
        template(&set).contains("text: <> OVERALL"),
        "{}",
        template(&set)
    );
    let read = read_dwg(&set.join("dwg/PLATE-001.dwg")).unwrap();
    assert_eq!(read.objects_not_read, 0, "{read:?}");
    let shown: Vec<String> = read
        .objects
        .iter()
        .filter_map(|o| match o {
            Object::Dimension(d) => {
                let style = read
                    .dimension_styles
                    .iter()
                    .find(|s| s.name == d.dimension_style)
                    .unwrap();
                Some(d.displayed_text(style))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        shown,
        [
            "160 OVERALL",
            "80",
            format!("2X {DIAMETER_SIGN}10").as_str()
        ]
    );
}

#[test]
fn a_templates_dimension_styles_are_added_changed_renamed_and_kept_while_in_use() {
    let set = plate_set("styles");
    let reply = apply(
        &set,
        json!([
            {"op": "DIMSTYLE", "template": "templates/plate.yaml", "name": "fine", "precision": 3},
            {"op": "change_dimension_style", "template": "templates/plate.yaml", "name": "callout", "set": {"arrowhead": "open"}},
            {"op": "rename_dimension_style", "template": "templates/plate.yaml", "name": "callout", "to": "big"},
            {"op": "rename_text_style", "template": "templates/plate.yaml", "name": "notes", "to": "labels"}
        ]),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    let text = template(&set);
    assert!(text.contains("fine: {precision: 3}"), "{text}");
    assert!(
        text.contains("big: {arrowhead: open, text_style: labels, text_height: 3}"),
        "{text}"
    );
    assert!(text.contains("dimension_style: big"), "{text}");
    // A style a dimension draws with, and a text style a dimension style
    // draws with, are kept.
    for (op, name, says) in [
        (
            "delete_dimension_style",
            "big",
            "the object PLATE-001/width",
        ),
        ("delete_text_style", "labels", "the dimension style big"),
        (
            "delete_dimension_style",
            "ISO-25",
            "every drawing's own dimension style",
        ),
    ] {
        let reply = apply(
            &set,
            json!([{"op": op, "template": "templates/plate.yaml", "name": name}]),
        );
        assert!(!reply.ok);
        let result = reply.result.unwrap();
        let e = result.operations[0].error.clone().unwrap();
        assert!(e.message.contains(says), "{op} {name}: {}", e.message);
    }
    let reply = apply(
        &set,
        json!([{"op": "delete_dimension_style", "template": "templates/plate.yaml", "name": "fine"}]),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    assert!(!template(&set).contains("fine:"));
}

#[test]
fn a_library_block_named_like_a_dimensions_arrowhead_but_drawn_otherwise_is_refused() {
    let dir = scratch("library-arrowhead");
    fs::write(
        dir.join("blocks.yaml"),
        "units: mm\nblocks:\n  _Dot:\n    objects:\n      - circle: {center: [0, 0, 0], radius: 3}\n",
    )
    .unwrap();
    let path = dir.join("sheet.yaml");
    fs::write(
        &path,
        "units: mm
block_library: blocks.yaml
dimension_styles:
  dots: {arrowhead: dot}
objects:
  - insert: {block: _Dot, position: [0, 0, 0]}
  - dimension: {kind: linear, points: [[0, 0, 0], [50, 0, 0]], location: [25, 10, 0], dimension_style: dots}
",
    )
    .unwrap();
    let e = build(&BuildRequest {
        description: path,
        output: None,
        overwrite: false,
    })
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::DuplicateName);
    assert!(
        e.message
            .contains("that a leader or a dimension needs, `_Dot`"),
        "{}",
        e.message
    );
}

#[test]
fn changing_autocads_own_style_lists_it_in_the_template() {
    let set = plate_set("own-style");
    let reply = apply(
        &set,
        json!([{"op": "change_dimension_style", "template": "templates/plate.yaml", "name": "ISO-25", "set": {"precision": 1}}]),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    let text = template(&set);
    assert!(text.contains("ISO-25: {precision: 1}"), "{text}");
    let read = read_dwg(&set.join("dwg/PLATE-001.dwg")).unwrap();
    assert_eq!(read.objects_not_read, 0);
}
