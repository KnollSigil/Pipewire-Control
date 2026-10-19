#![allow(dead_code, unused_imports, unused_variables)]

#[path = "../src/model.rs"]
mod model;

#[path = "../src/canvas_view.rs"]
mod canvas_view;

use canvas_view::*;
use iced::widget::canvas::{Event, Program};
use iced::{Color, Point, Rectangle, Size, Vector};
use model::{
    compute_dimmed_links, compute_dimmed_nodes, create_demo_graph, is_link_dimmed, matches_search,
    EdgeKey, NodeCategory, PortDirection, PwGraph, PwLink, PwNode, PwPort,
};
use std::collections::{BTreeMap, HashSet};

// ============================================================================
// HELPER: Synthetic Graph Builders
// ============================================================================

fn make_synthetic_node(
    object_serial: u64,
    name: &str,
    description: &str,
    category: NodeCategory,
    in_ports_count: usize,
    out_ports_count: usize,
) -> PwNode {
    let mut in_ports = Vec::new();
    for i in 0..in_ports_count {
        let pid = (object_serial * 100 + i as u64) as u32;
        in_ports.push(PwPort {
            id: pid,
            object_serial: pid as u64,
            node_id: object_serial as u32,
            node_serial: object_serial,
            name: format!("in_{i}"),
            channel: "FL".to_string(),
            direction: PortDirection::Input,
            is_monitor: false,
            props: BTreeMap::new(),
            params: Vec::new(),
        });
    }

    let mut out_ports = Vec::new();
    for i in 0..out_ports_count {
        let pid = (object_serial * 100 + 50 + i as u64) as u32;
        out_ports.push(PwPort {
            id: pid,
            object_serial: pid as u64,
            node_id: object_serial as u32,
            node_serial: object_serial,
            name: format!("out_{i}"),
            channel: "FL".to_string(),
            direction: PortDirection::Output,
            is_monitor: false,
            props: BTreeMap::new(),
            params: Vec::new(),
        });
    }

    PwNode {
        id: object_serial as u32,
        object_serial,
        name: name.to_string(),
        description: description.to_string(),
        app_name: None,
        media_name: None,
        media_class: "Audio/Sink".to_string(),
        category,
        state: "running".to_string(),
        input_ports: in_ports,
        output_ports: out_ports,
        volume: Some(1.0),
        muted: Some(false),
        channel_volumes: vec![1.0],
        channel_names: vec!["FL".to_string()],
        props: BTreeMap::new(),
        params: Vec::new(),
        device_id: None,
        device_serial: None,
    }
}

fn add_synthetic_link(
    graph: &mut PwGraph,
    link_serial: u64,
    out_node_serial: u64,
    in_node_serial: u64,
) {
    let out_node = graph.nodes.get(&out_node_serial).expect("out_node exists");
    let in_node = graph.nodes.get(&in_node_serial).expect("in_node exists");
    let out_port = &out_node.output_ports[0];
    let in_port = &in_node.input_ports[0];

    graph.links.insert(
        link_serial,
        PwLink {
            id: link_serial as u32,
            object_serial: link_serial,
            output_node_id: out_node.id,
            output_node_serial: out_node.object_serial,
            output_port_id: out_port.id,
            output_port_serial: out_port.object_serial,
            input_node_id: in_node.id,
            input_node_serial: in_node.object_serial,
            input_port_id: in_port.id,
            input_port_serial: in_port.object_serial,
            state: "active".to_string(),
            props: BTreeMap::new(),
        },
    );
}

// ============================================================================
// SUITE 1: Exact Set Membership & Mathematical Complement
// ============================================================================

