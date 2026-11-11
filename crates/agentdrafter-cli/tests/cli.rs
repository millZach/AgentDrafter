//! The command prints one line of JSON on success and on failure.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

#[path = "../../agentdrafter-core/tests/support/demo_set.rs"]
mod demo_set;

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("cli")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs `agentdrafter` and returns its exit code and the JSON it printed.
fn agentdrafter(args: &[&str], cwd: &Path) -> (i32, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_agentdrafter"))
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        out.stderr.is_empty(),
        "nothing on standard error: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout.lines().count(), 1, "one line of JSON: {stdout}");
    let json: Value = serde_json::from_str(&stdout).unwrap();
    (out.status.code().unwrap(), json)
}

fn copy_example(dir: &Path) {
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/one-line.yaml"),
        dir.join("one-line.yaml"),
    )
    .unwrap();
}

#[test]
fn build_prints_json_on_success() {
    let dir = scratch("success");
    copy_example(&dir);

    let (code, json) = agentdrafter(&["build", "one-line.yaml"], &dir);

    assert_eq!(code, 0, "{json}");
    assert_eq!(json["ok"], true);
    let result = &json["result"];
    assert_eq!(result["description"], "one-line.yaml");
    assert_eq!(result["dwg"], "one-line.dwg");
    assert_eq!(result["dwg_version"], "AC1032");
    assert_eq!(result["units"], "mm");
    assert_eq!(result["objects"]["line"], 1);
    let bytes = fs::read(dir.join("one-line.dwg")).unwrap();
    assert_eq!(result["bytes"], bytes.len());
    assert!(json.get("error").is_none());
}

#[test]
fn build_writes_to_the_output_path_and_replaces_only_when_asked() {
    let dir = scratch("output");
    copy_example(&dir);

    let (code, json) = agentdrafter(&["build", "one-line.yaml", "--output", "a.dwg"], &dir);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["dwg"], "a.dwg");

    let (code, json) = agentdrafter(&["build", "one-line.yaml", "-o", "a.dwg"], &dir);
    assert_eq!(code, 1, "{json}");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "output_exists");
    assert_eq!(json["error"]["file"], "a.dwg");

    let (code, json) = agentdrafter(
        &["build", "--overwrite", "one-line.yaml", "-o", "a.dwg"],
        &dir,
    );
    assert_eq!(code, 0, "{json}");
}

#[test]
fn build_prints_json_on_failure_naming_the_field() {
    let dir = scratch("failure");
    fs::write(
        dir.join("sheet.yaml"),
        "units: mm\nobjects:\n  - line:\n      from: [0, 0]\n      to: [1, 1, 0]\n",
    )
    .unwrap();

    let (code, json) = agentdrafter(&["build", "sheet.yaml"], &dir);

    assert_eq!(code, 1, "{json}");
    assert_eq!(json["ok"], false);
    assert!(json.get("result").is_none());
    let error = &json["error"];
    assert_eq!(error["code"], "wrong_type");
    assert_eq!(error["field"], "objects[0].line.from");
    assert_eq!(error["line"], 4);
    assert_eq!(error["column"], 13);
    assert_eq!(error["file"], "sheet.yaml");
    assert!(error["message"]
        .as_str()
        .unwrap()
        .contains("three coordinates"));
    assert!(!dir.join("sheet.dwg").exists());
}

#[test]
fn a_missing_unit_is_reported_as_json() {
    let dir = scratch("missing-unit");
    fs::write(dir.join("sheet.yaml"), "objects: []\n").unwrap();

    let (code, json) = agentdrafter(&["build", "sheet.yaml"], &dir);

    assert_eq!(code, 1, "{json}");
    assert_eq!(json["error"]["code"], "missing_unit");
    assert_eq!(json["error"]["field"], "units");
}

#[test]
fn a_missing_file_is_reported_as_json() {
    let dir = scratch("missing-file");
    let (code, json) = agentdrafter(&["build", "absent.yaml"], &dir);
    assert_eq!(code, 1, "{json}");
    assert_eq!(json["error"]["code"], "read_failed");
    assert_eq!(json["error"]["file"], "absent.yaml");
}

