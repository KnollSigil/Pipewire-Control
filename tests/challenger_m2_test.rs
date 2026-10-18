#![allow(dead_code, unused_imports, unused_variables)]

#[path = "../src/model.rs"]
mod model;
#[path = "../src/canvas_view.rs"]
mod canvas_view;

use canvas_view::{
    compute_tooltip_rect, compute_tooltip_width, should_show_link_badge, CanvasInteraction,
    CanvasLayer, CanvasMessage, GraphCanvas, NODE_HEIGHT, NODE_WIDTH,
};
use iced::mouse;
use iced::widget::canvas::{self, Event, Program};
use iced::{Point, Rectangle, Size, Vector};
use model::{create_demo_graph, CollapsedEdge, EdgeKey, PwGraph};
use std::collections::{BTreeMap, HashSet};

// =========================================================================
// 1. TRUTH TABLE COMPLETENESS FOR `should_show_link_badge` (R3)
// =========================================================================

#[test]
fn challenge_badge_truth_table_exhaustive() {
    // There are 2^3 = 8 possible combinations of (is_default, is_hovered, is_selected).
    // Requirement R3:
    // "The mini routing-matrix badge on a link is shown only when the connection is
    //  non-default, or the link is hovered, or the link is selected.
    //  Default, un-hovered, unselected links show no badge."

    let truth_table = [
        // (is_default, is_hovered, is_selected, expected_visible, description)
        (
            false,
            false,
            false,
            true,
            "Non-default, unhovered, unselected -> MUST show badge",
        ),
        (
            false,
            false,
            true,
            true,
            "Non-default, unhovered, selected -> MUST show badge",
        ),
        (
            false,
            true,
            false,
            true,
            "Non-default, hovered, unselected -> MUST show badge",
        ),
        (
            false,
            true,
            true,
            true,
            "Non-default, hovered, selected -> MUST show badge",
        ),
        (
            true,
            false,
            false,
            false,
            "Default, unhovered, unselected -> MUST NOT show badge (ONLY FALSE CASE)",
        ),
        (
            true,
            false,
            true,
            true,
            "Default, unhovered, selected -> MUST show badge",
        ),
        (
            true,
            true,
            false,
            true,
            "Default, hovered, unselected -> MUST show badge",
        ),
        (
            true,
            true,
            true,
            true,
            "Default, hovered, selected -> MUST show badge",
        ),
    ];

    let mut false_positives = 0;
    let mut false_negatives = 0;

    for (is_default, is_hovered, is_selected, expected, desc) in truth_table {
        let actual = should_show_link_badge(is_default, is_hovered, is_selected);
        if actual && !expected {
            false_positives += 1;
        }
        if !actual && expected {
            false_negatives += 1;
        }
        assert_eq!(actual, expected, "Failed branch: {}", desc);
    }

    assert_eq!(false_positives, 0, "Expected zero false positives");
    assert_eq!(false_negatives, 0, "Expected zero false negatives");
}

// =========================================================================
// 2. MEASURE TOOLTIP RENDERED MARGIN TIGHT-FITTING & PROPORTIONALITY (R2)
// =========================================================================

#[test]
fn challenge_tooltip_horizontal_padding_and_margin_tightness() {
    // Dispatch requirement:
    // "Measure tooltip rendered margin tight-fitting: verify that tooltip padding does not
    //  exceed 16px total horizontal padding and that width strictly scales with string length."

    let text_sizes = [9.5_f32, 10.0, 11.5, 12.0, 14.0, 20.0];

    for &ts in &text_sizes {
        // Test empty string
        let w_empty = compute_tooltip_width("", ts);
        assert_eq!(
            w_empty, 16.0,
            "Empty string tooltip width must be exactly 16.0 (total padding)"
        );

        // Test strings of varying character compositions
        let test_labels = [
            "a",
            "FL",
            "FL, FR",
            "capture_1 -> playback_1",
            "Built-in Audio -> Headphones (default)",
            "Extremely long pipewire routing link tooltip with multiple input and output channels mapped",
        ];

        for &label in &test_labels {
            let w = compute_tooltip_width(label, ts);

            // Recompute text units manually to check exact padding
            let char_units: f32 = label
                .chars()
                .map(|c| match c {
                    'i' | 'l' | '!' | '|' | ':' | ';' | '.' | ',' | '\'' | '`' => 0.28,
                    'r' | 't' | 'f' | 'j' | '(' | ')' | '[' | ']' | '{' | '}' | '-' => 0.38,
                    'm' | 'w' | 'M' | 'W' => 0.82,
                    'A'..='Z' => 0.65,
                    ' ' => 0.32,
                    _ => 0.54,
                })
                .sum();

            let estimated_text_w = char_units * ts;
            let total_horizontal_padding = w - estimated_text_w;

            assert!(
                (total_horizontal_padding - 16.0).abs() < 1e-4,
                "Tooltip horizontal padding must be exactly 16.0px (8px left + 8px right), got {}",
                total_horizontal_padding
            );
            assert!(
                total_horizontal_padding <= 16.0 + 1e-4,
                "Tooltip padding must not exceed 16px total horizontal padding, got {}",
                total_horizontal_padding
            );
        }
    }
}

