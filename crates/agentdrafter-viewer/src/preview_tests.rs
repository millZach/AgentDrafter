use agentdrafter_core::{Bounds, Color, DisplayList, FillRule, Item, Shape};
use serde_json::json;

use super::*;

fn stroke(object: usize, runs: Vec<Vec<[f64; 2]>>) -> Item {
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
        shape: Shape::Stroke {
            lineweight: 0.35,
            runs,
        },
    }
}

fn list(items: Vec<Item>) -> DisplayList {
    DisplayList {
        items,
        ..DisplayList::default()
    }
}

fn b(min: [f64; 2], max: [f64; 2]) -> Bounds {
    Bounds { min, max }
}

#[test]
fn a_reply_gives_what_it_adds_changes_and_had_and_a_move_gets_its_guide() {
    // Object 0 is a box that moved 2 to the right; object 1 a line added.
    let drawn = list(vec![
        stroke(0, vec![vec![[2.0, 0.0], [3.0, 0.0], [3.0, 1.0]]]),
        stroke(1, vec![vec![[5.0, 5.0], [6.0, 5.0]]]),
    ]);
    let ids = [Some("S/box".to_owned()), Some("S/new".to_owned())];
    let reply = json!({
        "change": {}, "objects": [],
        "added": [1], "changed": [0, 99],
        "was": [
            {"id": "S/box", "how": "changed", "items": [stroke(4, vec![vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]])]},
            {"id": "S/gone", "how": "deleted", "items": [stroke(5, vec![vec![[9.0, 9.0], [9.5, 9.5]]])]}
        ],
        "clouds": [{"rule": "text_on_line", "bounds": {"min": [1.0, 1.0], "max": [2.0, 2.0]}}]
    });
    let p = Preview::of(Some(&reply), &drawn, &ids).unwrap();
    assert_eq!(p.added, [1]);
    assert_eq!(p.changed, [0], "an item not in the list is let go");
    assert_eq!(p.was.len(), 2);
    assert!(!p.was[0].deleted && p.was[1].deleted);
    assert_eq!(p.clouds, [b([1.0, 1.0], [2.0, 2.0])]);
    assert_eq!(p.moves, [[[0.5, 0.5], [2.5, 0.5]]], "the box only moved");
    // The extent takes in the new places, the old and the clouds.
    assert_eq!(p.extent(&drawn), Some(b([0.0, 0.0], [9.5, 9.5])));
    assert_eq!(
        p.marked_objects(&drawn),
        [
            (true, b([5.0, 5.0], [6.0, 5.0])),
            (false, b([2.0, 0.0], [3.0, 1.0]))
        ]
    );
    // No preview, or one that cannot be read, is none.
    assert_eq!(Preview::of(None, &drawn, &ids), None);
    assert_eq!(Preview::of(Some(&json!({"was": 3})), &drawn, &ids), None);
}

#[test]
fn a_changed_object_of_another_size_has_no_guide() {
    assert_eq!(
        moved(b([0.0, 0.0], [1.0, 1.0]), b([0.0, 0.0], [2.0, 1.0])),
        None
    );
    assert_eq!(
        moved(b([0.0, 0.0], [1.0, 1.0]), b([0.0, 0.0], [1.0, 1.0])),
        None,
        "in place"
    );
    assert_eq!(
        moved(b([0.0, 0.0], [1.0, 1.0]), b([0.0, 3.0], [1.0, 4.0])),
        Some([[0.5, 0.5], [0.5, 3.5]])
    );
}

#[test]
fn an_outline_runs_through_a_strokes_runs_and_closes_a_fills_contours() {
    let line = stroke(
        0,
        vec![vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]], vec![[4.0, 4.0]]],
    );
    assert_eq!(
        outline(&line),
        [
            [[0.0, 0.0], [1.0, 0.0]],
            [[1.0, 0.0], [1.0, 1.0]],
            [[4.0, 4.0], [4.0, 4.0]]
        ]
    );
    assert_eq!(pen(&line), Some(0.35));
    let mut fill = line.clone();
    fill.shape = Shape::Fill {
        rule: FillRule::NonZero,
        contours: vec![vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]],
    };
    assert_eq!(outline(&fill).len(), 3, "closed back to its first point");
    assert_eq!(outline(&fill)[2], [[1.0, 1.0], [0.0, 0.0]]);
    assert_eq!(pen(&fill), None);
}

