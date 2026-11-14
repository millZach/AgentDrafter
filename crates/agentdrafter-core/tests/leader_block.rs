//! A multileader's block content: the reader takes a block of the sheet's
//! library with its attribute values and refuses what the model cannot
//! place; the one pipeline draws the block where AutoCAD 2026 put its own;
//! the DWG reads back the same content; the checker treats it as an
//! insert; and a template takes an attribute value from its row. Every
//! block here is made up.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use acadrust::entities::{EntityType, MultiLeader};
use acadrust::{CadDocument, DwgReader, DwgWriter};
use agentdrafter_core::{
    build, build_set_with, check_with, display_list_with, read_dwg, read_sheet_description,
    Attribute, BlockConnection, Bounds, BuildRequest, CheckRequest, DisplayListRequest, ErrorCode,
    MLeader, MLeaderBlock, MLeaderContent, Object, Rule, SetBuildRequest, Settings, Sheet,
};

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-leader-block")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The blocks AutoCAD 2026 made its own block content of, and two more: a
/// block that defines a tag twice, and blocks the model cannot place.
const LIBRARY: &str = "units: mm
blocks:
  BOXA:
    objects:
      - polyline: {points: [[5, -3, 0], [25, -3, 0], [25, 9, 0], [5, 9, 0]], closed: true}
      - attdef: {tag: TAG, prompt: Tag, default: '?', position: [15, 3, 0], height: 3, justify: middle_center}
  CIRC:
    objects:
      - circle: {center: [30, 20, 0], radius: 6}
      - attdef: {tag: NUM, prompt: Num, default: '0', position: [30, 20, 0], height: 2.5, justify: middle_center}
  NOTES:
    objects:
      - line: {from: [0, 0, 0], to: [10, 0, 0]}
      - attdef: {tag: N, prompt: N, default: n, position: [0, 2, 0], height: 1.5}
      - attdef: {tag: N, prompt: N, default: n, position: [0, 4, 0], height: 1.5}
  LABEL:
    objects:
      - circle: {center: [0, 0, 0], radius: 3}
      - text: {contents: X, position: [0, 0, 0], height: 2}
  CURVE:
    objects:
      - spline: {fit_points: [[0, 0, 0], [5, 5, 0], [10, 0, 0]]}
  DEGREES:
    objects:
      - circle: {center: [0, 0, 0], radius: 5}
      - attdef: {tag: T, prompt: T, default: '%%d', position: [0, 0, 0], height: 2.5, justify: middle_center}
  FAR:
    base_point: [1e12, 1e12, 0]
    objects:
      - circle: {center: [1e12, 1e12, 0], radius: 5}
  SPOT:
    objects:
      - polyline: {points: [{point: [0, 0, 0], bulge: 1}, [0, 0, 0]]}
";

/// Block content on both sides, by both connections, scaled and turned,
/// with attribute values, as AutoCAD 2026 made its own: the landing is
/// Standard's 8 mm.
const SHEET: &str = "units: mm
block_library: blocks.yaml
objects:
  - mleader: {id: extents-right, leaders: [[[100, 0, 0], [130, 20, 0]]], content: {block: BOXA, attributes: {TAG: R1}}}
  - mleader: {id: extents-left, leaders: [[[100, -60, 0], [70, -40, 0]]], content: {block: CIRC, attributes: {NUM: '4'}}}
  - mleader: {id: insertion-right, leaders: [[[100, -200, 0], [130, -180, 0]]], content: {block: BOXA, connection: insertion}}
  - mleader: {id: insertion-left, leaders: [[[300, -260, 0], [270, -240, 0]]], content: {block: CIRC, connection: insertion}}
  - mleader: {id: turned, leaders: [[[100, -460, 0], [70, -440, 0]]], content: {block: BOXA, scale: 2, rotation: 30, attributes: {TAG: L7}}}
  - mleader: {id: twice, leaders: [[[300, 0, 0], [330, 20, 0]], [[300, 40, 0], [330, 20, 0]]], arrowhead: dot, content: {block: NOTES, attributes: {N: [FIRST, SECOND]}}}
";

/// Writes the library and `sheet` into `dir`, and gives the sheet's path.
fn write(dir: &Path, sheet: &str) -> PathBuf {
    fs::write(dir.join("blocks.yaml"), LIBRARY).unwrap();
    let path = dir.join("sheet.yaml");
    fs::write(&path, sheet).unwrap();
    path
}

