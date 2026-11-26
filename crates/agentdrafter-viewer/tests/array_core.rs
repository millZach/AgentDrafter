//! Array's preview against the core (#66): each polar array the tool sets
//! up is sent through the core's own `change`, as a dry run on a copy of
//! the demo plant, and every copy the core would land turns as far, and
//! in the same order, as the copy the preview draws.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_set_with, change_with, display_list_with, ChangeRequest, DisplayListRequest,
    SetBuildRequest, Settings,
};
use agentdrafter_viewer::array::PathCurve;
use agentdrafter_viewer::tools::{load_geometry, Context, ObjectInfo, Reach, Taken, Tools};

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let name = entry.file_name();
        if name == "dwg" || name == "pdf" || name == ".agentdrafter" {
            continue;
        }
        if path.is_dir() {
            copy_dir(&path, &to.join(name));
        } else {
            fs::copy(&path, to.join(name)).unwrap();
        }
    }
}

/// A built copy of the demo plant, made up and public, in a folder of the
/// test's own, as the tests run side by side.
fn plant(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("viewer-array-core")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    let set = dir.join("demo-plant");
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo-plant"),
        &set,
    );
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

/// The tools on DP-LD-101, its device, the row's own, the one object they know.
fn tools() -> Tools {
    let mut t = Tools::default();
    t.load(Context {
        sheet: "DP-LD-101".to_owned(),
        units: Some("in".to_owned()),
        objects: vec![ObjectInfo {
            id: Some("DP-LD-101/device".to_owned()),
            kind: "insert".to_owned(),
            layer: "0".to_owned(),
            from_template: Some(false),
            bounds: Some([1.5, 6.3, 2.5, 6.7]),
            ..ObjectInfo::default()
        }],
        snaps: Vec::new(),
        template: Some(Reach {
            path: "templates/loop-2-wire.yaml".to_owned(),
            sheets: vec!["DP-LD-101".to_owned()],
        }),
        editable_dwg: false,
        past: false,
        space: None,
        layout: None,
    });
    t
}

#[test]
fn each_polar_copy_lands_where_the_preview_draws_it() {
    let set = plant("each");
    // What is typed in the command bar, with the total held or not.
    let cases: [(&str, bool); 6] = [
        ("4,100", false),
        ("3,120", false),
        ("7", false),
        ("8,-45", false),
        ("4,-360", true),
        ("5,-180", true),
    ];
    for (typed, total) in cases {
        let mut t = tools();
        t.start_word("ARRAYPOLAR", &[0]);
        t.point(Taken::typed([2.0, 4.5], 0.0));
        if total {
            t.option("total");
        }
        t.input(typed, None);
        let shown: Vec<f64> = t
            .band(None)
            .arrayed
            .unwrap_or_else(|| panic!("{typed}: no copy shown"))
            .1
            .iter()
            .map(|p| p.turn)
            .collect();
        let out = t.input("", None);
        let send = out
            .sends
            .first()
            .unwrap_or_else(|| panic!("{typed}: nothing sent"));
        let reply = change_with(
            &ChangeRequest {
                set: set.clone(),
                change_set: send.change_set.clone(),
                dry_run: true,
            },
            &Settings::default(),
        );
        assert!(reply.ok, "{typed}: {:#?}", reply.error);
        let report = reply.result.unwrap();
        let copies = &report.operations[0].copies;
        let landed: Vec<f64> = copies.iter().map(|c| c.turned.unwrap_or(0.0)).collect();
        let items: Vec<usize> = copies.iter().map(|c| c.item.unwrap()).collect();
        assert_eq!(
            items,
            (1..=shown.len()).collect::<Vec<_>>(),
            "{typed}: one copy landed for each shown, in order"
        );
        for (k, (a, b)) in shown.iter().zip(&landed).enumerate() {
            assert!(
                (a - b).abs() < 1e-6,
                "{typed}: copy {} shown turned {a}°, landed {b}°; sent {}",
                k + 1,
                send.change_set
            );
        }
    }
}

