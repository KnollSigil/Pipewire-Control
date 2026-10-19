#![allow(dead_code, unused_imports, unused_variables)]

#[path = "../src/model.rs"]
mod model;

#[path = "../src/canvas_view.rs"]
mod canvas_view;

use canvas_view::{
    compute_auto_layout_positions, find_sensible_free_spot, node_input_socket,
    node_intersects_link_curve, node_output_socket, COL_SPACING, INITIAL_X, INITIAL_Y,
    NODE_HEIGHT, NODE_WIDTH, ROW_HEIGHT,
};
use iced::{Point, Rectangle, Size};
use model::{
    compute_signal_flow_columns, NodeCategory, PortDirection, PwGraph, PwLink, PwNode, PwPort,
};
use std::collections::{BTreeMap, BTreeSet, HashSet};

fn build_node(
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
            params: Vec::new(),
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
            params: Vec::new(),
        });
    }
    PwNode {
        id: serial as u32,
        object_serial: serial,
        name: name.to_string(),
        description: name.to_string(),
        app_name: None,
        media_name: None,
        media_class: match category {
            NodeCategory::AudioSource => "Audio/Source".to_string(),
            NodeCategory::AudioSink => "Audio/Sink".to_string(),
            NodeCategory::AudioDuplex => "Audio/Duplex".to_string(),
            NodeCategory::AudioInputStream => "Stream/Input/Audio".to_string(),
            NodeCategory::AudioOutputStream => "Stream/Output/Audio".to_string(),
            NodeCategory::Midi => "Midi/Bridge".to_string(),
            NodeCategory::Video => "Video/Source".to_string(),
            NodeCategory::Internal => "PipeWire:Internal".to_string(),
        },
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
        device_id: None,
        device_serial: None,
    }
}

fn make_link(
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

/// Comprehensive layout invariant assertion:
/// 1. Node card non-overlap (minimum horizontal or vertical margin).
/// 2. Sampled bezier curve clearance (no node rectangle intersects a curve it is not an endpoint of).
/// 3. In DAG mode: topological strictly increasing columns along forward edges.
/// 4. Compact columns: column indices are dense 0..=max_col.
fn assert_stress_graph_invariants(
    graph: &PwGraph,
    label: &str,
    is_dag: bool,
) -> (BTreeMap<u64, usize>, BTreeMap<u64, Point>) {
    let visible: HashSet<u64> = graph.nodes.keys().copied().collect();
    let cols = compute_signal_flow_columns(graph, &visible);
    let positions = compute_auto_layout_positions(graph, &visible);

    // 1. Column and Position completeness
    for &serial in &visible {
        assert!(cols.contains_key(&serial), "{label}: node {serial} missing column");
        assert!(positions.contains_key(&serial), "{label}: node {serial} missing position");
    }

    // 2. Dense Column Compaction Invariant:
    if !cols.is_empty() {
        let max_col = cols.values().copied().max().unwrap_or(0);
        let assigned_cols: BTreeSet<usize> = cols.values().copied().collect();
        let expected_cols: BTreeSet<usize> = (0..=max_col).collect();
        assert_eq!(
            assigned_cols, expected_cols,
            "{label}: Columns are not dense! Assigned: {assigned_cols:?}, Expected: {expected_cols:?}"
        );
    }

    // 3. Forward DAG signal-flow depth check
    if is_dag {
        for edge in graph.collapsed_edges() {
            let u = edge.key.output_node_serial;
            let v = edge.key.input_node_serial;
            if u == v {
                continue;
            }
            if let (Some(&col_u), Some(&col_v)) = (cols.get(&u), cols.get(&v)) {
                assert!(
                    col_v > col_u,
                    "{label}: Signal-flow depth must strictly increase along forward link {u} -> {v}: col_u={col_u}, col_v={col_v}"
                );
            }
        }
    }

    // 4. Node Card Non-overlap check
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
                "{label}: Node overlap between {s_a} and {s_b}: p_a={p_a:?}, p_b={p_b:?}, dx={dx}, dy={dy}"
            );
        }
    }

    // 5. Sampled Bezier Link Clearance check
    for edge in graph.collapsed_edges() {
        let u = edge.key.output_node_serial;
        let v = edge.key.input_node_serial;
        if u == v {
            continue;
        }
        let (Some(&pos_u), Some(&pos_v)) = (positions.get(&u), positions.get(&v)) else {
            continue;
        };
        let p0 = node_output_socket(pos_u);
        let p3 = node_input_socket(pos_v);

        for (&w, &pos_w) in &positions {
            if w == u || w == v {
                continue;
            }
            let w_rect = Rectangle::new(pos_w, Size::new(NODE_WIDTH, NODE_HEIGHT));
            assert!(
                !node_intersects_link_curve(w_rect, p0, p3),
                "{label}: Node {w} at {pos_w:?} intersects link curve from {u} to {v} (p0={p0:?}, p3={p3:?})"
            );
        }
    }

    (cols, positions)
}

