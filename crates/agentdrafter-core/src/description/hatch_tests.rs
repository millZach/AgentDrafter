//! Hatch patterns and hatches in a sheet description.

use super::parse_sheet_description;
use crate::error::{Error, ErrorCode};
use crate::model::{
    Hatch, HatchEdge, HatchLoop, HatchPattern, HatchPatternLine, Object, Point3, Sheet,
};

fn sheet(text: &str) -> Sheet {
    parse_sheet_description(text).unwrap_or_else(|e| panic!("{e}"))
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
    assert!(e.line.is_some() && e.column.is_some(), "position of {e:?}");
    for word in words {
        assert!(e.message.contains(word), "`{word}` in {:?}", e.message);
    }
}

/// A sheet in mm with two patterns, STRIPES and DASHED, and one hatch
/// whose keys are `hatch`, written in flow style.
fn with_hatch(hatch: &str) -> String {
    format!(
        "units: mm
hatch_patterns:
  STRIPES: {{lines: [{{delta: [0, 2]}}]}}
  DASHED: {{lines: [{{angle: 45, delta: [0, 3], dashes: [2, -1]}}]}}
objects:
  - hatch: {{{hatch}}}
"
    )
}

const SQUARE: &str =
    "boundary: [{polyline: {points: [[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0]]}}]";

fn the_hatch(text: &str) -> Hatch {
    match sheet(text).objects.as_slice() {
        [Object::Hatch(hatch)] => hatch.clone(),
        other => panic!("expected one hatch, read {other:?}"),
    }
}

fn square() -> Vec<HatchLoop> {
    vec![HatchLoop::Polyline {
        points: vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ],
    }]
}

#[test]
fn a_solid_hatch_reads() {
    assert_eq!(
        the_hatch(&with_hatch(&format!("pattern: SOLID, {SQUARE}"))),
        Hatch {
            pattern: HatchPattern::solid(),
            scale: 1.0,
            angle: 0.0,
            boundary: square(),
            layer: "0".to_owned(),
            properties: Default::default(),
        }
    );
}

/// A pattern's line takes angle 0, origin [0, 0] and no dashes unless it
/// states them, and a hatch takes scale 1 and angle 0. The hatch carries
/// the pattern it names.
#[test]
fn a_patterned_hatch_carries_its_pattern_with_defaults() {
    let hatch = the_hatch(&with_hatch(&format!("pattern: STRIPES, {SQUARE}")));
    assert_eq!(
        hatch.pattern,
        HatchPattern {
            name: "STRIPES".to_owned(),
            lines: vec![HatchPatternLine {
                angle: 0.0,
                origin: [0.0, 0.0],
                delta: [0.0, 2.0],
                dashes: Vec::new(),
            }],
        }
    );
    assert_eq!((hatch.scale, hatch.angle), (1.0, 0.0));

    let hatch = the_hatch(&with_hatch(&format!(
        "pattern: DASHED, scale: 2.5, angle: -90, {SQUARE}"
    )));
    assert_eq!(hatch.pattern.lines[0].angle, 45.0);
    assert_eq!(hatch.pattern.lines[0].dashes, vec![2.0, -1.0]);
    assert_eq!((hatch.scale, hatch.angle), (2.5, 270.0));
}

#[test]
fn every_key_of_a_pattern_line_reads_with_its_units() {
    let hatch = the_hatch(
        "units: in
hatch_patterns:
  P:
    lines:
      - angle: 390
        origin: [25.4 mm, '-0.5']
        delta: [1 ft, 0.25]
        dashes: [0.5, -12.7mm, 0, -0.25]
objects:
  - hatch:
      pattern: P
      boundary:
        - polyline: {points: [[0, 0, 0], [1 ft, 0, 0], [1 ft, 6, 0]]}
",
    );
    assert_eq!(
        hatch.pattern.lines,
        vec![HatchPatternLine {
            angle: 30.0,
            origin: [1.0, -0.5],
            delta: [12.0, 0.25],
            dashes: vec![0.5, -0.5, 0.0, -0.25],
        }]
    );
    let HatchLoop::Polyline { points } = &hatch.boundary[0] else {
        panic!("{hatch:?}")
    };
    assert_eq!(points[1], Point3::new(12.0, 0.0, 0.0));
}

