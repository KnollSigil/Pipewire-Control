#![allow(dead_code, unused_imports, unused_variables)]

#[path = "../src/model.rs"]
mod model;

#[path = "../src/canvas_view.rs"]
mod canvas_view;

use canvas_view::{
    compute_auto_layout_positions, find_sensible_free_spot, node_input_socket,
    node_intersects_link_curve, node_output_socket, COL_SPACING, NODE_HEIGHT, NODE_WIDTH,
    ROW_HEIGHT,
};
use iced::{Point, Rectangle, Size};
use model::{
    compute_signal_flow_columns, create_demo_graph, NodeCategory, PortDirection, PwGraph,
    PwLink, PwNode, PwPort,
};
use std::collections::{BTreeMap, HashSet};

fn build_test_node(
    serial: u64,
    name: &str,
    category: NodeCategory,
    in_ports: usize,
    out_ports: usize,
) -> PwNode {
    let mut input_ports = Vec::new();
    for i in 0..in_ports {
        input_ports.push(PwPort {
            id: (serial * 100 + i as u64) as u32,
            object_serial: serial * 1000 + i as u64,
            node_id: serial as u32,
            node_serial: serial,
            name: format!("input_{}", i + 1),
            direction: PortDirection::Input,
            channel: "FL".to_string(),
            is_monitor: false,
            props: BTreeMap::new(),
        });
    }
    let mut output_ports = Vec::new();
    for i in 0..out_ports {
        output_ports.push(PwPort {
            id: (serial * 200 + i as u64) as u32,
            object_serial: serial * 2000 + i as u64,
            node_id: serial as u32,
            node_serial: serial,
            name: format!("output_{}", i + 1),
            direction: PortDirection::Output,
            channel: "FL".to_string(),
            is_monitor: false,
            props: BTreeMap::new(),
        });
    }
    PwNode {
        id: serial as u32,
        object_serial: serial,
        name: name.to_string(),
        description: name.to_string(),
        app_name: None,
        media_name: None,
        media_class: "Audio/Sink".to_string(),
        category,
        input_ports,
        output_ports,
        state: "running".to_string(),
        volume: Some(1.0),
        muted: Some(false),
        channel_names: Vec::new(),
        channel_volumes: Vec::new(),
        props: BTreeMap::new(),
        params: Vec::new(),
    }
}

fn make_test_link(
    serial: u64,
    out_serial: u64,
    out_port: u64,
    in_serial: u64,
    in_port: u64,
) -> PwLink {
    PwLink {
        id: serial as u32,
        object_serial: serial,
        output_node_id: out_serial as u32,
        output_node_serial: out_serial,
        output_port_id: out_port as u32,
        output_port_serial: out_port,
        input_node_id: in_serial as u32,
        input_node_serial: in_serial,
        input_port_id: in_port as u32,
        input_port_serial: in_port,
        state: "active".to_string(),
        props: BTreeMap::new(),
    }
}

fn create_chain_synthetic_graph() -> (PwGraph, HashSet<u64>) {
    let mut graph = PwGraph::default();

    // Source (1) -> Filter (2) -> Sink (3)
    let n1 = build_test_node(1, "Synthetic Source", NodeCategory::AudioSource, 0, 1);
    graph.ports.insert(1000, n1.output_ports[0].clone());
    graph.nodes.insert(1, n1);

    let n2 = build_test_node(2, "Synthetic Filter", NodeCategory::AudioDuplex, 1, 1);
    graph.ports.insert(2000, n2.input_ports[0].clone());
    graph.ports.insert(2001, n2.output_ports[0].clone());
    graph.nodes.insert(2, n2);

    let n3 = build_test_node(3, "Synthetic Sink", NodeCategory::AudioSink, 1, 0);
    graph.ports.insert(3000, n3.input_ports[0].clone());
    graph.nodes.insert(3, n3);

    graph.links.insert(1001, make_test_link(1001, 1, 1000, 2, 2000));
    graph.links.insert(1002, make_test_link(1002, 2, 2001, 3, 3000));

    let visible = [1, 2, 3].into_iter().collect();
    (graph, visible)
}

