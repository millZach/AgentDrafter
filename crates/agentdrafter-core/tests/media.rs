//! Raster images (`IMAGE` / `IMAGEATTACH`) and PDF underlays (`PDFUNDERLAY` /
//! `PDFATTACH`, #91): building, reading, drawing (PNG, PDF, DisplayList),
//! querying, checking, stamping, revising and change-set operations on made-up
//! drawings, images and PDFs.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_with_format, change_with, check_stale, check_with, decode_png_rgba, display_list_with,
    import_blocks, operations, plot_with, query_with, read_dwg, read_original, render_with,
    revise_with, sheet_description_schema, undo, Answer, BuildRequest, ChangeRequest, CheckRequest,
    DisplayListRequest, DrawingFormat, ImportBlocksRequest, MediaKind, Object, PlotRequest, Query,
    QueryRequest, ReadRequest, RenderRequest, ReviseRequest, Selection, SelectionBox, Settings,
    Shape, SnapKind, SnapRef, StaleRequest, UndoRequest, WarningCode, WipeoutFrame,
};
use serde_json::{json, Value};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("agentdrafter-media-{}", std::process::id()))
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn no_sys_fonts() -> Settings {
    Settings::default().without_system_fonts()
}

fn build_file(yaml_path: &Path, out_path: &Path, format: DrawingFormat) {
    let req = BuildRequest {
        description: yaml_path.to_path_buf(),
        output: Some(out_path.to_path_buf()),
        overwrite: true,
    };
    build_with_format(&req, Some(format), &no_sys_fonts())
        .unwrap_or_else(|e| panic!("build {} failed: {}", yaml_path.display(), e.message));
}

fn revise_req(dwg: &Path, ops: Value) -> ReviseRequest {
    ReviseRequest {
        set: dwg.to_path_buf(),
        change_set: json!({ "operations": ops }),
        dry_run: false,
        allow_loss: false,
        output: None,
        no_pdf: true,
        plot_style: None,
        expected_original_sha256: None,
    }
}

fn change_req(set: &Path, ops: Value) -> ChangeRequest {
    ChangeRequest {
        set: set.to_path_buf(),
        change_set: json!({ "operations": ops }),
        dry_run: false,
    }
}

/// Writes a made-up solid/patterned RGBA PNG image of `w x h` pixels.
fn write_png(path: &Path, w: u32, h: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) {
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            rgba.extend_from_slice(&pixel(x, y));
        }
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().unwrap();
        writer.write_image_data(&rgba).unwrap();
    }
    fs::write(path, out).unwrap();
}

/// Writes a made-up RGB JPEG image using `image`.
fn write_jpeg(path: &Path, w: u32, h: u32, rgb: [u8; 3]) {
    let mut img = image::RgbImage::new(w, h);
    for p in img.pixels_mut() {
        *p = image::Rgb(rgb);
    }
    img.save_with_format(path, image::ImageFormat::Jpeg)
        .unwrap();
}

/// Generates a made-up 2-page PDF via AgentDrafter's own `plot` command:
/// page 1 has a solid dark hatch on the left half of ISO A4 landscape, while
/// page 2 has a solid dark hatch on the right half (`x = 160..280`), so
/// rendering either page is immediately verifiable by pixel location.
fn write_two_page_pdf(dir: &Path, name: &str) -> PathBuf {
    let yaml = dir.join(format!("{name}.source.yaml"));
    let pdf = dir.join(name);
    fs::write(
        &yaml,
        r#"units: mm
layouts:
  - name: Page1
    paper: ISO A4 landscape
  - name: Page2
    paper: ISO A4 landscape
objects:
  - hatch:
      id: p1-bar
      space: Page1
      pattern: SOLID
      boundary:
        - polyline:
            points: [[15, 30, 0], [135, 30, 0], [135, 180, 0], [15, 180, 0]]
  - hatch:
      id: p2-bar
      space: Page2
      pattern: SOLID
      boundary:
        - polyline:
            points: [[160, 30, 0], [280, 30, 0], [280, 180, 0], [160, 180, 0]]
"#,
    )
    .unwrap();
    let req = PlotRequest {
        source: yaml,
        output: Some(pdf.clone()),
        plot_style: None,
        overwrite: true,
    };
    plot_with(&req, &no_sys_fonts()).expect("plot made-up 2-page PDF");
    assert!(pdf.is_file());
    pdf
}

