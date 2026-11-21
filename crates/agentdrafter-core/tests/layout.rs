//! Paper-space layouts and viewports: the reader takes a sheet's layout,
//! which space each object is in, and its viewports, and refuses what the
//! page setup cannot plot, naming the field; the DWG holds the layout, its
//! page setup and its viewports and reads them back; the one pipeline draws
//! paper space and model space through each viewport, scaled and cut at its
//! edge; the checker checks the layout on its paper; a query gives the
//! layout and its viewports; and change sets add viewports, change their
//! scale, view centre and lock, and move them. Every drawing here is made
//! up. AutoCAD 2026 opened, audited, regenerated, plotted and saved what
//! these write (see the pull request).

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build, build_set_with, change_with, check_with, display_list_with, operations,
    parse_sheet_description, plot_with, query_with, read_dwg, render_with,
    sheet_description_schema, Answer, BuildRequest, ChangeReport, ChangeRequest, CheckRequest,
    DisplayListRequest, ErrorCode, Item, Object, PlotRequest, Query, QueryRequest, RenderRequest,
    Reply, Rule, Selection, SelectionBox, SetBuildRequest, Settings, Shape,
};
use serde_json::{json, Value};

#[path = "support/pdf.rs"]
mod pdf;

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-layout")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A made-up panel 400 by 300, drawn at full size, sheeted on A4 at 1:4
/// with a 2:1 detail of a terminal.
const PANEL: &str = "units: mm
paper: ISO A4 landscape
layout:
  name: Panel
linetypes:
  DASHED: {pattern: [6, -3]}
layers:
  vports: {color: 8, plot: false}
  centre: {color: red, linetype: DASHED, lineweight: 0.5}
objects:
  - polyline: {id: panel, points: [[0, 0, 0], [400, 0, 0], [400, 300, 0], [0, 300, 0]], closed: true}
  - line: {id: axis, from: [-20, 150, 0], to: [420, 150, 0], layer: centre}
  - text: {id: tag, contents: PNL-1, position: [200, 200, 0], height: 20, justify: middle_center}
  - circle: {id: terminal, center: [50, 50, 0], radius: 5}
  - line: {id: far, from: [1000, 1000, 0], to: [1100, 1000, 0]}
  - polyline: {id: trim, space: paper, points: [[0, 0, 0], [297, 0, 0], [297, 210, 0], [0, 210, 0]], closed: true}
  - text: {id: title, space: paper, contents: PANEL LAYOUT, position: [200, 15, 0], height: 5}
  - viewport: {id: whole, center: [80, 105, 0], width: 120, height: 100, view_center: [200, 150, 0], scale: '1:4', layer: vports}
  - viewport: {id: detail, center: [220, 105, 0], width: 60, height: 60, view_center: [50, 50, 0], scale: 2, locked: true}
";

/// A model line along the bottom edge of a viewport, and one across it.
const TOUCHING_EDGE: &str = "units: mm
paper: ISO A4 landscape
layout: {name: Edge}
objects:
  - polyline: {points: [[0, 0, 0], [100, 0, 0], [100, 50, 0]]}
  - viewport: {center: [100, 100, 0], width: 100, height: 100, view_center: [50, 50, 0], scale: 1}
";

fn sheet_at(dir: &Path, text: &str) -> PathBuf {
    let path = dir.join("sheet.yaml");
    fs::write(&path, text).unwrap();
    path
}

fn built(dir: &Path, text: &str) -> PathBuf {
    let path = sheet_at(dir, text);
    build(&BuildRequest {
        description: path,
        output: None,
        overwrite: false,
    })
    .unwrap();
    dir.join("sheet.dwg")
}

#[test]
fn a_layout_its_spaces_and_its_viewports_read_with_what_they_give() {
    let sheet = parse_sheet_description(PANEL).unwrap();
    let layout = sheet.layout.as_ref().unwrap();
    assert_eq!(layout.name, "Panel");
    assert_eq!(
        layout.paper_space,
        [false, false, false, false, false, true, true, true, true]
    );
    let Object::Viewport(whole) = &sheet.objects[7] else {
        panic!("a viewport")
    };
    assert_eq!(whole.scale, 0.25);
    assert_eq!(whole.corners(), [[20.0, 55.0], [140.0, 155.0]]);
    assert_eq!(whole.view(), [[-40.0, -50.0], [440.0, 350.0]]);
    assert!(!whole.locked);
    let Object::Viewport(detail) = &sheet.objects[8] else {
        panic!("a viewport")
    };
    assert_eq!((detail.scale, detail.locked), (2.0, true));
    assert_eq!(detail.ratio(), "2:1");
    // A sheet with no layout has none, and every object in model space.
    let plain = parse_sheet_description("units: mm\nobjects: []\n").unwrap();
    assert!(plain.layout.is_none() && !plain.in_paper_space(0));
}