fn create_unconnected_sink_synthetic_graph() -> (PwGraph, HashSet<u64>) {
    let mut graph = PwGraph::default();

    let n1 = build_test_node(1, "Synthetic Source", NodeCategory::AudioSource, 0, 0);
    graph.nodes.insert(1, n1);

    let n2 = build_test_node(2, "Synthetic Unconnected Sink", NodeCategory::AudioSink, 1, 0);
    graph.ports.insert(2000, n2.input_ports[0].clone());
    graph.nodes.insert(2, n2);

    let visible = [1, 2].into_iter().collect();
    (graph, visible)
}

fn create_diamond_synthetic_graph() -> (PwGraph, HashSet<u64>) {
    let mut graph = PwGraph::default();

    // Source 1 feeds Filter 2 and Filter 3. Both 2 and 3 feed Sink 4.
    let n1 = build_test_node(1, "Source 1", NodeCategory::AudioOutputStream, 0, 1);
    graph.ports.insert(1000, n1.output_ports[0].clone());
    graph.nodes.insert(1, n1);

    for id in [2, 3] {
        let n = build_test_node(id, &format!("Filter {id}"), NodeCategory::AudioDuplex, 1, 1);
        graph.ports.insert(id * 1000, n.input_ports[0].clone());
        graph.ports.insert(id * 1000 + 1, n.output_ports[0].clone());
        graph.nodes.insert(id, n);
    }

    let n4 = build_test_node(4, "Sink 4", NodeCategory::AudioSink, 2, 0);
    graph.ports.insert(4000, n4.input_ports[0].clone());
    graph.ports.insert(4001, n4.input_ports[1].clone());
    graph.nodes.insert(4, n4);

    // Links: 1->2, 1->3, 2->4, 3->4
    graph.links.insert(101, make_test_link(101, 1, 1000, 2, 2000));
    graph.links.insert(102, make_test_link(102, 1, 1000, 3, 3000));
    graph.links.insert(103, make_test_link(103, 2, 2001, 4, 4000));
    graph.links.insert(104, make_test_link(104, 3, 3001, 4, 4001));

    let visible = [1, 2, 3, 4].into_iter().collect();
    (graph, visible)
}

#[test]
fn test_r4_demo_graph_signal_flow_depth_columns() {
    let graph = create_demo_graph();
    let visible: HashSet<u64> = [10, 20, 30, 40, 50, 60].into_iter().collect();
    let cols = compute_signal_flow_columns(&graph, &visible);

    // Column 0: Sources (Mic 10, Firefox 20, Spotify 30)
    assert_eq!(cols.get(&10), Some(&0), "Mic must be in Column 0");
    assert_eq!(cols.get(&20), Some(&0), "Firefox must be in Column 0");
    assert_eq!(cols.get(&30), Some(&0), "Spotify must be in Column 0");

    // Column 1: Sinks fed by Col 0 (EasyEffects 40, Discord 60)
    assert_eq!(cols.get(&40), Some(&1), "EasyEffects must be in Column 1");
    assert_eq!(cols.get(&60), Some(&1), "Discord must be in Column 1");

    // Column 2: Sinks fed by Col 1 (Headphones 50)
    assert_eq!(cols.get(&50), Some(&2), "Headphones must be in Column 2");
}

#[test]
fn test_r4_synthetic_graph_1_chain_source_filter_sink() {
    let (graph, visible) = create_chain_synthetic_graph();
    let cols = compute_signal_flow_columns(&graph, &visible);

    assert_eq!(cols.get(&1), Some(&0), "Chain source must be in Column 0");
    assert_eq!(cols.get(&2), Some(&1), "Chain filter must be in Column 1");
    assert_eq!(cols.get(&3), Some(&2), "Chain sink must be in Column 2");
}

#[test]
fn test_r4_synthetic_graph_2_unconnected_sink() {
    let (graph, visible) = create_unconnected_sink_synthetic_graph();
    let cols = compute_signal_flow_columns(&graph, &visible);

    assert_eq!(cols.get(&1), Some(&0), "Source must be in Column 0");
    assert_eq!(
        cols.get(&2),
        Some(&1),
        "Unconnected sink must be placed in Column 1 alongside sinks fed by Column 0"
    );
}

