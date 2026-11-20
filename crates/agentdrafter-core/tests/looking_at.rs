//! What the drafter is looking at (#156): the viewer's page records the
//! sheet open and the objects selected with `look-at`, and an agent reads
//! them with `looking-at`. It is kept beside the journal and is not one of
//! the set's records: recording it changes no record, no journal entry and
//! no undo state. The drafter's assistant gets only the commands that read
//! or propose.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    assistant_commands, commands, find_command, look_at, looking_at, Access, Interface,
    LookAtRequest, LookingAtRequest, Settings, JOURNAL_FILE, LOOKING_AT_FILE,
};
use serde_json::{json, Value};

#[path = "support/demo_set.rs"]
mod demo_set;
#[path = "support/snapshot.rs"]
mod snapshot;

/// A copy of the demo set in a folder of its own.
fn demo_set(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-looking-at")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    let set = dir.join("demo-set");
    demo_set::copy_demo_set(&set);
    set
}

fn look(set: &Path, sheet: &str, layout: Option<&str>, selected: &[&str]) -> Value {
    let reply = look_at(&LookAtRequest {
        set: set.to_owned(),
        sheet: sheet.to_owned(),
        layout: layout.map(str::to_owned),
        selected: selected.iter().map(|s| (*s).to_owned()).collect(),
    });
    serde_json::to_value(reply).unwrap()
}

fn asked(set: &Path) -> Value {
    serde_json::to_value(looking_at(&LookingAtRequest {
        set: set.to_owned(),
    }))
    .unwrap()
}

#[test]
fn before_the_viewer_records_anything_no_sheet_is_open() {
    let set = demo_set("nothing");
    let reply = asked(&set);
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["result"]["sheet"], Value::Null);
    assert_eq!(reply["result"]["selected"], json!([]));
    assert_eq!(reply["result"]["at"], Value::Null);
    assert!(!set.join(LOOKING_AT_FILE).exists(), "asking writes nothing");
}

#[test]
fn what_the_page_records_an_agent_reads_and_no_record_changes() {
    let set = demo_set("recorded");
    let before = snapshot::snapshot(&set);
    // The sheet in any case is the set's own name; a selection names an
    // object once.
    let reply = look(
        &set,
        "demo-001",
        None,
        &["DEMO-001/border", "demo-001/BORDER", "DEMO-001/title-block"],
    );
    assert_eq!(reply["ok"], true, "{reply}");
    let result = &reply["result"];
    assert_eq!(result["sheet"], "DEMO-001");
    assert_eq!(
        result["selected"],
        json!(["DEMO-001/border", "DEMO-001/title-block"])
    );
    assert!(result["at"].as_str().unwrap().ends_with('Z'));
    assert_eq!(
        result["message"],
        "the drafter has sheet DEMO-001 open, with 2 objects selected"
    );

    let read = asked(&set);
    assert_eq!(
        read["result"], *result,
        "looking-at gives what look-at recorded"
    );

    // Another sheet, a layout and one object replace it.
    let reply = look(&set, "DEMO-002", Some("Wiring"), &["DEMO-002/border"]);
    assert_eq!(reply["ok"], true, "{reply}");
    let read = asked(&set)["result"].clone();
    assert_eq!(read["sheet"], "DEMO-002");
    assert_eq!(read["layout"], "Wiring");
    assert_eq!(read["selected"], json!(["DEMO-002/border"]));
    assert_eq!(
        read["message"],
        "the drafter has sheet DEMO-002, layout Wiring open, with DEMO-002/border selected"
    );

    // Nothing selected.
    let reply = look(&set, "DEMO-002", None, &[]);
    assert_eq!(
        reply["result"]["message"],
        "the drafter has sheet DEMO-002 open, with nothing selected"
    );

    assert_eq!(
        snapshot::snapshot(&set),
        before,
        "no record of the set changed"
    );
    assert!(
        !set.join(JOURNAL_FILE).exists(),
        "the journal never sees it"
    );
}

#[test]
fn a_sheet_the_set_has_not_got_and_a_folder_that_is_no_set_are_refused() {
    let set = demo_set("refused");
    let reply = look(&set, "NO-SUCH-SHEET", None, &[]);
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["error"]["code"], "unknown_name", "{reply}");
    assert_eq!(reply["error"]["field"], "sheet");
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("DEMO-001"),
        "it names the sheets there are: {reply}"
    );
    assert!(!set.join(LOOKING_AT_FILE).exists(), "nothing written");

    let not_a_set = set.join("templates");
    for reply in [look(&not_a_set, "DEMO-001", None, &[]), asked(&not_a_set)] {
        assert_eq!(reply["error"]["code"], "read_failed", "{reply}");
    }

    let reply = look(&set, "DEMO-001", None, &[" "]);
    assert_eq!(reply["error"]["code"], "usage", "{reply}");
    assert_eq!(reply["error"]["field"], "selected");
}

#[test]
fn a_record_that_cannot_be_read_says_so() {
    let set = demo_set("unreadable");
    let path = set.join(LOOKING_AT_FILE);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "not JSON").unwrap();
    let reply = asked(&set);
    assert_eq!(reply["error"]["code"], "read_failed", "{reply}");
}

#[test]
fn the_commands_reach_it_through_the_registry_from_any_interface() {
    let set = demo_set("registry");
    let look_at = find_command("look-at").unwrap();
    let reply = look_at.run(
        json!({"set": set, "sheet": "DEMO-001", "selected": ["DEMO-001/border"]}),
        &Settings::default(),
        Interface::Viewer,
    );
    assert!(reply.ok, "{:?}", reply.error);
    for via in [
        Interface::Cli,
        Interface::Http,
        Interface::Mcp,
        Interface::Python,
    ] {
        let reply =
            find_command("looking-at")
                .unwrap()
                .run(json!({"set": set}), &Settings::default(), via);
        let result: Value = serde_json::from_str(reply.result.unwrap().get()).unwrap();
        assert_eq!(result["sheet"], "DEMO-001", "{via:?}");
        assert_eq!(result["selected"], json!(["DEMO-001/border"]), "{via:?}");
    }
}

#[test]
fn the_assistant_gets_the_commands_that_read_and_propose_and_none_that_write() {
    let given: Vec<&str> = assistant_commands().map(|c| c.name()).collect();
    for name in [
        "query",
        "looking-at",
        "propose",
        "proposals",
        "journal",
        "check",
    ] {
        assert!(given.contains(&name), "the assistant gets `{name}`");
    }
    for name in [
        "change",
        "undo",
        "accept",
        "keep",
        "discard",
        "build",
        "plot",
        "render",
        "import-blocks",
        "look-at",
    ] {
        assert!(!given.contains(&name), "the assistant never gets `{name}`");
        assert_eq!(find_command(name).unwrap().access(), Access::Writes);
    }
    assert_eq!(find_command("propose").unwrap().access(), Access::Proposes);
    assert_eq!(
        given.len(),
        commands()
            .iter()
            .filter(|c| c.access() != Access::Writes)
            .count()
    );
}
