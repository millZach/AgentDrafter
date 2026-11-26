//! Array's preview against the core (#66): each polar array the tool sets
//! up is sent through the core's own `change`, as a dry run on a copy of
//! the demo plant, and every copy the core would land turns as far, and
//! in the same order, as the copy the preview draws.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{build_set_with, change_with, ChangeRequest, SetBuildRequest, Settings};
use agentdrafter_viewer::tools::{Context, ObjectInfo, Reach, Taken, Tools};

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