#[test]
fn test_challenge_exact_set_complement_on_demo_graph() {
    let graph = create_demo_graph();
    let all_serials: HashSet<u64> = graph.nodes.keys().copied().collect();
    let total_nodes = all_serials.len();
    assert!(total_nodes >= 6, "Demo graph should have at least 6 nodes");

    let test_queries = [
        // Inactive / blank queries
        ("", true),
        ("   ", true),
        ("\t\n", true),
        (" \r\n ", true),
        // Active queries matching single nodes
        ("spotify", false),
        ("SPOTIFY", false),
        ("sPoTiFy", false),
        ("Focusrite", false),
        ("easyeffects", false),
        ("discord", false),
        ("obs", false),
        ("Firefox", false),
        // Active queries matching multiple nodes
        ("audio", false),
        ("alsa", false),
        ("analog", false),
        ("stereo", false),
        ("pci", false),
        // Substrings exclusively in description
        ("Headphones", false),
        ("Equalizer", false),
        ("Microphone", false),
        ("Speakers", false),
        // Active queries matching zero nodes
        ("nonexistent_node_xyz", false),
        ("!!!$$$###", false),
        ("1234567890_impossible", false),
    ];

    let node_positions = BTreeMap::new();

    for (query, is_inactive) in test_queries {
        let dimmed = compute_dimmed_nodes(&graph, query);

        // Compute ground-truth matching nodes
        let matching: HashSet<u64> = graph
            .nodes
            .iter()
            .filter(|(_, n)| matches_search(&n.name, &n.description, query))
            .map(|(&s, _)| s)
            .collect();

        let canvas = GraphCanvas {
            graph: &graph,
            visible_nodes: all_serials.clone(),
            node_positions: &node_positions,
            selected_edge: None,
            selected_node: None,
            selected_nodes: HashSet::new(),
            pan: Vector::ZERO,
            zoom: 1.0,
            layer: CanvasLayer::Base,
            search_query: query,
            modifiers: iced::keyboard::Modifiers::default(),
        };

        if is_inactive {
            // When query is empty or whitespace, no nodes must be dimmed
            assert!(
                dimmed.is_empty(),
                "Inactive query '{query}' must have 0 dimmed nodes, got {}",
                dimmed.len()
            );
            assert!(
                !canvas.is_search_active(),
                "Canvas must report search inactive for '{query}'"
            );
            for &serial in &all_serials {
                assert!(
                    !canvas.is_node_dimmed(serial),
                    "Node {serial} must not be dimmed on canvas for inactive query '{query}'"
                );
            }
        } else {
            // Active query:
            assert!(
                canvas.is_search_active(),
                "Canvas must report search active for '{query}'"
            );

            // 1. Disjointness: Dimmed ∩ Matching == ∅
            let intersection: HashSet<u64> = dimmed.intersection(&matching).copied().collect();
            assert!(
                intersection.is_empty(),
                "Disjointness violated for query '{query}': {:?} is in both dimmed and matching",
                intersection
            );

            // 2. Completeness / Union: Dimmed ∪ Matching == All Nodes
            let union: HashSet<u64> = dimmed.union(&matching).copied().collect();
            assert_eq!(
                union, all_serials,
                "Union completeness violated for query '{query}'"
            );

            // 3. Exact Cardinality: |Dimmed| + |Matching| == |V|
            assert_eq!(
                dimmed.len() + matching.len(),
                total_nodes,
                "Cardinality violated for query '{query}'"
            );

            // 4. Pointwise exact complement
            for &serial in &all_serials {
                let node = &graph.nodes[&serial];
                let is_match = matches_search(&node.name, &node.description, query);
                let is_dimmed = dimmed.contains(&serial);
                assert_eq!(
                    is_dimmed, !is_match,
                    "Pointwise complement failed for node {} on query '{query}'",
                    serial
                );
                assert_eq!(
                    canvas.is_node_dimmed(serial),
                    is_dimmed,
                    "Canvas node dimming does not match compute_dimmed_nodes for node {serial}"
                );
            }
        }
    }
}

