//! More than one layout in a DWG, clipped viewports and layers frozen in
//! one viewport: the reader takes a sheet drawn in several layouts, each on
//! its own paper, which space names each object's layout, a viewport's
//! clipping boundary and the layers frozen in it, and refuses what AutoCAD
//! would not hold, naming the field; the DWG holds every layout, boundary
//! and frozen list and reads them back; the one pipeline cuts a viewport's
//! contents at its boundary and leaves out the layers frozen in it; a
//! render writes a PNG and a plot a page for each layout; the checker
//! checks each layout and names it; a query lists every layout and selects
//! by space; and change sets clip a viewport and set its frozen layers.
//! Every drawing here is made up. AutoCAD 2026 opened, audited,
//! regenerated, plotted and saved what these write (see the pull request).

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build, build_set_with, change_with, check_with, display_list_with, operations,
    parse_sheet_description, plot_with, query_with, read_dwg, render_with,
    sheet_description_schema, Answer, BuildRequest, ChangeReport, ChangeRequest, CheckRequest,
    Clip, DisplayListRequest, ErrorCode, Object, PlotRequest, Query, QueryRequest, RenderRequest,
    Reply, Selection, SelectionBox, SetBuildRequest, Settings, Shape, Viewport,
};
use serde_json::{json, Value};

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-layouts")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A made-up panel 400 by 300 with a wire across it, sheeted in two
/// layouts: the whole panel at 1:2 on A4, and on A3 laid upright a round
/// detail at 1:1 and the panel at 1:2 with its wire frozen.
const TWO: &str = "units: mm
layouts:
  - {name: Front, paper: ISO A4 landscape}
  - {name: Back, paper: ISO A3 portrait}
layers:
  wires: {color: red}
  vp: {color: 8}
objects:
  - polyline: {id: panel, points: [[0, 0, 0], [400, 0, 0], [400, 300, 0], [0, 300, 0]], closed: true}
  - line: {id: wire, from: [0, 150, 0], to: [400, 150, 0], layer: wires}
  - circle: {id: dot, center: [200, 150, 0], radius: 10}
  - polyline: {id: front-trim, space: Front, points: [[0, 0, 0], [297, 0, 0], [297, 210, 0], [0, 210, 0]], closed: true}
  - viewport: {id: front-view, space: Front, center: [148.5, 105, 0], width: 210, height: 160, view_center: [200, 150, 0], scale: '1:2'}
  - polyline: {id: back-trim, space: Back, points: [[0, 0, 0], [297, 0, 0], [297, 420, 0], [0, 420, 0]], closed: true}
  - viewport: {id: back-round, space: Back, clip: {center: [148.5, 300, 0], radius: 50}, view_center: [200, 150, 0], scale: 1, layer: vp}
  - viewport: {id: back-bare, space: Back, center: [148.5, 120, 0], width: 220, height: 170, view_center: [200, 150, 0], scale: '1:2', frozen_layers: [wires], layer: vp}
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

fn viewport(object: &Object) -> &Viewport {
    match object {
        Object::Viewport(v) => v,
        other => panic!("a viewport, not {other:?}"),
    }
}

#[test]
fn a_sheet_reads_in_several_layouts_each_on_its_paper() {
    let sheet = parse_sheet_description(TWO).unwrap();
    let names: Vec<&str> = sheet.layouts().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Front", "Back"]);
    let front = sheet.layout.as_ref().unwrap();
    let back = &sheet.other_layouts[0];
    assert_eq!(
        front.paper_space,
        [false, false, false, true, true, false, false, false]
    );
    assert_eq!(
        back.paper_space,
        [false, false, false, false, false, true, true, true]
    );
    let size = |l: &agentdrafter_core::Layout| {
        let p = l.paper.as_ref().unwrap();
        (p.name.clone().unwrap(), p.width, p.height)
    };
    assert_eq!(size(front), ("ISO A4 landscape".into(), 297.0, 210.0));
    assert_eq!(size(back), ("ISO A3 portrait".into(), 297.0, 420.0));
    // The sheet's own paper is its first layout's.
    assert_eq!(sheet.paper, front.paper);
    assert!(sheet.in_model_space(1) && !sheet.in_model_space(6));
    // A clipped viewport's window is the box round its boundary.
    let round = viewport(&sheet.objects[6]);
    assert_eq!(
        round.clip,
        Some(Clip::Circle {
            center: agentdrafter_core::Point3::new(148.5, 300.0, 0.0),
            radius: 50.0
        })
    );
    assert_eq!(round.corners(), [[98.5, 250.0], [198.5, 350.0]]);
    assert_eq!(viewport(&sheet.objects[7]).frozen_layers, ["wires"]);
    // Each layout as it prints: itself first, on its own paper, every
    // object in its place.
    let pages = sheet.pages();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[1].layout.as_ref().unwrap().name, "Back");
    assert_eq!(pages[1].paper, back.paper);
    assert_eq!(pages[1].objects.len(), sheet.objects.len());
    assert_eq!(
        pages[1].viewports().map(|(i, _)| i).collect::<Vec<_>>(),
        [6, 7]
    );
    assert_eq!(
        pages[0].viewports().map(|(i, _)| i).collect::<Vec<_>>(),
        [4]
    );
    // A layout's own paper, or the sheet's when it gives none.
    let shared = parse_sheet_description(
        "units: mm\npaper: ISO A3 landscape\nlayouts: [{name: A}, {name: B, paper: ISO A4 portrait}]\nobjects: []\n",
    )
    .unwrap();
    let papers: Vec<String> = shared
        .layouts()
        .map(|l| l.paper.as_ref().unwrap().name.clone().unwrap())
        .collect();
    assert_eq!(papers, ["ISO A3 landscape", "ISO A4 portrait"]);
}

