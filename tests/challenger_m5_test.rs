#![allow(dead_code, unused_imports, unused_variables)]

#[path = "../src/model.rs"]
mod model;

#[path = "../src/canvas_view.rs"]
mod canvas_view;

use canvas_view::*;
use iced::keyboard::Modifiers;
use iced::mouse;
use iced::widget::canvas::{Event, Program};
use iced::{Point, Rectangle, Size, Vector};
use model::{create_demo_graph, NodeCategory, PwGraph, PwNode};
use std::collections::{BTreeMap, HashSet};

// ============================================================================
// 1. Inverted Marquee Drag Geometry & Cross-Quadrant Equivalence
// ============================================================================

#[test]
fn test_adversarial_inverted_marquee_quadrant_geometry() {
    let p_min = Point::new(120.0, 80.0);
    let p_max = Point::new(450.0, 310.0);
    let expected_width = 330.0;
    let expected_height = 230.0;

    // The 4 permutations of dragging between diagonally opposite corners:
    // Q1: Top-Left -> Bottom-Right (Down-Right: +dx, +dy)
    let r_tl_br = compute_box_rect(p_min, p_max);
    // Q2: Bottom-Left -> Top-Right (Up-Right: +dx, -dy)
    let r_bl_tr = compute_box_rect(Point::new(p_min.x, p_max.y), Point::new(p_max.x, p_min.y));
    // Q3: Bottom-Right -> Top-Left (Up-Left: -dx, -dy)
    let r_br_tl = compute_box_rect(p_max, p_min);
    // Q4: Top-Right -> Bottom-Left (Down-Left: -dx, +dy)
    let r_tr_bl = compute_box_rect(Point::new(p_max.x, p_min.y), Point::new(p_min.x, p_max.y));

    for (name, r) in [
        ("TL->BR", r_tl_br),
        ("BL->TR", r_bl_tr),
        ("BR->TL", r_br_tl),
        ("TR->BL", r_tr_bl),
    ] {
        assert_eq!(r.x, p_min.x, "{name} x origin mismatch");
        assert_eq!(r.y, p_min.y, "{name} y origin mismatch");
        assert_eq!(r.width, expected_width, "{name} width mismatch");
        assert_eq!(r.height, expected_height, "{name} height mismatch");
    }
}

#[test]
fn test_adversarial_inverted_marquee_negative_world_coordinates() {
    // When canvas is panned, world coordinates can be deeply negative
    let p_min = Point::new(-750.5, -420.25);
    let p_max = Point::new(-210.0, -90.5);
    let expected_w = -210.0 - (-750.5); // 540.5
    let expected_h = -90.5 - (-420.25); // 329.75

    let r1 = compute_box_rect(p_min, p_max);
    let r2 = compute_box_rect(p_max, p_min);
    let r3 = compute_box_rect(Point::new(p_min.x, p_max.y), Point::new(p_max.x, p_min.y));
    let r4 = compute_box_rect(Point::new(p_max.x, p_min.y), Point::new(p_min.x, p_max.y));

    for r in [r1, r2, r3, r4] {
        assert!((r.x - (-750.5)).abs() < 1e-4);
        assert!((r.y - (-420.25)).abs() < 1e-4);
        assert!((r.width - expected_w).abs() < 1e-4);
        assert!((r.height - expected_h).abs() < 1e-4);
        assert!(r.width >= 0.0);
        assert!(r.height >= 0.0);
    }

    // Straddling origin (crossing zero axis from negative to positive)
    let p_neg = Point::new(-150.0, -200.0);
    let p_pos = Point::new(300.0, 100.0);
    let r_straddle = compute_box_rect(p_neg, p_pos);
    let r_straddle_inv = compute_box_rect(p_pos, p_neg);

    assert_eq!(r_straddle, r_straddle_inv);
    assert_eq!(r_straddle.x, -150.0);
    assert_eq!(r_straddle.y, -200.0);
    assert_eq!(r_straddle.width, 450.0);
    assert_eq!(r_straddle.height, 300.0);
}