fn content(sheet: &Sheet, index: usize) -> &MLeaderBlock {
    match &sheet.objects[index] {
        Object::MLeader(MLeader {
            content: MLeaderContent::Block(block),
            ..
        }) => block,
        other => panic!("{other:?}"),
    }
}

#[test]
fn block_content_reads_with_its_attribute_values() {
    let dir = scratch("read");
    let sheet = read_sheet_description(&write(&dir, SHEET)).unwrap();
    let boxa = content(&sheet, 0);
    assert_eq!(boxa.block, "BOXA");
    assert_eq!(
        boxa.attributes,
        [Attribute {
            tag: "TAG".into(),
            value: "R1".into()
        }]
    );
    assert_eq!((boxa.scale, boxa.rotation), (1.0, 0.0));
    assert_eq!(boxa.connection, BlockConnection::Extents);
    // An attribute left out takes its definition's default.
    assert_eq!(content(&sheet, 2).attributes[0].value, "?");
    assert_eq!(content(&sheet, 2).connection, BlockConnection::Insertion);
    assert_eq!(
        (content(&sheet, 4).scale, content(&sheet, 4).rotation),
        (2.0, 30.0)
    );
    let values: Vec<&str> = content(&sheet, 5)
        .attributes
        .iter()
        .map(|a| a.value.as_str())
        .collect();
    assert_eq!(values, ["FIRST", "SECOND"]);
    // The sheet holds the blocks its block content draws.
    let mut names: Vec<&str> = sheet.blocks.iter().map(|b| b.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["BOXA", "CIRC", "NOTES"]);
}

#[test]
fn the_reader_refuses_block_content_it_cannot_place_and_names_the_field() {
    let dir = scratch("refused");
    let one = |content: &str, rest: &str| {
        format!(
            "units: mm\nblock_library: blocks.yaml\nobjects:\n  - mleader: {{leaders: [[[0, 0, 0], [20, 10, 0]]], content: {content}{rest}}}\n"
        )
    };
    let cases: [(String, ErrorCode, &str, &str); 8] = [
        (
            one("{block: BOXA, text: A}", ""),
            ErrorCode::UnknownKey,
            "objects[0].mleader.content.text",
            "text or a block, not both",
        ),
        (
            one("{text: A, connection: extents}", ""),
            ErrorCode::UnknownKey,
            "objects[0].mleader.content.connection",
            "block content's",
        ),
        (
            one("{block: BOXA}", ", gap: 1"),
            ErrorCode::BadValue,
            "objects[0].mleader.gap",
            "no gap",
        ),
        (
            one("{block: BOXA}", ", landing: 0"),
            ErrorCode::BadValue,
            "objects[0].mleader.landing",
            "has a landing",
        ),
        (
            one("{block: LABEL}", ""),
            ErrorCode::BadValue,
            "objects[0].mleader.content.block",
            "a text, which draws letters",
        ),
        (
            one("{block: CURVE}", ""),
            ErrorCode::BadValue,
            "objects[0].mleader.content.block",
            "a spline",
        ),
        (
            one("{block: NONE}", ""),
            ErrorCode::UnknownName,
            "objects[0].mleader.content.block",
            "unknown block",
        ),
        (
            one("{block: BOXA, attributes: {NOPE: A}}", ""),
            ErrorCode::UnknownName,
            "objects[0].mleader.content.attributes.NOPE",
            "unknown attribute",
        ),
    ];
    for (text, code, field, says) in cases {
        let error = read_sheet_description(&write(&dir, &text)).unwrap_err();
        assert_eq!(error.code, code, "{field}: {}", error.message);
        assert_eq!(error.field.as_deref(), Some(field), "{}", error.message);
        assert!(error.message.contains(says), "{}", error.message);
    }
}

/// The box round what the sheet's object `object` draws of its block.
fn block_drawn(dir: &Path, object: usize) -> Bounds {
    let report = display_list_with(
        &DisplayListRequest::new(dir.join("sheet.yaml")),
        &Settings::default(),
    )
    .unwrap();
    let points: Vec<[f64; 2]> = report
        .display_list
        .items
        .iter()
        .filter(|i| i.object == object && i.part.is_some())
        .flat_map(|i| i.shape.points())
        .collect();
    assert!(!points.is_empty(), "object {object} draws no block");
    let low = |k: usize| points.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min);
    let high = |k: usize| {
        points
            .iter()
            .map(|p| p[k])
            .fold(f64::NEG_INFINITY, f64::max)
    };
    Bounds {
        min: [low(0), low(1)],
        max: [high(0), high(1)],
    }
}

