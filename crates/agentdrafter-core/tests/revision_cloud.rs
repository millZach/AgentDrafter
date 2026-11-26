//! End-to-end tests for revision clouds (#72): sheet description + JSON Schema
//! agreement, DWG build / render (PNG) / plot (PDF) / display list / check /
//! query, `add_revision_cloud` (`REVCLOUD`) in drawing-set change sets
//! (`rectangle`, `polygon`, `around` + `margin`, snapped points, transforms,
//! undo) and revising an original DWG with `add_revision_cloud`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use agentdrafter_core::{
    build_set_with, build_with, change_set_schema, change_with, check_with, display_list_with,
    operations, parse_sheet_description, plot_with, query_with, read_dwg, render_with, revise_with,
    sheet_description_schema, undo, BuildRequest, ChangeRequest, CheckRequest, DisplayListRequest,
    ErrorCode, Object, PlotRequest, Query, QueryRequest, RenderRequest, ReviseRequest,
    RevisionCloudBoundary, RevisionCloudStyle, SetBuildRequest, Settings, UndoRequest,
};
use serde_json::{json, Value};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "agentdrafter-revcloud-{name}-{}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn copy_demo_set(to: &Path) {
    let src = repo_root().join("examples").join("demo-set");
    copy_dir(&src, to);
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("mkdir");
    for entry in fs::read_dir(from).expect("read_dir") {
        let entry = entry.expect("entry");
        let name = entry.file_name();
        if name == "out" || name == "dwg" || name == ".agentdrafter" {
            continue;
        }
        let dest = to.join(name);
        if entry.file_type().expect("type").is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), dest).expect("copy");
        }
    }
}

