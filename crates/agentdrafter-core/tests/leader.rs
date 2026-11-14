//! Leaders: the reader takes multileaders, their styles and older leaders
//! and refuses what the pipeline cannot draw, naming the field; the one
//! pipeline draws their lines, arrowheads, landings and text where AutoCAD
//! lays them out; the DWG reads back the same leaders and styles; the
//! checker tells a leader's lines from its letters; and a template takes a
//! leader's text from its row. Every leader here is made up.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build, build_set_with, check_with, display_list_with, parse_block_library,
    parse_sheet_description, read_dwg, Arrowhead, Bounds, BuildRequest, CheckRequest,
    DisplayListReport, DisplayListRequest, ErrorCode, Leader, LeaderAttachment, MLeader,
    MLeaderStyle, Object, Rule, SetBuildRequest, Settings, Shape, Sheet, Unit,
};

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-leader")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A made-up panel's callouts: a multileader with two leader lines in a
/// style of its own, one to the left with a dot, one in Standard, and an
/// older leader hooked to its annotation.
const CALLOUTS: &str = "units: mm
paper: [200, 120]
text_styles:
  notes: {font: romans.shx}
mleader_styles:
  callout:
    text_style: notes
    text_height: 3
    arrow_size: 3
    landing: 6
    landing_gap: 1.5
