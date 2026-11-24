//! End-to-end tests for annotation scale (`ANNOTATIVE`, `CANNOSCALE`,
//! `ANNOALLVISIBLE`, `OBJECTSCALE`, GitHub Issue #93).

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use agentdrafter_core::{
    build_with_format, change_with, display_list_with, plot_with, query_with, read_original,
    render_with, revise_with, undo, Answer, Bounds, BuildRequest, ChangeRequest, DisplayList,
    DisplayListRequest, DrawingFormat, Object, PlotRequest, Query, QueryRequest, ReadRequest,
    RenderRequest, ReviseRequest, Selection, Settings, Shape, UndoRequest,
};
use serde_json::json;

static NEXT: AtomicU64 = AtomicU64::new(1);

fn temp_dir(tag: &str) -> PathBuf {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "agentdrafter-annoscale-{tag}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn no_system_fonts() -> Settings {
    Settings::default()
}

/// Multi-viewport ISO A3 layout (`1:50` on the left, `1:100` on the right) with
/// all 6 annotative entity kinds plus a scale-filtered object and a plain
/// non-annotative reference text.
fn multi_viewport_sheet(anno_all_visible: Option<bool>) -> String {
    let aav = match anno_all_visible {
        Some(v) => format!("anno_all_visible: {v}\n"),
        None => String::new(),
    };
    format!(
        r#"units: mm
block_library: blocks.yaml
paper: ISO A3 landscape
layout: {{name: Plan}}
annotation_scales:
  '1:1': [1, 1]
  '1:50': [1, 50]
  '1:100': [1, 100]
annotation_scale: '1:50'
{aav}hatch_patterns:
  DIAG:
    lines:
      - {{angle: 45, origin: [0, 0], delta: [0, 2]}}
text_styles:
  ANNO_STYLE: {{font: txt.shx, annotative: true}}
dimension_styles:
  ANNO_DIM: {{annotative: true}}
mleader_styles:
  ANNO_ML: {{text_height: 2.5, arrow_size: 2.5, landing: 5, landing_gap: 1, annotative: true}}
objects:
  # Model-space geometry (a 4000 x 2000 mm room centred at [0, 0]).
  - polyline:
      id: room
      space: model
      points: [[-2000, -1000, 0], [2000, -1000, 0], [2000, 1000, 0], [-2000, 1000, 0]]
      closed: true
  # 0. Annotative text visible in both 1:50 and 1:100 (paper height 3.0 mm).
  - text:
      id: anno-both
      space: model
      contents: ROOM 101
      position: [-1500, 500, 0]
      height: 3.0
      annotative: true
      annotation_scales: ['1:50', '1:100']
  # 1. Annotative text visible ONLY in 1:50.
  - text:
      id: anno-50-only
      space: model
      contents: DETAIL ONLY
      position: [-1500, 0, 0]
      height: 2.5
      style: ANNO_STYLE
      annotation_scales: ['1:50']
  # 2. Plain non-annotative reference text (150 mm tall in model space -> 3 mm at 1:50, 1.5 mm at 1:100).
  - text:
      id: plain-ref
      space: model
      contents: MODEL TEXT
      position: [-1500, -500, 0]
      height: 150.0
  # 3. Annotative mtext supporting both scales.
  - mtext:
      id: anno-mtext
      space: model
      contents: "LINE ONE\nLINE TWO"
      position: [200, 500, 0]
      height: 2.5
      width: 30.0
      annotative: true
      annotation_scales: ['1:50', '1:100']
  # 4. Annotative dimension (via ANNO_DIM style) supporting both scales.
  - dimension:
      id: anno-dim
      space: model
      kind: linear
      points: [[-2000, -1000, 0], [2000, -1000, 0]]
      location: [0, -1400, 0]
      dimension_style: ANNO_DIM
      annotation_scales: ['1:50', '1:100']
  # 5. Annotative multileader supporting both scales.
  - mleader:
      id: anno-mleader
      space: model
      leaders: [[[500, 0, 0], [900, 300, 0]]]
      content: {{text: CALLOUT}}
      mleader_style: ANNO_ML
      annotation_scales: ['1:50', '1:100']
  # 6. Annotative patterned hatch supporting both scales.
  - hatch:
      id: anno-hatch
      space: model
      pattern: DIAG
      scale: 1.0
      boundary:
        - polyline:
            points: [[1000, -800, 0], [1800, -800, 0], [1800, -200, 0], [1000, -200, 0]]
      annotative: true
      annotation_scales: ['1:50', '1:100']
  # 7. Insert of an annotative block supporting both scales (4x4 mm paper square).
  - insert:
      id: anno-sym
      space: model
      block: ANNO_TAG
      position: [0, 0, 0]
      annotation_scales: ['1:50', '1:100']
  # Paper-space border + two viewports at 1:50 (left) and 1:100 (right).
  - polyline:
      id: border
      space: paper
      points: [[10, 10, 0], [410, 10, 0], [410, 287, 0], [10, 287, 0]]
      closed: true
  - viewport:
      id: vp-50
      space: paper
      center: [120, 150, 0]
      width: 180
      height: 160
      view_center: [0, 0, 0]
      scale: '1:50'
      annotation_scale: '1:50'
  - viewport:
      id: vp-100
      space: paper
      center: [310, 150, 0]
      width: 160
      height: 160
      view_center: [0, 0, 0]
      scale: '1:100'
      annotation_scale: '1:100'
"#
    )
}