#[test]
fn build_and_roundtrip_images_and_pdf_underlays_in_dwg_and_dxf() {
    let dir = scratch("roundtrip");
    write_png(&dir.join("logo.png"), 200, 100, |x, _| {
        if x < 100 {
            [220, 30, 30, 255]
        } else {
            [30, 30, 220, 128]
        }
    });
    write_jpeg(&dir.join("photo.jpg"), 80, 40, [40, 180, 60]);
    let _pdf = write_two_page_pdf(&dir, "markup.pdf");

    fs::write(
        dir.join("blocks.yaml"),
        r#"units: mm
blocks:
  TITLE:
    objects:
      - line: {from: [0, 0, 0], to: [60, 0, 0]}
      - image: {path: logo.png, position: [2, 2, 0], width: 20}
"#,
    )
    .unwrap();

    let sheet_yaml = dir.join("sheet.yaml");
    fs::write(
        &sheet_yaml,
        r#"units: mm
paper: [200, 140]
image_frame: on
block_library: blocks.yaml
objects:
  - pdf_underlay:
      id: markup-p1
      path: markup.pdf
      page: "1"
      position: [10, 10, 0]
      scale: 1.0
      rotation: 0
      contrast: 85
      fade: 20
      monochrome: false
      clip: [[15, 15, 0], [95, 15, 0], [95, 65, 0], [15, 65, 0]]
      clip_inverted: true
  - image:
      id: photo-1
      path: photo.jpg
      position: [110, 20, 0]
      width: 60
      rotation: 15
      brightness: 55
      contrast: 60
      fade: 10
      clip: [[112, 22, 0], [160, 30, 0], [150, 60, 0], [108, 45, 0]]
  - insert:
      id: tb
      block: TITLE
      position: [120, 90, 0]
"#,
    )
    .unwrap();

    for (ext, fmt) in [
        ("dwg", DrawingFormat::Dwg),
        ("ascii.dxf", DrawingFormat::Dxf),
        ("bin.dxf", DrawingFormat::DxfBinary),
    ] {
        let out = dir.join(format!("sheet.{ext}"));
        build_file(&sheet_yaml, &out, fmt);
        let read = read_dwg(&out).unwrap_or_else(|e| panic!("read {ext}: {}", e.message));
        assert_eq!(read.image_frame, WipeoutFrame::On, "{ext}");
        assert!(read.not_read.is_empty(), "{ext}: {:?}", read.not_read);
        assert_eq!(read.objects.len(), 3, "{ext}: {:?}", read.objects);

        match &read.objects[0] {
            Object::PdfUnderlay(p) => {
                assert_eq!(p.path, "markup.pdf");
                assert_eq!(p.page, "1");
                assert!((p.position.x - 10.0).abs() < 1e-6);
                assert!((p.position.y - 10.0).abs() < 1e-6);
                assert!((p.scale - 1.0).abs() < 1e-6);
                assert_eq!(p.contrast, 85);
                assert_eq!(p.fade, 20);
                assert!(!p.monochrome);
                assert_eq!(p.clip.as_ref().map(Vec::len), Some(4));
                assert!(p.clip_inverted);
            }
            other => panic!("{ext}: expected PdfUnderlay, got {other:?}"),
        }
        match &read.objects[1] {
            Object::RasterImage(img) => {
                assert_eq!(img.path, "photo.jpg");
                assert!((img.position.x - 110.0).abs() < 1e-6);
                assert!((img.position.y - 20.0).abs() < 1e-6);
                assert!((img.width - 60.0).abs() < 1e-4);
                // 80x40 JPEG -> height should be 30.0
                assert!((img.effective_height(None) - 30.0).abs() < 1e-3);
                assert!((img.rotation - 15.0).abs() < 1e-4);
                assert_eq!(img.brightness, 55);
                assert_eq!(img.contrast, 60);
                assert_eq!(img.fade, 10);
                assert_eq!(img.clip.as_ref().map(Vec::len), Some(4));
                assert!(!img.clip_inverted);
            }
            other => panic!("{ext}: expected RasterImage, got {other:?}"),
        }

        // Verify the TITLE block retained its embedded raster image.
        let title_block = read
            .blocks
            .iter()
            .find(|b| b.name == "TITLE")
            .expect("TITLE block");
        assert!(
            title_block
                .objects
                .iter()
                .any(|o| matches!(o, Object::RasterImage(i) if i.path == "logo.png")),
            "{ext}: TITLE block missing logo.png"
        );
    }
}

