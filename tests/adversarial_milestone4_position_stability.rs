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
// Helper: Synthetic Graph Node and Link Builder
// ============================================================================

fn make_test_node(
    serial: u64,
    name: &str,
    category: NodeCategory,
    in_ports_count: usize,
    out_ports_count: usize,
) -> PwNode {
    let mut in_ports = Vec::new();
    for i in 0..in_ports_count {
        let port_serial = serial * 1000 + i as u64;
        in_ports.push(PwPort {
            id: port_serial as u32,
            object_serial: port_serial,
            node_id: serial as u32,
            node_serial: serial,
            name: format!("input_{i}"),
            direction: PortDirection::Input,
            channel: if i == 0 { "FL".to_string() } else { "FR".to_string() },
            is_monitor: false,
            props: BTreeMap::new(),
        });
    }

    let mut out_ports = Vec::new();
    for i in 0..out_ports_count {
        let port_serial = serial * 2000 + i as u64;
        out_ports.push(PwPort {
            id: port_serial as u32,
            object_serial: port_serial,
            node_id: serial as u32,
            node_serial: serial,
            name: format!("output_{i}"),
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
        input_ports: in_ports,
        output_ports: out_ports,
        state: "running".to_string(),
        volume: Some(1.0),
        muted: Some(false),
        channel_names: vec!["FL".to_string(), "FR".to_string()],
        channel_volumes: vec![1.0, 1.0],
        props: BTreeMap::new(),
        params: Vec::new(),
    }
}

fn make_test_link(
    link_serial: u64,
    out_node_serial: u64,
    out_port_serial: u64,
    in_node_serial: u64,
    in_port_serial: u64,
) -> PwLink {
    PwLink {
        id: link_serial as u32,
        object_serial: link_serial,
        output_node_id: out_node_serial as u32,
        output_node_serial: out_node_serial,
        output_port_id: out_port_serial as u32,
        output_port_serial: out_port_serial,
        input_node_id: in_node_serial as u32,
        input_node_serial: in_node_serial,
        input_port_id: in_port_serial as u32,
        input_port_serial: in_port_serial,
        state: "active".to_string(),
        props: BTreeMap::new(),
    }
}

// ============================================================================
// Simulated Graph App State (Mirroring PipewireApp logic)
// ============================================================================

struct SimulatedPipewireApp {
    graph: PwGraph,
    node_positions: BTreeMap<u64, Point>,
    show_audio: bool,
    show_midi: bool,
    show_video: bool,
    show_internal: bool,
}

impl SimulatedPipewireApp {
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

        // When nodes are removed, prune them from node_positions without moving any remaining nodes (R4)
        self.node_positions
            .retain(|serial, _| self.graph.nodes.contains_key(serial));

        // When new nodes appear, place them in a sensible free spot without re-running auto-layout
        // or moving any existing nodes (R4)
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

    // Handlers simulating app.update(Message::...)
    fn change_volume(&mut self, node_serial: u64, volume: f32) {
        if let Some(node) = self.graph.nodes.get_mut(&node_serial) {
            node.volume = Some(volume);
        }
        // Notice: volume change never triggers auto-layout or position mutation!
    }

    fn toggle_mute(&mut self, node_serial: u64, muted: bool) {
        if let Some(node) = self.graph.nodes.get_mut(&node_serial) {
            node.muted = Some(muted);
        }
        // Notice: mute change never triggers auto-layout or position mutation!
    }

    fn change_channel_volume(&mut self, node_serial: u64, channel_idx: usize, volume: f32) {
        if let Some(node) = self.graph.nodes.get_mut(&node_serial) {
            if channel_idx < node.channel_volumes.len() {
                node.channel_volumes[channel_idx] = volume;
            }
        }
    }

    fn toggle_filter_audio(&mut self, val: bool) {
        self.show_audio = val;
        self.ensure_positions_for_new_nodes();
    }

    fn toggle_filter_midi(&mut self, val: bool) {
        self.show_midi = val;
        self.ensure_positions_for_new_nodes();
    }

    fn toggle_filter_video(&mut self, val: bool) {
        self.show_video = val;
        self.ensure_positions_for_new_nodes();
    }

    fn toggle_filter_internal(&mut self, val: bool) {
        self.show_internal = val;
        self.ensure_positions_for_new_nodes();
    }

    fn trigger_auto_layout(&mut self) {
        self.compute_auto_layout();
    }

    fn move_node(&mut self, node_serial: u64, pos: Point) {
        self.node_positions.insert(node_serial, pos);
        self.resolve_node_overlaps(Some(node_serial));
    }
}

// ============================================================================
// EMPIRICAL CHALLENGE TESTS
// ============================================================================

/// Verifies bit-for-bit invariance helper: checks both f32 equality and bitwise equality.
fn assert_bit_for_bit_identical(
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
            "{context}: Node {serial} x coordinate changed from {} ({:#x}) to {} ({:#x})!",
            base_pos.x,
            base_pos.x.to_bits(),
            curr_pos.x,
            curr_pos.x.to_bits()
        );
        assert_eq!(
            curr_pos.y.to_bits(),
            base_pos.y.to_bits(),
            "{context}: Node {serial} y coordinate changed from {} ({:#x}) to {} ({:#x})!",
            base_pos.y,
            base_pos.y.to_bits(),
            curr_pos.y,
            curr_pos.y.to_bits()
        );
    }
}