// ============================================================================
// EMPIRICAL DIAGNOSTIC TESTS FOR ITERATION 1 FAILURE MODES
// ============================================================================

#[test]
fn test_empirical_telemetry_flaw_1_figure_eight_cycle() {
    let mut graph = PwGraph::default();
    // Cycle 1: 1 -> 2 -> 3 -> 1
    // Cycle 2: 3 -> 4 -> 5 -> 3
    for id in 1..=5 {
        let n = build_node(id, &format!("Node {id}"), NodeCategory::AudioDuplex, 2, 2);
        for p in &n.input_ports {
            graph.ports.insert(p.object_serial, p.clone());
        }
        for p in &n.output_ports {
            graph.ports.insert(p.object_serial, p.clone());
        }
        graph.nodes.insert(id, n);
    }

    graph.links.insert(101, make_link(101, 1, 2000, 2, 2 * 1000));
    graph.links.insert(102, make_link(102, 2, 2 * 2000, 3, 3 * 1000));
    graph.links.insert(103, make_link(103, 3, 3 * 2000, 1, 1000));
    graph.links.insert(104, make_link(104, 3, 3 * 2000 + 1, 4, 4 * 1000));
    graph.links.insert(105, make_link(105, 4, 4 * 2000, 5, 5 * 1000));
    graph.links.insert(106, make_link(106, 5, 5 * 2000, 3, 3 * 1000 + 1));

    let (cols, positions) = assert_stress_graph_invariants(&graph, "Telemetry Figure-8", false);

    println!(">>> TELEMETRY FLAW 1: Figure-8 Columns: {cols:?}");
    println!(">>> TELEMETRY FLAW 1: Figure-8 Positions: {positions:?}");

    // Detailed verification of flaw 1 resolution:
    // In Iteration 1, all nodes were in Column 0 ($x=40.0$), causing Node 3 at (40.0, 6670.0)
    // to intersect link 1 -> 2.
    // In Iteration 2, nodes must be in distinct columns.
    assert_eq!(*cols.get(&1).unwrap(), 0, "Cycle root 1 at Col 0");
    assert_eq!(*cols.get(&2).unwrap(), 1, "Node 2 at Col 1");
    assert_eq!(*cols.get(&3).unwrap(), 2, "Intersection Node 3 at Col 2");
    assert_eq!(*cols.get(&4).unwrap(), 3, "Node 4 at Col 3");
    assert_eq!(*cols.get(&5).unwrap(), 4, "Node 5 at Col 4");

    // Check positions X coordinates reflect column assignments:
    for (id, &col) in &cols {
        let expected_x = INITIAL_X + col as f32 * COL_SPACING;
        assert_eq!(
            positions[id].x, expected_x,
            "Node {id} x position mismatch for column {col}"
        );
    }

    // Verify link 1 -> 2 curve endpoints and horizontal clearance with Node 3:
    let p0 = node_output_socket(positions[&1]);
    let p3 = node_input_socket(positions[&2]);
    let pos_3 = positions[&3];
    assert!(p0.x <= 275.0, "Socket 1 p0.x={}", p0.x);
    assert!(p3.x >= 380.0, "Socket 2 p3.x={}", p3.x);
    assert!(
        pos_3.x >= 720.0,
        "Node 3 x={} is far away horizontally from link corridor [40, 380]",
        pos_3.x
    );
}