/// Within a hundredth: a circle draws as chords within the pipeline's
/// tolerance.
fn near(got: [f64; 2], want: [f64; 2]) {
    assert!(
        (got[0] - want[0]).abs() < 1e-2 && (got[1] - want[1]).abs() < 1e-2,
        "{got:?} against {want:?}"
    );
}

#[test]
fn the_pipeline_draws_the_block_where_autocad_put_it() {
    let dir = scratch("drawn");
    write(&dir, SHEET);
    // AutoCAD 2026 put BOXA's base point at (133, 17): its box from 138 to
    // 158 across, the landing's end at its left side, and level with the
    // landing at its middle.
    let boxa = block_drawn(&dir, 0);
    near(boxa.min, [138.0, 14.0]);
    near(boxa.max, [158.0, 26.0]);
    // On the left, CIRC's base point at (26, -60): its circle's right side
    // at the landing's end, 62.
    let circ = block_drawn(&dir, 1);
    near(circ.min, [50.0, -46.0]);
    near(circ.max, [62.0, -34.0]);
    // By its insertion point, BOXA's base point at the landing's end less
    // the 5 to its left side; CIRC, which the landing's line misses, with
    // its base point at the landing's end, 262.
    near(block_drawn(&dir, 2).min, [138.0, -183.0]);
    let missed = block_drawn(&dir, 3);
    near(missed.min, [262.0 + 24.0, -240.0 + 14.0]);
}

#[test]
fn block_content_reads_back_from_the_dwg_as_it_was_written() {
    let dir = scratch("round-trip");
    let path = write(&dir, SHEET);
    let report = build(&BuildRequest {
        description: path.clone(),
        output: None,
        overwrite: false,
    })
    .unwrap();
    assert_eq!(report.objects.get("mleader"), Some(&6));
    let sheet = read_sheet_description(&path).unwrap();
    let read = read_dwg(&dir.join("sheet.dwg")).unwrap();
    assert_eq!(read.objects_not_read, 0, "{read:?}");
    assert_eq!(read.blocks_not_read, 0, "{read:?}");
    assert_eq!(read.objects.len(), sheet.objects.len());
    for (read, written) in read.objects.iter().zip(&sheet.objects) {
        let (Object::MLeader(read), Object::MLeader(written)) = (read, written) else {
            panic!("{read:?}");
        };
        assert_eq!(read.leaders, written.leaders);
        let (MLeaderContent::Block(r), MLeaderContent::Block(w)) =
            (&read.content, &written.content)
        else {
            panic!("{read:?}");
        };
        assert_eq!(
            (&r.block, &r.attributes, r.scale, r.connection),
            (&w.block, &w.attributes, w.scale, w.connection)
        );
        assert!((r.rotation - w.rotation).abs() < 1e-9);
        assert_eq!(read.arrowhead, written.arrowhead);
        assert_eq!(read.gap, None);
    }
}