/// Verifies that no two nodes overlap each other.
fn assert_no_node_overlaps(positions: &BTreeMap<u64, Point>, context: &str) {
    let list: Vec<(u64, Point)> = positions.iter().map(|(&s, &p)| (s, p)).collect();
    for i in 0..list.len() {
        for j in (i + 1)..list.len() {
            let (s1, p1) = list[i];
            let (s2, p2) = list[j];
            let dx = (p1.x - p2.x).abs();
            let dy = (p1.y - p2.y).abs();
            assert!(
                dx >= NODE_WIDTH + 12.0 || dy >= NODE_HEIGHT + 12.0,
                "{context}: Nodes {s1} ({p1:?}) and {s2} ({p2:?}) overlap! dx={dx:.1}, dy={dy:.1}",
            );
        }
    }
}

/// Verifies that no node intersects crossing link curves it is not part of.
fn assert_no_node_link_curve_collisions(
    graph: &PwGraph,
    positions: &BTreeMap<u64, Point>,
    context: &str,
) {
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
                "{context}: Node {serial} at {pos:?} collides with link curve from {u} to {v}!"
            );
        }
    }
}

// ============================================================================
// CHALLENGE 1: Dynamic Addition of 10 Nodes One by One
// ============================================================================

#[test]
fn test_challenge_dynamic_addition_of_10_nodes_step_by_step_invariance() {
    let demo_graph = create_demo_graph();
    let mut app = SimulatedPipewireApp::new(demo_graph);

    // Initial startup: positions empty; calling ensure_positions_for_new_nodes runs full layout
    assert!(app.node_positions.is_empty());
    app.ensure_positions_for_new_nodes();
    assert_eq!(app.node_positions.len(), 6);

    let baseline_positions = app.node_positions.clone();
    assert_no_node_overlaps(&baseline_positions, "Baseline startup");
    assert_no_node_link_curve_collisions(&app.graph, &baseline_positions, "Baseline startup");

    // Define 10 distinct nodes to add dynamically one by one
    let new_nodes_specs = vec![
        // 1. Unconnected audio input stream (e.g., Discord mic input)
        (101, "Zoom Voice In", NodeCategory::AudioInputStream, 1, 0, None),
        // 2. Connected downstream audio sink fed by Headphones (serial 50)
        (102, "USB Audio DAC", NodeCategory::AudioSink, 2, 0, Some((50, 5000, 102, 102000))),
        // 3. Audio source feeding into EasyEffects (serial 40)
        (103, "Podcasting Mic", NodeCategory::AudioSource, 0, 1, Some((103, 103000, 40, 4000))),
        // 4. Audio duplex filter between Spotify (30) and Headphones (50)
        (104, "Equalizer APO", NodeCategory::AudioDuplex, 1, 1, Some((30, 3000, 104, 104000))),
        // 5. MIDI keyboard controller
        (105, "Arturia MiniLab", NodeCategory::Midi, 0, 1, None),
        // 6. Video screen capture stream
        (106, "OBS Screen Capture", NodeCategory::Video, 0, 1, None),
        // 7. Internal PipeWire dummy driver
        (107, "PipeWire Dummy Driver", NodeCategory::Internal, 0, 0, None),
        // 8. Audio output stream (media player)
        (108, "VLC Media Player", NodeCategory::AudioOutputStream, 0, 2, None),
        // 9. Pure unconnected audio sink
        (109, "Virtual Cable Sink", NodeCategory::AudioSink, 2, 0, None),
        // 10. Multi-channel audio source
        (110, "Focusrite Scarlett 18i20", NodeCategory::AudioSource, 0, 8, None),
    ];

    let mut link_counter = 50000;

    for (step_idx, (serial, name, category, in_ports, out_ports, maybe_link)) in
        new_nodes_specs.into_iter().enumerate()
    {
        // Snapshot all currently placed nodes before adding the new node
        let pre_existing_snapshot = app.node_positions.clone();

        // 1. Add node to graph
        let node = make_test_node(serial, name, category, in_ports, out_ports);
        for port in &node.input_ports {
            app.graph.ports.insert(port.object_serial, port.clone());
        }
        for port in &node.output_ports {
            app.graph.ports.insert(port.object_serial, port.clone());
        }
        app.graph.nodes.insert(serial, node);

        // Optionally add link
        if let Some((out_node, out_port, in_node, in_port)) = maybe_link {
            link_counter += 1;
            let link = make_test_link(link_counter, out_node, out_port, in_node, in_port);
            app.graph.links.insert(link_counter, link);
        }

        // 2. Call ensure_positions_for_new_nodes
        app.ensure_positions_for_new_nodes();

        // 3. EMPIRICAL VERIFICATION 1: Every pre-existing node maintains identical coordinates BIT-FOR-BIT
        assert_bit_for_bit_identical(
            &app.node_positions,
            &pre_existing_snapshot,
            &format!("Step {} (Adding Node {})", step_idx + 1, serial),
        );

        // 4. EMPIRICAL VERIFICATION 2: The newly added node is assigned a valid position
        assert!(
            app.node_positions.contains_key(&serial),
            "Step {}: Node {} was not assigned a position!",
            step_idx + 1,
            serial
        );
        let new_node_pos = app.node_positions[&serial];

        // 5. EMPIRICAL VERIFICATION 3: Newly placed node NEVER overlaps ANY existing node
        for (&existing_serial, &existing_pos) in &pre_existing_snapshot {
            let dx = (existing_pos.x - new_node_pos.x).abs();
            let dy = (existing_pos.y - new_node_pos.y).abs();
            assert!(
                dx >= NODE_WIDTH + 12.0 || dy >= NODE_HEIGHT + 12.0,
                "Step {}: Newly added node {} ({new_node_pos:?}) overlaps existing node {} ({existing_pos:?})! dx={dx:.1}, dy={dy:.1}",
                step_idx + 1,
                serial,
                existing_serial
            );
        }

        // 6. EMPIRICAL VERIFICATION 4: No collision with existing link curves
        assert_no_node_link_curve_collisions(
            &app.graph,
            &app.node_positions,
            &format!("Step {} (After adding Node {})", step_idx + 1, serial),
        );
    }

    // Verify all 16 nodes are positioned and 100% collision/overlap free
    assert_eq!(app.node_positions.len(), 16);
    assert_no_node_overlaps(&app.node_positions, "Final 16-node state");
    assert_no_node_link_curve_collisions(&app.graph, &app.node_positions, "Final 16-node state");
}