#[test]
fn challenge_tooltip_width_strict_scaling_with_length() {
    let ts = 11.5_f32;

    // 1. Strict monotonicity: every added character increases width
    let mut current_str = String::new();
    let mut prev_width = compute_tooltip_width(&current_str, ts);

    let test_alphabet = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_:., ()";
    for c in test_alphabet.chars() {
        current_str.push(c);
        let new_width = compute_tooltip_width(&current_str, ts);
        assert!(
            new_width > prev_width,
            "Adding char '{}' must strictly increase width: prev {}, new {}",
            c,
            prev_width,
            new_width
        );
        let delta = new_width - prev_width;
        // Each character should contribute between 0.28*ts and 0.82*ts
        assert!(
            delta >= 0.28 * ts - 1e-4 && delta <= 0.82 * ts + 1e-4,
            "Delta {} for char '{}' outside expected unit range",
            delta,
            c
        );
        prev_width = new_width;
    }

    // 2. Strict proportionality: N identical characters have exact N * unit width
    for char_class in ['i', 'r', ' ', 'x', 'A', 'W'] {
        let w1 = compute_tooltip_width(&char_class.to_string(), ts) - 16.0;
        for n in [2, 5, 10, 20, 50, 100] {
            let s: String = std::iter::repeat_n(char_class, n).collect();
            let wn = compute_tooltip_width(&s, ts) - 16.0;
            let ratio = wn / w1;
            assert!(
                (ratio - n as f32).abs() < 1e-3,
                "Width for {}x'{}' must be exactly {}x single char width, got ratio {}",
                n,
                char_class,
                n,
                ratio
            );
        }
    }

    // 3. No artificial clamp / excessive margin for short strings
    let w_single = compute_tooltip_width("x", ts);
    assert!(
        w_single < 25.0,
        "Single char width must be tight (< 25px), got {}",
        w_single
    );
}

#[test]
fn challenge_tooltip_bounds_clamping_and_anchor_offset() {
    let bounds = Rectangle::new(Point::new(50.0, 50.0), Size::new(600.0, 400.0));
    let ts = 11.0;
    let label = "Stereo Out -> Main (default)";
    let w = compute_tooltip_width(label, ts);
    let h = ts + 10.0;

    // Normal anchor in middle of canvas
    let mid_anchor = Point::new(350.0, 250.0);
    let rect = compute_tooltip_rect(label, ts, mid_anchor, bounds);
    assert!((rect.x + rect.width * 0.5 - mid_anchor.x).abs() < 1e-4);
    assert_eq!(rect.y, mid_anchor.y - h - 10.0);

    // Anchor near top edge: must flip below anchor
    let top_anchor = Point::new(350.0, 60.0);
    let rect_top = compute_tooltip_rect(label, ts, top_anchor, bounds);
    assert_eq!(
        rect_top.y,
        top_anchor.y + 16.0,
        "Near top border, tooltip should flip below anchor"
    );

    // Extreme boundaries
    let extreme_anchors = [
        Point::new(0.0, 0.0),
        Point::new(1000.0, 1000.0),
        Point::new(-500.0, -500.0),
        Point::new(bounds.x, bounds.y),
        Point::new(bounds.x + bounds.width, bounds.y + bounds.height),
    ];

    for anchor in extreme_anchors {
        let r = compute_tooltip_rect(label, ts, anchor, bounds);
        assert!(r.x >= bounds.x, "r.x ({}) < bounds.x ({})", r.x, bounds.x);
        assert!(r.y >= bounds.y, "r.y ({}) < bounds.y ({})", r.y, bounds.y);
        assert!(
            r.x + r.width <= bounds.x + bounds.width + 1e-4,
            "r right ({}) > bounds right ({})",
            r.x + r.width,
            bounds.x + bounds.width
        );
        assert!(
            r.y + r.height <= bounds.y + bounds.height + 1e-4,
            "r bottom ({}) > bounds bottom ({})",
            r.y + r.height,
            bounds.y + bounds.height
        );
    }
}