#[test]
fn the_checker_treats_block_content_as_an_insert() {
    let dir = scratch("checked");
    // The block's own lines never count against its own attribute.
    let crossing = "units: mm
paper: [200, 100]
block_library: blocks.yaml
objects:
  - polyline: {points: [[0, 0, 0], [200, 0, 0], [200, 100, 0], [0, 100, 0]], closed: true}
  - mleader: {id: tag, leaders: [[[20, 20, 0], [50, 40, 0]]], content: {block: BOXA, attributes: {TAG: TB-1}}}
";
    let path = write(&dir, crossing);
    let report = check_with(
        &CheckRequest {
            source: path.clone(),
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap();
    assert!(report.passed, "{:#?}", report.findings);
    // A line through its attribute is found against it, named in the block.
    let through =
        format!("{crossing}  - line: {{id: through, from: [73, 30, 0], to: [73, 50, 0]}}\n");
    fs::write(&path, through).unwrap();
    let report = check_with(
        &CheckRequest {
            source: path,
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap();
    let found: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.rule == Rule::TextOnLine)
        .collect();
    assert_eq!(found.len(), 1, "{:#?}", report.findings);
    let text = found[0].object.as_ref().unwrap();
    assert_eq!(text.id, "sheet/tag");
    assert_eq!(text.text.as_deref(), Some("TB-1"));
    assert_eq!(found[0].other.as_ref().unwrap().id, "sheet/through");
}

#[test]
fn a_template_takes_block_contents_attribute_value_from_its_row() {
    let dir = scratch("row-value");
    let set = dir.join("set");
    fs::create_dir_all(set.join("templates")).unwrap();
    fs::write(set.join("blocks.yaml"), LIBRARY).unwrap();
    fs::write(
        set.join("set.yaml"),
        "units: mm\noutput: dwg\nrows: rows.yaml\n",
    )
    .unwrap();
    fs::write(
        set.join("rows.yaml"),
        "rows:\n  - sheet: S-1\n    template: templates/t.yaml\n    values: {TAG: FT-101, SECOND: B-2}\n",
    )
    .unwrap();
    fs::write(
        set.join("templates/t.yaml"),
        "units: mm
block_library: ../blocks.yaml
objects:
  - mleader: {leaders: [[[0, 0, 0], [20, 10, 0]]], content: {block: BOXA, attributes: {TAG: {row: TAG}}}}
  - mleader: {leaders: [[[50, 0, 0], [70, 10, 0]]], content: {block: NOTES, attributes: {N: [A-1, {row: SECOND}]}}}
",
    )
    .unwrap();
    let report = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: false,
        },
        &Settings::default(),
    );
    assert!(report.ok, "{:#?}", report.error);
    let report = report.result.unwrap();
    assert_eq!(report.built, 1, "{:#?}", report.sheets);
    let read = read_dwg(&set.join("dwg/S-1.dwg")).unwrap();
    assert_eq!(read.objects_not_read, 0, "{read:?}");
    let values = |i: usize| -> Vec<String> {
        match &read.objects[i] {
            Object::MLeader(MLeader {
                content: MLeaderContent::Block(b),
                ..
            }) => b.attributes.iter().map(|a| a.value.clone()).collect(),
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(values(0), ["FT-101"]);
    assert_eq!(values(1), ["A-1", "B-2"]);
}

/// Builds `sheet` with the library in a folder of its own, and gives the
/// folder and the drawing its DWG holds.
fn built_drawing(test: &str, sheet: &str) -> (PathBuf, CadDocument) {
    let dir = scratch(test);
    let path = write(&dir, sheet);
    build(&BuildRequest {
        description: path,
        output: None,
        overwrite: false,
    })
    .unwrap_or_else(|e| panic!("{e}"));
    let bytes = fs::read(dir.join("sheet.dwg")).unwrap();
    let drawing = DwgReader::from_stream(Cursor::new(bytes)).read().unwrap();
    (dir, drawing)
}

/// The drawing's one multileader.
fn the_mleader(drawing: &mut CadDocument) -> &mut MultiLeader {
    let handle = drawing
        .model_space_entities()
        .find(|e| matches!(e, EntityType::MultiLeader(_)))
        .map(|e| e.common().handle)
        .unwrap();
    match drawing.get_entity_mut(handle) {
        Some(EntityType::MultiLeader(m)) => m,
        _ => unreachable!(),
    }
}

/// Writes `drawing` beside the built DWG and reads it as the core does.
fn read_back(dir: &Path, drawing: &CadDocument) -> agentdrafter_core::DrawingContents {
    let path = dir.join("changed.dwg");
    fs::write(&path, DwgWriter::write_to_vec(drawing).unwrap()).unwrap();
    read_dwg(&path).unwrap()
}

const ONE: &str = "units: mm
block_library: blocks.yaml
objects:
  - mleader: {leaders: [[[0, 0, 0], [20, 10, 0]]], content: {block: BOXA, attributes: {TAG: TB-1}}}
";

#[test]
fn a_block_content_value_with_two_percent_signs_is_refused_at_its_field() {
    let dir = scratch("percent-refused");
    let one = |content: &str| {
        format!(
            "units: mm\nblock_library: blocks.yaml\nobjects:\n  - mleader: {{leaders: [[[0, 0, 0], [20, 10, 0]]], content: {content}}}\n"
        )
    };
    // AutoCAD 2026 draws `%%d` in block content as a degree sign however
    // it is escaped, and `%%%` as more percent signs.
    for (content, field) in [
        (
            "{block: BOXA, attributes: {TAG: '90%%d'}}",
            "objects[0].mleader.content.attributes.TAG",
        ),
        (
            "{block: BOXA, attributes: {tag: 'A%%%B'}}",
            "objects[0].mleader.content.attributes.tag",
        ),
        (
            "{block: NOTES, attributes: {N: ['50%', '%%c6']}}",
            "objects[0].mleader.content.attributes.N[1]",
        ),
        // A definition's default that holds `%%` is refused at its tag.
        (
            "{block: DEGREES}",
            "objects[0].mleader.content.attributes.T",
        ),
    ] {
        let error = read_sheet_description(&write(&dir, &one(content))).unwrap_err();
        assert_eq!(error.code, ErrorCode::BadValue, "{}", error.message);
        assert_eq!(error.field.as_deref(), Some(field), "{}", error.message);
        assert!(
            error
                .message
                .contains("AutoCAD draws in a multileader's block content as a control code"),
            "{}",
            error.message
        );
    }
    // The same block's default is given a value of its own.
    read_sheet_description(&write(
        &dir,
        &one("{block: DEGREES, attributes: {T: '90'}}"),
    ))
    .unwrap();
}

#[test]
fn a_block_content_value_with_one_percent_sign_reads_back_as_it_was_written() {
    let sheet = ONE.replace("TAG: TB-1", "TAG: '50% A%B'");
    let (dir, mut drawing) = built_drawing("percent", &sheet);
    // Written as it is: AutoCAD draws a lone `%` as itself.
    assert_eq!(
        the_mleader(&mut drawing).block_attributes[0].text,
        "50% A%B"
    );
    let read = read_dwg(&dir.join("sheet.dwg")).unwrap();
    assert_eq!(read.objects_not_read, 0, "{read:?}");
    let Object::MLeader(m) = &read.objects[0] else {
        panic!("{read:?}")
    };
    assert_eq!(m.content.block().unwrap().attributes[0].value, "50% A%B");
}

#[test]
fn a_multileader_whose_block_content_value_holds_two_percent_signs_is_not_read() {
    let (dir, mut drawing) = built_drawing("percent-not-read", ONE);
    for raw in ["90%%d", "A%%B", "%%%"] {
        the_mleader(&mut drawing).block_attributes[0].text = raw.into();
        let read = read_back(&dir, &drawing);
        assert_eq!(read.objects_not_read, 1, "{raw}: {read:?}");
        assert!(read.objects.is_empty(), "{raw}: {read:?}");
    }
}

#[test]
fn a_multileader_whose_left_out_value_takes_a_default_with_two_percent_signs_is_not_read() {
    let sheet = ONE.replace(
        "block: BOXA, attributes: {TAG: TB-1}",
        "block: DEGREES, attributes: {T: '90'}",
    );
    let (dir, mut drawing) = built_drawing("percent-default-not-read", &sheet);
    assert_eq!(read_back(&dir, &drawing).objects_not_read, 0);
    // With its value left out, the value is the definition's default, `%%d`.
    the_mleader(&mut drawing).block_attributes.clear();
    let read = read_back(&dir, &drawing);
    assert_eq!(read.objects_not_read, 1, "{read:?}");
    assert!(read.objects.is_empty(), "{read:?}");
}

#[test]
fn a_row_value_that_puts_two_percent_signs_in_block_content_fails_at_the_rows_value() {
    let dir = scratch("percent-row");
    let set = dir.join("set");
    fs::create_dir_all(set.join("templates")).unwrap();
    fs::write(set.join("blocks.yaml"), LIBRARY).unwrap();
    fs::write(
        set.join("set.yaml"),
        "units: mm\noutput: dwg\nrows: rows.yaml\n",
    )
    .unwrap();
    fs::write(
        set.join("rows.yaml"),
        "rows:\n  - sheet: S-1\n    template: templates/t.yaml\n    values: {TAG: FT-101}\n  - sheet: S-2\n    template: templates/t.yaml\n    values: {TAG: '90%%d'}\n",
    )
    .unwrap();
    fs::write(
        set.join("templates/t.yaml"),
        "units: mm
block_library: ../blocks.yaml
objects:
  - mleader: {leaders: [[[0, 0, 0], [20, 10, 0]]], content: {block: BOXA, attributes: {TAG: {row: TAG}}}}
",
    )
    .unwrap();
    let reply = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: false,
        },
        &Settings::default(),
    );
    let report = reply.result.unwrap();
    assert_eq!(
        (report.built, report.failed),
        (1, 1),
        "{:#?}",
        report.sheets
    );
    let error = report.sheets[1].error.as_ref().unwrap();
    assert_eq!(error.code, ErrorCode::BadValue, "{error:?}");
    assert_eq!(
        error.field.as_deref(),
        Some("rows[1].values.TAG"),
        "{error:?}"
    );
    assert!(error.message.contains("control code"), "{error:?}");
}

#[test]
fn block_content_out_of_the_leaders_plane_is_not_read() {
    let (dir, mut drawing) = built_drawing("raised", ONE);
    let read = read_back(&dir, &drawing);
    assert_eq!(read.objects_not_read, 0, "{read:?}");
    let m = the_mleader(&mut drawing);
    m.context.block_content_location.z = 7.0;
    m.context.content_base_point.z = 7.0;
    m.context.transform_matrix[11] = 7.0;
    let read = read_back(&dir, &drawing);
    assert_eq!(read.objects_not_read, 1, "{read:?}");
}

#[test]
fn block_content_whose_matrix_places_it_elsewhere_is_not_read() {
    let (dir, mut drawing) = built_drawing("matrix", ONE);
    the_mleader(&mut drawing).context.transform_matrix[3] += 1.0;
    let read = read_back(&dir, &drawing);
    assert_eq!(read.objects_not_read, 1, "{read:?}");
}

#[test]
fn block_content_with_no_landing_is_not_read() {
    let (dir, mut drawing) = built_drawing("no-landing", ONE);
    // The block where the model would place it with no landing: the
    // placing moves with the landing's end, 8 to the left.
    let m = the_mleader(&mut drawing);
    m.enable_dogleg = false;
    m.dogleg_length = 0.0;
    m.context.leader_roots[0].landing_distance = 0.0;
    m.context.block_content_location.x -= 8.0;
    m.context.content_base_point.x -= 8.0;
    m.context.transform_matrix[3] -= 8.0;
    let read = read_back(&dir, &drawing);
    assert_eq!(read.objects_not_read, 1, "{read:?}");
    assert!(read.objects.is_empty(), "{read:?}");
}

#[test]
fn a_bulged_segment_that_ends_where_it_starts_places_as_the_pipeline_draws_it() {
    let sheet = ONE.replace("{block: BOXA, attributes: {TAG: TB-1}}", "{block: SPOT}");
    let (dir, mut drawing) = built_drawing("spot", &sheet);
    let m = the_mleader(&mut drawing);
    let location = m.context.block_content_location;
    assert!(
        [location.x, location.y, location.z]
            .iter()
            .chain(&m.context.transform_matrix)
            .all(|v| v.is_finite()),
        "{location:?} {:?}",
        m.context.transform_matrix
    );
    // It draws a point, which crosses nothing: the point sits at the
    // landing's end.
    assert!(
        (location.x - 28.0).abs() < 1e-9 && (location.y - 10.0).abs() < 1e-9,
        "{location:?}"
    );
    let read = read_dwg(&dir.join("sheet.dwg")).unwrap();
    assert_eq!(read.objects_not_read, 0, "{read:?}");
}

/// A base point far from the block's own origin moves the matrix far, and
/// its rounding with it: the block still reads back.
#[test]
fn block_content_with_a_far_base_point_reads_back() {
    let sheet = ONE.replace(
        "{block: BOXA, attributes: {TAG: TB-1}}",
        "{block: FAR, rotation: 13.7}",
    );
    let (dir, _) = built_drawing("far-base", &sheet);
    let read = read_dwg(&dir.join("sheet.dwg")).unwrap();
    assert_eq!(read.objects_not_read, 0, "{read:?}");
    let Object::MLeader(m) = &read.objects[0] else {
        panic!("{read:?}")
    };
    let content = m.content.block().unwrap();
    assert_eq!(content.block, "FAR");
    assert!(
        (content.rotation - 13.7).abs() < 1e-9,
        "{}",
        content.rotation
    );
}