// ============================================================================
// CHALLENGE 2: Dynamic Removal of 5 Nodes
// ============================================================================

#[test]
fn test_challenge_dynamic_removal_of_5_nodes_step_by_step_invariance() {
    let demo_graph = create_demo_graph();
    let mut app = SimulatedPipewireApp::new(demo_graph);
    app.ensure_positions_for_new_nodes();

    // First, dynamically add 5 nodes so we have an extended graph
    for id in [101, 102, 103, 104, 105] {
        let node = make_test_node(id, &format!("Node {id}"), NodeCategory::AudioDuplex, 1, 1);
        app.graph.nodes.insert(id, node);
    }
    app.ensure_positions_for_new_nodes();
    assert_eq!(app.node_positions.len(), 11);

    // List 5 nodes to sequentially remove: a mix of original demo nodes and newly added nodes
    let nodes_to_remove = vec![105, 102, 40, 103, 20]; // 40=EasyEffects, 20=Firefox

    for (step_idx, serial_to_remove) in nodes_to_remove.into_iter().enumerate() {
        // Record all remaining node positions before removal
        let mut expected_remaining = app.node_positions.clone();
        expected_remaining.remove(&serial_to_remove);

        // 1. Remove node from graph (and any associated links)
        app.graph.nodes.remove(&serial_to_remove);
        app.graph.links.retain(|_, link| {
            link.output_node_serial != serial_to_remove && link.input_node_serial != serial_to_remove
        });

        // 2. Call ensure_positions_for_new_nodes
        app.ensure_positions_for_new_nodes();

        // 3. EMPIRICAL VERIFICATION 1: Removed node is pruned from node_positions
        assert!(
            !app.node_positions.contains_key(&serial_to_remove),
            "Step {}: Removed node {} must be pruned from node_positions!",
            step_idx + 1,
            serial_to_remove
        );

        // 4. EMPIRICAL VERIFICATION 2: All remaining nodes maintain identical coordinates BIT-FOR-BIT
        assert_bit_for_bit_identical(
            &app.node_positions,
            &expected_remaining,
            &format!("Step {} (After removing Node {})", step_idx + 1, serial_to_remove),
        );
        assert_eq!(
            app.node_positions.len(),
            expected_remaining.len(),
            "Positions count must match exactly"
        );
    }

    assert_eq!(app.node_positions.len(), 6);
    assert_no_node_overlaps(&app.node_positions, "After removing 5 nodes");
}