#[test]
fn test_challenge_exact_set_membership_on_synthetic_graphs() {
    // 1. Empty Graph (0 nodes)
    {
        let empty_graph = PwGraph::default();
        assert!(compute_dimmed_nodes(&empty_graph, "").is_empty());
        assert!(compute_dimmed_nodes(&empty_graph, "spotify").is_empty());
        assert!(compute_dimmed_nodes(&empty_graph, "xyz").is_empty());
    }

    // 2. Single-Node Graph
    {
        let mut single_graph = PwGraph::default();
        let n1 = make_synthetic_node(100, "my_synth_node", "Synthesizer Core", NodeCategory::AudioSource, 0, 2);
        single_graph.nodes.insert(n1.object_serial, n1);

        // Matching query
        let dimmed_match = compute_dimmed_nodes(&single_graph, "synth");
        assert!(dimmed_match.is_empty(), "Matching node must not be dimmed");

        // Non-matching query
        let dimmed_mismatch = compute_dimmed_nodes(&single_graph, "guitar");
        assert_eq!(dimmed_mismatch.len(), 1);
        assert!(dimmed_mismatch.contains(&100), "Non-matching node must be in dimmed set");

        // Inactive query
        let dimmed_inactive = compute_dimmed_nodes(&single_graph, "");
        assert!(dimmed_inactive.is_empty(), "Inactive query must not dim single node");
    }

    // 3. Multi-stage Pipeline (Source -> Filter 1 -> Filter 2 -> Sink)
    {
        let mut chain_graph = PwGraph::default();
        let s = make_synthetic_node(1, "source_mic", "USB Vocal Microphone", NodeCategory::AudioSource, 0, 1);
        let f1 = make_synthetic_node(2, "filter_denoise", "RNNoise Suppressor", NodeCategory::AudioSink, 1, 1);
        let f2 = make_synthetic_node(3, "filter_reverb", "Freeverb Spatializer", NodeCategory::AudioSink, 1, 1);
        let k = make_synthetic_node(4, "sink_out", "Studio Reference Monitors", NodeCategory::AudioSink, 1, 0);

        chain_graph.nodes.insert(s.object_serial, s);
        chain_graph.nodes.insert(f1.object_serial, f1);
        chain_graph.nodes.insert(f2.object_serial, f2);
        chain_graph.nodes.insert(k.object_serial, k);

        add_synthetic_link(&mut chain_graph, 101, 1, 2);
        add_synthetic_link(&mut chain_graph, 102, 2, 3);
        add_synthetic_link(&mut chain_graph, 103, 3, 4);

        // Query matching only Source (1)
        let dimmed_s = compute_dimmed_nodes(&chain_graph, "vocal");
        assert_eq!(dimmed_s, HashSet::from([2, 3, 4]));

        // Query matching both Filters (2 and 3)
        let dimmed_f = compute_dimmed_nodes(&chain_graph, "filter_");
        assert_eq!(dimmed_f, HashSet::from([1, 4]));

        // Query matching Sink (4)
        let dimmed_k = compute_dimmed_nodes(&chain_graph, "monitors");
        assert_eq!(dimmed_k, HashSet::from([1, 2, 3]));

        // Query matching All nodes ('e' appears in vocal, rnnoise, freeverb, monitors)
        let dimmed_all = compute_dimmed_nodes(&chain_graph, "e");
        assert!(dimmed_all.is_empty(), "When all match, dimmed set must be empty");

        // Query matching No nodes
        let dimmed_none = compute_dimmed_nodes(&chain_graph, "quantum_flux");
        assert_eq!(dimmed_none, HashSet::from([1, 2, 3, 4]));
    }

    // 4. Large 50-node Synthetic Graph with 16 Diverse Queries
    {
        let mut large_graph = PwGraph::default();
        let prefixes = ["alsa", "pulse", "jack", "bluez", "vban", "stream", "effect", "monitor"];
        let domains = ["gaming", "recording", "streaming", "music", "chat", "daw", "browser"];

        for i in 1..=50 {
            let pref = prefixes[(i * 7) % prefixes.len()];
            let dom = domains[(i * 13) % domains.len()];
            let name = format!("{pref}_{dom}_{i:03}");
            let desc = format!("Device Profile for {dom} on channel {i}");
            let n = make_synthetic_node(i as u64, &name, &desc, NodeCategory::AudioSink, 2, 2);
            large_graph.nodes.insert(n.object_serial, n);
        }

        let random_queries = [
            "alsa", "gaming", "channel 1", "005", "profile for chat",
            "BLUEZ", "NoNmAtChInG", "stream", "daw", "42", "music",
            "   vban   ", "channel 5", "recording", "unknown", "effect",
        ];

        for q in random_queries {
            let dimmed = compute_dimmed_nodes(&large_graph, q);
            let trimmed = q.trim();

            for (&serial, node) in &large_graph.nodes {
                let matches = matches_search(&node.name, &node.description, trimmed);
                assert_eq!(
                    dimmed.contains(&serial),
                    !matches,
                    "Exact complement property failed for node {serial} on query '{q}'"
                );
            }

            let matching_count = large_graph
                .nodes
                .values()
                .filter(|n| matches_search(&n.name, &n.description, trimmed))
                .count();

            assert_eq!(
                dimmed.len() + matching_count,
                50,
                "Partition cardinality must strictly sum to 50"
            );
        }
    }
}