#[test]
fn render_plot_display_list_query_check_and_stale_with_images_and_pdf() {
    let dir = scratch("pipeline");
    // Pure red PNG (64x32 -> aspect 2:1) with a transparent right half.
    let logo_path = dir.join("logo.png");
    write_png(&logo_path, 64, 32, |x, _| {
        if x < 32 {
            [240, 20, 20, 255]
        } else {
            [20, 20, 240, 0]
        }
    });
    let pdf_path = write_two_page_pdf(&dir, "underlay.pdf");

    let yaml = dir.join("sheet.yaml");
    fs::write(
        &yaml,
        r#"units: mm
paper: [140, 100]
image_frame: display
objects:
  - pdf_underlay:
      id: ref-p2
      path: underlay.pdf
      page: "2"
      position: [10, 10, 0]
      scale: 0.35
  - image:
      id: logo
      path: logo.png
      position: [15, 70, 0]
      width: 40
  - line:
      id: border-line
      from: [0, 0, 0]
      to: [140, 100, 0]
"#,
    )
    .unwrap();

    // 1. Display list carries two embedded images + two Shape::Image items.
    let dl = display_list_with(&DisplayListRequest::new(&yaml), &no_sys_fonts()).unwrap();
    assert!(dl.unresolved_media.is_empty());
    assert_eq!(dl.display_list.images.len(), 2);
    let image_items: Vec<_> = dl
        .display_list
        .items
        .iter()
        .filter(|i| matches!(i.shape, Shape::Image { .. }))
        .collect();
    assert_eq!(image_items.len(), 2);

    // 2. Render PNG: verify red pixels from logo and dark ink from PDF page 2's right bar appear.
    let png_out = dir.join("sheet.png");
    let mut render_req = RenderRequest::new(&yaml);
    render_req.output = Some(png_out.clone());
    render_req.width = Some(560);
    render_req.overwrite = true;
    let render_rep = render_with(&render_req, &no_sys_fonts()).unwrap();
    assert!(render_rep.unresolved_media.is_empty());
    let png_bytes = fs::read(&png_out).unwrap();
    let (w, h, rgba) = decode_png_rgba(&png_bytes).expect("decode rendered PNG");
    assert!(w >= 500 && h > 0);
    let mut red_px = 0usize;
    let mut dark_right_px = 0usize;
    for y in 0..h {
        for x in 0..w {
            let idx = ((y * w + x) * 4) as usize;
            let [r, g, b, _] = [rgba[idx], rgba[idx + 1], rgba[idx + 2], rgba[idx + 3]];
            if r > 180 && g < 80 && b < 80 {
                red_px += 1;
            }
            // Page 2 dark hatch sits around x in 65..105 mm (roughly middle-right, lower half).
            if x > w / 2 && y > h / 2 && r < 40 && g < 40 && b < 40 {
                dark_right_px += 1;
            }
        }
    }
    assert!(
        red_px > 100,
        "rendered PNG should contain red pixels from logo.png, found {red_px}"
    );
    assert!(
        dark_right_px > 500,
        "rendered PNG should contain dark pixels from page 2 of underlay.pdf, found {dark_right_px}"
    );

    // 3. Plot PDF: verify PDF embeds /Subtype /Image XObjects + /SMask for alpha PNG/PDF.
    let pdf_out = dir.join("sheet.pdf");
    let plot_rep = plot_with(
        &PlotRequest {
            source: yaml.clone(),
            output: Some(pdf_out.clone()),
            plot_style: None,
            overwrite: true,
        },
        &no_sys_fonts(),
    )
    .unwrap();
    assert!(plot_rep.unresolved_media.is_empty());
    let pdf_bytes = fs::read(&pdf_out).unwrap();
    let pdf_ascii = String::from_utf8_lossy(&pdf_bytes);
    assert!(
        pdf_ascii.contains("/Subtype /Image"),
        "plotted PDF must embed raster image XObjects"
    );
    assert!(
        pdf_ascii.contains("/SMask"),
        "plotted PDF must emit an /SMask for transparent PNG/PDF pixels"
    );

    // 4. Stale detection: fresh right after render, stale after modifying logo.png!
    let fresh = check_stale(&StaleRequest {
        output: png_out.clone(),
        source: None,
    })
    .unwrap();
    assert!(!fresh.stale, "fresh render should not be stale: {fresh:?}");
    write_png(&logo_path, 64, 32, |_, _| [10, 200, 10, 255]);
    let after_img_edit = check_stale(&StaleRequest {
        output: png_out.clone(),
        source: None,
    })
    .unwrap();
    assert!(
        after_img_edit.stale,
        "editing attached PNG must mark rendered PNG stale"
    );

    let after_pdf_stale = check_stale(&StaleRequest {
        output: pdf_out.clone(),
        source: None,
    })
    .unwrap();
    assert!(
        after_pdf_stale.stale,
        "logo.png edit also stales plotted PDF"
    );
    let _ = pdf_path;

    // 5. Query: geometry (area/extents), snaps (center), window selection.
    let geom = query_with(
        &QueryRequest {
            source: yaml.clone(),
            sheet: None,
            query: Query::Geometry(Selection {
                id: Some("sheet/logo".to_owned()),
                ..Selection::default()
            }),
        },
        &no_sys_fonts(),
    )
    .unwrap();
    match geom.answer {
        Answer::Geometry { objects, .. } => {
            assert_eq!(objects.len(), 1);
            // 40 mm wide x 20 mm tall (64x32 image) => area 800 mm^2, perimeter 120 mm
            assert!((objects[0].measures.area.unwrap() - 800.0).abs() < 1e-4);
            assert!((objects[0].measures.length.unwrap() - 120.0).abs() < 1e-4);
        }
        other => panic!("unexpected geometry answer: {other:?}"),
    }

    let snaps = query_with(
        &QueryRequest {
            source: yaml.clone(),
            sheet: None,
            query: Query::Snap(SnapRef {
                id: Some("sheet/logo".to_owned()),
                name: None,
                snap: Some(SnapKind::Center),
                which: None,
                with: None,
                from: None,
            }),
        },
        &no_sys_fonts(),
    )
    .unwrap();
    match snaps.answer {
        Answer::Snap { points, .. } => {
            assert_eq!(points.len(), 1);
            assert!((points[0].at.x - 35.0).abs() < 1e-4);
            assert!((points[0].at.y - 80.0).abs() < 1e-4);
        }
        other => panic!("unexpected snap answer: {other:?}"),
    }

    let found = query_with(
        &QueryRequest {
            source: yaml.clone(),
            sheet: None,
            query: Query::Find(Selection {
                window: Some(SelectionBox([[14.0, 69.0], [56.0, 91.0]])),
                ..Selection::default()
            }),
        },
        &no_sys_fonts(),
    )
    .unwrap();
    match found.answer {
        Answer::Find { objects, .. } => {
            assert!(
                objects.iter().any(|o| o.id == "sheet/logo"),
                "window selection should select logo: {objects:?}"
            );
        }
        other => panic!("unexpected find answer: {other:?}"),
    }

    // 6. Checker passes (nothing off-sheet, exact paper size).
    let chk = check_with(
        &CheckRequest {
            source: yaml,
            sheets: Vec::new(),
        },
        &no_sys_fonts(),
    )
    .unwrap();
    assert!(chk.passed, "check failed: {:?}", chk.findings);
    assert!(chk.unresolved_media.is_empty());
}

