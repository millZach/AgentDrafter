//! A change set drawn before it lands: `display-list` with a change set
//! draws a drawing set's sheet as the change set would leave it, applied
//! in memory as a dry run applies it, marks what it adds and changes,
//! gives what it changes or deletes as it draws now, says sheet by sheet
//! whether it would land and the checks each would then fail, and writes
//! nothing, the journal among it.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    display_list_with, DisplayListReport, DisplayListRequest, ErrorCode, HowChanged, Preview, Rule,
    Settings,
};
use serde_json::{json, Value};

#[path = "support/snapshot.rs"]
mod snapshot;

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-preview")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A copy of the demo plant's records, to preview change sets on.
fn demo_plant(test: &str) -> PathBuf {
    let set = scratch(test);
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
    set
}

/// A made-up set of two sheets built from one template, S-2's row holding
/// a circle of its own named `washer`.
fn two_sheets(test: &str) -> PathBuf {
    let set = scratch(test);
    fs::create_dir_all(set.join("templates")).unwrap();
    fs::write(
        set.join("set.yaml"),
        "units: mm\noutput: dwg\nrows: rows.yaml\n",
    )
    .unwrap();
    fs::write(
        set.join("templates/t.yaml"),
        "units: mm\npaper: [100, 80]\nobjects:\n  - circle: {center: [10, 10, 0], radius: 4, id: hole, name: hole}\n  - line: {from: [5, 40, 0], to: [95, 40, 0], id: rule}\n  - text: {contents: {row: TITLE}, position: [5, 60, 0], height: 3, id: heading}\n",
    )
    .unwrap();
    fs::write(
        set.join("rows.yaml"),
        "rows:\n  - sheet: S-1\n    template: templates/t.yaml\n    values: {TITLE: FIRST}\n  - sheet: S-2\n    template: templates/t.yaml\n    values: {TITLE: SECOND}\n    objects:\n      - circle: {center: [70, 10, 0], radius: 2.5, id: washer, name: washer}\n",
    )
    .unwrap();
    set
}

fn request(set: &Path, sheet: &str, change_set: Option<Value>) -> DisplayListRequest {
    let mut request = DisplayListRequest::new(set);
    request.sheet = Some(sheet.to_owned());
    request.change_set = change_set;
    request
}

fn draw(request: &DisplayListRequest) -> DisplayListReport {
    display_list_with(request, &Settings::default()).unwrap()
}

fn preview(report: &DisplayListReport) -> &Preview {
    report.preview.as_ref().expect("a preview")
}

/// The place among the report's objects of the object `id`.
fn place(report: &DisplayListReport, id: &str) -> usize {
    report
        .objects
        .iter()
        .position(|o| o.id.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("no object {id}"))
}

/// The places of the display list's items that the object `id` draws.
fn items_of(report: &DisplayListReport, id: &str) -> Vec<usize> {
    let at = place(report, id);
    report
        .display_list
        .items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.object == at)
        .map(|(i, _)| i)
        .collect()
}