// ============================================================================
// CHALLENGE 3: Parameter Changes (Volume / Mute) Do NOT Move Node Positions
// ============================================================================

#[test]
fn test_challenge_volume_and_mute_changes_preserve_positions_bit_for_bit() {
    let demo_graph = create_demo_graph();
    let mut app = SimulatedPipewireApp::new(demo_graph);
    app.ensure_positions_for_new_nodes();

    let baseline = app.node_positions.clone();

    // 1. Master volume changes across multiple nodes
    for &(serial, vol) in &[(10u64, 0.2f32), (20u64, 0.75f32), (30u64, 0.0f32), (50u64, 1.5f32)] {
        app.change_volume(serial, vol);
        assert_bit_for_bit_identical(
            &app.node_positions,
            &baseline,
            &format!("After change_volume({serial}, {vol})"),
        );
    }

    // 2. Mute toggles across multiple nodes
    for &(serial, muted) in &[(10u64, true), (30u64, true), (50u64, false), (10u64, false)] {
        app.toggle_mute(serial, muted);
        assert_bit_for_bit_identical(
            &app.node_positions,
            &baseline,
            &format!("After toggle_mute({serial}, {muted})"),
        );
    }

    // 3. Per-channel volume adjustments
    for channel in 0..2 {
        app.change_channel_volume(20, channel, 0.45);
        app.change_channel_volume(50, channel, 0.88);
        assert_bit_for_bit_identical(
            &app.node_positions,
            &baseline,
            &format!("After change_channel_volume for channel {channel}"),
        );
    }

    // 4. Call ensure_positions_for_new_nodes after all mutations
    app.ensure_positions_for_new_nodes();
    assert_bit_for_bit_identical(
        &app.node_positions,
        &baseline,
        "After ensure_positions_for_new_nodes post-volume/mute mutations",
    );
}