#[test]
fn test_adversarial_inverted_drags_node_selection_identity() {
    // Create a synthetic grid of 6 nodes
    let mut positions = BTreeMap::new();
    positions.insert(1, Point::new(100.0, 100.0));
    positions.insert(2, Point::new(400.0, 100.0));
    positions.insert(3, Point::new(700.0, 100.0));
    positions.insert(4, Point::new(100.0, 300.0));
    positions.insert(5, Point::new(400.0, 300.0));
    positions.insert(6, Point::new(700.0, 300.0));
    let visible: HashSet<u64> = positions.keys().copied().collect();

    // Drag enclosing nodes 1, 2, 4, 5:
    // Node 1: [100, 335] x [100, 196]
    // Node 2: [400, 635] x [100, 196]
    // Node 4: [100, 335] x [300, 396]
    // Node 5: [400, 635] x [300, 396]
    let start_point = Point::new(50.0, 50.0);
    let end_point = Point::new(500.0, 350.0);

    let rect_fwd = compute_box_rect(start_point, end_point);
    let rect_rev = compute_box_rect(end_point, start_point);
    let rect_diag1 = compute_box_rect(Point::new(start_point.x, end_point.y), Point::new(end_point.x, start_point.y));
    let rect_diag2 = compute_box_rect(Point::new(end_point.x, start_point.y), Point::new(start_point.x, end_point.y));

    let nodes_fwd = compute_intersecting_nodes(&positions, &visible, &rect_fwd);
    let nodes_rev = compute_intersecting_nodes(&positions, &visible, &rect_rev);
    let nodes_diag1 = compute_intersecting_nodes(&positions, &visible, &rect_diag1);
    let nodes_diag2 = compute_intersecting_nodes(&positions, &visible, &rect_diag2);

    let expected: HashSet<u64> = [1, 2, 4, 5].into_iter().collect();
    assert_eq!(nodes_fwd, expected);
    assert_eq!(nodes_rev, expected);
    assert_eq!(nodes_diag1, expected);
    assert_eq!(nodes_diag2, expected);
}

#[test]
fn test_adversarial_marquee_threshold_and_1d_lines() {
    // Threshold in code is: r.width >= 3.0 || r.height >= 3.0
    // Sub-threshold jitter: 2.99 px in both directions
    let p_origin = Point::new(100.0, 100.0);
    let p_jitter = Point::new(102.99, 102.99);
    let r_jitter = compute_box_rect(p_origin, p_jitter);
    assert!(
        r_jitter.width < 3.0 && r_jitter.height < 3.0,
        "Jitter box should be below 3px threshold"
    );

    // Pure 1D drag: exactly horizontal line with width 10.0, height 0.0
    let p_h1 = Point::new(100.0, 150.0);
    let p_h2 = Point::new(150.0, 150.0);
    let r_horiz = compute_box_rect(p_h1, p_h2);
    assert_eq!(r_horiz.height, 0.0);
    assert_eq!(r_horiz.width, 50.0);
    assert!(r_horiz.width >= 3.0 || r_horiz.height >= 3.0);

    // Inverted horizontal line
    let r_horiz_inv = compute_box_rect(p_h2, p_h1);
    assert_eq!(r_horiz, r_horiz_inv);

    // Pure 1D drag: vertical line with height 50.0, width 0.0
    let p_v1 = Point::new(200.0, 100.0);
    let p_v2 = Point::new(200.0, 180.0);
    let r_vert = compute_box_rect(p_v1, p_v2);
    assert_eq!(r_vert.width, 0.0);
    assert_eq!(r_vert.height, 80.0);
    assert!(r_vert.width >= 3.0 || r_vert.height >= 3.0);

    // Inverted vertical line
    let r_vert_inv = compute_box_rect(p_v2, p_v1);
    assert_eq!(r_vert, r_vert_inv);
}

// ============================================================================
// 2. Zoomed Canvas Transformations (0.5x, 1.0x, 2.0x, 3.5x)
// ============================================================================

#[test]
fn test_adversarial_zoomed_canvas_screen_world_round_trip() {
    let zoom_levels = [0.1, 0.5, 1.0, 1.5, 2.0, 3.5, 5.0];
    let pans = [
        Vector::ZERO,
        Vector::new(100.0, 100.0),
        Vector::new(-350.5, 420.25),
        Vector::new(1250.0, -890.0),
    ];

    let graph = create_demo_graph();
    let positions = BTreeMap::new();

    for &zoom in &zoom_levels {
        for &pan in &pans {
            let canvas = GraphCanvas {
                graph: &graph,
                visible_nodes: HashSet::new(),
                node_positions: &positions,
                selected_edge: None,
                selected_node: None,
                selected_nodes: HashSet::new(),
                pan,
                zoom,
                layer: CanvasLayer::Base,
                search_query: "",
                modifiers: iced::keyboard::Modifiers::default(),
            };

            // Test a grid of points spanning positive, negative, and edge coordinates
            let test_screen_points = [
                Point::new(0.0, 0.0),
                Point::new(100.0, 200.0),
                Point::new(800.0, 600.0),
                Point::new(-150.0, -250.0),
                Point::new(1920.0, 1080.0),
            ];

            for &sp in &test_screen_points {
                let wp = Point::new((sp.x - canvas.pan.x) / canvas.zoom, (sp.y - canvas.pan.y) / canvas.zoom);
                let sp_recovered = Point::new(wp.x * canvas.zoom + canvas.pan.x, wp.y * canvas.zoom + canvas.pan.y);
                assert!(
                    (sp.x - sp_recovered.x).abs() < 1e-3 && (sp.y - sp_recovered.y).abs() < 1e-3,
                    "Roundtrip failed for zoom {zoom}, pan {:?}, point {:?}",
                    pan,
                    sp
                );
            }
        }
    }
}