fn refused(text: &str) -> agentdrafter_core::Error {
    parse_sheet_description(text).unwrap_err()
}

#[test]
fn the_reader_refuses_layouts_spaces_clips_and_frozen_layers_it_cannot_hold() {
    let two = "units: mm\nlayouts: [{name: A, paper: ISO A4 landscape}, {name: B, paper: ISO A4 portrait}]\n";
    let cases: Vec<(String, ErrorCode, &str)> = vec![
        // `layout` and `layouts` together.
        (
            "units: mm\npaper: ISO A4 landscape\nlayout: {name: A}\nlayouts: [{name: B}, {name: C}]\nobjects: []\n".into(),
            ErrorCode::BadValue,
            "layouts",
        ),
        // One layout under `layouts`.
        (
            "units: mm\nlayouts: [{name: A, paper: ISO A4 landscape}]\nobjects: []\n".into(),
            ErrorCode::BadValue,
            "layouts",
        ),
        // Two layouts named alike, whatever the case.
        (
            "units: mm\nlayouts: [{name: Panel, paper: ISO A4 landscape}, {name: PANEL, paper: ISO A4 portrait}]\nobjects: []\n".into(),
            ErrorCode::DuplicateName,
            "layouts[1].name",
        ),
        // A layout with no paper, its own or the sheet's.
        (
            "units: mm\nlayouts: [{name: A, paper: ISO A4 landscape}, {name: B}]\nobjects: []\n".into(),
            ErrorCode::MissingKey,
            "layouts[1]",
        ),
        // A paper given by its size.
        (
            "units: mm\nlayouts: [{name: A, paper: [100, 100]}, {name: B, paper: ISO A4 portrait}]\nobjects: []\n".into(),
            ErrorCode::BadValue,
            "layouts[0].paper",
        ),
        // `paper` names no one layout of several.
        (
            format!("{two}objects:\n  - line: {{from: [0, 0, 0], to: [1, 0, 0], space: paper}}\n"),
            ErrorCode::BadValue,
            "objects[0].line.space",
        ),
        // A layout the sheet does not have.
        (
            format!("{two}objects:\n  - line: {{from: [0, 0, 0], to: [1, 0, 0], space: C}}\n"),
            ErrorCode::BadValue,
            "objects[0].line.space",
        ),
        // A viewport on a sheet of several layouts names its own.
        (
            format!("{two}objects:\n  - viewport: {{center: [50, 50, 0], width: 10, height: 10, view_center: [0, 0, 0], scale: 1}}\n"),
            ErrorCode::MissingKey,
            "objects[0].viewport",
        ),
        // A clipped viewport takes its window from its boundary.
        (
            format!("{two}objects:\n  - viewport: {{space: A, center: [50, 50, 0], clip: {{center: [50, 50, 0], radius: 5}}, view_center: [0, 0, 0], scale: 1}}\n"),
            ErrorCode::BadValue,
            "objects[0].viewport.center",
        ),
        // A boundary that encloses nothing.
        (
            format!("{two}objects:\n  - viewport: {{space: A, clip: {{points: [[0, 0, 0], [10, 0, 0], [20, 0, 0]]}}, view_center: [0, 0, 0], scale: 1}}\n"),
            ErrorCode::BadValue,
            "objects[0].viewport.clip",
        ),
        // A boundary's segment with a width.
        (
            format!("{two}objects:\n  - viewport: {{space: A, clip: {{points: [{{point: [0, 0, 0], start_width: 1}}, [10, 0, 0], [10, 10, 0]]}}, view_center: [0, 0, 0], scale: 1}}\n"),
            ErrorCode::BadValue,
            "objects[0].viewport.clip.points[0].start_width",
        ),
        // A boundary both ways.
        (
            format!("{two}objects:\n  - viewport: {{space: A, clip: {{center: [5, 5, 0], radius: 2, points: [[0, 0, 0], [10, 0, 0], [10, 10, 0]]}}, view_center: [0, 0, 0], scale: 1}}\n"),
            ErrorCode::BadValue,
            "objects[0].viewport.clip.center",
        ),
        // A layer the sheet does not define, frozen in a viewport.
        (
            format!("{two}objects:\n  - viewport: {{space: A, center: [50, 50, 0], width: 10, height: 10, view_center: [0, 0, 0], scale: 1, frozen_layers: [wires]}}\n"),
            ErrorCode::UnknownName,
            "objects[0].viewport.frozen_layers[0]",
        ),
        // A layer frozen twice.
        (
            format!("{two}layers: {{wires: {{color: red}}}}\nobjects:\n  - viewport: {{space: A, center: [50, 50, 0], width: 10, height: 10, view_center: [0, 0, 0], scale: 1, frozen_layers: [wires, wires]}}\n"),
            ErrorCode::BadValue,
            "objects[0].viewport.frozen_layers[1]",
        ),
    ];
    for (text, code, field) in cases {
        let e = refused(&text);
        assert_eq!(
            (e.code, e.field.as_deref()),
            (code, Some(field)),
            "{text}\n{e:?}"
        );
    }
    // On a sheet of one layout, `paper` and the layout's name are one space.
    let one = parse_sheet_description(
        "units: mm\npaper: ISO A4 landscape\nlayout: {name: Panel}\nobjects:\n  - line: {from: [0, 0, 0], to: [1, 0, 0], space: Panel}\n  - line: {from: [0, 0, 0], to: [1, 0, 0], space: paper}\n",
    )
    .unwrap();
    assert!(one.in_paper_space(0) && one.in_paper_space(1));
}