objects:
  - polyline: {points: [[0, 0, 0], [200, 0, 0], [200, 120, 0], [0, 120, 0]], closed: true}
  - mleader:
      id: terminals
      mleader_style: callout
      leaders:
        - [[40, 20, 0], [100, 40, 0]]
        - [[60, 10, 0], [100, 40, 0]]
      content: {text: TB-1 TERMINALS}
  - mleader:
      id: supply
      mleader_style: callout
      arrowhead: dot
      leaders:
        - [[120, 80, 0], [80, 95, 0]]
      content: {text: \"PS-1\\n24 VDC\", attachment: middle_of_text}
  - mleader:
      id: gland
      landing: 0
      leaders:
        - [[150, 10, 0], [170, 30, 0]]
      content: {text: GLAND, height: 2.5}
  - leader:
      id: note
      points: [[20, 60, 0], [30, 80, 0]]
      annotation: {text: SEE NOTE 3, style: notes}
";

fn sheet() -> Sheet {
    parse_sheet_description(CALLOUTS).unwrap()
}

fn mleader(sheet: &Sheet, index: usize) -> &MLeader {
    match &sheet.objects[index] {
        Object::MLeader(m) => m,
        other => panic!("{other:?}"),
    }
}

fn leader(sheet: &Sheet, index: usize) -> &Leader {
    match &sheet.objects[index] {
        Object::Leader(l) => l,
        other => panic!("{other:?}"),
    }
}

#[test]
fn multileaders_their_styles_and_leaders_read_with_what_they_give() {
    let sheet = sheet();
    let terminals = mleader(&sheet, 1);
    assert_eq!(terminals.leaders.len(), 2);
    assert_eq!(terminals.mleader_style, "callout");
    assert_eq!(terminals.content.text, "TB-1 TERMINALS");
    assert_eq!(terminals.arrowhead, None);
    let supply = mleader(&sheet, 2);
    assert_eq!(supply.arrowhead, Some(Arrowhead::Dot));
    assert_eq!(supply.content.text, "PS-1\n24 VDC");
    assert_eq!(
        supply.content.attachment,
        Some(LeaderAttachment::MiddleOfText)
    );
    assert_eq!(supply.side(), -1.0);
    let gland = mleader(&sheet, 3);
    assert_eq!(gland.landing, Some(0.0));
    assert_eq!(gland.mleader_style, "Standard");
    let style = sheet.mleader_style("callout").unwrap();
    assert_eq!(style.text_style, "notes");
    assert_eq!(style.landing, 6.0);
    assert_eq!(style.attachment, LeaderAttachment::MiddleOfTopLine);
    // Standard is there unlisted, as AutoCAD 2026's metric one is.
    let standard = sheet.mleader_style("Standard").unwrap();
    assert_eq!(standard, MLeaderStyle::standard(Unit::Mm));
    assert_eq!(
        (
            standard.text_height,
            standard.arrow_size,
            standard.landing,
            standard.landing_gap
        ),
        (4.0, 4.0, 8.0, 2.0)
    );
    // An older leader takes ISO-25's sizes on a metric sheet.
    let note = leader(&sheet, 4);
    assert_eq!((note.arrow_size, note.gap), (2.5, 0.625));
    assert_eq!(note.annotation.as_ref().unwrap().height, 2.5);
    assert!(note.hooked());
}

/// The sheet with `from` written as `to`, which must be there.
fn with(from: &str, to: &str) -> String {
    assert!(CALLOUTS.contains(from), "{from}");
    CALLOUTS.replacen(from, to, 1)
}

#[test]
fn the_reader_refuses_a_leader_it_cannot_draw_and_names_the_field() {
    let cases: [(String, ErrorCode, &str); 9] = [
        (
            with("[[60, 10, 0], [100, 40, 0]]", "[[60, 10, 0], [100, 41, 0]]"),
            ErrorCode::BadValue,
            "objects[1].mleader.leaders[1][1]",
        ),
        (
            with("[[60, 10, 0], [100, 40, 0]]", "[[99, 40, 0], [100, 40, 0]]"),
            ErrorCode::BadValue,
            "objects[1].mleader.leaders[1]",
        ),
        (
            with("arrowhead: dot", "arrowhead: oblique"),
            ErrorCode::BadValue,
            "objects[2].mleader.arrowhead",
        ),
        (
            with(
                "attachment: middle_of_text",
                "attachment: underline_bottom_line",
            ),
            ErrorCode::BadValue,
            "objects[2].mleader.content.attachment",
        ),
        (
            with("mleader_style: callout", "mleader_style: callouts"),
            ErrorCode::UnknownName,
            "objects[1].mleader.mleader_style",
        ),
        (
            with("landing: 0", "landing: -1"),
            ErrorCode::BadValue,
            "objects[3].mleader.landing",
        ),
        (
            with("[[20, 60, 0], [30, 80, 0]]", "[[20, 60, 0], [20, 60, 0]]"),
            ErrorCode::BadValue,
            "objects[4].leader.points[1]",
        ),
        (
            with("{text: GLAND, height: 2.5}", "{height: 2.5}"),
            ErrorCode::MissingKey,
            "objects[3].mleader.content.text",
        ),
        (
            with("  callout:\n", "  standard:\n"),
            ErrorCode::BadValue,
            "mleader_styles.standard",
        ),
    ];
    for (text, code, field) in cases {
        let error = parse_sheet_description(&text).unwrap_err();
        assert_eq!(error.code, code, "{field}: {}", error.message);
        assert_eq!(error.field.as_deref(), Some(field), "{}", error.message);
    }
}

#[test]
fn a_leader_belongs_on_a_sheet_not_in_a_block() {
    for (kind, object) in [
        (
            "multileader",
            "mleader: {leaders: [[[0, 0, 0], [10, 5, 0]]], content: {text: A}}",
        ),
        ("leader", "leader: {points: [[0, 0, 0], [10, 5, 0]]}"),
    ] {
        let library = format!("units: mm\nblocks:\n  B:\n    objects:\n      - {object}\n");
        let error = parse_block_library(&library).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnknownKey);
        assert!(
            error
                .message
                .contains(&format!("a {kind} belongs on a sheet")),
            "{}",
            error.message
        );
    }
}

fn display(dir: &Path, text: &str) -> DisplayListReport {
    let path = dir.join("sheet.yaml");
    fs::write(&path, text).unwrap();
    display_list_with(&DisplayListRequest::new(&path), &Settings::default()).unwrap()
}

