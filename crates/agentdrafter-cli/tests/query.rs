//! `agentdrafter query` asks the demo set's records and prints the core's
//! answer as one line of JSON: a query that matches nothing is ok and says
//! so, and a distance to an object nobody has is an error.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

#[path = "../../agentdrafter-core/tests/support/demo_set.rs"]
mod demo_set;

/// A copy of the demo set in an empty folder of its own, under Cargo's
/// scratch folder in `target/`.
fn demo_set(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("cli-query")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    demo_set::copy_demo_set(&dir.join("demo-set"));
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
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout.lines().count(), 1, "one line of JSON: {stdout}");
    (
        out.status.code().unwrap(),
        serde_json::from_str(&stdout).unwrap(),
    )
}

#[test]
fn query_finds_reads_attributes_and_measures_as_json() {
    let dir = demo_set("asks");

    let (code, json) = agentdrafter(
        &["query", "find", "demo-set", "--text", "notes", "--contains"],
        &dir,
    );
    assert_eq!(code, 0, "{json}");
    let result = &json["result"];
    assert_eq!(result["kind"], "find");
    assert_eq!(result["units"], "in");
    assert_eq!(result["count"], 3);
    assert_eq!(result["objects"][0]["id"], "DEMO-001/notes");
    assert_eq!(result["objects"][0]["object"]["type"], "mtext");

    let (code, json) = agentdrafter(
        &[
            "query",
            "attributes",
            "demo-set",
            "--sheet",
            "DEMO-003",
            "--name",
            "title-block",
        ],
        &dir,
    );
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["inserts"][0]["attributes"]["REV"], "A");

    let (code, json) = agentdrafter(
        &[
            "query",
            "measure",
            "demo-set",
            "--from",
            "DEMO-002/border",
            "--from-point",
            "points[0]",
            "--to",
            "DEMO-002/border",
            "--to-point",
            "points[1]",
        ],
        &dir,
    );
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["distance"], 16.0);
    assert_eq!(json["result"]["delta"], json!([16.0, 0.0, 0.0]));
}

#[test]
fn query_that_matches_nothing_is_ok_and_says_so() {
    let dir = demo_set("nothing");
    let (code, json) = agentdrafter(&["query", "find", "demo-set", "--block", "PUMP"], &dir);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["ok"], true);
    assert_eq!(json["result"]["count"], 0);
    assert_eq!(
        json["result"]["message"],
        "nothing matches: no object on the 3 sheets looked at has an insert of block `PUMP`"
    );
}

#[test]
fn query_measuring_to_an_object_nobody_has_matches_nothing() {
    let dir = demo_set("no-such");
    let (code, json) = agentdrafter(
        &[
            "query",
            "measure",
            "demo-set",
            "--from",
            "DEMO-001/nope",
            "--to",
            "DEMO-001/border",
        ],
        &dir,
    );
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["ok"], true);
    assert!(json["result"].get("distance").is_none(), "{json}");
    assert!(
        json["result"]["message"].as_str().unwrap().starts_with(
            "nothing matches: no object on the 3 sheets looked at has the ID `DEMO-001/nope`"
        ),
        "{json}"
    );

    // A request that cannot be answered as asked is still wrong.

    let (code, json) = agentdrafter(&["query", "blocks", "demo-set", "--id", "x"], &dir);
    assert_eq!(code, 2, "{json}");
    assert_eq!(json["error"]["code"], "usage");
}

#[test]
fn query_snaps_to_an_object_and_gives_its_geometry() {
    let dir = demo_set("snaps");
    let (code, json) = agentdrafter(
        &[
            "query",
            "snap",
            "demo-set",
            "--id",
            "DEMO-001/border",
            "--snap",
            "endpoint",
            "--which",
            "points[2]",
        ],
        &dir,
    );
    assert_eq!(code, 0, "{json}");
    assert_eq!(
        json["result"]["points"],
        json!([{"snap": "endpoint", "which": "points[2]", "at": [16.5, 10.5, 0.0]}])
    );

    // The nearest point of the border to the title block's insertion point,
    // which sits on its corner.
    let (code, json) = agentdrafter(
        &[
            "query",
            "snap",
            "demo-set",
            "--id",
            "DEMO-001/border",
            "--snap",
            "nearest",
            "--from",
            "DEMO-001/objects[2]",
            "--from-snap",
            "insertion",
            "--from-which",
            "position",
        ],
        &dir,
    );
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["from"]["at"], json!([16.5, 0.5, 0.0]));
    assert_eq!(json["result"]["points"][0]["at"], json!([16.5, 0.5, 0.0]));

    let (code, json) = agentdrafter(
        &["query", "geometry", "demo-set", "--id", "DEMO-002/border"],
        &dir,
    );
    assert_eq!(code, 0, "{json}");
    let border = &json["result"]["objects"][0];
    assert_eq!(border["length"], 52.0);
    assert_eq!(border["area"], 160.0);

    // A snap AutoCAD has not is the command called wrongly.
    let (code, json) = agentdrafter(
        &[
            "query",
            "snap",
            "demo-set",
            "--id",
            "DEMO-001/border",
            "--snap",
            "middle",
        ],
        &dir,
    );
    assert_eq!(code, 2, "{json}");
    assert_eq!(json["error"]["code"], "usage");
}

#[test]
fn query_snaps_from_where_two_other_objects_cross() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("cli-query")
        .join("from-with");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("cross.yaml"),
        "units: mm\nobjects:\n  - line: {id: a, from: [0, 0, 0], to: [10, 10, 0]}\n  - line: {id: b, from: [0, 10, 0], to: [10, 0, 0]}\n  - line: {id: c, from: [0, -5, 0], to: [10, -5, 0]}\n",
    )
    .unwrap();
    let (code, json) = agentdrafter(
        &[
            "query",
            "snap",
            "cross.yaml",
            "--id",
            "cross/c",
            "--snap",
            "nearest",
            "--from",
            "cross/a",
            "--from-snap",
            "intersection",
            "--from-with",
            "cross/b",
        ],
        &dir,
    );
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["from"]["at"], json!([5.0, 5.0, 0.0]));
    assert_eq!(json["result"]["points"][0]["at"], json!([5.0, -5.0, 0.0]));
}
