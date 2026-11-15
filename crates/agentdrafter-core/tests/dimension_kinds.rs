//! Angular and ordinate dimensions, unit formats, and dimensions added from
//! others: the reader takes them and keeps an angular dimension's location
//! where AutoCAD keeps it; the DWG reads back the same dimensions and
//! styles; a query gives an angle in degrees and the text a style shows;
//! their arcs snap; the checker tells their lines from their letters; and
//! change sets add them, and baseline and continued dimensions from them.
//! Every dimension here is made up. AutoCAD 2026 opened, audited, laid out
//! again and plotted what these write (see the pull request).

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build, build_set_with, change_with, check_with, operations, parse_sheet_description,
    query_with, read_dwg, AngularUnit, Answer, BuildRequest, ChangeReport, ChangeRequest,
    CheckRequest, Dimension, FractionFormat, Measured, Object, OrdinateAxis, Point3, Query,
    QueryRequest, Reply, Rule, SetBuildRequest, Settings, SnapKind, SnapRef, UnitFormat,
};
use serde_json::{json, Value};

fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-dimension-kinds")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A made-up bracket: its angled arm, a hole pattern dimensioned by
/// ordinates, and its angles, in ISO-25 and in styles of its own.
const BRACKET: &str = "units: mm
paper: [240, 160]
dimension_styles:
  dms:
    angular_unit: degrees_minutes_seconds
    angular_precision: 4
  grads:
    angular_unit: gradians
    angular_precision: 2
    suppress_angular_trailing_zeros: true
  fractions:
    unit_format: fractional
    precision: 4
    fraction_format: diagonal
  tight:
    placement: centered
    baseline_spacing: 5
objects:
  - polyline: {points: [[0, 0, 0], [240, 0, 0], [240, 160, 0], [0, 160, 0]], closed: true}
  - line: {id: base, from: [20, 20, 0], to: [120, 20, 0]}
  - line: {id: arm, from: [20, 20, 0], to: [90, 90, 0]}
  - dimension: {id: arm-angle, kind: angular, lines: [[[20, 20, 0], [120, 20, 0]], [[20, 20, 0], [90, 90, 0]]], location: [70, 35, 0]}
  - dimension: {id: arm-dms, kind: three_point_angular, center: [20, 20, 0], points: [[120, 20, 0], [90, 90, 0]], location: [80, 50, 0], dimension_style: dms}
  - dimension: {id: narrow, kind: three_point_angular, center: [150, 20, 0], points: [[190, 20, 0], [190, 23, 0]], location: [180, 21, 0], dimension_style: grads}
  - dimension: {id: hole-x, kind: ordinate, feature: [130, 60, 0], axis: x, location: [140, 90, 0]}
  - dimension: {id: hole-y, kind: ordinate, feature: [130, 60, 0], axis: y, location: [170, 60, 0], dimension_style: tight}
  - dimension: {id: slot, kind: linear, points: [[140, 100, 0], [165.6875, 100, 0]], location: [150, 110, 0], dimension_style: fractions}
  - dimension: {id: slot-next, kind: linear, points: [[165.6875, 100, 0], [215, 100, 0]], location: [190, 110, 0], continued: true}
";

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

fn dimensions(objects: &[Object]) -> Vec<&Dimension> {
    objects
        .iter()
        .filter_map(|o| match o {
            Object::Dimension(d) => Some(d),
            _ => None,
        })
        .collect()
}

fn close(a: Point3, b: [f64; 3]) {
    assert!(
        (a.x - b[0]).abs() < 1e-9 && (a.y - b[1]).abs() < 1e-9 && (a.z - b[2]).abs() < 1e-9,
        "{a:?} is not {b:?}"
    );
}