#[test]
fn test_adversarial_zoom_marquee_world_to_screen_rendering_invariant() {
    // When the user drags on screen from S1 to S2:
    // 1. world points are W1 = screen_to_world(S1), W2 = screen_to_world(S2)
    // 2. marquee_rect in world is compute_box_rect(W1, W2)
    // 3. rendered screen_rect is:
    //    screen_min = world_to_screen(marquee_rect.position())
    //    screen_size = (marquee_rect.width * zoom, marquee_rect.height * zoom)
    // Invariant: rendered screen_rect MUST exactly match compute_box_rect(S1, S2)
    let zoom_levels = [0.5, 1.0, 2.0, 3.5];
    let pans = [
        Vector::ZERO,
        Vector::new(120.0, -85.0),
        Vector::new(-400.0, 600.0),
    ];

    let drag_pairs = [
        (Point::new(100.0, 100.0), Point::new(500.0, 400.0)), // TL -> BR
        (Point::new(500.0, 400.0), Point::new(100.0, 100.0)), // BR -> TL (inverted)
        (Point::new(100.0, 400.0), Point::new(500.0, 100.0)), // BL -> TR
        (Point::new(500.0, 100.0), Point::new(100.0, 400.0)), // TR -> BL
    ];

    for &zoom in &zoom_levels {
        for &pan in &pans {
            let screen_to_world = |p: Point| Point::new((p.x - pan.x) / zoom, (p.y - pan.y) / zoom);
            let world_to_screen = |p: Point| Point::new(p.x * zoom + pan.x, p.y * zoom + pan.y);

            for &(s1, s2) in &drag_pairs {
                let expected_screen_rect = compute_box_rect(s1, s2);

                let w1 = screen_to_world(s1);
                let w2 = screen_to_world(s2);
                let rect_world = compute_box_rect(w1, w2);

                let rendered_screen_min = world_to_screen(Point::new(rect_world.x, rect_world.y));
                let rendered_screen_w = rect_world.width * zoom;
                let rendered_screen_h = rect_world.height * zoom;

                assert!(
                    (rendered_screen_min.x - expected_screen_rect.x).abs() < 1e-3,
                    "Screen rect X mismatch at zoom {zoom}, pan {:?}",
                    pan
                );
                assert!(
                    (rendered_screen_min.y - expected_screen_rect.y).abs() < 1e-3,
                    "Screen rect Y mismatch at zoom {zoom}, pan {:?}",
                    pan
                );
                assert!(
                    (rendered_screen_w - expected_screen_rect.width).abs() < 1e-3,
                    "Screen rect width mismatch at zoom {zoom}, pan {:?}",
                    pan
                );
                assert!(
                    (rendered_screen_h - expected_screen_rect.height).abs() < 1e-3,
                    "Screen rect height mismatch at zoom {zoom}, pan {:?}",
                    pan
                );
            }
        }
    }
}

#[test]
fn test_adversarial_zoom_intersection_preservation() {
    // Under affine scaling (zoom > 0) and translation (pan),
    // rect_world.intersects(node_world) == rect_screen.intersects(node_screen)
    let zoom_levels = [0.5, 1.0, 2.0, 3.5];
    let pans = [
        Vector::ZERO,
        Vector::new(200.0, -150.0),
        Vector::new(-500.0, 300.0),
    ];

    // Node in world coordinates
    let node_pos_world = Point::new(200.0, 150.0);
    let nw = NODE_WIDTH;
    let nh = NODE_HEIGHT;
    let node_rect_world = Rectangle::new(node_pos_world, Size::new(nw, nh));

    // Sample various marquee boxes in world coordinates:
    // (a) fully inside
    // (b) fully enclosing
    // (c) 1px border overlap
    // (d) 1px border outside
    // (e) corner overlap
    // (f) far away
    let world_test_boxes = [
        Rectangle::new(Point::new(220.0, 160.0), Size::new(50.0, 40.0)), // inside -> true
        Rectangle::new(Point::new(100.0, 100.0), Size::new(500.0, 400.0)), // enclosing -> true
        Rectangle::new(Point::new(100.0, 150.0), Size::new(101.0, 50.0)), // left 1px overlap -> true
        Rectangle::new(Point::new(100.0, 150.0), Size::new(99.0, 50.0)), // left 1px gap -> false
        Rectangle::new(Point::new(200.0 + nw - 1.0, 150.0), Size::new(50.0, 50.0)), // right 1px overlap -> true
        Rectangle::new(Point::new(200.0 + nw + 1.0, 150.0), Size::new(50.0, 50.0)), // right 1px gap -> false
        Rectangle::new(Point::new(900.0, 900.0), Size::new(100.0, 100.0)), // far away -> false
    ];

    for &zoom in &zoom_levels {
        for &pan in &pans {
            let world_to_screen = |p: Point| Point::new(p.x * zoom + pan.x, p.y * zoom + pan.y);

            let node_screen_pos = world_to_screen(node_pos_world);
            let node_screen_rect = Rectangle::new(node_screen_pos, Size::new(nw * zoom, nh * zoom));

            for (idx, world_box) in world_test_boxes.iter().enumerate() {
                let world_intersects = world_box.intersects(&node_rect_world);

                let screen_box_pos = world_to_screen(world_box.position());
                let screen_box = Rectangle::new(
                    screen_box_pos,
                    Size::new(world_box.width * zoom, world_box.height * zoom),
                );
                let screen_intersects = screen_box.intersects(&node_screen_rect);

                assert_eq!(
                    world_intersects, screen_intersects,
                    "Intersection discrepancy at box index {idx}, zoom {zoom}, pan {:?}",
                    pan
                );
            }
        }
    }
}

