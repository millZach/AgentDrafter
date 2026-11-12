//! Accept on the demo set: refused while the set is inconsistent; warns on
//! failed checks and lists them, issuing nothing until told to accept
//! despite them; marks every change set in place issued, after which undo
//! refuses them; records the interface it came through; and outputs made
//! after it say, in their stamps, that they came from an issued revision.
//! Then the set drawn as it was at each entry of its journal, rebuilt from
//! the undo states, and the entries that can no longer be shown.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    accept_with, build_set_with, change_with, check_stale, display_list_with, journal,
    plot_set_with, render_with, undo, AcceptReport, AcceptRequest, ChangeRequest,
    DisplayListRequest, EntryKind, ErrorCode, Interface, JournalRequest, PlotSetRequest,
    RenderRequest, Reply, SetBuildRequest, Settings, StaleRequest, UndoRequest, JOURNAL_FILE,
    KEPT_UNDO_STATES,
};
use serde_json::{json, Value};

#[path = "support/demo_set.rs"]
mod demo_set;
#[path = "support/snapshot.rs"]
mod snapshot;

use snapshot::snapshot;

/// A built copy of the demo set in a folder of its own, under Cargo's
/// scratch folder in `target/`.
fn demo_set(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-accept")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    let set = dir.join("demo-set");
    demo_set::copy_demo_set(&set);
    let reply = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: false,
        },
        &Settings::default(),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    set
}

fn apply(set: &Path, change_set: Value) -> u64 {
    let reply = change_with(
        &ChangeRequest {
            set: set.to_owned(),
            change_set,
            dry_run: false,
        },
        &Settings::default(),
    );
    reply.result.unwrap().journal_entry.unwrap()
}

fn accept(
    set: &Path,
    despite: bool,
    newest_seen: Option<u64>,
    dry_run: bool,
) -> Reply<AcceptReport> {
    accept_with(
        &AcceptRequest {
            set: set.to_owned(),
            despite_failed_checks: despite,
            newest_seen,
            dry_run,
        },
        &Settings::default(),
        Interface::Viewer,
    )
}

fn undo_code(set: &Path, change_set: u64) -> Option<ErrorCode> {
    let reply = undo(&UndoRequest {
        set: set.to_owned(),
        change_set,
        dry_run: false,
    });
    reply.error.map(|e| e.code)
}

/// The journal's lines that hold an entry, in order: not those that mark an
/// earlier entry done.
fn journal_lines(set: &Path) -> Vec<Value> {
    fs::read_to_string(set.join(JOURNAL_FILE))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|line| line.get("kind").is_some())
        .collect()
}

fn moved(by: f64) -> Value {
    json!({"operations": [{"op": "move", "target": {"id": "DEMO-001/border"}, "by": [by, 0, 0]}]})
}

#[test]
fn accept_is_refused_while_the_set_is_inconsistent() {
    let set = demo_set("inconsistent");
    let partial = json!({"operations": [
        {"op": "add_circle", "sheet": "DEMO-001", "center": [5, 5, 0], "radius": 0.5},
        {"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": 0.5, "layer": "nope"},
    ]});
    assert_eq!(apply(&set, partial), 1);
    let before = fs::read(set.join(JOURNAL_FILE)).unwrap();
    for dry_run in [true, false] {
        let reply = accept(&set, true, None, dry_run);
        let error = reply.error.unwrap();
        assert_eq!(error.code, ErrorCode::Inconsistent);
        assert!(
            error
                .message
                .contains("cannot be accepted while it is inconsistent"),
            "{}",
            error.message
        );
        assert!(error.message.contains("DEMO-002"), "{}", error.message);
        assert!(reply.result.is_none());
    }
    assert_eq!(
        fs::read(set.join(JOURNAL_FILE)).unwrap(),
        before,
        "nothing recorded"
    );

    // Fixed by undoing the change set, the set can be accepted.
    assert_eq!(undo_code(&set, 1), None);
    assert!(accept(&set, false, None, false).ok);
}