#[test]
fn test_empirical_telemetry_flaw_2_deep_chain_skip_connections() {
    let mut graph = PwGraph::default();
    let num_stages = 10;

    for i in 0..num_stages {
        let node_id = i as u64 + 1;
        let cat = if i == 0 {
            NodeCategory::AudioSource
        } else if i == num_stages - 1 {
            NodeCategory::AudioSink
        } else {
            NodeCategory::AudioDuplex
        };
        let node = build_node(node_id, &format!("SkipStage {i}"), cat, 5, 5);
        for p in &node.input_ports {
            graph.ports.insert(p.object_serial, p.clone());
        }
        for p in &node.output_ports {
            graph.ports.insert(p.object_serial, p.clone());
        }
        graph.nodes.insert(node_id, node);
    }

    let mut link_id = 10000;
    for i in 0..(num_stages - 1) {
        let u = i as u64 + 1;
        let v = (i + 1) as u64 + 1;
        graph.links.insert(link_id, make_link(link_id, u, u * 2000, v, v * 1000));
        link_id += 1;
    }

    let skip_pairs = [(1, 4), (2, 6), (4, 8), (5, 10)];
    for (idx, &(u, v)) in skip_pairs.iter().enumerate() {
        let out_port = u * 2000 + (idx as u64 + 1);
        let in_port = v * 1000 + (idx as u64 + 1);
        graph.links.insert(link_id, make_link(link_id, u, out_port, v, in_port));
        link_id += 1;
    }

    let (cols, positions) = assert_stress_graph_invariants(&graph, "Telemetry Skip Connections", true);

    println!(">>> TELEMETRY FLAW 2: Skip Chain Columns: {cols:?}");
    println!(">>> TELEMETRY FLAW 2: Skip Chain Positions: {positions:?}");

    // In Iteration 1, vertical positions exploded to > 6670.0 px and still failed.
    // In Iteration 2, verify bounded vertical envelope and zero collisions.
    let max_y = positions.values().map(|p| p.y).fold(0.0f32, f32::max);
    let min_y = positions.values().map(|p| p.y).fold(f32::MAX, f32::min);
    let total_height = max_y - min_y;

    println!(">>> TELEMETRY FLAW 2: max_y = {max_y}, total_height = {total_height}");
    assert!(
        max_y <= 1000.0,
        "Max Y must be comfortably bounded (got {max_y})"
    );
    assert!(
        total_height <= 1000.0,
        "Total vertical spread must not explode (got {total_height})"
    );

    // Verify all intermediate nodes along skip link (1, 4): nodes 2 and 3
    let p0 = node_output_socket(positions[&1]);
    let p3 = node_input_socket(positions[&4]);
    for mid in [2, 3] {
        let pos_mid = positions[&mid];
        let mid_rect = Rectangle::new(pos_mid, Size::new(NODE_WIDTH, NODE_HEIGHT));
        assert!(
            !node_intersects_link_curve(mid_rect, p0, p3),
            "Intermediate node {mid} intersects skip link 1 -> 4"
        );
        // Node mid must be below the wire
        assert!(
            pos_mid.y >= f32::max(positions[&1].y, positions[&4].y) + 120.0,
            "Intermediate node {mid} y={} should be well below endpoints y1={}, y4={}",
            pos_mid.y, positions[&1].y, positions[&4].y
        );
    }
}

#[test]
fn test_empirical_telemetry_flaw_3_reachable_feedback_cycle() {
    let mut graph = PwGraph::default();

    // 1 (Source) -> 2 (DSP A) <-> 3 (DSP B) -> 4 (Sink)
    let n1 = build_node(1, "Source", NodeCategory::AudioSource, 0, 1);
    graph.ports.insert(1000, n1.output_ports[0].clone());
    graph.nodes.insert(1, n1);

    let n2 = build_node(2, "DSP Filter A", NodeCategory::AudioDuplex, 2, 1);
    graph.ports.insert(2000, n2.input_ports[0].clone());
    graph.ports.insert(2001, n2.input_ports[1].clone());
    graph.ports.insert(2002, n2.output_ports[0].clone());
    graph.nodes.insert(2, n2);

    let n3 = build_node(3, "DSP Filter B", NodeCategory::AudioDuplex, 1, 2);
    graph.ports.insert(3000, n3.input_ports[0].clone());
    graph.ports.insert(3001, n3.output_ports[0].clone());
    graph.ports.insert(3002, n3.output_ports[1].clone());
    graph.nodes.insert(3, n3);

    let n4 = build_node(4, "Output Sink", NodeCategory::AudioSink, 1, 0);
    graph.ports.insert(4000, n4.input_ports[0].clone());
    graph.nodes.insert(4, n4);

    graph.links.insert(101, make_link(101, 1, 1000, 2, 2000));
    graph.links.insert(102, make_link(102, 2, 2002, 3, 3000));
    graph.links.insert(103, make_link(103, 3, 3001, 2, 2001)); // feedback
    graph.links.insert(104, make_link(104, 3, 3002, 4, 4000));

    let (cols, positions) = assert_stress_graph_invariants(&graph, "Telemetry Feedback Cycle", false);

    println!(">>> TELEMETRY FLAW 3: Columns: {cols:?}");
    println!(">>> TELEMETRY FLAW 3: Positions: {positions:?}");

    // In Iteration 1: {1: 0, 2: 11, 3: 12, 4: 13} (13 columns, void of 3400px)
    // In Iteration 2:
    assert_eq!(*cols.get(&1).unwrap(), 0);
    assert_eq!(*cols.get(&2).unwrap(), 1);
    assert_eq!(*cols.get(&3).unwrap(), 2);
    assert_eq!(*cols.get(&4).unwrap(), 3);

    // Bounded max column: exactly 3
    let max_col = cols.values().copied().max().unwrap_or(0);
    assert_eq!(max_col, 3, "Max column must be exactly 3, no explosion");

    // X span check: no empty 3400px void
    let x_span = positions[&4].x - positions[&1].x;
    assert_eq!(x_span, 3.0 * COL_SPACING, "Canvas span must be exactly 3 column steps (1020px)");
}

// ============================================================================
// ADVERSARIAL PATHOLOGICAL STRESS TESTS
// ============================================================================