#[test]
fn switching_between_items_and_the_total_keeps_the_preview_where_the_core_lands_it() {
    let set = plant("switching");
    // Typed in the command bar, then the spacing held switched once or twice.
    let cases: [(&str, &[&str]); 6] = [
        // Four 120° apart, refused as they stand: with the total held, a
        // whole turn spread evenly, 90° apart.
        ("4,120", &["total"]),
        ("4,120", &["total", "between"]),
        // Less than a turn, either way round.
        ("5,45", &["total"]),
        ("5,45", &["total", "between"]),
        ("5,-45", &["total", "between", "total"]),
        // A whole turn stays one.
        ("8,45", &["total", "between"]),
    ];
    for (typed, switches) in cases {
        let mut t = tools();
        t.start_word("ARRAYPOLAR", &[0]);
        t.point(Taken::typed([2.0, 4.5], 0.0));
        t.input(typed, None);
        for s in switches {
            t.option(s);
        }
        let what = format!("{typed} then {}", switches.join(", "));
        let shown: Vec<f64> = t
            .band(None)
            .arrayed
            .unwrap_or_else(|| panic!("{what}: no copy shown"))
            .1
            .iter()
            .map(|p| p.turn)
            .collect();
        let out = t.input("", None);
        let send = out
            .sends
            .first()
            .unwrap_or_else(|| panic!("{what}: nothing sent"));
        let reply = change_with(
            &ChangeRequest {
                set: set.clone(),
                change_set: send.change_set.clone(),
                dry_run: true,
            },
            &Settings::default(),
        );
        assert!(reply.ok, "{what}: {:#?}", reply.error);
        let copies = &reply.result.unwrap().operations[0].copies;
        let landed: Vec<f64> = copies.iter().map(|c| c.turned.unwrap_or(0.0)).collect();
        assert_eq!(
            shown.len(),
            landed.len(),
            "{what}: as many copies shown as land"
        );
        for (k, (a, b)) in shown.iter().zip(&landed).enumerate() {
            assert!(
                (a - b).abs() < 1e-6,
                "{what}: copy {} shown turned {a}°, landed {b}°; sent {}",
                k + 1,
                send.change_set
            );
        }
    }
}