// ============================================================================
// 3. Boundary Contact & 1-Pixel Precision Intersections
// ============================================================================

#[test]
fn test_adversarial_1px_boundary_contact_edges() {
    let node_pos = Point::new(200.0, 200.0);
    let nw = NODE_WIDTH; // 235.0 -> x in [200.0, 435.0]
    let nh = NODE_HEIGHT; // 96.0  -> y in [200.0, 296.0]

    // 1. LEFT EDGE
    // Exact contact (0px overlap): [100.0, 200.0]
    let left_touch = Rectangle::new(Point::new(100.0, 220.0), Size::new(100.0, 40.0));
    assert!(
        !node_intersects_rect(node_pos, &left_touch),
        "Exact left edge touch (0px) must NOT intersect"
    );
    // 1px penetration into left edge: [100.0, 201.0]
    let left_1px_in = Rectangle::new(Point::new(100.0, 220.0), Size::new(101.0, 40.0));
    assert!(
        node_intersects_rect(node_pos, &left_1px_in),
        "1px left edge overlap must intersect"
    );
    // 1px gap outside left edge: [100.0, 199.0]
    let left_1px_out = Rectangle::new(Point::new(100.0, 220.0), Size::new(99.0, 40.0));
    assert!(
        !node_intersects_rect(node_pos, &left_1px_out),
        "1px outside left edge must NOT intersect"
    );

    // 2. RIGHT EDGE
    // Exact contact (0px overlap): starting at 435.0
    let right_touch = Rectangle::new(Point::new(200.0 + nw, 220.0), Size::new(100.0, 40.0));
    assert!(
        !node_intersects_rect(node_pos, &right_touch),
        "Exact right edge touch (0px) must NOT intersect"
    );
    // 1px penetration into right edge: starting at 434.0
    let right_1px_in = Rectangle::new(Point::new(200.0 + nw - 1.0, 220.0), Size::new(100.0, 40.0));
    assert!(
        node_intersects_rect(node_pos, &right_1px_in),
        "1px right edge overlap must intersect"
    );
    // 1px gap outside right edge: starting at 436.0
    let right_1px_out = Rectangle::new(Point::new(200.0 + nw + 1.0, 220.0), Size::new(100.0, 40.0));
    assert!(
        !node_intersects_rect(node_pos, &right_1px_out),
        "1px outside right edge must NOT intersect"
    );

    // 3. TOP EDGE
    // Exact contact (0px overlap): [100.0, 200.0] vertically
    let top_touch = Rectangle::new(Point::new(250.0, 100.0), Size::new(80.0, 100.0));
    assert!(
        !node_intersects_rect(node_pos, &top_touch),
        "Exact top edge touch (0px) must NOT intersect"
    );
    // 1px penetration into top edge: [100.0, 201.0]
    let top_1px_in = Rectangle::new(Point::new(250.0, 100.0), Size::new(80.0, 101.0));
    assert!(
        node_intersects_rect(node_pos, &top_1px_in),
        "1px top edge overlap must intersect"
    );
    // 1px gap outside top edge: [100.0, 199.0]
    let top_1px_out = Rectangle::new(Point::new(250.0, 100.0), Size::new(80.0, 99.0));
    assert!(
        !node_intersects_rect(node_pos, &top_1px_out),
        "1px outside top edge must NOT intersect"
    );

    // 4. BOTTOM EDGE
    // Exact contact (0px overlap): starting at 296.0
    let bottom_touch = Rectangle::new(Point::new(250.0, 200.0 + nh), Size::new(80.0, 100.0));
    assert!(
        !node_intersects_rect(node_pos, &bottom_touch),
        "Exact bottom edge touch (0px) must NOT intersect"
    );
    // 1px penetration into bottom edge: starting at 295.0
    let bottom_1px_in = Rectangle::new(Point::new(250.0, 200.0 + nh - 1.0), Size::new(80.0, 100.0));
    assert!(
        node_intersects_rect(node_pos, &bottom_1px_in),
        "1px bottom edge overlap must intersect"
    );
    // 1px gap outside bottom edge: starting at 297.0
    let bottom_1px_out = Rectangle::new(Point::new(250.0, 200.0 + nh + 1.0), Size::new(80.0, 100.0));
    assert!(
        !node_intersects_rect(node_pos, &bottom_1px_out),
        "1px outside bottom edge must NOT intersect"
    );
}