#[test]
fn every_layout_boundary_and_frozen_layer_reads_back_from_the_dwg() {
    let dir = scratch("round-trip");
    let dwg = built(&dir, TWO);
    let read = read_dwg(&dwg).unwrap();
    let sheet = parse_sheet_description(TWO).unwrap();
    assert_eq!(read.objects, sheet.objects);
    assert_eq!(read.layout, sheet.layout);
    assert_eq!(read.other_layouts, sheet.other_layouts);
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
    assert_eq!(ids[6..], ["sheet/back-round", "sheet/back-bare"]);
    // A polygon, and a polyline with an arc each way, read back too.
    let clipped = "units: in\npaper: ANSI B landscape\nlayout: {name: Plan}\nobjects:\n  - viewport: {clip: {points: [[1, 1, 0], [5, 1, 0], [3, 4, 0]]}, view_center: [0, 0, 0], scale: '1:48'}\n  - viewport: {clip: {points: [{point: [6, 1, 0], bulge: 0.5}, [10, 1, 0], {point: [10, 6, 0], arc_angle: -90}, [6, 6, 0]]}, view_center: [100, 0, 0], scale: '1:24'}\n";
    let dwg = built(&scratch("polygons"), clipped);
    let read = read_dwg(&dwg).unwrap();
    assert_eq!(
        read.objects,
        parse_sheet_description(clipped).unwrap().objects
    );
    assert_eq!(read.objects_not_read, 0);
}