// ============================================================================
// CHALLENGE 4: Category Filter Toggles Back and Forth
// ============================================================================

#[test]
fn test_challenge_category_filter_toggles_preserve_positions_bit_for_bit() {
    let demo_graph = create_demo_graph();
    let mut app = SimulatedPipewireApp::new(demo_graph);

    // Add MIDI and Video nodes to test filtering across multiple categories
    let midi_node = make_test_node(201, "MIDI Synth", NodeCategory::Midi, 1, 1);
    let video_node = make_test_node(202, "Webcam", NodeCategory::Video, 0, 1);
    let internal_node = make_test_node(203, "PulseAudio Server", NodeCategory::Internal, 0, 0);

    app.graph.nodes.insert(201, midi_node);
    app.graph.nodes.insert(202, video_node);
    app.graph.nodes.insert(203, internal_node);

    // Initial startup layout
    app.ensure_positions_for_new_nodes();

    // Enable all filters so all nodes are positioned initially
    app.toggle_filter_midi(true);
    app.toggle_filter_video(true);
    app.toggle_filter_internal(true);

    assert_eq!(app.node_positions.len(), 9);
    let all_positioned_baseline = app.node_positions.clone();

    // 1. Toggle Audio filter off and on
    app.toggle_filter_audio(false);
    assert_bit_for_bit_identical(
        &app.node_positions,
        &all_positioned_baseline,
        "Audio filter OFF must preserve positions",
    );

    app.toggle_filter_audio(true);
    assert_bit_for_bit_identical(
        &app.node_positions,
        &all_positioned_baseline,
        "Audio filter ON must preserve positions",
    );

    // 2. Toggle MIDI filter off and on
    app.toggle_filter_midi(false);
    assert_bit_for_bit_identical(
        &app.node_positions,
        &all_positioned_baseline,
        "MIDI filter OFF must preserve positions",
    );

    app.toggle_filter_midi(true);
    assert_bit_for_bit_identical(
        &app.node_positions,
        &all_positioned_baseline,
        "MIDI filter ON must preserve positions",
    );

    // 3. Toggle Video filter off and on
    app.toggle_filter_video(false);
    assert_bit_for_bit_identical(
        &app.node_positions,
        &all_positioned_baseline,
        "Video filter OFF must preserve positions",
    );

    app.toggle_filter_video(true);
    assert_bit_for_bit_identical(
        &app.node_positions,
        &all_positioned_baseline,
        "Video filter ON must preserve positions",
    );

    // 4. Toggle Internal filter off and on
    app.toggle_filter_internal(false);
    assert_bit_for_bit_identical(
        &app.node_positions,
        &all_positioned_baseline,
        "Internal filter OFF must preserve positions",
    );

    app.toggle_filter_internal(true);
    assert_bit_for_bit_identical(
        &app.node_positions,
        &all_positioned_baseline,
        "Internal filter ON must preserve positions",
    );

    // 5. Stress test: 50 cycles of pseudorandom filter toggles
    let mut rng = 0x12345678u32;
    for cycle in 0..50 {
        rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
        let bit = rng >> 16;
        app.toggle_filter_audio((bit & 1) != 0);
        app.toggle_filter_midi((bit & 2) != 0);
        app.toggle_filter_video((bit & 4) != 0);
        app.toggle_filter_internal((bit & 8) != 0);

        assert_bit_for_bit_identical(
            &app.node_positions,
            &all_positioned_baseline,
            &format!("Random filter toggle cycle {cycle}"),
        );
    }
}