#[test]
fn a_loop_of_lines_and_arcs_reads_as_written() {
    let hatch = the_hatch(&with_hatch(
        "pattern: SOLID, boundary: [{edges: [{line: {from: [-2, 0, 3], to: [2, 0, 3]}}, {arc: {center: [0, 0, 3], radius: 2, start_angle: 0, end_angle: 540}}]}], layer: 0",
    ));
    assert_eq!(
        hatch.boundary,
        vec![HatchLoop::Edges(vec![
            HatchEdge::Line {
                from: Point3::new(-2.0, 0.0, 3.0),
                to: Point3::new(2.0, 0.0, 3.0),
            },
            HatchEdge::Arc {
                center: Point3::new(0.0, 0.0, 3.0),
                radius: 2.0,
                start_angle: 0.0,
                end_angle: 180.0,
            },
        ])]
    );
}

/// The model's JSON says what the sheet description says, in the same keys.
#[test]
fn a_hatch_in_json_takes_the_keys_of_the_sheet_description() {
    let read = sheet(&with_hatch(
        "pattern: DASHED, boundary: [{polyline: {points: [[0, 0, 0], [1, 0, 0], [1, 1, 0]]}}, {edges: [{line: {from: [0, 0, 0], to: [1, 0, 0]}}, {arc: {center: [0.5, 0, 0], radius: 0.5, start_angle: 0, end_angle: 180}}]}]",
    ));
    let json = serde_json::to_value(&read.objects[0]).unwrap();
    assert_eq!(json["type"], "hatch", "{json}");
    assert_eq!(json["pattern"]["name"], "DASHED", "{json}");
    assert_eq!(
        json["pattern"]["lines"][0]["delta"],
        serde_json::json!([0.0, 3.0])
    );
    assert_eq!(json["scale"], 1.0);
    assert_eq!(
        json["boundary"][0]["polyline"]["points"][2],
        serde_json::json!([1.0, 1.0, 0.0])
    );
    assert_eq!(json["boundary"][1]["edges"][1]["arc"]["radius"], 0.5);
    let back: Object = serde_json::from_value(json).unwrap();
    assert_eq!(back, read.objects[0]);
}

#[test]
fn an_unknown_pattern_lists_the_sheets_patterns() {
    let e = error(&with_hatch(&format!("pattern: ANSI31, {SQUARE}")));
    assert_names(
        &e,
        ErrorCode::UnknownName,
        "objects[0].hatch.pattern",
        &[
            "unknown hatch pattern `ANSI31`",
            "the sheet's hatch patterns: SOLID, STRIPES, DASHED",
        ],
    );
    // Names are exact, as layer and linetype names are.
    let e = error(&with_hatch(&format!("pattern: solid, {SQUARE}")));
    assert_names(
        &e,
        ErrorCode::UnknownName,
        "objects[0].hatch.pattern",
        &["`solid`", "SOLID, STRIPES, DASHED"],
    );
    let e = error(&with_hatch(&format!("pattern: [SOLID], {SQUARE}")));
    assert_names(
        &e,
        ErrorCode::WrongType,
        "objects[0].hatch.pattern",
        &["found a list", "SOLID, STRIPES, DASHED"],
    );
    let e = error(&with_hatch(SQUARE));
    assert_names(
        &e,
        ErrorCode::MissingKey,
        "objects[0].hatch.pattern",
        &["`pattern`"],
    );
}

#[test]
fn a_solid_hatch_takes_no_scale_or_angle() {
    for (key, words) in [
        ("scale: 2", ["no pattern to scale", "leave `scale` out"]),
        ("angle: 45", ["no pattern to turn", "leave `angle` out"]),
    ] {
        let e = error(&with_hatch(&format!("pattern: SOLID, {key}, {SQUARE}")));
        let field = format!("objects[0].hatch.{}", key.split(':').next().unwrap());
        assert_names(&e, ErrorCode::BadValue, &field, &words);
    }
}