#[test]
fn a_template_move_is_drawn_on_each_sheet_it_reaches_with_its_checks_and_writes_nothing() {
    let set = demo_plant("template-move");
    let before = snapshot::snapshot(&set);
    // The service text moved right, into the zone line on the loop sheets
    // whose zone line runs there: a template edit, so every sheet built
    // from the template follows.
    let move_service = json!({"operations": [{"op": "move", "target": {"id": "DP-LD-101/service"}, "by": [2.2, 0, 0]}]});
    let report = draw(&request(&set, "DP-LD-301", Some(move_service.clone())));
    let p = preview(&report);

    // The dry run's reply: it would land on all seven sheets of the
    // template, and not on DP-LD-103, which another builds.
    assert!(p.change.dry_run);
    let reached: Vec<&str> = p.change.sheets.iter().map(|s| s.sheet.as_str()).collect();
    assert_eq!(
        reached,
        [
            "DP-LD-101",
            "DP-LD-102",
            "DP-LD-104",
            "DP-LD-201",
            "DP-LD-202",
            "DP-LD-301",
            "DP-LD-302"
        ]
    );
    assert!(p.change.sheets.iter().all(|s| s.ok && s.error.is_none()));
    assert_eq!((p.change.landed, p.change.failed), (7, 0));
    // The objects it changes, on every sheet, as the journal would name them.
    assert_eq!(p.objects.len(), 7);
    assert!(p
        .objects
        .iter()
        .all(|o| o.how == HowChanged::Changed && o.id.ends_with("/service")));

    // On the sheet drawn: the service text where it would go, marked
    // changed, and where it is now under `was`.
    let service = items_of(&report, "DP-LD-301/service");
    assert!(!service.is_empty());
    assert_eq!(
        p.changed, service,
        "only the service text's items are marked"
    );
    assert!(p.added.is_empty());
    assert_eq!(p.was.len(), 1);
    assert_eq!(p.was[0].id, "DP-LD-301/service");
    assert_eq!(p.was[0].how, HowChanged::Changed);
    let x_of = |points: Vec<[f64; 2]>| points.iter().map(|p| p[0]).fold(f64::MAX, f64::min);
    let now_x = x_of(p.was[0].items.iter().flat_map(|i| i.points()).collect());
    let then_x = x_of(
        service
            .iter()
            .flat_map(|&i| report.display_list.items[i].points())
            .collect(),
    );
    assert!(
        (then_x - now_x - 2.2).abs() < 1e-6,
        "moved 2.2 in: {now_x} to {then_x}"
    );

    // The checks the sheets would fail, read from the records as they would
    // be written: the moved text runs through a zone line on three sheets.
    let checks = p.checks.as_ref().expect("checks").result.as_ref().unwrap();
    assert_eq!(checks.sheets.len(), 7);
    let failing: Vec<(&str, Rule)> = checks
        .findings
        .iter()
        .map(|f| (f.sheet.as_str(), f.rule))
        .collect();
    assert_eq!(
        failing,
        [
            ("DP-LD-202", Rule::TextOnLine),
            ("DP-LD-301", Rule::TextOnLine),
            ("DP-LD-302", Rule::TextOnLine)
        ]
    );
    // Clouded on the sheet drawn, round the text's letters.
    assert_eq!(p.clouds.len(), 1);
    assert_eq!(p.clouds[0].rule, Rule::TextOnLine);
    assert!((p.clouds[0].bounds.min[0] - then_x).abs() < 0.05);

    // A sheet it reaches that fails no check has no cloud.
    let other = draw(&request(&set, "DP-LD-101", Some(move_service)));
    assert!(preview(&other).clouds.is_empty());
    assert_eq!(
        preview(&other).changed,
        items_of(&other, "DP-LD-101/service")
    );

    // Nothing was written: the records, no DWG, no journal, no mark.
    assert_eq!(snapshot::snapshot(&set), before);
    assert!(!set.join(".agentdrafter").exists(), "no journal");
}

#[test]
fn an_added_object_is_marked_and_a_deleted_one_is_given_as_it_draws_now() {
    let set = two_sheets("add-delete");
    let before = snapshot::snapshot(&set);
    let change_set = json!({"operations": [
        {"op": "add_circle", "sheet": "S-2", "center": [50, 20, 0], "radius": 3, "id": "spot"},
        {"op": "delete", "target": {"id": "S-2/rule"}},
        {"op": "change", "target": {"id": "S-2/washer"}, "set": {"radius": 4}}
    ]});
    let report = draw(&request(&set, "S-2", Some(change_set)));
    let p = preview(&report);
    assert_eq!(p.added, items_of(&report, "S-2/spot"));
    assert_eq!(p.changed, items_of(&report, "S-2/washer"));
    assert!(
        !report
            .objects
            .iter()
            .any(|o| o.id.as_deref() == Some("S-2/rule")),
        "the deleted line is not drawn"
    );
    let deleted = p
        .was
        .iter()
        .find(|w| w.id == "S-2/rule")
        .expect("the line as it draws now");
    assert_eq!(deleted.how, HowChanged::Deleted);
    let points: Vec<[f64; 2]> = deleted.items.iter().flat_map(|i| i.points()).collect();
    assert!(points.contains(&[5.0, 40.0]) && points.contains(&[95.0, 40.0]));
    let changed = p
        .was
        .iter()
        .find(|w| w.id == "S-2/washer")
        .expect("the washer as it is");
    assert_eq!(changed.how, HowChanged::Changed);
    // The template's line is deleted on S-1 too, which is not drawn.
    assert!(p
        .objects
        .iter()
        .any(|o| o.id == "S-1/rule" && o.how == HowChanged::Deleted));
    assert_eq!(snapshot::snapshot(&set), before);
}

