#![allow(dead_code, unused_imports, unused_variables)]

#[path = "../src/model.rs"]
mod model;

#[path = "../src/canvas_view.rs"]
mod canvas_view;

use canvas_view::{
    apply_box_selection, apply_group_drag_delta, compute_box_rect, compute_intersecting_nodes,
    node_intersects_rect, resolve_node_overlaps_pinned, toggle_selection, CanvasInteraction,
    CanvasLayer, CanvasMessage, GraphCanvas, GroupDragState, NODE_HEIGHT, NODE_WIDTH,
};
use iced::keyboard::Modifiers;
use iced::mouse;
use iced::widget::canvas::{self, Event, Program};
use iced::{Point, Rectangle, Size, Vector};
use model::create_demo_graph;
use std::collections::{BTreeMap, HashSet};

// ============================================================================
// 1. Group Drag Float Precision & Translation Invariance
// ============================================================================

#[test]
fn test_challenger_group_drag_delta_bit_level_precision() {
    let mut initial_positions = BTreeMap::new();
    initial_positions.insert(1, Point::new(100.125, 200.25));
    initial_positions.insert(2, Point::new(-350.5, 450.75));
    initial_positions.insert(3, Point::new(0.0, 0.0));
    initial_positions.insert(4, Point::new(1234.56, -987.65));
    initial_positions.insert(5, Point::new(5000.0, 10000.0));

    let deltas = vec![
        Vector::new(123.45, -67.89),
        Vector::new(-987.654, 432.109),
        Vector::new(0.0001, -0.0002),
        Vector::new(0.0, 0.0),
        Vector::new(5432.1, -1234.5),
    ];

    for delta in deltas {
        let updated = apply_group_drag_delta(&initial_positions, delta);
        assert_eq!(updated.len(), initial_positions.len());

        for (&serial, &init_pos) in &initial_positions {
            let up_pos = updated[&serial];
            let expected_x = init_pos.x + delta.x;
            let expected_y = init_pos.y + delta.y;

            // Check bit-exact equivalence down to f32::to_bits()
            assert_eq!(
                up_pos.x.to_bits(),
                expected_x.to_bits(),
                "Node {serial} X float bits mismatch for delta ({}, {})",
                delta.x,
                delta.y
            );
            assert_eq!(
                up_pos.y.to_bits(),
                expected_y.to_bits(),
                "Node {serial} Y float bits mismatch for delta ({}, {})",
                delta.x,
                delta.y
            );
        }
    }
}

#[test]
fn test_challenger_relative_formation_invariance_throughout_drag() {
    let mut initial_positions = BTreeMap::new();
    initial_positions.insert(10, Point::new(100.0, 100.0));
    initial_positions.insert(20, Point::new(300.0, 150.0));
    initial_positions.insert(30, Point::new(200.0, 400.0));
    initial_positions.insert(40, Point::new(500.0, 350.0));

    // Calculate initial pairwise Euclidean distances
    let serials = [10, 20, 30, 40];
    let mut initial_distances = Vec::new();
    for i in 0..serials.len() {
        for j in (i + 1)..serials.len() {
            let s1 = serials[i];
            let s2 = serials[j];
            let p1 = initial_positions[&s1];
            let p2 = initial_positions[&s2];
            let dx = p2.x - p1.x;
            let dy = p2.y - p1.y;
            let dist = f32::sqrt(dx * dx + dy * dy);
            initial_distances.push((s1, s2, dist));
        }
    }

    // Simulate multi-step dragging across 500 steps along a non-linear trajectory
    for step in 0..500 {
        let angle = (step as f32) * 0.05;
        let radius = 200.0 + (step as f32) * 0.5;
        let delta = Vector::new(radius * angle.cos(), radius * angle.sin());

        let dragged = apply_group_drag_delta(&initial_positions, delta);

        for &(s1, s2, init_dist) in &initial_distances {
            let p1 = dragged[&s1];
            let p2 = dragged[&s2];
            let dx = p2.x - p1.x;
            let dy = p2.y - p1.y;
            let current_dist = f32::sqrt(dx * dx + dy * dy);

            // Verify pairwise distance is strictly preserved within floating point limits (1e-4)
            let diff = (current_dist - init_dist).abs();
            assert!(
                diff < 1e-4,
                "Relative formation distance drifted at step {step} between {s1} and {s2}: diff={diff}"
            );
        }
    }
}

