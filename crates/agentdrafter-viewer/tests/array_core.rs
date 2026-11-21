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

/// A built copy of the demo plant, made up and public.
fn plant() -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("viewer-array-core");
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
        template: Some(Reach {
            path: "templates/loop-2-wire.yaml".to_owned(),
            sheets: vec!["DP-LD-101".to_owned()],
        }),
        past: false,
        space: None,
        layout: None,
    });
    t
}

#[test]
fn each_polar_copy_lands_where_the_preview_draws_it() {
    let set = plant();
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