#[test]
fn missing_media_and_bad_pdf_page_warn_without_failing() {
    let dir = scratch("missing");
    let _pdf = write_two_page_pdf(&dir, "two.pdf");
    let yaml = dir.join("missing.yaml");
    fs::write(
        &yaml,
        r#"units: mm
paper: [100, 100]
objects:
  - image: {id: gone-img, path: does-not-exist.png, position: [5, 5, 0], width: 20}
  - pdf_underlay: {id: bad-page, path: two.pdf, page: "99", position: [30, 5, 0], scale: 10}
"#,
    )
    .unwrap();

    let dl = display_list_with(&DisplayListRequest::new(&yaml), &no_sys_fonts()).unwrap();
    assert_eq!(dl.unresolved_media.len(), 2);
    assert_eq!(dl.unresolved_media[0].kind, MediaKind::Image);
    assert_eq!(dl.unresolved_media[1].kind, MediaKind::PdfUnderlay);
    assert_eq!(dl.unresolved_media[1].page.as_deref(), Some("99"));

    // Build + read_original should report unresolved_media cleanly too.
    let dwg = dir.join("missing.dwg");
    build_file(&yaml, &dwg, DrawingFormat::Dwg);
    let read_rep = read_original(&ReadRequest {
        source: dwg,
        mirror: None,
        no_mirror: true,
    })
    .unwrap();
    assert_eq!(read_rep.unresolved_media.len(), 2);

    // Render still succeeds and emits a placeholder box instead of crashing.
    let png_out = dir.join("missing.png");
    let mut render_req = RenderRequest::new(&yaml);
    render_req.output = Some(png_out.clone());
    render_req.width = Some(300);
    render_req.overwrite = true;
    let render_rep = render_with(&render_req, &no_sys_fonts()).unwrap();
    assert_eq!(render_rep.unresolved_media.len(), 2);
    assert!(png_out.is_file());
    let _ = WarningCode::UnresolvedMedia;
}