// =========================================================================
// 3. CURSOR TRACKING IN CanvasInteraction & Program::update (R2)
// =========================================================================

fn make_test_graph() -> (PwGraph, HashSet<u64>, BTreeMap<u64, Point>, EdgeKey) {
    let graph = create_demo_graph();

    // Node 20 is Firefox (output), Node 50 is Built-in Headphones (input)
    let node1_serial = 20;
    let node2_serial = 50;

    let mut visible_nodes = HashSet::new();
    visible_nodes.insert(node1_serial);
    visible_nodes.insert(node2_serial);

    let mut node_positions = BTreeMap::new();
    node_positions.insert(node1_serial, Point::new(100.0, 200.0));
    node_positions.insert(node2_serial, Point::new(500.0, 200.0));

    let edge_key = EdgeKey {
        output_node_serial: node1_serial,
        input_node_serial: node2_serial,
    };

    (graph, visible_nodes, node_positions, edge_key)
}

#[test]
fn challenge_canvas_interaction_cursor_tracking_on_hovered_edge() {
    let (graph, visible_nodes, node_positions, edge_key) = make_test_graph();

    let canvas = GraphCanvas {
        graph: &graph,
        visible_nodes,
        node_positions: &node_positions,
        selected_edge: None,
        selected_node: None,
        pan: Vector::ZERO,
        zoom: 1.0,
        layer: CanvasLayer::Base,
        search_query: "",
    };

    let bounds = Rectangle::new(Point::ORIGIN, Size::new(800.0, 600.0));
    let mut state = CanvasInteraction::default();

    // Node 20 is at (100, 200), NODE_WIDTH = 235.0, NODE_HEIGHT = 96.0.
    // Output socket is at (100 + 235, 200 + 96/2) = (335.0, 248.0).
    // Node 50 is at (500, 200). Input socket is at (500, 200 + 96/2) = (500.0, 248.0).
    // The link bezier goes from (335.0, 248.0) to (500.0, 248.0).
    // Midpoint is at (417.5, 248.0).
    let edge_mid = Point::new(417.5, 248.0);

    // 1. Move cursor onto the edge midpoint
    let event1 = Event::Mouse(mouse::Event::CursorMoved {
        position: edge_mid,
    });
    let action1 = canvas.update(&mut state, &event1, bounds, mouse::Cursor::Available(edge_mid));

    assert_eq!(state.cursor_pos, Some(edge_mid));
    assert_eq!(
        state.hovered_edge,
        Some(edge_key.clone()),
        "Cursor at edge midpoint must hover the edge"
    );
    assert!(
        action1.is_some(),
        "Entering edge must request redraw"
    );

    // 2. Move cursor SLIGHTLY along the edge (stays on the edge!)
    // prev_edge is Some(edge_key), and new hovered_edge is STILL Some(edge_key)
    let edge_point2 = Point::new(420.0, 248.0);
    let event2 = Event::Mouse(mouse::Event::CursorMoved {
        position: edge_point2,
    });
    let action2 = canvas.update(
        &mut state,
        &event2,
        bounds,
        mouse::Cursor::Available(edge_point2),
    );

    assert_eq!(state.cursor_pos, Some(edge_point2));
    assert_eq!(
        state.hovered_edge,
        Some(edge_key),
        "Edge must REMAIN hovered while moving along edge"
    );
    // Crucial check: Program::update MUST publish request_redraw because state.hovered_edge.is_some()!
    assert!(
        action2.is_some(),
        "Moving cursor WHILE edge is hovered MUST request redraw to update tooltip position!"
    );

    // 3. Move cursor to empty space (no edge, no node, no socket)
    let empty_point = Point::new(50.0, 50.0);
    let event3 = Event::Mouse(mouse::Event::CursorMoved {
        position: empty_point,
    });
    let action3 = canvas.update(
        &mut state,
        &event3,
        bounds,
        mouse::Cursor::Available(empty_point),
    );

    assert_eq!(state.cursor_pos, Some(empty_point));
    assert_eq!(state.hovered_edge, None);
    assert!(
        action3.is_some(),
        "Leaving edge to empty canvas must request redraw (edge unhovered)"
    );

    // 4. Move cursor within empty space again!
    // prev_edge == None, hovered_edge == None, no sockets/nodes
    let empty_point2 = Point::new(55.0, 55.0);
    let event4 = Event::Mouse(mouse::Event::CursorMoved {
        position: empty_point2,
    });
    let action4 = canvas.update(
        &mut state,
        &event4,
        bounds,
        mouse::Cursor::Available(empty_point2),
    );

    assert_eq!(state.cursor_pos, Some(empty_point2));
    assert_eq!(state.hovered_edge, None);
    assert!(
        action4.is_none(),
        "Moving cursor across empty canvas must NOT request redraw (action must be None)"
    );

    // 5. Move cursor inside Node 1 card
    let node_point = Point::new(150.0, 230.0);
    let event5 = Event::Mouse(mouse::Event::CursorMoved {
        position: node_point,
    });
    let action5 = canvas.update(
        &mut state,
        &event5,
        bounds,
        mouse::Cursor::Available(node_point),
    );

    assert_eq!(state.hovered_node, Some(20));
    assert!(
        action5.is_some(),
        "Entering node card must request redraw"
    );

    // 6. Move cursor WITHIN Node 1 card (node hover unchanged, no edge)
    let node_point2 = Point::new(155.0, 235.0);
    let event6 = Event::Mouse(mouse::Event::CursorMoved {
        position: node_point2,
    });
    let action6 = canvas.update(
        &mut state,
        &event6,
        bounds,
        mouse::Cursor::Available(node_point2),
    );

    assert_eq!(state.hovered_node, Some(20));
    assert_eq!(state.hovered_edge, None);
    assert!(
        action6.is_none(),
        "Moving cursor within node card must NOT request redraw"
    );

    // 7. Cursor leaves canvas (CursorLeft)
    let action_left = canvas.update(
        &mut state,
        &Event::Mouse(mouse::Event::CursorLeft),
        bounds,
        mouse::Cursor::Unavailable,
    );

    assert_eq!(state.cursor_pos, None);
    assert_eq!(state.hovered_node, None);
    assert_eq!(state.hovered_edge, None);
    assert!(
        action_left.is_some(),
        "CursorLeft must request redraw to clear hover highlights"
    );
}