#[test]
fn accept_warns_of_failed_checks_and_issues_nothing_until_told_to_accept_despite_them() {
    let set = demo_set("checks");
    // A circle drawn past the paper's right edge fails off_sheet.
    let off = json!({"operations": [{"op": "add_circle", "sheet": "DEMO-003", "center": [17.2, 5, 0], "radius": 0.5, "layer": "notes", "id": "edge"}]});
    assert_eq!(apply(&set, off), 1);
    let before = snapshot(&set);
    let journal_before = fs::read(set.join(JOURNAL_FILE)).unwrap();

    for dry_run in [true, false] {
        let reply = accept(&set, false, Some(1), dry_run);
        assert!(!reply.ok);
        let error = reply.error.unwrap();
        assert_eq!(error.code, ErrorCode::ChecksFailed);
        assert!(
            error.message.contains("off_sheet on `DEMO-003`"),
            "{}",
            error.message
        );
        assert!(
            error.message.contains("despite_failed_checks"),
            "{}",
            error.message
        );
        let report = reply.result.expect("the report lists the failed checks");
        assert!(report.journal_entry.is_none());
        assert_eq!(report.issued, [1]);
        let rules: Vec<(String, String)> = report
            .checks
            .findings
            .iter()
            .map(|f| {
                (
                    f.sheet.clone(),
                    serde_json::to_value(f.rule)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_owned(),
                )
            })
            .collect();
        assert!(
            rules.contains(&("DEMO-003".into(), "off_sheet".into())),
            "{rules:?}"
        );
    }
    assert_eq!(snapshot(&set), before);
    assert_eq!(
        fs::read(set.join(JOURNAL_FILE)).unwrap(),
        journal_before,
        "nothing recorded"
    );

    // Accepted despite them, the entry records what was accepted.
    let reply = accept(&set, true, Some(1), false);
    assert!(reply.ok, "{:#?}", reply.error);
    let report = reply.result.unwrap();
    assert_eq!(report.journal_entry, Some(2));
    assert!(
        report.message.contains("despite 2 failed checks"),
        "{}",
        report.message
    );
    let entry = &journal_lines(&set)[1];
    assert_eq!(entry["kind"], "accept");
    assert_eq!(entry["via"], "viewer");
    // The circle is past the paper's edge, which makes the sheet too wide.
    let failed = entry["failed_checks"].as_array().unwrap();
    assert_eq!(failed.len(), 2, "{entry}");
    assert_eq!(failed[0]["rule"], "paper_size");
    assert_eq!(failed[1]["sheet"], "DEMO-003");
    assert_eq!(failed[1]["rule"], "off_sheet");
    assert_eq!(failed[1]["id"], "DEMO-003/edge");
}