#[test]
fn change_set_imageattach_pdfattach_transform_and_undo_on_set_and_original_dwg() {
    let dir = scratch("change");
    write_png(&dir.join("badge.png"), 40, 20, |_, _| [200, 40, 40, 255]);
    let _pdf = write_two_page_pdf(&dir, "base.pdf");

    // 1. Drawing set with IMAGEATTACH, PDFATTACH (draw_order: back), move/rotate/scale, and undo.
    let set_dir = dir.join("set");
    fs::create_dir_all(set_dir.join("templates")).unwrap();
    fs::copy(dir.join("badge.png"), set_dir.join("badge.png")).unwrap();
    fs::copy(dir.join("base.pdf"), set_dir.join("base.pdf")).unwrap();
    fs::write(
        set_dir.join("set.yaml"),
        "units: mm\nrows: rows.yaml\noutput: out\n",
    )
    .unwrap();
    fs::write(
        set_dir.join("templates/a4.yaml"),
        "units: mm\npaper: [200, 150]\nobjects:\n  - line: {id: border, from: [0, 0, 0], to: [200, 0, 0]}\n",
    )
    .unwrap();
    fs::write(
        set_dir.join("rows.yaml"),
        "rows:\n  - sheet: S-01\n    template: templates/a4.yaml\n",
    )
    .unwrap();

    let attached = change_with(
        &change_req(
            &set_dir,
            json!([
                {
                    "op": "IMAGEATTACH",
                    "sheet": "S-01",
                    "id": "logo",
                    "path": "badge.png",
                    "position": [140, 100, 0],
                    "width": 30
                },
                {
                    "op": "PDFATTACH",
                    "sheet": "S-01",
                    "id": "bg",
                    "path": "base.pdf",
                    "page": "1",
                    "position": [10, 10, 0],
                    "scale": 1.0,
                    "fade": 40,
                    "draw_order": "back"
                }
            ]),
        ),
        &no_sys_fonts(),
    );
    assert!(attached.ok, "{attached:?}");
    let dwg_s01 = set_dir.join("out/S-01.dwg");
    let read_after_attach = read_dwg(&dwg_s01).unwrap();
    // Template line draws first (0); `bg` with `draw_order: back` precedes `logo` in the row (1 vs 2).
    assert_eq!(read_after_attach.objects.len(), 3);
    assert!(matches!(read_after_attach.objects[0], Object::Line(_)));
    assert!(matches!(
        read_after_attach.objects[1],
        Object::PdfUnderlay(_)
    ));
    assert!(matches!(
        read_after_attach.objects[2],
        Object::RasterImage(_)
    ));

    // Move, rotate, and scale the attached image in the drawing set.
    let moved = change_with(
        &change_req(
            &set_dir,
            json!([
                {"op": "move", "target": {"id": "S-01/logo"}, "by": [10, 5, 0]},
                {"op": "rotate", "target": {"id": "S-01/logo"}, "base_point": [150, 105, 0], "angle": 90},
                {"op": "scale", "target": {"id": "S-01/logo"}, "base_point": [150, 105, 0], "factor": 2.0}
            ]),
        ),
        &no_sys_fonts(),
    );
    assert!(moved.ok, "{moved:?}");
    let read_transformed = read_dwg(&dwg_s01).unwrap();
    match &read_transformed.objects[2] {
        Object::RasterImage(img) => {
            assert!((img.position.x - 150.0).abs() < 1e-5);
            assert!((img.position.y - 105.0).abs() < 1e-5);
            assert!((img.rotation - 90.0).abs() < 1e-4);
            assert!((img.width - 60.0).abs() < 1e-4);
        }
        other => panic!("expected RasterImage, got {other:?}"),
    }

    // Undo restores the previous state byte for byte.
    let undone = undo(&UndoRequest {
        set: set_dir.clone(),
        change_set: moved.result.as_ref().unwrap().journal_entry.unwrap(),
        dry_run: false,
    });
    assert!(undone.ok, "{undone:?}");
    let read_undone = read_dwg(&dwg_s01).unwrap();
    match &read_undone.objects[2] {
        Object::RasterImage(img) => {
            assert!((img.position.x - 140.0).abs() < 1e-5);
            assert!((img.width - 30.0).abs() < 1e-4);
            assert!((img.rotation - 0.0).abs() < 1e-4);
        }
        other => panic!("expected RasterImage, got {other:?}"),
    }

    // 2. Original DWG revision: attach IMAGE + PDFUNDERLAY directly onto an existing DWG,
    //    and verify original content is preserved verbatim.
    let orig_yaml = dir.join("orig.yaml");
    let orig_dwg = dir.join("orig.dwg");
    fs::write(
        &orig_yaml,
        "units: mm\nobjects:\n  - line: {id: keep-me, from: [0, 0, 0], to: [100, 0, 0]}\n",
    )
    .unwrap();
    build_file(&orig_yaml, &orig_dwg, DrawingFormat::Dwg);

    let rev = revise_with(
        &revise_req(
            &orig_dwg,
            json!([
                {
                    "op": "IMAGEATTACH",
                    "id": "rev-img",
                    "path": "badge.png",
                    "position": [10, 20, 0],
                    "width": 25
                },
                {
                    "op": "PDFATTACH",
                    "id": "rev-pdf",
                    "path": "base.pdf",
                    "page": "2",
                    "position": [50, 20, 0],
                    "scale": 0.5,
                    "draw_order": "back"
                }
            ]),
        ),
        &no_sys_fonts(),
    );
    assert!(rev.ok, "{rev:?}");
    let rev_dwg = dir.join(&rev.result.unwrap().revision.unwrap().file);
    let rev_read = read_dwg(&rev_dwg).unwrap();
    assert!(rev_read
        .objects
        .iter()
        .any(|o| matches!(o, Object::RasterImage(i) if i.path == "badge.png")));
    assert!(rev_read
        .objects
        .iter()
        .any(|o| matches!(o, Object::PdfUnderlay(p) if p.path == "base.pdf" && p.page == "2")));
    assert!(rev_read
        .objects
        .iter()
        .any(|o| matches!(o, Object::Line(_))));

    // Verify operations list & schema include add_image and add_pdf_underlay.
    let ops = operations();
    assert!(ops
        .operations
        .iter()
        .any(|o| o.name == "add_image" && o.alias == "IMAGEATTACH"));
    assert!(ops
        .operations
        .iter()
        .any(|o| o.name == "add_pdf_underlay" && o.alias == "PDFATTACH"));
    let schema = sheet_description_schema();
    assert!(schema["$defs"]["image"].is_object());
    assert!(schema["$defs"]["pdf_underlay"].is_object());
}