// ============================================================================
// SUITE 2: Link Dimming Logic & Endpoints Invariant
// ============================================================================

#[test]
fn test_challenge_link_dimming_truth_table_exhaustive() {
    // Construct a diamond test graph with exactly 4 nodes:
    // Node 1 (Source A, matches query "match")
    // Node 2 (Source B, mismatches query "match")
    // Node 3 (Sink C, matches query "match")
    // Node 4 (Sink D, mismatches query "match")
    let mut graph = PwGraph::default();

    let n1 = make_synthetic_node(1, "node_match_source", "Matches query", NodeCategory::AudioSource, 0, 2);
    let n2 = make_synthetic_node(2, "node_other_source", "Unrelated source", NodeCategory::AudioSource, 0, 2);
    let n3 = make_synthetic_node(3, "node_match_sink", "Matches query", NodeCategory::AudioSink, 2, 0);
    let n4 = make_synthetic_node(4, "node_other_sink", "Unrelated sink", NodeCategory::AudioSink, 2, 0);

    graph.nodes.insert(n1.object_serial, n1);
    graph.nodes.insert(n2.object_serial, n2);
    graph.nodes.insert(n3.object_serial, n3);
    graph.nodes.insert(n4.object_serial, n4);

    // Links:
    // Link 1: 1 -> 3 (Match -> Match)
    // Link 2: 1 -> 4 (Match -> Mismatch)
    // Link 3: 2 -> 3 (Mismatch -> Match)
    // Link 4: 2 -> 4 (Mismatch -> Mismatch)
    add_synthetic_link(&mut graph, 10, 1, 3);
    add_synthetic_link(&mut graph, 20, 1, 4);
    add_synthetic_link(&mut graph, 30, 2, 3);
    add_synthetic_link(&mut graph, 40, 2, 4);

    let edge_match_match = EdgeKey { output_node_serial: 1, input_node_serial: 3 };
    let edge_match_mismatch = EdgeKey { output_node_serial: 1, input_node_serial: 4 };
    let edge_mismatch_match = EdgeKey { output_node_serial: 2, input_node_serial: 3 };
    let edge_mismatch_mismatch = EdgeKey { output_node_serial: 2, input_node_serial: 4 };

    // ------------------------------------------------------------------------
    // Part A: Active Search ("match")
    // ------------------------------------------------------------------------
    let query_active = "match";
    let dimmed_nodes = compute_dimmed_nodes(&graph, query_active);

    // Verify node matching states
    assert!(!dimmed_nodes.contains(&1), "Node 1 matches query");
    assert!(dimmed_nodes.contains(&2), "Node 2 does not match");
    assert!(!dimmed_nodes.contains(&3), "Node 3 matches query");
    assert!(dimmed_nodes.contains(&4), "Node 4 does not match");

    let dimmed_links = compute_dimmed_links(&graph, &dimmed_nodes, true);

    // 1. (Match -> Match): NEITHER endpoint dimmed => Link MUST NOT be dimmed (Active)
    assert!(
        !is_link_dimmed(&edge_match_match, &dimmed_nodes, true),
        "Link between two matching nodes must NOT be dimmed via is_link_dimmed"
    );
    assert!(
        !dimmed_links.contains(&edge_match_match),
        "Link between two matching nodes must NOT be in compute_dimmed_links"
    );

    // 2. (Match -> Mismatch): Input is dimmed => Link MUST be dimmed
    assert!(
        is_link_dimmed(&edge_match_mismatch, &dimmed_nodes, true),
        "Link with dimmed input must be dimmed via is_link_dimmed"
    );
    assert!(
        dimmed_links.contains(&edge_match_mismatch),
        "Link with dimmed input must be in compute_dimmed_links"
    );

    // 3. (Mismatch -> Match): Output is dimmed => Link MUST be dimmed
    assert!(
        is_link_dimmed(&edge_mismatch_match, &dimmed_nodes, true),
        "Link with dimmed output must be dimmed via is_link_dimmed"
    );
    assert!(
        dimmed_links.contains(&edge_mismatch_match),
        "Link with dimmed output must be in compute_dimmed_links"
    );

    // 4. (Mismatch -> Mismatch): Both endpoints dimmed => Link MUST be dimmed
    assert!(
        is_link_dimmed(&edge_mismatch_mismatch, &dimmed_nodes, true),
        "Link with both endpoints dimmed must be dimmed via is_link_dimmed"
    );
    assert!(
        dimmed_links.contains(&edge_mismatch_mismatch),
        "Link with both endpoints dimmed must be in compute_dimmed_links"
    );

    // Check GraphCanvas helpers consistency under active search
    let node_positions = BTreeMap::new();
    let canvas = GraphCanvas {
        graph: &graph,
        visible_nodes: HashSet::from([1, 2, 3, 4]),
        node_positions: &node_positions,
        selected_edge: None,
        selected_node: None,
        selected_nodes: HashSet::new(),
        pan: Vector::ZERO,
        zoom: 1.0,
        layer: CanvasLayer::Base,
        search_query: query_active,
        modifiers: iced::keyboard::Modifiers::default(),
    };

    assert!(canvas.is_search_active());
    assert!(!canvas.is_node_dimmed(1));
    assert!(canvas.is_node_dimmed(2));
    assert!(!canvas.is_node_dimmed(3));
    assert!(canvas.is_node_dimmed(4));

    assert!(!canvas.is_link_dimmed(&edge_match_match));
    assert!(canvas.is_link_dimmed(&edge_match_mismatch));
    assert!(canvas.is_link_dimmed(&edge_mismatch_match));
    assert!(canvas.is_link_dimmed(&edge_mismatch_mismatch));

    // ------------------------------------------------------------------------
    // Part B: Inactive Search ("" or "   ")
    // ------------------------------------------------------------------------
    for inactive_query in ["", "   ", "\t"] {
        let dimmed_nodes_inactive = compute_dimmed_nodes(&graph, inactive_query);
        assert!(dimmed_nodes_inactive.is_empty());

        let dimmed_links_inactive = compute_dimmed_links(&graph, &dimmed_nodes_inactive, false);
        assert!(
            dimmed_links_inactive.is_empty(),
            "Inactive search must have ZERO dimmed links"
        );

        // Every link must return false for is_link_dimmed
        assert!(!is_link_dimmed(&edge_match_match, &dimmed_nodes_inactive, false));
        assert!(!is_link_dimmed(&edge_match_mismatch, &dimmed_nodes_inactive, false));
        assert!(!is_link_dimmed(&edge_mismatch_match, &dimmed_nodes_inactive, false));
        assert!(!is_link_dimmed(&edge_mismatch_mismatch, &dimmed_nodes_inactive, false));

        let canvas_inactive = GraphCanvas {
            graph: &graph,
            visible_nodes: HashSet::from([1, 2, 3, 4]),
            node_positions: &node_positions,
            selected_edge: None,
            selected_node: None,
            selected_nodes: HashSet::new(),
            pan: Vector::ZERO,
            zoom: 1.0,
            layer: CanvasLayer::Base,
            search_query: inactive_query,
            modifiers: iced::keyboard::Modifiers::default(),
        };

        assert!(!canvas_inactive.is_search_active());
        assert!(!canvas_inactive.is_node_dimmed(1));
        assert!(!canvas_inactive.is_node_dimmed(2));
        assert!(!canvas_inactive.is_node_dimmed(3));
        assert!(!canvas_inactive.is_node_dimmed(4));

        assert!(!canvas_inactive.is_link_dimmed(&edge_match_match));
        assert!(!canvas_inactive.is_link_dimmed(&edge_match_mismatch));
        assert!(!canvas_inactive.is_link_dimmed(&edge_mismatch_match));
        assert!(!canvas_inactive.is_link_dimmed(&edge_mismatch_mismatch));
    }
}