#[test]
fn example_revision_cloud_builds_renders_plots_displays_and_checks() {
    let dir = scratch("example");
    let src = repo_root().join("examples").join("revision-cloud.yaml");
    let yaml_text = fs::read_to_string(&src).expect("read example");
    let sheet = parse_sheet_description(&yaml_text).expect("parse example");

    let clouds: Vec<_> = sheet
        .objects
        .iter()
        .filter_map(|o| match o {
            Object::RevisionCloud(c) => Some(c),
            _ => None,
        })
        .collect();
    assert_eq!(clouds.len(), 4);
    assert_eq!(clouds[0].style, RevisionCloudStyle::Normal);
    assert!(!clouds[0].reverse);
    assert!(matches!(
        clouds[0].boundary,
        RevisionCloudBoundary::Rectangle { .. }
    ));
    assert_eq!(clouds[1].style, RevisionCloudStyle::Calligraphy);
    assert!(matches!(
        clouds[2].boundary,
        RevisionCloudBoundary::Polygon(_)
    ));
    assert_eq!(clouds[3].style, RevisionCloudStyle::Calligraphy);
    assert!(clouds[3].reverse);

    let settings = Settings::default();
    let dwg_path = dir.join("revision-cloud.dwg");
    let built = build_with(
        &BuildRequest {
            description: src.clone(),
            output: Some(dwg_path.clone()),
            overwrite: true,
        },
        &settings,
    )
    .expect("build DWG");
    assert!(Path::new(&built.dwg).exists());

    // DWG reads back cleanly with closed polylines (1 trim + 4 clouds = 5).
    let read = read_dwg(&dwg_path).expect("read DWG");
    assert_eq!(read.objects_not_read, 0);
    let lw_count = read
        .objects
        .iter()
        .filter(|o| matches!(o, Object::Polyline(_)))
        .count();
    assert_eq!(lw_count, 5);

    // PNG render, PDF plot, display list and checker all succeed on both YAML and DWG.
    let png_yaml = dir.join("from-yaml.png");
    let png_dwg = dir.join("from-dwg.png");
    let r_yaml_req: RenderRequest = serde_json::from_value(json!({
        "source": src,
        "output": png_yaml,
        "overwrite": true,
    }))
    .unwrap();
    let r_dwg_req: RenderRequest = serde_json::from_value(json!({
        "source": dwg_path,
        "output": png_dwg,
        "overwrite": true,
    }))
    .unwrap();
    let r_yaml = render_with(&r_yaml_req, &settings).expect("render yaml");
    let r_dwg = render_with(&r_dwg_req, &settings).expect("render dwg");
    assert!(Path::new(&r_yaml.png).exists());
    assert!(Path::new(&r_dwg.png).exists());
    assert!(fs::metadata(&r_yaml.png).unwrap().len() > 2_000);
    assert!(fs::metadata(&r_dwg.png).unwrap().len() > 2_000);

    let pdf_path = dir.join("revision-cloud.pdf");
    let plotted = plot_with(
        &PlotRequest {
            source: src.clone(),
            output: Some(pdf_path.clone()),
            overwrite: true,
            plot_style: None,
        },
        &settings,
    )
    .expect("plot pdf");
    assert!(Path::new(&plotted.pdf).exists());
    assert!(fs::metadata(&plotted.pdf).unwrap().len() > 1_000);

    let dl_req: DisplayListRequest = serde_json::from_value(json!({
        "source": src,
    }))
    .unwrap();
    let dl = display_list_with(&dl_req, &settings).expect("display list");
    assert_eq!(dl.objects.len(), 9, "display list must carry all 9 objects");
    assert!(dl.extents.is_some(), "display list must have extents");

    let check_report = check_with(
        &CheckRequest {
            source: src.clone(),
            sheets: Vec::new(),
        },
        &settings,
    )
    .expect("check example");
    assert!(
        check_report.passed,
        "example sheet should pass all default checks: {:?}",
        check_report.sheets
    );

    let ext = query_with(
        &QueryRequest {
            source: src.clone(),
            sheet: None,
            query: Query::Extents {},
        },
        &settings,
    )
    .expect("extents query");
    assert!(!ext.message.is_empty());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn sheet_schema_and_reader_agree_on_revision_cloud_cases() {
    let schema_json = sheet_description_schema();
    let validator = jsonschema::validator_for(&schema_json).expect("valid schema");

    let cases: &[(&str, Value, bool)] = &[
        (
            "units: in\nobjects:\n  - revision_cloud:\n      rectangle: {from: [0, 0, 0], to: [4, 2, 0]}\n      arc_length: 0.5\n",
            json!({"units": "in", "objects": [{"revision_cloud": {"rectangle": {"from": [0, 0, 0], "to": [4, 2, 0]}, "arc_length": 0.5}}]}),
            true,
        ),
        (
            "units: in\nobjects:\n  - revision_cloud:\n      polygon: [[0, 0, 0], [4, 0, 0], [4, 2, 0], [0, 2, 0]]\n      arc_length: 0.5\n      style: calligraphy\n      reverse: true\n",
            json!({"units": "in", "objects": [{"revision_cloud": {"polygon": [[0, 0, 0], [4, 0, 0], [4, 2, 0], [0, 2, 0]], "arc_length": 0.5, "style": "calligraphy", "reverse": true}}]}),
            true,
        ),
        (
            "units: in\nobjects:\n  - revision_cloud:\n      rectangle: {from: [0, 0, 0], to: [4, 2, 0]}\n      polygon: [[0, 0, 0], [4, 0, 0], [4, 2, 0]]\n      arc_length: 0.5\n",
            json!({"units": "in", "objects": [{"revision_cloud": {"rectangle": {"from": [0, 0, 0], "to": [4, 2, 0]}, "polygon": [[0, 0, 0], [4, 0, 0], [4, 2, 0]], "arc_length": 0.5}}]}),
            false,
        ),
        (
            "units: in\nobjects:\n  - revision_cloud:\n      arc_length: 0.5\n",
            json!({"units": "in", "objects": [{"revision_cloud": {"arc_length": 0.5}}]}),
            false,
        ),
        (
            "units: in\nobjects:\n  - revision_cloud:\n      rectangle: {from: [0, 0, 0], to: [4, 2, 0]}\n",
            json!({"units": "in", "objects": [{"revision_cloud": {"rectangle": {"from": [0, 0, 0], "to": [4, 2, 0]}}}]}),
            false,
        ),
        (
            "units: in\nobjects:\n  - revision_cloud:\n      rectangle: {from: [0, 0, 0], to: [4, 2, 0]}\n      arc_length: 0\n",
            json!({"units": "in", "objects": [{"revision_cloud": {"rectangle": {"from": [0, 0, 0], "to": [4, 2, 0]}, "arc_length": 0}}]}),
            false,
        ),
        (
            "units: in\nobjects:\n  - revision_cloud:\n      rectangle: {from: [0, 0, 0], to: [4, 2, 0]}\n      arc_length: 0.5\n      style: sketchy\n",
            json!({"units": "in", "objects": [{"revision_cloud": {"rectangle": {"from": [0, 0, 0], "to": [4, 2, 0]}, "arc_length": 0.5, "style": "sketchy"}}]}),
            false,
        ),
        (
            "units: in\nobjects:\n  - revision_cloud:\n      polygon: [[0, 0, 0], [4, 0, 0]]\n      arc_length: 0.5\n",
            json!({"units": "in", "objects": [{"revision_cloud": {"polygon": [[0, 0, 0], [4, 0, 0]], "arc_length": 0.5}}]}),
            false,
        ),
    ];

    for (yaml, json_doc, expect_ok) in cases {
        assert_eq!(
            validator.is_valid(json_doc),
            *expect_ok,
            "schema disagreed on {json_doc}"
        );
        assert_eq!(
            parse_sheet_description(yaml).is_ok(),
            *expect_ok,
            "reader disagreed on {yaml}"
        );
    }

    let bad_geom = [
        "units: in\nobjects:\n  - revision_cloud:\n      rectangle: {from: [1, 0, 0], to: [1, 2, 0]}\n      arc_length: 0.5\n",
        "units: in\nobjects:\n  - revision_cloud:\n      rectangle: {from: [0, 0, 0], to: [4, 2, 1]}\n      arc_length: 0.5\n",
        "units: in\nobjects:\n  - revision_cloud:\n      polygon: [[0, 0, 0], [2, 0, 0], [4, 0, 0]]\n      arc_length: 0.5\n",
    ];
    for yaml in bad_geom {
        assert!(
            parse_sheet_description(yaml).is_err(),
            "expected reader error for {yaml}"
        );
    }
}

#[test]
fn change_set_schema_validates_add_revision_cloud_forms() {
    let ops = operations();
    assert!(
        ops.operations
            .iter()
            .any(|o| o.name == "add_revision_cloud" && o.alias == "REVCLOUD"),
        "add_revision_cloud / REVCLOUD must be registered"
    );

    let schema = change_set_schema();
    let validator = jsonschema::validator_for(&schema).expect("valid change set schema");

    let valid = [
        json!({"operations": [{"op": "add_revision_cloud", "sheet": "S", "rectangle": {"from": [0, 0, 0], "to": [4, 2, 0]}, "arc_length": 0.5}]}),
        json!({"operations": [{"op": "REVCLOUD", "template": "t.yaml", "polygon": [[0, 0, 0], [4, 0, 0], [4, 2, 0]], "arc_length": 0.5, "style": "calligraphy", "reverse": true}]}),
        json!({"operations": [{"op": "REVCLOUD", "around": {"id": "S/a"}}]}),
        json!({"operations": [{"op": "add_revision_cloud", "around": [{"id": "S/a"}, {"name": "b", "sheet": "S"}], "margin": 0.25, "arc_length": 0.4, "style": "calligraphy"}]}),
    ];
    for doc in valid {
        assert!(validator.is_valid(&doc), "expected valid: {doc}");
    }

    let invalid = [
        json!({"operations": [{"op": "add_revision_cloud", "sheet": "S", "around": {"id": "S/a"}, "rectangle": {"from": [0, 0, 0], "to": [4, 2, 0]}, "arc_length": 0.5}]}),
        json!({"operations": [{"op": "add_revision_cloud", "sheet": "S", "rectangle": {"from": [0, 0, 0], "to": [4, 2, 0]}, "arc_length": 0.5, "margin": 0.2}]}),
        json!({"operations": [{"op": "add_revision_cloud", "sheet": "S", "rectangle": {"from": [0, 0, 0], "to": [4, 2, 0]}}]}),
        json!({"operations": [{"op": "add_revision_cloud", "around": []}]}),
        json!({"operations": [{"op": "add_revision_cloud", "rectangle": {"from": [0, 0, 0], "to": [4, 2, 0]}, "arc_length": 0.5}]}),
    ];
    for doc in invalid {
        assert!(!validator.is_valid(&doc), "expected invalid: {doc}");
    }
}

#[test]
fn drawing_set_revcloud_around_transforms_and_undo() {
    let dir = scratch("set-revcloud");
    copy_demo_set(&dir);
    let settings = Settings::default();
    let built = build_set_with(
        &SetBuildRequest {
            set: dir.clone(),
            overwrite: true,
            clean: None,
        },
        &settings,
    );
    assert!(built.ok, "initial build failed: {:?}", built.error);

    let rows_before = fs::read(dir.join("rows.yaml")).expect("read rows before");

    let change_doc = json!({
        "operations": [
            {
                "op": "add_line",
                "sheet": "DEMO-001",
                "id": "rev-target-a",
                "from": [2.0, 2.0, 0.0],
                "to": [4.0, 3.0, 0.0],
                "layer": "notes"
            },
            {
                "op": "add_line",
                "sheet": "DEMO-001",
                "id": "rev-target-b",
                "from": [4.5, 2.5, 0.0],
                "to": [6.0, 4.0, 0.0],
                "layer": "notes"
            },
            {
                "op": "REVCLOUD",
                "id": "cloud-auto",
                "around": { "id": "DEMO-001/rev-target-a" },
                "layer": "notes"
            },
            {
                "op": "add_revision_cloud",
                "id": "cloud-multi",
                "around": [
                    { "id": "DEMO-001/rev-target-a" },
                    { "id": "DEMO-001/rev-target-b" }
                ],
                "margin": 0.25,
                "arc_length": 0.5,
                "style": "calligraphy",
                "reverse": false,
                "layer": "notes"
            },
            {
                "op": "add_revision_cloud",
                "id": "cloud-snapped",
                "sheet": "DEMO-001",
                "rectangle": {
                    "from": { "snap": { "id": "DEMO-001/rev-target-a", "snap": "endpoint", "which": "from" } },
                    "to": [5.0, 5.0, 0.0]
                },
                "arc_length": 0.4,
                "layer": "notes"
            }
        ]
    });
    let reply = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: change_doc,
            dry_run: false,
        },
        &settings,
    );
    assert!(reply.ok, "change set failed: {:?}", reply.error);
    let report = reply.result.expect("change report");
    assert_eq!(report.failed, 0);
    let entry_id = report.journal_entry.expect("recorded journal entry");

    // Verify `cloud-multi` framed both targets [2..6, 2..4] +/- 0.25 -> [1.75..6.25, 1.75..4.25].
    let rows_after = fs::read_to_string(dir.join("rows.yaml")).expect("read rows");
    assert!(rows_after.contains("cloud-auto"));
    assert!(rows_after.contains("cloud-multi"));
    assert!(rows_after.contains("1.75"));
    assert!(rows_after.contains("6.25"));
    assert!(rows_after.contains("4.25"));
    assert!(rows_after.contains("calligraphy"));

    // Transform the clouds: move, change, scale (scales arc_length), rotate 90 (stays rectangle),
    // rotate 30 (converts rectangle to polygon), copy.
    let transform_doc = json!({
        "operations": [
            {
                "op": "move",
                "target": { "id": "DEMO-001/cloud-auto" },
                "by": [0.5, 0.25, 0.0]
            },
            {
                "op": "change",
                "target": { "id": "DEMO-001/cloud-auto" },
                "set": { "style": "calligraphy", "reverse": true }
            },
            {
                "op": "scale",
                "target": { "id": "DEMO-001/cloud-multi" },
                "base_point": [4.0, 3.0, 0.0],
                "factor": 1.2
            },
            {
                "op": "rotate",
                "target": { "id": "DEMO-001/cloud-multi" },
                "base_point": [4.0, 3.0, 0.0],
                "angle": 90
            },
            {
                "op": "rotate",
                "target": { "id": "DEMO-001/cloud-snapped" },
                "base_point": [3.5, 3.5, 0.0],
                "angle": 30
            },
            {
                "op": "copy",
                "target": { "id": "DEMO-001/cloud-snapped" },
                "by": [1.0, 0.0, 0.0]
            }
        ]
    });
    let t_reply = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: transform_doc,
            dry_run: false,
        },
        &settings,
    );
    assert!(
        t_reply.ok,
        "transform change set failed: {:?}",
        t_reply.error
    );
    assert_eq!(t_reply.result.unwrap().failed, 0);

    let rows_transformed = fs::read_to_string(dir.join("rows.yaml")).expect("read rows");
    assert!(
        rows_transformed.contains("polygon:"),
        "30-degree rotate should convert rectangle cloud to polygon:\n{rows_transformed}"
    );
    assert!(
        rows_transformed.contains("arc_length: 0.6"),
        "scale by 1.2 should scale arc_length 0.5 -> 0.6:\n{rows_transformed}"
    );

    // Undo back to the first change set restores `rows.yaml` byte for byte.
    let undone = undo(&UndoRequest {
        set: dir.clone(),
        change_set: entry_id,
        newest_seen: None,
        dry_run: false,
    });
    assert!(undone.ok, "undo failed: {:?}", undone.error);
    let rows_restored = fs::read(dir.join("rows.yaml")).expect("read rows restored");
    assert_eq!(rows_before, rows_restored);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn revise_original_dwg_with_add_revision_cloud_and_around() {
    let dir = scratch("revise-orig");
    let settings = Settings::default();

    let yaml = dir.join("base.yaml");
    fs::write(
        &yaml,
        "units: in\npaper: ANSI A landscape\nlayers:\n  REVCLOUD: {color: red}\nobjects:\n  - polyline:\n      id: device-box\n      points: [[2.0, 2.0, 0], [5.0, 2.0, 0], [5.0, 4.0, 0], [2.0, 4.0, 0]]\n      closed: true\n",
    )
    .expect("write yaml");
    let orig_dwg = dir.join("base.dwg");
    build_with(
        &BuildRequest {
            description: yaml,
            output: Some(orig_dwg.clone()),
            overwrite: true,
        },
        &settings,
    )
    .expect("build base dwg");
    let orig_bytes = fs::read(&orig_dwg).expect("read original bytes");

    let change_doc = json!({
        "operations": [
            {
                "op": "REVCLOUD",
                "id": "around-device",
                "around": { "id": "base/device-box" },
                "margin": 0.3,
                "arc_length": 0.5,
                "style": "calligraphy",
                "layer": "REVCLOUD"
            },
            {
                "op": "add_revision_cloud",
                "id": "poly-note",
                "polygon": [[6.0, 2.0, 0], [9.5, 2.0, 0], [9.5, 4.5, 0], [6.0, 4.5, 0]],
                "arc_length": 0.45,
                "style": "normal",
                "layer": "REVCLOUD"
            }
        ]
    });

    let rev_req: ReviseRequest = serde_json::from_value(json!({
        "set": orig_dwg,
        "change_set": change_doc,
        "dry_run": false,
    }))
    .unwrap();
    let rev_reply = revise_with(&rev_req, &settings);
    assert!(rev_reply.ok, "revise failed: {:?}", rev_reply.error);
    let rev = rev_reply.result.expect("revision report");
    assert_eq!(rev.operations.len(), 2);
    assert!(rev.operations.iter().all(|o| o.ok));
    assert!(
        rev.unexpected_changes.is_empty(),
        "untouched verification must report 0 unexpected changes: {:?}",
        rev.unexpected_changes
    );

    // Original DWG untouched byte-for-byte.
    assert_eq!(fs::read(&orig_dwg).unwrap(), orig_bytes);

    // Revised DWG reads cleanly and carries 3 closed polylines (device-box + 2 clouds).
    let rev_dwg = dir.join(&rev.revision.as_ref().expect("revision file").file);
    let read_rev = read_dwg(&rev_dwg).expect("read revised dwg");
    assert_eq!(read_rev.objects_not_read, 0);
    let lw_count = read_rev
        .objects
        .iter()
        .filter(|o| matches!(o, Object::Polyline(_)))
        .count();
    assert_eq!(lw_count, 3);

    // Error refusal: negative margin on original DWG fails cleanly.
    let bad_req: ReviseRequest = serde_json::from_value(json!({
        "set": orig_dwg,
        "change_set": {
            "operations": [{
                "op": "REVCLOUD",
                "around": { "id": "base/device-box" },
                "margin": -0.1
            }]
        },
        "dry_run": true,
    }))
    .unwrap();
    let bad_reply = revise_with(&bad_req, &settings);
    assert!(!bad_reply.ok, "negative margin must fail");
    assert_eq!(bad_reply.error.unwrap().code, ErrorCode::BadValue);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn around_refusal_cases_in_drawing_set() {
    let dir = scratch("refusals");
    copy_demo_set(&dir);
    let settings = Settings::default();
    let built = build_set_with(
        &SetBuildRequest {
            set: dir.clone(),
            overwrite: true,
            clean: None,
        },
        &settings,
    );
    assert!(built.ok);

    let cases = [
        json!({"operations": [{"op": "REVCLOUD", "around": {"id": "DEMO-001/border"}, "rectangle": {"from": [0, 0, 0], "to": [1, 1, 0]}, "arc_length": 0.5}]}),
        json!({"operations": [{"op": "REVCLOUD", "sheet": "DEMO-001", "rectangle": {"from": [0, 0, 0], "to": [1, 1, 0]}, "arc_length": 0.5, "margin": 0.1}]}),
        json!({"operations": [{"op": "REVCLOUD", "around": []}]}),
    ];
    for doc in cases {
        let reply = change_with(
            &ChangeRequest {
                set: dir.clone(),
                change_set: doc,
                dry_run: true,
            },
            &settings,
        );
        assert!(!reply.ok, "expected refusal");
        assert_eq!(reply.error.unwrap().code, ErrorCode::BadValue);
    }

    let _ = fs::remove_dir_all(&dir);
}