#[test]
fn the_reader_takes_angular_and_ordinate_dimensions_and_the_styles_unit_formats() {
    let sheet = parse_sheet_description(BRACKET).unwrap();
    let d = dimensions(&sheet.objects);
    assert!(matches!(d[0].measured, Measured::Angular { .. }));
    // An angular dimension's location is kept a third of the way round its
    // arc, its distance from the vertex the arc's radius.
    let r = (50.0f64).hypot(15.0);
    let at = 15.0f64.to_radians();
    close(
        d[0].location,
        [20.0 + r * at.cos(), 20.0 + r * at.sin(), 0.0],
    );
    assert!((d[0].measurement() - 45.0).abs() < 1e-12);
    assert!(matches!(
        d[3].measured,
        Measured::Ordinate {
            axis: OrdinateAxis::X,
            ..
        }
    ));
    assert_eq!(d[3].measurement(), 130.0);
    assert!(d[6].continued && !d[5].continued);
    let fractions = sheet.dimension_style("fractions").unwrap();
    assert_eq!(fractions.unit_format, UnitFormat::Fractional);
    assert_eq!(fractions.fraction_format, FractionFormat::Diagonal);
    assert_eq!(d[5].displayed_text(&fractions), "25 11/16");
    let dms = sheet.dimension_style("dms").unwrap();
    assert_eq!(dms.angular_unit, AngularUnit::DegreesMinutesSeconds);
    assert_eq!(d[1].displayed_text(&dms), "45\u{b0}0'0\"");
    let grads = sheet.dimension_style("grads").unwrap();
    assert_eq!(d[2].displayed_text(&grads), "4,77g");
    assert_eq!(
        sheet.dimension_style("tight").unwrap().baseline_spacing,
        5.0
    );
}

#[test]
fn an_inch_unit_format_is_for_a_sheet_in_inches() {
    let on = |units: &str| {
        format!(
            "units: {units}\ndimension_styles:\n  s: {{unit_format: engineering}}\nobjects: []\n"
        )
    };
    let e = parse_sheet_description(&on("mm")).unwrap_err();
    assert_eq!(e.field.as_deref(), Some("dimension_styles.s.unit_format"));
    assert!(
        e.message
            .contains("for a sheet in `in`; this sheet is in `mm`"),
        "{}",
        e.message
    );
    assert!(parse_sheet_description(&on("in")).is_ok());
    let schema = agentdrafter_core::sheet_description_schema();
    let validator = jsonschema::validator_for(&schema).unwrap();
    for (units, accepted) in [("mm", false), ("ft", false), ("in", true)] {
        let sheet = json!({
            "units": units,
            "dimension_styles": {"s": {"unit_format": "architectural"}},
            "objects": []
        });
        assert_eq!(validator.is_valid(&sheet), accepted, "{units}");
    }
}

#[test]
fn angular_and_ordinate_dimensions_and_their_styles_read_back_from_the_dwg() {
    let dir = scratch("round-trip");
    let dwg = built(&dir, BRACKET);
    let sheet = parse_sheet_description(BRACKET).unwrap();
    let read = read_dwg(&dwg).unwrap();
    assert_eq!(read.objects_not_read, 0, "{read:?}");
    assert_eq!(read.dimension_styles_not_read, 0, "{read:?}");
    assert!(read.blocks.is_empty(), "{:?}", read.blocks);
    // An angular dimension whose text is outside, which AutoCAD keeps two
    // thirds of the way round its arc, reads back to its third within
    // rounding; every other point exactly.
    assert_eq!(read.objects.len(), sheet.objects.len());
    for (a, b) in read.objects.iter().zip(&sheet.objects) {
        match (a, b) {
            (Object::Dimension(a), Object::Dimension(b)) if a.kind().is_angular() => {
                let off = (a.location.x - b.location.x).hypot(a.location.y - b.location.y);
                assert!(off < 1e-12, "{a:?} {b:?}");
                let a = Dimension {
                    location: b.location,
                    ..a.clone()
                };
                assert_eq!(a, *b);
            }
            _ => assert_eq!(a, b),
        }
    }
    for name in ["ISO-25", "dms", "grads", "fractions", "tight"] {
        let style = read
            .dimension_styles
            .iter()
            .find(|s| s.name == name)
            .unwrap();
        assert_eq!(*style, sheet.dimension_style(name).unwrap(), "{name}");
    }
}