#[test]
fn a_dwg_saved_on_the_model_tab_reads_and_plots_every_layout() {
    let dir = scratch("model-tab");
    let dwg = built(&dir, TWO);
    let read = read_dwg(&dwg).unwrap();
    assert_eq!(read.opens_on.as_deref(), Some("Front"));
    // The same drawing with only its Model tab made current, as AutoCAD
    // saves one left on the Model tab.
    let mut drawing = acadrust::DwgReader::from_file(&dwg).unwrap().read().unwrap();
    drawing.header.show_model_space = true;
    let on_model = dir.join("model-tab.dwg");
    fs::write(
        &on_model,
        acadrust::DwgWriter::write_to_vec(&drawing).unwrap(),
    )
    .unwrap();
    let again = read_dwg(&on_model).unwrap();
    assert_eq!(again.opens_on, None);
    assert_eq!(again.layout, read.layout);
    assert_eq!(again.other_layouts, read.other_layouts);
    assert_eq!(again.objects, read.objects);
    assert_eq!(again.layouts_not_read, 0);
    // Its plot is a page for each layout, as the drawing saved on one.
    let mut request = PlotRequest::new(&on_model);
    request.output = Some(dir.join("model-tab.pdf"));
    let report = plot_with(&request, &Settings::default()).unwrap();
    let pages: Vec<&str> = report.layouts.iter().map(|l| l.layout.as_str()).collect();
    assert_eq!(pages, ["Front", "Back"]);
    // A drawing of model space alone, AutoCAD's own empty layout beside
    // it, still reads as model space alone.
    let plain = built(
        &scratch("model-alone"),
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [10, 0, 0]}\n",
    );
    let read = read_dwg(&plain).unwrap();
    assert!(read.layout.is_none() && read.opens_on.is_none());
    assert_eq!(read.layouts_not_read, 0);
}

fn drawn(path: &Path, layout: Option<&str>) -> agentdrafter_core::DisplayListReport {
    let mut request = DisplayListRequest::new(path);
    request.layout = layout.map(str::to_owned);
    display_list_with(&request, &Settings::default()).unwrap()
}

#[test]
fn the_pipeline_cuts_at_a_boundary_and_leaves_out_layers_frozen_in_a_viewport() {
    let dir = scratch("drawn");
    let path = sheet_at(&dir, TWO);
    // The first layout draws by default; the reply lists both.
    let front = drawn(&path, None);
    assert_eq!(front.layout.as_deref(), Some("Front"));
    assert_eq!(front.layouts, ["Front", "Back"]);
    assert_eq!(front.paper.as_ref().unwrap().width, 297.0);
    let wire_through = |list: &agentdrafter_core::DisplayList, v: usize| {
        list.items
            .iter()
            .any(|i| i.object == 1 && i.viewport == Some(v))
    };
    assert!(wire_through(&front.display_list, 4));
    // Nothing of the other layout's paper space draws on this one.
    assert!(!front.display_list.items.iter().any(|i| i.object >= 5));
    let back = drawn(&path, Some("back"));
    assert_eq!(back.layout.as_deref(), Some("Back"));
    assert_eq!(back.paper.as_ref().unwrap().height, 420.0);
    let list = &back.display_list;
    assert!(!list.items.iter().any(|i| i.object == 3 || i.object == 4));
    // The wire shows through the round detail, not where it is frozen.
    assert!(wire_through(list, 6));
    assert!(!wire_through(list, 7));
    assert!(list
        .items
        .iter()
        .any(|i| i.object == 0 && i.viewport == Some(7)));
    // Through the round detail, every item is cut at the circle: each
    // names its boundary, and every point of its lines lies within it.
    assert_eq!(list.clip_boundaries.len(), 1);
    let seen: Vec<_> = list
        .items
        .iter()
        .filter(|i| i.viewport == Some(6))
        .collect();
    assert!(!seen.is_empty());
    for item in &seen {
        assert_eq!(item.clip_boundary, Some(0));
        if let Shape::Stroke { runs, .. } = &item.shape {
            for p in runs.iter().flatten() {
                let r = (p[0] - 148.5).hypot(p[1] - 300.0);
                assert!(r <= 50.0 + 1e-6, "{p:?} is {r} from the centre");
            }
        }
    }
    // The wire crosses the circle, and is cut at it.
    assert!(seen.iter().any(|i| i.object == 1));
    // The boundary is the viewport's frame, drawn with its own pen's
    // lineweight, as AutoCAD 2026 plotted a clipping boundary; a
    // rectangular viewport's frame is its thinnest line.
    let frame = |v: usize| {
        list.items
            .iter()
            .find(|i| i.object == v && i.viewport.is_none())
            .map(|i| match &i.shape {
                Shape::Stroke { lineweight, .. } => *lineweight,
                Shape::Fill { .. } => -1.0,
            })
            .unwrap()
    };
    assert_eq!(frame(6), 0.25);
    assert_eq!(frame(7), 0.0);
    // Asking for a layout the sheet lacks names its layouts.
    let mut request = DisplayListRequest::new(&path);
    request.layout = Some("Side".into());
    let e = display_list_with(&request, &Settings::default()).unwrap_err();
    assert_eq!(e.code, ErrorCode::UnknownName);
    assert!(e.message.contains("`Front`, `Back`"), "{}", e.message);
}