// ============================================================================
// 2. Pinned Group Overlap Invariants (Bit-exactness)
// ============================================================================

#[test]
fn test_challenger_pinned_nodes_never_move_bit_level() {
    let mut positions = BTreeMap::new();
    let drop_1 = Point::new(100.0, 100.0);
    let drop_2 = Point::new(400.0, 100.0);
    let drop_3 = Point::new(700.0, 100.0);

    positions.insert(1, drop_1);
    positions.insert(2, drop_2);
    positions.insert(3, drop_3);

    // Colliding unpinned nodes
    positions.insert(4, drop_1);
    positions.insert(5, drop_2);
    positions.insert(6, drop_3);

    let pinned: HashSet<u64> = [1, 2, 3].into_iter().collect();

    resolve_node_overlaps_pinned(&mut positions, &pinned);

    // Verify pinned nodes are 100% bit-for-bit identical to drop position
    assert_eq!(
        positions[&1].x.to_bits(),
        drop_1.x.to_bits(),
        "Pinned node 1 X altered"
    );
    assert_eq!(
        positions[&1].y.to_bits(),
        drop_1.y.to_bits(),
        "Pinned node 1 Y altered"
    );
    assert_eq!(
        positions[&2].x.to_bits(),
        drop_2.x.to_bits(),
        "Pinned node 2 X altered"
    );
    assert_eq!(
        positions[&2].y.to_bits(),
        drop_2.y.to_bits(),
        "Pinned node 2 Y altered"
    );
    assert_eq!(
        positions[&3].x.to_bits(),
        drop_3.x.to_bits(),
        "Pinned node 3 X altered"
    );
    assert_eq!(
        positions[&3].y.to_bits(),
        drop_3.y.to_bits(),
        "Pinned node 3 Y altered"
    );
}

// ============================================================================
// 3. Pinned Overlap Resolution Failure Demonstrations (Sandwich Oscillations)
// ============================================================================

#[test]
fn test_challenger_pinned_overlap_horizontal_sandwich_failure() {
    // 2 pinned nodes with 300px spacing (dx=300 > min_dist_x=247, so they do not overlap each other)
    let mut positions = BTreeMap::new();
    let pinned_1 = Point::new(100.0, 100.0);
    let pinned_2 = Point::new(400.0, 100.0);
    positions.insert(1, pinned_1);
    positions.insert(2, pinned_2);

    // 1 unpinned node sandwiched between them at x=150, y=100
    // To clear both pinned nodes horizontally, gap would need to be 2*247 = 494px.
    // At gap=300px, it CANNOT fit along X, so it must escape along Y.
    // BUT because dy=0, ux > uy, the solver only pushes along X back and forth!
    positions.insert(3, Point::new(150.0, 100.0));

    let pinned: HashSet<u64> = [1, 2].into_iter().collect();
    resolve_node_overlaps_pinned(&mut positions, &pinned);

    let min_dist_x = NODE_WIDTH + 12.0; // 247.0
    let min_dist_y = NODE_HEIGHT + 12.0; // 108.0

    let p3 = positions[&3];
    let d1x = (p3.x - pinned_1.x).abs();
    let d1y = (p3.y - pinned_1.y).abs();
    let d2x = (p3.x - pinned_2.x).abs();
    let d2y = (p3.y - pinned_2.y).abs();

    let overlaps_1 = d1x < min_dist_x && d1y < min_dist_y;
    let overlaps_2 = d2x < min_dist_x && d2y < min_dist_y;

    // EMPIRICAL FINDING: The solver fails to push node 3 away; node 3 remains overlapping with node 1!
    // pos3 is at (153.0, 100.0) with dx=53.0 < 247.0 and dy=0.0 < 108.0!
    if overlaps_1 || overlaps_2 {
        eprintln!(
            "CONFIRMED BUG: Horizontal sandwich oscillation trapped node 3 at ({}, {}). Overlaps node 1: {overlaps_1}, overlaps node 2: {overlaps_2}",
            p3.x, p3.y
        );
    }
    // We assert that the sandwich is fully cleared:
    assert!(
        !overlaps_1 && !overlaps_2,
        "Expected horizontal sandwich to be fully resolved without overlaps"
    );
}

