#![allow(dead_code, unused_imports, unused_variables)]

#[path = "../src/model.rs"]
mod model;

#[path = "../src/canvas_view.rs"]
mod canvas_view;

use canvas_view::{
    apply_box_selection, apply_group_drag_delta, compute_box_rect, compute_intersecting_nodes,
    node_intersects_rect, resolve_node_overlaps_pinned, toggle_selection, CanvasInteraction,
    CanvasLayer, GraphCanvas, GroupDragState, NODE_HEIGHT, NODE_WIDTH,
};
use iced::keyboard::Modifiers;
use iced::{Point, Rectangle, Size, Vector};
use model::{create_demo_graph, NodeCategory, PwGraph, PwNode};
use std::collections::{BTreeMap, HashSet};

// ============================================================================
// 1. Marquee Geometry & Bounding Box Intersection Tests
// ============================================================================

#[test]
fn test_r5_marquee_rect_computation_all_quadrants() {
    // Quadrant 1: Down-right drag (start < end in both axes)
    let p1 = Point::new(10.0, 20.0);
    let p2 = Point::new(100.0, 150.0);
    let r1 = compute_box_rect(p1, p2);
    assert_eq!(r1.x, 10.0);
    assert_eq!(r1.y, 20.0);
    assert_eq!(r1.width, 90.0);
    assert_eq!(r1.height, 130.0);

    // Quadrant 2: Up-right drag (p2.x > p1.x, p2.y < p1.y)
    let p3 = Point::new(10.0, 150.0);
    let p4 = Point::new(100.0, 20.0);
    let r2 = compute_box_rect(p3, p4);
    assert_eq!(r2.x, 10.0);
    assert_eq!(r2.y, 20.0);
    assert_eq!(r2.width, 90.0);
    assert_eq!(r2.height, 130.0);

    // Quadrant 3: Up-left drag (p2.x < p1.x, p2.y < p1.y)
    let p5 = Point::new(100.0, 150.0);
    let p6 = Point::new(10.0, 20.0);
    let r3 = compute_box_rect(p5, p6);
    assert_eq!(r3.x, 10.0);
    assert_eq!(r3.y, 20.0);
    assert_eq!(r3.width, 90.0);
    assert_eq!(r3.height, 130.0);

    // Quadrant 4: Down-left drag (p2.x < p1.x, p2.y > p1.y)
    let p7 = Point::new(100.0, 20.0);
    let p8 = Point::new(10.0, 150.0);
    let r4 = compute_box_rect(p7, p8);
    assert_eq!(r4.x, 10.0);
    assert_eq!(r4.y, 20.0);
    assert_eq!(r4.width, 90.0);
    assert_eq!(r4.height, 130.0);

    // Edge case: Zero-area point drag
    let p_zero = Point::new(50.0, 50.0);
    let r_zero = compute_box_rect(p_zero, p_zero);
    assert_eq!(r_zero.x, 50.0);
    assert_eq!(r_zero.y, 50.0);
    assert_eq!(r_zero.width, 0.0);
    assert_eq!(r_zero.height, 0.0);
}