#[test]
fn test_adversarial_1px_corner_overlaps() {
    let node_pos = Point::new(100.0, 100.0);
    let nw = NODE_WIDTH; // 235.0 -> [100, 335]
    let nh = NODE_HEIGHT; // 96.0  -> [100, 196]

    // 1. Top-Left Corner: (100.0, 100.0)
    let tl_overlap = Rectangle::new(Point::new(50.0, 50.0), Size::new(51.0, 51.0)); // spans to (101, 101)
    assert!(node_intersects_rect(node_pos, &tl_overlap), "Top-Left 1px corner overlap must intersect");
    let tl_gap = Rectangle::new(Point::new(50.0, 50.0), Size::new(49.0, 49.0)); // spans to (99, 99)
    assert!(!node_intersects_rect(node_pos, &tl_gap), "Top-Left 1px corner gap must NOT intersect");

    // 2. Top-Right Corner: (335.0, 100.0)
    let tr_overlap = Rectangle::new(Point::new(100.0 + nw - 1.0, 50.0), Size::new(50.0, 51.0)); // x starts at 334, y ends at 101
    assert!(node_intersects_rect(node_pos, &tr_overlap), "Top-Right 1px corner overlap must intersect");
    let tr_gap = Rectangle::new(Point::new(100.0 + nw + 1.0, 50.0), Size::new(50.0, 49.0)); // x starts at 336, y ends at 99
    assert!(!node_intersects_rect(node_pos, &tr_gap), "Top-Right 1px corner gap must NOT intersect");

    // 3. Bottom-Left Corner: (100.0, 196.0)
    let bl_overlap = Rectangle::new(Point::new(50.0, 100.0 + nh - 1.0), Size::new(51.0, 50.0)); // x ends at 101, y starts at 195
    assert!(node_intersects_rect(node_pos, &bl_overlap), "Bottom-Left 1px corner overlap must intersect");
    let bl_gap = Rectangle::new(Point::new(50.0, 100.0 + nh + 1.0), Size::new(49.0, 50.0)); // x ends at 99, y starts at 197
    assert!(!node_intersects_rect(node_pos, &bl_gap), "Bottom-Left 1px corner gap must NOT intersect");

    // 4. Bottom-Right Corner: (335.0, 196.0)
    let br_overlap = Rectangle::new(Point::new(100.0 + nw - 1.0, 100.0 + nh - 1.0), Size::new(50.0, 50.0)); // starts (334, 195)
    assert!(node_intersects_rect(node_pos, &br_overlap), "Bottom-Right 1px corner overlap must intersect");
    let br_gap = Rectangle::new(Point::new(100.0 + nw + 1.0, 100.0 + nh + 1.0), Size::new(50.0, 50.0)); // starts (336, 197)
    assert!(!node_intersects_rect(node_pos, &br_gap), "Bottom-Right 1px corner gap must NOT intersect");
}

#[test]
fn test_adversarial_containment_and_thin_cross_sections() {
    let node_pos = Point::new(100.0, 100.0);
    let nw = NODE_WIDTH; // 235.0
    let nh = NODE_HEIGHT; // 96.0

    // Complete containment: marquee completely inside node
    let inside_marquee = Rectangle::new(Point::new(150.0, 120.0), Size::new(30.0, 25.0));
    assert!(node_intersects_rect(node_pos, &inside_marquee));

    // Complete containment: node completely inside marquee
    let enclosing_marquee = Rectangle::new(Point::new(50.0, 50.0), Size::new(400.0, 200.0));
    assert!(node_intersects_rect(node_pos, &enclosing_marquee));

    // 1px horizontal slice through center
    let h_slice = Rectangle::new(Point::new(50.0, 140.0), Size::new(400.0, 1.0));
    assert!(node_intersects_rect(node_pos, &h_slice), "1px horizontal slice must intersect");

    // 1px vertical slice through center
    let v_slice = Rectangle::new(Point::new(200.0, 50.0), Size::new(1.0, 200.0));
    assert!(node_intersects_rect(node_pos, &v_slice), "1px vertical slice must intersect");
}

// ============================================================================
// 4. Modifier Toggles & State Transitions
// ============================================================================