// ============================================================================
// CHALLENGE 5: Pressing "Auto-Layout" Button Resets / Updates Positions
// ============================================================================

#[test]
fn test_challenge_auto_layout_button_resets_and_updates_positions() {
    let demo_graph = create_demo_graph();
    let mut app = SimulatedPipewireApp::new(demo_graph);
    app.ensure_positions_for_new_nodes();

    let initial_layout = app.node_positions.clone();

    // 1. Manually displace several nodes arbitrarily far away
    app.move_node(10, Point::new(1500.0, 2000.0)); // Mic moved far away
    app.move_node(40, Point::new(50.0, 800.0));    // EasyEffects moved to wrong column
    app.move_node(50, Point::new(10.0, 10.0));     // Headphones moved to origin

    assert_eq!(app.node_positions[&10], Point::new(1500.0, 2000.0));
    assert_eq!(app.node_positions[&40], Point::new(50.0, 800.0));
    assert_eq!(app.node_positions[&50], Point::new(10.0, 10.0));

    // 2. Press "Auto-Layout" button (simulating Message::AutoLayout)
    app.trigger_auto_layout();

    // 3. EMPIRICAL VERIFICATION:
    // Mic (10) must be restored back to Column 0 (x = 40.0)
    assert_eq!(
        app.node_positions[&10].x, 40.0,
        "Auto-Layout must reset Mic (10) back to Column 0 (x = 40.0)!"
    );
    // EasyEffects (40) must be restored back to Column 1 (x = 380.0)
    assert_eq!(
        app.node_positions[&40].x, 380.0,
        "Auto-Layout must reset EasyEffects (40) back to Column 1 (x = 380.0)!"
    );
    // Headphones (50) must be restored back to Column 2 (x = 720.0)
    assert_eq!(
        app.node_positions[&50].x, 720.0,
        "Auto-Layout must reset Headphones (50) back to Column 2 (x = 720.0)!"
    );

    // Positions must be identical to canonical auto-layout positions
    assert_bit_for_bit_identical(
        &app.node_positions,
        &initial_layout,
        "Auto-Layout button must fully restore canonical layout",
    );

    // Verify non-overlap and wire clearance
    assert_no_node_overlaps(&app.node_positions, "Post Auto-Layout reset");
    assert_no_node_link_curve_collisions(&app.graph, &app.node_positions, "Post Auto-Layout reset");
}

// ============================================================================
// CHALLENGE 6: High-Density Stress Test (Adding 30 Nodes In Succession)
// ============================================================================

#[test]
fn test_challenge_high_density_30_node_dynamic_addition_stress() {
    let demo_graph = create_demo_graph();
    let mut app = SimulatedPipewireApp::new(demo_graph);
    app.ensure_positions_for_new_nodes();

    // Dynamically insert 30 nodes in succession across various categories
    for i in 0..30 {
        let serial = 300 + i as u64;
        let category = match i % 5 {
            0 => NodeCategory::AudioSource,
            1 => NodeCategory::AudioInputStream,
            2 => NodeCategory::AudioDuplex,
            3 => NodeCategory::AudioSink,
            _ => NodeCategory::AudioOutputStream,
        };
        let node = make_test_node(serial, &format!("Dense Node {serial}"), category, 1, 1);

        let previous_snapshot = app.node_positions.clone();

        app.graph.nodes.insert(serial, node);
        app.ensure_positions_for_new_nodes();

        // 1. Bit-for-bit check on all previously positioned nodes
        assert_bit_for_bit_identical(
            &app.node_positions,
            &previous_snapshot,
            &format!("30-node stress step {i} (adding {serial})"),
        );

        // 2. Newly added node must not overlap any other node
        let new_pos = app.node_positions[&serial];
        for (&s, &p) in &previous_snapshot {
            let dx = (p.x - new_pos.x).abs();
            let dy = (p.y - new_pos.y).abs();
            assert!(
                dx >= NODE_WIDTH + 12.0 || dy >= NODE_HEIGHT + 12.0,
                "Dense step {i}: node {serial} overlaps node {s}! dx={dx:.1}, dy={dy:.1}"
            );
        }
    }

    assert_eq!(app.node_positions.len(), 36);
    assert_no_node_overlaps(&app.node_positions, "After 30 dense additions");
}