#[test]
fn test_r5_node_intersects_rect_bounding_box_checks() {
    // Node positioned at (100.0, 100.0) with standard NODE_WIDTH and NODE_HEIGHT
    let node_pos = Point::new(100.0, 100.0);
    let nw = NODE_WIDTH; // 235.0 -> [100.0, 335.0]
    let nh = NODE_HEIGHT; // 96.0  -> [100.0, 196.0]

    // 1. Box completely inside the node
    let inside_rect = Rectangle {
        x: 120.0,
        y: 120.0,
        width: 50.0,
        height: 40.0,
    };
    assert!(
        node_intersects_rect(node_pos, &inside_rect),
        "Box inside node must intersect"
    );

    // 2. Box completely enclosing the node
    let enclosing_rect = Rectangle {
        x: 50.0,
        y: 50.0,
        width: 500.0,
        height: 300.0,
    };
    assert!(
        node_intersects_rect(node_pos, &enclosing_rect),
        "Box enclosing node must intersect"
    );

    // 3. Partial overlap: top-left corner
    let top_left_rect = Rectangle {
        x: 50.0,
        y: 50.0,
        width: 60.0, // spans [50, 110], node starts at 100
        height: 60.0,
    };
    assert!(
        node_intersects_rect(node_pos, &top_left_rect),
        "Top-left overlapping box must intersect"
    );

    // 4. Partial overlap: bottom-right corner
    let bottom_right_rect = Rectangle {
        x: 300.0, // node ends at 335
        y: 180.0, // node ends at 196
        width: 50.0,
        height: 50.0,
    };
    assert!(
        node_intersects_rect(node_pos, &bottom_right_rect),
        "Bottom-right overlapping box must intersect"
    );

    // 5. Completely outside: strictly to the left
    let left_rect = Rectangle {
        x: 10.0,
        y: 100.0,
        width: 80.0, // spans [10, 90]
        height: 50.0,
    };
    assert!(
        !node_intersects_rect(node_pos, &left_rect),
        "Box to the left must NOT intersect"
    );

    // 6. Completely outside: strictly to the right
    let right_rect = Rectangle {
        x: 350.0,
        y: 100.0,
        width: 50.0,
        height: 50.0,
    };
    assert!(
        !node_intersects_rect(node_pos, &right_rect),
        "Box to the right must NOT intersect"
    );

    // 7. Completely outside: strictly above
    let above_rect = Rectangle {
        x: 100.0,
        y: 20.0,
        width: 50.0,
        height: 70.0, // spans [20, 90]
    };
    assert!(
        !node_intersects_rect(node_pos, &above_rect),
        "Box above must NOT intersect"
    );

    // 8. Completely outside: strictly below
    let below_rect = Rectangle {
        x: 100.0,
        y: 210.0,
        width: 50.0,
        height: 50.0,
    };
    assert!(
        !node_intersects_rect(node_pos, &below_rect),
        "Box below must NOT intersect"
    );
}

#[test]
fn test_r5_compute_intersecting_nodes() {
    let mut positions = BTreeMap::new();
    positions.insert(1, Point::new(0.0, 0.0));
    positions.insert(2, Point::new(300.0, 0.0));
    positions.insert(3, Point::new(0.0, 200.0));
    positions.insert(4, Point::new(300.0, 200.0));

    let visible: HashSet<u64> = [1, 2, 3, 4].into_iter().collect();

    // Box selecting only Node 1
    let r1 = Rectangle {
        x: 0.0,
        y: 0.0,
        width: 50.0,
        height: 50.0,
    };
    let hit1 = compute_intersecting_nodes(&positions, &visible, &r1);
    assert_eq!(hit1, [1].into_iter().collect());

    // Box spanning across top row: Nodes 1 and 2
    let r_top = Rectangle {
        x: 50.0,
        y: 10.0,
        width: 350.0, // covers [50, 400], Node 1 is [0, 235], Node 2 is [300, 535]
        height: 50.0,
    };
    let hit_top = compute_intersecting_nodes(&positions, &visible, &r_top);
    assert_eq!(hit_top, [1, 2].into_iter().collect());

    // Box spanning all four nodes
    let r_all = Rectangle {
        x: 10.0,
        y: 10.0,
        width: 500.0,
        height: 300.0,
    };
    let hit_all = compute_intersecting_nodes(&positions, &visible, &r_all);
    assert_eq!(hit_all, [1, 2, 3, 4].into_iter().collect());

    // Box in empty space (e.g., between rows, x=[240, 290], y=[100, 190])
    let r_gap = Rectangle {
        x: 240.0,
        y: 110.0,
        width: 50.0,
        height: 80.0,
    };
    let hit_gap = compute_intersecting_nodes(&positions, &visible, &r_gap);
    assert!(hit_gap.is_empty(), "Gap box must intersect no nodes");

    // Hidden node (not in visible set) must never be selected
    let visible_partial: HashSet<u64> = [2, 4].into_iter().collect();
    let hit_filtered = compute_intersecting_nodes(&positions, &visible_partial, &r_all);
    assert_eq!(hit_filtered, [2, 4].into_iter().collect());
}

// ============================================================================
// 2. Selection Applicator & Multi-Select Operations
// ============================================================================