fn refused(text: &str) -> agentdrafter_core::Error {
    parse_sheet_description(text).unwrap_err()
}

#[test]
fn the_reader_refuses_a_layout_it_cannot_plot_and_names_the_field() {
    let with = |paper: &str, layout: &str, objects: &str| {
        format!("units: mm\n{paper}{layout}objects:\n{objects}")
    };
    let a4 = "paper: ISO A4 landscape\n";
    let named = "layout: {name: Panel}\n";
    let viewport = "  - viewport: {center: [100, 100, 0], width: 50, height: 50, view_center: [0, 0, 0], scale: '1:2'}\n";
    // A viewport, or an object in paper space, is on a sheet with a layout.
    let e = refused(&with(a4, "", viewport));
    assert_eq!(e.code, ErrorCode::BadValue);
    assert!(e.message.contains("declare the sheet's layout"), "{e:?}");
    let e = refused(&with(
        a4,
        "",
        "  - line: {from: [0, 0, 0], to: [1, 0, 0], space: paper}\n",
    ));
    assert_eq!(e.field.as_deref(), Some("objects[0].line"));
    // A layout plots a standard paper by name, in mm or in.
    let e = refused(&with("", named, "  []\n"));
    assert_eq!(
        (e.code, e.field.as_deref()),
        (ErrorCode::MissingKey, Some("layout"))
    );
    let e = refused(&with("paper: [300, 200]\n", named, "  []\n"));
    assert_eq!(e.field.as_deref(), Some("paper"));
    let e = refused("units: cm\npaper: ISO A4 landscape\nlayout: {name: Panel}\nobjects: []\n");
    assert!(e.message.contains("mm or in"), "{e:?}");
    // A layout's name is one AutoCAD takes.
    for name in ["Model", "'a/b'", "''"] {
        let e = refused(&with(a4, &format!("layout: {{name: {name}}}\n"), "  []\n"));
        assert_eq!(e.field.as_deref(), Some("layout.name"), "{name}: {e:?}");
    }
    // A viewport is in paper space, flat, and has a scale more than 0.
    let e = refused(&with(
        a4,
        named,
        "  - viewport: {center: [100, 100, 0], width: 50, height: 50, view_center: [0, 0, 0], scale: '1:2', space: model}\n",
    ));
    assert_eq!(e.field.as_deref(), Some("objects[0].viewport.space"));
    let e = refused(&with(
        a4,
        named,
        "  - viewport: {center: [100, 100, 0], width: 50, height: 50, view_center: [0, 0, 5], scale: '1:2'}\n",
    ));
    assert_eq!(e.field.as_deref(), Some("objects[0].viewport.view_center"));
    for scale in ["0", "'1:0'", "'a:b'", "'-1'"] {
        let e = refused(&with(
            a4,
            named,
            &format!("  - viewport: {{center: [100, 100, 0], width: 50, height: 50, view_center: [0, 0, 0], scale: {scale}}}\n"),
        ));
        assert_eq!(
            (e.code, e.field.as_deref()),
            (ErrorCode::BadNumber, Some("objects[0].viewport.scale")),
            "{scale}"
        );
    }
    let e = refused(&with(
        a4,
        named,
        "  - line: {from: [0, 0, 0], to: [1, 0, 0], space: sheet}\n",
    ));
    assert_eq!(e.field.as_deref(), Some("objects[0].line.space"));
}