/// The box round what the sheet's object `object` draws, its lines and
/// arrowheads (`part` none) or its text (`part` 0).
fn drawn(report: &DisplayListReport, object: usize, text: bool) -> Bounds {
    let points: Vec<[f64; 2]> = report
        .display_list
        .items
        .iter()
        .filter(|i| i.object == object && i.part.is_some() == text)
        .flat_map(|i| i.shape.points())
        .collect();
    assert!(!points.is_empty(), "object {object} draws nothing");
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

#[test]
fn the_pipeline_draws_arrowheads_landings_and_text_where_autocad_lays_them_out() {
    let dir = scratch("drawn");
    let report = display(&dir, CALLOUTS);
    let list = &report.display_list;
    // The closed filled arrowhead is a filled triangle with its point at
    // the arrow's point and its back an arrow's length up the line, a sixth
    // of it wide on each side, as AutoCAD 2026 exploded one.
    let fills: Vec<&Vec<Vec<[f64; 2]>>> = list
        .items
        .iter()
        .filter(|i| i.object == 1)
        .filter_map(|i| match &i.shape {
            Shape::Fill { contours, .. } => Some(contours),
            Shape::Stroke { .. } => None,
        })
        .collect();
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].len(), 2, "one arrowhead for each leader line");
    let first = &fills[0][0];
    assert!(first.contains(&[40.0, 20.0]), "{first:?}");
    let back = first.iter().find(|p| **p != [40.0, 20.0]).unwrap();
    let along = 3.0 / 60f64.hypot(20.0);
    let base = [40.0 + 60.0 * along, 20.0 + 20.0 * along];
    let wing = ((back[0] - base[0]).hypot(back[1] - base[1]) - 0.5).abs();
    assert!(wing < 1e-9, "{first:?}");
    // The leader line starts behind the arrowhead.
    let starts: Vec<[f64; 2]> = list
        .items
        .iter()
        .filter(|i| i.object == 1 && i.part.is_none())
        .flat_map(|i| match &i.shape {
            Shape::Stroke { runs, .. } => runs.iter().map(|r| r[0]).collect(),
            Shape::Fill { .. } => Vec::new(),
        })
        .collect();
    assert!(
        starts
            .iter()
            .any(|p| (p[0] - base[0]).abs() < 1e-9 && (p[1] - base[1]).abs() < 1e-9),
        "{starts:?}"
    );
    // The landing runs right, 6 long, and the text starts its gap past it,
    // with the middle of its top line on the landing.
    let lines = drawn(&report, 1, false);
    assert!((lines.max[0] - 106.0).abs() < 1e-9, "{lines:?}");
    let text = drawn(&report, 1, true);
    assert!(text.min[0] >= 107.5 - 1e-9, "{text:?}");
    assert!(text.min[0] < 108.0, "{text:?}");
    assert!((text.max[1] - 41.5).abs() < 0.05, "{text:?}");
    // A leader line running left takes its landing and text to the left:
    // the text ends its gap short of the landing's end, and the landing is
    // at the middle of its two lines.
    let lines = drawn(&report, 2, false);
    assert!((lines.min[0] - 74.0).abs() < 1e-9, "{lines:?}");
    let text = drawn(&report, 2, true);
    assert!(text.max[0] <= 72.5 + 1e-9 && text.max[0] > 72.0, "{text:?}");
    assert!(
        ((text.max[1] + text.min[1]) / 2.0 - 95.0).abs() < 0.6,
        "{text:?}"
    );
    // The dot is a filled disc as wide as the arrow is long.
    let dot: Vec<[f64; 2]> = list
        .items
        .iter()
        .filter(|i| i.object == 2)
        .filter_map(|i| match &i.shape {
            Shape::Fill { contours, .. } => Some(contours[0].clone()),
            Shape::Stroke { .. } => None,
        })
        .next()
        .unwrap();
    for p in &dot {
        assert!(((p[0] - 120.0).hypot(p[1] - 80.0) - 1.5).abs() < 1e-9);
    }
    // No landing: the text starts Standard's 2 mm gap past the leader's end.
    let text = drawn(&report, 3, true);
    assert!(
        text.min[0] >= 172.0 - 1e-9 && text.min[0] < 172.5,
        "{text:?}"
    );
    // The older leader is hooked: its line ends in a horizontal segment as
    // long as its arrow, and its annotation starts its gap past the end.
    let ends: Vec<Vec<[f64; 2]>> = list
        .items
        .iter()
        .filter(|i| i.object == 4 && i.part.is_none())
        .flat_map(|i| match &i.shape {
            Shape::Stroke { runs, .. } => runs.clone(),
            Shape::Fill { .. } => Vec::new(),
        })
        .collect();
    assert!(
        ends.iter()
            .any(|run| run.ends_with(&[[27.5, 80.0], [30.0, 80.0]])),
        "{ends:?}"
    );
    let text = drawn(&report, 4, true);
    assert!(
        text.min[0] >= 30.625 - 1e-9 && text.min[0] < 31.2,
        "{text:?}"
    );
    // The text of each is a run that says it.
    let run = list
        .text
        .iter()
        .find(|r| r.object == 1 && r.part == Some(0))
        .unwrap();
    let says: String = run.characters.iter().map(|(c, _)| c).collect();
    assert_eq!(says, "TB-1 TERMINALS");
}

