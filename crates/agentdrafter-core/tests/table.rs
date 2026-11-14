//! Tables: the reader takes them and refuses what is wrong, naming the
//! field; the one pipeline draws their borders and each cell's text where
//! AutoCAD lays them out; the DWG reads back the same tables and table
//! styles; the checker holds each cell's text to its cell, as AutoCAD would
//! grow its row; and a template takes a cell's text from its row. Every
//! table here is made up.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build, build_set_with, check_with, display_list_with, parse_block_library,
    parse_sheet_description, read_dwg, Attachment, BuildRequest, CheckRequest, DisplayListRequest,
    ErrorCode, Object, ObjectLineweight, Rule, SetBuildRequest, Settings, Shape, Sheet, Table,
    TableStyle,
};

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-table")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A made-up schedule: a title over four columns, a header and three rows,
/// one cell spanning two columns, with a style, a height and a
/// justification of their own.
const SCHEDULE: &str = "units: mm
paper: [200, 100]
text_styles:
  notes: {font: romans.shx}
table_styles:
  schedule:
    text_style: notes
    text_height: 2.5
    horizontal_margin: 1
    vertical_margin: 1
    border_lineweight: 0.35
    title: {text_height: 3.5}
    data: {justify: middle_left}
objects:
  - table:
      id: cables
      position: [10, 90, 0]
      columns: [30, 40, 40, 25]
      rows: [8, 6, 6, 6]
      title: true
      header: true
      table_style: schedule
      cells:
        - [{text: CABLE SCHEDULE, merge: [1, 4]}, '', '', '']
        - [CABLE, FROM, TO, CORES]
        - [C-1, JB-1, MCC-1, {text: '4', justify: middle_center}]
        - [{text: C-2, style: Standard}, {text: JB-2, height: 2}, {text: SPARE, merge: [1, 2]}, '']
";

fn the_table(sheet: &Sheet) -> &Table {
    sheet
        .objects
        .iter()
        .find_map(|o| match o {
            Object::Table(t) => Some(t),
            _ => None,
        })
        .expect("a table")
}

#[test]
fn a_table_reads_with_its_rows_columns_cells_and_style() {
    let sheet = parse_sheet_description(SCHEDULE).unwrap();
    let table = the_table(&sheet);
    assert_eq!(table.columns, vec![30.0, 40.0, 40.0, 25.0]);
    assert_eq!(table.rows, vec![8.0, 6.0, 6.0, 6.0]);
    assert!(table.title && table.header);
    assert_eq!(table.table_style, "schedule");
    assert_eq!(table.cells[0][0].merge, Some([1, 4]));
    assert_eq!(table.cells[2][3].justify, Some(Attachment::MiddleCenter));
    assert_eq!(table.cells[3][0].style.as_deref(), Some("Standard"));
    assert_eq!(table.cells[3][1].height, Some(2.0));
    assert_eq!(table.cells[3][2].merge, Some([1, 2]));
    let style = sheet.table_style("schedule").unwrap();
    assert_eq!(style.title.text_height, 3.5);
    assert_eq!(style.header.text_height, 2.5);
    assert_eq!(style.data.justify, Attachment::MiddleLeft);
    assert_eq!(style.header.justify, Attachment::MiddleCenter);
    assert_eq!(style.data.text_style, "notes");
    assert_eq!(
        style.border_lineweight,
        ObjectLineweight::Width(agentdrafter_core::Lineweight::from_millimetres(0.35).unwrap())
    );
    // Standard is there unlisted, as AutoCAD's metric one is.
    let standard = sheet.table_style("Standard").unwrap();
    assert_eq!(standard, TableStyle::standard(agentdrafter_core::Unit::Mm));
    assert_eq!(standard.data.text_height, 4.5);
    assert_eq!(standard.title.text_height, 6.0);
    assert_eq!(standard.horizontal_margin, 1.5);
}

/// The sheet with `from` written as `to`, which must be there.
fn with(from: &str, to: &str) -> String {
    assert!(SCHEDULE.contains(from), "{from}");
    SCHEDULE.replacen(from, to, 1)
}