#[test]
fn test_challenger_pinned_overlap_vertical_sandwich_failure() {
    // 2 pinned nodes with 150px vertical spacing (dy=150 > min_dist_y=108, non-overlapping)
    let mut positions = BTreeMap::new();
    let pinned_1 = Point::new(100.0, 100.0);
    let pinned_2 = Point::new(100.0, 250.0);
    positions.insert(1, pinned_1);
    positions.insert(2, pinned_2);

    // Unpinned node sandwiched between them at x=100, y=175
    // At gap=150px, cannot fit along Y (needs 216px). Must escape along X.
    // BUT because dx=0, uy > ux, solver only pushes along Y!
    positions.insert(3, Point::new(100.0, 175.0));

    let pinned: HashSet<u64> = [1, 2].into_iter().collect();
    resolve_node_overlaps_pinned(&mut positions, &pinned);

    let min_dist_x = NODE_WIDTH + 12.0;
    let min_dist_y = NODE_HEIGHT + 12.0;

    let p3 = positions[&3];
    let d1x = (p3.x - pinned_1.x).abs();
    let d1y = (p3.y - pinned_1.y).abs();
    let d2x = (p3.x - pinned_2.x).abs();
    let d2y = (p3.y - pinned_2.y).abs();

    let overlaps_1 = d1x < min_dist_x && d1y < min_dist_y;
    let overlaps_2 = d2x < min_dist_x && d2y < min_dist_y;

    if overlaps_1 || overlaps_2 {
        eprintln!(
            "CONFIRMED BUG: Vertical sandwich oscillation trapped node 3 at ({}, {}). Overlaps node 1: {overlaps_1}, overlaps node 2: {overlaps_2}",
            p3.x, p3.y
        );
    }
    assert!(
        !overlaps_1 && !overlaps_2,
        "Expected vertical sandwich to be fully resolved without overlaps"
    );
}

// ============================================================================
// 4. Marquee Micro-Drag vs Full Drag Threshold Tests
// ============================================================================

#[test]
fn test_challenger_marquee_micro_drag_vs_full_drag_thresholds() {
    let graph = create_demo_graph();
    let positions = BTreeMap::new();
    let canvas = GraphCanvas {
        graph: &graph,
        visible_nodes: graph.nodes.keys().copied().collect(),
        node_positions: &positions,
        selected_edge: None,
        selected_node: None,
        selected_nodes: HashSet::new(),
        pan: Vector::ZERO,
        zoom: 1.0,
        layer: CanvasLayer::Base,
        search_query: "",
        modifiers: iced::keyboard::Modifiers::default(),
    };

    let bounds = Rectangle {
        x: 0.0,
        y: 0.0,
        width: 1000.0,
        height: 1000.0,
    };

    // Scenario A: Micro-drag (< 3.0px) on empty canvas (e.g. 2.9px x 2.9px)
    // Should be treated as a click -> publishes ClearSelection
    {
        let mut state = CanvasInteraction::default();
        // Mouse press at (500.0, 500.0) on empty space
        let _ = canvas.update(
            &mut state,
            &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            bounds,
            mouse::Cursor::Available(Point::new(500.0, 500.0)),
        );
        assert!(state.marquee_start.is_some());

        // Cursor moved by 2.9px
        let _ = canvas.update(
            &mut state,
            &Event::Mouse(mouse::Event::CursorMoved {
                position: Point::new(502.9, 502.9),
            }),
            bounds,
            mouse::Cursor::Available(Point::new(502.9, 502.9)),
        );
        let m_rect = state.marquee_rect.unwrap();
        assert!((m_rect.width - 2.9).abs() < 1e-4);
        assert!((m_rect.height - 2.9).abs() < 1e-4);

        // Mouse release: width < 3.0 and height < 3.0 -> is_drag is false -> ClearSelection
        let action = canvas.update(
            &mut state,
            &Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            bounds,
            mouse::Cursor::Available(Point::new(502.9, 502.9)),
        );
        assert!(
            action.is_some(),
            "Expected action on micro-drag release on empty canvas"
        );
    }

    // Scenario B: Full drag (>= 3.0px) on empty canvas (e.g. 3.0px x 0.5px)
    // Should be treated as box selection -> publishes SelectNodes
    {
        let mut state = CanvasInteraction::default();
        let _ = canvas.update(
            &mut state,
            &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            bounds,
            mouse::Cursor::Available(Point::new(500.0, 500.0)),
        );
        let _ = canvas.update(
            &mut state,
            &Event::Mouse(mouse::Event::CursorMoved {
                position: Point::new(503.0, 500.5),
            }),
            bounds,
            mouse::Cursor::Available(Point::new(503.0, 500.5)),
        );
        let m_rect = state.marquee_rect.unwrap();
        assert!((m_rect.width - 3.0).abs() < 1e-4);
        assert!((m_rect.height - 0.5).abs() < 1e-4);

        let action = canvas.update(
            &mut state,
            &Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            bounds,
            mouse::Cursor::Available(Point::new(503.0, 500.5)),
        );
        assert!(action.is_some(), "Expected SelectNodes action on full drag");
    }
}

