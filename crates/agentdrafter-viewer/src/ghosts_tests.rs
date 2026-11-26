use agentdrafter_core::{Bounds, Color, DisplayList, FillRule, Item, Shape};
use serde_json::json;

use super::*;

fn item(object: usize, shape: Shape) -> Item {
    Item {
        object,
        part: None,
        nested: Vec::new(),
        cell: None,
        viewport: None,
        clip: None,
        clip_boundary: None,
        color: Color(7),
        true_color: None,
        plot_style: None,
        transparency: 0,
        shape,
    }
}

fn stroke(object: usize, run: Vec<[f64; 2]>) -> Item {
    item(
        object,
        Shape::Stroke {
            lineweight: 0.35,
            runs: vec![run],
        },
    )
}

fn b(min: [f64; 2], max: [f64; 2]) -> Bounds {
    Bounds { min, max }
}

/// The text of a `display-list` reply with a change set: object 0, a box,
/// moved 2 to the right; object 1, a line, added; object 2, a letter
/// drawn as a fill, changed; and an object deleted.
fn reply(layout: Option<&str>) -> String {
    let list = DisplayList {
        items: vec![
            stroke(0, vec![[2.0, 0.0], [3.0, 0.0], [3.0, 1.0]]),
            stroke(1, vec![[5.0, 5.0], [6.0, 5.0]]),
            item(
                2,
                Shape::Fill {
                    rule: FillRule::NonZero,
                    contours: vec![vec![[8.0, 8.0], [8.5, 8.0], [8.5, 8.5]]],
                },
            ),
        ],
        ..DisplayList::default()
    };
    json!({
        "ok": true,
        "result": {
            "source": "demo-plant", "sheet": "DP-LD-101", "sheets": ["DP-LD-101"],
            "layout": layout, "millimetres_per_unit": 25.4,
            "objects": [{"id": "DP-LD-101/box"}, {"id": "DP-LD-101/new"}, {"id": "DP-LD-101/tag"}],
            "preview": {
                "change": {}, "objects": [],
                "added": [1], "changed": [0, 2],
                "was": [
                    {"id": "DP-LD-101/box", "how": "changed", "items": [stroke(9, vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]])]},
                    {"id": "DP-LD-101/gone", "how": "deleted", "items": [stroke(9, vec![[10.0, 1.0], [11.0, 2.0]])]}
                ]
            },
            "display_list": list,
        }
    })
    .to_string()
}

#[test]
fn a_ghost_keeps_what_the_proposal_adds_changes_moves_and_deletes() {
    let g = Ghost::of(3, &reply(None), false).unwrap();
    assert_eq!(g.proposal, 3);
    assert_eq!(g.sheet, "DP-LD-101");
    // What it adds, then what it changes, each item's lines: the letter a
    // fill, drawn solid.
    assert_eq!(g.runs.len(), 3);
    assert_eq!(g.runs[0].lines, [[[5.0, 5.0], [6.0, 5.0]]]);
    assert_eq!(g.runs[0].pen, Some(0.35));
    assert_eq!(g.runs[0].clip, None);
    assert!(!g.runs[0].fill && g.runs[2].fill);
    assert_eq!(g.runs[2].pen, None);
    assert_eq!(
        g.boxes.len(),
        3,
        "a box round each object it adds or changes"
    );
    assert_eq!(
        g.struck,
        [b([10.0, 1.0], [11.0, 2.0])],
        "what it deletes, struck"
    );
    assert_eq!(g.moves, [[[0.5, 0.5], [2.5, 0.5]]], "the box only moved");
    // Its card goes by all it marks.
    assert_eq!(g.extent(), Some(b([2.0, 0.0], [11.0, 8.5])));
}

#[test]
fn a_ghost_is_drawn_on_its_own_sheet_and_layout_only() {
    let g = Ghost::of(1, &reply(Some("Layout1")), true).unwrap();
    assert!(g.refused);
    assert!(g.on("dp-ld-101", Some("LAYOUT1")));
    assert!(!g.on("DP-LD-101", None));
    assert!(!g.on("DP-LD-102", Some("Layout1")));
    let g = Ghost::of(1, &reply(None), false).unwrap();
    assert!(g.on("DP-LD-101", None));
}

#[test]
fn a_reply_with_no_preview_or_a_refusal_gives_no_ghost() {
    let mut plain: serde_json::Value = serde_json::from_str(&reply(None)).unwrap();
    plain["result"].as_object_mut().unwrap().remove("preview");
    assert!(Ghost::of(1, &plain.to_string(), false).is_err());
    let refused =
        json!({"ok": false, "error": {"code": "unknown_name", "message": "no such sheet"}});
    assert_eq!(
        Ghost::of(1, &refused.to_string(), false).unwrap_err(),
        "no such sheet"
    );
}