#[test]
fn test_r5_apply_box_selection_normal_mode_replaces_selection() {
    let mut selected: HashSet<u64> = [10, 20].into_iter().collect();
    let intersecting: HashSet<u64> = [30, 40].into_iter().collect();

    apply_box_selection(&mut selected, &intersecting, false);
    assert_eq!(
        selected,
        [30, 40].into_iter().collect(),
        "Normal box selection must replace the existing selection"
    );

    // Empty intersection clears selection
    let empty_set: HashSet<u64> = HashSet::new();
    apply_box_selection(&mut selected, &empty_set, false);
    assert!(selected.is_empty(), "Selecting empty box clears selection");
}

#[test]
fn test_r5_apply_box_selection_toggle_mode_performs_symmetric_difference() {
    let mut selected: HashSet<u64> = [10, 20, 30].into_iter().collect();
    // Intersecting contains 20 (already selected) and 40 (not selected)
    let intersecting: HashSet<u64> = [20, 40].into_iter().collect();

    apply_box_selection(&mut selected, &intersecting, true);
    // 10 remains, 30 remains, 20 is toggled OFF, 40 is toggled ON
    assert_eq!(
        selected,
        [10, 30, 40].into_iter().collect(),
        "Toggle box selection must toggle intersecting nodes in/out"
    );

    // Toggle again with the same intersecting set: 20 becomes ON, 40 becomes OFF
    apply_box_selection(&mut selected, &intersecting, true);
    assert_eq!(
        selected,
        [10, 20, 30].into_iter().collect(),
        "Re-toggling must restore previous state"
    );
}

#[test]
fn test_r5_toggle_selection_single_node() {
    let mut selected: HashSet<u64> = HashSet::new();

    // Toggle on
    toggle_selection(&mut selected, 10);
    assert!(selected.contains(&10));
    assert_eq!(selected.len(), 1);

    // Toggle on another
    toggle_selection(&mut selected, 20);
    assert!(selected.contains(&10));
    assert!(selected.contains(&20));
    assert_eq!(selected.len(), 2);

    // Toggle off first node
    toggle_selection(&mut selected, 10);
    assert!(!selected.contains(&10));
    assert!(selected.contains(&20));
    assert_eq!(selected.len(), 1);

    // Toggle off second node
    toggle_selection(&mut selected, 20);
    assert!(selected.is_empty());
}

// ============================================================================
// 3. Group Drag Delta & Translation Invariants
// ============================================================================

#[test]
fn test_r5_apply_group_drag_delta_exact_translation() {
    let mut initial_positions = BTreeMap::new();
    initial_positions.insert(10, Point::new(100.0, 150.0));
    initial_positions.insert(20, Point::new(250.0, 300.0));
    initial_positions.insert(30, Point::new(450.0, 50.0));

    let delta = Vector::new(35.5, -42.25);
    let updated = apply_group_drag_delta(&initial_positions, delta);

    // All nodes must be present
    assert_eq!(updated.len(), 3);

    // Verify absolute translations
    assert_eq!(updated[&10], Point::new(135.5, 107.75));
    assert_eq!(updated[&20], Point::new(285.5, 257.75));
    assert_eq!(updated[&30], Point::new(485.5, 7.75));

    // Verify relative pairwise distances are strictly preserved (rigid body translation)
    let init_vec_10_20 = Vector::new(
        initial_positions[&20].x - initial_positions[&10].x,
        initial_positions[&20].y - initial_positions[&10].y,
    );
    let updated_vec_10_20 =
        Vector::new(updated[&20].x - updated[&10].x, updated[&20].y - updated[&10].y);
    assert_eq!(init_vec_10_20, updated_vec_10_20);

    let init_vec_20_30 = Vector::new(
        initial_positions[&30].x - initial_positions[&20].x,
        initial_positions[&30].y - initial_positions[&20].y,
    );
    let updated_vec_20_30 =
        Vector::new(updated[&30].x - updated[&20].x, updated[&30].y - updated[&20].y);
    assert_eq!(init_vec_20_30, updated_vec_20_30);
}

// ============================================================================
// 4. Pinned Overlap Nudging Invariants (Group Drag Settlement)
// ============================================================================