// ============================================================================
// 5. Stress Tests: High-Density Clusters & Heavy Pinned Overlap Collisions
// ============================================================================

#[test]
fn test_challenger_stress_pinned_overlap_drop_3_on_3_full_clearance() {
    let mut positions = BTreeMap::new();
    let drop_1 = Point::new(100.0, 100.0);
    let drop_2 = Point::new(400.0, 100.0);
    let drop_3 = Point::new(700.0, 100.0);

    // Group of 3 nodes dragged and dropped directly on top of 3 unpinned existing nodes
    positions.insert(1, drop_1);
    positions.insert(2, drop_2);
    positions.insert(3, drop_3);

    positions.insert(4, drop_1);
    positions.insert(5, drop_2);
    positions.insert(6, drop_3);

    let pinned: HashSet<u64> = [1, 2, 3].into_iter().collect();

    resolve_node_overlaps_pinned(&mut positions, &pinned);

    let min_dist_x = NODE_WIDTH + 12.0; // 247.0
    let min_dist_y = NODE_HEIGHT + 12.0; // 108.0

    let keys: Vec<u64> = positions.keys().copied().collect();
    let mut violations = Vec::new();
    for i in 0..keys.len() {
        for j in (i + 1)..keys.len() {
            let k1 = keys[i];
            let k2 = keys[j];
            let p1 = positions[&k1];
            let p2 = positions[&k2];
            let dx = (p1.x - p2.x).abs();
            let dy = (p1.y - p2.y).abs();
            if dx < min_dist_x && dy < min_dist_y {
                violations.push(format!(
                    "Nodes {} and {} overlap: dx={dx} (< {min_dist_x}), dy={dy} (< {min_dist_y})",
                    k1, k2
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "Requirement R5 violation: unpinned colliding nodes must be pushed away until NO overlap exists, but found: {:?}",
        violations
    );
}

#[test]
fn test_challenger_stress_high_density_clusters_heavy_collisions() {
    let mut positions = BTreeMap::new();
    // 4 pinned nodes arranged in a 2x2 cluster
    positions.insert(1, Point::new(100.0, 100.0));
    positions.insert(2, Point::new(400.0, 100.0));
    positions.insert(3, Point::new(100.0, 300.0));
    positions.insert(4, Point::new(400.0, 300.0));

    // 8 unpinned nodes heavily colliding in the same area
    positions.insert(5, Point::new(100.0, 100.0));
    positions.insert(6, Point::new(400.0, 100.0));
    positions.insert(7, Point::new(100.0, 300.0));
    positions.insert(8, Point::new(400.0, 300.0));
    positions.insert(9, Point::new(250.0, 100.0));
    positions.insert(10, Point::new(250.0, 300.0));
    positions.insert(11, Point::new(100.0, 200.0));
    positions.insert(12, Point::new(400.0, 200.0));

    let pinned: HashSet<u64> = [1, 2, 3, 4].into_iter().collect();

    resolve_node_overlaps_pinned(&mut positions, &pinned);

    let min_dist_x = NODE_WIDTH + 12.0;
    let min_dist_y = NODE_HEIGHT + 12.0;

    let keys: Vec<u64> = positions.keys().copied().collect();
    let mut violations = Vec::new();
    for i in 0..keys.len() {
        for j in (i + 1)..keys.len() {
            let k1 = keys[i];
            let k2 = keys[j];
            let p1 = positions[&k1];
            let p2 = positions[&k2];
            let dx = (p1.x - p2.x).abs();
            let dy = (p1.y - p2.y).abs();
            if dx < min_dist_x && dy < min_dist_y {
                violations.push(format!(
                    "Nodes {} and {} overlap: dx={dx} (< {min_dist_x}), dy={dy} (< {min_dist_y})",
                    k1, k2
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "High-density stress test failed: unpinned colliding nodes must be resolved without remaining overlaps: {:?}",
        violations
    );
}