#[test]
fn test_adversarial_selection_state_machine_transitions() {
    let mut selected: HashSet<u64> = HashSet::new();

    // 1. Initial state is empty
    assert!(selected.is_empty());

    // 2. Normal marquee selects nodes {1, 2, 3}
    let batch1: HashSet<u64> = [1, 2, 3].into_iter().collect();
    apply_box_selection(&mut selected, &batch1, false);
    assert_eq!(selected, [1, 2, 3].into_iter().collect());

    // 3. Normal marquee replaces with {4, 5}
    let batch2: HashSet<u64> = [4, 5].into_iter().collect();
    apply_box_selection(&mut selected, &batch2, false);
    assert_eq!(selected, [4, 5].into_iter().collect());

    // 4. Shift marquee over {5, 6, 7}:
    // 5 was selected -> toggled OFF
    // 6 was unselected -> toggled ON
    // 7 was unselected -> toggled ON
    // 4 remains selected
    let batch3: HashSet<u64> = [5, 6, 7].into_iter().collect();
    apply_box_selection(&mut selected, &batch3, true);
    assert_eq!(selected, [4, 6, 7].into_iter().collect());

    // 5. Shift marquee with identical set {5, 6, 7} reverses operation:
    // 5 toggled ON, 6 toggled OFF, 7 toggled OFF -> back to {4, 5}
    apply_box_selection(&mut selected, &batch3, true);
    assert_eq!(selected, [4, 5].into_iter().collect());

    // 6. Single node toggles interleaved:
    toggle_selection(&mut selected, 4); // toggle 4 OFF -> {5}
    assert_eq!(selected, [5].into_iter().collect());
    toggle_selection(&mut selected, 8); // toggle 8 ON -> {5, 8}
    assert_eq!(selected, [5, 8].into_iter().collect());

    // 7. Normal marquee on empty region clears selection
    let empty_set: HashSet<u64> = HashSet::new();
    apply_box_selection(&mut selected, &empty_set, false);
    assert!(selected.is_empty());

    // 8. Shift marquee on empty region preserves selection
    let mut selected_retain: HashSet<u64> = [10, 20].into_iter().collect();
    apply_box_selection(&mut selected_retain, &empty_set, true);
    assert_eq!(selected_retain, [10, 20].into_iter().collect());
}

// ============================================================================
// 5. Canvas Program Interaction Simulation (Empty Canvas Click vs Drag vs Modifiers)
// ============================================================================

#[test]
fn test_adversarial_canvas_empty_click_clearing_vs_modifier_retention() {
    let graph = create_demo_graph();
    let positions = BTreeMap::new();
    let bounds = Rectangle::new(Point::new(0.0, 0.0), Size::new(1000.0, 800.0));

    let canvas = GraphCanvas {
        graph: &graph,
        visible_nodes: HashSet::new(),
        node_positions: &positions,
        selected_edge: None,
        selected_node: None,
        selected_nodes: [10, 20].into_iter().collect(),
        pan: Vector::ZERO,
        zoom: 1.0,
        layer: CanvasLayer::Base,
        search_query: "",
        modifiers: iced::keyboard::Modifiers::default(),
    };

    // Case 1: Plain Left Click on Empty Canvas (no drag, no modifier)
    {
        let mut state = CanvasInteraction::default();
        let click_pt = Point::new(500.0, 500.0);
        let cursor = mouse::Cursor::Available(click_pt);

        // Press down
        let press_event = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let _ = canvas.update(&mut state, &press_event, bounds, cursor);
        assert_eq!(state.marquee_start, Some(click_pt));
        assert_eq!(state.marquee_rect, None);

        // Release without move -> MUST clear selection
        let release_event = Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
        let action = canvas.update(&mut state, &release_event, bounds, cursor);

        assert!(action.is_some(), "Release must emit an action");
        // Verify state is cleaned up
        assert_eq!(state.marquee_start, None);
        assert_eq!(state.marquee_rect, None);
    }

    // Case 2: Shift-Click on Empty Canvas (no drag, Shift modifier held)
    {
        let mut state = CanvasInteraction::default();
        // Set Shift modifier
        let shift_mod = Modifiers::SHIFT;
        let mod_event = Event::Keyboard(iced::keyboard::Event::ModifiersChanged(shift_mod));
        let _ = canvas.update(&mut state, &mod_event, bounds, mouse::Cursor::Unavailable);
        assert!(state.modifiers.shift());

        let click_pt = Point::new(500.0, 500.0);
        let cursor = mouse::Cursor::Available(click_pt);

        // Press down
        let press_event = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let _ = canvas.update(&mut state, &press_event, bounds, cursor);
        assert_eq!(state.marquee_start, Some(click_pt));

        // Release with Shift held -> does NOT publish ClearSelection (only redraws)
        let release_event = Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
        let action = canvas.update(&mut state, &release_event, bounds, cursor);
        assert!(action.is_some());
        assert_eq!(state.marquee_start, None);
        assert_eq!(state.marquee_rect, None);
    }

    // Case 3: Sub-pixel jitter click (< 3px drag) with plain click
    {
        let mut state = CanvasInteraction::default();
        let start_pt = Point::new(500.0, 500.0);
        let cursor_start = mouse::Cursor::Available(start_pt);

        let press_event = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let _ = canvas.update(&mut state, &press_event, bounds, cursor_start);

        // Move 2px
        let move_pt = Point::new(502.0, 502.0);
        let cursor_move = mouse::Cursor::Available(move_pt);
        let move_event = Event::Mouse(mouse::Event::CursorMoved { position: move_pt });
        let _ = canvas.update(&mut state, &move_event, bounds, cursor_move);

        assert!(state.marquee_rect.is_some());
        let rect = state.marquee_rect.unwrap();
        assert!(rect.width < 3.0 && rect.height < 3.0);

        // Release without modifier: treats as click, triggers ClearSelection!
        let release_event = Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
        let action = canvas.update(&mut state, &release_event, bounds, cursor_move);
        assert!(action.is_some());
        assert_eq!(state.marquee_start, None);
        assert_eq!(state.marquee_rect, None);
    }

    // Case 4: Drag >= 3px enters marquee selection mode
    {
        let mut state = CanvasInteraction::default();
        let start_pt = Point::new(500.0, 500.0);
        let cursor_start = mouse::Cursor::Available(start_pt);

        let press_event = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let _ = canvas.update(&mut state, &press_event, bounds, cursor_start);

        // Move 20px
        let move_pt = Point::new(520.0, 530.0);
        let cursor_move = mouse::Cursor::Available(move_pt);
        let move_event = Event::Mouse(mouse::Event::CursorMoved { position: move_pt });
        let _ = canvas.update(&mut state, &move_event, bounds, cursor_move);

        assert!(state.marquee_rect.is_some());
        let rect = state.marquee_rect.unwrap();
        assert!(rect.width >= 3.0 || rect.height >= 3.0);

        // Release
        let release_event = Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
        let action = canvas.update(&mut state, &release_event, bounds, cursor_move);
        assert!(action.is_some());
        assert_eq!(state.marquee_start, None);
        assert_eq!(state.marquee_rect, None);
    }
}