/// ADVERSARIAL STRESS 1: Complete Directed Graph K_5 (Dense Feedback Mesh)
/// All 5 nodes connect to every other node in both directions (20 directed links).
/// Maximum cycle density: every pair forms a 2-cycle, 3-cycle, 4-cycle, and 5-cycle.
#[test]
fn test_pathological_1_complete_directed_mesh_k5() {
    let mut graph = PwGraph::default();
    let n = 5;

    for i in 1..=n {
        let node = build_node(i, &format!("K5_Node_{i}"), NodeCategory::AudioDuplex, n as usize, n as usize);
        for p in &node.input_ports {
            graph.ports.insert(p.object_serial, p.clone());
        }
        for p in &node.output_ports {
            graph.ports.insert(p.object_serial, p.clone());
        }
        graph.nodes.insert(i, node);
    }

    let mut link_id = 50000;
    for u in 1..=n {
        for v in 1..=n {
            if u != v {
                let out_p = u * 2000 + v;
                let in_p = v * 1000 + u;
                graph.links.insert(link_id, make_link(link_id, u, out_p, v, in_p));
                link_id += 1;
            }
        }
    }

    let (cols, positions) = assert_stress_graph_invariants(&graph, "K5 Complete Mesh", false);

    println!(">>> STRESS 1 (K5): cols = {cols:?}");
    println!(">>> STRESS 1 (K5): positions = {positions:?}");

    // Max column must be strictly bounded <= n-1 = 4
    let max_col = cols.values().copied().max().unwrap_or(0);
    assert!(max_col <= (n as usize - 1), "Max column for K_5 must be <= 4 (got {max_col})");

    // Column compaction check: exactly 0..=max_col
    let assigned: BTreeSet<usize> = cols.values().copied().collect();
    let expected: BTreeSet<usize> = (0..=max_col).collect();
    assert_eq!(assigned, expected);
}

/// ADVERSARIAL STRESS 2: Multi-Tiered Dense Cross-Skip DAG
/// 8 nodes with multiple criss-crossing bypass links:
/// Chain 1->2->3->4->5->6->7->8
/// Skips: 1->3, 1->5, 1->7, 2->4, 2->6, 2->8, 3->6, 4->7, 5->8
/// Multiple nodes are bypassed by multiple simultaneous curves across different row/column slots.
#[test]
fn test_pathological_2_dense_cross_skip_dag() {
    let mut graph = PwGraph::default();
    let n = 8;

    for i in 1..=n {
        let cat = if i == 1 {
            NodeCategory::AudioSource
        } else if i == n {
            NodeCategory::AudioSink
        } else {
            NodeCategory::AudioDuplex
        };
        let node = build_node(i, &format!("CrossSkip_{i}"), cat, 8, 8);
        for p in &node.input_ports {
            graph.ports.insert(p.object_serial, p.clone());
        }
        for p in &node.output_ports {
            graph.ports.insert(p.object_serial, p.clone());
        }
        graph.nodes.insert(i, node);
    }

    let mut link_id = 60000;
    // Sequential backbone
    for i in 1..n {
        graph.links.insert(link_id, make_link(link_id, i, i * 2000, i + 1, (i + 1) * 1000));
        link_id += 1;
    }

    // Heavy cross skips
    let skips = [(1, 3), (1, 5), (1, 7), (2, 4), (2, 6), (2, 8), (3, 6), (4, 7), (5, 8)];
    for (idx, &(u, v)) in skips.iter().enumerate() {
        let out_p = u * 2000 + (idx as u64 + 1);
        let in_p = v * 1000 + (idx as u64 + 1);
        graph.links.insert(link_id, make_link(link_id, u, out_p, v, in_p));
        link_id += 1;
    }

    let (cols, positions) = assert_stress_graph_invariants(&graph, "Dense Cross-Skip DAG", true);

    println!(">>> STRESS 2 (Cross-Skip): cols = {cols:?}");
    println!(">>> STRESS 2 (Cross-Skip): positions = {positions:?}");

    // Invariant: heights must converge reasonably and not oscillate or explode
    let max_y = positions.values().map(|p| p.y).fold(0.0f32, f32::max);
    assert!(max_y <= 3000.0, "Max Y in cross-skip DAG must be bounded <= 3000 (got {max_y})");
}