#[test]
fn leaders_and_their_styles_read_back_from_the_dwg() {
    let dir = scratch("round-trip");
    let path = dir.join("sheet.yaml");
    fs::write(&path, CALLOUTS).unwrap();
    let report = build(&BuildRequest {
        description: path.clone(),
        output: None,
        overwrite: false,
    })
    .unwrap();
    assert_eq!(report.objects.get("mleader"), Some(&3));
    assert_eq!(report.objects.get("leader"), Some(&1));
    let sheet = sheet();
    let read = read_dwg(&dir.join("sheet.dwg")).unwrap();
    assert_eq!(read.objects_not_read, 0, "{read:?}");
    assert_eq!(read.mleader_styles_not_read, 0, "{read:?}");
    // The dot's block is read with its leader, not as a block.
    assert!(read.blocks.is_empty(), "{:?}", read.blocks);
    assert_eq!(read.blocks_not_read, 0, "{read:?}");
    // The annotation is read with its leader, not as an mtext of its own.
    assert_eq!(read.objects, sheet.objects);
    assert_eq!(read.ids_and_names[1].id.as_deref(), Some("sheet/terminals"));
    assert_eq!(read.ids_and_names[4].id.as_deref(), Some("sheet/note"));
    for name in ["callout", "Standard"] {
        let style = read.mleader_styles.iter().find(|s| s.name == name).unwrap();
        assert_eq!(*style, sheet.mleader_style(name).unwrap(), "{name}");
    }
}