#[test]
fn challenge_overlay_layer_does_not_request_redraw() {
    let (graph, visible_nodes, node_positions, _edge_key) = make_test_graph();

    let canvas_overlay = GraphCanvas {
        graph: &graph,
        visible_nodes,
        node_positions: &node_positions,
        selected_edge: None,
        selected_node: None,
        pan: Vector::ZERO,
        zoom: 1.0,
        layer: CanvasLayer::Overlay,
        search_query: "",
    };

    let bounds = Rectangle::new(Point::ORIGIN, Size::new(800.0, 600.0));
    let mut state = CanvasInteraction::default();

    let pt = Point::new(417.5, 248.0);
    let event = Event::Mouse(mouse::Event::CursorMoved { position: pt });
    let action = canvas_overlay.update(&mut state, &event, bounds, mouse::Cursor::Available(pt));

    assert_eq!(state.cursor_pos, Some(pt));
    assert!(
        action.is_none(),
        "Overlay layer update must return None (Base layer handles redraw)"
    );
}

#[test]
fn challenge_worker_test_flaw_analysis() {
    // In the worker's original test `test_r2_canvas_interaction_tracks_cursor_and_requests_redraw`:
    // It constructed GraphCanvas with an empty PwGraph.
    // It manually set `state.hovered_edge = Some(EdgeKey { 1, 2 })`.
    // Then it triggered CursorMoved.
    // We demonstrate empirically that the worker's test resulted in action.is_some()
    // because hovered_edge became None (transition), NOT because it was maintained!

    let empty_graph = PwGraph::default();
    let empty_visible = HashSet::new();
    let empty_positions = BTreeMap::new();
    let canvas = GraphCanvas {
        graph: &empty_graph,
        visible_nodes: empty_visible,
        node_positions: &empty_positions,
        selected_edge: None,
        selected_node: None,
        pan: Vector::ZERO,
        zoom: 1.0,
        layer: CanvasLayer::Base,
        search_query: "",
    };

    let bounds = Rectangle::new(Point::ORIGIN, Size::new(800.0, 600.0));
    let mut state = CanvasInteraction {
        hovered_edge: Some(EdgeKey {
            output_node_serial: 1,
            input_node_serial: 2,
        }),
        ..Default::default()
    };

    let next_cursor_point = Point::new(155.0, 205.0);
    let next_event = Event::Mouse(mouse::Event::CursorMoved {
        position: next_cursor_point,
    });
    let action = canvas.update(
        &mut state,
        &next_event,
        bounds,
        mouse::Cursor::Available(next_cursor_point),
    );

    // Notice that hovered_edge became None!
    assert_eq!(
        state.hovered_edge, None,
        "Worker test had empty graph, so hit_test_edge returned None and cleared hovered_edge!"
    );
    // Action was Some only because prev_edge (Some) != state.hovered_edge (None)
    assert!(action.is_some());
}