#[test]
fn wrong_arguments_are_reported_as_json() {
    let dir = scratch("usage");
    for args in [
        &[][..],
        &["draw"][..],
        &["build"][..],
        &["build", "a.yaml", "b.yaml"][..],
        &["build", "a.yaml", "--colour"][..],
        &["build", "a.yaml", "--output"][..],
    ] {
        let (code, json) = agentdrafter(args, &dir);
        assert_eq!(code, 2, "{args:?}: {json}");
        assert_eq!(json["ok"], false, "{args:?}");
        assert_eq!(json["error"]["code"], "usage", "{args:?}");
    }
}

#[test]
fn help_and_version_are_json() {
    let dir = scratch("help");
    let (code, json) = agentdrafter(&["--help"], &dir);
    assert_eq!(code, 0);
    assert_eq!(json["ok"], true);
    assert!(json["result"]["commands"]["build"]["usage"]
        .as_str()
        .unwrap()
        .starts_with("agentdrafter build"));

    let (code, json) = agentdrafter(&["--version"], &dir);
    assert_eq!(code, 0);
    assert_eq!(json["result"]["version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn schema_prints_the_sheet_descriptions_json_schema() {
    let dir = scratch("schema");
    let (code, json) = agentdrafter(&["schema"], &dir);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["ok"], true);
    let schema = &json["result"];
    assert_eq!(
        schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(schema["required"], serde_json::json!(["units", "objects"]));
    assert_eq!(
        schema["properties"]["units"]["enum"],
        serde_json::json!(["mm", "cm", "m", "in", "ft"])
    );
    for object_type in [
        "line", "polyline", "arc", "circle", "ellipse", "spline", "hatch",
    ] {
        assert!(
            schema["$defs"][object_type]["properties"]["layer"].is_object(),
            "{object_type} takes a layer"
        );
    }
    assert!(schema["$defs"]["layer"]["properties"]["lineweight"].is_object());
    assert!(schema["$defs"]["linetype"]["properties"]["pattern"].is_object());
    assert!(schema["properties"]["hatch_patterns"].is_object());

    assert!(schema["$defs"]["insert"]["properties"]["attributes"].is_object());
    let (_, named) = agentdrafter(&["schema", "sheet-description"], &dir);
    assert_eq!(named, json);

    let (code, json) = agentdrafter(&["schema", "block-library"], &dir);
    assert_eq!(code, 0, "{json}");
    let library = &json["result"];
    assert_eq!(library["title"], "AgentDrafter block library");
    assert_eq!(library["required"], serde_json::json!(["units", "blocks"]));
    assert!(library["$defs"]["attdef"]["properties"]["tag"].is_object());

    for (kind, title) in [
        ("template", "AgentDrafter template"),
        ("set", "AgentDrafter drawing set settings"),
        ("rows", "AgentDrafter drawing set rows"),
    ] {
        let (code, json) = agentdrafter(&["schema", kind], &dir);
        assert_eq!(code, 0, "{json}");
        assert_eq!(json["result"]["title"], title);
    }

    for extra in [
        &["schema", "extra"][..],
        &["schema", "block-library", "extra"],
    ] {
        let (code, json) = agentdrafter(extra, &dir);
        assert_eq!(code, 2, "{json}");
        assert_eq!(json["error"]["code"], "usage");
    }

    let (_, json) = agentdrafter(&["--help"], &dir);
    assert_eq!(
        json["result"]["commands"]["schema"]["usage"],
        "agentdrafter schema [sheet-description | block-library | template | set | rows | change-set]"
    );
}

#[test]
fn operations_are_listed_and_a_change_set_is_applied_from_a_file() {
    let dir = scratch("change");
    let set = copy_demo_set(&dir);
    let (code, json) = agentdrafter(&["operations"], &dir);
    assert_eq!(code, 0, "{json}");
    let listed = json["result"]["operations"].as_array().unwrap();
    assert_eq!(listed.len(), 14);
    assert_eq!(listed[11]["name"], "move");
    assert_eq!(listed[11]["alias"], "MOVE");
    assert!(listed[11]["schema"]["$defs"]["point"].is_object());
    let (code, json) = agentdrafter(&["schema", "change-set"], &dir);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["title"], "AgentDrafter change set");

    let (code, json) = agentdrafter(&["build", "demo-set"], &dir);
    assert_eq!(code, 0, "{json}");
    fs::write(
        dir.join("move.json"),
        r#"{"operations": [{"op": "MOVE", "target": {"id": "DEMO-001/border"}, "by": [0.1, 0, 0]}]}"#,
    )
    .unwrap();
    let rows = fs::read(set.join("templates/ansi-b.yaml")).unwrap();
    let (code, json) = agentdrafter(&["change", "demo-set", "move.json", "--dry-run"], &dir);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["dry_run"], true);
    assert_eq!(fs::read(set.join("templates/ansi-b.yaml")).unwrap(), rows);
    let (code, json) = agentdrafter(&["change", "demo-set", "move.json"], &dir);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["landed"], 3);
    assert_ne!(fs::read(set.join("templates/ansi-b.yaml")).unwrap(), rows);

    fs::write(
        dir.join("bad.json"),
        r#"{"operations": [{"op": "add_circle", "sheet": "DEMO-001", "center": [1, 1, 0], "radius": 1}, {"op": "add_circle", "sheet": "DEMO-002", "center": [1, 1, 0], "radius": 1, "layer": "nope"}]}"#,
    )
    .unwrap();
    let (code, json) = agentdrafter(&["change", "demo-set", "bad.json"], &dir);
    assert_eq!(code, 1, "{json}");
    assert_eq!(json["error"]["code"], "sheets_failed");
    assert_eq!(
        json["result"]["inconsistent"]["failures"][0]["sheet"],
        "DEMO-002"
    );

    for args in [
        &["change", "demo-set"][..],
        &["change", "demo-set", "move.json", "--nope"],
        &["operations", "extra"],
    ] {
        let (code, json) = agentdrafter(args, &dir);
        assert_eq!(code, 2, "{json}");
        assert_eq!(json["error"]["code"], "usage");
    }
    fs::write(dir.join("not.json"), "operations: []").unwrap();
    let (code, json) = agentdrafter(&["change", "demo-set", "not.json"], &dir);
    assert_eq!(code, 1, "{json}");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("not JSON"));
}