const BLOCKS_YAML: &str = r#"units: mm
blocks:
  ANNO_TAG:
    annotative: true
    objects:
      - polyline:
          points: [[-2, -2, 0], [2, -2, 0], [2, 2, 0], [-2, 2, 0]]
          closed: true
"#;

/// Computes the bounds of all display-list items drawn for `object_index` whose
/// X coordinates lie inside `[x_min, x_max]` (to isolate left `1:50` viewport vs
/// right `1:100` viewport).
fn object_bounds_in_x_range(
    list: &DisplayList,
    object_index: usize,
    x_min: f64,
    x_max: f64,
) -> Option<Bounds> {
    let mut bounds: Option<Bounds> = None;
    let mut include = |p: [f64; 2]| {
        if p[0] < x_min || p[0] > x_max {
            return;
        }
        bounds = Some(match bounds {
            None => Bounds { min: p, max: p },
            Some(b) => Bounds {
                min: [b.min[0].min(p[0]), b.min[1].min(p[1])],
                max: [b.max[0].max(p[0]), b.max[1].max(p[1])],
            },
        });
    };
    for item in &list.items {
        if item.object != object_index {
            continue;
        }
        match &item.shape {
            Shape::Stroke { runs, .. } => {
                for run in runs {
                    for &pt in run {
                        include(pt);
                    }
                }
            }
            Shape::Fill { contours, .. } => {
                for c in contours {
                    for &pt in c {
                        include(pt);
                    }
                }
            }
            _ => {}
        }
    }
    bounds
}

