use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::*;
use crate::build::{build_with, BuildRequest};
use crate::commands::find_command;
use crate::dwg;
use crate::interface::Interface;
use crate::original::read_original;
use crate::ReadRequest;

fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("agentdrafter-revise-{}", std::process::id()))
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Builds a rich made-up sheet description (lines, circle, arc, text, mtext,
/// block with attributes, locked layer) and strips AgentDrafter's build mark
/// and extended data so it behaves as a plant's original DWG (ADR-0008).
fn build_made_up_original(dir: &Path, name: &str) -> PathBuf {
    let blocks = dir.join("blocks.yaml");
    fs::write(
        &blocks,
        "units: mm\n\
         blocks:\n\
         \x20\x20INSTR:\n\
         \x20\x20\x20\x20base_point: [0, 0, 0]\n\
         \x20\x20\x20\x20objects:\n\
         \x20\x20\x20\x20\x20\x20- circle: {center: [0, 0, 0], radius: 5}\n\
         \x20\x20\x20\x20\x20\x20- attdef: {tag: TAG, prompt: Tag, default: X-000, position: [0, 1, 0], height: 2, justify: middle_center}\n\
         \x20\x20\x20\x20\x20\x20- attdef: {tag: REV, prompt: Rev, default: '0', position: [0, -2, 0], height: 1.5, justify: middle_center}\n",
    )
    .unwrap();
    let yaml = dir.join(format!("{name}.yaml"));
    fs::write(
        &yaml,
        "units: mm\n\
         block_library: blocks.yaml\n\
         layers:\n\
         \x20\x20wire: {color: 1}\n\
         \x20\x20notes: {color: 3}\n\
         \x20\x20locked_layer: {color: 5, locked: true}\n\
         objects:\n\
         \x20\x20- line: {from: [0, 0, 0], to: [100, 0, 0], layer: wire}\n\
         \x20\x20- line: {from: [0, 20, 0], to: [100, 20, 0], layer: wire}\n\
         \x20\x20- circle: {center: [50, 50, 0], radius: 10, layer: notes}\n\
         \x20\x20- arc: {center: [10, 50, 0], radius: 8, start_angle: 0, end_angle: 90, layer: notes}\n\
         \x20\x20- text: {contents: MADE UP TITLE, position: [10, 80, 0], height: 4, layer: notes}\n\
         \x20\x20- mtext: {contents: LINE ONE\\nLINE TWO, position: [60, 80, 0], height: 3, width: 30, layer: notes}\n\
         \x20\x20- insert: {block: INSTR, position: [30, 30, 0], layer: notes, attributes: {TAG: PT-101, REV: A}}\n\
         \x20\x20- line: {from: [0, 95, 0], to: [20, 95, 0], layer: locked_layer}\n",
    )
    .unwrap();
    let built = dir.join(format!("{name}_built.dwg"));
    let settings = Settings::default().without_system_fonts();
    build_with(
        &BuildRequest {
            description: yaml,
            output: Some(built.clone()),
            overwrite: true,
        },
        &settings,
    )
    .expect("built made-up DWG");
    let orig = dir.join(format!("{name}.dwg"));
    dwg::as_someone_elses(&built, &orig);
    let _ = fs::remove_file(&built);
    orig
}

fn dir_files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn revises_made_up_original_with_all_operations_and_preserves_untouched_objects() {
    let dir = scratch("all_ops");
    let orig = build_made_up_original(&dir, "LOOP-101");
    let orig_bytes = fs::read(&orig).unwrap();
    let settings = Settings::default().without_system_fonts();

    // Read original to discover handle IDs of every object.
    let loaded = crate::original::read_dwg_record(&orig, false).unwrap();
    let dwg_origin = loaded.record.dwg.as_ref().unwrap();
    // 0: line (keep UNTOUCHED!), 1: line (move + change), 2: circle (change radius + scale),
    // 3: arc (rotate), 4: text (change contents + mirror copy), 5: mtext (change text),
    // 6: insert (set_attribute + copy + array), 7: locked line (untouched).
    let untouched_handle = dwg_origin.handles[0].clone();
    let untouched_raw_before =
        dwg::raw_record_bytes_of(&orig, &untouched_handle).expect("untouched line raw_record");
    let line2_id = loaded.record.objects[1].id.clone();
    let circle_id = loaded.record.objects[2].id.clone();
    let arc_id = loaded.record.objects[3].id.clone();
    let text_id = loaded.record.objects[4].id.clone();
    let mtext_id = loaded.record.objects[5].id.clone();
    let insert_id = loaded.record.objects[6].id.clone();
    let line1_id = loaded.record.objects[0].id.clone();

    let change_set = json!({
        "operations": [
            {"op": "set_attribute", "target": {"id": insert_id}, "tag": "REV", "value": "B"},
            {"op": "change", "target": {"id": text_id}, "set": {"contents": "REVISED TITLE", "color": 2}},
            {"op": "change", "target": {"id": mtext_id}, "set": {"contents": "UPDATED NOTE"}},
            {"op": "change", "target": {"id": circle_id}, "set": {"radius": 15, "layer": "wire"}},
            {"op": "move", "target": {"id": line2_id}, "from": {"snap": {"id": line1_id, "snap": "endpoint", "which": "from"}}, "to": [5, 5, 0]},
            {"op": "rotate", "target": {"id": arc_id}, "base_point": [10, 50, 0], "angle": 90},
            {"op": "scale", "target": {"id": circle_id}, "base_point": [50, 50, 0], "factor": 2},
            {"op": "mirror", "target": {"id": text_id}, "first_point": [50, 0, 0], "second_point": [50, 100, 0]},
            {"op": "copy", "target": {"id": insert_id}, "by": [25, 0, 0]},
            {"op": "array", "kind": "rectangular", "target": {"id": circle_id}, "rows": 1, "columns": 2, "column_spacing": 40},
            {"op": "add_layer", "name": "added_layer", "color": 4},
            {"op": "change_layer", "name": "added_layer", "set": {"color": 6, "description": "Revised"}},
            {"op": "add_line", "layer": "added_layer", "from": {"snap": {"id": line1_id, "snap": "midpoint"}}, "to": [50, 15, 0]},
            {"op": "add_circle", "center": [80, 40, 0], "radius": 6, "layer": "notes"},
            {"op": "add_text", "contents": "NEW CALLOUT", "position": [80, 55, 0], "height": 3, "layer": "notes"},
            {"op": "insert_block", "block": "INSTR", "position": [75, 25, 0], "layer": "notes", "attributes": {"TAG": "FT-202", "REV": "0"}},
            {"op": "delete", "target": {"id": line2_id}}
        ]
    });

    let reply = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set,
            dry_run: false,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: false,
        },
        &settings,
    );
    assert!(reply.ok, "revision failed: {reply:?}");
    let report = reply.result.expect("revision report");
    assert_eq!(report.original.dwg_version, "AC1032");
    assert_eq!(
        report.revision.as_ref().unwrap().dwg_version,
        report.original.dwg_version
    );
    assert!(
        report.unexpected_changes.is_empty(),
        "unexpected changes: {:?}",
        report.unexpected_changes
    );
    assert_eq!(report.operations.len(), 17);
    assert!(report.checks.is_some());

    // Original file must be untouched byte-for-byte.
    assert_eq!(fs::read(&orig).unwrap(), orig_bytes);

    // Default output files: LOOP-101.rev1.dwg, LOOP-101.rev1.changes.json, LOOP-101.rev1.pdf
    let rev1_dwg = dir.join("LOOP-101.rev1.dwg");
    let rev1_changes = dir.join("LOOP-101.rev1.changes.json");
    let rev1_pdf = dir.join("LOOP-101.rev1.pdf");
    assert!(rev1_dwg.is_file(), "rev1.dwg exists");
    assert!(rev1_changes.is_file(), "rev1.changes.json exists");
    assert!(rev1_pdf.is_file(), "rev1.pdf exists");
    assert!(fs::read(&rev1_pdf).unwrap().starts_with(b"%PDF-"));

    // Untouched line's raw record bytes survived verbatim!
    let untouched_raw_after = dwg::raw_record_bytes_of(&rev1_dwg, &untouched_handle)
        .expect("untouched line raw_record after");
    assert_eq!(untouched_raw_before, untouched_raw_after);

    // Change list JSON has all required fields.
    let cl: Value = serde_json::from_str(&fs::read_to_string(&rev1_changes).unwrap()).unwrap();
    assert_eq!(cl["format"], 1);
    assert_eq!(cl["original"]["dwg_version"], "AC1032");
    assert_eq!(cl["revision"]["dwg_version"], "AC1032");
    assert_eq!(cl["allowed_loss"], false);
    assert_eq!(cl["unexpected_changes"], json!([]));
    assert_eq!(cl["operations"].as_array().unwrap().len(), 17);

    // Re-read the revision DWG and verify edited content landed cleanly.
    let re_read = read_original(&ReadRequest {
        source: rev1_dwg.clone(),
        mirror: None,
        no_mirror: true,
    })
    .unwrap();
    assert!(re_read.decoded);
    let rev_loaded = crate::original::read_dwg_record(&rev1_dwg, false).unwrap();
    let has_rev_b = rev_loaded.record.sheet.objects.iter().any(|o| match o {
        crate::model::Object::Insert(ins) => {
            ins.attributes
                .iter()
                .any(|a| a.tag == "TAG" && a.value == "PT-101")
                && ins
                    .attributes
                    .iter()
                    .any(|a| a.tag == "REV" && a.value == "B")
        }
        _ => false,
    });
    assert!(has_rev_b, "PT-101 insert should now carry REV=B");
    let has_new_insert = rev_loaded.record.sheet.objects.iter().any(|o| match o {
        crate::model::Object::Insert(ins) => ins
            .attributes
            .iter()
            .any(|a| a.tag == "TAG" && a.value == "FT-202"),
        _ => false,
    });
    assert!(has_new_insert, "inserted FT-202 block should exist");
    let has_new_layer = rev_loaded
        .record
        .sheet
        .layers
        .iter()
        .any(|l| l.name == "added_layer" && l.color == 6 && l.description == "Revised");
    assert!(
        has_new_layer,
        "added_layer should have updated color & description"
    );

    // Second revision without --output picks LOOP-101.rev2.dwg without clobbering rev1!
    let rev1_bytes = fs::read(&rev1_dwg).unwrap();
    let reply2 = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: json!({"operations": [{"op": "add_line", "from": [0, 0, 0], "to": [10, 10, 0]}]}),
            dry_run: false,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(reply2.ok, "second revision failed: {reply2:?}");
    assert!(dir.join("LOOP-101.rev2.dwg").is_file());
    assert!(dir.join("LOOP-101.rev2.changes.json").is_file());
    assert!(!dir.join("LOOP-101.rev2.pdf").exists());
    assert_eq!(fs::read(&rev1_dwg).unwrap(), rev1_bytes);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn revision_preserves_original_dwg_version_across_multiple_versions() {
    let dir = scratch("versions");
    let base = build_made_up_original(&dir, "BASE");
    let settings = Settings::default().without_system_fonts();

    for version in ["AC1015", "AC1018", "AC1021", "AC1024", "AC1027", "AC1032"] {
        let orig = dir.join(format!("VER_{version}.dwg"));
        dwg::as_original_of_version(&base, &orig, version);
        let orig_bytes = fs::read(&orig).unwrap();
        assert_eq!(&orig_bytes[..6], version.as_bytes());

        let loaded = crate::original::read_dwg_record(&orig, false).unwrap();
        let line_id = loaded.record.objects[1].id.clone();
        let text_id = loaded.record.objects[4].id.clone();

        let reply = revise_with(
            &ReviseRequest {
                set: orig.clone(),
                change_set: json!({
                    "operations": [
                        {"op": "move", "target": {"id": line_id}, "by": [2, 3, 0]},
                        {"op": "change", "target": {"id": text_id}, "set": {"contents": format!("REV {version}")}},
                        {"op": "add_line", "from": [1, 2, 0], "to": [11, 12, 0]}
                    ]
                }),
                dry_run: false,
                output: None,
                allow_loss: false,
                plot_style: None,
                no_pdf: true,
            },
            &settings,
        );
        assert!(reply.ok, "version {version} failed: {reply:?}");
        let report = reply.result.unwrap();
        assert_eq!(report.original.dwg_version, version);
        assert_eq!(report.revision.as_ref().unwrap().dwg_version, version);
        assert!(
            report.unexpected_changes.is_empty(),
            "version {version} unexpected changes: {:?}",
            report.unexpected_changes
        );

        let rev_path = PathBuf::from(report.revision_dwg.unwrap());
        let rev_bytes = fs::read(&rev_path).unwrap();
        assert_eq!(
            &rev_bytes[..6],
            version.as_bytes(),
            "revision header must stay {version}"
        );
        assert_eq!(fs::read(&orig).unwrap(), orig_bytes);
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn decode_loss_gate_refuses_without_allow_loss_and_preserves_custom_object_with_allow_loss() {
    let dir = scratch("decode_gate");
    let base = build_made_up_original(&dir, "GATE");
    let custom_dwg = dir.join("CUSTOM-101.dwg");
    let custom_handle = dwg::with_a_made_up_custom_object(&base, &custom_dwg, "MADE_UP_VALVE");
    let custom_raw_before =
        dwg::raw_record_bytes_of(&custom_dwg, &custom_handle).expect("custom object raw bytes");
    let before_files = dir_files(&dir);
    let settings = Settings::default().without_system_fonts();

    let cs = json!({"operations": [{"op": "add_line", "from": [5, 5, 0], "to": [25, 5, 0]}]});

    // Without allow_loss -> refused, 0 files written, naming kind & handle.
    let refused = revise_with(
        &ReviseRequest {
            set: custom_dwg.clone(),
            change_set: cs.clone(),
            dry_run: false,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(!refused.ok);
    let err = refused.error.expect("incomplete_decode error");
    assert_eq!(err.code, ErrorCode::IncompleteDecode);
    assert!(err.message.contains("MADE_UP_VALVE"), "{}", err.message);
    assert!(err.message.contains(&custom_handle), "{}", err.message);
    let partial = refused.result.expect("partial decode report");
    assert!(!partial.decode.decoded);
    assert!(partial
        .decode
        .not_decoded
        .iter()
        .any(|k| k.kind == "MADE_UP_VALVE" && k.handles.contains(&custom_handle)));
    assert_eq!(
        dir_files(&dir),
        before_files,
        "zero files written on refusal"
    );

    // With allow_loss -> succeeds, records accepted loss, AND preserves custom object raw bytes!
    let allowed = revise_with(
        &ReviseRequest {
            set: custom_dwg.clone(),
            change_set: cs,
            dry_run: false,
            output: None,
            allow_loss: true,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(allowed.ok, "allowed revision failed: {allowed:?}");
    let report = allowed.result.unwrap();
    assert!(report.allowed_loss);
    assert!(!report.decode.decoded);
    assert!(
        report.unexpected_changes.is_empty(),
        "{:?}",
        report.unexpected_changes
    );

    let rev_dwg = PathBuf::from(report.revision_dwg.unwrap());
    let custom_raw_after =
        dwg::raw_record_bytes_of(&rev_dwg, &custom_handle).expect("custom object preserved");
    assert_eq!(custom_raw_before, custom_raw_after);

    let cl: Value =
        serde_json::from_str(&fs::read_to_string(report.change_list.unwrap()).unwrap()).unwrap();
    assert_eq!(cl["allowed_loss"], true);
    assert_eq!(cl["decode_report"]["decoded"], false);
    assert_eq!(
        cl["decode_report"]["not_decoded"][0]["kind"],
        "MADE_UP_VALVE"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn safety_refuses_original_existing_symlink_dir_and_non_dwg_output() {
    let dir = scratch("safety");
    let orig = build_made_up_original(&dir, "SAFE-1");
    let orig_bytes = fs::read(&orig).unwrap();
    let settings = Settings::default().without_system_fonts();
    let cs = json!({"operations": [{"op": "add_line", "from": [0, 0, 0], "to": [5, 0, 0]}]});

    // 1. Output equal to original -> refused, original unchanged.
    let r = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: cs.clone(),
            dry_run: false,
            output: Some(orig.clone()),
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(!r.ok);
    assert_eq!(r.error.unwrap().code, ErrorCode::Usage);
    assert_eq!(fs::read(&orig).unwrap(), orig_bytes);

    // 2. Output already exists -> refused, existing file unchanged.
    let existing = dir.join("someone_else.dwg");
    fs::write(&existing, b"not-a-dwg-important").unwrap();
    let r = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: cs.clone(),
            dry_run: false,
            output: Some(existing.clone()),
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(!r.ok);
    assert_eq!(r.error.unwrap().code, ErrorCode::OutputExists);
    assert_eq!(fs::read(&existing).unwrap(), b"not-a-dwg-important");

    // 3. Output is a directory -> refused.
    let subdir = dir.join("folder.dwg");
    fs::create_dir(&subdir).unwrap();
    let r = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: cs.clone(),
            dry_run: false,
            output: Some(subdir),
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(!r.ok);
    assert_eq!(r.error.unwrap().code, ErrorCode::OutputExists);

    // 4. Output does not end .dwg -> refused.
    let r = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: cs.clone(),
            dry_run: false,
            output: Some(dir.join("out.yaml")),
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(!r.ok);
    assert_eq!(r.error.unwrap().code, ErrorCode::Usage);

    // 5. Output is a symlink -> refused (Unix).
    #[cfg(unix)]
    {
        let link = dir.join("link.dwg");
        std::os::unix::fs::symlink(dir.join("nonexistent_target.dwg"), &link).unwrap();
        let r = revise_with(
            &ReviseRequest {
                set: orig.clone(),
                change_set: cs.clone(),
                dry_run: false,
                output: Some(link),
                allow_loss: false,
                plot_style: None,
                no_pdf: true,
            },
            &settings,
        );
        assert!(!r.ok);
        assert_eq!(r.error.unwrap().code, ErrorCode::OutputExists);
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn all_or_nothing_and_dry_run_write_zero_files() {
    let dir = scratch("atomic_and_dry");
    let orig = build_made_up_original(&dir, "ATOM-1");
    let before = dir_files(&dir);
    let settings = Settings::default().without_system_fonts();

    // Op 0 valid, op 1 fails -> refused whole, 0 files written.
    let r = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: json!({
                "operations": [
                    {"op": "add_line", "from": [0, 0, 0], "to": [10, 0, 0]},
                    {"op": "delete", "target": {"id": "ATOM-1/handle:DEAD"}}
                ]
            }),
            dry_run: false,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: false,
        },
        &settings,
    );
    assert!(!r.ok);
    assert_eq!(r.error.unwrap().code, ErrorCode::UnknownName);
    assert_eq!(dir_files(&dir), before);

    // Dry run succeeds and writes 0 files.
    let dry = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: json!({"operations": [{"op": "add_line", "from": [0, 0, 0], "to": [10, 0, 0]}]}),
            dry_run: true,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: false,
        },
        &settings,
    );
    assert!(dry.ok, "dry run failed: {dry:?}");
    let report = dry.result.unwrap();
    assert!(report.dry_run);
    assert!(report.unexpected_changes.is_empty());
    assert!(report.checks.is_some());
    assert_eq!(dir_files(&dir), before, "dry run must write zero files");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn target_resolution_locked_layer_and_ambiguous_names() {
    let dir = scratch("targets");
    let orig = build_made_up_original(&dir, "TARG-1");
    let settings = Settings::default().without_system_fonts();
    let loaded = crate::original::read_dwg_record(&orig, false).unwrap();
    let locked_id = loaded.record.objects[7].id.clone();

    // Locked layer refuses edit with ErrorCode::Locked.
    let locked_reply = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: json!({"operations": [{"op": "delete", "target": {"id": locked_id}}]}),
            dry_run: true,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(!locked_reply.ok);
    assert_eq!(locked_reply.error.unwrap().code, ErrorCode::Locked);

    // Wrong sheet name refused cleanly.
    let wrong_sheet = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: json!({"operations": [{"op": "add_line", "sheet": "OTHER", "from": [0, 0, 0], "to": [1, 0, 0]}]}),
            dry_run: true,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(!wrong_sheet.ok);
    assert_eq!(wrong_sheet.error.unwrap().code, ErrorCode::UnknownName);

    // Unsupported operation refused cleanly before writing anything.
    let unsupported = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: json!({"operations": [{"op": "import_blocks", "from": "other.dwg"}]}),
            dry_run: true,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(!unsupported.ok);
    assert_eq!(unsupported.error.unwrap().code, ErrorCode::Usage);

    // Ambiguous carried name fails listing all matching IDs.
    let dup = dir.join("DUP.dwg");
    dwg::with_duplicate_carried_name(&orig, &dup, "shared_name");
    let ambig = revise_with(
        &ReviseRequest {
            set: dup,
            change_set: json!({"operations": [{"op": "move", "target": {"name": "shared_name"}, "by": [1, 0, 0]}]}),
            dry_run: true,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(!ambig.ok);
    let err = ambig.error.unwrap();
    assert_eq!(err.code, ErrorCode::BadValue);
    assert!(err.message.contains("DUP/obj-0"), "{}", err.message);
    assert!(err.message.contains("DUP/obj-1"), "{}", err.message);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn command_dispatch_distinguishes_dwg_folder_from_dwg_file_and_refuses_original_options_on_set() {
    let dir = scratch("dispatch");
    let settings = Settings::default().without_system_fonts();
    let change_cmd = find_command("change").unwrap();

    // A drawing-set folder whose name ends `.DWG` must STILL be handled as a drawing set!
    let set_dir = dir.join("PLANT.DWG");
    fs::create_dir_all(&set_dir).unwrap();
    fs::write(
        set_dir.join("set.yaml"),
        "units: mm\noutput: dwg\nrows: rows.yaml\n",
    )
    .unwrap();
    fs::write(
        set_dir.join("tpl.yaml"),
        "units: mm\npaper: ISO A4 landscape\nobjects:\n  - line: {id: base, from: [0, 0, 0], to: [10, 0, 0]}\n",
    )
    .unwrap();
    fs::write(
        set_dir.join("rows.yaml"),
        "rows:\n  - sheet: S-1\n    template: tpl.yaml\n",
    )
    .unwrap();

    // Original-only options on a drawing-set folder are refused with Usage!
    for (key, val) in [
        ("output", json!("out.dwg")),
        ("allow_loss", json!(true)),
        ("plot_style", json!("monochrome.ctb")),
        ("plot_styles", json!(["styles"])),
        ("no_pdf", json!(true)),
    ] {
        let mut input = json!({
            "set": set_dir.display().to_string(),
            "change_set": {"operations": [{"op": "add_line", "sheet": "S-1", "from": [0, 1, 0], "to": [5, 1, 0]}]},
            "dry_run": true
        });
        input[key] = val;
        let reply = change_cmd.run(input, &settings, Interface::Cli);
        assert!(!reply.ok, "option `{key}` should be refused on a set");
        assert_eq!(reply.error.unwrap().code, ErrorCode::Usage);
    }

    // Normal drawing-set change on `PLANT.DWG/` succeeds as a drawing set!
    let ok_set = change_cmd.run(
        json!({
            "set": set_dir.display().to_string(),
            "change_set": {"operations": [{"op": "add_line", "sheet": "S-1", "from": [0, 1, 0], "to": [5, 1, 0]}]},
            "dry_run": true
        }),
        &settings,
        Interface::Cli,
    );
    assert!(ok_set.ok, "set change on .DWG folder failed: {ok_set:?}");

    // Standalone `.dwg` file routes to original-DWG revision and skips journal recovery!
    let orig = build_made_up_original(&dir, "ALONE");
    let ok_dwg = change_cmd.run(
        json!({
            "set": orig.display().to_string(),
            "change_set": {"operations": [{"op": "add_line", "from": [0, 0, 0], "to": [5, 5, 0]}]},
            "dry_run": true
        }),
        &settings,
        Interface::Cli,
    );
    assert!(ok_dwg.ok, "original DWG change failed: {ok_dwg:?}");
    let val: Value = serde_json::from_str(ok_dwg.result.unwrap().get()).unwrap();
    assert_eq!(val["sheet"], "ALONE");
    assert_eq!(val["original"]["dwg_version"], "AC1032");
    assert!(ok_dwg.rolled_back.is_none());

    let _ = fs::remove_dir_all(&dir);
}