#[test]
fn a_scale_is_a_number_more_than_zero() {
    for scale in ["0", "-1", "-0"] {
        let e = error(&with_hatch(&format!(
            "pattern: STRIPES, scale: {scale}, {SQUARE}"
        )));
        assert_names(
            &e,
            ErrorCode::BadValue,
            "objects[0].hatch.scale",
            &["more than 0"],
        );
    }
    let e = error(&with_hatch(&format!(
        "pattern: STRIPES, scale: 2 mm, {SQUARE}"
    )));
    assert_names(
        &e,
        ErrorCode::BadNumber,
        "objects[0].hatch.scale",
        &["with no unit"],
    );
    let e = error(&with_hatch(&format!(
        "pattern: STRIPES, angle: 45 deg, {SQUARE}"
    )));
    assert_names(&e, ErrorCode::BadNumber, "objects[0].hatch.angle", &[]);
}

#[test]
fn unknown_keys_list_the_keys_allowed_there() {
    let e = error(&with_hatch(&format!(
        "pattern: STRIPES, colour: red, {SQUARE}"
    )));
    assert_names(
        &e,
        ErrorCode::UnknownKey,
        "objects[0].hatch.colour",
        &["a hatch takes pattern, scale, angle, boundary, layer"],
    );
    let e = error(
        "units: mm\nhatch_patterns:\n  P: {lines: [{delta: [0, 1]}], description: x}\nobjects: []\n",
    );
    assert_names(
        &e,
        ErrorCode::UnknownKey,
        "hatch_patterns.P.description",
        &["a hatch pattern takes lines"],
    );
    let e = error(
        "units: mm\nhatch_patterns:\n  P: {lines: [{delta: [0, 1], offset: [0, 1]}]}\nobjects: []\n",
    );
    assert_names(
        &e,
        ErrorCode::UnknownKey,
        "hatch_patterns.P.lines[0].offset",
        &["a hatch pattern line takes angle, origin, delta, dashes"],
    );
    let e = error(&with_hatch(
        "pattern: SOLID, boundary: [{circle: {center: [0, 0, 0], radius: 1}}]",
    ));
    assert_names(
        &e,
        ErrorCode::UnknownKey,
        "objects[0].hatch.boundary[0].circle",
        &["a loop is one of polyline, edges"],
    );
    let e = error(&with_hatch(
        "pattern: SOLID, boundary: [{polyline: {points: [[0, 0, 0], [1, 0, 0], [1, 1, 0]], closed: true}}]",
    ));
    assert_names(
        &e,
        ErrorCode::UnknownKey,
        "objects[0].hatch.boundary[0].polyline.closed",
        &["a boundary polyline, which is always closed, takes points"],
    );
    let e = error(&with_hatch(
        "pattern: SOLID, boundary: [{edges: [{spline: {}}, {line: {from: [0, 0, 0], to: [1, 0, 0]}}]}]",
    ));
    assert_names(
        &e,
        ErrorCode::UnknownKey,
        "objects[0].hatch.boundary[0].edges[0].spline",
        &["an edge is one of line, arc"],
    );
    let e = error(&with_hatch(
        "pattern: SOLID, boundary: [{edges: [{line: {from: [0, 0, 0], to: [1, 0, 0], layer: 0}}, {line: {from: [1, 0, 0], to: [0, 0, 0]}}]}]",
    ));
    assert_names(
        &e,
        ErrorCode::UnknownKey,
        "objects[0].hatch.boundary[0].edges[0].line.layer",
        &["a line in a boundary takes from, to"],
    );
}

#[test]
fn hatch_pattern_names_follow_autocads_rules() {
    for (name, words) in [
        ("SOLID", "AutoCAD's own hatch `SOLID`"),
        ("solid", "AutoCAD's own hatch `SOLID`"),
        ("_User", "AutoCAD's own hatch `_USER`"),
        (
            "ABCDEFGHIJKLMNOPQRSTUVWXYZ012345678",
            "at most 34 characters",
        ),
        ("TWO WORDS", "cannot hold a space"),
        ("A|B", "cannot hold `|`"),
        // AutoCAD reads a hatch pattern's name in capitals (seen in AutoCAD
        // 2026's reading of a built DWG), so the sheet writes it that way.
        (
            "Fine_mm",
            "written in capitals, as AutoCAD keeps it: `FINE_MM`",
        ),
        ("éclat", "written in capitals, as AutoCAD keeps it: `ÉCLAT`"),
    ] {
        let e = error(&format!(
            "units: mm\nhatch_patterns:\n  '{name}': {{lines: [{{delta: [0, 1]}}]}}\nobjects: []\n"
        ));
        assert_names(
            &e,
            ErrorCode::BadValue,
            &format!("hatch_patterns.{name}"),
            &[words],
        );
    }
    // 34 characters is allowed.
    sheet("units: mm\nhatch_patterns:\n  ABCDEFGHIJKLMNOPQRSTUVWXYZ01234567: {lines: [{delta: [0, 1]}]}\nobjects: []\n");
    let e = error(
        "units: mm\nhatch_patterns:\n  Fine: {lines: [{delta: [0, 1]}]}\n  FINE: {lines: [{delta: [0, 2]}]}\nobjects: []\n",
    );
    assert_names(
        &e,
        ErrorCode::DuplicateKey,
        "hatch_patterns.FINE",
        &[
            "hatch pattern `FINE` is hatch pattern `Fine` again",
            "line 3",
        ],
    );
}