#[test]
fn test_challenge_link_dimming_on_demo_graph_all_queries() {
    let graph = create_demo_graph();
    let edges = graph.collapsed_edges();
    assert!(!edges.is_empty());

    let queries = [
        "",
        "   ",
        "spotify",
        "audio",
        "easyeffects",
        "Focusrite",
        "pci",
        "nonexistent",
    ];

    let node_positions = BTreeMap::new();

    for q in queries {
        let dimmed_nodes = compute_dimmed_nodes(&graph, q);
        let search_active = !q.trim().is_empty();
        let dimmed_links = compute_dimmed_links(&graph, &dimmed_nodes, search_active);

        let canvas = GraphCanvas {
            graph: &graph,
            visible_nodes: graph.nodes.keys().copied().collect(),
            node_positions: &node_positions,
            selected_edge: None,
            selected_node: None,
            selected_nodes: HashSet::new(),
            pan: Vector::ZERO,
            zoom: 1.0,
            layer: CanvasLayer::Base,
            search_query: q,
            modifiers: iced::keyboard::Modifiers::default(),
        };

        for edge in &edges {
            let out_dimmed = dimmed_nodes.contains(&edge.key.output_node_serial);
            let in_dimmed = dimmed_nodes.contains(&edge.key.input_node_serial);
            let expected_dimmed = search_active && (out_dimmed || in_dimmed);

            assert_eq!(
                dimmed_links.contains(&edge.key),
                expected_dimmed,
                "Link {:?} dimming failure for query '{q}'",
                edge.key
            );

            assert_eq!(
                is_link_dimmed(&edge.key, &dimmed_nodes, search_active),
                expected_dimmed,
                "is_link_dimmed mismatch for link {:?} on query '{q}'",
                edge.key
            );

            assert_eq!(
                canvas.is_link_dimmed(&edge.key),
                expected_dimmed,
                "canvas.is_link_dimmed mismatch for link {:?} on query '{q}'",
                edge.key
            );
        }
    }
}

