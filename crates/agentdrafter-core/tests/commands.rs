//! The registry of commands: every interface takes its commands from it, so
//! each command's input schema must say exactly what the command takes, and
//! what a command makes of its input is decided here, once.

use agentdrafter_core::{
    block_library_schema, change_set_schema, commands, find_command, operations, rows_schema,
    set_schema, sheet_description_schema, template_schema, ErrorCode, Interface, JsonReply,
    Settings,
};
use serde_json::{json, Value};

#[path = "support/demo_set.rs"]
mod demo_set;
#[path = "support/snapshot.rs"]
mod snapshot;

/// A reply as plain JSON.
fn plain(reply: &JsonReply) -> Value {
    serde_json::to_value(reply).unwrap()
}

fn run(command: &str, input: Value) -> Value {
    plain(
        &find_command(command)
            .unwrap()
            .run(input, &Settings::default(), Interface::Cli),
    )
}

/// For each command, an input its schema accepts, naming files that are not
/// there, so it is read whole and then fails on the file, not on its shape.
fn samples() -> Vec<(&'static str, Value)> {
    vec![
        (
            "build",
            json!({"source": "missing.yaml", "output": "x.dwg", "overwrite": true, "fonts": ["f"]}),
        ),
        ("schema", json!({"kind": "change-set"})),
        (
            "measure-text",
            json!({"contents": "PUMP 101", "font": "missing.ttf", "height": 2.5, "width_factor": 0.8, "fonts": []}),
        ),
        (
            "render",
            json!({"source": "missing.yaml", "output": "x.png", "overwrite": false, "dpi": 96, "width": 320, "height": 200, "margin": 2, "fonts": ["f"]}),
        ),
        (
            "display-list",
            json!({"source": "missing", "sheet": "DEMO-001", "tolerance": 0.01, "entry": 2, "fonts": ["f"]}),
        ),
        (
            "plot",
            json!({"source": "missing.yaml", "output": "x.pdf", "plot_style": "monochrome.ctb", "plot_styles": ["p"], "fonts": ["f"]}),
        ),
        (
            "stale",
            json!({"output": "missing.png", "source": "missing.yaml"}),
        ),
        (
            "query",
            json!({"source": "missing", "sheet": "DEMO-001", "query": {"kind": "measure", "from": {"id": "DEMO-001/a"}, "to": {"name": "b", "point": "center"}}, "fonts": ["f"]}),
        ),
        ("operations", json!({})),
        (
            "change",
            json!({"set": "missing", "change_set": {"operations": [{"op": "move", "target": {"id": "DEMO-001/border"}, "by": [1, 0, 0]}], "fixes": ["DEMO-002"]}, "dry_run": true, "fonts": ["f"]}),
        ),
        ("journal", json!({"set": "missing"})),
        (
            "undo",
            json!({"set": "missing", "change_set": 3, "dry_run": true}),
        ),
        (
            "accept",
            json!({"set": "missing", "despite_failed_checks": true, "newest_seen": 4, "dry_run": true, "fonts": ["f"]}),
        ),
        (
            "check",
            json!({"source": "missing", "sheets": ["DEMO-001"], "fonts": ["f"]}),
        ),
    ]
}

#[test]
fn every_command_has_one_name_and_an_object_schema_for_its_input() {
    let names: Vec<&str> = commands().iter().map(|c| c.name()).collect();
    let samples = samples();
    assert_eq!(
        names,
        samples.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
        "a sample input for every command, in the registry's order"
    );
    for command in commands() {
        assert_eq!(find_command(command.name()).unwrap().name(), command.name());
        let schema = command.input_schema();
        assert_eq!(schema["type"], "object", "{}", command.name());
        assert_eq!(schema["additionalProperties"], false, "{}", command.name());
        jsonschema::validator_for(&schema).expect("a valid JSON Schema");
        assert!(command.does().len() > 40, "{}", command.name());
        let info = command.info();
        assert_eq!(
            (info.name.as_str(), info.input_schema),
            (command.name(), schema)
        );
    }
    assert!(find_command("draw").is_none());
    assert!(find_command("Build").is_none(), "names are matched whole");
}