#[test]
fn a_run_is_dashed_along_its_lines_and_a_short_one_drawn_whole() {
    // Two joined lines, 10 and 10 long, in dashes of 4 with gaps of 2: the
    // pattern runs on round the corner.
    let run = [[0.0, 0.0, 10.0, 0.0], [10.0, 0.0, 10.0, 10.0]];
    let dashes = dash_run(&run, 4.0, 2.0);
    assert_eq!(
        dashes,
        [
            [0.0, 0.0, 4.0, 0.0],
            [6.0, 0.0, 10.0, 0.0],
            [10.0, 2.0, 10.0, 6.0],
            [10.0, 8.0, 10.0, 10.0],
        ]
    );
    // A letter's stroke, shorter than three dashes, is drawn whole.
    let short = [[0.0, 0.0, 5.0, 0.0]];
    assert_eq!(dash_run(&short, 4.0, 2.0), short);
    // A line that starts away from the last one's end starts the pattern
    // again.
    let apart = [[0.0, 0.0, 5.0, 0.0], [20.0, 0.0, 30.0, 0.0]];
    assert_eq!(dash_run(&apart, 4.0, 2.0)[1], [20.0, 0.0, 24.0, 0.0]);
}

#[test]
fn a_run_is_dashed_at_a_fractional_pixel_ratio_and_always_ends() {
    // At 110%, as a Windows laptop shows, a pen of 7 by 4 CSS pixels is
    // 7.7 by 4.4 device pixels, which rounding made the old loop stop on.
    let ratio = 1.1;
    let (dash, gap) = (7.0 * ratio, 4.0 * ratio);
    for pieces in [1usize, 3, 7, 100, 1000] {
        // A 100-pixel stroke, in `pieces` lines end to end.
        let step = 100.0 / pieces as f64;
        let run: Vec<[f64; 4]> = (0..pieces)
            .map(|i| [i as f64 * step, 0.0, (i + 1) as f64 * step, 0.0])
            .collect();
        let dashes = dash_run(&run, dash, gap);
        let drawn: f64 = dashes.iter().map(|[x0, _, x1, _]| x1 - x0).sum();
        // 100 / 12.1 is 8 whole periods and 3.2 into a dash: 64.8 drawn.
        assert!((drawn - 64.8).abs() < 1e-6, "{pieces} pieces: {drawn}");
        assert!(dashes.iter().all(|[x0, _, x1, _]| x1 - x0 <= dash + 1e-9));
        assert!(dashes.iter().all(|[x0, _, x1, _]| x1 >= x0));
    }
    // Lengths that are not whole numbers of anything, and a diagonal.
    let odd = [[0.3, 0.1, 37.31, 52.77], [37.31, 52.77, 101.999, 52.77]];
    let dashes = dash_run(&odd, 7.0 * 1.25, 4.0 * 1.25);
    assert!(!dashes.is_empty() && dashes.len() < 20, "{}", dashes.len());
}

#[test]
fn ghost_runs_divides_segment_budget_evenly_across_copies() {
    // A 30,000-segment polyline copied 3 times would starve copy 3 under a
    // single shared 50,000-segment cap; dividing evenly gives every copy
    // 16,666 segments within the 50,000 total budget (#248).
    let pts: Vec<[f64; 2]> = (0..=30_000).map(|i| [i as f64, 0.0]).collect();
    let runs: Vec<&[[f64; 2]]> = vec![pts.as_slice()];
    let offsets = [[0.0, 10.0], [0.0, 20.0], [0.0, 30.0]];
    let out = ghost_runs(&offsets, &runs, MOST_GHOST_SEGMENTS, |&[dx, dy], p| {
        (p[0] + dx, p[1] + dy)
    });
    assert_eq!(out.len(), 3, "every copy gets geometry");
    let total: usize = out.iter().map(Vec::len).sum();
    assert!(total <= MOST_GHOST_SEGMENTS, "total={total}");
    for (i, &[_, dy]) in offsets.iter().enumerate() {
        assert_eq!(out[i].len(), 16_666);
        assert_eq!(out[i][0][1], dy);
    }
    // Small selections draw every segment of every copy untouched.
    let small = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]];
    let out = ghost_runs(
        &offsets,
        &[&small[..]],
        MOST_GHOST_SEGMENTS,
        |&[dx, dy], p| (p[0] + dx, p[1] + dy),
    );
    assert_eq!(out.len(), 3);
    assert!(out.iter().all(|r| r.len() == 2));
}

#[test]
fn ghost_runs_draws_single_point_dot_runs_for_every_copy() {
    // A dot-only hatch pattern produces 1-point runs; each copy emits visible
    // zero-length dot segments (drawn as square dots by `overlay::triangles`)
    // within the per-copy budget (#259).
    let d1 = [[1.0, 2.0]];
    let d2 = [[3.0, 4.0]];
    let runs: Vec<&[[f64; 2]]> = vec![&d1[..], &d2[..]];
    let offsets = [[10.0, 0.0], [20.0, 0.0]];
    let out = ghost_runs(&offsets, &runs, MOST_GHOST_SEGMENTS, |&[dx, dy], p| {
        (p[0] + dx, p[1] + dy)
    });
    assert_eq!(out.len(), 4);
    assert_eq!(out[0], vec![[11.0, 2.0, 11.0, 2.0]]);
    assert_eq!(out[1], vec![[13.0, 4.0, 13.0, 4.0]]);
    assert_eq!(out[2], vec![[21.0, 2.0, 21.0, 2.0]]);
    assert_eq!(out[3], vec![[23.0, 4.0, 23.0, 4.0]]);
}