#[test]
fn architectural_and_engineering_dimensions_read_back_from_an_imperial_dwg() {
    let dir = scratch("imperial");
    let text = "units: in
dimension_styles:
  arch:
    unit_format: architectural
    precision: 4
    suppress_zero_inches: false
  eng:
    unit_format: engineering
    precision: 2
    suppress_zero_feet: false
    suppress_trailing_zeros: true
  dms:
    angular_unit: degrees_minutes_seconds
    angular_precision: 3
objects:
  - dimension: {kind: linear, points: [[0, 0, 0], [12.5, 0, 0]], location: [6, 1, 0], dimension_style: arch}
  - dimension: {kind: linear, points: [[0, 3, 0], [8.25, 3, 0]], location: [4, 4, 0], dimension_style: eng}
  - dimension: {kind: three_point_angular, center: [20, 0, 0], points: [[24, 0, 0], [22, 3, 0]], location: [23, 1, 0], dimension_style: dms}
  - dimension: {kind: ordinate, feature: [31.5, 2, 0], axis: x, location: [32, 5, 0], dimension_style: arch}
";
    let sheet = parse_sheet_description(text).unwrap();
    let d = dimensions(&sheet.objects);
    let style = |name: &str| sheet.dimension_style(name).unwrap();
    assert_eq!(d[0].displayed_text(&style("arch")), "1'-0 1/2\"");
    assert_eq!(d[1].displayed_text(&style("eng")), "0'-8.25\"");
    assert_eq!(d[3].displayed_text(&style("arch")), "2'-7 1/2\"");
    let read = read_dwg(&built(&dir, text)).unwrap();
    assert_eq!(read.objects_not_read, 0, "{read:?}");
    assert_eq!(read.dimension_styles_not_read, 0, "{read:?}");
    assert_eq!(read.objects, sheet.objects);
    let dms = read
        .dimension_styles
        .iter()
        .find(|s| s.name == "dms")
        .unwrap();
    assert_eq!(dms.angular_precision, 3);
}

#[test]
fn a_query_gives_an_angle_in_degrees_and_the_text_its_style_shows() {
    let dir = scratch("query");
    let path = dir.join("sheet.yaml");
    fs::write(&path, BRACKET).unwrap();
    let ask = |id: &str| {
        let report = query_with(
            &QueryRequest {
                source: path.clone(),
                sheet: None,
                query: Query::Geometry(agentdrafter_core::Selection {
                    id: Some(id.into()),
                    ..Default::default()
                }),
            },
            &Settings::default(),
        )
        .unwrap();
        let Answer::Geometry { objects, .. } = report.answer else {
            panic!()
        };
        (
            objects[0].measures.measurement.unwrap(),
            objects[0].measures.text.clone().unwrap(),
        )
    };
    let (angle, text) = ask("sheet/arm-dms");
    assert!((angle - 45.0).abs() < 1e-12);
    assert_eq!(text, "45\u{b0}0'0\"");
    let (x, text) = ask("sheet/hole-x");
    assert_eq!((x, text.as_str()), (130.0, "130"));
}

#[test]
fn an_angular_dimensions_arc_snaps_to_its_vertex_and_ends() {
    let dir = scratch("snap");
    let path = dir.join("sheet.yaml");
    fs::write(&path, BRACKET).unwrap();
    let report = query_with(
        &QueryRequest {
            source: path,
            sheet: None,
            query: Query::Snap(SnapRef {
                id: Some("sheet/arm-angle".into()),
                snap: Some(SnapKind::Center),
                ..SnapRef::default()
            }),
        },
        &Settings::default(),
    )
    .unwrap();
    let Answer::Snap { points, .. } = &report.answer else {
        panic!("{report:#?}")
    };
    assert!(!points.is_empty());
    for point in points {
        close(point.at, [20.0, 20.0, 0.0]);
    }
}

