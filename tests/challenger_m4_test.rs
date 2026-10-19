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
    compute_signal_flow_columns, create_demo_graph, NodeCategory, PortDirection, PwGraph,
    PwLink, PwNode, PwPort,
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

/// Verifies invariants across arbitrary graphs:
/// 1. Zero node card overlaps (dx >= NODE_WIDTH + 12 || dy >= NODE_HEIGHT + 12).
/// 2. Zero node rectangles intersecting sampled cubic bezier curves they are not endpoints of.
/// 3. Signal-flow depths: each node has a column, and for DAG forward edges, depth strictly increases.
fn assert_graph_invariants(graph: &PwGraph, label: &str, is_dag: bool) -> (BTreeMap<u64, usize>, BTreeMap<u64, Point>) {
    let visible: HashSet<u64> = graph.nodes.keys().copied().collect();
    let cols = compute_signal_flow_columns(graph, &visible);
    let positions = compute_auto_layout_positions(graph, &visible);

    // 1. Column assignment invariant: Every visible node must have a column and position.
    for &serial in &visible {
        assert!(cols.contains_key(&serial), "{label}: node {serial} missing column");
        assert!(positions.contains_key(&serial), "{label}: node {serial} missing position");
    }

    // 2. Signal-flow depth check: for a DAG, every link u -> v must satisfy cols[v] > cols[u].
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

    // 3. Zero node rectangle overlaps:
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
                "{label}: Overlap between node {s_a} and node {s_b}! p_a={p_a:?}, p_b={p_b:?}, dx={dx}, dy={dy}"
            );
        }
    }

    // 4. Zero node rectangles intersecting sampled cubic bezier curves they are not endpoints of:
    for edge in graph.collapsed_edges() {
        let u = edge.key.output_node_serial;
        let v = edge.key.input_node_serial;
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
// 1. DEEP LINEAR CHAIN (10 stages)
// ============================================================================

#[test]
fn test_adversarial_deep_linear_chain_10_stages() {
    let mut graph = PwGraph::default();
    let num_stages = 10;

    for i in 0..num_stages {
        let cat = if i == 0 {
            NodeCategory::AudioSource
        } else if i == num_stages - 1 {
            NodeCategory::AudioSink
        } else {
            NodeCategory::AudioDuplex
        };
        let in_p = if i == 0 { 0 } else { 1 };
        let out_p = if i == num_stages - 1 { 0 } else { 1 };
        let node = build_node(i as u64 + 1, &format!("Stage {i}"), cat, in_p, out_p);
        for p in &node.input_ports {
            graph.ports.insert(p.object_serial, p.clone());
        }
        for p in &node.output_ports {
            graph.ports.insert(p.object_serial, p.clone());
        }
        graph.nodes.insert(node.object_serial, node);
    }

    for i in 0..(num_stages - 1) {
        let u = i as u64 + 1;
        let v = (i + 1) as u64 + 1;
        let link_id = 5000 + i as u64;
        let out_port = u * 2000;
        let in_port = v * 1000;
        graph.links.insert(link_id, make_link(link_id, u, out_port, v, in_port));
    }

    let (cols, positions) = assert_graph_invariants(&graph, "Deep Linear Chain 10 Stages", true);

    for i in 0..num_stages {
        let node_id = i as u64 + 1;
        assert_eq!(
            cols.get(&node_id),
            Some(&i),
            "Stage {i} (node {node_id}) must be in Column {i}"
        );
        let pos = positions.get(&node_id).unwrap();
        assert_eq!(pos.x, INITIAL_X + i as f32 * COL_SPACING);
    }
}

// ============================================================================
// 2. WIDE MULTI-BRANCH FAN-OUT / FAN-IN GRAPHS
// ============================================================================

#[test]
fn test_adversarial_wide_fan_out_fan_in_1_to_10_to_1() {
    let mut graph = PwGraph::default();

    let source = build_node(1, "Master Source", NodeCategory::AudioSource, 0, 10);
    for p in &source.output_ports {
        graph.ports.insert(p.object_serial, p.clone());
    }
    graph.nodes.insert(1, source);

    for i in 0..10 {
        let id = 10 + i as u64;
        let node = build_node(id, &format!("Filter Branch {i}"), NodeCategory::AudioDuplex, 1, 1);
        graph.ports.insert(node.input_ports[0].object_serial, node.input_ports[0].clone());
        graph.ports.insert(node.output_ports[0].object_serial, node.output_ports[0].clone());
        graph.nodes.insert(id, node);
    }

    let sink = build_node(99, "Master Sink", NodeCategory::AudioSink, 10, 0);
    for p in &sink.input_ports {
        graph.ports.insert(p.object_serial, p.clone());
    }
    graph.nodes.insert(99, sink);

    let mut link_id = 20000;
    for i in 0..10 {
        let branch_id = 10 + i as u64;
        let out_port = 2000 + i as u64;
        let in_port = branch_id * 1000;
        graph.links.insert(link_id, make_link(link_id, 1, out_port, branch_id, in_port));
        link_id += 1;
    }

    for i in 0..10 {
        let branch_id = 10 + i as u64;
        let out_port = branch_id * 2000;
        let in_port = 99 * 1000 + i as u64;
        graph.links.insert(link_id, make_link(link_id, branch_id, out_port, 99, in_port));
        link_id += 1;
    }

    let (cols, positions) = assert_graph_invariants(&graph, "Wide Fan-Out Fan-In (1 -> 10 -> 1)", true);

    assert_eq!(cols.get(&1), Some(&0), "Master Source must be in Col 0");
    for i in 0..10 {
        let branch_id = 10 + i as u64;
        assert_eq!(cols.get(&branch_id), Some(&1), "Branch {i} must be in Col 1");
    }
    assert_eq!(cols.get(&99), Some(&2), "Master Sink must be in Col 2");
}

#[test]
fn test_adversarial_wide_multi_tier_cross_connected_mesh() {
    let mut graph = PwGraph::default();

    for s in 0..4 {
        let id = 100 + s as u64;
        let node = build_node(id, &format!("Src {s}"), NodeCategory::AudioOutputStream, 0, 6);
        for p in &node.output_ports {
            graph.ports.insert(p.object_serial, p.clone());
        }
        graph.nodes.insert(id, node);
    }

    for f in 0..6 {
        let id = 200 + f as u64;
        let node = build_node(id, &format!("Filt {f}"), NodeCategory::AudioDuplex, 4, 4);
        for p in &node.input_ports {
            graph.ports.insert(p.object_serial, p.clone());
        }
        for p in &node.output_ports {
            graph.ports.insert(p.object_serial, p.clone());
        }
        graph.nodes.insert(id, node);
    }

    for d in 0..4 {
        let id = 300 + d as u64;
        let node = build_node(id, &format!("Dst {d}"), NodeCategory::AudioSink, 6, 0);
        for p in &node.input_ports {
            graph.ports.insert(p.object_serial, p.clone());
        }
        graph.nodes.insert(id, node);
    }

    let mut link_id = 30000;
    for s in 0..4 {
        for f in 0..6 {
            let u = 100 + s as u64;
            let v = 200 + f as u64;
            let out_port = u * 2000 + f as u64;
            let in_port = v * 1000 + s as u64;
            graph.links.insert(link_id, make_link(link_id, u, out_port, v, in_port));
            link_id += 1;
        }
    }

    for f in 0..6 {
        for d in 0..4 {
            let u = 200 + f as u64;
            let v = 300 + d as u64;
            let out_port = u * 2000 + d as u64;
            let in_port = v * 1000 + f as u64;
            graph.links.insert(link_id, make_link(link_id, u, out_port, v, in_port));
            link_id += 1;
        }
    }

    let (cols, positions) = assert_graph_invariants(&graph, "Wide Multi-Tier Mesh", true);
    for s in 0..4 {
        assert_eq!(cols.get(&(100 + s as u64)), Some(&0));
    }
    for f in 0..6 {
        assert_eq!(cols.get(&(200 + f as u64)), Some(&1));
    }
    for d in 0..4 {
        assert_eq!(cols.get(&(300 + d as u64)), Some(&2));
    }
}

// ============================================================================
// 3. COMPLETE BIPARTITE GRAPHS (K_4,4 and K_6,6)
// ============================================================================

#[test]
fn test_adversarial_complete_bipartite_k_4_4() {
    let mut graph = PwGraph::default();
    let m = 4;
    let n = 4;

    for i in 0..m {
        let id = 10 + i as u64;
        let node = build_node(id, &format!("Source {i}"), NodeCategory::AudioSource, 0, n);
        for p in &node.output_ports {
            graph.ports.insert(p.object_serial, p.clone());
        }
        graph.nodes.insert(id, node);
    }

    for j in 0..n {
        let id = 50 + j as u64;
        let node = build_node(id, &format!("Sink {j}"), NodeCategory::AudioSink, m, 0);
        for p in &node.input_ports {
            graph.ports.insert(p.object_serial, p.clone());
        }
        graph.nodes.insert(id, node);
    }

    let mut link_id = 40000;
    for i in 0..m {
        for j in 0..n {
            let u = 10 + i as u64;
            let v = 50 + j as u64;
            let out_port = u * 2000 + j as u64;
            let in_port = v * 1000 + i as u64;
            graph.links.insert(link_id, make_link(link_id, u, out_port, v, in_port));
            link_id += 1;
        }
    }

    let (cols, positions) = assert_graph_invariants(&graph, "Complete Bipartite K_4,4", true);

    for i in 0..m {
        assert_eq!(cols.get(&(10 + i as u64)), Some(&0));
    }
    for j in 0..n {
        assert_eq!(cols.get(&(50 + j as u64)), Some(&1));
    }
}

#[test]
fn test_adversarial_complete_bipartite_k_6_6() {
    let mut graph = PwGraph::default();
    let m = 6;
    let n = 6;

    for i in 0..m {
        let id = 10 + i as u64;
        let node = build_node(id, &format!("Source {i}"), NodeCategory::AudioSource, 0, n);
        for p in &node.output_ports {
            graph.ports.insert(p.object_serial, p.clone());
        }
        graph.nodes.insert(id, node);
    }

    for j in 0..n {
        let id = 50 + j as u64;
        let node = build_node(id, &format!("Sink {j}"), NodeCategory::AudioSink, m, 0);
        for p in &node.input_ports {
            graph.ports.insert(p.object_serial, p.clone());
        }
        graph.nodes.insert(id, node);
    }

    let mut link_id = 45000;
    for i in 0..m {
        for j in 0..n {
            let u = 10 + i as u64;
            let v = 50 + j as u64;
            let out_port = u * 2000 + j as u64;
            let in_port = v * 1000 + i as u64;
            graph.links.insert(link_id, make_link(link_id, u, out_port, v, in_port));
            link_id += 1;
        }
    }

    let (cols, positions) = assert_graph_invariants(&graph, "Complete Bipartite K_6,6", true);

    for i in 0..m {
        assert_eq!(cols.get(&(10 + i as u64)), Some(&0));
    }
    for j in 0..n {
        assert_eq!(cols.get(&(50 + j as u64)), Some(&1));
    }
}

// ============================================================================
// 4. DISCONNECTED COMPONENTS AND ISOLATED NODES
// ============================================================================

#[test]
fn test_adversarial_disconnected_components_and_isolated_nodes() {
    let mut graph = PwGraph::default();

    // Component 1: Linear chain (10 -> 11 -> 12)
    let c1_src = build_node(10, "C1 Source", NodeCategory::AudioSource, 0, 1);
    let c1_mid = build_node(11, "C1 Mid", NodeCategory::AudioDuplex, 1, 1);
    let c1_snk = build_node(12, "C1 Sink", NodeCategory::AudioSink, 1, 0);
    for n in [&c1_src, &c1_mid, &c1_snk] {
        for p in &n.input_ports {
            graph.ports.insert(p.object_serial, p.clone());
        }
        for p in &n.output_ports {
            graph.ports.insert(p.object_serial, p.clone());
        }
        graph.nodes.insert(n.object_serial, n.clone());
    }
    graph.links.insert(60001, make_link(60001, 10, 10 * 2000, 11, 11 * 1000));
    graph.links.insert(60002, make_link(60002, 11, 11 * 2000, 12, 12 * 1000));

    // Component 2: Independent pair (20 -> 21)
    let c2_src = build_node(20, "C2 Player", NodeCategory::AudioOutputStream, 0, 1);
    let c2_snk = build_node(21, "C2 Headset", NodeCategory::AudioSink, 1, 0);
    for n in [&c2_src, &c2_snk] {
        for p in &n.input_ports {
            graph.ports.insert(p.object_serial, p.clone());
        }
        for p in &n.output_ports {
            graph.ports.insert(p.object_serial, p.clone());
        }
        graph.nodes.insert(n.object_serial, n.clone());
    }
    graph.links.insert(60003, make_link(60003, 20, 20 * 2000, 21, 21 * 1000));

    // Component 3: Isolated nodes
    let iso_src = build_node(30, "Isolated Mic", NodeCategory::AudioSource, 0, 1);
    let iso_duplex = build_node(31, "Isolated DSP", NodeCategory::AudioDuplex, 2, 2);
    let iso_sink = build_node(32, "Isolated Speaker", NodeCategory::AudioSink, 2, 0);
    let iso_midi = build_node(33, "Isolated MIDI", NodeCategory::Midi, 1, 1);
    for n in [&iso_src, &iso_duplex, &iso_sink, &iso_midi] {
        for p in &n.input_ports {
            graph.ports.insert(p.object_serial, p.clone());
        }
        for p in &n.output_ports {
            graph.ports.insert(p.object_serial, p.clone());
        }
        graph.nodes.insert(n.object_serial, n.clone());
    }

    let (cols, positions) = assert_graph_invariants(&graph, "Disconnected Components & Isolated Nodes", false);

    assert_eq!(cols.get(&10), Some(&0));
    assert_eq!(cols.get(&11), Some(&1));
    assert_eq!(cols.get(&12), Some(&2));

    assert_eq!(cols.get(&20), Some(&0));
    assert_eq!(cols.get(&21), Some(&1));

    assert_eq!(cols.get(&32), Some(&1));
    assert_eq!(cols.get(&30), Some(&0));
}

// ============================================================================
// 5. SEQUENTIAL FREE SPOT ALLOCATION UNDER HEAVY LOAD
// ============================================================================

#[test]
fn test_adversarial_sequential_free_spot_allocations_in_dense_canvas() {
    let graph = create_demo_graph();
    let visible: HashSet<u64> = [10, 20, 30, 40, 50, 60].into_iter().collect();
    let mut positions = compute_auto_layout_positions(&graph, &visible);
    let initial_positions = positions.clone();

    let mut working_graph = graph.clone();

    for i in 1..=15 {
        let new_id = 1000 + i as u64;
        let cat = match i % 5 {
            0 => NodeCategory::AudioSource,
            1 => NodeCategory::AudioOutputStream,
            2 => NodeCategory::AudioDuplex,
            3 => NodeCategory::AudioSink,
            _ => NodeCategory::AudioInputStream,
        };
        let new_node = build_node(new_id, &format!("Sequential Node {i}"), cat, 2, 2);
        for p in &new_node.input_ports {
            working_graph.ports.insert(p.object_serial, p.clone());
        }
        for p in &new_node.output_ports {
            working_graph.ports.insert(p.object_serial, p.clone());
        }
        working_graph.nodes.insert(new_id, new_node.clone());

        if i % 2 == 0 {
            let link_id = 70000 + i as u64;
            let out_node = 10;
            working_graph.links.insert(
                link_id,
                make_link(link_id, out_node, 10 * 2000, new_id, new_id * 1000),
            );
        }

        let spot = find_sensible_free_spot(&working_graph, &positions, &new_node);

        for (&existing_id, &existing_pos) in &positions {
            let dx = (existing_pos.x - spot.x).abs();
            let dy = (existing_pos.y - spot.y).abs();
            assert!(
                dx >= NODE_WIDTH + 12.0 || dy >= NODE_HEIGHT + 12.0,
                "Sequential step {i}: New node {new_id} at {spot:?} overlaps with node {existing_id} at {existing_pos:?}"
            );
        }

        let candidate_rect = Rectangle::new(spot, Size::new(NODE_WIDTH, NODE_HEIGHT));
        for edge in working_graph.collapsed_edges() {
            let u = edge.key.output_node_serial;
            let v = edge.key.input_node_serial;
            if u == new_id || v == new_id {
                continue;
            }
            if let (Some(&pos_u), Some(&pos_v)) = (positions.get(&u), positions.get(&v)) {
                let p0 = node_output_socket(pos_u);
                let p3 = node_input_socket(pos_v);
                assert!(
                    !node_intersects_link_curve(candidate_rect, p0, p3),
                    "Sequential step {i}: New node {new_id} at {spot:?} intersects link curve {u}->{v}"
                );
            }
        }

        positions.insert(new_id, spot);
    }

    for (&s, &orig_p) in &initial_positions {
        assert_eq!(
            positions[&s], orig_p,
            "Initial node {s} moved after 15 sequential additions!"
        );
    }
}

// ============================================================================
// 6. EMPIRICAL FLAW DEMONSTRATIONS (BUGS FOUND BY ADVERSARIAL CHALLENGER)
// ============================================================================

/// FLAW 1 VERIFICATION: Figure-8 Nested Cycles Wire Collision Cleared.
/// With 3-color DFS cycle breaking, cycle nodes receive distinct sequential columns.
/// Link curves route between columns, and envelope clearance guarantees Node 3 does NOT
/// collide with link curve 1 -> 2. All layout invariants hold.
#[test]
fn test_adversarial_empirical_flaw_1_figure_eight_cycle_violates_clearance() {
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

    let visible: HashSet<u64> = (1..=5).collect();
    let positions = compute_auto_layout_positions(&graph, &visible);

    // Node 3 does NOT collide with link curve 1 -> 2:
    let p0 = node_output_socket(positions[&1]);
    let p3 = node_input_socket(positions[&2]);
    let pos_3 = positions[&3];
    let w_rect = Rectangle::new(pos_3, Size::new(NODE_WIDTH, NODE_HEIGHT));
    assert!(
        !node_intersects_link_curve(w_rect, p0, p3),
        "Node 3 at {:?} intersects link curve from 1 to 2",
        pos_3
    );

    // Full invariant verification
    assert_graph_invariants(&graph, "Figure 8 nested cycle", false);
}

/// FLAW 2 VERIFICATION: Deep Chain with Cascading Skip Connections Converges.
/// Sorting edges longest-span-first and using single-step envelope jump clearance ensures
/// intermediate nodes clear bypass wires in <= 3 passes without runaway vertical expansion.
/// Node 3 does NOT intersect skip link curve 1 -> 4, and all layout invariants hold.
#[test]
fn test_adversarial_empirical_flaw_2_deep_chain_skip_connections_violates_clearance() {
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

    let visible: HashSet<u64> = (1..=num_stages as u64).collect();
    let positions = compute_auto_layout_positions(&graph, &visible);

    let p0 = node_output_socket(positions[&1]);
    let p3 = node_input_socket(positions[&4]);
    let pos_3 = positions[&3];
    let w_rect = Rectangle::new(pos_3, Size::new(NODE_WIDTH, NODE_HEIGHT));
    assert!(
        !node_intersects_link_curve(w_rect, p0, p3),
        "Node 3 at {:?} intersects link curve from 1 to 4",
        pos_3
    );

    // Full invariant verification (is_dag = true since all edges are u < v)
    assert_graph_invariants(&graph, "Deep chain with skip connections", true);
}

/// FLAW 3 VERIFICATION: Feedback Cycle Reachable from Source Has Bounded Columns & Contiguous Layout.
/// Cycle breaking prevents unbounded relaxation depth inflation. Dense column compaction
/// eliminates all empty column voids. Output Sink is placed downstream of the DSP filters.
#[test]
fn test_adversarial_empirical_flaw_3_feedback_cycle_column_explosion_and_inversion() {
    let mut graph = PwGraph::default();

    // Source 1
    let n1 = build_node(1, "Source", NodeCategory::AudioSource, 0, 1);
    graph.ports.insert(1000, n1.output_ports[0].clone());
    graph.nodes.insert(1, n1);

    // Feedback loop 2 <-> 3
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

    // Output Sink 4
    let n4 = build_node(4, "Output Sink", NodeCategory::AudioSink, 1, 0);
    graph.ports.insert(4000, n4.input_ports[0].clone());
    graph.nodes.insert(4, n4);

    // 1 -> 2 -> 3 -> 2 (feedback), 3 -> 4
    graph.links.insert(101, make_link(101, 1, 1000, 2, 2000));
    graph.links.insert(102, make_link(102, 2, 2002, 3, 3000));
    graph.links.insert(103, make_link(103, 3, 3001, 2, 2001)); // feedback
    graph.links.insert(104, make_link(104, 3, 3002, 4, 4000));

    let visible: HashSet<u64> = (1..=4).collect();
    let cols = compute_signal_flow_columns(&graph, &visible);
    let col_1 = *cols.get(&1).unwrap();
    let col_2 = *cols.get(&2).unwrap();
    let col_3 = *cols.get(&3).unwrap();
    let col_4 = *cols.get(&4).unwrap();

    // Signal flow order: Source (1) < Filter A (2) <= Filter B (3) <= Output Sink (4)
    assert_eq!(col_1, 0, "Source 1 must be at Column 0");
    assert!(col_1 < col_2, "Filter A (2) must be downstream of Source (1)");
    assert!(col_2 <= col_3, "Filter B (3) must be at or downstream of Filter A (2)");
    assert!(col_4 >= col_3, "Output Sink (4) must be at or downstream of Filter B (3)");

    // Dense compaction: no runaway column explosion, max column <= 3
    let max_col = cols.values().copied().max().unwrap_or(0);
    assert!(max_col <= 3, "Max column must be <= 3 (got {max_col})");

    let assigned_cols: BTreeSet<usize> = cols.values().copied().collect();
    let expected_cols: BTreeSet<usize> = (0..=max_col).collect();
    assert_eq!(assigned_cols, expected_cols, "Columns must be densely packed without voids");

    // Full invariant verification
    assert_graph_invariants(&graph, "Feedback Cycle Reachable from Source", false);
}