#[test]
fn a_pattern_line_follows_the_rules_of_a_pat_file() {
    let pattern_error = |lines: &str| {
        error(&format!(
            "units: mm\nhatch_patterns:\n  P: {{lines: {lines}}}\nobjects: []\n"
        ))
    };
    assert_names(
        &pattern_error("[]"),
        ErrorCode::WrongType,
        "hatch_patterns.P.lines",
        &["at least one line", "`SOLID`"],
    );
    assert_names(
        &pattern_error("[{angle: 45}]"),
        ErrorCode::MissingKey,
        "hatch_patterns.P.lines[0].delta",
        &["`delta`"],
    );
    assert_names(
        &pattern_error("[{delta: [1, 0]}]"),
        ErrorCode::BadValue,
        "hatch_patterns.P.lines[0].delta",
        &["distance between neighbouring lines", "cannot be 0"],
    );
    assert_names(
        &pattern_error("[{delta: [0, 1, 0]}]"),
        ErrorCode::WrongType,
        "hatch_patterns.P.lines[0].delta",
        &["two lengths [x, y]", "found 3"],
    );
    assert_names(
        &pattern_error("[{delta: [0, 1], origin: 5}]"),
        ErrorCode::WrongType,
        "hatch_patterns.P.lines[0].origin",
        &["two lengths [x, y]"],
    );
    assert_names(
        &pattern_error("[{delta: [0, 1 furlong]}]"),
        ErrorCode::UnknownUnit,
        "hatch_patterns.P.lines[0].delta[1]",
        &["furlong", "mm, cm, m, in, ft"],
    );
    for dashes in ["[1]", "[1, 2]", "[-1, -2]", "[]", "[-1]"] {
        assert_names(
            &pattern_error(&format!("[{{delta: [0, 1], dashes: {dashes}}}]")),
            ErrorCode::BadValue,
            "hatch_patterns.P.lines[0].dashes",
            &[
                "at least one dash or dot",
                "at least one gap",
                "leave `dashes` out",
            ],
        );
    }
    // A gap may come first, and a dot is a dash of 0.
    for dashes in ["[-1, 2]", "[0, -1]", "[1, -1, 0, -1]"] {
        sheet(&format!(
            "units: mm\nhatch_patterns:\n  P: {{lines: [{{delta: [0, 1], dashes: {dashes}}}]}}\nobjects: []\n"
        ));
    }
    assert_names(
        &error("units: mm\nhatch_patterns: [P]\nobjects: []\n"),
        ErrorCode::WrongType,
        "hatch_patterns",
        &["a mapping from hatch pattern names to hatch patterns"],
    );
}