/// A copy of the demo plant's drawing set in `dir`, as `demo-set`.
fn copy_demo_set(dir: &Path) -> PathBuf {
    let to = dir.join("demo-set");
    demo_set::copy_demo_set(&to);
    to
}

#[test]
fn build_builds_a_drawing_set_from_its_folder() {
    let dir = scratch("set");
    let set = copy_demo_set(&dir);

    let (code, json) = agentdrafter(&["build", "demo-set"], &dir);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["ok"], true);
    let result = &json["result"];
    assert_eq!(result["units"], "in");
    assert_eq!(
        (result["built"].as_u64(), result["failed"].as_u64()),
        (Some(3), Some(0))
    );
    let sheets: Vec<&str> = result["sheets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["sheet"].as_str().unwrap())
        .collect();
    assert_eq!(sheets, ["DEMO-001", "DEMO-002", "DEMO-003"]);
    for sheet in &sheets {
        assert!(set.join("dwg").join(format!("{sheet}.dwg")).is_file());
    }

    // Built again without --overwrite, every sheet fails and nothing is touched.
    let (code, json) = agentdrafter(&["build", "demo-set"], &dir);
    assert_eq!(code, 1, "{json}");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "sheets_failed");
    assert_eq!(json["result"]["failed"], 3);
    assert_eq!(
        json["result"]["sheets"][0]["error"]["code"],
        "output_exists"
    );

    let (code, json) = agentdrafter(&["build", "demo-set", "--overwrite"], &dir);
    assert_eq!(code, 0, "{json}");

    // A set's DWGs go where its settings say.
    let (code, json) = agentdrafter(&["build", "demo-set", "--output", "x.dwg"], &dir);
    assert_eq!(code, 2, "{json}");
    assert_eq!(json["error"]["code"], "usage");
}

#[test]
fn the_geometry_example_builds_every_object_type() {
    let dir = scratch("geometry");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/geometry.yaml"),
        dir.join("geometry.yaml"),
    )
    .unwrap();

    let (code, json) = agentdrafter(&["build", "geometry.yaml"], &dir);

    assert_eq!(code, 0, "{json}");
    let objects = &json["result"]["objects"];
    for object_type in ["line", "polyline", "arc", "circle", "ellipse", "spline"] {
        assert!(
            objects[object_type].as_u64().unwrap_or(0) > 0,
            "{object_type} in {objects}"
        );
    }
}

