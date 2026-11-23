//! The journal and undo through the command: a change set applied by one
//! process is undone by another, from the journal in the set's folder, and
//! every file comes back byte for byte, DWGs included; and none of it needs
//! git, or the set's folder to be a repository. A process killed mid-commit
//! leaves its entry pending, and the next command rolls it back.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value};

#[path = "../../agentdrafter-core/tests/support/demo_set.rs"]
mod demo_set;
#[path = "../../agentdrafter-core/tests/support/snapshot.rs"]
mod snapshot;

use snapshot::snapshot;

/// Runs `agentdrafter` in `cwd` as a process of its own, with `PATH` set to
/// `path` when given, and returns its exit code and the JSON it printed.
fn agentdrafter(args: &[&str], cwd: &Path, path: Option<&Path>) -> (i32, Value) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_agentdrafter"));
    command.args(args).current_dir(cwd);
    if let Some(path) = path {
        command.env("PATH", path);
    }
    let out = command.output().unwrap();
    assert!(
        out.stderr.is_empty(),
        "nothing on standard error: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1, "one line of JSON: {stdout}");
    (
        out.status.code().unwrap(),
        serde_json::from_str(&stdout).unwrap(),
    )
}

const MOVE: &str =
    r#"{"operations": [{"op": "move", "target": {"id": "DEMO-001/border"}, "by": [0.1, 0, 0]}]}"#;
const PARTIAL: &str = r#"{"operations": [{"op": "add_circle", "sheet": "DEMO-001", "center": [5, 5, 0], "radius": 0.5}, {"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": 0.5, "layer": "nope"}]}"#;

/// Builds the demo set in `dir`, applies a template edit and a change set
/// that marks the set inconsistent, each in a process of its own, then
/// undoes both from a new process and checks every file is as it was.
fn change_then_undo_in_new_processes(dir: &Path, path: Option<&Path>) {
    let set = dir.join("demo-set");
    demo_set::copy_demo_set(&set);
    fs::write(dir.join("move.json"), MOVE).unwrap();
    fs::write(dir.join("partial.json"), PARTIAL).unwrap();
    let (code, json) = agentdrafter(&["build", "demo-set"], dir, path);
    assert_eq!(code, 0, "{json}");
    let start = snapshot(&set);
    let dwgs: Vec<Vec<u8>> = ["DEMO-001", "DEMO-002", "DEMO-003"]
        .iter()
        .map(|s| fs::read(set.join(format!("dwg/{s}.dwg"))).unwrap())
        .collect();

    let (code, json) = agentdrafter(&["change", "demo-set", "move.json"], dir, path);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["journal_entry"], 1);
    let (code, json) = agentdrafter(&["change", "demo-set", "partial.json"], dir, path);
    assert_eq!(code, 1, "{json}");
    assert_eq!(json["result"]["journal_entry"], 2);
    assert!(set.join(".agentdrafter/inconsistent.yaml").is_file());
    for (sheet, before) in ["DEMO-001", "DEMO-002", "DEMO-003"].iter().zip(&dwgs) {
        assert_ne!(
            &fs::read(set.join(format!("dwg/{sheet}.dwg"))).unwrap(),
            before
        );
    }

    let (code, json) = agentdrafter(&["journal", "demo-set"], dir, path);
    assert_eq!(code, 0, "{json}");
    let entries = json["result"]["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["kind"], "change_set");
    assert_eq!(entries[0]["change_set"]["operations"][0]["op"], "move");
    assert_eq!(entries[1]["failed_on"][0]["sheet"], "DEMO-002");
    assert_eq!(json["result"]["can_undo"], serde_json::json!([1, 2]));

    // A dry run from another process writes nothing.
    let journal = fs::read(set.join(".agentdrafter/journal.jsonl")).unwrap();
    let marked = snapshot(&set);
    let (code, json) = agentdrafter(&["undo", "demo-set", "1", "--dry-run"], dir, path);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["dry_run"], true);
    assert_eq!(snapshot(&set), marked);
    assert_eq!(
        fs::read(set.join(".agentdrafter/journal.jsonl")).unwrap(),
        journal
    );

    // Undoing the first undoes the second with it.
    let (code, json) = agentdrafter(&["undo", "demo-set", "1"], dir, path);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["undid"], serde_json::json!([2, 1]));
    assert_eq!(json["result"]["journal_entry"], 3);
    assert!(json["result"].get("inconsistent").is_none());
    assert_eq!(snapshot(&set), start, "every file as it was, byte for byte");
    for (sheet, before) in ["DEMO-001", "DEMO-002", "DEMO-003"].iter().zip(&dwgs) {
        assert_eq!(
            &fs::read(set.join(format!("dwg/{sheet}.dwg"))).unwrap(),
            before,
            "{sheet}'s DWG"
        );
    }
    assert!(!set.join(".agentdrafter/inconsistent.yaml").exists());

    let (code, json) = agentdrafter(&["journal", "demo-set"], dir, path);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["entries"][0]["undo_state"], "undone");
    assert_eq!(json["result"]["entries"][0]["undone_by"], 3);
    assert_eq!(json["result"]["entries"][2]["kind"], "undo");

    // An undone change set is not undone again: the reply says so.
    let (code, json) = agentdrafter(&["undo", "demo-set", "2"], dir, path);
    assert_eq!(code, 1, "{json}");
    assert_eq!(json["error"]["code"], "bad_value");
}