#[test]
fn a_boundary_has_loops_of_the_right_shape() {
    let boundary_error = |boundary: &str| {
        error(&with_hatch(&format!(
            "pattern: SOLID, boundary: {boundary}"
        )))
    };
    assert_names(
        &boundary_error("[]"),
        ErrorCode::WrongType,
        "objects[0].hatch.boundary",
        &["at least one loop"],
    );
    assert_names(
        &boundary_error("{polyline: {points: [[0, 0, 0], [1, 0, 0], [1, 1, 0]]}}"),
        ErrorCode::WrongType,
        "objects[0].hatch.boundary",
        &["a list of loops"],
    );
    assert_names(
        &boundary_error("[{polyline: {points: [[0, 0, 0], [1, 0, 0]]}}]"),
        ErrorCode::WrongType,
        "objects[0].hatch.boundary[0].polyline.points",
        &["at least 3 points, found 2"],
    );
    assert_names(
        &boundary_error("[{polyline: {points: [[0, 0, 0], [1, 0, 0], [1, 1, 0]]}, edges: []}]"),
        ErrorCode::WrongType,
        "objects[0].hatch.boundary[0]",
        &["a mapping with one key: polyline or edges"],
    );
    assert_names(
        &boundary_error("[[0, 0, 0]]"),
        ErrorCode::WrongType,
        "objects[0].hatch.boundary[0]",
        &["polyline or edges"],
    );
    assert_names(
        &boundary_error("[{edges: [{line: {from: [0, 0, 0], to: [1, 0, 0]}}]}]"),
        ErrorCode::WrongType,
        "objects[0].hatch.boundary[0].edges",
        &["at least 2 edges; found 1"],
    );
    assert_names(
        &boundary_error("[{edges: [{line: {from: [0, 0, 0], to: [1, 0, 0]}, arc: {center: [0, 0, 0], radius: 1, start_angle: 0, end_angle: 90}}, {line: {from: [1, 0, 0], to: [0, 0, 0]}}]}]"),
        ErrorCode::WrongType,
        "objects[0].hatch.boundary[0].edges[0]",
        &["a mapping with one key: line or arc"],
    );
}

/// A hatch lies flat: the error names the z that leaves the plane.
#[test]
fn a_boundary_lies_flat() {
    let e = error(&with_hatch(
        "pattern: SOLID, boundary: [{polyline: {points: [[0, 0, 1], [1, 0, 1], [1, 1, 1]]}}, {polyline: {points: [[0.2, 0.2, 1], [0.4, 0.2, 2], [0.4, 0.4, 1]]}}]",
    ));
    assert_names(
        &e,
        ErrorCode::BadValue,
        "objects[0].hatch.boundary[1].polyline.points[1][2]",
        &["lies flat", "the z of the first, 1; this one has 2"],
    );
    let e = error(&with_hatch(
        "pattern: SOLID, boundary: [{edges: [{line: {from: [0, 0, 0], to: [2, 0, 0]}}, {arc: {center: [1, 0, 0.5], radius: 1, start_angle: 0, end_angle: 180}}]}]",
    ));
    assert_names(
        &e,
        ErrorCode::BadValue,
        "objects[0].hatch.boundary[0].edges[1].arc.center[2]",
        &["lies flat"],
    );
}

#[test]
fn the_edges_of_a_loop_join_end_to_end_and_close() {
    let edges_error = |edges: &str| {
        error(&with_hatch(&format!(
            "pattern: SOLID, boundary: [{{edges: [{edges}]}}]"
        )))
    };
    // A line starts where the edge before it ends.
    let e = edges_error(
        "{line: {from: [0, 0, 0], to: [4, 0, 0]}}, {line: {from: [4, 1, 0], to: [0, 4, 0]}}, {line: {from: [0, 4, 0], to: [0, 0, 0]}}",
    );
    assert_names(
        &e,
        ErrorCode::BadValue,
        "objects[0].hatch.boundary[0].edges[1]",
        &[
            "each line in a loop starts where the edge before it ends, at [4, 0, 0]",
            "this one starts at [4, 1, 0]",
        ],
    );
    // A line written backwards does not join.
    let e = edges_error(
        "{line: {from: [0, 0, 0], to: [4, 0, 0]}}, {line: {from: [0, 4, 0], to: [4, 0, 0]}}, {line: {from: [0, 4, 0], to: [0, 0, 0]}}",
    );
    assert_names(
        &e,
        ErrorCode::BadValue,
        "objects[0].hatch.boundary[0].edges[1]",
        &["starts where the edge before it ends"],
    );
    // An arc may run either way, but must touch the edge before it.
    let e = edges_error(
        "{line: {from: [0, 0, 0], to: [4, 0, 0]}}, {arc: {center: [4, 3, 0], radius: 2, start_angle: 270, end_angle: 90}}, {line: {from: [4, 5, 0], to: [0, 0, 0]}}",
    );
    assert_names(
        &e,
        ErrorCode::BadValue,
        "objects[0].hatch.boundary[0].edges[1]",
        &[
            "each arc in a loop starts or ends where the edge before it ends, at [4, 0, 0]",
            "this one runs from",
        ],
    );
    // After a first arc, the walk could be at either of its ends.
    let e = edges_error(
        "{arc: {center: [0, 0, 0], radius: 1, start_angle: 0, end_angle: 180}}, {line: {from: [0, 5, 0], to: [1, 0, 0]}}",
    );
    assert_names(
        &e,
        ErrorCode::BadValue,
        "objects[0].hatch.boundary[0].edges[1]",
        &[" or [1, 0, 0]", "this one starts at [0, 5, 0]"],
    );
    // The last edge ends where the first starts.
    let e = edges_error(
        "{line: {from: [0, 0, 0], to: [4, 0, 0]}}, {line: {from: [4, 0, 0], to: [4, 4, 0]}}, {line: {from: [4, 4, 0], to: [0, 4, 0]}}",
    );
    assert_names(
        &e,
        ErrorCode::BadValue,
        "objects[0].hatch.boundary[0].edges[2]",
        &[
            "a loop closes: its last edge ends at [0, 4, 0], but its first edge starts at [0, 0, 0]",
        ],
    );
    // An arc's ends meet a line's though the arc's are worked out from its angles.
    sheet(&with_hatch(
        "pattern: SOLID, boundary: [{edges: [{line: {from: [0, 0, 0], to: [4, 0, 0]}}, {arc: {center: [4, 3, 0], radius: 3, start_angle: 270, end_angle: 90}}, {line: {from: [4, 6, 0], to: [0, 0, 0]}}]}]",
    ));
}