#[test]
fn each_schema_accepts_what_its_command_reads_and_refuses_what_it_refuses() {
    for (name, sample) in samples() {
        let command = find_command(name).unwrap();
        let validator = jsonschema::validator_for(&command.input_schema()).unwrap();
        assert!(
            validator.is_valid(&sample),
            "{name}'s schema takes {sample}"
        );
        let reply = run(name, sample.clone());
        assert_ne!(
            reply["error"]["code"], "usage",
            "`{name}` reads {sample} whole: {reply}"
        );

        // A key the command does not know is refused by both.
        let mut extra = sample.clone();
        extra["colour"] = json!(1);
        assert!(!validator.is_valid(&extra), "{name}");
        let reply = run(name, extra);
        assert_eq!(reply["ok"], false);
        assert_eq!(reply["error"]["code"], "usage", "{name}: {reply}");
        assert!(reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("colour"));

        // So is a required key left out, for a command that has one.
        let required = command.input_schema()["required"].clone();
        for key in required.as_array().unwrap() {
            let mut missing = sample.clone();
            missing
                .as_object_mut()
                .unwrap()
                .remove(key.as_str().unwrap());
            assert!(!validator.is_valid(&missing), "{name} without {key}");
            assert_eq!(
                run(name, missing)["error"]["code"],
                "usage",
                "{name} without {key}"
            );
        }
    }
}

#[test]
fn a_key_the_schema_does_not_name_is_refused_in_every_kind_of_query() {
    let validator =
        jsonschema::validator_for(&find_command("query").unwrap().input_schema()).unwrap();
    let end = json!({"id": "DEMO-001/border"});
    for (query, key) in [
        (json!({"kind": "find", "name": "title-block"}), "colour"),
        (json!({"kind": "blocks"}), "name"),
        (
            json!({"kind": "attributes", "block": "TITLE_BLOCK"}),
            "colour",
        ),
        (json!({"kind": "measure", "from": end, "to": end}), "colour"),
        (json!({"kind": "extents"}), "typo"),
    ] {
        let taken = json!({"source": "missing", "query": query});
        assert!(validator.is_valid(&taken), "{taken}");
        assert_ne!(
            run("query", taken.clone())["error"]["code"],
            "usage",
            "{taken}"
        );

        let mut refused = taken.clone();
        refused["query"][key] = json!("x");
        assert!(!validator.is_valid(&refused), "{refused}");
        let reply = run("query", refused.clone());
        assert_eq!(reply["error"]["code"], "usage", "{refused}: {reply}");
        assert!(reply["error"]["message"].as_str().unwrap().contains(key));
    }
    // And in an end of a distance.
    let refused = json!({"source": "missing", "query": {
        "kind": "measure", "from": {"id": "DEMO-001/border", "colour": 1}, "to": end}});
    assert!(!validator.is_valid(&refused));
    assert_eq!(run("query", refused)["error"]["code"], "usage");
}