#[test]
fn a_render_writes_a_png_and_a_plot_a_page_for_each_layout() {
    let dir = scratch("outputs");
    let path = sheet_at(&dir, TWO);
    let report = render_with(&RenderRequest::new(&path), &Settings::default()).unwrap();
    assert_eq!(report.layout.as_deref(), Some("Front"));
    assert!(report.png.ends_with("sheet-Front.png"), "{}", report.png);
    assert_eq!(report.other_layouts.len(), 1);
    let back = &report.other_layouts[0];
    assert_eq!(back.layout.as_deref(), Some("Back"));
    assert!(back.png.ends_with("sheet-Back.png"), "{}", back.png);
    assert!(dir.join("sheet-Front.png").exists() && dir.join("sheet-Back.png").exists());
    assert!(!dir.join("sheet.png").exists());
    // Each covers its own paper: A4 laid across, A3 laid upright.
    assert!(report.width > report.height && back.height > back.width);
    // A render again replaces neither without `overwrite`.
    let again = render_with(&RenderRequest::new(&path), &Settings::default()).unwrap_err();
    assert_eq!(again.code, ErrorCode::OutputExists);
    // A plot is one PDF of a page for each, at its paper.
    let report = plot_with(&PlotRequest::new(&path), &Settings::default()).unwrap();
    let sizes: Vec<(String, f64, f64)> = report
        .layouts
        .iter()
        .map(|l| {
            (
                l.layout.clone(),
                l.sheet_size.width_mm,
                l.sheet_size.height_mm,
            )
        })
        .collect();
    assert_eq!(
        sizes,
        [
            ("Front".into(), 297.0, 210.0),
            ("Back".into(), 297.0, 420.0)
        ]
    );
    let bytes = fs::read(dir.join("sheet.pdf")).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert_eq!(
        text.matches("/Type /Page\n").count()
            + text.matches("/Type /Page/").count()
            + text.matches("/Type /Page ").count(),
        2,
        "two pages"
    );
    // A sheet of one layout is one PNG by the source's name, as before.
    let one = scratch("one");
    let path = sheet_at(
        &one,
        "units: mm\npaper: ISO A4 landscape\nlayout: {name: Panel}\nobjects:\n  - line: {from: [0, 0, 0], to: [100, 0, 0]}\n",
    );
    let report = render_with(&RenderRequest::new(&path), &Settings::default()).unwrap();
    assert!(report.png.ends_with("sheet.png") && report.layout.is_none());
    assert!(report.other_layouts.is_empty());
}