#[test]
fn a_sheet_that_would_refuse_a_template_edit_holds_back_every_sheet_and_is_drawn_as_it_is() {
    let set = two_sheets("refused");
    let before = snapshot::snapshot(&set);
    // Naming the template's circle `washer` clashes with S-2's own.
    let rename = json!({"operations": [{"op": "change", "target": {"id": "S-1/hole"}, "set": {"name": "washer"}}]});
    let report = draw(&request(&set, "S-1", Some(rename)));
    let p = preview(&report);
    assert_eq!((p.change.landed, p.change.failed), (0, 2));
    let s1 = &p.change.sheets[0];
    let s2 = &p.change.sheets[1];
    assert_eq!((s1.sheet.as_str(), s1.ok), ("S-1", false));
    assert_eq!(
        s1.error.as_ref().unwrap().code,
        ErrorCode::SharedEditFailed,
        "held back"
    );
    assert_eq!((s2.sheet.as_str(), s2.ok), ("S-2", false));
    assert_eq!(s2.error.as_ref().unwrap().code, ErrorCode::DuplicateName);
    // Nothing would land: no checks, nothing marked, the sheet as it is.
    assert!(p.checks.is_none() && p.objects.is_empty());
    assert!(p.added.is_empty() && p.changed.is_empty() && p.was.is_empty());
    let now = draw(&request(&set, "S-1", None));
    assert_eq!(report.display_list, now.display_list);
    assert_eq!(snapshot::snapshot(&set), before);
}

#[test]
fn a_sheet_whose_dwg_cannot_be_written_is_refused_and_takes_it_once_it_can() {
    let set = two_sheets("not-writable");
    agentdrafter_core::build_set_with(
        &agentdrafter_core::SetBuildRequest {
            set: set.clone(),
            overwrite: false,
        },
        &Settings::default(),
    );
    let dwg = set.join("dwg/S-2.dwg");
    let mut permissions = fs::metadata(&dwg).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&dwg, permissions.clone()).unwrap();
    // As a user who may write any file, such as root, the DWG still opens
    // to write, and there is nothing to refuse.
    let opens = fs::OpenOptions::new().append(true).open(&dwg).is_ok();
    let move_hole =
        json!({"operations": [{"op": "move", "target": {"id": "S-1/hole"}, "by": [1, 0, 0]}]});
    if !opens {
        let report = draw(&request(&set, "S-1", Some(move_hole.clone())));
        let p = preview(&report);
        assert_eq!(p.change.landed, 0);
        let s2 = p.change.sheets.iter().find(|s| s.sheet == "S-2").unwrap();
        assert_eq!(s2.error.as_ref().unwrap().code, ErrorCode::WriteFailed);
    }
    // Once it can be written, the same change set would land.
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    fs::set_permissions(&dwg, permissions).unwrap();
    let report = draw(&request(&set, "S-1", Some(move_hole)));
    assert_eq!(preview(&report).change.landed, 2);
}

#[test]
fn a_preview_takes_snaps_origins_and_values_and_without_a_change_set_the_reply_is_as_it_was() {
    let set = two_sheets("options");
    let mut plain = request(&set, "S-1", None);
    plain.snaps = true;
    plain.origins = true;
    plain.values = true;
    let without = draw(&plain);
    assert!(without.preview.is_none());
    let text = serde_json::to_value(&without).unwrap();
    assert!(text.get("preview").is_none(), "no preview key without one");

    // A change set whose point is a snapped point, as the drafting app's
    // Move sends it, drawn with snaps, origins and values.
    let mut with = plain.clone();
    with.change_set = Some(
        json!({"operations": [{"op": "move", "target": {"id": "S-1/heading"},
        "from": {"snap": {"id": "S-1/hole", "snap": "center"}}, "to": [20, 20, 0]}]}),
    );
    let report = draw(&with);
    let p = preview(&report);
    assert_eq!(p.change.landed, 2);
    assert!(report.snaps.as_ref().is_some_and(|s| !s.is_empty()));
    assert!(report.template.is_some());
    let heading = &report.objects[place(&report, "S-1/heading")];
    assert!(heading.from.is_some() && heading.values.is_some());
    assert_eq!(p.changed, items_of(&report, "S-1/heading"));
    // The rest of the reply is the sheet as it would be: its own objects,
    // in the same order as without.
    assert_eq!(report.objects.len(), without.objects.len());
}