#[test]
fn a_new_process_undoes_change_sets_and_every_file_comes_back_byte_for_byte() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("cli-journal")
        .join("new-process");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    change_then_undo_in_new_processes(&dir, None);
}

#[test]
fn nothing_depends_on_the_sets_folder_being_a_git_repository() {
    // A folder in the system's temporary folder, outside this checkout and
    // any other repository, and a PATH that reaches no git.
    let dir = std::env::temp_dir().join(format!("agentdrafter-no-git-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for folder in dir.ancestors() {
        assert!(
            !folder.join(".git").exists(),
            "{} is in a git repository; the test needs a temporary folder outside one",
            dir.display()
        );
    }
    let no_programs = dir.join("no-programs");
    fs::create_dir(&no_programs).unwrap();
    change_then_undo_in_new_processes(&dir, Some(&no_programs));
    assert!(!dir.join("demo-set/.git").exists());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_change_whose_process_is_killed_mid_commit_is_rolled_back_by_the_next_command() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("cli-journal")
        .join("killed");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let set = dir.join("demo-set");
    demo_set::copy_demo_set(&set);
    fs::write(dir.join("move.json"), MOVE).unwrap();
    let (code, json) = agentdrafter(&["build", "demo-set"], &dir, None);
    assert_eq!(code, 0, "{json}");
    let start = snapshot(&set);
    let template = set.join("templates/ansi-b.yaml");
    let dwg = |sheet: &str| set.join(format!("dwg/{sheet}.dwg"));

    // The test-only seam stops the commit at step 16, once the template and
    // DEMO-001's DWG are in place and before the other DWGs are, and the
    // process waits there; then it is killed.
    let mut child = Command::new(env!("CARGO_BIN_EXE_agentdrafter"))
        .args(["change", "demo-set", "move.json"])
        .current_dir(&dir)
        .env("AGENTDRAFTER_TEST_STOP_COMMIT_AT", "16")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    let (said, heard) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = said.send(line);
        }
    });
    let line = heard
        .recv_timeout(Duration::from_secs(120))
        .expect("the command stops at the seam");
    assert!(
        line.starts_with("agentdrafter: stopped the commit at step 16, once "),
        "{line}"
    );
    child.kill().unwrap();
    child.wait().unwrap();

    // Killed mid-commit: the template is new, the DWGs disagree with it,
    // and the journal holds the change set's entry pending.
    assert_ne!(
        fs::read(&template).unwrap(),
        start[&template].clone().unwrap()
    );
    assert_ne!(
        fs::read(dwg("DEMO-001")).unwrap(),
        start[&dwg("DEMO-001")].clone().unwrap()
    );
    assert_eq!(
        fs::read(dwg("DEMO-002")).unwrap(),
        start[&dwg("DEMO-002")].clone().unwrap()
    );
    let text = fs::read_to_string(set.join(".agentdrafter/journal.jsonl")).unwrap();
    assert_eq!(text.lines().count(), 1);
    let pending: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(pending["entry"], 1);
    assert_eq!(pending["state"], "pending");

    // The next command, from a new process, puts every file back byte for
    // byte, says so, and the journal marks the entry rolled back.
    let (code, json) = agentdrafter(&["journal", "demo-set"], &dir, None);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["rolled_back"]["entry"], 1);
    assert_eq!(json["rolled_back"]["kind"], "change_set");
    // Named as a reply names a file of the set, from the folder given.
    let files: Vec<(String, &Value)> = json["rolled_back"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            let file = Path::new(f["file"].as_str().unwrap());
            let name = file.file_name().unwrap().to_str().unwrap().to_owned();
            assert!(file.starts_with("demo-set"), "{}", file.display());
            (name, &f["action"])
        })
        .collect();
    assert_eq!(
        files,
        [
            ("ansi-b.yaml".to_owned(), &json!("restored")),
            ("DEMO-001.dwg".to_owned(), &json!("restored"))
        ]
    );
    assert!(
        snapshot(&set) == start,
        "every file as it was, byte for byte"
    );
    assert_eq!(json["result"]["entries"][0]["state"], "rolled_back");
    assert_eq!(json["result"]["can_undo"], serde_json::json!([]));
    let text = fs::read_to_string(set.join(".agentdrafter/journal.jsonl")).unwrap();
    let marked: Value = serde_json::from_str(text.lines().nth(1).unwrap()).unwrap();
    assert_eq!(marked["entry"], 1);
    assert_eq!(marked["state"], "rolled_back");

    // A further change set lands normally.
    let (code, json) = agentdrafter(&["change", "demo-set", "move.json"], &dir, None);
    assert_eq!(code, 0, "{json}");
    assert!(json.get("rolled_back").is_none());
    assert_eq!(json["result"]["journal_entry"], 2);
    let (code, json) = agentdrafter(&["journal", "demo-set"], &dir, None);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["entries"][1]["state"], "done");
    assert_eq!(json["result"]["can_undo"], serde_json::json!([2]));
}