#[test]
fn a_layout_and_its_viewports_read_back_from_the_dwg() {
    let dir = scratch("round-trip");
    let dwg = built(&dir, PANEL);
    let read = read_dwg(&dwg).unwrap();
    let sheet = parse_sheet_description(PANEL).unwrap();
    assert_eq!(read.objects, sheet.objects);
    assert_eq!(read.layout, sheet.layout);
    assert_eq!(read.paper, sheet.paper);
    assert_eq!(
        (read.objects_not_read, read.layouts_not_read),
        (0, 0),
        "{read:?}"
    );
    let ids: Vec<&str> = read
        .ids_and_names
        .iter()
        .map(|i| i.id.as_deref().unwrap())
        .collect();
    assert_eq!(ids[7..], ["sheet/whole", "sheet/detail"]);
    // A sheet with no layout opens on model space, with none read.
    let plain = built(
        &scratch("plain"),
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [10, 0, 0]}\n",
    );
    let read = read_dwg(&plain).unwrap();
    assert!(read.layout.is_none() && read.paper.is_none());
    assert_eq!(read.layouts_not_read, 0);
}

#[test]
fn an_inch_layout_and_a_millimetre_one_on_inch_paper_read_back() {
    for (name, text) in [
        (
            "inch",
            "units: in
paper: ANSI B landscape
layout: {name: Sheet 1}
objects:
  - circle: {center: [24, 24, 0], radius: 6}
  - viewport: {center: [8.5, 5.5, 0], width: 12, height: 8, view_center: [24, 24, 0], scale: '1:4'}
  - polyline: {space: paper, points: [[0, 0, 0], [17, 0, 0], [17, 11, 0], [0, 11, 0]], closed: true}
",
        ),
        (
            "mixed",
            "units: mm
paper: ANSI A portrait
layout: {name: Tall}
objects:
  - circle: {center: [100, 100, 0], radius: 40}
  - viewport: {center: [100, 140, 0], width: 150, height: 150, view_center: [100, 100, 0], scale: '3:8'}
",
        ),
    ] {
        let dwg = built(&scratch(name), text);
        let read = read_dwg(&dwg).unwrap();
        let sheet = parse_sheet_description(text).unwrap();
        assert_eq!(read.objects, sheet.objects, "{name}");
        assert_eq!(read.layout, sheet.layout, "{name}");
        assert_eq!(read.paper, sheet.paper, "{name}");
        assert_eq!(read.layouts_not_read, 0, "{name}");
    }
}

fn drawn(path: &Path) -> agentdrafter_core::DisplayListReport {
    display_list_with(&DisplayListRequest::new(path), &Settings::default()).unwrap()
}

fn points(items: &[&Item]) -> Vec<[f64; 2]> {
    items.iter().flat_map(|i| i.points()).collect()
}