#[test]
fn a_change_set_is_drawn_only_on_a_drawing_sets_sheet_as_it_is() {
    let set = two_sheets("refused-requests");
    let change_set =
        json!({"operations": [{"op": "move", "target": {"id": "S-1/hole"}, "by": [1, 0, 0]}]});
    let mut at_entry = request(&set, "S-1", Some(change_set.clone()));
    at_entry.entry = Some(1);
    let e = display_list_with(&at_entry, &Settings::default()).unwrap_err();
    assert_eq!(
        (e.code, e.field.as_deref()),
        (ErrorCode::BadValue, Some("change_set"))
    );

    let description = scratch("refused-description").join("one.yaml");
    fs::write(
        &description,
        "units: mm\nobjects:\n  - circle: {center: [0, 0, 0], radius: 1, id: c}\n",
    )
    .unwrap();
    let mut one = DisplayListRequest::new(&description);
    one.change_set = Some(change_set);
    let e = display_list_with(&one, &Settings::default()).unwrap_err();
    assert_eq!(
        (e.code, e.field.as_deref()),
        (ErrorCode::BadValue, Some("change_set"))
    );

    // A change set the core refuses whole gives its error, as change does.
    let wrong = request(
        &set,
        "S-1",
        Some(json!({"operations": [{"op": "no_such_operation"}]})),
    );
    assert!(display_list_with(&wrong, &Settings::default()).is_err());
}

/// A made-up set of one sheet, `S-1`, built from a template of `objects`.
fn one_sheet(test: &str, objects: &str) -> PathBuf {
    let set = scratch(test);
    fs::create_dir_all(set.join("templates")).unwrap();
    fs::write(
        set.join("set.yaml"),
        "units: mm\noutput: dwg\nrows: rows.yaml\n",
    )
    .unwrap();
    fs::write(
        set.join("templates/t.yaml"),
        format!("units: mm\npaper: [100, 80]\nobjects:\n{objects}"),
    )
    .unwrap();
    fs::write(
        set.join("rows.yaml"),
        "rows:\n  - sheet: S-1\n    template: templates/t.yaml\n",
    )
    .unwrap();
    set
}

#[test]
fn a_change_set_that_repairs_a_sheet_that_does_not_draw_now_is_drawn_with_a_warning() {
    // A line on a layer the template lacks: the sheet does not draw.
    let set = one_sheet(
        "repair",
        "  - line: {from: [0, 0, 0], to: [100, 80, 0], id: frame}\n  - line: {from: [5, 40, 0], to: [95, 40, 0], id: rule, layer: notes}\n",
    );
    assert!(display_list_with(&request(&set, "S-1", None), &Settings::default()).is_err());
    let before = snapshot::snapshot(&set);
    let change_set = json!({"operations": [
        {"op": "add_layer", "template": "templates/t.yaml", "name": "notes"},
        {"op": "move", "target": {"id": "S-1/rule"}, "by": [0, 5, 0]}
    ]});
    let report = draw(&request(&set, "S-1", Some(change_set)));
    let p = preview(&report);
    assert!(p.change.sheets.iter().all(|s| s.ok), "{:?}", p.change);
    assert!(p.checks.is_some());
    // Without the sheet as it is, there are no old places, and every item
    // of what it changes is marked.
    assert_eq!(p.changed, items_of(&report, "S-1/rule"));
    assert!(p.was.is_empty());
    let warning = p.warning.as_deref().expect("a warning");
    assert!(warning.contains("cannot be drawn as it is"), "{warning}");
    assert!(warning.contains("notes"), "{warning}");
    assert_eq!(snapshot::snapshot(&set), before);

    // A sheet that draws as it is gives no warning.
    let set = two_sheets("repair-none");
    let moved =
        json!({"operations": [{"op": "move", "target": {"id": "S-1/rule"}, "by": [0, 5, 0]}]});
    assert!(preview(&draw(&request(&set, "S-1", Some(moved))))
        .warning
        .is_none());
}

#[test]
fn a_sheet_emptied_by_a_change_set_is_clouded_on_its_paper() {
    let set = one_sheet(
        "emptied",
        "  - circle: {center: [50, 40, 0], radius: 4, id: hole}\n",
    );
    let change_set = json!({"operations": [{"op": "delete", "target": {"id": "S-1/hole"}}]});
    let report = draw(&request(&set, "S-1", Some(change_set)));
    let p = preview(&report);
    let cloud = p
        .clouds
        .iter()
        .find(|c| c.rule == Rule::PaperSize)
        .unwrap_or_else(|| panic!("a cloud for the paper: {:?}", p.checks));
    assert_eq!(
        (cloud.bounds.min, cloud.bounds.max),
        ([0.0, 0.0], [100.0, 80.0])
    );
}