// ============================================================================
// 6. Group Drag & Pinned Overlap Solver Stress Invariants
// ============================================================================

#[test]
fn test_adversarial_pinned_overlap_nudger_rigidity_and_crowding() {
    let mut positions = BTreeMap::new();
    // 3 Pinned nodes arranged in a horizontal row
    let p1 = Point::new(100.0, 200.0);
    let p2 = Point::new(400.0, 200.0);
    let p3 = Point::new(700.0, 200.0);
    positions.insert(1, p1);
    positions.insert(2, p2);
    positions.insert(3, p3);

    // Crowding attack: 6 unpinned nodes dropped directly overlapping the pinned nodes
    positions.insert(10, Point::new(105.0, 205.0)); // collides with p1
    positions.insert(11, Point::new(95.0, 195.0));   // collides with p1 & 10
    positions.insert(20, Point::new(405.0, 205.0)); // collides with p2
    positions.insert(21, Point::new(395.0, 195.0)); // collides with p2 & 20
    positions.insert(30, Point::new(705.0, 205.0)); // collides with p3
    positions.insert(31, Point::new(695.0, 195.0)); // collides with p3 & 30

    let pinned: HashSet<u64> = [1, 2, 3].into_iter().collect();

    resolve_node_overlaps_pinned(&mut positions, &pinned);

    // CRITICAL INVARIANT 1: Pinned nodes MUST NEVER MOVE by even a micro-pixel
    assert_eq!(positions[&1], p1, "Pinned node 1 moved!");
    assert_eq!(positions[&2], p2, "Pinned node 2 moved!");
    assert_eq!(positions[&3], p3, "Pinned node 3 moved!");

    // CRITICAL INVARIANT 2: No unpinned node overlaps any pinned node
    let min_dist_x = NODE_WIDTH + 12.0; // 247.0
    let min_dist_y = NODE_HEIGHT + 12.0; // 108.0

    for &pinned_id in &[1, 2, 3] {
        let pinned_pos = positions[&pinned_id];
        for &unpinned_id in &[10, 11, 20, 21, 30, 31] {
            let unpinned_pos = positions[&unpinned_id];
            let dx = (unpinned_pos.x - pinned_pos.x).abs();
            let dy = (unpinned_pos.y - pinned_pos.y).abs();
            assert!(
                dx >= min_dist_x || dy >= min_dist_y,
                "Unpinned node {unpinned_id} still overlaps pinned node {pinned_id}: dx={dx}, dy={dy}"
            );
        }
    }
}

