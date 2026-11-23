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

fn dir_files_no_journal(dir: &Path) -> Vec<String> {
    dir_files(dir)
        .into_iter()
        .filter(|n| n != ".agentdrafter")
        .collect()
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
        dir_files_no_journal(&dir),
        before_files,
        "zero revision files written on refusal"
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
    assert_eq!(dir_files_no_journal(&dir), before);

    // Dry run on a fresh original succeeds and writes 0 files (not even `.agentdrafter/`).
    let dry_dir = scratch("atomic_and_dry_only");
    let dry_orig = build_made_up_original(&dry_dir, "DRY-1");
    let dry_before = dir_files(&dry_dir);
    let dry = revise_with(
        &ReviseRequest {
            set: dry_orig.clone(),
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
    assert!(report.journal_entry.is_none());
    assert!(report.unexpected_changes.is_empty());
    assert!(report.checks.is_some());
    assert_eq!(
        dir_files(&dry_dir),
        dry_before,
        "dry run must write zero files including .agentdrafter"
    );
    let _ = fs::remove_dir_all(&dry_dir);

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

#[test]
fn rollback_checks_file_identity_before_unlinking_and_reports_notes() {
    use std::io::Write;
    let dir = scratch("rollback_id");
    let dest = dir.join("rev.dwg");
    let (tmp1, mut f1) = crate::original::create_temp_beside_kind(&dest, "rev").unwrap();
    #[cfg(any(unix, windows))]
    let id1 = crate::build::file_id_of_file(&f1);
    #[cfg(not(any(unix, windows)))]
    let id1: Option<FileId> = None;
    f1.write_all(b"ours").unwrap();
    drop(f1);
    crate::original::place_new_file(&tmp1, &dest, "revision DWG", ErrorCode::OutputExists).unwrap();

    // Case 1: file at dest still has our FileId -> rollback removes it cleanly.
    let notes = crate::change::rollback_placed_new_files(&[(&dest, id1)], &[], None);
    assert!(notes.is_empty(), "unexpected rollback notes: {notes:?}");
    assert!(!dest.exists(), "rollback should remove our placed file");

    // Case 2: file at dest is replaced by a different file -> rollback must NOT
    // delete the unrelated file, and must return a note.
    let (tmp2, mut f2) = crate::original::create_temp_beside_kind(&dest, "rev").unwrap();
    #[cfg(any(unix, windows))]
    let id2 = crate::build::file_id_of_file(&f2);
    #[cfg(not(any(unix, windows)))]
    let id2: Option<FileId> = None;
    f2.write_all(b"ours-2").unwrap();
    drop(f2);
    crate::original::place_new_file(&tmp2, &dest, "revision DWG", ErrorCode::OutputExists).unwrap();
    let other = dir.join("other.dwg");
    fs::write(&other, b"someone-else").unwrap();
    fs::rename(&other, &dest).unwrap();

    #[cfg(any(unix, windows))]
    {
        let notes = crate::change::rollback_placed_new_files(&[(&dest, id2)], &[], None);
        assert!(
            !notes.is_empty(),
            "rollback must report when file identity changed"
        );
        assert_eq!(
            fs::read(&dest).unwrap(),
            b"someone-else",
            "rollback must never unlink an unrelated file swapped into place"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn change_layer_in_place_preserves_unmodeled_true_color_linetype_and_lineweight() {
    let dir = scratch("true_color_layer");
    let base = build_made_up_original(&dir, "BASE");
    let orig = dir.join("TC-1.dwg");
    dwg::with_true_color_locked_layer(&base, &orig, "TRUECOL", 12, 134, 210);
    let settings = Settings::default().without_system_fonts();

    let reply = revise_with(
        &ReviseRequest {
            set: orig,
            change_set: json!({
                "operations": [
                    {"op": "change_layer", "name": "TRUECOL", "set": {"locked": false}}
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
    assert!(reply.ok, "unlocking true-color layer failed: {reply:?}");
    let rev_path = PathBuf::from(reply.result.unwrap().revision_dwg.unwrap());
    let (color, lt, lw, locked) = dwg::layer_details_of(&rev_path, "TRUECOL").expect("layer");
    assert_eq!(color, "Rgb { r: 12, g: 134, b: 210 }");
    assert!(lt.eq_ignore_ascii_case("DASHED"), "got linetype {lt}");
    assert_eq!(lw, "Value(35)");
    assert!(!locked);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn refuses_non_exact_six_byte_dwg_header_like_ad1032() {
    let dir = scratch("header_6_bytes");
    let base = build_made_up_original(&dir, "BASE");
    let ad = dir.join("AD-1.dwg");
    dwg::with_custom_header(&base, &ad, b"AD1032");
    let before = dir_files(&dir);
    let settings = Settings::default().without_system_fonts();

    let reply = revise_with(
        &ReviseRequest {
            set: ad,
            change_set: json!({
                "operations": [{"op": "add_line", "from": [0, 0, 0], "to": [5, 0, 0]}]
            }),
            dry_run: false,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(
        !reply.ok,
        "AD1032 must be refused rather than silently rewritten as AC1032"
    );
    assert!(
        matches!(
            reply.error.as_ref().unwrap().code,
            ErrorCode::DwgWrite | ErrorCode::DwgRead
        ),
        "{:?}",
        reply.error
    );
    assert_eq!(
        dir_files_no_journal(&dir),
        before,
        "no revision files may be written"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn damaged_dwg_refused_without_allow_loss_and_revises_with_allow_loss() {
    let dir = scratch("damaged_dwg");
    let base = build_made_up_original(&dir, "BASE");
    let damaged = dir.join("DMG-1.dwg");
    let _damaged_handle = dwg::with_a_damaged_line(&base, &damaged);
    let before = dir_files(&dir);
    let settings = Settings::default().without_system_fonts();
    let cs = json!({
        "operations": [{"op": "add_line", "from": [0, 0, 0], "to": [10, 0, 0]}]
    });

    // Without allow_loss -> refused with IncompleteDecode and 0 files written.
    let refused = revise_with(
        &ReviseRequest {
            set: damaged.clone(),
            change_set: cs.clone(),
            dry_run: false,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(
        !refused.ok,
        "damaged DWG must be refused without allow_loss"
    );
    assert_eq!(
        refused.error.as_ref().unwrap().code,
        ErrorCode::IncompleteDecode,
        "{refused:?}"
    );
    let refused_report = refused.result.expect("partial report on decode refusal");
    assert!(!refused_report.decode.decoded);
    assert!(
        refused_report.decode.read_past_damage.is_some()
            || !refused_report.decode.not_decoded.is_empty()
            || refused_report.decode.skipped > 0
            || refused_report.decode.recovered_errors > 0
    );
    assert_eq!(dir_files_no_journal(&dir), before);

    // With allow_loss -> succeeds and records decode loss in .changes.json.
    let allowed = revise_with(
        &ReviseRequest {
            set: damaged,
            change_set: cs,
            dry_run: false,
            output: None,
            allow_loss: true,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(
        allowed.ok,
        "damaged DWG should revise with allow_loss: {allowed:?}"
    );
    let rep = allowed.result.unwrap();
    assert!(rep.allowed_loss);
    assert!(!rep.decode.decoded);
    let cl_json: Value =
        serde_json::from_str(&fs::read_to_string(rep.change_list.unwrap()).unwrap()).unwrap();
    assert_eq!(cl_json["allowed_loss"], true);
    assert_eq!(cl_json["decode_report"]["decoded"], false);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn polyline3d_points_change_preserves_xdata_and_copy_allocates_distinct_vertex_handles() {
    let dir = scratch("p3d");
    let base = build_made_up_original(&dir, "BASE");
    let orig = dir.join("P3D-1.dwg");
    let p3_handle = dwg::with_polyline3d_original(&base, &orig);
    let p3_id = format!("P3D-1/handle:{p3_handle}");
    let settings = Settings::default().without_system_fonts();

    let reply = revise_with(
        &ReviseRequest {
            set: orig,
            change_set: json!({
                "operations": [
                    {
                        "op": "change",
                        "target": {"id": p3_id},
                        "set": {
                            "points": [
                                [0, 0, 0],
                                [20, 0, 5],
                                [20, 20, 10],
                                [0, 20, 15]
                            ]
                        }
                    },
                    {
                        "op": "copy",
                        "target": {"id": p3_id},
                        "by": [100, 0, 0]
                    }
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
    assert!(reply.ok, "Polyline3D edit + copy failed: {reply:?}");
    let rep = reply.result.unwrap();
    assert!(
        rep.unexpected_changes.is_empty(),
        "{:?}",
        rep.unexpected_changes
    );
    let rev_dwg = PathBuf::from(rep.revision_dwg.unwrap());
    let copy_handle = rep.operations[1].copies[0].handle.clone();

    let (src_vh, src_pts, src_xd) =
        dwg::polyline3d_details_of(&rev_dwg, &p3_handle).expect("edited Polyline3D");
    assert!(
        src_xd,
        "custom TESTAPP xdata must survive Polyline3D points edit"
    );
    assert_eq!(src_pts.len(), 4);
    assert_eq!(src_pts[1], [20.0, 0.0, 5.0]);
    assert_eq!(src_pts[3], [0.0, 20.0, 15.0]);

    let (cpy_vh, cpy_pts, _) =
        dwg::polyline3d_details_of(&rev_dwg, &copy_handle).expect("copied Polyline3D");
    assert_eq!(cpy_pts.len(), 4);
    assert_eq!(cpy_pts[0], [100.0, 0.0, 0.0]);
    assert_eq!(cpy_pts[3], [100.0, 20.0, 15.0]);
    let all_handles: HashSet<&String> = src_vh.iter().chain(cpy_vh.iter()).collect();
    assert_eq!(
        all_handles.len(),
        src_vh.len() + cpy_vh.len(),
        "vertex handles must be distinct between source ({src_vh:?}) and copy ({cpy_vh:?})"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn copy_strips_raw_dwg_eed_id_and_block_def_handle_is_refused() {
    let dir = scratch("eed_and_block_def");
    let yaml = dir.join("CARRIED.yaml");
    fs::write(
        &yaml,
        "units: mm\n\
         objects:\n\
         \x20\x20- line: {id: my_line, name: named_line, from: [0, 0, 0], to: [10, 0, 0]}\n",
    )
    .unwrap();
    let orig = dir.join("CARRIED.dwg");
    let settings = Settings::default().without_system_fonts();
    build_with(
        &BuildRequest {
            description: yaml,
            output: Some(orig.clone()),
            overwrite: true,
        },
        &settings,
    )
    .expect("build CARRIED.dwg");

    // Copy the carried-ID/name line; verify the clone does NOT inherit the old
    // ID or name even after a full DWG round-trip (raw_dwg_eed stripped!).
    let reply = revise_with(
        &ReviseRequest {
            set: orig,
            change_set: json!({
                "operations": [
                    {"op": "copy", "target": {"id": "CARRIED/my_line"}, "by": [0, 20, 0]}
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
    assert!(reply.ok, "copy failed: {reply:?}");
    let rep = reply.result.unwrap();
    let rev_dwg = PathBuf::from(rep.revision_dwg.unwrap());
    let copy_handle = rep.operations[0].copies[0].handle.clone();
    assert_eq!(
        dwg::carried_id_and_name_of(&rev_dwg, &copy_handle),
        (None, None),
        "copy must not inherit carried id or name from raw_dwg_eed"
    );

    // Targeting a block-definition entity handle must be refused cleanly.
    let with_blk = build_made_up_original(&dir, "BLKREF");
    let blk_ent_hex =
        dwg::block_definition_entity_handle(&with_blk).expect("INSTR block has a circle entity");
    let before = dir_files_no_journal(&dir);
    let refused = revise_with(
        &ReviseRequest {
            set: with_blk,
            change_set: json!({
                "operations": [
                    {"op": "delete", "target": {"id": format!("BLKREF/handle:{blk_ent_hex}")}}
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
    assert!(
        !refused.ok,
        "block-definition entity handle must be refused"
    );
    assert!(
        matches!(
            refused.error.as_ref().unwrap().code,
            ErrorCode::UnknownName | ErrorCode::BadValue
        ),
        "{:?}",
        refused.error
    );
    assert_eq!(dir_files_no_journal(&dir), before);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn mirror_asymmetric_insert_and_move_neg_z_arc_roundtrip_accurately() {
    let dir = scratch("mirror_and_ocs");
    let blocks = dir.join("asym_blocks.yaml");
    fs::write(
        &blocks,
        "units: mm\n\
         blocks:\n\
         \x20\x20ARROW:\n\
         \x20\x20\x20\x20base_point: [0, 0, 0]\n\
         \x20\x20\x20\x20objects:\n\
         \x20\x20\x20\x20\x20\x20- line: {from: [0, 0, 0], to: [10, 0, 0]}\n\
         \x20\x20\x20\x20\x20\x20- line: {from: [10, 0, 0], to: [10, 4, 0]}\n",
    )
    .unwrap();
    let yaml = dir.join("ASYM.yaml");
    fs::write(
        &yaml,
        "units: mm\n\
         block_library: asym_blocks.yaml\n\
         objects:\n\
         \x20\x20- insert: {block: ARROW, position: [20, 10, 0], rotation: 30}\n",
    )
    .unwrap();
    let built = dir.join("ASYM_built.dwg");
    let settings = Settings::default().without_system_fonts();
    build_with(
        &BuildRequest {
            description: yaml,
            output: Some(built.clone()),
            overwrite: true,
        },
        &settings,
    )
    .unwrap();
    let orig = dir.join("ASYM.dwg");
    dwg::as_someone_elses(&built, &orig);
    let _ = fs::remove_file(&built);

    let loaded = crate::original::read_dwg_record(&orig, false).unwrap();
    let ins_id = loaded.record.objects[0].id.clone();

    let reply = revise_with(
        &ReviseRequest {
            set: orig,
            change_set: json!({
                "operations": [
                    {
                        "op": "mirror",
                        "target": {"id": ins_id},
                        "first_point": [0, 0, 0],
                        "second_point": [100, 0, 0],
                        "erase_source": true
                    }
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
    assert!(reply.ok, "mirror insert failed: {reply:?}");
    let rev_dwg = PathBuf::from(reply.result.unwrap().revision_dwg.unwrap());
    let after = crate::original::read_dwg_record(&rev_dwg, false).unwrap();
    match &after.record.sheet.objects[0] {
        Object::Insert(ins) => {
            assert!((ins.position.x - 20.0).abs() < 1e-6);
            assert!((ins.position.y - -10.0).abs() < 1e-6);
            assert!(ins.mirrored, "mirrored insert must have mirrored=true");
            assert!((ins.scale - 1.0).abs() < 1e-6, "scale={}", ins.scale);
            // Reflecting 30° across 0° (X axis) with negated X scale yields
            // (2*0 - 30 + 180) mod 360 = 150°.
            assert!(
                (ins.rotation - 150.0).abs() < 1e-6,
                "expected 150 deg, got {}",
                ins.rotation
            );
        }
        other => panic!("expected Insert, got {other:?}"),
    }

    // Moving an arc with -Z normal translates its world center accurately.
    let base = build_made_up_original(&dir, "BASE");
    let arc_orig = dir.join("NEGZ.dwg");
    let arc_hex = dwg::with_neg_z_arc_original(&base, &arc_orig);
    let (wcs_before, norm_before) = dwg::arc_world_center_and_normal(&arc_orig, &arc_hex).unwrap();
    assert!((wcs_before[0] - -10.0).abs() < 1e-6);
    assert!((wcs_before[1] - 20.0).abs() < 1e-6);
    assert!((norm_before[2] - -1.0).abs() < 1e-6);

    let arc_reply = revise_with(
        &ReviseRequest {
            set: arc_orig,
            change_set: json!({
                "operations": [
                    {"op": "move", "target": {"id": format!("NEGZ/handle:{arc_hex}")}, "by": [5, 10, 0]}
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
    assert!(arc_reply.ok, "move -Z arc failed: {arc_reply:?}");
    let arc_rev = PathBuf::from(arc_reply.result.unwrap().revision_dwg.unwrap());
    let (wcs_after, norm_after) = dwg::arc_world_center_and_normal(&arc_rev, &arc_hex).unwrap();
    assert!(
        (wcs_after[0] - -5.0).abs() < 1e-6 && (wcs_after[1] - 30.0).abs() < 1e-6,
        "expected WCS (-5, 30, 0), got {wcs_after:?}"
    );
    assert!((norm_after[2] - -1.0).abs() < 1e-6);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn constant_and_locked_layer_attributes_and_locked_copies_refused() {
    let dir = scratch("attr_and_copy_locks");
    let base = build_made_up_original(&dir, "BASE");
    let orig = dir.join("ATTLOCK.dwg");
    let ins_hex = dwg::with_constant_and_locked_attrs(&base, &orig);
    let ins_id = format!("ATTLOCK/handle:{ins_hex}");
    let settings = Settings::default().without_system_fonts();

    // Occurrence 1 skips the constant attribute and hits the locked-layer attribute -> Locked!
    let r_locked = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: json!({
                "operations": [
                    {"op": "set_attribute", "target": {"id": ins_id}, "tag": "TAG", "occurrence": 1, "value": "NO"}
                ]
            }),
            dry_run: true,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(!r_locked.ok);
    assert_eq!(r_locked.error.unwrap().code, ErrorCode::Locked);

    // Occurrence 2 hits the unlocked editable attribute -> succeeds!
    let r_ok = revise_with(
        &ReviseRequest {
            set: orig,
            change_set: json!({
                "operations": [
                    {"op": "set_attribute", "target": {"id": ins_id}, "tag": "TAG", "occurrence": 2, "value": "UPDATED"}
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
    assert!(r_ok.ok, "occurrence 1 should succeed: {r_ok:?}");

    // Copy, array, and keep-source mirror on a locked-layer object must all be refused.
    let loaded = crate::original::read_dwg_record(&base, false).unwrap();
    let locked_id = loaded.record.objects[7].id.clone();
    for cs in [
        json!({"operations": [{"op": "copy", "target": {"id": locked_id}, "by": [10, 0, 0]}]}),
        json!({"operations": [{"op": "array", "kind": "rectangular", "target": {"id": locked_id}, "rows": 1, "columns": 2, "row_spacing": 10, "column_spacing": 10}]}),
        json!({"operations": [{"op": "mirror", "target": {"id": locked_id}, "first_point": [0, 0, 0], "second_point": [10, 0, 0], "erase_source": false}]}),
    ] {
        let r = revise_with(
            &ReviseRequest {
                set: base.clone(),
                change_set: cs.clone(),
                dry_run: true,
                output: None,
                allow_loss: false,
                plot_style: None,
                no_pdf: true,
            },
            &settings,
        );
        assert!(!r.ok, "expected locked refusal for {cs}");
        assert_eq!(r.error.unwrap().code, ErrorCode::Locked);
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn duplicate_transform_and_copy_targets_refused_atomically() {
    let dir = scratch("dup_targets");
    let yaml = dir.join("DUPT.yaml");
    fs::write(
        &yaml,
        "units: mm\n\
         objects:\n\
         \x20\x20- line: {id: l1, name: dup_line, from: [0, 0, 0], to: [10, 0, 0]}\n",
    )
    .unwrap();
    let orig = dir.join("DUPT.dwg");
    let settings = Settings::default().without_system_fonts();
    build_with(
        &BuildRequest {
            description: yaml,
            output: Some(orig.clone()),
            overwrite: true,
        },
        &settings,
    )
    .unwrap();
    let before = dir_files(&dir);

    for cs in [
        json!({
            "operations": [{
                "op": "rotate",
                "targets": [{"id": "DUPT/l1"}, {"name": "dup_line"}],
                "base_point": [0, 0, 0],
                "angle": 90
            }]
        }),
        json!({
            "operations": [{
                "op": "copy",
                "targets": [{"id": "DUPT/l1"}, {"name": "dup_line"}],
                "by": [5, 0, 0]
            }]
        }),
    ] {
        let r = revise_with(
            &ReviseRequest {
                set: orig.clone(),
                change_set: cs,
                dry_run: false,
                output: None,
                allow_loss: false,
                plot_style: None,
                no_pdf: true,
            },
            &settings,
        );
        assert!(!r.ok, "duplicate targets must be refused");
        assert_eq!(r.error.unwrap().code, ErrorCode::BadValue);
    }
    assert_eq!(dir_files_no_journal(&dir), before);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn name_only_add_and_rename_keep_handle_id_and_unicode_id_does_not_panic() {
    let dir = scratch("name_and_unicode");
    let orig = build_made_up_original(&dir, "NAME-1");
    let loaded = crate::original::read_dwg_record(&orig, false).unwrap();
    let orig_line_id = loaded.record.objects[0].id.clone();
    let orig_line_hex = loaded.record.dwg.as_ref().unwrap().handles[0].clone();
    let settings = Settings::default().without_system_fonts();

    // 1. Multi-byte UTF-8 ID whose byte slice at HANDLE_ID.len() is mid-char must
    //    return UnknownName cleanly without panicking!
    let uni = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: json!({
                "operations": [{"op": "delete", "target": {"id": "NAME-1/ééééé"}}]
            }),
            dry_run: true,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(!uni.ok);
    assert_eq!(uni.error.unwrap().code, ErrorCode::UnknownName);

    // 2. Add a name-only line, snap to it in the next op, and rename an existing
    //    handle-ID object; neither should get an empty carried ID.
    let reply = revise_with(
        &ReviseRequest {
            set: orig,
            change_set: json!({
                "operations": [
                    {"op": "add_line", "name": "guide_line", "from": [0, 10, 0], "to": [20, 10, 0]},
                    {
                        "op": "add_line",
                        "from": {"snap": {"name": "guide_line", "snap": "endpoint", "which": "to"}},
                        "to": [20, 30, 0]
                    },
                    {"op": "change", "target": {"id": orig_line_id}, "set": {"name": "renamed_orig"}}
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
    assert!(reply.ok, "name-only add + snap + rename failed: {reply:?}");
    let rep = reply.result.unwrap();
    let rev_dwg = PathBuf::from(rep.revision_dwg.unwrap());
    let added_hex = rep.operations[0].handle.as_ref().unwrap();
    assert_eq!(
        dwg::carried_id_and_name_of(&rev_dwg, added_hex),
        (None, Some("guide_line".to_owned()))
    );
    assert_eq!(
        dwg::carried_id_and_name_of(&rev_dwg, &orig_line_hex),
        (None, Some("renamed_orig".to_owned()))
    );
    let after = crate::original::read_dwg_record(&rev_dwg, false).unwrap();
    let found_guide = after
        .record
        .objects
        .iter()
        .find(|o| o.name.as_deref() == Some("guide_line"))
        .expect("guide_line present");
    assert!(
        found_guide.id.contains("/handle:"),
        "expected stable handle ID, got {}",
        found_guide.id
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn schema_accepts_original_change_set_and_set_dispatch_refuses_null_or_false_original_keys() {
    let dir = scratch("schema_and_keys");
    let change_cmd = find_command("change").unwrap();
    let schema = change_cmd.input_schema();
    let validator = jsonschema::validator_for(&schema).expect("valid change input schema");

    // Original-DWG change set omitting `sheet` and passing explicit null `output`/`plot_style`
    // must pass JSON Schema validation.
    let valid_original = json!({
        "set": "drawing.dwg",
        "change_set": {
            "operations": [
                {"op": "add_line", "from": [0, 0, 0], "to": [10, 0, 0]},
                {"op": "add_layer", "name": "NEW_LAYER", "color": 2}
            ]
        },
        "output": null,
        "plot_style": null
    });
    assert!(
        validator.is_valid(&valid_original),
        "schema rejected valid original-DWG change request: {:?}",
        validator.iter_errors(&valid_original).collect::<Vec<_>>()
    );

    // Drawing-set dispatch must refuse even null/false original-only keys!
    let set_dir = dir.join("SET");
    fs::create_dir_all(&set_dir).unwrap();
    fs::write(
        set_dir.join("set.yaml"),
        "units: mm\noutput: dwg\nrows: rows.yaml\n",
    )
    .unwrap();
    fs::write(
        set_dir.join("tpl.yaml"),
        "units: mm\nobjects:\n  - line: {id: base, from: [0, 0, 0], to: [10, 0, 0]}\n",
    )
    .unwrap();
    fs::write(
        set_dir.join("rows.yaml"),
        "rows:\n  - sheet: S-1\n    template: tpl.yaml\n",
    )
    .unwrap();
    let settings = Settings::default().without_system_fonts();
    for (key, val) in [
        ("output", json!(false)),
        ("output", json!(null)),
        ("plot_styles", json!(null)),
        ("allow_loss", json!(false)),
        ("no_pdf", json!(false)),
        ("plot_style", json!(null)),
    ] {
        let mut input = json!({
            "set": set_dir.display().to_string(),
            "change_set": {"operations": [{"op": "add_line", "sheet": "S-1", "from": [0, 1, 0], "to": [5, 1, 0]}]},
            "dry_run": true
        });
        input[key] = val.clone();
        let reply = change_cmd.run(input, &settings, Interface::Cli);
        assert!(
            !reply.ok,
            "drawing-set change must refuse `{key}: {val}` with Usage"
        );
        assert_eq!(reply.error.unwrap().code, ErrorCode::Usage);
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn array_path_refuses_non_curve_insert() {
    let dir = scratch("array_path_insert");
    let orig = build_made_up_original(&dir, "PATH-1");
    let loaded = crate::original::read_dwg_record(&orig, false).unwrap();
    let line_id = loaded.record.objects[0].id.clone();
    let insert_id = loaded.record.objects[6].id.clone();
    let settings = Settings::default().without_system_fonts();

    let reply = revise_with(
        &ReviseRequest {
            set: orig,
            change_set: json!({
                "operations": [{
                    "op": "array",
                    "kind": "path",
                    "target": {"id": line_id},
                    "path": {"id": insert_id},
                    "count": 3
                }]
            }),
            dry_run: true,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(!reply.ok, "ARRAYPATH must refuse an Insert as path");
    assert_eq!(reply.error.unwrap().code, ErrorCode::BadValue);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn allow_loss_never_publishes_missing_requested_add_or_surviving_delete() {
    let dir = scratch("allow_loss_requested_gate");
    let source = dir.join("GATE.dwg");
    let orig_meta = RevisionFile {
        file: "GATE.dwg".to_owned(),
        sha256: "0".repeat(64),
        dwg_version: "AC1032".to_owned(),
    };
    let decode = DecodeReport {
        decoded: true,
        not_decoded: Vec::new(),
        skipped: 0,
        recovered_errors: 0,
        warnings: 0,
        diagnostics_truncated: false,
        read_past_damage: None,
    };
    let verified = RevisionVerification {
        bytes: Vec::new(),
        requested_failures: vec![
            UnexpectedChange {
                kind: "ADDED_OBJECT".to_owned(),
                handle: "DEAD".to_owned(),
                change: crate::dwg::UnexpectedChangeKind::Removed,
                detail: "requested added object was missing after writing the revision".to_owned(),
            },
            UnexpectedChange {
                kind: "LINE".to_owned(),
                handle: "BEEF".to_owned(),
                change: crate::dwg::UnexpectedChangeKind::Added,
                detail: "requested deleted LINE was still present in the revision after writing"
                    .to_owned(),
            },
        ],
        unexpected_changes: vec![UnexpectedChange {
            kind: "DICTIONARY".to_owned(),
            handle: "42".to_owned(),
            change: crate::dwg::UnexpectedChangeKind::Changed,
            detail: "untouched non-graphical DICTIONARY changed in the revision".to_owned(),
        }],
    };

    // Even with `allow_loss = true`, requested-edit failures MUST be refused,
    // and `requested_failures` stays distinct from `unexpected_changes`.
    let (report, err) = enforce_verification_gates(
        &source,
        "GATE",
        false,
        true,
        &orig_meta,
        Some(Unit::Mm),
        &decode,
        &[],
        &verified,
    )
    .expect("allow_loss must not waive missing adds or surviving deletes");
    assert_eq!(err.code, ErrorCode::DwgWrite);
    assert!(
        !err.message.contains("--allow-loss"),
        "error should not suggest --allow-loss can bypass requested-edit failures: {}",
        err.message
    );
    assert_eq!(report.requested_failures.len(), 2);
    assert_eq!(report.unexpected_changes.len(), 1);
    assert_eq!(report.unexpected_changes[0].handle, "42");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn mirror_asymmetric_minsert_across_x_and_y_axes_mirrors_every_grid_instance() {
    let dir = scratch("mirror_minsert");
    let base = build_made_up_original(&dir, "BASE");
    let orig = dir.join("MINS.dwg");
    let hex = dwg::with_asymmetric_minsert_original(&base, &orig);
    let before = dwg::minsert_grid_positions_of(&orig, &hex).expect("MINSERT grid before");
    assert_eq!(before.len(), 4);
    let settings = Settings::default().without_system_fonts();

    // 1. Mirror across X axis: (x, y) -> (x, -y) for all 4 grid instances.
    let rx = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: json!({
                "operations": [{
                    "op": "mirror",
                    "target": {"id": format!("MINS/handle:{hex}")},
                    "first_point": [0, 0, 0],
                    "second_point": [100, 0, 0],
                    "erase_source": true
                }]
            }),
            dry_run: false,
            output: Some(dir.join("MINS.mx.dwg")),
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(rx.ok, "mirror MINSERT across X axis failed: {rx:?}");
    let after_x =
        dwg::minsert_grid_positions_of(&dir.join("MINS.mx.dwg"), &hex).expect("grid after X");
    for (b, a) in before.iter().zip(after_x.iter()) {
        assert!(
            (a[0] - b[0]).abs() < 1e-6 && (a[1] - -b[1]).abs() < 1e-6,
            "X mirror mismatch: before={b:?}, after={a:?}"
        );
    }

    // 2. Mirror across Y axis: (x, y) -> (-x, y) for all 4 grid instances.
    let ry = revise_with(
        &ReviseRequest {
            set: orig,
            change_set: json!({
                "operations": [{
                    "op": "mirror",
                    "target": {"id": format!("MINS/handle:{hex}")},
                    "first_point": [0, 0, 0],
                    "second_point": [0, 100, 0],
                    "erase_source": true
                }]
            }),
            dry_run: false,
            output: Some(dir.join("MINS.my.dwg")),
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(ry.ok, "mirror MINSERT across Y axis failed: {ry:?}");
    let after_y =
        dwg::minsert_grid_positions_of(&dir.join("MINS.my.dwg"), &hex).expect("grid after Y");
    for (b, a) in before.iter().zip(after_y.iter()) {
        assert!(
            (a[0] - -b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6,
            "Y mirror mismatch: before={b:?}, after={a:?}"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn unlocked_insert_with_locked_attribute_refuses_move_transform_copy_and_delete() {
    let dir = scratch("locked_child_attr");
    let base = build_made_up_original(&dir, "BASE");
    let orig = dir.join("CHILDLOCK.dwg");
    let ins_hex = dwg::with_constant_and_locked_attrs(&base, &orig);
    let ins_id = format!("CHILDLOCK/handle:{ins_hex}");
    let before = dir_files(&dir);
    let settings = Settings::default().without_system_fonts();

    for cs in [
        json!({"operations": [{"op": "move", "target": {"id": ins_id}, "by": [10, 0, 0]}]}),
        json!({"operations": [{"op": "rotate", "target": {"id": ins_id}, "base_point": [0, 0, 0], "angle": 90}]}),
        json!({"operations": [{"op": "scale", "target": {"id": ins_id}, "base_point": [0, 0, 0], "factor": 2}]}),
        json!({"operations": [{"op": "mirror", "target": {"id": ins_id}, "first_point": [0, 0, 0], "second_point": [10, 0, 0], "erase_source": true}]}),
        json!({"operations": [{"op": "mirror", "target": {"id": ins_id}, "first_point": [0, 0, 0], "second_point": [10, 0, 0], "erase_source": false}]}),
        json!({"operations": [{"op": "copy", "target": {"id": ins_id}, "by": [10, 0, 0]}]}),
        json!({"operations": [{"op": "delete", "target": {"id": ins_id}}]}),
        json!({"operations": [{"op": "change", "target": {"id": ins_id}, "set": {"position": [60, 60, 0]}}]}),
    ] {
        let r = revise_with(
            &ReviseRequest {
                set: orig.clone(),
                change_set: cs.clone(),
                dry_run: false,
                output: None,
                allow_loss: false,
                plot_style: None,
                no_pdf: true,
            },
            &settings,
        );
        assert!(!r.ok, "insert with locked attribute must refuse {cs}");
        assert_eq!(
            r.error.as_ref().unwrap().code,
            ErrorCode::Locked,
            "expected Locked for {cs}, got {:?}",
            r.error
        );
    }
    assert_eq!(
        dir_files_no_journal(&dir),
        before,
        "no revision files may be written"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn leading_zero_handle_alias_is_deduplicated_in_transform_and_copy() {
    let dir = scratch("zero_pad_dup");
    let orig = build_made_up_original(&dir, "ZPAD");
    let loaded = crate::original::read_dwg_record(&orig, false).unwrap();
    let hex = loaded.record.dwg.as_ref().unwrap().handles[0].clone();
    let id_plain = format!("ZPAD/handle:{hex}");
    let id_padded = format!("ZPAD/handle:0{hex}");
    let id_spaced = format!("ZPAD/handle: 0{hex}");
    let before = dir_files(&dir);
    let settings = Settings::default().without_system_fonts();

    for cs in [
        json!({
            "operations": [{
                "op": "rotate",
                "targets": [{"id": id_plain}, {"id": id_padded}],
                "base_point": [0, 0, 0],
                "angle": 45
            }]
        }),
        json!({
            "operations": [{
                "op": "copy",
                "targets": [{"id": id_plain}, {"id": id_padded}],
                "by": [10, 0, 0]
            }]
        }),
        json!({
            "operations": [{
                "op": "rotate",
                "targets": [{"id": id_plain}, {"id": id_spaced}],
                "base_point": [0, 0, 0],
                "angle": 45
            }]
        }),
    ] {
        let r = revise_with(
            &ReviseRequest {
                set: orig.clone(),
                change_set: cs.clone(),
                dry_run: false,
                output: None,
                allow_loss: false,
                plot_style: None,
                no_pdf: true,
            },
            &settings,
        );
        assert!(
            !r.ok,
            "leading-zero or spaced handle duplicate must be refused for {cs}"
        );
        assert_eq!(r.error.unwrap().code, ErrorCode::BadValue);
    }
    let single = revise_with(
        &ReviseRequest {
            set: orig,
            change_set: json!({
                "operations": [{"op": "move", "target": {"id": id_spaced}, "by": [1, 0, 0]}]
            }),
            dry_run: true,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(
        single.ok,
        "whitespace-padded handle should resolve: {single:?}"
    );
    assert_eq!(
        dir_files_no_journal(&dir),
        before,
        "zero revision files may be written"
    );

    let _ = fs::remove_dir_all(&dir);
}
#[test]
fn scale_minsert_overflow_refused_with_bad_value() {
    let dir = scratch("scale_minsert_overflow");
    let base = build_made_up_original(&dir, "BASE");
    let orig = dir.join("OVF.dwg");
    let hex = dwg::with_asymmetric_minsert_original(&base, &orig);
    let before = dir_files(&dir);
    let settings = Settings::default().without_system_fonts();

    let reply = revise_with(
        &ReviseRequest {
            set: orig,
            change_set: json!({
                "operations": [{
                    "op": "scale",
                    "target": {"id": format!("OVF/handle:{hex}")},
                    "base_point": [0, 0, 0],
                    "factor": 1e308
                }]
            }),
            dry_run: false,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(
        !reply.ok,
        "scaling MINSERT spacing past f64::MAX must be refused"
    );
    assert_eq!(reply.error.unwrap().code, ErrorCode::BadValue);
    assert_eq!(
        dir_files_no_journal(&dir),
        before,
        "zero revision files may be written"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ── #71 Part 2: per-original journal, crash recovery, undo, accept & stamps ─

#[test]
fn original_journal_per_file_isolation_empty_read_and_safe_key() {
    use crate::change::original_journal::test_original_key;
    use crate::change::{accept_with, journal, AcceptRequest, JournalRequest};

    // Collision-free safe-stem keys across distinct names, punctuation, case,
    // and Windows reserved device names.
    let k1 = test_original_key(Path::new("P-101.dwg"));
    let k2 = test_original_key(Path::new("P-102.dwg"));
    let k_lower = test_original_key(Path::new("p-101.dwg"));
    let k_space = test_original_key(Path::new("A B.dwg"));
    let k_under = test_original_key(Path::new("A_B.dwg"));
    let k_con = test_original_key(Path::new("CON.dwg"));
    assert_ne!(k1, k2);
    assert_ne!(k1, k_lower);
    assert_ne!(k_space, k_under);
    assert!(k_con.starts_with("con-") && k_con.len() > 4);

    let dir = scratch("orig_journal_isolation");
    let p101 = build_made_up_original(&dir, "P-101");
    let p102 = build_made_up_original(&dir, "P-102");
    let before = dir_files(&dir);
    let settings = Settings::default().without_system_fonts();

    // `journal` on an original with no journal yet returns an empty report
    // without creating `.agentdrafter/`.
    let empty = journal(&JournalRequest { set: p101.clone() });
    assert!(empty.ok, "{empty:?}");
    let report = empty.result.unwrap();
    assert!(report.entries.is_empty());
    assert!(report.can_undo.is_empty());
    assert_eq!(report.kept_undo_states, crate::KEPT_UNDO_STATES);
    assert_eq!(
        dir_files(&dir),
        before,
        "journal on unrevised original must not create .agentdrafter"
    );

    // `accept` on an unrevised original returns NothingToAccept without
    // creating `.agentdrafter/`.
    let no_acc = accept_with(
        &AcceptRequest {
            set: p101.clone(),
            despite_failed_checks: false,
            newest_seen: None,
            dry_run: false,
        },
        &settings,
        Interface::Cli,
    );
    assert!(!no_acc.ok);
    assert_eq!(no_acc.error.unwrap().code, ErrorCode::NothingToAccept);
    assert_eq!(dir_files(&dir), before);

    // Revise both originals in the same folder: each gets its own distinct journal.
    let cs = json!({"operations": [{"op": "add_line", "from": [1, 1, 0], "to": [10, 1, 0]}]});
    let r1 = revise_with(
        &ReviseRequest {
            set: p101.clone(),
            change_set: cs.clone(),
            dry_run: false,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    let r2 = revise_with(
        &ReviseRequest {
            set: p102.clone(),
            change_set: cs,
            dry_run: false,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(r1.ok && r2.ok);
    let j1 = journal(&JournalRequest { set: p101 }).result.unwrap();
    let j2 = journal(&JournalRequest { set: p102 }).result.unwrap();
    assert_ne!(
        j1.file, j2.file,
        "separate originals must have separate journals"
    );
    assert_eq!(j1.entries.len(), 1);
    assert_eq!(j2.entries.len(), 1);
    assert_eq!(j1.entries[0].recorded.landed_on, vec!["P-101"]);
    assert_eq!(j2.entries[0].recorded.landed_on, vec!["P-102"]);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn original_change_journal_stack_undo_and_dry_run() {
    use crate::change::{
        journal, undo, CommitState, EntryKind, JournalRequest, UndoAction, UndoRequest, UndoState,
    };

    let dir = scratch("orig_stack_undo");
    let orig = build_made_up_original(&dir, "LOOP-201");
    let orig_bytes = fs::read(&orig).unwrap();
    let settings = Settings::default().without_system_fonts();

    // Change 1 -> rev1.dwg, rev1.changes.json, rev1.pdf
    let r1 = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: json!({
                "operations": [{"op": "add_line", "from": [1, 1, 0], "to": [20, 1, 0]}]
            }),
            dry_run: false,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: false,
        },
        &settings,
    );
    assert!(r1.ok, "{r1:?}");
    let rep1 = r1.result.unwrap();
    assert_eq!(rep1.journal_entry, Some(1));
    let rev1_dwg = PathBuf::from(rep1.revision_dwg.as_ref().unwrap());
    let rev1_chg = PathBuf::from(rep1.change_list.as_ref().unwrap());
    let rev1_pdf = PathBuf::from(rep1.pdf.as_ref().unwrap());
    assert!(rev1_dwg.is_file() && rev1_chg.is_file() && rev1_pdf.is_file());

    // Change 2 -> rev2.dwg, rev2.changes.json, rev2.pdf
    let r2 = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: json!({
                "operations": [{"op": "add_circle", "center": [15, 15, 0], "radius": 3}]
            }),
            dry_run: false,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: false,
        },
        &settings,
    );
    assert!(r2.ok, "{r2:?}");
    let rep2 = r2.result.unwrap();
    assert_eq!(rep2.journal_entry, Some(2));
    let rev2_dwg = PathBuf::from(rep2.revision_dwg.as_ref().unwrap());
    let rev2_chg = PathBuf::from(rep2.change_list.as_ref().unwrap());
    let rev2_pdf = PathBuf::from(rep2.pdf.as_ref().unwrap());
    assert!(rev2_dwg.is_file() && rev2_chg.is_file() && rev2_pdf.is_file());

    // Journal shows both change sets undoable with revision metadata.
    let j = journal(&JournalRequest { set: orig.clone() })
        .result
        .unwrap();
    assert_eq!(j.entries.len(), 2);
    assert_eq!(j.can_undo, vec![1, 2]);
    assert_eq!(j.entries[0].recorded.state, Some(CommitState::Done));
    assert_eq!(j.entries[0].undo_state, Some(UndoState::Kept));
    assert!(j.entries[0].recorded.original_revision.is_some());
    assert!(!j.entries[0].recorded.objects.is_empty());

    // Undo --dry-run on entry 1 reports both 2 and 1, removes nothing.
    let dry = undo(&UndoRequest {
        set: orig.clone(),
        change_set: 1,
        dry_run: true,
    });
    assert!(dry.ok, "{dry:?}");
    let dry_rep = dry.result.unwrap();
    assert!(dry_rep.dry_run);
    assert!(dry_rep.journal_entry.is_none());
    assert_eq!(dry_rep.undid, vec![2, 1]);
    assert_eq!(dry_rep.files.len(), 6);
    assert!(dry_rep
        .files
        .iter()
        .all(|f| f.action == UndoAction::Removed));
    assert!(rev1_dwg.is_file() && rev2_dwg.is_file());

    // Real stack undo of entry 1 removes rev2 and rev1 completely, leaving original untouched.
    let u = undo(&UndoRequest {
        set: orig.clone(),
        change_set: 1,
        dry_run: false,
    });
    assert!(u.ok, "{u:?}");
    let u_rep = u.result.unwrap();
    assert!(!u_rep.dry_run);
    assert_eq!(u_rep.journal_entry, Some(3));
    assert_eq!(u_rep.undid, vec![2, 1]);
    assert_eq!(u_rep.files.len(), 6);
    for p in [
        &rev1_dwg, &rev1_chg, &rev1_pdf, &rev2_dwg, &rev2_chg, &rev2_pdf,
    ] {
        assert!(!p.exists(), "undo must remove {}", p.display());
    }
    assert_eq!(fs::read(&orig).unwrap(), orig_bytes, "original untouched");

    // Journal records undone_by = Some(3), undo_state = Undone, can_undo = [].
    let j_after = journal(&JournalRequest { set: orig.clone() })
        .result
        .unwrap();
    assert_eq!(j_after.entries.len(), 3);
    assert_eq!(j_after.can_undo, Vec::<u64>::new());
    assert_eq!(j_after.entries[0].undo_state, Some(UndoState::Undone));
    assert_eq!(j_after.entries[0].undone_by, Some(3));
    assert_eq!(j_after.entries[1].undo_state, Some(UndoState::Undone));
    assert_eq!(j_after.entries[1].undone_by, Some(3));
    assert_eq!(j_after.entries[2].recorded.kind, EntryKind::Undo);
    assert_eq!(j_after.entries[2].recorded.undid, vec![2, 1]);

    // Second undo of entry 1 refuses with BadValue.
    let again = undo(&UndoRequest {
        set: orig,
        change_set: 1,
        dry_run: false,
    });
    assert!(!again.ok);
    assert_eq!(again.error.unwrap().code, ErrorCode::BadValue);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn original_undo_refuses_hand_edited_missing_symlink_and_original_file() {
    use crate::change::original_journal::test_pending_journal_path;
    use crate::change::{undo, UndoRequest};

    let dir = scratch("orig_undo_safety");
    let orig = build_made_up_original(&dir, "SAFE-UNDO");
    let orig_bytes = fs::read(&orig).unwrap();
    let settings = Settings::default().without_system_fonts();

    let r1 = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: json!({
                "operations": [{"op": "add_line", "from": [1, 1, 0], "to": [10, 1, 0]}]
            }),
            dry_run: false,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: false,
        },
        &settings,
    );
    assert!(r1.ok, "{r1:?}");
    let rep = r1.result.unwrap();
    let rev_dwg = PathBuf::from(rep.revision_dwg.unwrap());
    let rev_chg = PathBuf::from(rep.change_list.unwrap());
    let rev_pdf = PathBuf::from(rep.pdf.unwrap());
    let rev_dwg_bytes = fs::read(&rev_dwg).unwrap();
    let rev_chg_bytes = fs::read(&rev_chg).unwrap();
    let rev_pdf_bytes = fs::read(&rev_pdf).unwrap();

    // 1. Hand-edited revision DWG -> refused with ChangedByHand, nothing deleted.
    fs::write(&rev_dwg, b"hand-edited").unwrap();
    let u = undo(&UndoRequest {
        set: orig.clone(),
        change_set: 1,
        dry_run: false,
    });
    assert!(!u.ok);
    assert_eq!(u.error.as_ref().unwrap().code, ErrorCode::ChangedByHand);
    assert!(u.error.unwrap().message.contains("SAFE-UNDO.rev1.dwg"));
    assert_eq!(fs::read(&rev_chg).unwrap(), rev_chg_bytes);
    assert_eq!(fs::read(&rev_pdf).unwrap(), rev_pdf_bytes);
    fs::write(&rev_dwg, &rev_dwg_bytes).unwrap();

    // 2. Deleted revision change list -> refused with ChangedByHand, nothing deleted.
    fs::remove_file(&rev_chg).unwrap();
    let u2 = undo(&UndoRequest {
        set: orig.clone(),
        change_set: 1,
        dry_run: false,
    });
    assert!(!u2.ok);
    assert_eq!(u2.error.as_ref().unwrap().code, ErrorCode::ChangedByHand);
    assert_eq!(fs::read(&rev_dwg).unwrap(), rev_dwg_bytes);
    assert_eq!(fs::read(&rev_pdf).unwrap(), rev_pdf_bytes);
    fs::write(&rev_chg, &rev_chg_bytes).unwrap();

    // 3. Revision PDF replaced with a symlink -> refused without following/removing symlink.
    #[cfg(unix)]
    {
        let decoy = dir.join("decoy.pdf");
        fs::write(&decoy, &rev_pdf_bytes).unwrap();
        fs::remove_file(&rev_pdf).unwrap();
        std::os::unix::fs::symlink(&decoy, &rev_pdf).unwrap();
        let u3 = undo(&UndoRequest {
            set: orig.clone(),
            change_set: 1,
            dry_run: false,
        });
        assert!(!u3.ok);
        assert_eq!(u3.error.unwrap().code, ErrorCode::ChangedByHand);
        assert!(
            fs::symlink_metadata(&rev_pdf)
                .unwrap()
                .file_type()
                .is_symlink(),
            "symlink must never be unlinked"
        );
        assert!(decoy.is_file());
        fs::remove_file(&rev_pdf).unwrap();
        fs::write(&rev_pdf, &rev_pdf_bytes).unwrap();
    }

    // 4. Tampered journal entry naming the original DWG itself -> refused and
    //    original DWG is NEVER modified or removed (ADR-0004).
    let jpath = test_pending_journal_path(&orig).unwrap();
    let tampered = fs::read_to_string(&jpath)
        .unwrap()
        .replace("SAFE-UNDO.rev1.dwg", "SAFE-UNDO.dwg");
    fs::write(&jpath, tampered).unwrap();
    let u4 = undo(&UndoRequest {
        set: orig.clone(),
        change_set: 1,
        dry_run: false,
    });
    assert!(!u4.ok);
    assert_eq!(u4.error.unwrap().code, ErrorCode::BadValue);
    assert_eq!(fs::read(&orig).unwrap(), orig_bytes, "original untouched");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn original_failed_attempt_and_dry_run_refuses_pending() {
    use crate::change::original_journal::test_pending_journal_path;
    use crate::change::{journal, EntryKind, JournalRequest};

    let dir = scratch("orig_failed_and_dry_pending");
    let orig = build_made_up_original(&dir, "FAIL-1");
    let settings = Settings::default().without_system_fonts();

    // Non-dry-run bad operation records a failed_attempt in the original's journal.
    let bad = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: json!({
                "operations": [{"op": "delete", "target": {"id": "FAIL-1/no-such-id"}}]
            }),
            dry_run: false,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(!bad.ok);
    assert_eq!(bad.error.unwrap().code, ErrorCode::UnknownName);

    let j = journal(&JournalRequest { set: orig.clone() })
        .result
        .unwrap();
    assert_eq!(j.entries.len(), 1);
    assert_eq!(j.entries[0].recorded.kind, EntryKind::FailedAttempt);
    assert!(j.entries[0].recorded.error.is_some());
    assert!(j.can_undo.is_empty());

    // Append an unfinished pending entry directly and verify `--dry-run`
    // refuses cleanly without modifying or rolling back the journal.
    let jpath = test_pending_journal_path(&orig).unwrap();
    let mut text = fs::read_to_string(&jpath).unwrap();
    text.push_str(
        "{\"entry\":2,\"kind\":\"change_set\",\"state\":\"pending\",\"at\":\"2026-11-22T00:00:00Z\",\"files\":[]}\n",
    );
    fs::write(&jpath, &text).unwrap();

    let dry = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: json!({
                "operations": [{"op": "add_line", "from": [0, 0, 0], "to": [5, 0, 0]}]
            }),
            dry_run: true,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(!dry.ok);
    assert_eq!(dry.error.unwrap().code, ErrorCode::RollbackFailed);
    assert_eq!(
        fs::read_to_string(&jpath).unwrap(),
        text,
        "dry-run must not mutate the pending journal"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn original_crash_recovery_rolls_back_pending_change_undo_accept_and_torn_line() {
    use crate::change::original_journal::test_pending_journal_path;
    use crate::change::{
        journal, undo, CommitState, EntryKind, JournalRequest, UndoAction, UndoRequest, STOP_AT,
    };

    let dir = scratch("orig_crash_recovery");
    let orig = build_made_up_original(&dir, "CRASH-1");
    let orig_bytes = fs::read(&orig).unwrap();
    let settings = Settings::default().without_system_fonts();
    let cs = json!({"operations": [{"op": "add_line", "from": [1, 1, 0], "to": [10, 1, 0]}]});

    // 1. Kill `change` mid-commit after placing rev1.dwg (step 0=pending,
    //    1=stage dwg, 2=stage changes, 3=place dwg -> stop at 3!).
    STOP_AT.with(|s| s.set(Some(3)));
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        revise_with(
            &ReviseRequest {
                set: orig.clone(),
                change_set: cs.clone(),
                dry_run: false,
                output: None,
                allow_loss: false,
                plot_style: None,
                no_pdf: true,
            },
            &settings,
        )
    }));
    STOP_AT.with(|s| s.set(None));
    assert!(panicked.is_err(), "STOP_AT should unwind mid-commit");
    let rev1_dwg = dir.join("CRASH-1.rev1.dwg");
    assert!(
        rev1_dwg.is_file(),
        "partial rev1.dwg should be on disk before recovery"
    );

    // Next `journal` recovers the pending change_set, removes partial rev1.dwg,
    // reports `rolled_back`, and keeps original untouched.
    let j = journal(&JournalRequest { set: orig.clone() });
    assert!(j.ok, "{j:?}");
    let rb = j.rolled_back.expect("must report rolled_back");
    assert_eq!(rb.entry, Some(1));
    assert_eq!(rb.kind, Some(EntryKind::ChangeSet));
    assert_eq!(rb.files.len(), 1);
    assert_eq!(rb.files[0].action, UndoAction::Removed);
    assert!(!rev1_dwg.exists(), "partial rev1.dwg must be removed");
    assert_eq!(fs::read(&orig).unwrap(), orig_bytes, "original untouched");
    let j_rep = j.result.unwrap();
    assert_eq!(
        j_rep.entries[0].recorded.state,
        Some(CommitState::RolledBack)
    );
    assert!(j_rep.can_undo.is_empty());

    // 2. Apply a real revision WITH PDF (entry 2, lands as rev1.dwg +
    //    rev1.changes.json + rev1.pdf), then kill `undo` mid-commit after
    //    moving rev1.dwg aside, before `done`.
    let r = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: cs,
            dry_run: false,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: false,
        },
        &settings,
    );
    assert!(r.ok, "{r:?}");
    let rep = r.result.unwrap();
    assert_eq!(rep.journal_entry, Some(2));
    let rev1_bytes = fs::read(&rev1_dwg).unwrap();
    let rev1_chg = PathBuf::from(rep.change_list.unwrap());
    let rev1_chg_bytes = fs::read(&rev1_chg).unwrap();
    let rev1_pdf = PathBuf::from(rep.pdf.unwrap());
    let rev1_pdf_bytes = fs::read(&rev1_pdf).unwrap();

    // Undo has 3 undo states (stage: 1,2,3; place: 4,5,6) + 3 file removals
    // (move aside: 7=dwg, 8=changes, 9=pdf). Stop at step 9 right after ALL
    // THREE files are moved aside, before `done`, so crash recovery restores
    // all three outputs byte-for-byte from `undo-states/`!
    STOP_AT.with(|s| s.set(Some(9)));
    let undo_panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        undo(&UndoRequest {
            set: orig.clone(),
            change_set: 2,
            dry_run: false,
        })
    }));
    STOP_AT.with(|s| s.set(None));
    assert!(undo_panicked.is_err(), "STOP_AT should unwind mid-undo");
    assert!(
        !rev1_dwg.exists() && !rev1_chg.exists() && !rev1_pdf.exists(),
        "all three outputs must have been moved aside before crash recovery"
    );

    // Next `journal` rolls back the pending undo and restores ALL THREE outputs
    // (rev1.dwg, rev1.changes.json, rev1.pdf) byte-for-byte from undo-states/!
    let j2 = journal(&JournalRequest { set: orig.clone() });
    assert!(j2.ok, "{j2:?}");
    let rb2 = j2.rolled_back.expect("pending undo must be rolled back");
    assert_eq!(rb2.entry, Some(3));
    assert_eq!(rb2.kind, Some(EntryKind::Undo));
    assert_eq!(rb2.files.len(), 3, "must restore all 3 files: {rb2:?}");
    assert_eq!(fs::read(&rev1_dwg).unwrap(), rev1_bytes);
    assert_eq!(fs::read(&rev1_chg).unwrap(), rev1_chg_bytes);
    assert_eq!(fs::read(&rev1_pdf).unwrap(), rev1_pdf_bytes);
    assert_eq!(j2.result.unwrap().can_undo, vec![2]);

    // 3. Pending `accept` + torn last line -> rolled back and torn line cut off;
    //    entry 2 stays undoable.
    let jpath = test_pending_journal_path(&orig).unwrap();
    let mut text = fs::read_to_string(&jpath).unwrap();
    text.push_str(
        "{\"entry\":4,\"kind\":\"accept\",\"state\":\"pending\",\"at\":\"2026-11-22T00:00:00Z\",\"via\":\"cli\",\"issued\":[2]}\n",
    );
    text.push_str("{\"entry\":4,\"state\":\"do"); // torn line without newline
    fs::write(&jpath, text).unwrap();

    let j3 = journal(&JournalRequest { set: orig.clone() });
    assert!(j3.ok, "{j3:?}");
    let rb3 = j3
        .rolled_back
        .expect("pending accept + torn line recovered");
    assert_eq!(rb3.entry, Some(4));
    assert_eq!(rb3.kind, Some(EntryKind::Accept));
    assert!(rb3.cut_off_line.is_some());
    assert_eq!(j3.result.unwrap().can_undo, vec![2]);
    assert_eq!(fs::read(&orig).unwrap(), orig_bytes);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn original_accept_checks_newest_seen_blocks_undo_and_stamps_outputs_issued() {
    use crate::change::{accept_with, journal, undo, AcceptRequest, JournalRequest, UndoRequest};
    use crate::plot::{plot_with, PlotRequest};
    use crate::render::{render_with, RenderRequest};
    use crate::stamp::{check_stale, StaleRequest};

    let dir = scratch("orig_accept_and_stamps");
    let orig = build_made_up_original(&dir, "ACC-101");
    let orig_bytes = fs::read(&orig).unwrap();
    let settings = Settings::default().without_system_fonts();

    // Revise adding text right on top of existing MADE UP TITLE at [10, 80, 0]
    // so the checker triggers `text_on_text`.
    let r1 = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: json!({
                "operations": [{
                    "op": "add_text",
                    "contents": "OVERLAP",
                    "position": [10, 80, 0],
                    "height": 4,
                    "layer": "notes"
                }]
            }),
            dry_run: false,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(r1.ok, "{r1:?}");
    let rep1 = r1.result.unwrap();
    assert_eq!(rep1.journal_entry, Some(1));
    let rev1_dwg = PathBuf::from(rep1.revision_dwg.unwrap());
    let rev1_bytes = fs::read(&rev1_dwg).unwrap();

    // 1. Accept without --despite-failed-checks -> refused with ChecksFailed,
    //    issues nothing, entry 1 still undoable.
    let fail_acc = accept_with(
        &AcceptRequest {
            set: orig.clone(),
            despite_failed_checks: false,
            newest_seen: Some(1),
            dry_run: false,
        },
        &settings,
        Interface::Cli,
    );
    assert!(!fail_acc.ok);
    assert_eq!(
        fail_acc.error.as_ref().unwrap().code,
        ErrorCode::ChecksFailed
    );
    let partial = fail_acc
        .result
        .expect("partial accept report with findings");
    assert!(partial.journal_entry.is_none());
    assert!(!partial.checks.findings.is_empty());
    assert_eq!(
        journal(&JournalRequest { set: orig.clone() })
            .result
            .unwrap()
            .can_undo,
        vec![1]
    );

    // 2. Wrong --newest-seen -> MovedOn.
    let stale_seen = accept_with(
        &AcceptRequest {
            set: orig.clone(),
            despite_failed_checks: true,
            newest_seen: Some(0),
            dry_run: false,
        },
        &settings,
        Interface::Cli,
    );
    assert!(!stale_seen.ok);
    assert_eq!(stale_seen.error.unwrap().code, ErrorCode::MovedOn);

    // 3. Dry run accepts without writing an entry.
    let dry_acc = accept_with(
        &AcceptRequest {
            set: orig.clone(),
            despite_failed_checks: true,
            newest_seen: Some(1),
            dry_run: true,
        },
        &settings,
        Interface::Cli,
    );
    assert!(dry_acc.ok, "{dry_acc:?}");
    assert!(dry_acc.result.unwrap().journal_entry.is_none());

    // 4. Before accept, render & plot of rev1.dwg are NOT issued.
    let png_before = dir.join("before.png");
    let ren_before = render_with(
        &RenderRequest {
            output: Some(png_before.clone()),
            ..RenderRequest::new(rev1_dwg.clone())
        },
        &settings,
    )
    .unwrap();
    assert!(!ren_before.stamp.revision.as_ref().unwrap().issued);
    assert!(
        !check_stale(&StaleRequest {
            output: png_before,
            source: None,
        })
        .unwrap()
        .issued
    );

    // 5. Hand-edited rev1.dwg refuses accept with ChangedByHand.
    fs::write(&rev1_dwg, b"tampered").unwrap();
    let tampered = accept_with(
        &AcceptRequest {
            set: orig.clone(),
            despite_failed_checks: true,
            newest_seen: Some(1),
            dry_run: false,
        },
        &settings,
        Interface::Viewer,
    );
    assert!(!tampered.ok);
    assert_eq!(tampered.error.unwrap().code, ErrorCode::ChangedByHand);
    fs::write(&rev1_dwg, &rev1_bytes).unwrap();

    // 6. Real accept with --despite-failed-checks issues entry 1!
    let acc = accept_with(
        &AcceptRequest {
            set: orig.clone(),
            despite_failed_checks: true,
            newest_seen: Some(1),
            dry_run: false,
        },
        &settings,
        Interface::Viewer,
    );
    assert!(acc.ok, "{acc:?}");
    let acc_rep = acc.result.unwrap();
    assert_eq!(acc_rep.journal_entry, Some(2));
    assert_eq!(acc_rep.via, Interface::Viewer);
    assert_eq!(acc_rep.issued, vec![1]);
    assert_eq!(acc_rep.revision.len(), 1);
    assert_eq!(acc_rep.revision[0].path, "ACC-101.rev1.dwg");

    // 7. Second accept with nothing new -> NothingToAccept.
    let acc2 = accept_with(
        &AcceptRequest {
            set: orig.clone(),
            despite_failed_checks: true,
            newest_seen: Some(2),
            dry_run: false,
        },
        &settings,
        Interface::Cli,
    );
    assert!(!acc2.ok);
    assert_eq!(acc2.error.unwrap().code, ErrorCode::NothingToAccept);

    // 8. Undo of issued entry 1 is refused with Issued!
    let u = undo(&UndoRequest {
        set: orig.clone(),
        change_set: 1,
        dry_run: false,
    });
    assert!(!u.ok);
    assert_eq!(u.error.unwrap().code, ErrorCode::Issued);

    // 9. `render` and `plot` of accepted rev1.dwg carry the ISSUED stamp!
    let png_after = dir.join("after.png");
    let ren_after = render_with(
        &RenderRequest {
            output: Some(png_after.clone()),
            ..RenderRequest::new(rev1_dwg.clone())
        },
        &settings,
    )
    .unwrap();
    let png_rev = ren_after.stamp.revision.as_ref().unwrap();
    assert!(png_rev.issued, "PNG stamp must be issued: {png_rev:?}");
    assert_eq!(png_rev.accept, Some(2));
    assert_eq!(png_rev.via, Some(Interface::Viewer));
    let stale_png = check_stale(&StaleRequest {
        output: png_after,
        source: None,
    })
    .unwrap();
    assert!(!stale_png.stale);
    assert!(stale_png.issued);

    let pdf_after = dir.join("after.pdf");
    let plt_after = plot_with(
        &PlotRequest {
            source: rev1_dwg.clone(),
            output: Some(pdf_after.clone()),
            overwrite: true,
            plot_style: None,
        },
        &settings,
    )
    .unwrap();
    let pdf_rev = plt_after.stamp.revision.as_ref().unwrap();
    assert!(pdf_rev.issued, "PDF stamp must be issued: {pdf_rev:?}");
    assert_eq!(pdf_rev.accept, Some(2));
    assert_eq!(pdf_rev.via, Some(Interface::Viewer));
    let stale_pdf = check_stale(&StaleRequest {
        output: pdf_after,
        source: None,
    })
    .unwrap();
    assert!(!stale_pdf.stale);
    assert!(stale_pdf.issued);

    // 10. A new revision after accept (entry 3) CAN be undone back to the
    //     accept while leaving issued rev1.dwg untouched!
    let r2 = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: json!({
                "operations": [{"op": "add_line", "from": [0, 0, 0], "to": [5, 0, 0]}]
            }),
            dry_run: false,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(r2.ok, "{r2:?}");
    let rev2_dwg = PathBuf::from(r2.result.unwrap().revision_dwg.unwrap());
    let j = journal(&JournalRequest { set: orig.clone() })
        .result
        .unwrap();
    assert_eq!(j.can_undo, vec![3]);
    assert_eq!(j.entries[0].issued_by, Some(2));

    let u3 = undo(&UndoRequest {
        set: orig.clone(),
        change_set: 3,
        dry_run: false,
    });
    assert!(u3.ok, "{u3:?}");
    assert!(!rev2_dwg.exists());
    assert!(rev1_dwg.is_file(), "issued rev1.dwg must remain");
    assert_eq!(fs::read(&orig).unwrap(), orig_bytes);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn dwg_extension_folder_still_routes_to_drawing_set_journal_undo_and_accept() {
    let dir = scratch("dwg_folder_routing");
    let set_dir = dir.join("MY-SET.DWG");
    fs::create_dir_all(&set_dir).unwrap();
    fs::write(
        set_dir.join("set.yaml"),
        "units: mm\noutput: dwg\nrows: rows.yaml\n",
    )
    .unwrap();
    fs::write(
        set_dir.join("template.yaml"),
        "units: mm\npaper: [100, 100]\nobjects:\n  - line: {id: l1, from: [0, 0, 0], to: [100, 100, 0]}\n",
    )
    .unwrap();
    fs::write(
        set_dir.join("rows.yaml"),
        "rows:\n  - {sheet: S-1, template: template.yaml}\n",
    )
    .unwrap();

    let settings = Settings::default().without_system_fonts();
    let set_str = set_dir.to_string_lossy().into_owned();

    // `change` via command registry routes to drawing set.
    let ch = find_command("change").unwrap().run(
        json!({
            "set": set_str,
            "change_set": {
                "operations": [{"op": "add_circle", "sheet": "S-1", "center": [50, 50, 0], "radius": 5}]
            }
        }),
        &settings,
        Interface::Cli,
    );
    assert!(ch.ok, "{ch:?}");
    assert!(
        set_dir
            .join(".agentdrafter")
            .join("journal.jsonl")
            .is_file(),
        "must write drawing-set journal inside MY-SET.DWG/.agentdrafter/journal.jsonl"
    );
    assert!(
        !dir.join(".agentdrafter").exists(),
        "must NOT treat MY-SET.DWG directory as an original DWG"
    );

    // `journal` routes to drawing set.
    let j =
        find_command("journal")
            .unwrap()
            .run(json!({"set": set_str}), &settings, Interface::Cli);
    assert!(j.ok, "{j:?}");
    let jv: Value = serde_json::from_str(j.result.unwrap().get()).unwrap();
    assert_eq!(jv["can_undo"], json!([1]));

    // `undo` routes to drawing set.
    let u = find_command("undo").unwrap().run(
        json!({"set": set_str, "change_set": 1}),
        &settings,
        Interface::Cli,
    );
    assert!(u.ok, "{u:?}");

    // `accept` routes to drawing set.
    let a = find_command("accept").unwrap().run(
        json!({"set": set_str, "despite_failed_checks": true}),
        &settings,
        Interface::Cli,
    );
    assert!(a.ok, "{a:?}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn original_dry_run_undo_and_accept_refuse_pending_and_never_write() {
    use crate::change::original_journal::test_pending_journal_path;
    use crate::change::{accept_with, undo, AcceptRequest, UndoRequest, STOP_AT};

    let dir = scratch("orig_dry_run_undo_accept_pending");
    let orig = build_made_up_original(&dir, "DRY-PEND");
    let orig_bytes = fs::read(&orig).unwrap();
    let settings = Settings::default().without_system_fonts();

    // Before any journal exists: undo/accept (both dry-run and real) create NO
    // `.agentdrafter/` folder or lock file.
    for dry in [true, false] {
        let u = undo(&UndoRequest {
            set: orig.clone(),
            change_set: 1,
            dry_run: dry,
        });
        assert!(!u.ok);
        assert_eq!(u.error.unwrap().code, ErrorCode::UnknownName);
        let a = accept_with(
            &AcceptRequest {
                set: orig.clone(),
                despite_failed_checks: true,
                newest_seen: None,
                dry_run: dry,
            },
            &settings,
            Interface::Cli,
        );
        assert!(!a.ok);
        assert_eq!(a.error.unwrap().code, ErrorCode::NothingToAccept);
        assert!(
            !dir.join(".agentdrafter").exists(),
            "must not create .agentdrafter/ on unjournaled original (dry={dry})"
        );
    }

    // Create a real revision first, then leave a pending revision on disk.
    let r1 = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: json!({
                "operations": [{"op": "add_line", "from": [1, 1, 0], "to": [10, 1, 0]}]
            }),
            dry_run: false,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(r1.ok, "{r1:?}");

    // Stop second revision mid-commit after placing rev2.dwg.
    STOP_AT.with(|s| s.set(Some(3)));
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        revise_with(
            &ReviseRequest {
                set: orig.clone(),
                change_set: json!({
                    "operations": [{"op": "add_line", "from": [1, 2, 0], "to": [10, 2, 0]}]
                }),
                dry_run: false,
                output: None,
                allow_loss: false,
                plot_style: None,
                no_pdf: true,
            },
            &settings,
        )
    }));
    STOP_AT.with(|s| s.set(None));
    assert!(panicked.is_err());

    let jpath = test_pending_journal_path(&orig).unwrap();
    let pending_journal = fs::read(&jpath).unwrap();
    let rev2_dwg = dir.join("DRY-PEND.rev2.dwg");
    assert!(rev2_dwg.is_file());
    let rev2_bytes = fs::read(&rev2_dwg).unwrap();

    // `undo --dry-run` (direct AND via Command::run) refuses with RollbackFailed
    // and leaves journal + partial rev2.dwg byte-for-byte untouched!
    let dry_u = undo(&UndoRequest {
        set: orig.clone(),
        change_set: 1,
        dry_run: true,
    });
    assert!(!dry_u.ok);
    assert!(dry_u.rolled_back.is_none());
    assert_eq!(dry_u.error.unwrap().code, ErrorCode::RollbackFailed);
    assert_eq!(fs::read(&jpath).unwrap(), pending_journal);
    assert_eq!(fs::read(&rev2_dwg).unwrap(), rev2_bytes);

    let dry_u_cmd = find_command("undo").unwrap().run(
        json!({"set": orig.to_string_lossy(), "change_set": 1, "dry_run": true}),
        &settings,
        Interface::Cli,
    );
    assert!(!dry_u_cmd.ok);
    assert!(dry_u_cmd.rolled_back.is_none());
    assert_eq!(dry_u_cmd.error.unwrap().code, ErrorCode::RollbackFailed);
    assert_eq!(fs::read(&jpath).unwrap(), pending_journal);
    assert_eq!(fs::read(&rev2_dwg).unwrap(), rev2_bytes);

    // `accept --dry-run` (direct AND via Command::run) refuses with RollbackFailed
    // and leaves journal + partial rev2.dwg byte-for-byte untouched!
    let dry_a = accept_with(
        &AcceptRequest {
            set: orig.clone(),
            despite_failed_checks: true,
            newest_seen: None,
            dry_run: true,
        },
        &settings,
        Interface::Cli,
    );
    assert!(!dry_a.ok);
    assert!(dry_a.rolled_back.is_none());
    assert_eq!(dry_a.error.unwrap().code, ErrorCode::RollbackFailed);
    assert_eq!(fs::read(&jpath).unwrap(), pending_journal);
    assert_eq!(fs::read(&rev2_dwg).unwrap(), rev2_bytes);

    let dry_a_cmd = find_command("accept").unwrap().run(
        json!({"set": orig.to_string_lossy(), "despite_failed_checks": true, "dry_run": true}),
        &settings,
        Interface::Cli,
    );
    assert!(!dry_a_cmd.ok);
    assert!(dry_a_cmd.rolled_back.is_none());
    assert_eq!(dry_a_cmd.error.unwrap().code, ErrorCode::RollbackFailed);
    assert_eq!(fs::read(&jpath).unwrap(), pending_journal);
    assert_eq!(fs::read(&rev2_dwg).unwrap(), rev2_bytes);
    assert_eq!(fs::read(&orig).unwrap(), orig_bytes);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn original_recovery_cleanup_is_scoped_and_never_touches_bystanders() {
    use crate::change::original_journal::test_original_temp_scope;
    use crate::change::{journal, JournalRequest, STOP_AT};

    let dir = scratch("orig_scoped_cleanup");
    let orig_a = build_made_up_original(&dir, "SCOPE-A");
    let orig_b = build_made_up_original(&dir, "SCOPE-B");
    let settings = Settings::default().without_system_fonts();

    // Plant bystander files in the same folder that share the old unscoped
    // `.agentdrafter-entry-1-` prefix AND another original's entry-1 temp!
    let master_bystander = dir.join(".agentdrafter-entry-1-master.dwg");
    fs::write(&master_bystander, b"important-master-dwg").unwrap();
    let scope_b = test_original_temp_scope(&fs::canonicalize(&orig_b).unwrap());
    let orig_b_tmp = dir.join(format!(
        ".agentdrafter-orig-{scope_b}-entry-1-f0-deadbeef.rev.tmp"
    ));
    fs::write(&orig_b_tmp, b"other-original-in-flight").unwrap();

    // Two originals in DIFFERENT folders with the SAME filename (`east/A.dwg`
    // and `west/A.dwg`) sharing an output folder must get distinct temp scopes
    // so recovering `east/A.dwg` entry 1 never deletes `west/A.dwg`'s temp!
    let east = dir.join("east");
    let west = dir.join("west");
    let shared_out = dir.join("shared-out");
    fs::create_dir_all(&east).unwrap();
    fs::create_dir_all(&west).unwrap();
    fs::create_dir_all(&shared_out).unwrap();
    let east_a = build_made_up_original(&east, "SAME");
    let west_a = build_made_up_original(&west, "SAME");
    let scope_east = test_original_temp_scope(&fs::canonicalize(&east_a).unwrap());
    let scope_west = test_original_temp_scope(&fs::canonicalize(&west_a).unwrap());
    assert_ne!(
        scope_east, scope_west,
        "east/SAME.dwg and west/SAME.dwg must have distinct temp scopes"
    );
    let west_shared_tmp = shared_out.join(format!(
        ".agentdrafter-orig-{scope_west}-entry-1-f0-cafebabe.rev.tmp"
    ));
    fs::write(&west_shared_tmp, b"west-in-flight").unwrap();

    STOP_AT.with(|s| s.set(Some(3)));
    let east_panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        revise_with(
            &ReviseRequest {
                set: east_a.clone(),
                change_set: json!({
                    "operations": [{"op": "add_line", "from": [1, 1, 0], "to": [10, 1, 0]}]
                }),
                dry_run: false,
                output: Some(shared_out.join("east-r1.dwg")),
                allow_loss: false,
                plot_style: None,
                no_pdf: true,
            },
            &settings,
        )
    }));
    STOP_AT.with(|s| s.set(None));
    assert!(east_panicked.is_err());
    let j_east = journal(&JournalRequest { set: east_a });
    assert!(j_east.ok, "{j_east:?}");
    assert!(j_east.rolled_back.is_some());
    assert_eq!(
        fs::read(&west_shared_tmp).unwrap(),
        b"west-in-flight",
        "recovering east/SAME.dwg in shared output folder must never touch west/SAME.dwg's temp"
    );

    // Kill revision entry 1 on SCOPE-A mid-commit.
    STOP_AT.with(|s| s.set(Some(3)));
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        revise_with(
            &ReviseRequest {
                set: orig_a.clone(),
                change_set: json!({
                    "operations": [{"op": "add_line", "from": [1, 1, 0], "to": [10, 1, 0]}]
                }),
                dry_run: false,
                output: None,
                allow_loss: false,
                plot_style: None,
                no_pdf: true,
            },
            &settings,
        )
    }));
    STOP_AT.with(|s| s.set(None));
    assert!(panicked.is_err());

    // Recover SCOPE-A -> must clean up SCOPE-A's own temp/partial files while
    // leaving `.agentdrafter-entry-1-master.dwg` and SCOPE-B's temp untouched!
    let j = journal(&JournalRequest { set: orig_a });
    assert!(j.ok, "{j:?}");
    assert!(j.rolled_back.is_some());
    assert_eq!(
        fs::read(&master_bystander).unwrap(),
        b"important-master-dwg",
        "recovery must never delete a .dwg file matching an entry prefix"
    );
    assert_eq!(
        fs::read(&orig_b_tmp).unwrap(),
        b"other-original-in-flight",
        "recovery of SCOPE-A must never delete SCOPE-B's temp file"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn original_journal_and_prune_refuse_symlinks_without_modifying_targets() {
    use crate::change::original_journal::test_pending_journal_path;
    use crate::change::{journal, undo, JournalRequest, UndoRequest};

    let dir = scratch("orig_journal_symlinks");
    let orig = build_made_up_original(&dir, "SYM-1");
    let orig_bytes = fs::read(&orig).unwrap();
    let settings = Settings::default().without_system_fonts();

    // 1. Symlinked `journal.jsonl` pointing at the original DWG itself ->
    //    `change` and `journal` refuse cleanly without modifying the original!
    let jpath = test_pending_journal_path(&orig).unwrap();
    let jdir = jpath.parent().unwrap();
    fs::create_dir_all(jdir).unwrap();
    std::os::unix::fs::symlink(&orig, &jpath).unwrap();

    let j_res = journal(&JournalRequest { set: orig.clone() });
    assert!(!j_res.ok, "symlinked journal.jsonl must be refused");
    assert_eq!(j_res.error.unwrap().code, ErrorCode::ReadFailed);

    let ch_res = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: json!({
                "operations": [{"op": "add_line", "from": [1, 1, 0], "to": [5, 1, 0]}]
            }),
            dry_run: false,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(!ch_res.ok, "change with symlinked journal.jsonl must fail");
    assert_eq!(
        fs::read(&orig).unwrap(),
        orig_bytes,
        "original DWG must never be modified through a symlinked journal.jsonl"
    );
    fs::remove_file(&jpath).unwrap();

    // 2. Real revision succeeds, then replace `undo-states` with a symlink to
    //    the drawing folder containing the original and a bystander file ->
    //    pruning / undo must NEVER delete anything inside the target folder!
    let r1 = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: json!({
                "operations": [{"op": "add_line", "from": [1, 1, 0], "to": [5, 1, 0]}]
            }),
            dry_run: false,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(r1.ok, "{r1:?}");
    let bystander_hex = dir.join("0".repeat(64));
    fs::write(&bystander_hex, b"must-survive").unwrap();
    let undo_states_dir = jdir.join("undo-states");
    let _ = fs::remove_dir_all(&undo_states_dir);
    std::os::unix::fs::symlink(&dir, &undo_states_dir).unwrap();

    // Undo refuses because `undo-states` is a symlink, and deletes nothing!
    let u = undo(&UndoRequest {
        set: orig.clone(),
        change_set: 1,
        dry_run: false,
    });
    assert!(!u.ok, "undo with symlinked undo-states must be refused");
    assert_eq!(fs::read(&orig).unwrap(), orig_bytes);
    assert_eq!(fs::read(&bystander_hex).unwrap(), b"must-survive");

    // Another revision triggers `prune_original_at` — symlinked `undo-states`
    // must be skipped completely so neither `orig` nor `bystander_hex` is deleted!
    let r2 = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: json!({
                "operations": [{"op": "add_line", "from": [1, 2, 0], "to": [5, 2, 0]}]
            }),
            dry_run: false,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(r2.ok, "{r2:?}");
    assert_eq!(fs::read(&orig).unwrap(), orig_bytes);
    assert_eq!(fs::read(&bystander_hex).unwrap(), b"must-survive");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn original_cross_folder_output_stamps_issued_on_render_and_plot() {
    use crate::change::{accept_with, AcceptRequest};
    use crate::plot::{plot_with, PlotRequest};
    use crate::render::{render_with, RenderRequest};
    use crate::stamp::{check_stale, StaleRequest};

    let dir = scratch("orig_cross_folder_stamp");
    let drawings_dir = dir.join("drawings");
    let out_dir = dir.join("issued-out");
    fs::create_dir_all(&drawings_dir).unwrap();
    fs::create_dir_all(&out_dir).unwrap();
    let orig = build_made_up_original(&drawings_dir, "CROSS-100");
    let rev_dwg = out_dir.join("R.dwg");
    let settings = Settings::default().without_system_fonts();

    let r = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: json!({
                "operations": [{"op": "add_line", "from": [2, 2, 0], "to": [12, 2, 0]}]
            }),
            dry_run: false,
            output: Some(rev_dwg.clone()),
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(r.ok, "{r:?}");

    // Before accept: render of `<out_dir>/R.dwg` is NOT issued.
    let png_before = out_dir.join("before.png");
    let ren_before = render_with(
        &RenderRequest {
            output: Some(png_before),
            ..RenderRequest::new(rev_dwg.clone())
        },
        &settings,
    )
    .unwrap();
    assert!(!ren_before.stamp.revision.as_ref().unwrap().issued);

    // Accept the original in `<drawings_dir>/CROSS-100.dwg`.
    let acc = accept_with(
        &AcceptRequest {
            set: orig,
            despite_failed_checks: true,
            newest_seen: Some(1),
            dry_run: false,
        },
        &settings,
        Interface::Cli,
    );
    assert!(acc.ok, "{acc:?}");

    // Render and plot of `<out_dir>/R.dwg` (in a DIFFERENT folder!) both carry
    // `issued: true` and `check_stale` confirms `issued: true`!
    let png_after = out_dir.join("after.png");
    let ren_after = render_with(
        &RenderRequest {
            output: Some(png_after.clone()),
            ..RenderRequest::new(rev_dwg.clone())
        },
        &settings,
    )
    .unwrap();
    let png_rev = ren_after.stamp.revision.as_ref().unwrap();
    assert!(
        png_rev.issued,
        "cross-folder PNG must be issued: {png_rev:?}"
    );
    assert_eq!(png_rev.accept, Some(2));
    assert!(
        check_stale(&StaleRequest {
            output: png_after,
            source: None,
        })
        .unwrap()
        .issued
    );

    let pdf_after = out_dir.join("after.pdf");
    let plt_after = plot_with(
        &PlotRequest {
            source: rev_dwg,
            output: Some(pdf_after.clone()),
            overwrite: true,
            plot_style: None,
        },
        &settings,
    )
    .unwrap();
    let pdf_rev = plt_after.stamp.revision.as_ref().unwrap();
    assert!(
        pdf_rev.issued,
        "cross-folder PDF must be issued: {pdf_rev:?}"
    );
    assert_eq!(pdf_rev.accept, Some(2));
    assert!(
        check_stale(&StaleRequest {
            output: pdf_after,
            source: None,
        })
        .unwrap()
        .issued
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn original_child_folder_output_stamps_issued_on_render_and_plot() {
    use crate::change::{accept_with, AcceptRequest};
    use crate::plot::{plot_with, PlotRequest};
    use crate::render::{render_with, RenderRequest};
    use crate::stamp::{check_stale, StaleRequest};

    let dir = scratch("orig_child_folder_stamp");
    let drawings_dir = dir.join("drawings");
    let child_out = drawings_dir.join("out");
    fs::create_dir_all(&child_out).unwrap();
    let orig = build_made_up_original(&drawings_dir, "CHILD-100");
    let rev_dwg = child_out.join("R.dwg");
    let settings = Settings::default().without_system_fonts();

    let r = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: json!({
                "operations": [{"op": "add_line", "from": [2, 2, 0], "to": [12, 2, 0]}]
            }),
            dry_run: false,
            output: Some(rev_dwg.clone()),
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(r.ok, "{r:?}");

    let acc = accept_with(
        &AcceptRequest {
            set: orig,
            despite_failed_checks: true,
            newest_seen: Some(1),
            dry_run: false,
        },
        &settings,
        Interface::Cli,
    );
    assert!(acc.ok, "{acc:?}");

    // Render and plot of `<drawings>/out/R.dwg` (child folder of original!)
    // both carry `issued: true`!
    let png_after = child_out.join("after.png");
    let ren_after = render_with(
        &RenderRequest {
            output: Some(png_after.clone()),
            ..RenderRequest::new(rev_dwg.clone())
        },
        &settings,
    )
    .unwrap();
    let png_rev = ren_after.stamp.revision.as_ref().unwrap();
    assert!(
        png_rev.issued,
        "child-folder PNG must be issued: {png_rev:?}"
    );
    assert_eq!(png_rev.accept, Some(2));
    assert!(
        check_stale(&StaleRequest {
            output: png_after,
            source: None,
        })
        .unwrap()
        .issued
    );

    let pdf_after = child_out.join("after.pdf");
    let plt_after = plot_with(
        &PlotRequest {
            source: rev_dwg,
            output: Some(pdf_after.clone()),
            overwrite: true,
            plot_style: None,
        },
        &settings,
    )
    .unwrap();
    let pdf_rev = plt_after.stamp.revision.as_ref().unwrap();
    assert!(
        pdf_rev.issued,
        "child-folder PDF must be issued: {pdf_rev:?}"
    );
    assert_eq!(pdf_rev.accept, Some(2));
    assert!(
        check_stale(&StaleRequest {
            output: pdf_after,
            source: None,
        })
        .unwrap()
        .issued
    );

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn original_symlink_source_resolves_consistently_and_stamps_refuse_symlinks() {
    use crate::change::{
        accept_with, journal, undo, AcceptRequest, JournalRequest, UndoRequest, STOP_AT,
    };
    use crate::render::{render_with, RenderRequest};

    let dir = scratch("orig_symlink_source_and_stamps");
    let drawings = dir.join("drawings");
    let aliases = dir.join("aliases");
    fs::create_dir_all(&drawings).unwrap();
    fs::create_dir_all(&aliases).unwrap();
    let real_orig = build_made_up_original(&drawings, "ALIAS-1");
    let orig_bytes = fs::read(&real_orig).unwrap();
    let sym_orig = aliases.join("ALIAS-1.dwg");
    std::os::unix::fs::symlink(&real_orig, &sym_orig).unwrap();
    let settings = Settings::default().without_system_fonts();
    let cs = json!({"operations": [{"op": "add_line", "from": [1, 1, 0], "to": [10, 1, 0]}]});

    // 1. Revise via symlink `aliases/ALIAS-1.dwg` with `--output drawings/R1.dwg`:
    //    `journal`, `undo`, `accept`, and crash recovery MUST all resolve
    //    `drawings/R1.dwg` (NEVER `aliases/R1.dwg`)!
    let r1_dwg = drawings.join("R1.dwg");
    let r1 = revise_with(
        &ReviseRequest {
            set: sym_orig.clone(),
            change_set: cs.clone(),
            dry_run: false,
            output: Some(r1_dwg.clone()),
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(r1.ok, "{r1:?}");
    assert!(r1_dwg.is_file());

    let j = journal(&JournalRequest {
        set: sym_orig.clone(),
    });
    assert!(j.ok, "{j:?}");
    assert_eq!(j.result.unwrap().can_undo, vec![1]);

    let u = undo(&UndoRequest {
        set: sym_orig.clone(),
        change_set: 1,
        dry_run: false,
    });
    assert!(u.ok, "{u:?}");
    assert!(
        !r1_dwg.exists(),
        "undo via symlinked source must remove drawings/R1.dwg"
    );

    // 2. Killed commit via symlinked source recovers `drawings/R2.dwg`.
    let r2_dwg = drawings.join("R2.dwg");
    STOP_AT.with(|s| s.set(Some(3)));
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        revise_with(
            &ReviseRequest {
                set: sym_orig.clone(),
                change_set: cs.clone(),
                dry_run: false,
                output: Some(r2_dwg.clone()),
                allow_loss: false,
                plot_style: None,
                no_pdf: true,
            },
            &settings,
        )
    }));
    STOP_AT.with(|s| s.set(None));
    assert!(panicked.is_err());
    assert!(r2_dwg.is_file());
    let j_rec = journal(&JournalRequest {
        set: sym_orig.clone(),
    });
    assert!(j_rec.ok, "{j_rec:?}");
    assert!(j_rec.rolled_back.is_some());
    assert!(
        !r2_dwg.exists(),
        "recovery via symlinked source must remove drawings/R2.dwg"
    );

    // 3. Revise & accept `drawings/R3.dwg` via symlinked source, then verify
    //    `issued_of_revision_dwg` refuses symlinks on BOTH sides!
    let r3_dwg = drawings.join("R3.dwg");
    let r3 = revise_with(
        &ReviseRequest {
            set: sym_orig.clone(),
            change_set: cs,
            dry_run: false,
            output: Some(r3_dwg.clone()),
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(r3.ok, "{r3:?}");
    let acc = accept_with(
        &AcceptRequest {
            set: sym_orig,
            despite_failed_checks: true,
            newest_seen: None,
            dry_run: false,
        },
        &settings,
        Interface::Cli,
    );
    assert!(acc.ok, "{acc:?}");

    // Real accepted R3.dwg renders with `issued: true`.
    let ren_real = render_with(
        &RenderRequest {
            output: Some(dir.join("real.png")),
            ..RenderRequest::new(r3_dwg.clone())
        },
        &settings,
    )
    .unwrap();
    assert!(ren_real.stamp.revision.as_ref().unwrap().issued);

    // Symlink pointing at accepted R3.dwg must NOT stamp as issued.
    let link_to_r3 = drawings.join("link-to-r3.dwg");
    std::os::unix::fs::symlink(&r3_dwg, &link_to_r3).unwrap();
    let ren_link = render_with(
        &RenderRequest {
            output: Some(dir.join("link.png")),
            ..RenderRequest::new(link_to_r3)
        },
        &settings,
    )
    .unwrap();
    assert!(
        !ren_link.stamp.revision.as_ref().unwrap().issued,
        "symlink to accepted revision must not stamp issued"
    );

    // Copy R3.dwg to `unissued-copy.dwg` and replace R3.dwg with a symlink to
    // `unissued-copy.dwg` -> neither must stamp as issued!
    let unissued_copy = drawings.join("unissued-copy.dwg");
    fs::copy(&r3_dwg, &unissued_copy).unwrap();
    fs::remove_file(&r3_dwg).unwrap();
    std::os::unix::fs::symlink(&unissued_copy, &r3_dwg).unwrap();
    let ren_copy = render_with(
        &RenderRequest {
            output: Some(dir.join("copy.png")),
            ..RenderRequest::new(unissued_copy)
        },
        &settings,
    )
    .unwrap();
    assert!(
        !ren_copy.stamp.revision.as_ref().unwrap().issued,
        "unissued copy targeted by symlink at accepted path must not stamp issued"
    );
    let ren_replaced = render_with(
        &RenderRequest {
            output: Some(dir.join("replaced.png")),
            ..RenderRequest::new(r3_dwg)
        },
        &settings,
    )
    .unwrap();
    assert!(
        !ren_replaced.stamp.revision.as_ref().unwrap().issued,
        "symlink replacing accepted revision must not stamp issued"
    );
    assert_eq!(fs::read(&real_orig).unwrap(), orig_bytes);

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn original_undo_refuses_symlinked_output_subdirectory() {
    use crate::change::{undo, UndoRequest};

    let dir = scratch("orig_undo_symlink_subdir");
    let drawings = dir.join("drawings");
    let sub = drawings.join("sub");
    let moved_sub = dir.join("moved-sub");
    fs::create_dir_all(&sub).unwrap();
    let orig = build_made_up_original(&drawings, "SUB-1");
    let rev_dwg = sub.join("R.dwg");
    let settings = Settings::default().without_system_fonts();

    let r = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: json!({
                "operations": [{"op": "add_line", "from": [1, 1, 0], "to": [10, 1, 0]}]
            }),
            dry_run: false,
            output: Some(rev_dwg.clone()),
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(r.ok, "{r:?}");

    // Move `drawings/sub` out and replace it with a symlink -> `undo` must
    // refuse without deleting anything inside `moved-sub`!
    fs::rename(&sub, &moved_sub).unwrap();
    std::os::unix::fs::symlink(&moved_sub, &sub).unwrap();

    let u = undo(&UndoRequest {
        set: orig,
        change_set: 1,
        dry_run: false,
    });
    assert!(
        !u.ok,
        "undo through symlinked output subdir must be refused"
    );
    assert_eq!(u.error.unwrap().code, ErrorCode::ChangedByHand);
    assert!(
        moved_sub.join("R.dwg").is_file(),
        "file inside symlinked target dir must never be deleted"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn original_external_output_refuses_symlinked_ancestor_in_undo_and_stamps() {
    use crate::change::{accept_with, undo, AcceptRequest, UndoRequest};
    use crate::render::{render_with, RenderRequest};

    let dir = scratch("orig_external_symlink_ancestor");
    let drawings = dir.join("drawings");
    let job1_sub = dir.join("external").join("job1").join("sub");
    let job2_sub = dir.join("external").join("job2").join("sub");
    fs::create_dir_all(&drawings).unwrap();
    fs::create_dir_all(&job1_sub).unwrap();
    fs::create_dir_all(&job2_sub).unwrap();
    let orig = build_made_up_original(&drawings, "EXT-ANC");
    let settings = Settings::default().without_system_fonts();
    let cs = json!({"operations": [{"op": "add_line", "from": [1, 1, 0], "to": [10, 1, 0]}]});

    // 1. Revise into external/job1/sub/R1.dwg and accept -> stamps issued.
    let r1_dwg = job1_sub.join("R1.dwg");
    let r1 = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: cs.clone(),
            dry_run: false,
            output: Some(r1_dwg.clone()),
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(r1.ok, "{r1:?}");
    let acc = accept_with(
        &AcceptRequest {
            set: orig.clone(),
            despite_failed_checks: true,
            newest_seen: Some(1),
            dry_run: false,
        },
        &settings,
        Interface::Cli,
    );
    assert!(acc.ok, "{acc:?}");
    let ren_ok = render_with(
        &RenderRequest {
            output: Some(dir.join("ok.png")),
            ..RenderRequest::new(r1_dwg.clone())
        },
        &settings,
    )
    .unwrap();
    assert!(ren_ok.stamp.revision.as_ref().unwrap().issued);

    // Replace ancestor `external/job1` with a symlink -> `render` must no
    // longer treat `external/job1/sub/R1.dwg` as issued!
    let job1 = dir.join("external").join("job1");
    let moved_job1 = dir.join("external").join("moved-job1");
    fs::rename(&job1, &moved_job1).unwrap();
    std::os::unix::fs::symlink(&moved_job1, &job1).unwrap();
    let ren_sym = render_with(
        &RenderRequest {
            output: Some(dir.join("sym.png")),
            ..RenderRequest::new(r1_dwg)
        },
        &settings,
    )
    .unwrap();
    assert!(
        !ren_sym.stamp.revision.as_ref().unwrap().issued,
        "external revision behind symlinked ancestor dir must not stamp issued"
    );

    // 2. Unaccepted revision in external/job2/sub/R2.dwg: replace ancestor
    //    `external/job2` with a symlink -> `undo` refuses without deleting R2!
    let r2_dwg = job2_sub.join("R2.dwg");
    let r2 = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: cs,
            dry_run: false,
            output: Some(r2_dwg),
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(r2.ok, "{r2:?}");
    let job2 = dir.join("external").join("job2");
    let moved_job2 = dir.join("external").join("moved-job2");
    fs::rename(&job2, &moved_job2).unwrap();
    std::os::unix::fs::symlink(&moved_job2, &job2).unwrap();

    let u = undo(&UndoRequest {
        set: orig,
        change_set: 3,
        dry_run: false,
    });
    assert!(
        !u.ok,
        "undo through symlinked external ancestor must be refused"
    );
    assert_eq!(u.error.unwrap().code, ErrorCode::ChangedByHand);
    assert!(
        moved_job2.join("sub").join("R2.dwg").is_file(),
        "R2.dwg behind symlinked ancestor must never be deleted"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn original_recovery_verifies_undo_states_before_modifying_outputs() {
    use crate::change::original_journal::test_pending_journal_path;
    use crate::change::{journal, JournalRequest, STOP_AT};

    let dir = scratch("orig_recovery_symlink_undo_states_first");
    let orig = build_made_up_original(&dir, "REC-SYM");
    let settings = Settings::default().without_system_fonts();

    // Stop revision 1 mid-commit after placing rev1.dwg on disk.
    STOP_AT.with(|s| s.set(Some(3)));
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        revise_with(
            &ReviseRequest {
                set: orig.clone(),
                change_set: json!({
                    "operations": [{"op": "add_line", "from": [1, 1, 0], "to": [10, 1, 0]}]
                }),
                dry_run: false,
                output: None,
                allow_loss: false,
                plot_style: None,
                no_pdf: true,
            },
            &settings,
        )
    }));
    STOP_AT.with(|s| s.set(None));
    assert!(panicked.is_err());

    let rev1_dwg = dir.join("REC-SYM.rev1.dwg");
    assert!(rev1_dwg.is_file());
    let rev1_bytes = fs::read(&rev1_dwg).unwrap();

    // Plant a symlink at `undo-states` -> recovery MUST fail BEFORE touching
    // or deleting `rev1.dwg`!
    let jpath = test_pending_journal_path(&orig).unwrap();
    let undo_states = jpath.parent().unwrap().join("undo-states");
    let _ = fs::remove_dir_all(&undo_states);
    std::os::unix::fs::symlink(&dir, &undo_states).unwrap();

    let j = journal(&JournalRequest { set: orig });
    assert!(!j.ok, "recovery with symlinked undo-states must refuse");
    assert_eq!(
        fs::read(&rev1_dwg).unwrap(),
        rev1_bytes,
        "partial output must not be modified before undo-states chain is verified"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn successive_revisions_accumulate_edits_and_stack_undo_restores_each_step() {
    use crate::change::{undo, UndoRequest};
    use crate::model::Object;

    let dir = scratch("orig_accumulate");
    let orig = build_made_up_original(&dir, "LOOP-ACC");
    let orig_bytes = fs::read(&orig).unwrap();
    let settings = Settings::default().without_system_fonts();

    let count_lines = |path: &Path| {
        crate::original::read_dwg_record(path, false)
            .unwrap()
            .record
            .sheet
            .objects
            .iter()
            .filter(|o| matches!(o, Object::Line(_)))
            .count()
    };
    let base_lines = count_lines(&orig);

    // Edit 1: add Line A -> rev1.dwg has base + 1 line.
    let r1 = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: json!({
                "operations": [{"op": "add_line", "from": [1, 1, 0], "to": [10, 1, 0]}]
            }),
            dry_run: false,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(r1.ok, "{r1:?}");
    let rev1 = PathBuf::from(r1.result.unwrap().revision_dwg.unwrap());
    assert_eq!(count_lines(&rev1), base_lines + 1);

    // Edit 2: add Line B -> rev2.dwg chains onto rev1 and has BOTH Line A and Line B!
    let r2 = revise_with(
        &ReviseRequest {
            set: orig.clone(),
            change_set: json!({
                "operations": [{"op": "add_line", "from": [1, 5, 0], "to": [10, 5, 0]}]
            }),
            dry_run: false,
            output: None,
            allow_loss: false,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(r2.ok, "{r2:?}");
    let rev2 = PathBuf::from(r2.result.unwrap().revision_dwg.unwrap());
    assert_eq!(
        count_lines(&rev2),
        base_lines + 2,
        "rev2 must retain edit 1 (Line A) alongside edit 2 (Line B)"
    );
    assert_eq!(count_lines(&rev1), base_lines + 1, "rev1 stays untouched");
    assert_eq!(fs::read(&orig).unwrap(), orig_bytes, "original untouched");

    // Undo entry 2 -> rev2 removed, rev1 still has Line A only.
    let u2 = undo(&UndoRequest {
        set: orig.clone(),
        change_set: 2,
        dry_run: false,
    });
    assert!(u2.ok, "{u2:?}");
    assert!(!rev2.exists());
    assert!(rev1.is_file());
    assert_eq!(count_lines(&rev1), base_lines + 1);

    // Undo entry 1 -> rev1 removed, original byte-for-byte untouched.
    let u1 = undo(&UndoRequest {
        set: orig.clone(),
        change_set: 1,
        dry_run: false,
    });
    assert!(u1.ok, "{u1:?}");
    assert!(!rev1.exists());
    assert_eq!(fs::read(&orig).unwrap(), orig_bytes);

    let _ = fs::remove_dir_all(&dir);
}