#[test]
fn the_pipeline_draws_model_space_through_each_viewport_scaled_and_cut_at_its_edge() {
    let dir = scratch("drawn");
    let path = sheet_at(&dir, PANEL);
    let report = drawn(&path);
    let list = &report.display_list;
    let of = |object: usize, viewport: Option<usize>| -> Vec<&Item> {
        list.items
            .iter()
            .filter(|i| i.object == object && i.viewport == viewport)
            .collect()
    };
    // The panel shows through the whole view at 1:4, round its centre.
    let panel = points(&of(0, Some(7)));
    let close = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9;
    assert!(panel.iter().any(|&p| close(p, [30.0, 67.5])), "{panel:?}");
    assert!(panel.iter().any(|&p| close(p, [130.0, 142.5])), "{panel:?}");
    // Through the detail at 2:1, cut at its edge: nothing past it.
    let detail = points(&of(3, Some(8)));
    assert!(!detail.is_empty());
    for p in &detail {
        assert!(p[0] >= 190.0 - 1e-9 && p[0] <= 250.0 + 1e-9, "{p:?}");
        assert!(p[1] >= 75.0 - 1e-9 && p[1] <= 135.0 + 1e-9, "{p:?}");
    }
    // A model object no viewport shows draws nothing; paper space draws as
    // it is, and a viewport draws its frame on the paper.
    assert!(list.items.iter().all(|i| i.object != 4));
    assert!(points(&of(5, None))
        .iter()
        .any(|&p| close(p, [297.0, 210.0])));
    assert!(points(&of(8, None))
        .iter()
        .any(|&p| close(p, [250.0, 135.0])));
    // AutoCAD 2026 plotted a viewport's frame as its thinnest line,
    // whatever its lineweight.
    for frame in of(8, None) {
        let Shape::Stroke { lineweight, .. } = &frame.shape else {
            panic!("a stroke")
        };
        assert_eq!(*lineweight, 0.0);
    }
    // A line along a viewport's edge is left out, as AutoCAD left it out:
    // the panel's bottom edge, y 0, lies along nothing; through the whole
    // view its edges are inside.
    let edge = TOUCHING_EDGE;
    let path = sheet_at(&scratch("edge"), edge);
    let along = drawn(&path).display_list;
    assert!(
        along
            .items
            .iter()
            .all(|i| i.object != 0 || i.points().all(|p| p[1] > 50.0 + 1e-9)),
        "the edge along the viewport's bottom is left out"
    );
    // A linetype's dashes are as long on the paper as in paper space: the
    // axis's dashes past its ends, which are laid out to draw alike, are 6
    // long whatever the scale.
    let axis = of(1, Some(7));
    let Shape::Stroke { lineweight, runs } = &axis[0].shape else {
        panic!("a stroke")
    };
    assert_eq!(*lineweight, 0.5, "a lineweight is a width on paper");
    let inner = &runs[1];
    let dash = (inner[1][0] - inner[0][0]).abs();
    assert!((dash - 6.0).abs() < 1e-6, "{dash}");
    // Text scales with the viewport: 20 tall at 1:4 is 5 on the paper.
    let tag = list
        .text
        .iter()
        .find(|r| r.object == 2 && r.viewport == Some(7))
        .unwrap();
    let up = tag.up[0].hypot(tag.up[1]);
    assert!((up - 5.0).abs() < 1e-9, "{up}");
    // The page is the paper.
    assert_eq!(report.paper.unwrap().width, 297.0);
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
fn the_checker_checks_a_layout_on_its_paper_through_its_viewports() {
    let dir = scratch("checked");
    // The model's tag shows through the whole view where the paper's note
    // sits; a viewport runs off the paper; and model text cut at the
    // detail's edge does not count where it does not print.
    let text = PANEL.replace(
        "  - text: {id: title, space: paper, contents: PANEL LAYOUT, position: [200, 15, 0], height: 5}\n",
        "  - text: {id: title, space: paper, contents: PANEL LAYOUT, position: [200, 15, 0], height: 5}
  - text: {id: note, space: paper, contents: NOTE, position: [80, 117.5, 0], height: 3, justify: middle_center}
  - viewport: {id: stray, center: [290, 190, 0], width: 30, height: 30, view_center: [0, 0, 0], scale: 1}
  - text: {id: beside, space: paper, contents: X, position: [252, 128, 0], height: 3}
  - text: {id: cut, contents: CUT TEXT HERE, position: [60, 80, 0], height: 4}
",
    );
    let path = sheet_at(&dir, &text);
    let report = check(&path);
    let rules: Vec<(Rule, String, Option<String>)> = report
        .findings
        .iter()
        .filter_map(|f| {
            let object = f.object.as_ref()?;
            Some((f.rule, object.id.clone(), object.viewport.clone()))
        })
        .collect();
    assert!(
        rules.contains(&(Rule::TextOnText, "sheet/note".to_owned(), None))
            || rules.contains(&(
                Rule::TextOnText,
                "sheet/tag".to_owned(),
                Some("sheet/whole".to_owned())
            )),
        "{rules:#?}"
    );
    let on_text = report
        .findings
        .iter()
        .find(|f| f.rule == Rule::TextOnText)
        .unwrap();
    let both = [on_text.object.as_ref(), on_text.other.as_ref()];
    assert!(
        both.iter()
            .any(|o| o.unwrap().viewport.as_deref() == Some("sheet/whole")),
        "{on_text:#?}"
    );
    assert!(
        rules.contains(&(Rule::OffSheet, "sheet/stray".to_owned(), None)),
        "{rules:#?}"
    );
    assert!(
        !rules
            .iter()
            .any(|(_, id, _)| id == "sheet/beside" || id == "sheet/cut"),
        "{rules:#?}"
    );
    // The panel itself passes on its paper.
    let fine = check(&sheet_at(&scratch("fine"), PANEL));
    assert!(fine.passed, "{:#?}", fine.findings);
}

fn query(path: &Path, query: Query) -> agentdrafter_core::QueryReport {
    query_with(
        &QueryRequest {
            source: path.to_owned(),
            sheet: None,
            query,
        },
        &Settings::default(),
    )
    .unwrap()
}

#[test]
fn a_query_gives_the_layout_its_viewports_and_selects_by_each_space() {
    let dir = scratch("query");
    let path = sheet_at(&dir, PANEL);
    let report = query(&path, Query::Layout);
    let Answer::Layout { count, layouts } = &report.answer else {
        panic!("a layout answer")
    };
    assert_eq!(*count, 2);
    let layout = &layouts[0];
    assert_eq!(layout.layout.as_deref(), Some("Panel"));
    assert_eq!((layout.paper_space, layout.model_space), (4, 5));
    let setup = layout.page_setup.as_ref().unwrap();
    assert_eq!(setup.paper, "ISO_full_bleed_A4_(297.00_x_210.00_MM)");
    assert_eq!(setup.scale, "1:1");
    let whole = &layout.viewports[0];
    assert_eq!(
        (whole.id.as_str(), whole.ratio.as_str()),
        ("sheet/whole", "1:4")
    );
    assert_eq!(whole.view, [[-40.0, -50.0], [440.0, 350.0]]);
    assert!(layout.viewports[1].locked);
    // A box selects model space at full size and paper space on the paper.
    let found = query(
        &path,
        Query::Find(Selection {
            window: Some(SelectionBox([[-1.0, -1.0], [401.0, 301.0]])),
            ..Selection::default()
        }),
    );
    let Answer::Find { objects, .. } = &found.answer else {
        panic!("a find answer")
    };
    let ids: Vec<&str> = objects.iter().map(|o| o.id.as_str()).collect();
    for id in [
        "sheet/panel",
        "sheet/tag",
        "sheet/terminal",
        "sheet/trim",
        "sheet/whole",
    ] {
        assert!(ids.contains(&id), "{id}: {ids:?}");
    }
    assert!(!ids.contains(&"sheet/far"), "{ids:?}");
    // The sheet's extents are what it prints: its paper's.
    let Answer::Extents { extents, .. } = &query(&path, Query::Extents).answer else {
        panic!("an extents answer")
    };
    let e = extents[0].extents.unwrap();
    assert_eq!((e.min, e.max), ([0.0, 0.0], [297.0, 210.0]));
}

fn panel_set(test: &str) -> PathBuf {
    let set = scratch(test);
    fs::create_dir_all(set.join("templates")).unwrap();
    let write = |name: &str, text: &str| fs::write(set.join(name), text).unwrap();
    write("set.yaml", "units: mm\noutput: dwg\nrows: rows.yaml\n");
    write(
        "rows.yaml",
        "rows:\n  - sheet: PNL-001\n    template: templates/panel.yaml\n",
    );
    write("templates/panel.yaml", PANEL);
    let built = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: false,
            clean: None,
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
    fs::read_to_string(set.join("templates/panel.yaml")).unwrap()
}

#[test]
fn viewports_are_added_and_changed_by_their_own_operations_with_autocads_alias() {
    let listed = operations();
    for (name, alias) in [("add_viewport", "MVIEW"), ("change_viewport", "")] {
        let info = listed.operations.iter().find(|o| o.name == name).unwrap();
        assert_eq!(info.alias, alias, "{name}");
    }
    let info = listed
        .operations
        .iter()
        .find(|o| o.name == "change_viewport")
        .unwrap();
    let validator = jsonschema::validator_for(&info.schema).unwrap();
    assert!(validator.is_valid(&json!({
        "op": "change_viewport", "target": {"id": "S/v"}, "scale": "1:2",
        "view_center": {"snap": {"id": "S/hole", "snap": "center"}}
    })));
    assert!(!validator.is_valid(&json!({"op": "change_viewport", "target": {"id": "S/v"}})));

    // A template drawn in a layout builds, and change sets edit its viewports.
    let set = panel_set("changed");
    let read = read_dwg(&set.join("dwg/PNL-001.dwg")).unwrap();
    assert_eq!(read.layout.unwrap().name, "Panel");
    let reply = apply(
        &set,
        json!([
            {
                "op": "MVIEW", "template": "templates/panel.yaml", "id": "corner",
                "center": [260, 170, 0], "width": 40, "height": 40,
                "view_center": [400, 300, 0], "scale": "1:1", "layer": "vports"
            },
            {"op": "add_text", "template": "templates/panel.yaml", "id": "corner-label", "space": "paper",
             "contents": "CORNER", "position": [245, 195, 0], "height": 3},
            {"op": "change_viewport", "target": {"id": "PNL-001/whole"}, "scale": "1:5",
             "view_center": {"snap": {"id": "PNL-001/terminal", "snap": "center"}}},
            {"op": "move", "target": {"id": "PNL-001/whole"}, "by": [5, 0, 0]}
        ]),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    let written = template(&set);
    for expected in [
        "id: corner
",
        "space: paper",
        "scale: '1:5'",
        "view_center: [50, 50, 0]",
        "center: [85, 105, 0]",
    ] {
        assert!(written.contains(expected), "{expected}: {written}");
    }
    let read = read_dwg(&set.join("dwg/PNL-001.dwg")).unwrap();
    let viewports: Vec<_> = read
        .objects
        .iter()
        .filter_map(|o| match o {
            Object::Viewport(v) => Some(v),
            _ => None,
        })
        .collect();
    assert_eq!(viewports.len(), 3);
    assert_eq!((viewports[0].scale, viewports[0].center.x), (0.2, 85.0));

    // A locked viewport keeps its view unless the change unlocks it.
    let reply = apply(
        &set,
        json!([{"op": "change_viewport", "target": {"id": "PNL-001/detail"}, "scale": 1}]),
    );
    let error = reply.error.unwrap();
    assert!(
        error.message.contains("`PNL-001/detail` is locked"),
        "{error:?}"
    );
    let reply = apply(
        &set,
        json!([{"op": "change", "target": {"id": "PNL-001/detail"}, "set": {"view_center": [60, 60, 0]}}]),
    );
    assert!(reply.error.unwrap().message.contains("is locked"));
    let reply = apply(
        &set,
        json!([{"op": "change_viewport", "target": {"id": "PNL-001/detail"}, "scale": 1, "locked": false}]),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    // change_viewport changes a viewport alone.
    let reply = apply(
        &set,
        json!([{"op": "change_viewport", "target": {"id": "PNL-001/panel"}, "locked": true}]),
    );
    let error = reply.error.unwrap();
    assert!(
        error.message.contains("change_viewport changes a viewport"),
        "{error:?}"
    );
}

/// A line 2.11 mm wide across a viewport whose window runs from x 100 to
/// 120 mm: its ink, round ends among it, stops at the window's edge.
const THICK: &str = "units: mm
paper: ISO A4 landscape
layout: {name: Thick}
objects:
  - line: {id: x, from: [-20, 0, 0], to: [20, 0, 0], color: red, lineweight: 2.11}
  - viewport: {id: v, center: [110, 110, 0], width: 20, height: 20, view_center: [0, 0, 0], scale: 1}
";

#[test]
fn a_viewport_cuts_its_contents_ink_at_its_edge_in_every_output() {
    let dir = scratch("thick");
    let path = sheet_at(&dir, THICK);
    // Each item model space draws through the viewport keeps its window.
    let list = drawn(&path).display_list;
    let line: Vec<&Item> = list.items.iter().filter(|i| i.object == 0).collect();
    assert!(!line.is_empty());
    for item in line {
        let clip = item.clip.unwrap();
        assert_eq!((clip.min, clip.max), ([100.0, 100.0], [120.0, 120.0]));
    }
    // The PNG, at 10 pixels to the millimetre, draws no ink past x 120 mm.
    let png = dir.join("thick.png");
    let mut request = RenderRequest::new(&path);
    request.output = Some(png.clone());
    request.dpi = 254.0;
    request.scale = Some(1.0);
    let report = render_with(&request, &Settings::default()).unwrap();
    let decoder = png::Decoder::new(std::io::BufReader::new(fs::File::open(&png).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    let at = |x: f64, y: f64| {
        let column = ((x - report.top_left[0]) * 10.0).floor() as usize;
        let row = ((report.top_left[1] - y) * 10.0).floor() as usize;
        let i = (row * info.width as usize + column) * 3;
        [pixels[i], pixels[i + 1], pixels[i + 2]]
    };
    assert_eq!(at(110.0, 110.0), [255, 0, 0], "the line inside the window");
    assert_eq!(
        at(120.5, 110.0),
        [255, 255, 255],
        "its round end past the edge"
    );
    // The PDF draws it inside a clipping path round the window.
    let pdf_path = dir.join("thick.pdf");
    let mut request = PlotRequest::new(&path);
    request.output = Some(pdf_path.clone());
    plot_with(&request, &Settings::default()).unwrap();
    let clips = pdf::Pdf::read(fs::read(&pdf_path).unwrap()).clips();
    let mm = 72.0 / 25.4;
    assert_eq!(clips.len(), 1, "{clips:?}");
    for (got, want) in clips[0]
        .iter()
        .zip([100.0 * mm, 100.0 * mm, 20.0 * mm, 20.0 * mm])
    {
        assert!((got - want).abs() < 0.01, "{clips:?}");
    }
}

#[test]
fn change_moves_an_object_to_paper_space_as_it_stands() {
    let set = panel_set("change-space");
    let reply = apply(
        &set,
        json!([{"op": "change", "target": {"id": "PNL-001/tag"}, "set": {"space": "paper"}}]),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    let written = template(&set);
    assert!(
        written.contains("contents: PNL-1, position: [200, 200, 0], height: 20, justify: middle_center, space: paper}"),
        "{written}"
    );
    let read = read_dwg(&set.join("dwg/PNL-001.dwg")).unwrap();
    assert!(read.layout.unwrap().paper_space[2]);
}

#[test]
fn a_scale_whose_view_a_dwg_cannot_hold_is_refused() {
    let with = |height: &str, scale: &str| {
        format!(
            "units: mm\npaper: ISO A4 landscape\nlayout: {{name: Demo}}\nobjects:\n  - viewport: {{center: [110, 110, 0], width: 20, height: {height}, view_center: [0, 0, 0], scale: {scale}}}\n"
        )
    };
    for (height, scale) in [
        ("20", "1e-309"),
        ("1e-200", "1e200"),
        ("20", "1e-320"),
        ("20", "'1e-200:1e200'"),
    ] {
        let e = refused(&with(height, scale));
        assert_eq!(
            (e.code, e.field.as_deref()),
            (ErrorCode::BadNumber, Some("objects[0].viewport.scale")),
            "{height} at {scale}: {e:?}"
        );
    }
    // A scale near the largest a DWG holds reads back.
    let text = with("20", "1e300");
    let dwg = built(&scratch("huge-scale"), &text);
    let read = read_dwg(&dwg).unwrap();
    assert_eq!(read.objects_not_read, 0, "{read:?}");
    assert_eq!(
        read.objects,
        parse_sheet_description(&text).unwrap().objects
    );
}

#[test]
fn model_space_far_off_the_view_is_cut_without_overflowing() {
    let dir = scratch("far");
    let path = sheet_at(
        &dir,
        "units: mm
paper: ISO A4 landscape
layout: {name: Far}
linetypes:
  DASH: {pattern: [1, -1]}
objects:
  - line: {from: [-1e308, 5, 0], to: [1e308, 5, 0], linetype: DASH}
  - viewport: {center: [110, 110, 0], width: 20, height: 20, view_center: [0, 0, 0], scale: 1}
",
    );
    let list = drawn(&path).display_list;
    let points = points(
        &list
            .items
            .iter()
            .filter(|i| i.object == 0)
            .collect::<Vec<_>>(),
    );
    assert!(!points.is_empty());
    for p in &points {
        assert!(p.iter().all(|v| v.is_finite()), "{points:?}");
        assert!(
            (100.0..=120.0).contains(&p[0]) && p[1] == 115.0,
            "{points:?}"
        );
    }
}

#[test]
fn the_schema_takes_a_layout_and_a_viewport_as_the_reader_does() {
    let schema = sheet_description_schema();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let sheet = |layout: Value, object: Value| json!({"units": "mm", "paper": "ISO A4 landscape", "layout": layout, "objects": [object]});
    let viewport = |key: &str, value: Value| {
        let mut v = json!({"center": [110, 110, 0], "width": 20, "height": 20, "view_center": [0, 0, 0], "scale": 1});
        v[key] = value;
        json!({ "viewport": v })
    };
    let line = |space: Value| json!({"line": {"from": [0, 0, 0], "to": [1, 0, 0], "space": space}});
    let named = |name: Value| json!({ "name": name });
    const LINE: &str = "  - line: {from: [0, 0, 0], to: [1, 0, 0], space: ";
    const VIEWPORT: &str =
        "  - viewport: {center: [110, 110, 0], width: 20, height: 20, view_center: [0, 0, 0], ";
    // Each case as JSON, as the YAML the reader reads, and whether both
    // take it.
    let cases: Vec<(Value, String, bool)> = vec![
        (
            sheet(named(json!("A:B")), line(json!("paper"))),
            format!("layout: {{name: 'A:B'}}\nobjects:\n{LINE}paper}}"),
            false,
        ),
        (
            sheet(named(json!(123)), line(json!("paper"))),
            format!("layout: {{name: 123}}\nobjects:\n{LINE}paper}}"),
            true,
        ),
        (
            sheet(named(json!("Panel")), line(json!(null))),
            format!("layout: {{name: Panel}}\nobjects:\n{LINE}null}}"),
            false,
        ),
        (
            sheet(named(json!("Panel")), viewport("scale", json!("1:0"))),
            format!("layout: {{name: Panel}}\nobjects:\n{VIEWPORT}scale: '1:0'}}"),
            false,
        ),
        (
            sheet(named(json!("Panel")), viewport("scale", json!("0.5:2"))),
            format!("layout: {{name: Panel}}\nobjects:\n{VIEWPORT}scale: '0.5:2'}}"),
            true,
        ),
        (
            sheet(named(json!("Panel")), viewport("locked", json!("true"))),
            format!("layout: {{name: Panel}}\nobjects:\n{VIEWPORT}scale: 1, locked: 'true'}}"),
            true,
        ),
        (
            sheet(named(json!("Panel")), viewport("locked", json!("yes"))),
            format!("layout: {{name: Panel}}\nobjects:\n{VIEWPORT}scale: 1, locked: 'yes'}}"),
            false,
        ),
    ];
    for (json, yaml, takes) in cases {
        let text = format!("units: mm\npaper: ISO A4 landscape\n{yaml}\n");
        let read = parse_sheet_description(&text);
        assert_eq!(read.is_ok(), takes, "the reader on {yaml}: {read:?}");
        assert_eq!(validator.is_valid(&json), takes, "the schema on {json}");
    }
}

#[test]
fn query_layout_gives_model_spaces_extents_as_query_extents_does() {
    let hatch = "hatch_patterns:
  SPARSE:
    lines:
      - {angle: 0, origin: [0, 0], delta: [0, 1000]}
objects:
  - hatch: {id: area, pattern: SPARSE, boundary: [{polyline: {points: [[0, 10, 0], [20, 10, 0], [20, 20, 0], [0, 20, 0]]}}]}
";
    let dir = scratch("hatch-extents");
    let laid = sheet_at(
        &dir,
        &format!(
            "units: mm\npaper: ISO A4 landscape\nlayout: {{name: Demo}}\n{hatch}  - viewport: {{center: [110, 110, 0], width: 40, height: 40, view_center: [10, 15, 0], scale: 1}}\n"
        ),
    );
    let Answer::Layout { layouts, .. } = query(&laid, Query::Layout).answer else {
        panic!("a layout answer")
    };
    let alone = dir.join("alone.yaml");
    fs::write(&alone, format!("units: mm\n{hatch}")).unwrap();
    let Answer::Extents { extents, .. } = query(&alone, Query::Extents).answer else {
        panic!("an extents answer")
    };
    let (laid, alone) = (
        layouts[0].model_extents.unwrap(),
        extents[0].extents.unwrap(),
    );
    assert_eq!((laid.min, laid.max), (alone.min, alone.max));
    assert_eq!((laid.min, laid.max), ([0.0, 10.0], [20.0, 20.0]));
}

#[test]
fn a_line_along_no_edge_shows_at_any_enlargement() {
    // A line 4e-10 long seen at 1e11 is 40 long on the paper, and runs
    // right across the 20 wide window: the tolerance a line along an edge
    // is left out within is a length on the paper, not in model space.
    let dir = scratch("enlarged");
    let path = sheet_at(
        &dir,
        "units: mm
paper: ISO A4 landscape
layout: {name: Small}
objects:
  - line: {from: [-2e-10, 0, 0], to: [2e-10, 0, 0], color: red, lineweight: 0.5}
  - viewport: {center: [110, 110, 0], width: 20, height: 20, view_center: [0, 0, 0], scale: 1e11}
",
    );
    let list = drawn(&path).display_list;
    let line: Vec<&Item> = list.items.iter().filter(|i| i.object == 0).collect();
    let points = points(&line);
    assert_eq!(points.len(), 2, "{points:?}");
    let close = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9;
    assert!(
        close(points[0], [100.0, 110.0]) && close(points[1], [120.0, 110.0]),
        "{points:?}"
    );
}