#[test]
fn test_r5_pinned_overlap_nudging_pinned_nodes_never_move() {
    let mut positions = BTreeMap::new();
    // Pinned group: Nodes 10 and 20
    let pinned_10_pos = Point::new(200.0, 200.0);
    let pinned_20_pos = Point::new(500.0, 200.0);
    positions.insert(10, pinned_10_pos);
    positions.insert(20, pinned_20_pos);

    // External unpinned nodes overlapping with the pinned nodes:
    // Node 30 overlaps pinned 10
    positions.insert(30, Point::new(210.0, 205.0));
    // Node 40 overlaps pinned 20
    positions.insert(40, Point::new(490.0, 195.0));
    // Node 50 non-overlapping
    positions.insert(50, Point::new(800.0, 600.0));

    let pinned: HashSet<u64> = [10, 20].into_iter().collect();

    resolve_node_overlaps_pinned(&mut positions, &pinned);

    // INVARIANT 1: Pinned nodes must remain EXACTLY at their pre-solve coordinates
    assert_eq!(
        positions[&10], pinned_10_pos,
        "Pinned node 10 MUST NOT move during overlap resolution"
    );
    assert_eq!(
        positions[&20], pinned_20_pos,
        "Pinned node 20 MUST NOT move during overlap resolution"
    );

    // INVARIANT 2: External overlapping nodes must be nudged away to satisfy clearance
    let min_dist_x = NODE_WIDTH + 12.0; // 247.0
    let min_dist_y = NODE_HEIGHT + 12.0; // 108.0

    // Node 30 vs Node 10
    let dx_30_10 = (positions[&30].x - positions[&10].x).abs();
    let dy_30_10 = (positions[&30].y - positions[&10].y).abs();
    assert!(
        dx_30_10 >= min_dist_x || dy_30_10 >= min_dist_y,
        "Node 30 must be nudged away from pinned node 10: dx={dx_30_10}, dy={dy_30_10}"
    );

    // Node 40 vs Node 20
    let dx_40_20 = (positions[&40].x - positions[&20].x).abs();
    let dy_40_20 = (positions[&40].y - positions[&20].y).abs();
    assert!(
        dx_40_20 >= min_dist_x || dy_40_20 >= min_dist_y,
        "Node 40 must be nudged away from pinned node 20: dx={dx_40_20}, dy={dy_40_20}"
    );

    // Invariant 3: No external-external collisions
    let dx_30_40 = (positions[&30].x - positions[&40].x).abs();
    let dy_30_40 = (positions[&30].y - positions[&40].y).abs();
    assert!(
        dx_30_40 >= min_dist_x || dy_30_40 >= min_dist_y,
        "Node 30 and 40 must not overlap after resolution: dx={dx_30_40}, dy={dy_30_40}"
    );
}

#[test]
fn test_r5_pinned_overlap_nudging_overlapping_group_members_never_nudge_each_other() {
    let mut positions = BTreeMap::new();
    // Suppose user has nodes in selection that overlap each other
    let pos_a = Point::new(100.0, 100.0);
    let pos_b = Point::new(110.0, 110.0); // directly overlaps pos_a
    positions.insert(1, pos_a);
    positions.insert(2, pos_b);

    // An external node
    positions.insert(3, Point::new(105.0, 105.0)); // also overlaps both

    let pinned: HashSet<u64> = [1, 2].into_iter().collect();

    resolve_node_overlaps_pinned(&mut positions, &pinned);

    // Invariant: Pinned 1 and Pinned 2 must NEVER nudge each other!
    assert_eq!(
        positions[&1], pos_a,
        "Pinned group member 1 must remain stationary"
    );
    assert_eq!(
        positions[&2], pos_b,
        "Pinned group member 2 must remain stationary"
    );

    // External node 3 must yield to both
    let min_dist_x = NODE_WIDTH + 12.0;
    let min_dist_y = NODE_HEIGHT + 12.0;

    let dx_3_1 = (positions[&3].x - positions[&1].x).abs();
    let dy_3_1 = (positions[&3].y - positions[&1].y).abs();
    assert!(
        dx_3_1 >= min_dist_x || dy_3_1 >= min_dist_y,
        "Node 3 must yield to pinned node 1"
    );

    let dx_3_2 = (positions[&3].x - positions[&2].x).abs();
    let dy_3_2 = (positions[&3].y - positions[&2].y).abs();
    assert!(
        dx_3_2 >= min_dist_x || dy_3_2 >= min_dist_y,
        "Node 3 must yield to pinned node 2"
    );
}