#[test]
fn the_reader_refuses_a_table_it_cannot_draw_and_names_the_field() {
    let cases: [(String, ErrorCode, &str); 11] = [
        (
            with("rows: [8, 6, 6, 6]", "rows: [8, 6, 6]"),
            ErrorCode::BadValue,
            "objects[0].table.cells",
        ),
        (
            with("[CABLE, FROM, TO, CORES]", "[CABLE, FROM, TO]"),
            ErrorCode::BadValue,
            "objects[0].table.cells[1]",
        ),
        (
            with("merge: [1, 4]", "merge: [1, 5]"),
            ErrorCode::BadValue,
            "objects[0].table.cells[0][0].merge",
        ),
        (
            with("merge: [1, 4]", "merge: [2, 2]"),
            ErrorCode::BadValue,
            "objects[0].table.cells[1][0]",
        ),
        (
            with(
                "{text: SPARE, merge: [1, 2]}, ''",
                "{text: SPARE, merge: [1, 2]}, X",
            ),
            ErrorCode::BadValue,
            "objects[0].table.cells[3][3]",
        ),
        (
            with("merge: [1, 2]", "merge: [1, 1]"),
            ErrorCode::BadValue,
            "objects[0].table.cells[3][2].merge",
        ),
        (
            with("table_style: schedule", "table_style: list"),
            ErrorCode::UnknownName,
            "objects[0].table.table_style",
        ),
        (
            with("columns: [30, 40, 40, 25]", "columns: [30, 0, 40, 25]"),
            ErrorCode::BadValue,
            "objects[0].table.columns[1]",
        ),
        (
            with(
                "{text: '4', justify: middle_center}",
                "{text: '4', justify: middle}",
            ),
            ErrorCode::BadValue,
            "objects[0].table.cells[2][3].justify",
        ),
        (
            with("    text_height: 2.5\n", ""),
            ErrorCode::MissingKey,
            "table_styles.schedule.text_height",
        ),
        (
            with("  schedule:\n", "  STANDARD:\n"),
            ErrorCode::BadValue,
            "table_styles.STANDARD",
        ),
    ];
    for (text, code, field) in cases {
        let error = parse_sheet_description(&text).expect_err(field);
        assert_eq!(error.code, code, "{field}: {error:?}");
        assert_eq!(error.field.as_deref(), Some(field), "{error:?}");
    }
}

#[test]
fn a_merge_too_large_to_add_is_past_the_table_not_a_panic() {
    // From row 3, column 2, a span of the largest whole number would run
    // past the end of the numbers if added to its place.
    for merge in [
        "merge: [18446744073709551615, 1]",
        "merge: [1, 18446744073709551615]",
    ] {
        let text = with("merge: [1, 2]", merge);
        let error = parse_sheet_description(&text).unwrap_err();
        assert_eq!(error.code, ErrorCode::BadValue, "{error:?}");
        assert_eq!(
            error.field.as_deref(),
            Some("objects[0].table.cells[3][2].merge")
        );
        assert!(
            error.message.contains("past the table"),
            "{}",
            error.message
        );
    }
}

#[test]
fn a_table_belongs_on_a_sheet_not_in_a_block() {
    let library = "units: mm
blocks:
  B:
    objects:
      - table: {position: [0, 0, 0], columns: [10], rows: [5], cells: [[A]]}
";
    let error = parse_block_library(library).unwrap_err();
    assert_eq!(error.code, ErrorCode::UnknownKey);
    assert!(
        error.message.contains("a table belongs on a sheet"),
        "{}",
        error.message
    );
}

fn display(dir: &Path, text: &str) -> agentdrafter_core::DisplayListReport {
    let path = dir.join("sheet.yaml");
    fs::write(&path, text).unwrap();
    display_list_with(&DisplayListRequest::new(&path), &Settings::default()).unwrap()
}

#[test]
fn the_pipeline_draws_the_borders_and_each_cells_text_in_its_margins() {
    let dir = scratch("drawn");
    let report = display(&dir, SCHEDULE);
    let list = &report.display_list;
    // The borders: each row's boundary across, each column's down, the
    // inside of a merged cell left out, at the style's 0.35 mm.
    let borders: Vec<&Vec<[f64; 2]>> = list
        .items
        .iter()
        .filter(|i| i.cell.is_none())
        .flat_map(|i| match &i.shape {
            Shape::Stroke { lineweight, runs } => {
                assert_eq!(*lineweight, 0.35);
                runs.iter().collect::<Vec<_>>()
            }
            Shape::Fill { .. } => Vec::new(),
        })
        .collect();
    assert_eq!(borders.len(), 5 + 5);
    assert!(borders.contains(&&vec![[10.0, 90.0], [145.0, 90.0]]));
    // Under the title only: the first inner column boundary starts at 82.
    assert!(borders.contains(&&vec![[40.0, 82.0], [40.0, 64.0]]));
    // Under SPARE, which spans two columns, the third inner one stops at 70.
    assert!(borders.contains(&&vec![[120.0, 82.0], [120.0, 70.0]]));
    // Each cell's text is marked with its cell and sits in its margins.
    let ink = |cell: [usize; 2]| {
        let points: Vec<[f64; 2]> = list
            .items
            .iter()
            .filter(|i| i.cell == Some(cell))
            .flat_map(|i| i.shape.points())
            .collect();
        assert!(!points.is_empty(), "cell {cell:?} draws nothing");
        let low = |k: usize| points.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min);
        let high = |k: usize| {
            points
                .iter()
                .map(|p| p[k])
                .fold(f64::NEG_INFINITY, f64::max)
        };
        agentdrafter_core::Bounds {
            min: [low(0), low(1)],
            max: [high(0), high(1)],
        }
    };
    // The title, middle centre of the merged row.
    let title = ink([0, 0]);
    assert!(
        ((title.min[0] + title.max[0]) / 2.0 - 77.5).abs() < 0.2,
        "{title:?}"
    );
    assert!(
        ((title.min[1] + title.max[1]) / 2.0 - 86.0).abs() < 0.2,
        "{title:?}"
    );
    // C-1, middle left: its ink starts at the cell's left margin.
    let data = ink([2, 0]);
    assert!((data.min[0] - 11.0).abs() < 0.3, "{data:?}");
    assert!(
        ((data.min[1] + data.max[1]) / 2.0 - 73.0).abs() < 0.2,
        "{data:?}"
    );
    // JB-2 at its own height.
    assert!((ink([3, 1]).height() - 2.0).abs() < 0.05);
    // Each line of a cell's text is a run that says it.
    let run = list.text.iter().find(|r| r.cell == Some([1, 3])).unwrap();
    let says: String = run.characters.iter().map(|(c, _)| c).collect();
    assert_eq!(says, "CORES");
}