#[test]
fn the_checker_checks_each_layout_on_its_paper_and_names_it() {
    let dir = scratch("checked");
    // The Back layout's trim line is off its paper; Front's is on it.
    let text = TWO.replace(
        "[[0, 0, 0], [297, 0, 0], [297, 420, 0], [0, 420, 0]]",
        "[[0, 0, 0], [297, 0, 0], [297, 430, 0], [0, 430, 0]]",
    );
    let path = sheet_at(&dir, &text);
    let report = check_with(
        &CheckRequest {
            source: path,
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap();
    let found: Vec<(Option<&str>, String)> = report
        .findings
        .iter()
        .map(|f| (f.layout.as_deref(), format!("{:?}", f.rule)))
        .collect();
    assert!(!found.is_empty());
    assert!(
        found.iter().all(|(layout, _)| *layout == Some("Back")),
        "{found:?}"
    );
    let off = report
        .findings
        .iter()
        .find(|f| f.object.as_ref().is_some_and(|o| o.id == "sheet/back-trim"))
        .expect("the trim line off the paper");
    assert_eq!(off.paper.as_ref().unwrap().height, 420.0);
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
fn a_query_lists_every_layout_and_selects_by_space() {
    let dir = scratch("query");
    let path = sheet_at(&dir, TWO);
    let report = query(&path, Query::Layout);
    let Answer::Layout { count, layouts } = &report.answer else {
        panic!("{:?}", report.answer)
    };
    assert_eq!(*count, 3);
    let listed: Vec<(Option<&str>, usize, usize, usize)> = layouts
        .iter()
        .map(|l| {
            (
                l.layout.as_deref(),
                l.paper_space,
                l.model_space,
                l.viewports.len(),
            )
        })
        .collect();
    assert_eq!(listed, [(Some("Front"), 2, 3, 1), (Some("Back"), 3, 3, 2)]);
    let round = &layouts[1].viewports[0];
    assert!(matches!(round.clip, Some(Clip::Circle { .. })));
    assert_eq!(layouts[1].viewports[1].frozen_layers, ["wires"]);
    assert!(report.message.contains("2 layouts"), "{}", report.message);
    // A box on the paper of one layout selects in that layout alone.
    let mut selection = Selection {
        crossing: Some(SelectionBox([[-1.0, -1.0], [300.0, 430.0]])),
        ..Selection::default()
    };
    selection.space = Some("Back".into());
    let report = query(&path, Query::Find(selection.clone()));
    let Answer::Find { objects, .. } = &report.answer else {
        panic!()
    };
    let ids: Vec<&str> = objects.iter().map(|o| o.id.as_str()).collect();
    assert_eq!(
        ids,
        ["sheet/back-trim", "sheet/back-round", "sheet/back-bare"]
    );
    assert!(objects.iter().all(|o| o.space.as_deref() == Some("Back")));
    selection.space = Some("model".into());
    selection.crossing = Some(SelectionBox([[-1.0, -1.0], [401.0, 301.0]]));
    let report = query(&path, Query::Find(selection));
    let Answer::Find { objects, .. } = &report.answer else {
        panic!()
    };
    assert_eq!(objects.len(), 3);
    assert!(objects.iter().all(|o| o.space.as_deref() == Some("model")));
}

/// A drawing set of one sheet drawn from the two-layout template.
fn two_set(test: &str) -> PathBuf {
    let set = scratch(test);
    fs::create_dir_all(set.join("templates")).unwrap();
    let write = |name: &str, text: &str| fs::write(set.join(name), text).unwrap();
    write("set.yaml", "units: mm\noutput: dwg\nrows: rows.yaml\n");
    write(
        "rows.yaml",
        "rows:\n  - sheet: TWO-001\n    template: templates/two.yaml\n",
    );
    write("templates/two.yaml", TWO);
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

fn template_sheet(set: &Path) -> agentdrafter_core::Sheet {
    parse_sheet_description(&fs::read_to_string(set.join("templates/two.yaml")).unwrap()).unwrap()
}

#[test]
fn clip_viewport_and_set_frozen_layers_are_operations_with_autocads_aliases() {
    let listed = operations();
    for (name, alias) in [
        ("clip_viewport", "VPCLIP"),
        ("set_frozen_layers", "VPLAYER"),
    ] {
        let info = listed.operations.iter().find(|o| o.name == name).unwrap();
        assert_eq!(info.alias, alias, "{name}");
    }
    let set = two_set("operations");
    let before = template_sheet(&set);
    let model_at = |sheet: &agentdrafter_core::Sheet, i: usize, p: [f64; 2]| {
        viewport(&sheet.objects[i]).to_paper(p)
    };
    // Clipped to a triangle: model space stays where it showed.
    let reply = apply(
        &set,
        json!([{ "op": "clip_viewport", "target": {"id": "TWO-001/back-bare"},
                 "clip": {"points": [[40, 40, 0], [260, 40, 0], [150, 200, 0]]} }]),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    let after = template_sheet(&set);
    let clipped = viewport(&after.objects[7]);
    assert!(matches!(clipped.clip, Some(Clip::Polyline { .. })));
    assert_eq!(clipped.corners(), [[40.0, 40.0], [260.0, 200.0]]);
    for p in [[0.0, 0.0], [200.0, 150.0], [400.0, 300.0]] {
        let (a, b) = (model_at(&before, 7, p), model_at(&after, 7, p));
        assert!(
            (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9,
            "{p:?}"
        );
    }
    assert_eq!(clipped.frozen_layers, ["wires"]);
    // Moved, its boundary moves with it.
    let reply = apply(
        &set,
        json!([{ "op": "move", "target": {"id": "TWO-001/back-bare"}, "by": [10, 5, 0] }]),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    let moved = template_sheet(&set);
    assert_eq!(
        viewport(&moved.objects[7]).corners(),
        [[50.0, 45.0], [270.0, 205.0]]
    );
    // Taken away, the boundary leaves the viewport rectangular at its box.
    let reply = apply(
        &set,
        json!([{ "op": "clip_viewport", "target": {"id": "TWO-001/back-bare"}, "clip": null }]),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    let unclipped = template_sheet(&set);
    let plain = viewport(&unclipped.objects[7]);
    assert!(plain.clip.is_none());
    assert_eq!(plain.corners(), [[50.0, 45.0], [270.0, 205.0]]);
    assert_eq!(plain.view_center, viewport(&moved.objects[7]).view_center);
    // A viewport that is not clipped has no boundary to take away.
    let reply = apply(
        &set,
        json!([{ "op": "clip_viewport", "target": {"id": "TWO-001/back-bare"}, "clip": null }]),
    );
    let e = reply.error.unwrap();
    assert!(e.message.contains("no boundary to take away"), "{e:?}");
    // The frozen layers are set, and an empty list thaws them all.
    let reply = apply(
        &set,
        json!([{ "op": "set_frozen_layers", "target": {"id": "TWO-001/front-view"}, "frozen_layers": ["wires", "0"] }]),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    assert_eq!(
        // In the layer table's order, layer 0 first, as AutoCAD keeps them.
        viewport(&template_sheet(&set).objects[4]).frozen_layers,
        ["0", "wires"]
    );
    let reply = apply(
        &set,
        json!([{ "op": "set_frozen_layers", "target": {"id": "TWO-001/front-view"}, "frozen_layers": [] }]),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    assert!(viewport(&template_sheet(&set).objects[4])
        .frozen_layers
        .is_empty());
    // A layer that is not the sheet's is refused, naming the field.
    let reply = apply(
        &set,
        json!([{ "op": "set_frozen_layers", "target": {"id": "TWO-001/front-view"}, "frozen_layers": ["ducts"] }]),
    );
    let e = reply.error.unwrap();
    assert!(e.message.contains("layer `ducts` is not defined"), "{e:?}");
    // A layer frozen in a viewport is renamed there too, and is in use.
    let reply = apply(
        &set,
        json!([{ "op": "rename_layer", "template": "templates/two.yaml", "name": "wires", "to": "cables" }]),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    let renamed = template_sheet(&set);
    assert_eq!(
        viewport(&renamed.objects[6]).frozen_layers,
        Vec::<String>::new()
    );
    assert_eq!(viewport(&renamed.objects[7]).frozen_layers, ["cables"]);
    let reply = apply(
        &set,
        json!([
            { "op": "delete", "target": {"id": "TWO-001/wire"} },
            { "op": "delete_layer", "template": "templates/two.yaml", "name": "cables" }
        ]),
    );
    let e = reply.error.unwrap();
    assert!(e.message.contains("is in use"), "{e:?}");
    assert!(e.message.contains("back-bare"), "{}", e.message);
}

#[test]
fn a_shared_templates_viewport_is_clipped_through_any_sheet_built_from_it() {
    let set = scratch("shared");
    fs::create_dir_all(set.join("templates")).unwrap();
    let write = |name: &str, text: &str| fs::write(set.join(name), text).unwrap();
    write("set.yaml", "units: mm\noutput: dwg\nrows: rows.yaml\n");
    write(
        "rows.yaml",
        "rows:\n  - {sheet: A, template: templates/t.yaml}\n  - {sheet: B, template: templates/t.yaml}\n",
    );
    write(
        "templates/t.yaml",
        "units: mm\npaper: ISO A4 landscape\nlayout: {name: Main}\nlayers:\n  wires: {color: red}\nobjects:\n  - line: {id: wire, from: [-20, 0, 0], to: [20, 0, 0], layer: wires}\n  - viewport: {id: v, center: [50, 50, 0], width: 80, height: 80, view_center: [0, 0, 0], scale: 1}\n",
    );
    let built = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: false,
        },
        &Settings::default(),
    );
    assert!(built.ok, "{:#?}", built.error);
    let template = || {
        parse_sheet_description(&fs::read_to_string(set.join("templates/t.yaml")).unwrap())
            .unwrap()
    };
    // Named on the second sheet, the template's viewport is clipped, and
    // the change lands on both sheets built from it.
    for (op, check) in [
        (
            json!({ "op": "clip_viewport", "target": {"id": "B/v"}, "clip": {"center": [50, 50, 0], "radius": 20} }),
            "clip",
        ),
        (
            json!({ "op": "set_frozen_layers", "target": {"id": "B/v"}, "frozen_layers": ["wires"] }),
            "frozen",
        ),
        (
            json!({ "op": "change_viewport", "target": {"id": "B/v"}, "scale": "1:2" }),
            "scale",
        ),
    ] {
        let reply = apply(&set, json!([op]));
        assert!(reply.ok, "{check}: {:#?}", reply.error);
        let landed = reply.result.as_ref().unwrap().landed;
        assert_eq!(landed, 2, "{check}");
    }
    let v = template();
    let v = viewport(&v.objects[1]);
    assert!(matches!(v.clip, Some(Clip::Circle { .. })));
    assert_eq!(v.frozen_layers, ["wires"]);
    assert_eq!(v.scale, 0.5);
    // Both sheets build with the clipped viewport.
    for sheet in ["A", "B"] {
        let read = read_dwg(&set.join(format!("dwg/{sheet}.dwg"))).unwrap();
        assert!(
            matches!(&read.objects[1], Object::Viewport(r) if r.clip.is_some()),
            "{sheet}"
        );
    }
}

#[test]
fn an_add_names_the_layout_whose_paper_space_it_draws_in() {
    let set = two_set("add-space");
    let reply = apply(
        &set,
        json!([
            { "op": "add_text", "template": "templates/two.yaml", "id": "back-note", "contents": "BACK", "position": [20, 400, 0], "height": 5, "space": "Back" },
            { "op": "add_viewport", "template": "templates/two.yaml", "id": "front-round", "space": "Front", "clip": {"center": [250, 170, 0], "radius": 30}, "view_center": [200, 150, 0], "scale": 1 }
        ]),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    let sheet = template_sheet(&set);
    let n = sheet.objects.len();
    assert_eq!(sheet.layout_of(n - 2).unwrap().name, "Back");
    assert_eq!(sheet.layout_of(n - 1).unwrap().name, "Front");
    // `paper` names no one of several layouts.
    let reply = apply(
        &set,
        json!([{ "op": "add_line", "template": "templates/two.yaml", "from": [0, 0, 0], "to": [1, 0, 0], "space": "paper" }]),
    );
    let e = reply.error.unwrap();
    assert!(
        e.message.contains("`paper` is not a space of this sheet"),
        "{e:?}"
    );
}

#[test]
fn the_schema_takes_layouts_clips_and_frozen_layers_as_the_reader_does() {
    let schema = sheet_description_schema();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let two = |objects: Value| json!({"units": "mm", "layouts": [{"name": "A", "paper": "ISO A4 landscape"}, {"name": "B", "paper": "ISO A4 portrait"}], "objects": objects});
    let vp = |extra: Value| {
        let mut v = json!({"space": "A", "view_center": [0, 0, 0], "scale": 1});
        for (k, value) in extra.as_object().unwrap() {
            v[k] = value.clone();
        }
        json!({ "viewport": v })
    };
    let window = json!({"center": [50, 50, 0], "width": 10, "height": 10});
    let mut with_clip = window.clone();
    with_clip["clip"] = json!({"center": [50, 50, 0], "radius": 5});
    let cases: Vec<(Value, bool)> = vec![
        (two(json!([vp(window.clone())])), true),
        (
            two(json!([vp(
                json!({"clip": {"center": [50, 50, 0], "radius": 5}})
            )])),
            true,
        ),
        (
            two(json!([vp(
                json!({"clip": {"points": [[0, 0, 0], {"point": [10, 0, 0], "bulge": 1}, [10, 10, 0]]}})
            )])),
            true,
        ),
        (two(json!([vp(with_clip)])), false),
        (
            two(json!([vp(
                json!({"clip": {"points": [[0, 0, 0], [10, 0, 0]]}})
            )])),
            false,
        ),
        (
            two(json!([vp(
                json!({"clip": {"points": [{"point": [0, 0, 0], "end_width": 1}, [10, 0, 0], [10, 10, 0]]}})
            )])),
            false,
        ),
        (
            two(json!([vp(
                json!({"center": [50, 50, 0], "width": 10, "height": 10, "frozen_layers": ["0"]})
            )])),
            true,
        ),
        (
            two(json!([{"line": {"from": [0, 0, 0], "to": [1, 0, 0], "space": "B"}}])),
            true,
        ),
        (
            json!({"units": "mm", "layouts": [{"name": "A", "paper": "ISO A4 landscape"}], "objects": []}),
            false,
        ),
        (
            json!({"units": "mm", "paper": "ISO A4 landscape", "layout": {"name": "A"}, "layouts": [{"name": "B"}, {"name": "C"}], "objects": []}),
            false,
        ),
        (
            json!({"units": "mm", "layouts": [{"name": "A", "paper": [100, 100]}, {"name": "B", "paper": "ISO A4 portrait"}], "objects": []}),
            false,
        ),
    ];
    for (case, valid) in cases {
        assert_eq!(validator.is_valid(&case), valid, "{case}");
        // The reader agrees.
        let yaml = serde_json::to_string(&case).unwrap();
        assert_eq!(parse_sheet_description(&yaml).is_ok(), valid, "{yaml}");
    }
}