#[test]
fn import_blocks_carries_raster_image_inside_block() {
    let dir = scratch("import");
    write_png(&dir.join("stamp.png"), 32, 32, |_, _| [10, 10, 220, 255]);
    fs::write(
        dir.join("blocks.yaml"),
        "units: mm\nblocks:\n  LOGO_BLK:\n    objects:\n      - image: {path: stamp.png, position: [0, 0, 0], width: 15}\n",
    )
    .unwrap();
    let sheet = dir.join("s.yaml");
    let dwg = dir.join("s.dwg");
    fs::write(
        &sheet,
        "units: mm\nblock_library: blocks.yaml\nobjects:\n  - insert: {block: LOGO_BLK, position: [0, 0, 0]}\n",
    )
    .unwrap();
    build_file(&sheet, &dwg, DrawingFormat::Dwg);

    let out_lib = dir.join("imported.yaml");
    let rep = import_blocks(&ImportBlocksRequest {
        drawing: dwg,
        library: out_lib.clone(),
        blocks: vec!["LOGO_BLK".to_owned()],
        replace: true,
        units: None,
        dry_run: false,
    })
    .unwrap();
    assert_eq!(rep.imported.blocks_added.len(), 1);
    let text = fs::read_to_string(&out_lib).unwrap();
    assert!(text.contains("image:"), "{text}");
    assert!(text.contains("stamp.png"), "{text}");
}
