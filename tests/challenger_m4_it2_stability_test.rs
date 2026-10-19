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
use iced::{Point, Rectangle, Size, Vector};
use model::{
    compute_signal_flow_columns, create_demo_graph, NodeCategory, PortDirection, PwGraph,
    PwLink, PwNode, PwPort,
};
use std::collections::{BTreeMap, HashSet};

// ============================================================================
// Helper Builders
// ============================================================================

fn make_node(
    serial: u64,
    name: &str,
    category: NodeCategory,
    in_ports: usize,
    out_ports: usize,
) -> PwNode {
    let mut inputs = Vec::new();
    for i in 0..in_ports {
        let p_serial = serial * 1000 + i as u64;
        inputs.push(PwPort {
            id: p_serial as u32,
            object_serial: p_serial,
            node_id: serial as u32,
            node_serial: serial,
            name: format!("in_{i}"),
            direction: PortDirection::Input,
            channel: if i == 0 { "FL".to_string() } else { "FR".to_string() },
            is_monitor: false,
            props: BTreeMap::new(),
        });
    }

    let mut outputs = Vec::new();
    for i in 0..out_ports {
        let p_serial = serial * 2000 + i as u64;
        outputs.push(PwPort {
            id: p_serial as u32,
            object_serial: p_serial,
            node_id: serial as u32,
            node_serial: serial,
            name: format!("out_{i}"),
            direction: PortDirection::Output,
            channel: if i == 0 { "FL".to_string() } else { "FR".to_string() },
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
            NodeCategory::AudioOutputStream => "Stream/Output/Audio".to_string(),
            NodeCategory::AudioInputStream => "Stream/Input/Audio".to_string(),
            NodeCategory::AudioDuplex => "Audio/Duplex".to_string(),
            NodeCategory::Midi => "Midi/Bridge".to_string(),
            NodeCategory::Video => "Video/Source".to_string(),
            NodeCategory::Internal => "Internal".to_string(),
        },
        category,
        input_ports: inputs,
        output_ports: outputs,
        state: "running".to_string(),
        volume: Some(1.0),
        muted: Some(false),
        channel_names: vec!["FL".to_string(), "FR".to_string()],
        channel_volumes: vec![1.0, 1.0],
        props: BTreeMap::new(),
        params: Vec::new(),
    }
}

fn make_link(link_serial: u64, out_node: u64, out_port: u64, in_node: u64, in_port: u64) -> PwLink {
    PwLink {
        id: link_serial as u32,
        object_serial: link_serial,
        output_node_id: out_node as u32,
        output_node_serial: out_node,
        output_port_id: out_port as u32,
        output_port_serial: out_port,
        input_node_id: in_node as u32,
        input_node_serial: in_node,
        input_port_id: in_port as u32,
        input_port_serial: in_port,
        state: "active".to_string(),
        props: BTreeMap::new(),
    }
}

// ============================================================================
// Simulated App State (Exact match to PipewireApp state machine)
// ============================================================================

struct HarnessApp {
    graph: PwGraph,
    node_positions: BTreeMap<u64, Point>,
    show_audio: bool,
    show_midi: bool,
    show_video: bool,
    show_internal: bool,
}

impl HarnessApp {
    fn new(graph: PwGraph) -> Self {
        Self {
            graph,
            node_positions: BTreeMap::new(),
            show_audio: true,
            show_midi: false,
            show_video: false,
            show_internal: false,
        }
    }

    fn visible_node_serials(&self) -> HashSet<u64> {
        self.graph
            .nodes
            .values()
            .filter(|n| match n.category {
                NodeCategory::AudioSource
                | NodeCategory::AudioOutputStream
                | NodeCategory::AudioDuplex
                | NodeCategory::AudioInputStream
                | NodeCategory::AudioSink => self.show_audio,
                NodeCategory::Midi => self.show_midi,
                NodeCategory::Video => self.show_video,
                NodeCategory::Internal => self.show_internal,
            })
            .map(|n| n.object_serial)
            .collect()
    }

    fn compute_auto_layout(&mut self) {
        let visible = self.visible_node_serials();
        self.node_positions = compute_auto_layout_positions(&self.graph, &visible);
        self.resolve_node_overlaps(None);
    }

    fn find_sensible_free_spot(&self, new_node: &PwNode) -> Point {
        canvas_view::find_sensible_free_spot(&self.graph, &self.node_positions, new_node)
    }

    fn ensure_positions_for_new_nodes(&mut self) {
        if self.node_positions.is_empty() {
            self.compute_auto_layout();
            return;
        }

        self.node_positions
            .retain(|serial, _| self.graph.nodes.contains_key(serial));

        let missing: Vec<u64> = self
            .graph
            .nodes
            .keys()
            .copied()
            .filter(|serial| !self.node_positions.contains_key(serial))
            .collect();

        for serial in missing {
            if let Some(new_node) = self.graph.nodes.get(&serial) {
                let spot = self.find_sensible_free_spot(new_node);
                self.node_positions.insert(serial, spot);
            }
        }
    }

    fn resolve_node_overlaps(&mut self, priority_serial: Option<u64>) {
        let visible = self.visible_node_serials();
        let min_dist_x = NODE_WIDTH + 12.0;
        let min_dist_y = NODE_HEIGHT + 12.0;

        let active_serials: Vec<u64> = self
            .node_positions
            .keys()
            .copied()
            .filter(|s| visible.contains(s))
            .collect();

        let pinned_pos = priority_serial.and_then(|prio| self.node_positions.get(&prio).copied());

        for _ in 0..15 {
            let mut any_overlap = false;
            for i in 0..active_serials.len() {
                for j in (i + 1)..active_serials.len() {
                    let s_a = active_serials[i];
                    let s_b = active_serials[j];
                    let (pos_a, pos_b) = match (
                        self.node_positions.get(&s_a),
                        self.node_positions.get(&s_b),
                    ) {
                        (Some(&a), Some(&b)) => (a, b),
                        _ => continue,
                    };

                    let dx = (pos_b.x - pos_a.x).abs();
                    let dy = (pos_b.y - pos_a.y).abs();

                    if dx < min_dist_x && dy < min_dist_y {
                        any_overlap = true;
                        let overlap_x = min_dist_x - dx;
                        let overlap_y = min_dist_y - dy;

                        if overlap_y <= overlap_x {
                            let shift_y = overlap_y + 4.0;
                            if pos_b.y >= pos_a.y {
                                if priority_serial == Some(s_a) {
                                    self.node_positions.insert(s_b, Point::new(pos_b.x, pos_b.y + shift_y));
                                } else if priority_serial == Some(s_b) {
                                    self.node_positions.insert(s_a, Point::new(pos_a.x, pos_a.y - shift_y));
                                } else {
                                    let half = shift_y * 0.5;
                                    self.node_positions.insert(s_b, Point::new(pos_b.x, pos_b.y + half));
                                    self.node_positions.insert(s_a, Point::new(pos_a.x, pos_a.y - half));
                                }
                            } else {
                                if priority_serial == Some(s_a) {
                                    self.node_positions.insert(s_b, Point::new(pos_b.x, pos_b.y - shift_y));
                                } else if priority_serial == Some(s_b) {
                                    self.node_positions.insert(s_a, Point::new(pos_a.x, pos_a.y + shift_y));
                                } else {
                                    let half = shift_y * 0.5;
                                    self.node_positions.insert(s_b, Point::new(pos_b.x, pos_b.y - half));
                                    self.node_positions.insert(s_a, Point::new(pos_a.x, pos_a.y + half));
                                }
                            }
                        }
                    }
                }
            }
            if !any_overlap {
                break;
            }
        }

        if let (Some(prio), Some(orig_pos)) = (priority_serial, pinned_pos) {
            self.node_positions.insert(prio, orig_pos);
        }
    }
}

// ============================================================================
// Bit-for-Bit Verifier Helpers
// ============================================================================

fn assert_bit_identical(
    current: &BTreeMap<u64, Point>,
    baseline: &BTreeMap<u64, Point>,
    context: &str,
) {
    for (&serial, &base_pos) in baseline {
        let curr_pos = current.get(&serial).unwrap_or_else(|| {
            panic!("{context}: Node {serial} missing from current positions!")
        });
        assert_eq!(
            curr_pos.x.to_bits(),
            base_pos.x.to_bits(),
            "{context}: Node {serial} x bit-shift: was {} ({:#010x}), now {} ({:#010x})",
            base_pos.x,
            base_pos.x.to_bits(),
            curr_pos.x,
            curr_pos.x.to_bits()
        );
        assert_eq!(
            curr_pos.y.to_bits(),
            base_pos.y.to_bits(),
            "{context}: Node {serial} y bit-shift: was {} ({:#010x}), now {} ({:#010x})",
            base_pos.y,
            base_pos.y.to_bits(),
            curr_pos.y,
            curr_pos.y.to_bits()
        );
    }
}

fn assert_no_overlaps(positions: &BTreeMap<u64, Point>, context: &str) {
    let list: Vec<(u64, Point)> = positions.iter().map(|(&s, &p)| (s, p)).collect();
    for i in 0..list.len() {
        for j in (i + 1)..list.len() {
            let (s1, p1) = list[i];
            let (s2, p2) = list[j];
            let dx = (p1.x - p2.x).abs();
            let dy = (p1.y - p2.y).abs();
            assert!(
                dx >= NODE_WIDTH + 12.0 || dy >= NODE_HEIGHT + 12.0,
                "{context}: Node {s1} ({p1:?}) overlaps Node {s2} ({p2:?})! dx={dx:.1}, dy={dy:.1}"
            );
        }
    }
}

fn assert_no_wire_collisions(graph: &PwGraph, positions: &BTreeMap<u64, Point>, context: &str) {
    for edge in graph.collapsed_edges() {
        let u = edge.key.output_node_serial;
        let v = edge.key.input_node_serial;
        let (Some(&pos_u), Some(&pos_v)) = (positions.get(&u), positions.get(&v)) else {
            continue;
        };
        let p0 = node_output_socket(pos_u);
        let p3 = node_input_socket(pos_v);

        for (&serial, &pos) in positions {
            if serial == u || serial == v {
                continue;
            }
            let node_rect = Rectangle::new(pos, Size::new(NODE_WIDTH, NODE_HEIGHT));
            assert!(
                !node_intersects_link_curve(node_rect, p0, p3),
                "{context}: Node {serial} at {pos:?} intersects wire from {u} to {v}"
            );
        }
    }
}

// ============================================================================
// CHALLENGER 2 EMPIRICAL VERIFICATION TESTS
// ============================================================================

#[test]
fn test_c2_dynamic_addition_10_sequential_nodes_bit_for_bit() {
    let mut app = HarnessApp::new(create_demo_graph());
    app.ensure_positions_for_new_nodes();
    assert_eq!(app.node_positions.len(), 6);

    let baseline = app.node_positions.clone();

    // 10 diverse additions: sources, sinks, duplex filters, stream outputs, stream inputs, video, midi
    let additions = [
        (101, "Microphone 2", NodeCategory::AudioSource, 0, 1),
        (102, "USB Headset", NodeCategory::AudioSink, 1, 0),
        (103, "Noise Torch", NodeCategory::AudioDuplex, 1, 1),
        (104, "OBS Studio Stream", NodeCategory::AudioOutputStream, 0, 2),
        (105, "Discord Audio In", NodeCategory::AudioInputStream, 2, 0),
        (106, "Midi Keyboard", NodeCategory::Midi, 0, 1),
        (107, "Webcam Video", NodeCategory::Video, 0, 1),
        (108, "Internal Routing", NodeCategory::Internal, 0, 0),
        (109, "Master Bus Compressor", NodeCategory::AudioDuplex, 2, 2),
        (110, "Subwoofer Out", NodeCategory::AudioSink, 1, 0),
    ];

    for (idx, (serial, name, cat, in_p, out_p)) in additions.iter().enumerate() {
        let snapshot_before = app.node_positions.clone();

        let node = make_node(*serial, name, *cat, *in_p, *out_p);
        app.graph.nodes.insert(*serial, node);
        app.ensure_positions_for_new_nodes();

        // 1. Bit-for-bit invariance for all pre-existing nodes
        assert_bit_identical(
            &app.node_positions,
            &snapshot_before,
            &format!("Sequential addition step {} (serial {})", idx + 1, serial),
        );

        // 2. Newly added node exists
        assert!(app.node_positions.contains_key(serial));

        // 3. Newly added node does not overlap any existing node
        let new_pos = app.node_positions[serial];
        for (&prev_s, &prev_pos) in &snapshot_before {
            let dx = (prev_pos.x - new_pos.x).abs();
            let dy = (prev_pos.y - new_pos.y).abs();
            assert!(
                dx >= NODE_WIDTH + 12.0 || dy >= NODE_HEIGHT + 12.0,
                "New node {serial} overlaps existing node {prev_s}"
            );
        }

        // 4. Wire clearance
        assert_no_wire_collisions(&app.graph, &app.node_positions, "Post addition");
    }

    assert_eq!(app.node_positions.len(), 16);
    assert_no_overlaps(&app.node_positions, "16 nodes final");
}

#[test]
fn test_c2_dynamic_removal_5_sequential_nodes_bit_for_bit() {
    let mut app = HarnessApp::new(create_demo_graph());
    app.ensure_positions_for_new_nodes();

    // Add 5 nodes first
    for i in 1..=5 {
        let s = 200 + i;
        app.graph.nodes.insert(s, make_node(s, "Extra", NodeCategory::AudioDuplex, 1, 1));
    }
    app.ensure_positions_for_new_nodes();
    assert_eq!(app.node_positions.len(), 11);

    // Remove 5 nodes sequentially: original nodes (40, 20) and added nodes (205, 202, 201)
    let removals = [205, 40, 202, 20, 201];
    for (idx, &serial) in removals.iter().enumerate() {
        let mut expected = app.node_positions.clone();
        expected.remove(&serial);

        app.graph.nodes.remove(&serial);
        app.graph.links.retain(|_, l| l.output_node_serial != serial && l.input_node_serial != serial);
        app.ensure_positions_for_new_nodes();

        // Pruned
        assert!(!app.node_positions.contains_key(&serial));

        // Surviving nodes bit-for-bit identical
        assert_bit_identical(
            &app.node_positions,
            &expected,
            &format!("Removal step {} (removed {})", idx + 1, serial),
        );
        assert_eq!(app.node_positions.len(), expected.len());
    }

    assert_eq!(app.node_positions.len(), 6);
    assert_no_overlaps(&app.node_positions, "After 5 removals");
}

#[test]
fn test_c2_parameter_mutations_bit_for_bit_invariance() {
    let mut app = HarnessApp::new(create_demo_graph());
    app.ensure_positions_for_new_nodes();
    let baseline = app.node_positions.clone();

    // 1. Volume sweep
    for vol in [0.0, 0.1, 0.5, 0.99, 1.0, 1.25, 2.0] {
        for node in app.graph.nodes.values_mut() {
            node.volume = Some(vol);
        }
        app.ensure_positions_for_new_nodes();
        assert_bit_identical(&app.node_positions, &baseline, &format!("Volume sweep {vol}"));
    }

    // 2. Mute toggles
    for muted in [true, false, true, false, true, false] {
        for node in app.graph.nodes.values_mut() {
            node.muted = Some(muted);
        }
        app.ensure_positions_for_new_nodes();
        assert_bit_identical(&app.node_positions, &baseline, &format!("Mute toggle {muted}"));
    }

    // 3. Channel volumes
    for ch_vol in [0.05, 0.33, 0.77, 1.0] {
        for node in app.graph.nodes.values_mut() {
            node.channel_volumes.fill(ch_vol);
        }
        app.ensure_positions_for_new_nodes();
        assert_bit_identical(&app.node_positions, &baseline, &format!("Channel vol {ch_vol}"));
    }
}

#[test]
fn test_c2_category_filter_toggles_50_cycles_bit_for_bit() {
    let mut app = HarnessApp::new(create_demo_graph());
    // Add nodes in all categories
    app.graph.nodes.insert(301, make_node(301, "Midi", NodeCategory::Midi, 1, 1));
    app.graph.nodes.insert(302, make_node(302, "Video", NodeCategory::Video, 1, 1));
    app.graph.nodes.insert(303, make_node(303, "Internal", NodeCategory::Internal, 0, 0));

    // Turn all categories on to position all nodes
    app.show_audio = true;
    app.show_midi = true;
    app.show_video = true;
    app.show_internal = true;
    app.ensure_positions_for_new_nodes();
    assert_eq!(app.node_positions.len(), 9);

    let baseline = app.node_positions.clone();

    let mut state: u32 = 0xCAFEBABE;
    for cycle in 0..50 {
        state = state.wrapping_mul(1103515245).wrapping_add(12345);
        let bits = state >> 16;
        app.show_audio = (bits & 1) != 0;
        app.show_midi = (bits & 2) != 0;
        app.show_video = (bits & 4) != 0;
        app.show_internal = (bits & 8) != 0;

        app.ensure_positions_for_new_nodes();

        assert_bit_identical(
            &app.node_positions,
            &baseline,
            &format!("Category filter toggle cycle {cycle}"),
        );
    }
}

#[test]
fn test_c2_auto_layout_button_resets_manually_dragged_coordinates() {
    let mut app = HarnessApp::new(create_demo_graph());
    app.ensure_positions_for_new_nodes();
    let canonical = app.node_positions.clone();

    // Verify canonical demo columns
    assert_eq!(canonical[&10].x, 40.0);  // Mic -> Col 0
    assert_eq!(canonical[&20].x, 40.0);  // Firefox -> Col 0
    assert_eq!(canonical[&30].x, 40.0);  // Spotify -> Col 0
    assert_eq!(canonical[&40].x, 380.0); // EasyEffects -> Col 1
    assert_eq!(canonical[&60].x, 380.0); // Discord -> Col 1
    assert_eq!(canonical[&50].x, 720.0); // Headphones -> Col 2

    // Scramble all 6 nodes to completely wrong coordinates
    app.node_positions.insert(10, Point::new(1200.0, 500.0));
    app.node_positions.insert(20, Point::new(1400.0, 700.0));
    app.node_positions.insert(30, Point::new(900.0, 300.0));
    app.node_positions.insert(40, Point::new(40.0, 40.0));
    app.node_positions.insert(50, Point::new(0.0, 0.0));
    app.node_positions.insert(60, Point::new(2000.0, 2000.0));

    // Press Auto-Layout button
    app.compute_auto_layout();

    // Verify bit-for-bit restoration to canonical positions
    assert_bit_identical(&app.node_positions, &canonical, "Post Auto-Layout reset");
    assert_no_overlaps(&app.node_positions, "Post Auto-Layout reset");
    assert_no_wire_collisions(&app.graph, &app.node_positions, "Post Auto-Layout reset");
}

#[test]
fn test_c2_ensure_positions_idempotency_100_passes() {
    let mut app = HarnessApp::new(create_demo_graph());
    app.ensure_positions_for_new_nodes();
    let baseline = app.node_positions.clone();

    for pass in 0..100 {
        app.ensure_positions_for_new_nodes();
        assert_bit_identical(
            &app.node_positions,
            &baseline,
            &format!("Idempotency pass {pass}"),
        );
    }
}

#[test]
fn test_c2_extreme_user_dragged_coordinates_preserved_under_dynamic_churn() {
    let mut app = HarnessApp::new(create_demo_graph());
    app.ensure_positions_for_new_nodes();

    // User places node 10 at negative coordinates and node 50 at far positive coordinates
    let custom_10 = Point::new(-850.0, -420.0);
    let custom_50 = Point::new(99999.0, 88888.0);
    app.node_positions.insert(10, custom_10);
    app.node_positions.insert(50, custom_50);

    // Dynamic additions
    for i in 1..=10 {
        let s = 500 + i;
        app.graph.nodes.insert(s, make_node(s, "Churn", NodeCategory::AudioInputStream, 1, 0));
        app.ensure_positions_for_new_nodes();

        assert_eq!(app.node_positions[&10].x.to_bits(), custom_10.x.to_bits());
        assert_eq!(app.node_positions[&10].y.to_bits(), custom_10.y.to_bits());
        assert_eq!(app.node_positions[&50].x.to_bits(), custom_50.x.to_bits());
        assert_eq!(app.node_positions[&50].y.to_bits(), custom_50.y.to_bits());
    }

    // Dynamic removals
    for i in 1..=5 {
        let s = 500 + i;
        app.graph.nodes.remove(&s);
        app.ensure_positions_for_new_nodes();

        assert_eq!(app.node_positions[&10].x.to_bits(), custom_10.x.to_bits());
        assert_eq!(app.node_positions[&10].y.to_bits(), custom_10.y.to_bits());
        assert_eq!(app.node_positions[&50].x.to_bits(), custom_50.x.to_bits());
        assert_eq!(app.node_positions[&50].y.to_bits(), custom_50.y.to_bits());
    }
}