#[test]
fn the_checker_counts_an_angular_dimensions_arc_against_other_text_but_not_its_own() {
    let dir = scratch("checked");
    let path = dir.join("sheet.yaml");
    fs::write(&path, BRACKET).unwrap();
    let check = || {
        check_with(
            &CheckRequest {
                source: path.clone(),
                sheets: Vec::new(),
            },
            &Settings::default(),
        )
        .unwrap()
    };
    let report = check();
    assert!(report.passed, "{:#?}", report.findings);
    // A label laid across the arc is found against the dimension.
    let r = (50.0f64).hypot(15.0);
    let crossed = format!(
        "{BRACKET}  - text: {{id: label, contents: XXXXXXXX, position: [{}, {}, 0], height: 2.5}}\n",
        20.0 + r - 8.0,
        28.0
    );
    fs::write(&path, &crossed).unwrap();
    let found: Vec<_> = check()
        .findings
        .into_iter()
        .filter(|f| f.rule == Rule::TextOnLine)
        .collect();
    assert!(!found.is_empty());
    assert!(found
        .iter()
        .all(|f| f.object.as_ref().unwrap().id == "sheet/label"
            && f.other.as_ref().unwrap().id == "sheet/arm-angle"));
}

/// A made-up drawing set of one sheet: the bracket's template, built.
fn bracket_set(test: &str) -> PathBuf {
    let set = scratch(test);
    fs::create_dir_all(set.join("templates")).unwrap();
    let write = |name: &str, text: &str| fs::write(set.join(name), text).unwrap();
    write("set.yaml", "units: mm\noutput: dwg\nrows: rows.yaml\n");
    write(
        "rows.yaml",
        "rows:\n  - sheet: BRACKET-001\n    template: templates/bracket.yaml\n",
    );
    write(
        "templates/bracket.yaml",
        "units: mm
objects:
  - line: {id: base, from: [20, 20, 0], to: [120, 20, 0]}
  - line: {id: arm, from: [20, 20, 0], to: [90, 90, 0]}
  - circle: {id: hole, center: [130, 60, 0], radius: 4}
  - dimension: {id: width, kind: linear, points: [[20, 20, 0], [60, 20, 0]], location: [40, 10, 0]}
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

#[test]
fn each_new_kind_is_added_by_its_own_operation_with_autocads_alias() {
    let listed = operations();
    for (name, alias) in [
        ("add_angular_dimension", "DIMANGULAR"),
        ("add_three_point_angular_dimension", ""),
        ("add_ordinate_dimension", "DIMORDINATE"),
        ("add_baseline_dimension", "DIMBASELINE"),
        ("add_continued_dimension", "DIMCONTINUE"),
    ] {
        let info = listed.operations.iter().find(|o| o.name == name).unwrap();
        assert_eq!(info.alias, alias, "{name}");
    }
    let schema_of = |name: &str| {
        let info = listed.operations.iter().find(|o| o.name == name).unwrap();
        jsonschema::validator_for(&info.schema).unwrap()
    };
    let snap = json!({"snap": {"id": "S/hole", "snap": "center"}});
    for (op, keys, accepted) in [
        (
            "add_angular_dimension",
            json!({"lines": [[[0, 0, 0], snap], [[0, 0, 0], [1, 1, 0]]], "location": [2, 1, 0]}),
            true,
        ),
        (
            "add_angular_dimension",
            json!({"points": [[0, 0, 0], [1, 1, 0]], "location": [2, 1, 0]}),
            false,
        ),
        (
            "add_three_point_angular_dimension",
            json!({"center": snap, "points": [[1, 0, 0], [0, 1, 0]], "location": [2, 1, 0]}),
            true,
        ),
        (
            "add_ordinate_dimension",
            json!({"feature": snap, "axis": "y", "location": [9, 0, 0]}),
            true,
        ),
        (
            "add_ordinate_dimension",
            json!({"feature": snap, "location": [9, 0, 0]}),
            false,
        ),
        (
            "add_baseline_dimension",
            json!({"target": {"id": "S/width"}, "point": snap}),
            true,
        ),
        (
            "add_continued_dimension",
            json!({"target": {"id": "S/width"}}),
            false,
        ),
    ] {
        let mut request = keys.as_object().unwrap().clone();
        request.insert("op".into(), json!(op));
        if !op.contains("baseline") && !op.contains("continued") {
            request.insert("sheet".into(), json!("S"));
        }
        let request = Value::Object(request);
        assert_eq!(schema_of(op).is_valid(&request), accepted, "{request}");
    }
}

#[test]
fn a_change_set_adds_angles_ordinates_and_baseline_and_continued_dimensions() {
    let set = bracket_set("added");
    let reply = apply(
        &set,
        json!([
            {
                "op": "dimangular", "sheet": "BRACKET-001", "id": "arm-angle",
                "lines": [
                    [{"snap": {"id": "BRACKET-001/base", "snap": "endpoint", "which": "from"}}, [120, 20, 0]],
                    [[20, 20, 0], {"snap": {"id": "BRACKET-001/arm", "snap": "endpoint", "which": "to"}}]
                ],
                "location": [70, 35, 0]
            },
            {
                "op": "dimordinate", "sheet": "BRACKET-001", "id": "hole-x", "axis": "x",
                "feature": {"snap": {"id": "BRACKET-001/hole", "snap": "center"}},
                "location": [130, 90, 0]
            },
            {"op": "dimbaseline", "target": {"id": "BRACKET-001/width"}, "point": [90, 20, 0], "id": "overall"},
            {"op": "dimcontinue", "target": {"id": "BRACKET-001/width"}, "point": [80, 20, 0], "id": "next"},
            {
                "op": "dimcontinue", "target": {"id": "BRACKET-001/hole-x"}, "id": "edge-x",
                "point": {"snap": {"id": "BRACKET-001/base", "snap": "endpoint", "which": "to"}}
            }
        ]),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    let rows = fs::read_to_string(set.join("rows.yaml")).unwrap();
    let template = fs::read_to_string(set.join("templates/bracket.yaml")).unwrap();
    for written in [
        "kind: angular",
        "lines: [[[20, 20, 0], [120, 20, 0]], [[20, 20, 0], [90, 90, 0]]]",
        "kind: ordinate",
        "feature: [130, 60, 0]",
    ] {
        assert!(rows.contains(written), "{written}: {rows}");
    }
    // A dimension from one of the template's is written in the template:
    // a baseline further out, a continued one on the same line, marked.
    for written in [
        "      points: [[20, 20, 0], [90, 20, 0]]\n      location: [60, 6.25, 0]\n",
        "      points: [[60, 20, 0], [80, 20, 0]]\n      location: [60, 10, 0]\n",
        "      continued: true\n      id: next\n",
    ] {
        assert!(template.contains(written), "{written}: {template}");
    }
    // An ordinate's continued one ends level with it.
    let ordinate = "feature: [120, 20, 0]\n          axis: x\n          location: [120, 90, 0]\n";
    assert!(rows.contains(ordinate), "{rows}");
    let read = read_dwg(&set.join("dwg/BRACKET-001.dwg")).unwrap();
    assert_eq!(read.objects_not_read, 0, "{read:?}");
    let shown: Vec<String> = dimensions(&read.objects)
        .iter()
        .map(|d| {
            let style = read
                .dimension_styles
                .iter()
                .find(|s| s.name == d.dimension_style)
                .unwrap();
            d.displayed_text(style)
        })
        .collect();
    assert_eq!(shown, ["40", "70", "20", "45\u{b0}", "130", "120"]);
    // A dimension is not added from a radius.
    let refused = apply(
        &set,
        json!([
            {"op": "add_radius_dimension", "sheet": "BRACKET-001", "id": "r", "center": [130, 60, 0], "point": [134, 60, 0], "location": [140, 70, 0]},
            {"op": "dimbaseline", "target": {"id": "BRACKET-001/r"}, "point": [0, 0, 0]}
        ]),
    );
    let e = refused.error.unwrap();
    assert!(e.message.contains("radius or diameter"), "{}", e.message);
}