// ============================================================================
// SUITE 3: Clear Action State Invariant & Opacity Restoration
// ============================================================================

#[test]
fn test_challenge_clear_action_restores_complete_graph_opacity() {
    let graph = create_demo_graph();
    let visible_nodes: HashSet<u64> = graph.nodes.keys().copied().collect();
    let node_positions = BTreeMap::new();

    // Simulated search state holder (mirroring PipewireApp::search_query)
    let mut search_query = String::new();

    // 1. Initial State: search_query is empty
    let canvas_initial = GraphCanvas {
        graph: &graph,
        visible_nodes: visible_nodes.clone(),
        node_positions: &node_positions,
        selected_edge: None,
        selected_node: None,
        selected_nodes: HashSet::new(),
        pan: Vector::ZERO,
        zoom: 1.0,
        layer: CanvasLayer::Base,
        search_query: &search_query,
        modifiers: iced::keyboard::Modifiers::default(),
    };

    assert!(!canvas_initial.is_search_active());
    for &s in &visible_nodes {
        assert!(!canvas_initial.is_node_dimmed(s));
    }
    for e in graph.collapsed_edges() {
        assert!(!canvas_initial.is_link_dimmed(&e.key));
    }

    // 2. Active Search Query Applied
    search_query = "spotify".to_string();
    let canvas_active = GraphCanvas {
        graph: &graph,
        visible_nodes: visible_nodes.clone(),
        node_positions: &node_positions,
        selected_edge: None,
        selected_node: None,
        selected_nodes: HashSet::new(),
        pan: Vector::ZERO,
        zoom: 1.0,
        layer: CanvasLayer::Base,
        search_query: &search_query,
        modifiers: iced::keyboard::Modifiers::default(),
    };

    assert!(canvas_active.is_search_active());
    assert!(!canvas_active.is_node_dimmed(30)); // Spotify is not dimmed
    assert!(canvas_active.is_node_dimmed(10));  // Microphone is dimmed
    assert!(canvas_active.is_node_dimmed(20));  // Firefox is dimmed

    // 3. Clear Action Triggered: equivalent to on_press(Message::SearchQueryChanged(String::new()))
    search_query.clear(); // search_query = String::new();

    let canvas_cleared = GraphCanvas {
        graph: &graph,
        visible_nodes: visible_nodes.clone(),
        node_positions: &node_positions,
        selected_edge: None,
        selected_node: None,
        selected_nodes: HashSet::new(),
        pan: Vector::ZERO,
        zoom: 1.0,
        layer: CanvasLayer::Base,
        search_query: &search_query,
        modifiers: iced::keyboard::Modifiers::default(),
    };

    // 4. Invariant Verification: 100% restored to initial state
    assert_eq!(search_query, "");
    assert!(
        !canvas_cleared.is_search_active(),
        "Search must be inactive after clear"
    );

    let dimmed_nodes_cleared = compute_dimmed_nodes(&graph, &search_query);
    assert!(
        dimmed_nodes_cleared.is_empty(),
        "Dimmed nodes must be strictly empty after clear"
    );

    let dimmed_links_cleared = compute_dimmed_links(&graph, &dimmed_nodes_cleared, false);
    assert!(
        dimmed_links_cleared.is_empty(),
        "Dimmed links must be strictly empty after clear"
    );

    for &serial in &visible_nodes {
        assert!(
            !canvas_cleared.is_node_dimmed(serial),
            "Node {serial} must NOT be dimmed after clear"
        );
    }

    for edge in graph.collapsed_edges() {
        assert!(
            !canvas_cleared.is_link_dimmed(&edge.key),
            "Link {:?} must NOT be dimmed after clear",
            edge.key
        );
    }
}