#[test]
fn each_path_copy_lands_where_the_preview_draws_it() {
    let set = plant("path");
    let mut base = Tools::default();
    base.load(Context {
        sheet: "DP-LD-101".to_owned(),
        units: Some("in".to_owned()),
        objects: vec![
            ObjectInfo {
                id: Some("DP-LD-101/device".to_owned()),
                kind: "insert".to_owned(),
                layer: "0".to_owned(),
                from_template: Some(false),
                bounds: Some([1.5, 6.3, 2.5, 6.7]),
                ..ObjectInfo::default()
            },
            // Open line from templates/loop-2-wire.yaml: [3.25, 6.75] -> [7.2, 6.75]
            ObjectInfo {
                id: Some("DP-LD-101/field-wire-1".to_owned()),
                kind: "line".to_owned(),
                layer: "wire".to_owned(),
                from_template: Some(true),
                bounds: Some([3.25, 6.75, 7.2, 6.75]),
                path: PathCurve::from_runs("line", &[&[[3.25, 6.75], [7.2, 6.75]]]),
                ..ObjectInfo::default()
            },
            // Closed polyline border from templates/loop-2-wire.yaml
            ObjectInfo {
                id: Some("DP-LD-101/border".to_owned()),
                kind: "polyline".to_owned(),
                layer: "border".to_owned(),
                from_template: Some(true),
                bounds: Some([0.5, 0.5, 16.5, 10.5]),
                path: PathCurve::from_runs(
                    "polyline",
                    &[&[
                        [0.5, 0.5],
                        [16.5, 0.5],
                        [16.5, 10.5],
                        [0.5, 10.5],
                        [0.5, 0.5],
                    ]],
                ),
                ..ObjectInfo::default()
            },
        ],
        snaps: Vec::new(),
        template: Some(Reach {
            path: "templates/loop-2-wire.yaml".to_owned(),
            sheets: vec!["DP-LD-101".to_owned()],
        }),
        editable_dwg: false,
        past: false,
        space: None,
        layout: None,
    });

    // (path index, command-bar input, align)
    let cases: [(usize, &str, bool); 4] = [
        (1, "4", true),
        (1, "3,1.2", true),
        (2, "6", true),
        (2, "5,8", false),
    ];
    for (path_idx, typed, align) in cases {
        let mut t = base.clone();
        t.start_word("ARRAYPATH", &[0]);
        t.pick(Some(path_idx));
        if !align {
            t.option("no-align");
        }
        t.input(typed, None);
        let shown = t
            .band(None)
            .arrayed
            .unwrap_or_else(|| panic!("path {path_idx} {typed}: no copy shown"))
            .1;
        let out = t.input("", None);
        let send = out
            .sends
            .first()
            .unwrap_or_else(|| panic!("path {path_idx} {typed}: nothing sent"));
        let reply = change_with(
            &ChangeRequest {
                set: set.clone(),
                change_set: send.change_set.clone(),
                dry_run: true,
            },
            &Settings::default(),
        );
        assert!(reply.ok, "path {path_idx} {typed}: {:#?}", reply.error);
        let copies = &reply.result.unwrap().operations[0].copies;
        assert_eq!(shown.len(), copies.len(), "path {path_idx} {typed}");
        for (k, (place, landed)) in shown.iter().zip(copies).enumerate() {
            let turn = landed.turned.unwrap_or(0.0);
            assert!(
                (place.turn - turn).abs() < 1e-6,
                "path {path_idx} {typed}: copy {} shown turned {}°, landed {turn}°; sent {}",
                k + 1,
                place.turn,
                send.change_set
            );
            let moved = landed.by;
            assert!(
                (place.by[0] - moved[0]).abs() < 1e-6 && (place.by[1] - moved[1]).abs() < 1e-6,
                "path {path_idx} {typed}: copy {} shown by {:?}, landed {:?}; sent {}",
                k + 1,
                place.by,
                moved,
                send.change_set
            );
        }
    }
}