#[test]
fn leaders_on_an_imperial_sheet_read_back_with_its_sizes() {
    let dir = scratch("imperial");
    let text = "units: in
objects:
  - mleader: {leaders: [[[1, 1, 0], [2, 1.5, 0]]], content: {text: P-101}, arrowhead: open}
  - leader: {points: [[3, 1, 0], [2, 2, 0]], arrowhead: none, annotation: {text: VENT}}
";
    let path = dir.join("sheet.yaml");
    fs::write(&path, text).unwrap();
    build(&BuildRequest {
        description: path,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let read = read_dwg(&dir.join("sheet.dwg")).unwrap();
    let sheet = parse_sheet_description(text).unwrap();
    assert_eq!(read.objects, sheet.objects);
    let standard = read
        .mleader_styles
        .iter()
        .find(|s| s.name == "Standard")
        .unwrap();
    assert_eq!(*standard, MLeaderStyle::standard(Unit::In));
    let Object::Leader(vent) = &read.objects[1] else {
        panic!()
    };
    assert_eq!((vent.arrow_size, vent.gap), (0.18, 0.09));
    assert_eq!(vent.side(), -1.0);
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
fn the_checker_counts_a_leaders_lines_against_other_text_and_its_text_against_other_lines() {
    let dir = scratch("checked");
    let path = dir.join("sheet.yaml");
    // A leader's own lines, landing and arrowhead never touch its own text.
    fs::write(&path, CALLOUTS).unwrap();
    let report = check(&path);
    assert!(report.passed, "{:#?}", report.findings);
    // A line through the multileader's text is found against its text.
    let crossed =
        format!("{CALLOUTS}  - line: {{id: through, from: [110, 30, 0], to: [110, 50, 0]}}\n");
    fs::write(&path, &crossed).unwrap();
    let report = check(&path);
    let found: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.rule == Rule::TextOnLine)
        .collect();
    assert_eq!(found.len(), 1, "{found:#?}");
    let text = found[0].object.as_ref().unwrap();
    assert_eq!(text.id, "sheet/terminals");
    assert_eq!(text.kind, "mleader");
    assert_eq!(text.text.as_deref(), Some("TB-1 TERMINALS"));
    assert_eq!(found[0].other.as_ref().unwrap().id, "sheet/through");
    // A leader line through another text is found against that text.
    let under = format!(
        "{CALLOUTS}  - text: {{id: label, contents: XXXXXXXX, position: [60, 22, 0], height: 3}}\n"
    );
    fs::write(&path, &under).unwrap();
    let report = check(&path);
    let found: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.rule == Rule::TextOnLine)
        .collect();
    assert!(!found.is_empty(), "{:#?}", report.findings);
    assert!(found
        .iter()
        .all(|f| f.object.as_ref().unwrap().id == "sheet/label"
            && f.other.as_ref().unwrap().id == "sheet/terminals"));
}

#[test]
fn a_template_takes_a_leaders_text_from_its_row() {
    let dir = scratch("row-value");
    let set = dir.join("set");
    fs::create_dir_all(set.join("templates")).unwrap();
    fs::write(
        set.join("set.yaml"),
        "units: mm\noutput: dwg\nrows: rows.yaml\n",
    )
    .unwrap();
    fs::write(
        set.join("rows.yaml"),
        "rows:\n  - sheet: S-1\n    template: templates/t.yaml\n    values: {STRIP: TB-7, NOTE: SEE NOTE 4}\n",
    )
    .unwrap();
    fs::write(
        set.join("templates/t.yaml"),
        "units: mm
objects:
  - mleader: {leaders: [[[0, 0, 0], [20, 10, 0]]], content: {text: {row: STRIP}}}
  - leader: {points: [[50, 0, 0], [60, 10, 0]], annotation: {text: {row: NOTE}}}
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
    let Object::MLeader(strip) = &read.objects[0] else {
        panic!("{read:?}");
    };
    assert_eq!(strip.content.text, "TB-7");
    let Object::Leader(note) = &read.objects[1] else {
        panic!("{read:?}");
    };
    assert_eq!(note.annotation.as_ref().unwrap().text, "SEE NOTE 4");
}

#[test]
fn a_multileaders_own_landing_gap_draws_and_reads_back() {
    let dir = scratch("own-gap");
    // Standard's metric gap is 2: this one's own is 10.
    let text = "units: mm
objects:
  - mleader: {leaders: [[[0, 0, 0], [20, 10, 0]]], content: {text: NOTE}, gap: 10}
";
    let sheet = parse_sheet_description(text).unwrap();
    let Object::MLeader(own) = &sheet.objects[0] else {
        panic!()
    };
    assert_eq!(own.gap, Some(10.0));
    // The text starts its own gap past Standard's 8 long landing.
    let report = display(&dir, text);
    let drawn = drawn(&report, 0, true);
    assert!(
        drawn.min[0] >= 38.0 - 1e-9 && drawn.min[0] < 38.5,
        "{drawn:?}"
    );
    let path = dir.join("sheet.yaml");
    build(&BuildRequest {
        description: path,
        output: None,
        overwrite: false,
    })
    .unwrap();
    let read = read_dwg(&dir.join("sheet.dwg")).unwrap();
    assert_eq!(read.objects_not_read, 0, "{read:?}");
    assert_eq!(read.objects, sheet.objects);
}

/// AutoCAD 2026's own `_Open` arrowhead block, as it made it: the
/// triangle's sides and its middle, `ByBlock` in colour, linetype and
/// lineweight.
const AUTOCAD_OPEN: &str = "units: mm
blocks:
  _Open:
    objects:
      - line: {from: [-1, 0.16666666666666666, 0], to: [0, 0, 0], color: ByBlock, linetype: ByBlock, lineweight: ByBlock}
      - line: {from: [-1, -0.16666666666666666, 0], to: [0, 0, 0], color: ByBlock, linetype: ByBlock, lineweight: ByBlock}
      - line: {from: [0, 0, 0], to: [-1, 0, 0], color: ByBlock, linetype: ByBlock, lineweight: ByBlock}
";

const OPEN_SHEET: &str = "units: mm
block_library: blocks.yaml
objects:
  - insert: {block: _Open, position: [50, 50, 0], scale: 5}
  - mleader: {leaders: [[[0, 0, 0], [20, 10, 0]]], content: {text: P-101}, arrowhead: open}
";

#[test]
fn a_library_block_that_is_autocads_arrowhead_is_written_once_and_shared() {
    let dir = scratch("library-arrowhead");
    fs::write(dir.join("blocks.yaml"), AUTOCAD_OPEN).unwrap();
    let path = dir.join("sheet.yaml");
    fs::write(&path, OPEN_SHEET).unwrap();
    build(&BuildRequest {
        description: path.clone(),
        output: None,
        overwrite: false,
    })
    .unwrap_or_else(|e| panic!("{e}"));
    let read = read_dwg(&dir.join("sheet.dwg")).unwrap();
    assert_eq!(read.objects_not_read, 0, "{read:?}");
    let names: Vec<&str> = read.blocks.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["_Open"]);
    assert!(matches!(&read.objects[1], Object::MLeader(m) if m.arrowhead == Some(Arrowhead::Open)));
}

#[test]
fn a_library_block_named_like_an_arrowhead_but_drawn_otherwise_is_refused() {
    let dir = scratch("library-arrowhead-differs");
    fs::write(
        dir.join("blocks.yaml"),
        AUTOCAD_OPEN.replacen("to: [-1, 0, 0]", "to: [-2, 0, 0]", 1),
    )
    .unwrap();
    let path = dir.join("sheet.yaml");
    fs::write(&path, OPEN_SHEET).unwrap();
    let error = build(&BuildRequest {
        description: path,
        output: None,
        overwrite: false,
    })
    .unwrap_err();
    assert!(
        error.message.contains("`_Open`")
            && error
                .message
                .contains("differs from AutoCAD's arrowhead block that a leader needs"),
        "{}",
        error.message
    );
    assert!(
        !error.message.contains("already exists"),
        "{}",
        error.message
    );
}

#[test]
fn a_first_segment_as_long_as_the_arrow_draws_no_shaft() {
    let dir = scratch("exact-arrow");
    // Standard's metric arrow is 4: the first segment is exactly that.
    let text = "units: mm
objects:
  - mleader: {leaders: [[[0, 0, 0], [4, 0, 0], [20, 10, 0]]], content: {text: NOTE}}
  - leader: {points: [[50, 0, 0], [52.5, 0, 0], [60, 10, 0]]}
";
    let report = display(&dir, text);
    for (object, tip, end) in [(0, [0.0, 0.0], [4.0, 0.0]), (1, [50.0, 0.0], [52.5, 0.0])] {
        let runs: Vec<Vec<[f64; 2]>> = report
            .display_list
            .items
            .iter()
            .filter(|i| i.object == object && i.part.is_none())
            .flat_map(|i| match &i.shape {
                Shape::Stroke { runs, .. } => runs.clone(),
                Shape::Fill { .. } => Vec::new(),
            })
            .collect();
        // Nothing strokes the first segment: the line starts where the
        // arrow ends.
        for run in &runs {
            for pair in run.windows(2) {
                let on_first =
                    |p: [f64; 2]| p[1] == tip[1] && p[0] >= tip[0] - 1e-9 && p[0] <= end[0] + 1e-9;
                assert!(
                    !(on_first(pair[0]) && on_first(pair[1])),
                    "object {object} strokes {pair:?}"
                );
            }
        }
        assert!(runs.iter().any(|r| r[0] == end), "{runs:?}");
    }
}

/// A library block drawn as AutoCAD's arrowhead is shared however its
/// objects come in turn, and whichever way round a line is written.
#[test]
fn a_library_arrowhead_drawn_alike_in_another_order_is_shared() {
    let dir = scratch("library-arrowhead-reordered");
    let lines: Vec<&str> = AUTOCAD_OPEN.lines().collect();
    let reordered = [
        lines[0],
        lines[1],
        lines[2],
        lines[3],
        lines[5],
        lines[4],
        "      - line: {from: [-1, 0, 0], to: [0, 0, 0], color: ByBlock, linetype: ByBlock, lineweight: ByBlock}",
    ]
    .join("\n")
        + "\n";
    fs::write(dir.join("blocks.yaml"), reordered).unwrap();
    let path = dir.join("sheet.yaml");
    fs::write(&path, OPEN_SHEET).unwrap();
    build(&BuildRequest {
        description: path,
        output: None,
        overwrite: false,
    })
    .unwrap_or_else(|e| panic!("{e}"));
    let read = read_dwg(&dir.join("sheet.dwg")).unwrap();
    assert_eq!(read.objects_not_read, 0, "{read:?}");
    let names: Vec<&str> = read.blocks.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["_Open"]);
}