#[test]
fn accept_issues_every_change_set_in_place_and_an_issued_one_cannot_be_undone() {
    let set = demo_set("issues");
    assert_eq!(apply(&set, moved(0.1)), 1);
    assert_eq!(apply(&set, moved(0.1)), 2);
    assert_eq!(undo_code(&set, 2), None, "entry 3 undoes change set 2");
    assert_eq!(apply(&set, moved(-0.2)), 4);

    // The journal moved on past what the caller saw: refused.
    let error = accept(&set, false, Some(3), false).error.unwrap();
    assert_eq!(error.code, ErrorCode::MovedOn);
    assert!(
        error.message.contains("change set 4 came since"),
        "{}",
        error.message
    );

    // A dry run says what it would do and records nothing.
    let lines = journal_lines(&set).len();
    let dry = accept(&set, false, Some(4), true).result.unwrap();
    assert_eq!(
        dry.issued,
        [1, 4],
        "every change set in place, not the undone one"
    );
    assert_eq!(dry.journal_entry, None);
    assert_eq!(journal_lines(&set).len(), lines);

    let report = accept(&set, false, Some(4), false).result.unwrap();
    assert_eq!(report.issued, [1, 4]);
    assert_eq!(report.sheets, ["DEMO-001", "DEMO-002", "DEMO-003"]);
    assert_eq!(report.via, Interface::Viewer);
    assert_eq!(report.journal_entry, Some(5));
    let paths: Vec<&str> = report.revision.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "set.yaml",
            "rows.yaml",
            "templates/ansi-b.yaml",
            "blocks.yaml",
            "dwg/DEMO-001.dwg",
            "dwg/DEMO-002.dwg",
            "dwg/DEMO-003.dwg"
        ]
    );

    // The journal says what became of each change set.
    let read = journal(&JournalRequest { set: set.clone() })
        .result
        .unwrap();
    let issued_by: Vec<(u64, Option<u64>)> = read
        .entries
        .iter()
        .filter(|e| e.recorded.kind == EntryKind::ChangeSet)
        .map(|e| (e.recorded.entry, e.issued_by))
        .collect();
    assert_eq!(issued_by, [(1, Some(5)), (2, None), (4, Some(5))]);
    assert!(read.can_undo.is_empty(), "{:?}", read.can_undo);
    assert!(read.message.contains("and 1 accept"), "{}", read.message);

    // Undo refuses an issued change set, and changes nothing.
    let before = snapshot(&set);
    for change_set in [1, 4] {
        assert_eq!(undo_code(&set, change_set), Some(ErrorCode::Issued));
    }
    assert_eq!(snapshot(&set), before);

    // A change set after the accept can be undone; one before it with it
    // cannot.
    assert_eq!(apply(&set, moved(0.3)), 6);
    assert_eq!(undo_code(&set, 4), Some(ErrorCode::Issued));
    assert_eq!(undo_code(&set, 6), None);

    // Accepting again with nothing new issues nothing.
    let error = accept(&set, false, None, false).error.unwrap();
    assert_eq!(error.code, ErrorCode::NothingToAccept);
    assert!(error.message.contains("entry 5"), "{}", error.message);
}