/// ADVERSARIAL STRESS 3: Double Nested Rings with Inter-Ring Cross Links & Back-Feedback
/// Ring A: 10 -> 11 -> 12 -> 10
/// Ring B: 20 -> 21 -> 22 -> 20
/// Cross-connections: 11 -> 20, 21 -> 12, and long back-feedback 22 -> 10.
/// Source 1 feeds into 10; Sink 30 fed by 22.
#[test]
fn test_pathological_3_double_nested_rings_with_cross_feedback() {
    let mut graph = PwGraph::default();

    // Source 1
    let s = build_node(1, "MasterSource", NodeCategory::AudioSource, 0, 1);
    graph.ports.insert(1000, s.output_ports[0].clone());
    graph.nodes.insert(1, s);

    // Ring A: 10, 11, 12
    for id in [10, 11, 12] {
        let n = build_node(id, &format!("RingA_{id}"), NodeCategory::AudioDuplex, 3, 3);
        for p in &n.input_ports { graph.ports.insert(p.object_serial, p.clone()); }
        for p in &n.output_ports { graph.ports.insert(p.object_serial, p.clone()); }
        graph.nodes.insert(id, n);
    }

    // Ring B: 20, 21, 22
    for id in [20, 21, 22] {
        let n = build_node(id, &format!("RingB_{id}"), NodeCategory::AudioDuplex, 3, 3);
        for p in &n.input_ports { graph.ports.insert(p.object_serial, p.clone()); }
        for p in &n.output_ports { graph.ports.insert(p.object_serial, p.clone()); }
        graph.nodes.insert(id, n);
    }

    // Sink 30
    let sink = build_node(30, "MasterSink", NodeCategory::AudioSink, 1, 0);
    graph.ports.insert(30000, sink.input_ports[0].clone());
    graph.nodes.insert(30, sink);

    let links = [
        // Source to Ring A
        (1, 1000, 10, 10000),
        // Ring A
        (10, 10000, 11, 11000),
        (11, 11000, 12, 12000),
        (12, 12000, 10, 10001),
        // Ring B
        (20, 20000, 21, 21000),
        (21, 21000, 22, 22000),
        (22, 22000, 20, 20001),
        // Cross links
        (11, 11001, 20, 20002),
        (21, 21001, 12, 12001),
        (22, 22001, 10, 10002),
        // Ring B to Sink
        (22, 22002, 30, 30000),
    ];

    for (idx, &(u, op, v, ip)) in links.iter().enumerate() {
        let lid = 70000 + idx as u64;
        graph.links.insert(lid, make_link(lid, u, op, v, ip));
    }

    let (cols, positions) = assert_stress_graph_invariants(&graph, "Double Nested Rings", false);

    println!(">>> STRESS 3 (Double Rings): cols = {cols:?}");
    println!(">>> STRESS 3 (Double Rings): positions = {positions:?}");

    // Source 1 must be at Column 0
    assert_eq!(cols[&1], 0);
    // Sink 30 must be downstream of Source 1
    assert!(cols[&30] > cols[&1]);
    // Max col must be bounded <= 8
    let max_col = cols.values().copied().max().unwrap_or(0);
    assert!(max_col <= 8, "Max column must be bounded (got {max_col})");
}

/// ADVERSARIAL STRESS 4: Heterogeneous Graph with SCC, Multiple Sources/Sinks, Unconnected Sinks & Isolated Nodes
/// Mixed node categories:
/// - 2 AudioSources (1, 2)
/// - 4 Duplex SCC nodes (10, 11, 12, 13) with full circular feedback
/// - 2 AudioSinks connected to SCC (20, 21)
/// - 2 Unconnected AudioSinks (30, 31)
/// - 2 Isolated Duplex nodes (40, 41)
#[test]
fn test_pathological_4_heterogeneous_graph_full_zoo() {
    let mut graph = PwGraph::default();

    // Sources
    for id in [1, 2] {
        let s = build_node(id, &format!("Source_{id}"), NodeCategory::AudioSource, 0, 2);
        for p in &s.output_ports { graph.ports.insert(p.object_serial, p.clone()); }
        graph.nodes.insert(id, s);
    }

    // SCC Duplex
    for id in [10, 11, 12, 13] {
        let n = build_node(id, &format!("SCC_{id}"), NodeCategory::AudioDuplex, 3, 3);
        for p in &n.input_ports { graph.ports.insert(p.object_serial, p.clone()); }
        for p in &n.output_ports { graph.ports.insert(p.object_serial, p.clone()); }
        graph.nodes.insert(id, n);
    }

    // Connected Sinks
    for id in [20, 21] {
        let n = build_node(id, &format!("ConnSink_{id}"), NodeCategory::AudioSink, 2, 0);
        for p in &n.input_ports { graph.ports.insert(p.object_serial, p.clone()); }
        graph.nodes.insert(id, n);
    }

    // Unconnected Sinks
    for id in [30, 31] {
        let n = build_node(id, &format!("UnconnSink_{id}"), NodeCategory::AudioSink, 2, 0);
        for p in &n.input_ports { graph.ports.insert(p.object_serial, p.clone()); }
        graph.nodes.insert(id, n);
    }

    // Isolated Duplex
    for id in [40, 41] {
        let n = build_node(id, &format!("Isolated_{id}"), NodeCategory::AudioDuplex, 2, 2);
        for p in &n.input_ports { graph.ports.insert(p.object_serial, p.clone()); }
        for p in &n.output_ports { graph.ports.insert(p.object_serial, p.clone()); }
        graph.nodes.insert(id, n);
    }

    let links = [
        // Sources to SCC
        (1, 2000, 10, 10 * 1000),
        (2, 2 * 2000, 11, 11 * 1000),
        // SCC cycle: 10 -> 11 -> 12 -> 13 -> 10
        (10, 10 * 2000, 11, 11 * 1000 + 1),
        (11, 11 * 2000, 12, 12 * 1000),
        (12, 12 * 2000, 13, 13 * 1000),
        (13, 13 * 2000, 10, 10 * 1000 + 1),
        // SCC to Sinks
        (12, 12 * 2000 + 1, 20, 20 * 1000),
        (13, 13 * 2000 + 1, 21, 21 * 1000),
    ];

    for (idx, &(u, op, v, ip)) in links.iter().enumerate() {
        let lid = 80000 + idx as u64;
        graph.links.insert(lid, make_link(lid, u, op, v, ip));
    }

    let (cols, positions) = assert_stress_graph_invariants(&graph, "Full Zoo Heterogeneous", false);

    println!(">>> STRESS 4 (Full Zoo): cols = {cols:?}");

    // Sources must be at Column 0
    assert_eq!(cols[&1], 0);
    assert_eq!(cols[&2], 0);

    // Unconnected sinks must be placed in Column 1 per R4 contract:
    // "sinks fed by column 0 plus sinks with no connected source"
    assert_eq!(cols[&30], 1, "Unconnected sink 30 should be in Column 1");
    assert_eq!(cols[&31], 1, "Unconnected sink 31 should be in Column 1");

    // Connected sinks (20, 21) must be strictly downstream of sources
    assert!(cols[&20] > cols[&1]);
    assert!(cols[&21] > cols[&2]);
}