// ============================================================================
// CHALLENGE 7: Free-Spot Placement Respects Signal Flow Direction
// ============================================================================

#[test]
fn test_challenge_free_spot_respects_connected_upstream_and_downstream_columns() {
    let demo_graph = create_demo_graph();
    let mut app = SimulatedPipewireApp::new(demo_graph);
    app.ensure_positions_for_new_nodes();

    // Headphones (50) is at Column 2 (x = 720.0).
    // Adding a node connected downstream of Headphones should be placed at Column 3 (x >= 1060.0).
    let downstream_node = make_test_node(401, "Subwoofer Output", NodeCategory::AudioSink, 1, 0);
    app.graph.nodes.insert(401, downstream_node);
    let link_downstream = make_test_link(60001, 50, 5000, 401, 401000);
    app.graph.links.insert(60001, link_downstream);

    app.ensure_positions_for_new_nodes();
    let spot_downstream = app.node_positions[&401];

    assert!(
        spot_downstream.x >= 1060.0,
        "Node fed by Column 2 must be placed in Column >= 3 (x >= 1060.0), got x = {}",
        spot_downstream.x
    );

    // Spotify (30) is at Column 0 (x = 40.0).
    // Adding a node that feeds INTO Spotify should be placed at Column 0 or sensible position.
    let upstream_feeder = make_test_node(402, "DSP Virtual Synth", NodeCategory::AudioSource, 0, 1);
    app.graph.nodes.insert(402, upstream_feeder);
    let link_upstream = make_test_link(60002, 402, 402000, 30, 3000);
    app.graph.links.insert(60002, link_upstream);

    app.ensure_positions_for_new_nodes();
    let spot_upstream = app.node_positions[&402];

    // Must be placed in a non-overlapping spot
    assert_no_node_overlaps(&app.node_positions, "Upstream and downstream placement");
}

// ============================================================================
// CHALLENGE 8: User-Moved Nodes Retain Custom Coordinates Under Additions
// ============================================================================

#[test]
fn test_challenge_user_moved_nodes_retain_custom_coordinates_under_new_node_additions() {
    let demo_graph = create_demo_graph();
    let mut app = SimulatedPipewireApp::new(demo_graph);
    app.ensure_positions_for_new_nodes();

    // User moves Node 10 to custom location (1234.0, 567.0)
    let custom_pos_10 = Point::new(1234.0, 567.0);
    app.move_node(10, custom_pos_10);
    assert_eq!(app.node_positions[&10], custom_pos_10);

    // Now dynamically add 10 nodes one by one
    for i in 1..=10 {
        let serial = 500 + i;
        let node = make_test_node(serial, &format!("Additional Node {serial}"), NodeCategory::AudioInputStream, 1, 0);
        app.graph.nodes.insert(serial, node);
        app.ensure_positions_for_new_nodes();

        // Node 10 MUST retain custom location bit-for-bit
        assert_eq!(
            app.node_positions[&10].x.to_bits(),
            custom_pos_10.x.to_bits(),
            "User-dragged Node 10 x coordinate must not change after adding node {serial}!"
        );
        assert_eq!(
            app.node_positions[&10].y.to_bits(),
            custom_pos_10.y.to_bits(),
            "User-dragged Node 10 y coordinate must not change after adding node {serial}!"
        );
    }
}

// ============================================================================
// CHALLENGE 9: Full Graph Drain and Reconnection Cycle
// ============================================================================