#[test]
fn test_r4_diamond_graph_signal_flow_depth() {
    let (graph, visible) = create_diamond_synthetic_graph();
    let cols = compute_signal_flow_columns(&graph, &visible);

    assert_eq!(cols.get(&1), Some(&0), "Source must be in Column 0");
    assert_eq!(cols.get(&2), Some(&1), "Filter 2 must be in Column 1");
    assert_eq!(cols.get(&3), Some(&1), "Filter 3 must be in Column 1");
    assert_eq!(cols.get(&4), Some(&2), "Sink 4 must be in Column 2");
}

#[test]
fn test_r4_non_overlap_invariants_across_all_graphs() {
    let graphs = [
        create_demo_graph(),
        create_chain_synthetic_graph().0,
        create_unconnected_sink_synthetic_graph().0,
        create_diamond_synthetic_graph().0,
    ];

    for (idx, graph) in graphs.iter().enumerate() {
        let visible: HashSet<u64> = graph.nodes.keys().copied().collect();
        let positions = compute_auto_layout_positions(graph, &visible);

        let serials: Vec<u64> = positions.keys().copied().collect();
        for i in 0..serials.len() {
            for j in (i + 1)..serials.len() {
                let s_a = serials[i];
                let s_b = serials[j];
                let p_a = positions[&s_a];
                let p_b = positions[&s_b];

                let dx = (p_b.x - p_a.x).abs();
                let dy = (p_b.y - p_a.y).abs();

                assert!(
                    dx >= NODE_WIDTH + 12.0 || dy >= NODE_HEIGHT + 12.0,
                    "Graph {idx}: nodes {s_a} and {s_b} must not overlap: p_a={p_a:?}, p_b={p_b:?}, dx={dx}, dy={dy}"
                );
            }
        }
    }
}

#[test]
fn test_r4_link_curve_clearance_no_card_on_crossing_wires() {
    let graphs = [
        create_demo_graph(),
        create_chain_synthetic_graph().0,
        create_diamond_synthetic_graph().0,
    ];

    for (idx, graph) in graphs.iter().enumerate() {
        let visible: HashSet<u64> = graph.nodes.keys().copied().collect();
        let positions = compute_auto_layout_positions(graph, &visible);

        for edge in graph.collapsed_edges() {
            let u = edge.key.output_node_serial;
            let v = edge.key.input_node_serial;
            if !positions.contains_key(&u) || !positions.contains_key(&v) {
                continue;
            }

            let p0 = node_output_socket(positions[&u]);
            let p3 = node_input_socket(positions[&v]);

            for (&w, &pos_w) in &positions {
                if w == u || w == v {
                    continue; // Endpoints naturally touch their own links
                }

                let w_rect = Rectangle::new(pos_w, Size::new(NODE_WIDTH, NODE_HEIGHT));
                assert!(
                    !node_intersects_link_curve(w_rect, p0, p3),
                    "Graph {idx}: Node {w} at {pos_w:?} intersects link curve from {u} ({p0:?}) to {v} ({p3:?})"
                );
            }
        }
    }
}

#[test]
fn test_r4_find_sensible_free_spot_places_without_overlap_or_wire_collision() {
    let graph = create_demo_graph();
    let visible: HashSet<u64> = [10, 20, 30, 40, 50, 60].into_iter().collect();
    let positions = compute_auto_layout_positions(&graph, &visible);

    // 1. Add a new isolated node (Node 99: "New Player", AudioOutputStream -> should go to Col 0)
    let new_source = build_test_node(99, "New Player", NodeCategory::AudioOutputStream, 0, 1);

    let spot_source = find_sensible_free_spot(&graph, &positions, &new_source);
    // Spot must not overlap any existing node
    for (&s, &pos) in &positions {
        let dx = (pos.x - spot_source.x).abs();
        let dy = (pos.y - spot_source.y).abs();
        assert!(
            dx >= NODE_WIDTH + 12.0 || dy >= NODE_HEIGHT + 12.0,
            "Free spot {spot_source:?} must not overlap existing node {s} at {pos:?}"
        );
    }

    // Spot must not collide with any existing link curve
    let rect_source = Rectangle::new(spot_source, Size::new(NODE_WIDTH, NODE_HEIGHT));
    for edge in graph.collapsed_edges() {
        let u = edge.key.output_node_serial;
        let v = edge.key.input_node_serial;
        if positions.contains_key(&u) && positions.contains_key(&v) {
            let p0 = node_output_socket(positions[&u]);
            let p3 = node_input_socket(positions[&v]);
            assert!(
                !node_intersects_link_curve(rect_source, p0, p3),
                "Free spot {spot_source:?} must not collide with wire from {u} to {v}"
            );
        }
    }

    // 2. Add a new connected sink node fed by Headphones (Node 50 in Col 2) -> should go to Col 3
    let new_sink = build_test_node(100, "Downstream Recorder", NodeCategory::AudioSink, 1, 0);

    let mut graph_with_sink = graph.clone();
    graph_with_sink.nodes.insert(100, new_sink.clone());
    let link = make_test_link(9999, 50, 5000, 100, 1000);
    graph_with_sink.links.insert(9999, link);

    let spot_sink = find_sensible_free_spot(&graph_with_sink, &positions, &new_sink);
    // Headphones is at x=720 (col 2), so downstream node should be at x=1060 (col 3)
    assert!(spot_sink.x >= 1060.0, "Downstream node should be placed in col >= 3: got x={}", spot_sink.x);

    for (&s, &pos) in &positions {
        let dx = (pos.x - spot_sink.x).abs();
        let dy = (pos.y - spot_sink.y).abs();
        assert!(
            dx >= NODE_WIDTH + 12.0 || dy >= NODE_HEIGHT + 12.0,
            "Free spot for downstream sink {spot_sink:?} must not overlap existing node {s}"
        );
    }
}