/// ADVERSARIAL STRESS 5: Self-Loop and Multi-Edge Graph
/// Node 1 has a self-loop (1 -> 1).
/// Nodes 1 and 2 have 4 parallel links.
/// Nodes 2 and 1 have 2 feedback links.
#[test]
fn test_pathological_5_self_loops_and_parallel_edges() {
    let mut graph = PwGraph::default();

    let n1 = build_node(1, "SelfLoopNode", NodeCategory::AudioDuplex, 5, 5);
    for p in &n1.input_ports { graph.ports.insert(p.object_serial, p.clone()); }
    for p in &n1.output_ports { graph.ports.insert(p.object_serial, p.clone()); }
    graph.nodes.insert(1, n1);

    let n2 = build_node(2, "PartnerNode", NodeCategory::AudioDuplex, 5, 5);
    for p in &n2.input_ports { graph.ports.insert(p.object_serial, p.clone()); }
    for p in &n2.output_ports { graph.ports.insert(p.object_serial, p.clone()); }
    graph.nodes.insert(2, n2);

    // Self-loop on node 1
    graph.links.insert(90001, make_link(90001, 1, 2000, 1, 1000));

    // Parallel forward links 1 -> 2
    for i in 1..=3 {
        graph.links.insert(90001 + i, make_link(90001 + i, 1, 2000 + i, 2, 2 * 1000 + i));
    }

    // Parallel feedback links 2 -> 1
    for i in 1..=2 {
        graph.links.insert(90010 + i, make_link(90010 + i, 2, 2 * 2000 + i, 1, 1000 + i));
    }

    let (cols, positions) = assert_stress_graph_invariants(&graph, "Self-Loops & Multi-Edges", false);

    println!(">>> STRESS 5 (Self-Loops): cols = {cols:?}");
    println!(">>> STRESS 5 (Self-Loops): positions = {positions:?}");

    assert_eq!(cols.len(), 2);
    assert_ne!(cols[&1], cols[&2]);
}

/// ADVERSARIAL STRESS 6: Long Deep Chain (25 stages) with Interleaved Feedback and Skips
/// 25 nodes, feedback loops from 10 -> 5, 20 -> 15, 24 -> 1, and skips 1 -> 10, 10 -> 20.
/// Verifies relaxation |V|+2 limit and fast clearance convergence on large scale graph.
#[test]
fn test_pathological_6_long_chain_with_feedback_and_skips() {
    let mut graph = PwGraph::default();
    let num_stages = 25;

    for i in 1..=num_stages {
        let cat = if i == 1 {
            NodeCategory::AudioSource
        } else if i == num_stages {
            NodeCategory::AudioSink
        } else {
            NodeCategory::AudioDuplex
        };
        let node = build_node(i, &format!("Stage_{i}"), cat, 4, 4);
        for p in &node.input_ports { graph.ports.insert(p.object_serial, p.clone()); }
        for p in &node.output_ports { graph.ports.insert(p.object_serial, p.clone()); }
        graph.nodes.insert(i, node);
    }

    let mut link_id = 95000;
    // Sequential backbone
    for i in 1..num_stages {
        graph.links.insert(link_id, make_link(link_id, i, i * 2000, i + 1, (i + 1) * 1000));
        link_id += 1;
    }

    // Forward skips
    graph.links.insert(link_id, make_link(link_id, 1, 2000 + 1, 10, 10 * 1000 + 1)); link_id += 1;
    graph.links.insert(link_id, make_link(link_id, 10, 10 * 2000 + 1, 20, 20 * 1000 + 1)); link_id += 1;

    // Backward feedback loops
    graph.links.insert(link_id, make_link(link_id, 10, 10 * 2000 + 2, 5, 5 * 1000 + 2)); link_id += 1;
    graph.links.insert(link_id, make_link(link_id, 20, 20 * 2000 + 2, 15, 15 * 1000 + 2)); link_id += 1;
    graph.links.insert(link_id, make_link(link_id, 24, 24 * 2000 + 2, 1, 1000 + 2));

    let (cols, positions) = assert_stress_graph_invariants(&graph, "25-Stage Chain with Skips & Feedback", false);

    println!(">>> STRESS 6 (25 stages): max_col = {}", cols.values().max().unwrap());

    // Source 1 is at Col 0
    assert_eq!(cols[&1], 0);

    // Max column is bounded <= 25
    let max_col = *cols.values().max().unwrap();
    assert!(max_col <= 25, "Max col should be <= 25 (got {max_col})");

    // All columns dense
    let assigned: BTreeSet<usize> = cols.values().copied().collect();
    let expected: BTreeSet<usize> = (0..=max_col).collect();
    assert_eq!(assigned, expected);
}