#[test]
fn the_edges_of_a_loop_are_checked() {
    let edges_error = |edges: &str| {
        error(&with_hatch(&format!(
            "pattern: SOLID, boundary: [{{edges: [{edges}]}}]"
        )))
    };
    let e = edges_error(
        "{line: {from: [1, 0, 0], to: [1, 0, 0]}}, {line: {from: [1, 0, 0], to: [0, 0, 0]}}",
    );
    assert_names(
        &e,
        ErrorCode::BadValue,
        "objects[0].hatch.boundary[0].edges[0].line.to",
        &["`to` differs from `from`"],
    );
    let e = edges_error(
        "{arc: {center: [0, 0, 0], radius: 0, start_angle: 0, end_angle: 180}}, {line: {from: [0, 0, 0], to: [0, 0, 0]}}",
    );
    assert_names(
        &e,
        ErrorCode::BadValue,
        "objects[0].hatch.boundary[0].edges[0].arc.radius",
        &["more than 0"],
    );
    let e = edges_error(
        "{arc: {center: [0, 0, 0], radius: 1, start_angle: 90, end_angle: 450}}, {line: {from: [0, 1, 0], to: [0, 0, 0]}}",
    );
    assert_names(
        &e,
        ErrorCode::BadValue,
        "objects[0].hatch.boundary[0].edges[0].arc.end_angle",
        &[
            "must differ from its start angle",
            "a whole circle in a boundary is two arcs",
        ],
    );
    let e = edges_error(
        "{arc: {center: [0, 0, 0], start_angle: 0, end_angle: 180}}, {line: {from: [-1, 0, 0], to: [1, 0, 0]}}",
    );
    assert_names(
        &e,
        ErrorCode::MissingKey,
        "objects[0].hatch.boundary[0].edges[0].arc.radius",
        &["`radius`"],
    );
}

/// Keys are spelled as AutoCAD spells them: `pattern`, `scale`, `angle`.
#[test]
fn hatch_keys_are_spelled_as_autocad_spells_them() {
    let e = error(&with_hatch(&format!(
        "pattern: STRIPES, angel: 5, {SQUARE}"
    )));
    assert_names(
        &e,
        ErrorCode::UnknownKey,
        "objects[0].hatch.angel",
        &["pattern, scale, angle, boundary, layer"],
    );
    let e = error(&with_hatch(&format!(
        "pattern: STRIPES, layer: walls, {SQUARE}"
    )));
    assert_names(
        &e,
        ErrorCode::UnknownName,
        "objects[0].hatch.layer",
        &["unknown layer `walls`", "the sheet's layers: 0"],
    );
}