#[test]
fn a_key_the_schema_does_not_name_is_refused_in_a_change_set_and_changes_nothing() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("commands-unknown-key");
    let _ = std::fs::remove_dir_all(&dir);
    let set = dir.join("demo-set");
    demo_set::copy_demo_set(&set);
    let before = snapshot::snapshot(&set);
    let validator =
        jsonschema::validator_for(&find_command("change").unwrap().input_schema()).unwrap();
    let operation = json!({"op": "move", "target": {"id": "DEMO-001/border"}, "by": [0.1, 0, 0]});
    let mut in_operation = operation.clone();
    in_operation["colour"] = json!(1);
    for change_set in [
        json!({"operations": [operation], "colour": 1}),
        json!({"operations": [in_operation]}),
    ] {
        let input = json!({"set": set, "change_set": change_set, "dry_run": false});
        assert!(!validator.is_valid(&input), "{input}");
        let reply = run("change", input.clone());
        assert_eq!(reply["ok"], false, "{input}: {reply}");
        assert!(
            ["unknown_key", "usage"].contains(&reply["error"]["code"].as_str().unwrap()),
            "{reply}"
        );
    }
    // Every record as it was, and no DWG built; only the journal says the
    // attempts were made.
    assert_eq!(snapshot::snapshot(&set), before);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_value_of_the_wrong_shape_is_a_usage_error_naming_it() {
    for (name, input) in [
        ("build", json!({"source": "a.yaml", "fonts": "one-folder"})),
        ("plot", json!({"source": "a.yaml", "plot_styles": [1]})),
        (
            "build",
            json!({"source": "a.yaml", "plot_style": "monochrome.ctb"}),
        ),
        ("undo", json!({"set": "s", "change_set": "first"})),
        ("schema", json!({"kind": "drawing"})),
        (
            "query",
            json!({"source": "s", "query": {"kind": "everything"}}),
        ),
    ] {
        let reply = run(name, input.clone());
        assert_eq!(reply["ok"], false, "{name}: {input}");
        assert_eq!(reply["error"]["code"], "usage", "{name}: {input}: {reply}");
        let validator =
            jsonschema::validator_for(&find_command(name).unwrap().input_schema()).unwrap();
        assert!(
            !validator.is_valid(&input),
            "{name}'s schema refuses {input}"
        );
    }
    let reply = run("build", json!({"source": "a.yaml", "fonts": "one-folder"}));
    assert_eq!(reply["error"]["field"], "fonts");
    for not_an_object in [json!(null), json!([]), json!("build"), json!(3)] {
        let reply = run("operations", not_an_object.clone());
        assert_eq!(reply["error"]["code"], "usage", "{not_an_object}");
    }
}

#[test]
fn a_command_gives_what_the_core_entry_point_gives() {
    for (kind, schema) in [
        (None, sheet_description_schema()),
        (Some("sheet-description"), sheet_description_schema()),
        (Some("block-library"), block_library_schema()),
        (Some("template"), template_schema()),
        (Some("set"), set_schema()),
        (Some("rows"), rows_schema()),
        (Some("change-set"), change_set_schema()),
    ] {
        let input = kind.map_or(json!({}), |kind| json!({ "kind": kind }));
        let reply = run("schema", input);
        assert_eq!(reply, json!({"ok": true, "result": schema}), "{kind:?}");
    }
    let reply = run("operations", json!({}));
    assert_eq!(reply["result"], serde_json::to_value(operations()).unwrap());

    // The reply keeps the order the core gives its fields, not the
    // alphabet's.
    let reply = find_command("measure-text").unwrap().run(
        json!({"contents": "PUMP 101", "font": "missing.ttf", "height": 2.5}),
        &Settings::default(),
        Interface::Cli,
    );
    let text = serde_json::to_string(&reply).unwrap();
    assert!(
        text.starts_with(r#"{"ok":true,"result":{"width":"#),
        "{text}"
    );

    // A change set's input schema holds the change set's own schema.
    let input = find_command("change").unwrap().input_schema();
    let mut own = change_set_schema();
    let defs = own.as_object_mut().unwrap().remove("$defs").unwrap();
    own.as_object_mut().unwrap().remove("$schema");
    assert_eq!(input["properties"]["change_set"], own);
    assert_eq!(input["$defs"], defs);
}

#[test]
fn a_folder_is_a_drawing_set_whose_outputs_go_where_its_settings_say() {
    let dir = std::env::temp_dir().join(format!("agentdrafter-commands-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["build", "plot"] {
        let reply = run(name, json!({"source": dir, "output": "x"}));
        assert_eq!(reply["error"]["code"], "usage", "{name}: {reply}");
        assert!(reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("is for one sheet description"));
        // Without an output path the folder is read as a set, which has no
        // settings here.
        let reply = run(name, json!({"source": dir}));
        assert_ne!(reply["error"]["code"], "usage", "{name}: {reply}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(
        serde_json::to_value(ErrorCode::Unauthorized).unwrap(),
        "unauthorized"
    );
    assert_eq!(
        serde_json::to_value(ErrorCode::ListenFailed).unwrap(),
        "listen_failed"
    );
}