/// Telemetry function simulating the exact clearance loop of compute_auto_layout_positions
/// to count the exact number of passes required to converge to zero collisions.
fn count_clearance_passes(graph: &PwGraph, visible_nodes: &HashSet<u64>) -> (usize, BTreeMap<u64, Point>) {
    let columns_map = model::compute_signal_flow_columns(graph, visible_nodes);
    let mut col_groups: BTreeMap<usize, Vec<u64>> = BTreeMap::new();
    for &serial in visible_nodes {
        if graph.nodes.contains_key(&serial) {
            let col = columns_map.get(&serial).copied().unwrap_or(0);
            col_groups.entry(col).or_default().push(serial);
        }
    }

    let mut positions: BTreeMap<u64, Point> = BTreeMap::new();
    for nodes in col_groups.values_mut() {
        nodes.sort();
        for (row_idx, &serial) in nodes.iter().enumerate() {
            let col = columns_map.get(&serial).copied().unwrap_or(0);
            positions.insert(
                serial,
                Point::new(
                    INITIAL_X + col as f32 * COL_SPACING,
                    INITIAL_Y + row_idx as f32 * ROW_HEIGHT,
                ),
            );
        }
    }

    let collapsed_edges = graph.collapsed_edges();
    let mut sorted_edges = collapsed_edges;
    sorted_edges.sort_by(|a, b| {
        let u_a = a.key.output_node_serial;
        let v_a = a.key.input_node_serial;
        let u_b = b.key.output_node_serial;
        let v_b = b.key.input_node_serial;

        let col_u_a = columns_map.get(&u_a).copied().unwrap_or(0) as isize;
        let col_v_a = columns_map.get(&v_a).copied().unwrap_or(0) as isize;
        let span_a = (col_v_a - col_u_a).abs();

        let col_u_b = columns_map.get(&u_b).copied().unwrap_or(0) as isize;
        let col_v_b = columns_map.get(&v_b).copied().unwrap_or(0) as isize;
        let span_b = (col_v_b - col_u_b).abs();

        span_b
            .cmp(&span_a)
            .then_with(|| u_a.cmp(&u_b))
            .then_with(|| v_a.cmp(&v_b))
    });

    let mut passes = 0;
    for pass in 0..50 {
        passes = pass + 1;
        let mut any_shift = false;

        for edge in &sorted_edges {
            let u = edge.key.output_node_serial;
            let v = edge.key.input_node_serial;
            let (Some(&pos_u), Some(&pos_v)) = (positions.get(&u), positions.get(&v)) else {
                continue;
            };
            let p0 = node_output_socket(pos_u);
            let p3 = node_input_socket(pos_v);

            for (&w_serial, pos_w) in positions.iter_mut() {
                if w_serial == u || w_serial == v {
                    continue;
                }
                let w_rect = Rectangle::new(*pos_w, Size::new(NODE_WIDTH, NODE_HEIGHT));
                if node_intersects_link_curve(w_rect, p0, p3) {
                    let target_y = f32::max(
                        pos_w.y + ROW_HEIGHT,
                        f32::max(pos_u.y, pos_v.y) + ROW_HEIGHT,
                    );
                    pos_w.y = target_y;
                    any_shift = true;
                }
            }
        }

        for nodes in col_groups.values_mut() {
            nodes.sort_by(|&a, &b| {
                let ya = positions.get(&a).map_or(0.0, |p| p.y);
                let yb = positions.get(&b).map_or(0.0, |p| p.y);
                ya.partial_cmp(&yb)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.cmp(&b))
            });

            let mut min_y = INITIAL_Y;
            for &serial in nodes.iter() {
                if let Some(pos) = positions.get_mut(&serial) {
                    if pos.y < min_y {
                        pos.y = min_y;
                        any_shift = true;
                    }
                    min_y = pos.y + ROW_HEIGHT;
                }
            }
        }

        if !any_shift {
            break;
        }
    }

    (passes, positions)
}

