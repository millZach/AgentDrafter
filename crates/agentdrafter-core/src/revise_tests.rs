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

#[test]
fn rollback_checks_file_identity_before_unlinking_and_reports_notes() {
    let dir = scratch("rollback_id");
    let dest = dir.join("rev.dwg");
    let staged = write_temp(&dest, b"ours", "rev", "revision DWG").unwrap();
    place_new_file(&staged.path, &dest, "revision DWG", ErrorCode::OutputExists).unwrap();

    // Case 1: file at dest still has our FileId -> rollback removes it cleanly.
    let notes = rollback_files(&[(&dest, staged.id)], &[]);
    assert!(notes.is_empty(), "unexpected rollback notes: {notes:?}");
    assert!(!dest.exists(), "rollback should remove our placed file");

    // Case 2: file at dest is replaced by a different file -> rollback must NOT
    // delete the unrelated file, and must return a note.
    let staged2 = write_temp(&dest, b"ours-2", "rev", "revision DWG").unwrap();
    place_new_file(
        &staged2.path,
        &dest,
        "revision DWG",
        ErrorCode::OutputExists,
    )
    .unwrap();
    let other = dir.join("other.dwg");
    fs::write(&other, b"someone-else").unwrap();
    fs::rename(&other, &dest).unwrap();

    #[cfg(any(unix, windows))]
    {
        let notes = rollback_files(&[(&dest, staged2.id)], &[]);
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
    assert_eq!(dir_files(&dir), before, "no files may be written");

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
    assert_eq!(dir_files(&dir), before);

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
    let before = dir_files(&dir);
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
    assert_eq!(dir_files(&dir), before);

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
    assert_eq!(dir_files(&dir), before);

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
    let refusal = enforce_verification_gates(
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
    assert!(!refusal.ok);
    assert_eq!(refusal.error.as_ref().unwrap().code, ErrorCode::DwgWrite);
    assert!(
        !refusal
            .error
            .as_ref()
            .unwrap()
            .message
            .contains("--allow-loss"),
        "error should not suggest --allow-loss can bypass requested-edit failures: {}",
        refusal.error.as_ref().unwrap().message
    );
    let report = refusal.result.unwrap();
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
    assert_eq!(dir_files(&dir), before, "no files may be written");

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
    assert_eq!(dir_files(&dir), before, "zero files may be written");

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
    assert_eq!(dir_files(&dir), before, "zero files may be written");

    let _ = fs::remove_dir_all(&dir);
}