#[test]
fn test_r4_position_stability_adding_node_preserves_existing_positions_exactly() {
    let graph = create_demo_graph();
    let visible: HashSet<u64> = [10, 20, 30, 40, 50, 60].into_iter().collect();
    let original_positions = compute_auto_layout_positions(&graph, &visible);

    // Simulate adding a node to existing positions map
    let mut updated_positions = original_positions.clone();
    let new_node = build_test_node(88, "Background Noise Reducer", NodeCategory::AudioDuplex, 1, 1);

    let spot = find_sensible_free_spot(&graph, &updated_positions, &new_node);
    updated_positions.insert(88, spot);

    // All original nodes must be bit-for-bit identical
    for (&serial, &orig_pos) in &original_positions {
        let new_pos = updated_positions[&serial];
        assert_eq!(
            new_pos, orig_pos,
            "Node {serial} position must remain strictly identical when a new node is added!"
        );
    }

    // New node position must not overlap any original node
    for (&serial, &orig_pos) in &original_positions {
        let dx = (orig_pos.x - spot.x).abs();
        let dy = (orig_pos.y - spot.y).abs();
        assert!(
            dx >= NODE_WIDTH + 12.0 || dy >= NODE_HEIGHT + 12.0,
            "New node spot {spot:?} overlaps with pre-existing node {serial} at {orig_pos:?}"
        );
    }
}

#[test]
fn test_r4_cycles_and_feedback_loops_handled_gracefully() {
    let mut graph = PwGraph::default();

    // Node A (1) -> Node B (2) -> Node C (3) -> Node A (1) (cycle!)
    for id in [1, 2, 3] {
        let n = build_test_node(id, &format!("Cycle Node {id}"), NodeCategory::AudioDuplex, 1, 1);
        graph.ports.insert(id * 1000, n.input_ports[0].clone());
        graph.ports.insert(id * 1000 + 1, n.output_ports[0].clone());
        graph.nodes.insert(id, n);
    }

    graph.links.insert(101, make_test_link(101, 1, 1001, 2, 2000));
    graph.links.insert(102, make_test_link(102, 2, 2001, 3, 3000));
    graph.links.insert(103, make_test_link(103, 3, 3001, 1, 1000));

    let visible: HashSet<u64> = [1, 2, 3].into_iter().collect();

    // Must not loop infinitely or panic
    let cols = compute_signal_flow_columns(&graph, &visible);
    assert_eq!(cols.len(), 3);

    let positions = compute_auto_layout_positions(&graph, &visible);
    assert_eq!(positions.len(), 3);

    // Assert no overlap
    for i in 1..=3 {
        for j in (i + 1)..=3 {
            let p_a = positions[&i];
            let p_b = positions[&j];
            let dx = (p_a.x - p_b.x).abs();
            let dy = (p_a.y - p_b.y).abs();
            assert!(
                dx >= NODE_WIDTH + 12.0 || dy >= NODE_HEIGHT + 12.0,
                "Nodes {i} and {j} in cycle graph must not overlap"
            );
        }
    }
}