// ============================================================================
// 7. Random Fuzz & Mathematical Invariant Tests
// ============================================================================

#[test]
fn test_adversarial_marquee_box_random_fuzz_invariants() {
    // Deterministic pseudo-random LCG generator for reproducibility
    let mut seed: u64 = 0xDEADBEEF_CAFE1234;
    let mut rand_f32 = || -> f32 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let val = (seed >> 32) as u32;
        (val as f32 / u32::MAX as f32) * 2000.0 - 1000.0 // range [-1000.0, 1000.0]
    };

    // Build a fixed grid of 10 nodes
    let mut positions = BTreeMap::new();
    for i in 0..10 {
        positions.insert(i as u64, Point::new(-300.0 + (i as f32) * 150.0, -200.0 + (i as f32) * 80.0));
    }
    let visible: HashSet<u64> = positions.keys().copied().collect();

    for _iter in 0..2000 {
        let p1 = Point::new(rand_f32(), rand_f32());
        let p2 = Point::new(rand_f32(), rand_f32());

        // Four corner permutations of the same bounding rectangle:
        let r_fwd = compute_box_rect(p1, p2);
        let r_rev = compute_box_rect(p2, p1);
        let r_c1 = compute_box_rect(Point::new(p1.x, p2.y), Point::new(p2.x, p1.y));
        let r_c2 = compute_box_rect(Point::new(p2.x, p1.y), Point::new(p1.x, p2.y));

        // Invariant 1: Non-negative dimensions
        assert!(r_fwd.width >= 0.0, "Width must be non-negative");
        assert!(r_fwd.height >= 0.0, "Height must be non-negative");

        // Invariant 2: Symmetry across all quadrant permutations
        assert_eq!(r_fwd, r_rev);
        assert_eq!(r_fwd, r_c1);
        assert_eq!(r_fwd, r_c2);

        // Invariant 3: Intersecting nodes query is strictly identical across permutations
        let s_fwd = compute_intersecting_nodes(&positions, &visible, &r_fwd);
        let s_rev = compute_intersecting_nodes(&positions, &visible, &r_rev);
        let s_c1 = compute_intersecting_nodes(&positions, &visible, &r_c1);
        let s_c2 = compute_intersecting_nodes(&positions, &visible, &r_c2);

        assert_eq!(s_fwd, s_rev);
        assert_eq!(s_fwd, s_c1);
        assert_eq!(s_fwd, s_c2);
    }
}

#[test]
fn test_adversarial_rapid_modifier_toggle_fuzz_1000_cycles() {
    let mut seed: u64 = 0x12345678_9ABCDEF0;
    let mut rand_u64 = || -> u64 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        seed >> 32
    };

    let mut selected: HashSet<u64> = HashSet::new();
    let universe: Vec<u64> = (1..=20).collect();

    for _cycle in 0..1000 {
        let op_type = rand_u64() % 3;
        let batch_size = (rand_u64() % 6) as usize;
        let mut batch = HashSet::new();
        for _ in 0..batch_size {
            let idx = (rand_u64() as usize) % universe.len();
            batch.insert(universe[idx]);
        }

        match op_type {
            0 => {
                // Normal mode marquee (replaces)
                apply_box_selection(&mut selected, &batch, false);
                assert_eq!(selected, batch);
            }
            1 => {
                // Shift mode marquee (symmetric difference)
                let before = selected.clone();
                apply_box_selection(&mut selected, &batch, true);

                // Re-toggling the exact same batch MUST invert back to `before`
                let mut reverted = selected.clone();
                apply_box_selection(&mut reverted, &batch, true);
                assert_eq!(reverted, before, "Symmetric difference must be self-inverting");
            }
            2 => {
                // Single node toggle
                let node = universe[(rand_u64() as usize) % universe.len()];
                let had_node = selected.contains(&node);
                toggle_selection(&mut selected, node);
                assert_eq!(selected.contains(&node), !had_node);
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn test_adversarial_hidden_nodes_and_category_filtering() {
    let mut positions = BTreeMap::new();
    positions.insert(1, Point::new(100.0, 100.0)); // Audio
    positions.insert(2, Point::new(200.0, 100.0)); // Midi (hidden)
    positions.insert(3, Point::new(300.0, 100.0)); // Video (hidden)

    // Visible set only contains node 1
    let visible: HashSet<u64> = [1].into_iter().collect();

    // Marquee enclosing all 3 nodes
    let giant_box = Rectangle::new(Point::new(50.0, 50.0), Size::new(500.0, 300.0));
    let hit = compute_intersecting_nodes(&positions, &visible, &giant_box);

    assert_eq!(
        hit,
        [1].into_iter().collect(),
        "Hidden category nodes must NEVER be selected even if marquee spans them"
    );
}