#[test]
fn a_cloud_bulges_outwards_round_its_box_and_meets_itself() {
    let lines = cloud([10.0, 10.0, 50.0, 30.0], 10.0);
    // Every point lies outside the box or on its edge, and the cloud
    // reaches past each side by about half an arc.
    let points: Vec<[f64; 2]> = lines
        .iter()
        .flat_map(|l| [[l[0], l[1]], [l[2], l[3]]])
        .collect();
    let inside = |p: &[f64; 2]| {
        p[0] > 10.0 + 1e-9 && p[0] < 50.0 - 1e-9 && p[1] > 10.0 + 1e-9 && p[1] < 30.0 - 1e-9
    };
    assert!(!points.iter().any(inside), "nothing inside the box");
    let top = points.iter().map(|p| p[1]).fold(f64::MAX, f64::min);
    let left = points.iter().map(|p| p[0]).fold(f64::MAX, f64::min);
    assert!(
        (top - 5.0).abs() < 1e-6 && (left - 5.0).abs() < 1e-6,
        "{top} {left}"
    );
    // One closed path: each line starts where the last ended, and the last
    // ends where the first starts.
    for pair in lines.windows(2) {
        assert!((pair[0][2] - pair[1][0]).abs() < 1e-9 && (pair[0][3] - pair[1][1]).abs() < 1e-9);
    }
    let (first, last) = (lines[0], lines[lines.len() - 1]);
    assert!((last[2] - first[0]).abs() < 1e-9 && (last[3] - first[1]).abs() < 1e-9);
    // A small box still gets two arcs a side.
    assert_eq!(cloud([0.0, 0.0, 2.0, 2.0], 10.0).len(), 4 * 2 * 8);
}

#[test]
fn a_strike_crosses_its_box_and_dashes_break_a_line() {
    assert_eq!(
        strike([0.0, 0.0, 4.0, 2.0]),
        [[0.0, 2.0, 4.0, 0.0], [0.0, 0.0, 4.0, 2.0]]
    );
    assert_eq!(sides([0.0, 0.0, 4.0, 2.0])[1], [4.0, 0.0, 4.0, 2.0]);
    let dashes = dashed([0.0, 0.0, 20.0, 0.0], 4.0, 3.0);
    assert_eq!(dashes.len(), 3);
    assert_eq!(dashes[1], [7.0, 0.0, 11.0, 0.0]);
    assert_eq!(dashes[2], [14.0, 0.0, 18.0, 0.0]);
    assert_eq!(
        dashed([0.0, 0.0, 2.0, 0.0], 4.0, 3.0).len(),
        1,
        "too short to break"
    );
}

#[test]
fn preview_preserves_old_clip_boundaries_and_resolves_viewport_clip_polygons() {
    let circle = vec![[65.0, 40.0], [50.0, 55.0], [35.0, 40.0], [50.0, 25.0]];
    let mut old_item = stroke(0, vec![vec![[38.0, 25.0], [56.0, 55.0]]]);
    old_item.clip = Some(b([35.0, 25.0], [65.0, 55.0]));
    old_item.clip_boundary = Some(0);
    let mut rect_item = stroke(1, vec![vec![[10.0, 10.0], [20.0, 20.0]]]);
    rect_item.clip = Some(b([5.0, 5.0], [25.0, 25.0]));
    let paper_item = stroke(2, vec![vec![[0.0, 0.0], [10.0, 0.0]]]);

    let drawn = list(vec![
        old_item.clone(),
        rect_item.clone(),
        paper_item.clone(),
    ]);
    let ids = [
        Some("S/pad".to_owned()),
        Some("S/rect".to_owned()),
        Some("S/paper".to_owned()),
    ];
    let reply = json!({
        "change": {}, "objects": [],
        "changed": [0],
        "was": [{"id": "S/pad", "how": "changed", "items": [old_item]}],
        "was_clip_boundaries": [circle]
    });
    let p = Preview::of(Some(&reply), &drawn, &ids).unwrap();
    assert_eq!(p.was_clip_boundaries, vec![circle.clone()]);
    assert_eq!(
        clip_polygon(&p.was[0].items[0], &p.was_clip_boundaries),
        Some(circle.clone()),
        "non-rectangular clip_boundary takes precedence"
    );
    assert_eq!(
        clip_polygon(&rect_item, &[]),
        Some(vec![[5.0, 5.0], [25.0, 5.0], [25.0, 25.0], [5.0, 25.0],]),
        "rectangular viewport falls back to clip bounds"
    );
    assert_eq!(clip_polygon(&paper_item, &p.was_clip_boundaries), None);

    let fan = stencil_fan(&circle, &|x, y| (x * 2.0, y * 2.0));
    assert_eq!(fan.len(), (circle.len() - 2) * 6, "two triangles for quad");
    assert_eq!(&fan[0..6], &[130.0, 80.0, 100.0, 110.0, 70.0, 80.0]);
    assert!(stencil_fan(&[[0.0, 0.0], [1.0, 0.0]], &|x, y| (x, y)).is_empty());
}