#[test]
fn test_challenge_full_graph_drain_and_reconnection() {
    let demo_graph = create_demo_graph();
    let mut app = SimulatedPipewireApp::new(demo_graph);
    app.ensure_positions_for_new_nodes();
    assert_eq!(app.node_positions.len(), 6);

    // 1. Drain graph completely (simulating backend disconnect or server shutdown)
    app.graph.nodes.clear();
    app.graph.links.clear();
    app.graph.ports.clear();
    app.ensure_positions_for_new_nodes();

    // node_positions must be pruned to empty
    assert!(
        app.node_positions.is_empty(),
        "Draining all nodes from graph must prune all positions!"
    );

    // 2. Reconnection: demo graph restored
    app.graph = create_demo_graph();
    app.ensure_positions_for_new_nodes();

    // Auto-layout runs cleanly on empty startup
    assert_eq!(app.node_positions.len(), 6);
    assert_no_node_overlaps(&app.node_positions, "Reconnected graph layout");
    assert_no_node_link_curve_collisions(&app.graph, &app.node_positions, "Reconnected graph layout");
}

// ============================================================================
// CHALLENGE 10: Rapid Interleaved Addition and Removal Churn
// ============================================================================

#[test]
fn test_challenge_rapid_interleaved_addition_removal_churn() {
    let demo_graph = create_demo_graph();
    let mut app = SimulatedPipewireApp::new(demo_graph);
    app.ensure_positions_for_new_nodes();

    let mut active_serials: Vec<u64> = vec![10, 20, 30, 40, 50, 60];
    let mut next_serial = 1000u64;

    for step in 0..30 {
        // Interleave additions and removals
        if step % 3 == 0 && active_serials.len() > 3 {
            // Remove a node
            let remove_idx = (step * 7) % active_serials.len();
            let serial_to_remove = active_serials.remove(remove_idx);

            let mut expected_remaining = app.node_positions.clone();
            expected_remaining.remove(&serial_to_remove);

            app.graph.nodes.remove(&serial_to_remove);
            app.graph.links.retain(|_, link| {
                link.output_node_serial != serial_to_remove && link.input_node_serial != serial_to_remove
            });

            app.ensure_positions_for_new_nodes();

            assert_bit_for_bit_identical(
                &app.node_positions,
                &expected_remaining,
                &format!("Churn removal step {step} (removed {serial_to_remove})"),
            );
        } else {
            // Add a node
            next_serial += 1;
            let node = make_test_node(next_serial, &format!("Churn Node {next_serial}"), NodeCategory::AudioDuplex, 1, 1);
            active_serials.push(next_serial);

            let pre_snapshot = app.node_positions.clone();
            app.graph.nodes.insert(next_serial, node);

            app.ensure_positions_for_new_nodes();

            assert_bit_for_bit_identical(
                &app.node_positions,
                &pre_snapshot,
                &format!("Churn addition step {step} (added {next_serial})"),
            );
        }

        assert_no_node_overlaps(&app.node_positions, &format!("Churn step {step} non-overlap"));
    }
}

// ============================================================================
// CHALLENGE 11: 15 Nodes in Same Column Stack Without Overlap
// ============================================================================

#[test]
fn test_challenge_15_nodes_in_same_column_stack_without_overlap() {
    let demo_graph = create_demo_graph();
    let mut app = SimulatedPipewireApp::new(demo_graph);
    app.ensure_positions_for_new_nodes();

    // Add 15 AudioSource nodes (all targeting Column 0)
    for i in 0..15 {
        let serial = 700 + i;
        let node = make_test_node(serial, &format!("Microphone {i}"), NodeCategory::AudioSource, 0, 1);
        let pre_snapshot = app.node_positions.clone();

        app.graph.nodes.insert(serial, node);
        app.ensure_positions_for_new_nodes();

        assert_bit_for_bit_identical(
            &app.node_positions,
            &pre_snapshot,
            &format!("Col 0 stacking step {i} (serial {serial})"),
        );

        let pos = app.node_positions[&serial];
        assert_eq!(pos.x, 40.0, "AudioSource must be placed in Column 0 (x=40.0)");
    }

    assert_no_node_overlaps(&app.node_positions, "15 stacked nodes in Column 0");
}