#[test]
fn journal_and_undo_take_their_arguments_or_say_how() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("cli-journal")
        .join("usage");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    demo_set::copy_demo_set(&dir.join("demo-set"));
    for args in [
        &["journal"][..],
        &["journal", "demo-set", "extra"],
        &["journal", "demo-set", "--dry-run"],
        &["undo", "demo-set"],
        &["undo", "demo-set", "first"],
        &["undo", "demo-set", "1", "--force"],
    ] {
        let (code, json) = agentdrafter(args, &dir, None);
        assert_eq!(code, 2, "{args:?}: {json}");
        assert_eq!(json["error"]["code"], "usage");
    }
    let (code, json) = agentdrafter(&["journal", "demo-set"], &dir, None);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["entries"], serde_json::json!([]));
    let (code, json) = agentdrafter(&["undo", "demo-set", "1"], &dir, None);
    assert_eq!(code, 1, "{json}");
    assert_eq!(json["error"]["code"], "unknown_name");
    let (code, json) = agentdrafter(&["help"], &dir, None);
    assert_eq!(code, 0, "{json}");
    assert!(json["result"]["commands"]["journal"]["usage"].is_string());
    assert!(json["result"]["commands"]["undo"]["usage"].is_string());
}

#[test]
fn original_relative_output_works_across_different_working_directories() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("cli-journal")
        .join("original-relative-output");
    let _ = fs::remove_dir_all(&dir);
    let drawings = dir.join("drawings");
    let run_a = dir.join("run_a");
    let run_b = dir.join("run_b");
    let out_dir = run_a.join("out");
    fs::create_dir_all(&drawings).unwrap();
    fs::create_dir_all(&out_dir).unwrap();
    fs::create_dir_all(&run_b).unwrap();

    // Build a made-up original DWG inside `drawings/A.dwg`.
    fs::write(
        drawings.join("A.yaml"),
        "units: mm\npaper: [100, 100]\nobjects:\n  - line: {id: base, from: [5, 5, 0], to: [95, 5, 0]}\n",
    )
    .unwrap();
    let (code, json) = agentdrafter(&["build", "A.yaml"], &drawings, None);
    assert_eq!(code, 0, "{json}");
    let orig_bytes = fs::read(drawings.join("A.dwg")).unwrap();

    let edit = r#"{"operations": [{"op": "add_line", "from": [10, 20, 0], "to": [50, 20, 0]}]}"#;
    fs::write(run_a.join("edits.json"), edit).unwrap();

    // 1. From `run_a`, revise `../drawings/A.dwg` with relative `--output out/R1.dwg`!
    let (code, json) = agentdrafter(
        &[
            "change",
            "../drawings/A.dwg",
            "edits.json",
            "--output",
            "out/R1.dwg",
            "--no-pdf",
        ],
        &run_a,
        None,
    );
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["journal_entry"], 1);
    assert!(out_dir.join("R1.dwg").is_file());
    assert!(out_dir.join("R1.changes.json").is_file());

    // 2. From a COMPLETELY DIFFERENT working directory (`run_b`), run `journal`
    //    and `undo` on `../drawings/A.dwg` — must find and remove `run_a/out/R1.*`!
    let (code, json) = agentdrafter(&["journal", "../drawings/A.dwg"], &run_b, None);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["can_undo"], json!([1]));

    let (code, json) = agentdrafter(&["undo", "../drawings/A.dwg", "1"], &run_b, None);
    assert_eq!(code, 0, "{json}");
    assert!(!out_dir.join("R1.dwg").exists(), "R1.dwg must be removed");
    assert!(
        !out_dir.join("R1.changes.json").exists(),
        "R1.changes.json must be removed"
    );
    assert_eq!(fs::read(drawings.join("A.dwg")).unwrap(), orig_bytes);

    // 3. Revise again from `run_a` to `--output out/R2.dwg`, accept from `run_b`,
    //    and render `R2.dwg` from `run_b` -> stamp must say `issued: true`!
    let (code, json) = agentdrafter(
        &[
            "change",
            "../drawings/A.dwg",
            "edits.json",
            "--output",
            "out/R2.dwg",
            "--no-pdf",
        ],
        &run_a,
        None,
    );
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["journal_entry"], 3);

    let (code, json) = agentdrafter(
        &["accept", "../drawings/A.dwg", "--newest-seen", "3"],
        &run_b,
        None,
    );
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["result"]["journal_entry"], 4);

    let (code, json) = agentdrafter(
        &[
            "render",
            "../run_a/out/R2.dwg",
            "--output",
            "R2.png",
            "--no-system-fonts",
        ],
        &run_b,
        None,
    );
    assert_eq!(code, 0, "{json}");
    assert_eq!(
        json["result"]["stamp"]["revision"]["issued"], true,
        "{json}"
    );
    assert_eq!(json["result"]["stamp"]["revision"]["accept"], 4);

    // 4. Crash recovery across working directories: kill `change` mid-commit in
    //    `run_a` with `--output out/R3.dwg`, then recover from `run_b`!
    let mut child = Command::new(env!("CARGO_BIN_EXE_agentdrafter"))
        .args([
            "change",
            "../drawings/A.dwg",
            "edits.json",
            "--output",
            "out/R3.dwg",
            "--no-pdf",
        ])
        .current_dir(&run_a)
        .env("AGENTDRAFTER_TEST_STOP_COMMIT_AT", "3")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    let (said, heard) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = said.send(line);
        }
    });
    let line = heard
        .recv_timeout(Duration::from_secs(120))
        .expect("original change stops at seam");
    assert!(
        line.starts_with("agentdrafter: stopped the commit at step 3, once "),
        "{line}"
    );
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(out_dir.join("R3.dwg").is_file());

    let (code, json) = agentdrafter(&["journal", "../drawings/A.dwg"], &run_b, None);
    assert_eq!(code, 0, "{json}");
    assert_eq!(json["rolled_back"]["entry"], 5);
    assert!(
        !out_dir.join("R3.dwg").exists(),
        "partial R3.dwg must be cleaned up by recovery from another CWD"
    );
    assert_eq!(fs::read(drawings.join("A.dwg")).unwrap(), orig_bytes);

    let _ = fs::remove_dir_all(&dir);
}
