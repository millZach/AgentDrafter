//! End-to-end tests for dynamic blocks (#79):
//! visibility states, lookup tables, flips, linear/stretch distances,
//! rotations, one-pipeline PNG/PDF/display-list/snaps/extents, DWG round-trip,
//! `insert_block` with `state`, `set_block_state` (`PROPERTIES`), and `undo`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use agentdrafter_core::{
    block_library_schema, build_set_with, build_with, change_with, check_with, display_list_with,
    plot_with, query_with, read_original, read_sheet_description, render_with,
    sheet_description_schema, undo, Answer, BuildRequest, ChangeRequest, CheckRequest,
    DisplayListRequest, DynamicValue, ErrorCode, Object, PlotRequest, Query, QueryRequest,
    ReadRequest, RenderRequest, Selection, SetBuildRequest, Settings, SnapKind, SnapRef,
    UndoRequest,
};
use serde_json::json;

static SCRATCH_ID: AtomicU64 = AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = SCRATCH_ID.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ad-dyn-{name}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn no_system_fonts() -> Settings {
    Settings {
        font_folders: Vec::new(),
        system_font_folders: Vec::new(),
        plot_style_folders: Vec::new(),
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn example_dynamic_blocks_build_read_query_render_plot_and_check() {
    let dir = scratch("example");
    let src_dir = repo_root().join("examples").join("dynamic-blocks");
    fs::copy(src_dir.join("blocks.yaml"), dir.join("blocks.yaml")).unwrap();
    fs::copy(src_dir.join("sheet.yaml"), dir.join("sheet.yaml")).unwrap();

    let sheet_path = dir.join("sheet.yaml");
    let dwg_path = dir.join("sheet.dwg");
    let png_path = dir.join("sheet.png");
    let pdf_path = dir.join("sheet.pdf");
    let settings = no_system_fonts();

    // 1. Build to DWG 2018.
    let built = build_with(
        &BuildRequest {
            description: sheet_path.clone(),
            output: Some(dwg_path.clone()),
            overwrite: true,
        },
        &settings,
    )
    .expect("build dynamic-blocks example");
    assert!(dwg_path.exists());
    assert!(built.dwg.ends_with("sheet.dwg"));

    // 2. Check passes on the sheet description.
    let checked = check_with(
        &CheckRequest {
            source: sheet_path.clone(),
            sheets: Vec::new(),
        },
        &settings,
    )
    .expect("check sheet");
    assert!(checked.passed, "expected checks to pass: {checked:#?}");

    // 3. Read built DWG back whole with zero unread objects.
    let read = read_original(&ReadRequest {
        source: dwg_path.clone(),
        mirror: None,
        no_mirror: true,
    })
    .expect("read built DWG");
    assert!(read.decoded, "expected decoded DWG: {read:#?}");
    assert!(
        read.not_read.is_empty(),
        "expected every dynamic block object to read back: {:#?}",
        read.not_read
    );
    assert_eq!(read.objects.get("insert"), Some(&6));

    // 4. Query `blocks` on BOTH the sheet description and the built DWG:
    //    effective block names must be `SELECTOR_SWITCH` and `TERMINAL_RAIL`,
    //    never `*U1`.
    for source in [&sheet_path, &dwg_path] {
        let q = query_with(
            &QueryRequest {
                source: source.clone(),
                sheet: None,
                query: Query::Blocks,
            },
            &settings,
        )
        .expect("query blocks");
        let Answer::Blocks { count, blocks } = q.answer else {
            panic!("expected blocks answer");
        };
        assert_eq!(count, 6);
        let names: Vec<&str> = blocks[0].blocks.iter().map(|b| b.block.as_str()).collect();
        assert_eq!(names, ["SELECTOR_SWITCH", "TERMINAL_RAIL"]);
    }

    // 5. Query `find` on the built DWG recovers dynamic `state` on inserts.
    let found = query_with(
        &QueryRequest {
            source: dwg_path.clone(),
            sheet: None,
            query: Query::Find(Selection {
                name: Some("rail-long".into()),
                ..Selection::default()
            }),
        },
        &settings,
    )
    .expect("find rail-long in DWG");
    let Answer::Find { objects, .. } = found.answer else {
        panic!("expected find answer");
    };
    assert_eq!(objects.len(), 1);
    let Object::Insert(ins) = &objects[0].object else {
        panic!("expected insert");
    };
    assert_eq!(ins.block, "TERMINAL_RAIL");
    assert_eq!(ins.state.get("Length"), Some(&DynamicValue::Number(60.0)));

    // 6. Snaps on stretched `rail-long` reflect the stretched length (60, not 20)!
    let snapped = query_with(
        &QueryRequest {
            source: sheet_path.clone(),
            sheet: None,
            query: Query::Snap(SnapRef {
                name: Some("rail-long".into()),
                snap: Some(SnapKind::Center),
                ..SnapRef::default()
            }),
        },
        &settings,
    )
    .expect("snap rail-long centers");
    let Answer::Snap { points, .. } = snapped.answer else {
        panic!("expected snap answer");
    };
    // Right end-stop started at x=18 (for Length=20); stretched to Length=60 -> +40 -> 58,
    // placed at x=95 -> x=153! Left end-stop stays at 95+2=97.
    let xs: Vec<f64> = points.iter().map(|p| p.at.x).collect();
    assert!(
        xs.iter().any(|x| (*x - 97.0).abs() < 1e-6),
        "left circle center should stay at 97: {xs:?}"
    );
    assert!(
        xs.iter().any(|x| (*x - 153.0).abs() < 1e-6),
        "right circle center should move to 153 after stretch: {xs:?}"
    );

    // 7. Render PNG, plot PDF, and produce viewer display list from BOTH sheet and DWG.
    let rendered = render_with(
        &RenderRequest {
            source: sheet_path.clone(),
            output: Some(png_path.clone()),
            overwrite: true,
            width: Some(800),
            height: None,
            dpi: 150.0,
            scale: None,
            margin: 0.0,
            plot_style: None,
            monochrome: false,
        },
        &settings,
    )
    .expect("render PNG");
    assert!(png_path.exists());
    assert!(rendered.width > 0 && rendered.height > 0);

    let plotted = plot_with(
        &PlotRequest {
            source: dwg_path.clone(),
            output: Some(pdf_path.clone()),
            overwrite: true,
            plot_style: None,
        },
        &settings,
    )
    .expect("plot built DWG to PDF");
    assert!(pdf_path.exists());
    assert!(plotted.pdf.ends_with("sheet.pdf"));

    let list_sheet = display_list_with(&DisplayListRequest::new(&sheet_path), &settings)
        .expect("display list from sheet");
    let list_dwg = display_list_with(&DisplayListRequest::new(&dwg_path), &settings)
        .expect("display list from DWG");
    assert!(!list_sheet.display_list.items.is_empty());
    assert_eq!(
        list_sheet.display_list.items.len(),
        list_dwg.display_list.items.len(),
        "sheet and built DWG should produce the same display-list item count"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn visibility_flip_linear_rotation_and_lookup_produce_distinct_drawn_geometry_and_pixels() {
    let dir = scratch("pixels");
    let src_dir = repo_root().join("examples").join("dynamic-blocks");
    fs::copy(src_dir.join("blocks.yaml"), dir.join("blocks.yaml")).unwrap();
    let settings = no_system_fonts();

    let render_state = |tag: &str, block: &str, state_yaml: &str| -> (Vec<u8>, usize) {
        let yaml = format!(
            "units: mm\nblock_library: blocks.yaml\npaper: [100, 60]\n\
             objects:\n  - insert: {{id: item, block: {block}, position: [20, 30, 0], {state_yaml}}}\n"
        );
        let sheet_path = dir.join(format!("{tag}.yaml"));
        let png_path = dir.join(format!("{tag}.png"));
        fs::write(&sheet_path, yaml).unwrap();
        render_with(
            &RenderRequest {
                source: sheet_path.clone(),
                output: Some(png_path.clone()),
                overwrite: true,
                width: Some(400),
                height: None,
                dpi: 150.0,
                scale: None,
                margin: 0.0,
                plot_style: None,
                monochrome: false,
            },
            &settings,
        )
        .unwrap();
        let dl = display_list_with(&DisplayListRequest::new(&sheet_path), &settings).unwrap();
        (fs::read(png_path).unwrap(), dl.display_list.items.len())
    };

    let (png_2p, items_2p) = render_state("v2", "SELECTOR_SWITCH", "state: {Visibility: 2-POS}");
    let (png_3p, items_3p) = render_state("v3", "SELECTOR_SWITCH", "state: {Visibility: 3-POS}");
    let (png_flip, _) = render_state(
        "flip",
        "SELECTOR_SWITCH",
        "state: {Visibility: 2-POS, Flip: Reversed}",
    );
    let (png_rot, _) = render_state(
        "rot",
        "SELECTOR_SWITCH",
        "state: {Visibility: 2-POS, Angle: 90}",
    );
    let (png_lookup, _) = render_state("lkp", "SELECTOR_SWITCH", "state: {Preset: Keyed-Reversed}");
    let (png_short, _) = render_state("r20", "TERMINAL_RAIL", "state: {Length: 20}");
    let (png_long, _) = render_state("r50", "TERMINAL_RAIL", "state: {Length: 50}");

    assert!(items_3p > items_2p, "3-POS draws more geometry than 2-POS");
    assert_ne!(png_2p, png_3p, "visibility state changes rendered PNG");
    assert_ne!(png_2p, png_flip, "flip parameter changes rendered PNG");
    assert_ne!(png_2p, png_rot, "rotation parameter changes rendered PNG");
    assert_ne!(png_2p, png_lookup, "lookup row changes rendered PNG");
    assert_ne!(png_short, png_long, "linear stretch changes rendered PNG");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn set_block_state_properties_and_insert_block_with_state_and_undo() {
    let dir = scratch("changeset");
    let src_dir = repo_root().join("examples").join("dynamic-blocks");
    fs::copy(src_dir.join("blocks.yaml"), dir.join("blocks.yaml")).unwrap();
    fs::write(
        dir.join("set.yaml"),
        "units: mm\nrows: rows.yaml\noutput: out\n",
    )
    .unwrap();
    fs::write(
        dir.join("template.yaml"),
        "units: mm\nblock_library: blocks.yaml\npaper: [210, 148]\n\
         objects:\n  - polyline: {id: border, points: [[0, 0, 0], [210, 0, 0], [210, 148, 0], [0, 148, 0]], closed: true}\n",
    )
    .unwrap();
    fs::write(
        dir.join("rows.yaml"),
        "rows:\n  - sheet: DYN-001\n    template: template.yaml\n    objects:\n      \
         - insert: {id: sw1, name: sw1, block: SELECTOR_SWITCH, position: [50, 70, 0], attributes: {TAG: HS-1}}\n",
    )
    .unwrap();
    let settings = no_system_fonts();

    // Build initial set.
    let built_set = build_set_with(
        &SetBuildRequest {
            set: dir.clone(),
            overwrite: true,
            clean: Some(false),
        },
        &settings,
    );
    assert!(built_set.ok, "initial set build: {built_set:#?}");
    let rows_before = fs::read(dir.join("rows.yaml")).unwrap();

    // 1. Change sw1 via `PROPERTIES` alias + add a stretched rail via `insert_block`.
    let cs = json!({
        "operations": [
            {
                "op": "PROPERTIES",
                "target": {"id": "DYN-001/sw1"},
                "state": {"Visibility": "3-POS", "Flip": true, "Angle": 90}
            },
            {
                "op": "insert_block",
                "sheet": "DYN-001",
                "id": "rail1",
                "block": "TERMINAL_RAIL",
                "position": [100, 70, 0],
                "attributes": {"RAIL": "TB-9"},
                "state": {"Length": 45}
            }
        ]
    });
    let reply = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: cs,
            dry_run: false,
        },
        &settings,
    );
    assert!(reply.ok, "change reply: {reply:#?}");
    let report = reply.result.expect("change report");
    let entry = report.journal_entry.expect("journal entry");

    // Verify `query find` sees both updated dynamic states.
    let q = query_with(
        &QueryRequest {
            source: dir.clone(),
            sheet: Some("DYN-001".into()),
            query: Query::Find(Selection {
                id: Some("DYN-001/sw1".into()),
                ..Selection::default()
            }),
        },
        &settings,
    )
    .unwrap();
    let Answer::Find { objects, .. } = q.answer else {
        panic!("expected find");
    };
    let Object::Insert(sw) = &objects[0].object else {
        panic!("expected insert");
    };
    assert_eq!(
        sw.state.get("Visibility"),
        Some(&DynamicValue::Text("3-POS".into()))
    );
    assert_eq!(sw.state.get("Flip"), Some(&DynamicValue::Bool(true)));
    assert_eq!(sw.state.get("Angle"), Some(&DynamicValue::Number(90.0)));

    // 2. Reset `Flip` to default using `null` via canonical `set_block_state`.
    let reset = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "set_block_state",
                        "target": {"id": "DYN-001/sw1"},
                        "state": {"Flip": null}
                    }
                ]
            }),
            dry_run: false,
        },
        &settings,
    );
    assert!(reset.ok, "{reset:#?}");

    // 3. Undo back to the first entry and verify byte-for-byte restoration.
    let undo_reply = undo(&UndoRequest {
        set: dir.clone(),
        change_set: entry,
        newest_seen: None,
        dry_run: false,
    });
    assert!(undo_reply.ok, "undo dynamic block changes: {undo_reply:#?}");
    let undone = undo_reply.result.expect("undo report");
    assert!(!undone.undid.is_empty());
    assert_eq!(
        fs::read(dir.join("rows.yaml")).unwrap(),
        rows_before,
        "undo restores rows.yaml byte for byte"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn invalid_dynamic_states_are_refused_and_schemas_match() {
    // Validate JSON Schemas expose `parameters` on `block` and `state` on `insert`.
    let lib_schema = block_library_schema();
    assert!(
        lib_schema["$defs"]["block"]["properties"]["parameters"].is_object(),
        "block library schema must document parameters"
    );
    assert!(lib_schema["$defs"]["dynamic_parameter"].is_object());
    let sheet_schema = sheet_description_schema();
    assert!(
        sheet_schema["$defs"]["insert"]["properties"]["state"].is_object(),
        "sheet description schema must document insert state"
    );

    // Invalid state errors carry exact codes and messages.
    let dir = scratch("errors");
    let src_dir = repo_root().join("examples").join("dynamic-blocks");
    fs::copy(src_dir.join("blocks.yaml"), dir.join("blocks.yaml")).unwrap();

    let parse_one = |state_yaml: &str| {
        let p = dir.join("bad.yaml");
        fs::write(
            &p,
            format!(
                "units: mm\nblock_library: blocks.yaml\nobjects:\n  \
                 - insert: {{block: SELECTOR_SWITCH, position: [0, 0, 0], {state_yaml}}}\n"
            ),
        )
        .unwrap();
        read_sheet_description(&p)
    };

    let err_unknown_param = parse_one("state: {NoSuchParam: 1}").unwrap_err();
    assert_eq!(err_unknown_param.code, ErrorCode::UnknownName);

    let err_bad_vis = parse_one("state: {Visibility: 4-POS}").unwrap_err();
    assert_eq!(err_bad_vis.code, ErrorCode::BadValue);

    let err_bad_angle = parse_one("state: {Angle: 45}").unwrap_err();
    assert_eq!(err_bad_angle.code, ErrorCode::BadValue);

    let err_bad_lookup = parse_one("state: {Preset: UnknownPreset}").unwrap_err();
    assert_eq!(err_bad_lookup.code, ErrorCode::BadValue);

    // Out-of-range linear distance.
    let p_rail = dir.join("bad_rail.yaml");
    fs::write(
        &p_rail,
        "units: mm\nblock_library: blocks.yaml\nobjects:\n  \
         - insert: {block: TERMINAL_RAIL, position: [0, 0, 0], state: {Length: 5}}\n",
    )
    .unwrap();
    let err_min = read_sheet_description(&p_rail).unwrap_err();
    assert_eq!(err_min.code, ErrorCode::BadValue);

    // Unit conversion on linear parameter ("2 in" = 50.8 mm, within [15, 80] mm).
    let p_in = dir.join("in_rail.yaml");
    fs::write(
        &p_in,
        "units: mm\nblock_library: blocks.yaml\nobjects:\n  \
         - insert: {block: TERMINAL_RAIL, position: [0, 0, 0], state: {Length: 2 in}}\n",
    )
    .unwrap();
    let sheet = read_sheet_description(&p_in).unwrap();
    let Object::Insert(ins) = &sheet.objects[0] else {
        panic!("expected insert");
    };
    let Some(DynamicValue::Number(len)) = ins.state.get("Length") else {
        panic!("expected numeric Length");
    };
    assert!((*len - 50.8).abs() < 1e-9, "got {len}");

    let _ = fs::remove_dir_all(&dir);
}
