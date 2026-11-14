//! A sheet's display list as plain data, for the viewer: a drawing set's
//! sheet by name, a sheet description or a DWG, with the set's sheets, its
//! paper and its scale, written nowhere; the set's sheet draws what its
//! DWG draws.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_set_with, display_list_with, DisplayListReport, DisplayListRequest, ErrorCode,
    SetBuildRequest, Settings, Shape,
};

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