#[test]
fn an_unknown_layer_is_reported_with_the_choices() {
    let dir = scratch("unknown-layer");
    fs::write(
        dir.join("sheet.yaml"),
        "units: mm\nlayers: {outline: {}}\nobjects:\n  - circle: {center: [0, 0, 0], radius: 5, layer: hidden}\n",
    )
    .unwrap();

    let (code, json) = agentdrafter(&["build", "sheet.yaml"], &dir);

    assert_eq!(code, 1, "{json}");
    let error = &json["error"];
    assert_eq!(error["code"], "unknown_name");
    assert_eq!(error["field"], "objects[0].circle.layer");
    assert_eq!(
        error["message"],
        "unknown layer `hidden`; the sheet's layers: 0, outline"
    );
    assert!(!dir.join("sheet.dwg").exists());
}

#[test]
fn the_hatch_example_builds_solid_and_patterned_hatches() {
    let dir = scratch("hatch");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/hatch.yaml"),
        dir.join("hatch.yaml"),
    )
    .unwrap();

    let (code, json) = agentdrafter(&["build", "hatch.yaml"], &dir);

    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["objects"]["hatch"], 8, "{json}");
    assert!(dir.join("hatch.dwg").exists());
}

#[test]
fn an_unknown_hatch_pattern_is_reported_with_the_choices() {
    let dir = scratch("unknown-pattern");
    fs::write(
        dir.join("sheet.yaml"),
        "units: mm\nhatch_patterns: {GRID: {lines: [{delta: [0, 5]}]}}\nobjects:\n  - hatch: {pattern: ANSI31, boundary: [{polyline: {points: [[0, 0, 0], [1, 0, 0], [1, 1, 0]]}}]}\n",
    )
    .unwrap();

    let (code, json) = agentdrafter(&["build", "sheet.yaml"], &dir);

    assert_eq!(code, 1, "{json}");
    let error = &json["error"];
    assert_eq!(error["code"], "unknown_name");
    assert_eq!(error["field"], "objects[0].hatch.pattern");
    assert_eq!(
        error["message"],
        "unknown hatch pattern `ANSI31`; the sheet's hatch patterns: SOLID, GRID"
    );
    assert!(!dir.join("sheet.dwg").exists());
}

#[test]
fn templates_are_grouped_by_the_file_their_path_names() {
    // Two rows whose templates are two files, `../t.yaml` and
    // `../../../t.yaml`, with the set named from the folder it is in. A
    // change to one template reaches only the sheet built from it.
    let root = scratch("template-files");
    let cwd = root.join("a").join("b");
    let set = cwd.join("set");
    fs::create_dir_all(&set).unwrap();
    let template = "units: mm\nobjects:\n  - circle: {center: [0, 0, 0], radius: 1, id: c}\n";
    fs::write(cwd.join("t.yaml"), template).unwrap();
    fs::write(root.join("t.yaml"), template).unwrap();
    fs::write(
        set.join("set.yaml"),
        "units: mm\noutput: dwg\nrows: rows.yaml\n",
    )
    .unwrap();
    fs::write(
        set.join("rows.yaml"),
        "rows:\n  - sheet: S-1\n    template: ../t.yaml\n  - sheet: S-2\n    template: ../../../t.yaml\n",
    )
    .unwrap();
    fs::write(
        cwd.join("change.json"),
        r#"{"operations": [{"op": "change", "target": {"id": "S-2/c"}, "set": {"radius": 2}}]}"#,
    )
    .unwrap();
    let (code, json) = agentdrafter(&["change", "set", "change.json"], &cwd);
    assert_eq!(code, 0, "{json}");
    assert_eq!(
        json["result"]["operations"][0]["sheets"],
        serde_json::json!(["S-2"])
    );
    assert_eq!(fs::read_to_string(cwd.join("t.yaml")).unwrap(), template);
    assert!(fs::read_to_string(root.join("t.yaml"))
        .unwrap()
        .contains("radius: 2"));
}