#[test]
fn multi_viewport_layout_draws_annotative_objects_at_exact_paper_size_and_filters_by_scale() {
    let dir = temp_dir("viewports");
    fs::write(dir.join("blocks.yaml"), BLOCKS_YAML).unwrap();
    let sheet_path = dir.join("plan.yaml");
    fs::write(&sheet_path, multi_viewport_sheet(None)).unwrap();

    let settings = no_system_fonts();
    let dl = display_list_with(&DisplayListRequest::new(&sheet_path), &settings).unwrap();
    assert_eq!(dl.annotation_scale.as_deref(), Some("1:50"));
    assert_eq!(dl.annotation_scales.len(), 3);

    let obj_idx = |local_id: &str| {
        dl.objects
            .iter()
            .position(|o| o.id.as_deref() == Some(&format!("plan/{local_id}")))
            .unwrap_or_else(|| panic!("missing object `{local_id}`"))
    };

    // Left viewport (1:50) spans x in [30, 210]; right viewport (1:100) spans x in [230, 390].
    // 1. Annotative text supporting both 1:50 and 1:100 draws at the EXACT same paper height!
    let both_idx = obj_idx("anno-both");
    let b50 = object_bounds_in_x_range(&dl.display_list, both_idx, 30.0, 210.0)
        .expect("anno-both must draw in 1:50 viewport");
    let b100 = object_bounds_in_x_range(&dl.display_list, both_idx, 230.0, 390.0)
        .expect("anno-both must draw in 1:100 viewport");
    assert!(
        (b50.height() - b100.height()).abs() < 1e-3,
        "annotative text height should match across viewports: 1:50={}, 1:100={}",
        b50.height(),
        b100.height()
    );
    assert!(
        (b50.width() - b100.width()).abs() < 1e-3,
        "annotative text width should match across viewports: 1:50={}, 1:100={}",
        b50.width(),
        b100.width()
    );

    // 2. Non-annotative reference text shrinks by 2x in the 1:100 viewport!
    let plain_idx = obj_idx("plain-ref");
    let p50 = object_bounds_in_x_range(&dl.display_list, plain_idx, 30.0, 210.0).unwrap();
    let p100 = object_bounds_in_x_range(&dl.display_list, plain_idx, 230.0, 390.0).unwrap();
    assert!(
        (p50.height() / p100.height() - 2.0).abs() < 1e-3,
        "plain text should be 2x taller at 1:50 than 1:100: 1:50={}, 1:100={}",
        p50.height(),
        p100.height()
    );

    // 3. Scale-filtered annotative text (`['1:50']` only) draws in 1:50 and HIDES in 1:100!
    let only50_idx = obj_idx("anno-50-only");
    assert!(
        object_bounds_in_x_range(&dl.display_list, only50_idx, 30.0, 210.0).is_some(),
        "anno-50-only must draw in 1:50 viewport"
    );
    assert!(
        object_bounds_in_x_range(&dl.display_list, only50_idx, 230.0, 390.0).is_none(),
        "anno-50-only must be hidden in 1:100 viewport when anno_all_visible is false"
    );

    // 4. Annotative block insert (4x4 mm paper square) draws at exact 4x4 mm in BOTH viewports!
    let sym_idx = obj_idx("anno-sym");
    let s50 = object_bounds_in_x_range(&dl.display_list, sym_idx, 30.0, 210.0).unwrap();
    let s100 = object_bounds_in_x_range(&dl.display_list, sym_idx, 230.0, 390.0).unwrap();
    assert!(
        (s50.width() - 4.0).abs() < 1e-3,
        "1:50 sym width {}",
        s50.width()
    );
    assert!(
        (s100.width() - 4.0).abs() < 1e-3,
        "1:100 sym width {}",
        s100.width()
    );
    assert!((s50.height() - 4.0).abs() < 1e-3);
    assert!((s100.height() - 4.0).abs() < 1e-3);

    // 5. Annotative mtext, dimension, mleader, and hatch all draw in BOTH viewports!
    for local_id in ["anno-mtext", "anno-dim", "anno-mleader", "anno-hatch"] {
        let idx = obj_idx(local_id);
        assert!(
            object_bounds_in_x_range(&dl.display_list, idx, 30.0, 210.0).is_some(),
            "{local_id} must draw in 1:50 viewport"
        );
        assert!(
            object_bounds_in_x_range(&dl.display_list, idx, 230.0, 390.0).is_some(),
            "{local_id} must draw in 1:100 viewport"
        );
    }

    // 6. With anno_all_visible: true, the 1:50-only text ALSO shows in the 1:100 viewport!
    let all_vis_path = dir.join("plan-all-vis.yaml");
    fs::write(&all_vis_path, multi_viewport_sheet(Some(true))).unwrap();
    let dl_all = display_list_with(&DisplayListRequest::new(&all_vis_path), &settings).unwrap();
    let only50_all = dl_all
        .objects
        .iter()
        .position(|o| o.id.as_deref() == Some("plan-all-vis/anno-50-only"))
        .unwrap();
    assert!(
        object_bounds_in_x_range(&dl_all.display_list, only50_all, 230.0, 390.0).is_some(),
        "anno-50-only must draw in 1:100 viewport when anno_all_visible is true"
    );

    // 7. PNG render and PDF plot succeed cleanly through the same single pipeline.
    let png_out = dir.join("plan.png");
    let mut render_req = RenderRequest::new(&sheet_path);
    render_req.output = Some(png_out.clone());
    render_with(&render_req, &settings).unwrap();
    assert!(png_out.exists());

    let pdf_out = dir.join("plan.pdf");
    let mut plot_req = PlotRequest::new(&sheet_path);
    plot_req.output = Some(pdf_out.clone());
    plot_with(&plot_req, &settings).unwrap();
    assert!(pdf_out.exists());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn dwg_and_dxf_roundtrip_preserves_annotation_scales_and_paper_sizes() {
    let dir = temp_dir("roundtrip");
    fs::write(dir.join("blocks.yaml"), BLOCKS_YAML).unwrap();
    let sheet_path = dir.join("plan.yaml");
    fs::write(&sheet_path, multi_viewport_sheet(Some(false))).unwrap();
    let settings = no_system_fonts();

    for (ext, format) in [
        ("dwg", DrawingFormat::Dwg),
        ("dxf", DrawingFormat::Dxf),
        ("bin.dxf", DrawingFormat::DxfBinary),
    ] {
        let out_path = dir.join(format!("plan.{ext}"));
        build_with_format(
            &BuildRequest {
                description: sheet_path.clone(),
                output: Some(out_path.clone()),
                overwrite: true,
            },
            Some(format),
            &settings,
        )
        .unwrap();

        let read_rep = read_original(&ReadRequest {
            source: out_path.clone(),
            mirror: None,
            no_mirror: true,
        })
        .unwrap();
        assert!(read_rep.decoded, "{ext}: {:?}", read_rep.not_decoded);
        assert!(
            read_rep.not_read.is_empty(),
            "{ext}: {:?}",
            read_rep.not_read
        );

        // Query annotation-scales on the built DWG/DXF!
        let q = query_with(
            &QueryRequest {
                source: out_path.clone(),
                sheet: None,
                query: Query::AnnotationScales,
            },
            &settings,
        )
        .unwrap();
        let Answer::AnnotationScales {
            count,
            annotation_scales,
        } = q.answer
        else {
            panic!("expected AnnotationScales answer");
        };
        assert_eq!(
            count, 7,
            "{ext}: expected 7 annotative objects, got {annotation_scales:?}"
        );
        let s = &annotation_scales[0];
        assert_eq!(s.current, "1:50", "{ext}");
        assert!((s.factor - 0.02).abs() < 1e-9, "{ext}");
        assert_eq!(s.anno_all_visible, Some(false), "{ext}");
        assert!(s.scales.iter().any(|sc| sc.name == "1:50"), "{ext}");
        assert!(s.scales.iter().any(|sc| sc.name == "1:100"), "{ext}");

        let both = s
            .objects
            .iter()
            .find(|o| o.id.ends_with("/anno-both"))
            .unwrap_or_else(|| panic!("{ext}: missing anno-both"));
        assert_eq!(both.scales, vec!["1:50", "1:100"], "{ext}");

        let only50 = s
            .objects
            .iter()
            .find(|o| o.id.ends_with("/anno-50-only"))
            .unwrap();
        assert_eq!(only50.scales, vec!["1:50"], "{ext}");

        // Verify paper height of `anno-both` roundtrips back to 3.0 mm (not 150.0)!
        let q_find = query_with(
            &QueryRequest {
                source: out_path.clone(),
                sheet: None,
                query: Query::Find(Selection {
                    id: Some("plan/anno-both".to_owned()),
                    ..Default::default()
                }),
            },
            &settings,
        )
        .unwrap();
        let Answer::Find { objects, .. } = q_find.answer else {
            panic!("expected Find");
        };
        let Object::Text(t) = &objects[0].object else {
            panic!("expected Text");
        };
        assert!(
            (t.height - 3.0).abs() < 1e-4,
            "{ext}: expected paper height 3.0, got {}",
            t.height
        );

        // Verify viewport `annotation_scale` roundtrips in `query layout`!
        let q_lay = query_with(
            &QueryRequest {
                source: out_path,
                sheet: None,
                query: Query::Layout,
            },
            &settings,
        )
        .unwrap();
        let Answer::Layout { layouts, .. } = q_lay.answer else {
            panic!("expected Layout");
        };
        let vp_scales: Vec<Option<String>> = layouts[0]
            .viewports
            .iter()
            .map(|v| v.annotation_scale.clone())
            .collect();
        assert_eq!(
            vp_scales,
            vec![Some("1:50".to_owned()), Some("1:100".to_owned())],
            "{ext}"
        );
    }

    // Plain non-annotative sheet omits annotation_scale metadata from display_list.
    let plain_path = dir.join("plain.yaml");
    fs::write(
        &plain_path,
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [10, 0, 0]}\n",
    )
    .unwrap();
    let plain_dl = display_list_with(&DisplayListRequest::new(&plain_path), &settings).unwrap();
    assert_eq!(plain_dl.annotation_scale, None);
    assert!(plain_dl.annotation_scales.is_empty());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn change_set_cannoscale_and_objectscale_on_drawing_set_and_undo() {
    let dir = temp_dir("set-ops");
    fs::write(
        dir.join("set.yaml"),
        "units: mm\noutput: out\nrows: rows.yaml\n",
    )
    .unwrap();
    fs::write(
        dir.join("rows.yaml"),
        "rows:\n  - sheet: S-01\n    template: tmpl.yaml\n",
    )
    .unwrap();
    let tmpl_path = dir.join("tmpl.yaml");
    let initial_tmpl = r#"units: mm
annotation_scales:
  '1:1': [1, 1]
  '1:50': [1, 50]
annotation_scale: '1:50'
objects:
  - text: {id: note, contents: LABEL, position: [10, 10, 0], height: 2.5, annotative: true, annotation_scales: ['1:50']}
  - text: {id: plain, contents: PLAIN, position: [10, 20, 0], height: 2.5}
  - line: {id: edge, from: [0, 0, 0], to: [50, 0, 0]}
"#;
    fs::write(&tmpl_path, initial_tmpl).unwrap();
    let settings = no_system_fonts();

    // Apply CANNOSCALE + OBJECTSCALE via change set!
    let cs = json!({
        "operations": [
            {
                "op": "CANNOSCALE",
                "sheet": "S-01",
                "add_scales": { "1:20": [1, 20] },
                "scale": "1:20",
                "anno_all_visible": true
            },
            {
                "op": "OBJECTSCALE",
                "target": { "id": "S-01/note" },
                "add": ["1:20"]
            },
            {
                "op": "OBJECTSCALE",
                "target": { "id": "S-01/plain" },
                "annotative": true,
                "scales": ["1:20", "1:50"]
            }
        ]
    });
    let rep = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: cs,
            dry_run: false,
        },
        &settings,
    );
    assert!(rep.ok, "{rep:?}");
    let entry = rep
        .result
        .as_ref()
        .and_then(|r| r.journal_entry)
        .expect("journal entry");

    // Verify query annotation-scales reflects every change.
    let q = query_with(
        &QueryRequest {
            source: dir.clone(),
            sheet: None,
            query: Query::AnnotationScales,
        },
        &settings,
    )
    .unwrap();
    let Answer::AnnotationScales {
        count,
        annotation_scales,
    } = q.answer
    else {
        panic!("expected AnnotationScales");
    };
    assert_eq!(count, 2);
    let s = &annotation_scales[0];
    assert_eq!(s.current, "1:20");
    assert_eq!(s.anno_all_visible, Some(true));
    assert!(s.scales.iter().any(|sc| sc.name == "1:20"));
    let note = s.objects.iter().find(|o| o.id == "S-01/note").unwrap();
    assert_eq!(note.scales, vec!["1:50", "1:20"]);
    let plain = s.objects.iter().find(|o| o.id == "S-01/plain").unwrap();
    assert_eq!(plain.scales, vec!["1:20", "1:50"]);

    // Refuses OBJECTSCALE on a non-annotative kind (line).
    let bad = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [
                    { "op": "OBJECTSCALE", "target": { "id": "S-01/edge" }, "annotative": true }
                ]
            }),
            dry_run: true,
        },
        &settings,
    );
    assert!(!bad.ok);

    // Undo restores template byte-for-byte!
    let undone = undo(&UndoRequest {
        set: dir.clone(),
        change_set: entry,
        dry_run: false,
    });
    assert!(undone.ok, "{undone:?}");
    assert_eq!(fs::read_to_string(&tmpl_path).unwrap(), initial_tmpl);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn original_dwg_revision_preserves_annotative_objects_and_supports_cannoscale_and_objectscale() {
    let dir = temp_dir("revise");
    fs::write(dir.join("blocks.yaml"), BLOCKS_YAML).unwrap();
    let sheet_path = dir.join("plan.yaml");
    fs::write(&sheet_path, multi_viewport_sheet(Some(false))).unwrap();
    let orig_dwg = dir.join("plan.dwg");
    let settings = no_system_fonts();

    build_with_format(
        &BuildRequest {
            description: sheet_path,
            output: Some(orig_dwg.clone()),
            overwrite: true,
        },
        Some(DrawingFormat::Dwg),
        &settings,
    )
    .unwrap();

    // 1. Unrelated edit (MOVE room) leaves annotative objects untouched with 0 unexpected changes.
    let rev1_out = dir.join("plan.rev1.dwg");
    let rev1_reply = revise_with(
        &ReviseRequest {
            set: orig_dwg.clone(),
            change_set: json!({
                "operations": [
                    { "op": "MOVE", "target": { "id": "plan/room" }, "by": [10, 0, 0] }
                ]
            }),
            dry_run: false,
            output: Some(rev1_out.clone()),
            allow_loss: false,
            expected_original_sha256: None,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(rev1_reply.ok, "{rev1_reply:?}");
    let rev1 = rev1_reply.result.unwrap();
    assert!(
        rev1.unexpected_changes.is_empty(),
        "unexpected changes: {:?}",
        rev1.unexpected_changes
    );
    assert!(rev1.requested_failures.is_empty());

    // 2. CANNOSCALE + OBJECTSCALE on original DWG with 0 unexpected changes!
    let rev2_out = dir.join("plan.rev2.dwg");
    let rev2_reply = revise_with(
        &ReviseRequest {
            set: orig_dwg,
            change_set: json!({
                "operations": [
                    {
                        "op": "CANNOSCALE",
                        "add_scales": { "1:25": [1, 25] },
                        "scale": "1:25",
                        "anno_all_visible": true
                    },
                    {
                        "op": "OBJECTSCALE",
                        "target": { "id": "plan/anno-50-only" },
                        "add": ["1:25"]
                    },
                    {
                        "op": "OBJECTSCALE",
                        "target": { "id": "plan/anno-both" },
                        "annotative": false
                    }
                ]
            }),
            dry_run: false,
            output: Some(rev2_out.clone()),
            allow_loss: false,
            expected_original_sha256: None,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(rev2_reply.ok, "{rev2_reply:?}");
    let rev2 = rev2_reply.result.unwrap();
    assert!(
        rev2.unexpected_changes.is_empty(),
        "unexpected changes: {:?}",
        rev2.unexpected_changes
    );
    assert!(rev2.requested_failures.is_empty());

    let q = query_with(
        &QueryRequest {
            source: rev2_out,
            sheet: None,
            query: Query::AnnotationScales,
        },
        &settings,
    )
    .unwrap();
    let Answer::AnnotationScales {
        count,
        annotation_scales,
    } = q.answer
    else {
        panic!("expected AnnotationScales");
    };
    // Started with 7 annotative objects; turned anno-both off -> 6 remain.
    assert_eq!(count, 6, "objects: {:?}", annotation_scales[0].objects);
    let s = &annotation_scales[0];
    assert_eq!(s.current, "1:25");
    assert_eq!(s.anno_all_visible, Some(true));
    assert!(s.scales.iter().any(|sc| sc.name == "1:25"));
    let only50 = s
        .objects
        .iter()
        .find(|o| o.id.ends_with("/anno-50-only"))
        .unwrap();
    assert_eq!(only50.scales, vec!["1:50", "1:25"]);
    assert!(!s.objects.iter().any(|o| o.id.ends_with("/anno-both")));

    let _ = fs::remove_dir_all(&dir);
}