#[test]
fn test_challenge_rapid_toggle_and_repeated_clear_stress() {
    let graph = create_demo_graph();
    let visible_nodes: HashSet<u64> = graph.nodes.keys().copied().collect();
    let node_positions = BTreeMap::new();

    let queries = [
        "spotify",
        "audio",
        "Focusrite",
        "NONEXISTENT_xyz_123",
        "   ",
        "alsa_output",
        "discord",
        "OBS",
        "stereo",
        "easyeffects",
        "123",
        "headphones",
        "Firefox",
    ];

    let mut query_buf;

    for cycle in 0..100 {
        let q = queries[cycle % queries.len()];

        // Phase 1: Set query
        query_buf = q.to_string();
        let search_active = !query_buf.trim().is_empty();
        let dimmed_nodes = compute_dimmed_nodes(&graph, &query_buf);

        if search_active {
            let matching_count = graph
                .nodes
                .values()
                .filter(|n| matches_search(&n.name, &n.description, query_buf.trim()))
                .count();
            assert_eq!(
                dimmed_nodes.len() + matching_count,
                graph.nodes.len(),
                "Cycle {cycle}: Partition cardinality check failed during active query"
            );
        } else {
            assert!(
                dimmed_nodes.is_empty(),
                "Cycle {cycle}: Inactive query must have 0 dimmed nodes"
            );
        }

        // Phase 2: Clear query
        query_buf.clear(); // on_press clear action

        let cleared_dimmed_nodes = compute_dimmed_nodes(&graph, &query_buf);
        assert!(
            cleared_dimmed_nodes.is_empty(),
            "Cycle {cycle}: Cleared dimmed nodes must be empty"
        );

        let canvas = GraphCanvas {
            graph: &graph,
            visible_nodes: visible_nodes.clone(),
            node_positions: &node_positions,
            selected_edge: None,
            selected_node: None,
            selected_nodes: HashSet::new(),
            pan: Vector::ZERO,
            zoom: 1.0,
            layer: CanvasLayer::Base,
            search_query: &query_buf,
            modifiers: iced::keyboard::Modifiers::default(),
        };

        assert!(
            !canvas.is_search_active(),
            "Cycle {cycle}: Canvas search must be inactive after clear"
        );
        for &s in &visible_nodes {
            assert!(
                !canvas.is_node_dimmed(s),
                "Cycle {cycle}: Node {s} must not be dimmed after clear"
            );
        }
        for edge in graph.collapsed_edges() {
            assert!(
                !canvas.is_link_dimmed(&edge.key),
                "Cycle {cycle}: Edge {:?} must not be dimmed after clear",
                edge.key
            );
        }
    }
}