#[test]
fn a_change_set_s_entry_names_the_objects_it_changed_on_each_sheet() {
    let set = demo_set("objects");
    let change_set = json!({"operations": [
        {"op": "move", "target": {"id": "DEMO-001/border"}, "by": [0.1, 0, 0]},
        {"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": 0.5, "layer": "notes", "id": "tank"},
        {"op": "set_attribute", "target": {"name": "title-block", "sheet": "DEMO-003"}, "tag": "REV", "value": "A"},
    ]});
    assert_eq!(apply(&set, change_set), 1);
    let entry = &journal_lines(&set)[0];
    let objects: Vec<(String, String)> = entry["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| {
            (
                o["id"].as_str().unwrap().to_owned(),
                o["how"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    for wanted in [
        ("DEMO-001/border", "changed"),
        ("DEMO-002/border", "changed"),
        ("DEMO-003/border", "changed"),
        ("DEMO-002/tank", "added"),
    ] {
        assert!(
            objects.contains(&(wanted.0.to_owned(), wanted.1.to_owned())),
            "{wanted:?} in {objects:?}"
        );
    }
    assert!(
        objects.iter().any(|(id, how)| id.starts_with("DEMO-003/")
            && how == "changed"
            && id != "DEMO-003/border"),
        "the title block whose row value was set: {objects:?}"
    );
}

#[test]
fn outputs_made_after_an_accept_say_in_their_stamps_that_they_came_from_an_issued_revision() {
    let set = demo_set("stamps");
    let render = |name: &str| {
        let mut request = RenderRequest::new(set.join("dwg").join("DEMO-001.dwg"));
        request.output = Some(set.join(name));
        request.width = Some(200);
        let reply = render_with(&request, &Settings::default());
        assert!(reply.is_ok(), "{:?}", reply.err());
        set.join(name)
    };
    let plot = || {
        let reply = plot_set_with(
            &PlotSetRequest {
                set: set.clone(),
                overwrite: true,
                plot_style: None,
            },
            &Settings::default(),
        );
        assert!(reply.ok, "{:#?}", reply.error);
        set.join("dwg").join("DEMO-001.pdf")
    };
    let stale = |output: &Path| {
        check_stale(&StaleRequest {
            output: output.to_owned(),
            source: None,
        })
        .unwrap()
    };

    // Before any accept: the working revision, not issued.
    let png = render("before.png");
    let report = stale(&png);
    assert!(!report.issued);
    assert_eq!(
        report.stamp.revision.as_ref().map(|r| r.issued),
        Some(false)
    );
    assert!(
        report.revision.contains("working revision"),
        "{}",
        report.revision
    );

    assert_eq!(apply(&set, moved(0.1)), 1);
    let accepted = accept(&set, false, Some(1), false).result.unwrap();
    assert_eq!(accepted.journal_entry, Some(2));

    // A PNG and a PDF made now say the accept in entry 2 issued them.
    let png = render("after.png");
    let pdf = plot();
    for output in [&png, &pdf] {
        let report = stale(output);
        assert!(!report.stale);
        assert!(report.issued, "{}", output.display());
        let revision = report.stamp.revision.clone().unwrap();
        assert_eq!(revision.accept, Some(2));
        assert_eq!(revision.via, Some(Interface::Viewer));
        assert!(revision.accepted_at.is_some());
        assert!(
            report.revision.contains("the accept in entry 2")
                && report.revision.contains("the viewer"),
            "{}",
            report.revision
        );
    }

    // Another change set: what is made now is not issued, and what was
    // made from the issued revision is stale.
    assert_eq!(apply(&set, moved(0.1)), 3);
    let png_now = render("later.png");
    assert!(!stale(&png_now).issued);
    let old = stale(&png);
    assert!(
        old.stale && old.issued,
        "made from the issued revision, which has moved on"
    );
    let pdf_now = plot();
    assert!(!stale(&pdf_now).issued);
}

/// A built copy of the demo set whose template, or whose rows file, is kept
/// outside its folder, as a set's paths may name them: the template at
/// `../outside/ansi-b.yaml`, or the rows at `../shared/rows.yaml`.
fn outside_set(test: &str, outside: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-accept")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    let set = dir.join("demo-set");
    demo_set::copy_demo_set(&set);
    match outside {
        "template" => {
            fs::create_dir_all(dir.join("outside")).unwrap();
            fs::rename(
                set.join("templates").join("ansi-b.yaml"),
                dir.join("outside").join("ansi-b.yaml"),
            )
            .unwrap();
            // The template names its block library from its own folder.
            fs::copy(set.join("blocks.yaml"), dir.join("blocks.yaml")).unwrap();
            let rows = fs::read_to_string(set.join("rows.yaml")).unwrap();
            let rows = rows.replace(
                "template: templates/ansi-b.yaml",
                "template: ../outside/ansi-b.yaml",
            );
            fs::write(set.join("rows.yaml"), rows).unwrap();
        }
        _ => {
            fs::create_dir_all(dir.join("shared")).unwrap();
            fs::rename(set.join("rows.yaml"), dir.join("shared").join("rows.yaml")).unwrap();
            let settings = fs::read_to_string(set.join("set.yaml")).unwrap();
            fs::write(
                set.join("set.yaml"),
                settings.replace("rows: rows.yaml", "rows: ../shared/rows.yaml"),
            )
            .unwrap();
        }
    }
    let reply = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: false,
        },
        &Settings::default(),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    set
}

#[test]
fn stepping_back_reads_the_records_the_journal_kept_even_outside_the_set_s_folder() {
    for (outside, first, second) in [
        // A template edit writes the template outside the set.
        ("template", moved(0.1), moved(0.2)),
        // An object of a row's own writes the rows file outside the set.
        (
            "rows",
            json!({"operations": [{"op": "add_circle", "sheet": "DEMO-001", "center": [5, 5, 0], "radius": 0.5, "layer": "notes", "id": "tank"}]}),
            json!({"operations": [{"op": "move", "target": {"id": "DEMO-001/tank"}, "by": [1, 0, 0]}]}),
        ),
    ] {
        let set = outside_set(&format!("outside-{outside}"), outside);
        assert_eq!(apply(&set, first), 1, "{outside}");
        let at_first = drawn(&set, None).unwrap();
        assert_eq!(apply(&set, second), 2, "{outside}");
        let now = drawn(&set, None).unwrap();
        assert_ne!(
            now, at_first,
            "{outside}: the second change set changed the sheet"
        );
        assert_eq!(
            drawn(&set, Some(1)).unwrap(),
            at_first,
            "{outside}: entry 1 is drawn from the record the journal kept, not today's"
        );
        assert_eq!(drawn(&set, Some(2)).unwrap(), now, "{outside}");
    }
}

/// The text of `sheet`'s display list as the set was at `entry`, or now.
fn drawn(set: &Path, entry: Option<u64>) -> Result<String, ErrorCode> {
    let mut request = DisplayListRequest::new(set);
    request.sheet = Some("DEMO-001".into());
    request.entry = entry;
    display_list_with(&request, &Settings::default())
        .map(|report| serde_json::to_string(&report.display_list).unwrap())
        .map_err(|e| e.code)
}

#[test]
fn the_set_is_drawn_as_it_was_at_each_entry_and_pruned_entries_say_they_cannot_be() {
    let set = demo_set("history");
    let at_start = drawn(&set, None).unwrap();
    let mut states = vec![at_start.clone()];
    for i in 0..3 {
        apply(&set, moved(0.1 * f64::from(i + 1)));
        states.push(drawn(&set, None).unwrap());
    }
    // Entry 4 undoes change set 3; entry 5 is a failed attempt.
    assert_eq!(undo_code(&set, 3), None);
    change_with(
        &ChangeRequest {
            set: set.clone(),
            change_set: json!({"operations": [{"op": "nope"}]}),
            dry_run: false,
        },
        &Settings::default(),
    );
    let now = snapshot(&set);
    for entry in 1..=2 {
        assert_eq!(
            drawn(&set, Some(entry)).unwrap(),
            states[entry as usize],
            "entry {entry}"
        );
    }
    // Change set 3 was undone: the journal keeps no copy of what it left.
    let mut request = DisplayListRequest::new(&set);
    request.entry = Some(3);
    let error = display_list_with(&request, &Settings::default()).unwrap_err();
    assert_eq!(error.code, ErrorCode::Pruned);
    assert!(error.message.contains("undone"), "{}", error.message);
    for entry in [4, 5] {
        assert_eq!(
            drawn(&set, Some(entry)).unwrap(),
            states[2],
            "entry {entry}"
        );
    }
    assert_eq!(snapshot(&set), now, "nothing is written");
    assert_eq!(drawn(&set, Some(6)).unwrap_err(), ErrorCode::UnknownName);

    // Once KEPT_UNDO_STATES newer change sets have landed, the oldest
    // states can no longer be rebuilt, and say why.
    for _ in 0..=KEPT_UNDO_STATES {
        apply(&set, moved(0.01));
    }
    let error = display_list_with(
        &DisplayListRequest {
            entry: Some(1),
            ..DisplayListRequest::new(&set)
        },
        &Settings::default(),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::Pruned);
    assert!(
        error
            .message
            .contains("the set as it was at entry 1 cannot be shown")
            && error.message.contains("was pruned when change set"),
        "{}",
        error.message
    );
    // The newest entries are drawn as they were.
    let newest = journal_lines(&set).last().unwrap()["entry"]
        .as_u64()
        .unwrap();
    assert_eq!(
        drawn(&set, Some(newest)).unwrap(),
        drawn(&set, None).unwrap()
    );
}