#[test]
fn test_clearance_convergence_passes_benchmark() {
    let demo_graph = model::create_demo_graph();
    let demo_vis: HashSet<u64> = demo_graph.nodes.keys().copied().collect();
    let (passes_demo, _) = count_clearance_passes(&demo_graph, &demo_vis);
    println!(">>> BENCHMARK PASSES: Demo Graph converged in {passes_demo} passes");
    assert!(passes_demo <= 5, "Demo graph should converge in <= 5 passes (got {passes_demo})");

    // Flaw 1: Figure-8
    let mut g_f1 = PwGraph::default();
    for id in 1..=5 {
        let n = build_node(id, &format!("Node {id}"), NodeCategory::AudioDuplex, 2, 2);
        for p in &n.input_ports { g_f1.ports.insert(p.object_serial, p.clone()); }
        for p in &n.output_ports { g_f1.ports.insert(p.object_serial, p.clone()); }
        g_f1.nodes.insert(id, n);
    }
    g_f1.links.insert(101, make_link(101, 1, 2000, 2, 2000));
    g_f1.links.insert(102, make_link(102, 2, 4000, 3, 3000));
    g_f1.links.insert(103, make_link(103, 3, 6000, 1, 1000));
    g_f1.links.insert(104, make_link(104, 3, 6001, 4, 4000));
    g_f1.links.insert(105, make_link(105, 4, 8000, 5, 5000));
    g_f1.links.insert(106, make_link(106, 5, 10000, 3, 3001));
    let vis_f1: HashSet<u64> = (1..=5).collect();
    let (passes_f1, _) = count_clearance_passes(&g_f1, &vis_f1);
    println!(">>> BENCHMARK PASSES: Flaw 1 (Figure-8) converged in {passes_f1} passes");
    assert!(passes_f1 <= 5, "Flaw 1 should converge in <= 5 passes (got {passes_f1})");

    // Flaw 2: 10-stage chain with 4 skips
    let mut g_f2 = PwGraph::default();
    for i in 0..10 {
        let nid = i as u64 + 1;
        let n = build_node(nid, &format!("S_{i}"), NodeCategory::AudioDuplex, 5, 5);
        for p in &n.input_ports { g_f2.ports.insert(p.object_serial, p.clone()); }
        for p in &n.output_ports { g_f2.ports.insert(p.object_serial, p.clone()); }
        g_f2.nodes.insert(nid, n);
    }
    let mut lid = 1000;
    for i in 1..10 {
        g_f2.links.insert(lid, make_link(lid, i, i * 2000, i + 1, (i + 1) * 1000));
        lid += 1;
    }
    for (idx, &(u, v)) in [(1, 4), (2, 6), (4, 8), (5, 10)].iter().enumerate() {
        g_f2.links.insert(lid, make_link(lid, u, u * 2000 + (idx as u64 + 1), v, v * 1000 + (idx as u64 + 1)));
        lid += 1;
    }
    let vis_f2: HashSet<u64> = (1..=10).collect();
    let (passes_f2, _) = count_clearance_passes(&g_f2, &vis_f2);
    println!(">>> BENCHMARK PASSES: Flaw 2 (10-stage skip chain) converged in {passes_f2} passes");
    assert!(passes_f2 <= 5, "Flaw 2 should converge in <= 5 passes (got {passes_f2})");

    // Stress 2: 8 nodes with 9 cross-skips
    let mut g_s2 = PwGraph::default();
    for i in 1..=8 {
        let n = build_node(i, &format!("N_{i}"), NodeCategory::AudioDuplex, 8, 8);
        for p in &n.input_ports { g_s2.ports.insert(p.object_serial, p.clone()); }
        for p in &n.output_ports { g_s2.ports.insert(p.object_serial, p.clone()); }
        g_s2.nodes.insert(i, n);
    }
    let mut lid2 = 2000;
    for i in 1..8 {
        g_s2.links.insert(lid2, make_link(lid2, i, i * 2000, i + 1, (i + 1) * 1000));
        lid2 += 1;
    }
    for (idx, &(u, v)) in [(1, 3), (1, 5), (1, 7), (2, 4), (2, 6), (2, 8), (3, 6), (4, 7), (5, 8)].iter().enumerate() {
        g_s2.links.insert(lid2, make_link(lid2, u, u * 2000 + (idx as u64 + 1), v, v * 1000 + (idx as u64 + 1)));
        lid2 += 1;
    }
    let vis_s2: HashSet<u64> = (1..=8).collect();
    let (passes_s2, _) = count_clearance_passes(&g_s2, &vis_s2);
    println!(">>> BENCHMARK PASSES: Stress 2 (9 cross-skips) converged in {passes_s2} passes");
    assert!(passes_s2 < 50, "Stress 2 must converge before 50 passes (got {passes_s2})");
}