#[test]
fn a_cells_text_draws_in_the_tables_colour_and_its_layers_lineweight() {
    // AutoCAD 2026 wrote a laid-out table's texts ByBlock in colour and
    // ByLayer in lineweight, on the table's layer: its borders take the
    // table's own lineweight, its text the layer's.
    let dir = scratch("text-pen");
    let text = "units: mm
layers:
  list: {color: green, lineweight: 0.18}
objects:
  - table: {position: [0, 50, 0], columns: [40], rows: [10], cells: [[PUMP]], layer: list, color: red, lineweight: 0.7}
";
    let report = display(&dir, text);
    let list = &report.display_list;
    let mut cells = 0;
    for item in &list.items {
        let Shape::Stroke { lineweight, .. } = &item.shape else {
            continue;
        };
        assert_eq!(item.color.0, 1, "{item:?}");
        if item.cell.is_some() {
            cells += 1;
            assert_eq!(*lineweight, 0.18, "the text: {item:?}");
        } else {
            assert_eq!(*lineweight, 0.7, "a border: {item:?}");
        }
    }
    assert!(cells > 0);
}

#[test]
fn a_blank_line_in_a_cell_counts_towards_the_height_its_text_needs() {
    // Three lines, the middle one blank: 2.5 + 2 * 5/3 * 2.5 = 10.8333 and
    // both margins, 12.8333, in a row 10 high.
    let dir = scratch("blank-line");
    let path = dir.join("sheet.yaml");
    fs::write(
        &path,
        "units: mm
paper: [100, 100]
table_styles:
  s: {text_height: 2.5, horizontal_margin: 1, vertical_margin: 1}
objects:
  - polyline: {points: [[0, 0, 0], [100, 0, 0], [100, 100, 0], [0, 100, 0]], closed: true}
  - table:
      position: [10, 90, 0]
      columns: [40]
      rows: [10]
      table_style: s
      cells:
        - [\"A\\n\\nB\"]
",
    )
    .unwrap();
    let sheet = parse_sheet_description(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(the_table(&sheet).cells[0][0].text, "A\n\nB");
    let report = check(&path);
    let found: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.rule == Rule::TextOverflow)
        .collect();
    assert_eq!(found.len(), 1, "{:#?}", report.findings);
    let past = found[0].past.as_ref().unwrap();
    // Data is top centre: the lines hang from the top margin.
    assert!(past.top.abs() < 1e-9, "{past:?}");
    assert!(
        (past.bottom - (12.8333333333 - 10.0)).abs() < 1e-6,
        "{past:?}"
    );
    assert!(
        found[0].message.contains("10.833333"),
        "{}",
        found[0].message
    );
}

#[test]
fn a_table_and_its_style_read_back_from_the_dwg() {
    let dir = scratch("round-trip");
    let path = dir.join("sheet.yaml");
    fs::write(&path, SCHEDULE).unwrap();
    let report = build(&BuildRequest {
        description: path.clone(),
        output: None,
        overwrite: false,
    })
    .unwrap();
    assert_eq!(report.objects.get("table"), Some(&1));
    let sheet = parse_sheet_description(SCHEDULE).unwrap();
    let read = read_dwg(&dir.join("sheet.dwg")).unwrap();
    assert_eq!(read.objects_not_read, 0, "{read:?}");
    // The table's own block is read with it.
    assert_eq!(read.blocks_not_read, 0, "{read:?}");
    assert_eq!(read.table_styles_not_read, 0, "{read:?}");
    assert_eq!(read.objects, sheet.objects);
    assert_eq!(read.ids_and_names[0].id.as_deref(), Some("sheet/cables"));
    let schedule = read
        .table_styles
        .iter()
        .find(|s| s.name == "schedule")
        .unwrap();
    assert_eq!(*schedule, sheet.table_style("schedule").unwrap());
}

#[test]
fn a_table_in_standard_writes_the_standard_style_of_its_unit() {
    let dir = scratch("standard");
    let text = "units: in
objects:
  - table: {position: [0, 5, 0], columns: [2, 3], rows: [0.5], cells: [[ITEM, PUMP]]}
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
    let standard = read
        .table_styles
        .iter()
        .find(|s| s.name == "Standard")
        .unwrap();
    assert_eq!(*standard, TableStyle::standard(agentdrafter_core::Unit::In));
    assert_eq!(read.objects, parse_sheet_description(text).unwrap().objects);
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
fn the_checker_holds_each_cells_text_to_its_cell_as_autocad_grows_its_row() {
    let dir = scratch("overflow");
    // A clean table, its trim line round the paper.
    let clean = format!(
        "{SCHEDULE}  - polyline: {{points: [[0, 0, 0], [200, 0, 0], [200, 100, 0], [0, 100, 0]], closed: true}}\n"
    );
    let path = dir.join("sheet.yaml");
    fs::write(&path, &clean).unwrap();
    let report = check(&path);
    assert!(report.passed, "{:#?}", report.findings);
    // A row of 4 for one line of 2.5 mm text: AutoCAD sets the line in
    // 4/3 of its height and both margins, 5.333, so it needs 1.333 more,
    // split about the middle of a middle-left cell.
    let low = clean.replacen("rows: [8, 6, 6, 6]", "rows: [8, 6, 4, 6]", 1);
    fs::write(&path, &low).unwrap();
    let report = check(&path);
    let found: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.rule == Rule::TextOverflow)
        .collect();
    // Every cell of that row with text: C-1, JB-1, MCC-1 and 4.
    assert_eq!(found.len(), 4, "{found:#?}");
    let first = found[0];
    let object = first.object.as_ref().unwrap();
    assert_eq!(object.id, "sheet/cables");
    assert_eq!(object.table_cell, Some([2, 0]));
    assert_eq!(object.text.as_deref(), Some("C-1"));
    let past = first.past.as_ref().unwrap();
    assert!((past.top - 2.0 / 3.0).abs() < 1e-9, "{past:?}");
    assert!((past.bottom - 2.0 / 3.0).abs() < 1e-9, "{past:?}");
    assert!(
        first
            .message
            .contains("the cell at row 2, column 0 of the table `sheet/cables`"),
        "{}",
        first.message
    );
    // Text that wraps to two lines needs 2.5 + 5/3 of 2.5 = 6.667 and the
    // margins: a 6 high row is short by 2.667.
    let long = clean.replacen("[C-1, JB-1,", "[C-1, JB-1 TERMINAL STRIP B,", 1);
    fs::write(&path, &long).unwrap();
    let report = check(&path);
    let found: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.rule == Rule::TextOverflow)
        .collect();
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].object.as_ref().unwrap().table_cell, Some([2, 1]));
    // Two lines, split about the middle: (6.667 - 4) / 2 each way.
    let past = found[0].past.as_ref().unwrap();
    assert!((past.top - 4.0 / 3.0).abs() < 1e-9, "{past:?}");
    assert!((past.bottom - 4.0 / 3.0).abs() < 1e-9, "{past:?}");
}

#[test]
fn a_template_takes_a_cells_text_from_its_row() {
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
        "rows:\n  - sheet: S-1\n    template: templates/t.yaml\n    values: {PUMP: P-101, VALVE: V-7}\n",
    )
    .unwrap();
    fs::write(
        set.join("templates/t.yaml"),
        "units: mm
objects:
  - table:
      position: [0, 50, 0]
      columns: [20, 40]
      rows: [10, 10]
      cells:
        - [ITEM, {row: PUMP}]
        - ['2', {text: {row: VALVE}, height: 3}]
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
    assert!(
        report.sheets[0].notices.is_empty(),
        "{:?}",
        report.sheets[0].notices
    );
    let read = read_dwg(&set.join("dwg/S-1.dwg")).unwrap();
    let Object::Table(table) = &read.objects[0] else {
        panic!("{read:?}");
    };
    assert_eq!(table.cells[0][1].text, "P-101");
    assert_eq!(table.cells[1][1].text, "V-7");
    assert_eq!(table.cells[1][1].height, Some(3.0));
}