#[test]
fn near_full_polar_arrays_match_core_and_unturned_hatch_uses_boundary_box() {
    let set = plant("rare259");

    // Near-full polar fills (#259 items 7 & 8): preview and core agree on every copy.
    for (typed, switch_total) in [("4,119.9999999", false), ("44,8.372093", true)] {
        let mut t = tools();
        t.start_word("ARRAYPOLAR", &[0]);
        t.point(Taken::typed([2.0, 4.5], 0.0));
        t.input(typed, None);
        if switch_total {
            t.option("total");
        }
        let shown: Vec<f64> = t
            .band(None)
            .arrayed
            .unwrap()
            .1
            .iter()
            .map(|p| p.turn)
            .collect();
        let send = t.input("", None).sends.remove(0);
        let reply = change_with(
            &ChangeRequest {
                set: set.clone(),
                change_set: send.change_set.clone(),
                dry_run: true,
            },
            &Settings::default(),
        );
        assert!(reply.ok, "{typed}: {:#?}", reply.error);
        let copies = &reply.result.unwrap().operations[0].copies;
        assert_eq!(shown.len(), copies.len(), "{typed}");
        for (k, (a, c)) in shown.iter().zip(copies).enumerate() {
            let b = c.turned.unwrap_or(0.0);
            assert!(
                (a - b).abs() < 1e-5,
                "{typed}: copy {} shown {a}°, landed {b}°; sent {}",
                k + 1,
                send.change_set
            );
        }
    }

    // Unturned polar array of a patterned hatch whose single stroke at y=1.5 is
    // asymmetric inside its [1,1]..[5,4] boundary (#259 item 1): preview must
    // use `list.boundaries` (centre [3.0, 2.5]) just like core `extents_center`,
    // not the stroke box alone (centre [3.0, 1.5]).
    let tpl_path = set.join("templates/loop-2-wire.yaml");
    let tpl_text = fs::read_to_string(&tpl_path).unwrap();
    fs::write(
        &tpl_path,
        tpl_text.replace(
            "linetypes:\n",
            "hatch_patterns:\n  SPARSE:\n    lines:\n      - {angle: 0, origin: [0, 1.5], delta: [0, 10]}\nlinetypes:\n",
        ),
    )
    .unwrap();
    let rebuilt = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: true,
            clean: None,
        },
        &Settings::default(),
    );
    assert!(rebuilt.ok, "{:#?}", rebuilt.error);
    let added = change_with(
        &ChangeRequest {
            set: set.clone(),
            change_set: serde_json::json!({
                "operations": [{
                    "op": "add_hatch",
                    "sheet": "DP-LD-101",
                    "id": "sparse-hatch",
                    "layer": "notes",
                    "pattern": "SPARSE",
                    "boundary": [{
                        "polyline": {
                            "points": [[1.0, 1.0, 0.0], [5.0, 1.0, 0.0], [5.0, 4.0, 0.0], [1.0, 4.0, 0.0]]
                        }
                    }]
                }]
            }),
            dry_run: false,
        },
        &Settings::default(),
    );
    assert!(added.ok, "{:#?}", added.error);
    let report = display_list_with(
        &DisplayListRequest {
            source: set.clone(),
            sheet: Some("DP-LD-101".to_owned()),
            tolerance: None,
            layout: None,
            entry: None,
            snaps: false,
            origins: true,
            values: false,
            change_set: None,
            all_or_none: false,
            read_past_damage: false,
        },
        &Settings::default(),
    )
    .expect("display_list_with succeeds");
    let mut objects: Vec<ObjectInfo> = report
        .objects
        .iter()
        .map(|o| ObjectInfo::of(&serde_json::to_value(o).unwrap()))
        .collect();
    load_geometry(&mut objects, &report.display_list);
    let hatch_idx = objects
        .iter()
        .position(|o| o.id.as_deref() == Some("DP-LD-101/sparse-hatch"))
        .expect("sparse-hatch present");
    // The full boundary box [1, 1, 5, 4] must be captured even if pattern strokes are smaller.
    assert_eq!(objects[hatch_idx].bounds, Some([1.0, 1.0, 5.0, 4.0]));
    let mut t = Tools::default();
    t.load(Context {
        sheet: "DP-LD-101".to_owned(),
        units: Some("in".to_owned()),
        objects,
        snaps: Vec::new(),
        template: Some(Reach {
            path: "templates/loop-2-wire.yaml".to_owned(),
            sheets: vec!["DP-LD-101".to_owned()],
        }),
        editable_dwg: false,
        past: false,
        space: None,
        layout: None,
    });
    t.start_word("ARRAYPOLAR", &[hatch_idx]);
    t.point(Taken::typed([0.0, 0.0], 0.0));
    t.option("no-turn");
    t.input("4", None);
    let shown = t.band(None).arrayed.unwrap().1;
    let send = t.input("", None).sends.remove(0);
    let reply = change_with(
        &ChangeRequest {
            set,
            change_set: send.change_set.clone(),
            dry_run: true,
        },
        &Settings::default(),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    let copies = &reply.result.unwrap().operations[0].copies;
    assert_eq!(shown.len(), copies.len());
    for (k, (place, landed)) in shown.iter().zip(copies).enumerate() {
        assert!(
            (place.by[0] - landed.by[0]).abs() < 1e-6 && (place.by[1] - landed.by[1]).abs() < 1e-6,
            "hatch copy {} shown by {:?}, landed {:?}; sent {}",
            k + 1,
            place.by,
            landed.by,
            send.change_set
        );
    }
}