#[test]
fn test_r5_pinned_overlap_nudging_cascade_resolution() {
    let mut positions = BTreeMap::new();
    // Pinned node at (200, 200)
    let pinned_pos = Point::new(200.0, 200.0);
    positions.insert(1, pinned_pos);

    // Chain of unpinned nodes closely packed behind it
    positions.insert(2, Point::new(205.0, 205.0)); // Collides with 1
    positions.insert(3, Point::new(210.0, 210.0)); // Collides with 1 & 2
    positions.insert(4, Point::new(215.0, 215.0)); // Collides with 1 & 2 & 3

    let pinned: HashSet<u64> = [1].into_iter().collect();
    resolve_node_overlaps_pinned(&mut positions, &pinned);

    // Pinned 1 must remain at (200, 200)
    assert_eq!(positions[&1], pinned_pos);

    let min_dist_x = NODE_WIDTH + 12.0;
    let min_dist_y = NODE_HEIGHT + 12.0;

    // Check all pairs for non-overlap
    let keys: Vec<u64> = positions.keys().copied().collect();
    for i in 0..keys.len() {
        for j in (i + 1)..keys.len() {
            let k1 = keys[i];
            let k2 = keys[j];
            let p1 = positions[&k1];
            let p2 = positions[&k2];
            let dx = (p1.x - p2.x).abs();
            let dy = (p1.y - p2.y).abs();
            assert!(
                dx >= min_dist_x || dy >= min_dist_y,
                "Nodes {k1} and {k2} must not overlap after cascade: dx={dx}, dy={dy}"
            );
        }
    }
}

// ============================================================================
// 5. GraphCanvas Selection Query Helpers
// ============================================================================

#[test]
fn test_r5_graph_canvas_selection_queries() {
    let graph = create_demo_graph();
    let positions = BTreeMap::new();

    // 1. Empty selection
    let canvas_empty = GraphCanvas {
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
    assert!(!canvas_empty.is_node_selected(10));
    assert_eq!(canvas_empty.selected_node(), None);

    // 2. Single selection in selected_nodes
    let mut single_sel = HashSet::new();
    single_sel.insert(10);
    let canvas_single = GraphCanvas {
        graph: &graph,
        visible_nodes: graph.nodes.keys().copied().collect(),
        node_positions: &positions,
        selected_edge: None,
        selected_node: None,
        selected_nodes: single_sel,
        pan: Vector::ZERO,
        zoom: 1.0,
        layer: CanvasLayer::Base,
        search_query: "",
        modifiers: iced::keyboard::Modifiers::default(),
    };
    assert!(canvas_single.is_node_selected(10));
    assert!(!canvas_single.is_node_selected(20));
    assert_eq!(canvas_single.selected_node(), Some(10));

    // 3. Multi selection in selected_nodes
    let mut multi_sel = HashSet::new();
    multi_sel.insert(10);
    multi_sel.insert(20);
    multi_sel.insert(30);
    let canvas_multi = GraphCanvas {
        graph: &graph,
        visible_nodes: graph.nodes.keys().copied().collect(),
        node_positions: &positions,
        selected_edge: None,
        selected_node: None,
        selected_nodes: multi_sel,
        pan: Vector::ZERO,
        zoom: 1.0,
        layer: CanvasLayer::Base,
        search_query: "",
        modifiers: iced::keyboard::Modifiers::default(),
    };
    assert!(canvas_multi.is_node_selected(10));
    assert!(canvas_multi.is_node_selected(20));
    assert!(canvas_multi.is_node_selected(30));
    assert!(!canvas_multi.is_node_selected(40));
    // Multiple nodes selected -> selected_node() returns None
    assert_eq!(canvas_multi.selected_node(), None);
}
