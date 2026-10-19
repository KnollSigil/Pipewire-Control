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
    EdgeKey, NodeCategory, PwGraph, PwNode, PwPort,
};
use std::collections::{BTreeMap, HashSet};

#[test]
fn test_m3_matches_search_exhaustive_case_and_fields() {
    let name = "alsa_output.pci-0000_00_1f.3.analog-stereo";
    let desc = "Built-in Audio Analog Stereo Headphones";

    // Whitespace and empty queries
    assert!(matches_search(name, desc, ""));
    assert!(matches_search(name, desc, " "));
    assert!(matches_search(name, desc, "   \t\n  "));

    // Substrings in name
    assert!(matches_search(name, desc, "alsa"));
    assert!(matches_search(name, desc, "ALSA"));
    assert!(matches_search(name, desc, "Alsa"));
    assert!(matches_search(name, desc, "pci-0000"));
    assert!(matches_search(name, desc, "00_1F.3"));
    assert!(matches_search(name, desc, "analog-stereo"));

    // Substrings in description
    assert!(matches_search(name, desc, "built-in"));
    assert!(matches_search(name, desc, "BUILT-IN"));
    assert!(matches_search(name, desc, "Built-In"));
    assert!(matches_search(name, desc, "audio"));
    assert!(matches_search(name, desc, "AUDIO"));
    assert!(matches_search(name, desc, "Headphones"));
    assert!(matches_search(name, desc, "headphones"));

    // Trimming around valid query
    assert!(matches_search(name, desc, "  audio  "));
    assert!(matches_search(name, desc, "\tHeadphones\n"));

    // Negative matches
    assert!(!matches_search(name, desc, "microphone"));
    assert!(!matches_search(name, desc, "spotify"));
    assert!(!matches_search(name, desc, "firefox"));
    assert!(!matches_search(name, desc, "usb"));
}

#[test]
fn test_m3_matches_search_unicode_and_special_characters() {
    let name = "dac_Über_Kopfhörer";
    let desc = "München Studio Monitor — 24bit/96kHz";

    assert!(matches_search(name, desc, "über"));
    assert!(matches_search(name, desc, "ÜBER"));
    assert!(matches_search(name, desc, "kopfhörer"));
    assert!(matches_search(name, desc, "münchen"));
    assert!(matches_search(name, desc, "MÜNCHEN"));
    assert!(matches_search(name, desc, "24bit/96kHz"));
    assert!(matches_search(name, desc, "—"));
    assert!(!matches_search(name, desc, "berlin"));
}

#[test]
fn test_m3_dimmed_nodes_truth_table_on_demo_graph() {
    let graph = create_demo_graph();

    // 1. Inactive search -> empty dimmed set
    let dimmed_empty = compute_dimmed_nodes(&graph, "");
    assert!(dimmed_empty.is_empty());

    let dimmed_whitespace = compute_dimmed_nodes(&graph, "   ");
    assert!(dimmed_whitespace.is_empty());

    // 2. Query matching exactly one node (Spotify, serial 30)
    let dimmed_spotify = compute_dimmed_nodes(&graph, "spotify");
    assert_eq!(dimmed_spotify.len(), graph.nodes.len() - 1);
    assert!(!dimmed_spotify.contains(&30));
    for &serial in graph.nodes.keys() {
        if serial != 30 {
            assert!(dimmed_spotify.contains(&serial));
        }
    }

    // 3. Query matching nothing -> all nodes dimmed
    let dimmed_none = compute_dimmed_nodes(&graph, "completely_nonexistent_token");
    assert_eq!(dimmed_none.len(), graph.nodes.len());
    for &serial in graph.nodes.keys() {
        assert!(dimmed_none.contains(&serial));
    }

    // 4. Query matching multiple nodes
    let dimmed_audio = compute_dimmed_nodes(&graph, "audio");
    for &serial in &dimmed_audio {
        let node = graph.nodes.get(&serial).unwrap();
        assert!(!matches_search(&node.name, &node.description, "audio"));
    }
}

#[test]
fn test_m3_dimmed_links_require_both_endpoints_matching_to_be_active() {
    let graph = create_demo_graph();
    let edges = graph.collapsed_edges();
    assert!(!edges.is_empty());

    // With active query "spotify", only serial 30 matches.
    let dimmed_nodes = compute_dimmed_nodes(&graph, "spotify");
    let dimmed_links = compute_dimmed_links(&graph, &dimmed_nodes, true);

    for edge in &edges {
        let out_matches = !dimmed_nodes.contains(&edge.key.output_node_serial);
        let in_matches = !dimmed_nodes.contains(&edge.key.input_node_serial);

        if out_matches && in_matches {
            assert!(
                !dimmed_links.contains(&edge.key),
                "Link with both matching endpoints must NOT be dimmed: {:?}",
                edge.key
            );
        } else {
            assert!(
                dimmed_links.contains(&edge.key),
                "Link with at least one dimmed endpoint must be dimmed: {:?}",
                edge.key
            );
        }
    }

    // When search is inactive, NO links should be dimmed
    let dimmed_links_inactive = compute_dimmed_links(&graph, &dimmed_nodes, false);
    assert!(dimmed_links_inactive.is_empty());
}

#[test]
fn test_m3_canvas_dimming_opacity_invariants() {
    let graph = create_demo_graph();
    let visible_nodes: HashSet<u64> = graph.nodes.keys().copied().collect();
    let node_positions = BTreeMap::new();

    // Canvas with search query active: "spotify"
    let canvas = GraphCanvas {
        graph: &graph,
        visible_nodes,
        node_positions: &node_positions,
        selected_edge: None,
        selected_node: None,
        selected_nodes: HashSet::new(),
        pan: Vector::ZERO,
        zoom: 1.0,
        layer: CanvasLayer::Base,
        search_query: "spotify",
    };

    assert!(canvas.is_search_active());
    // Serial 30 is Spotify -> not dimmed
    assert!(!canvas.is_node_dimmed(30));
    // Serial 10 is Scarlett Solo Mic -> dimmed
    assert!(canvas.is_node_dimmed(10));
    // Serial 50 is Built-in Audio -> dimmed
    assert!(canvas.is_node_dimmed(50));

    // Link from 30 to 50: output node matches, input node does not -> dimmed!
    assert!(canvas.is_link_dimmed(&EdgeKey {
        output_node_serial: 30,
        input_node_serial: 50,
    }));

    // Inactive search canvas:
    let canvas_inactive = GraphCanvas {
        graph: &graph,
        visible_nodes: canvas.visible_nodes.clone(),
        node_positions: &node_positions,
        selected_edge: None,
        selected_node: None,
        selected_nodes: HashSet::new(),
        pan: Vector::ZERO,
        zoom: 1.0,
        layer: CanvasLayer::Base,
        search_query: "   ",
    };
    assert!(!canvas_inactive.is_search_active());
    assert!(!canvas_inactive.is_node_dimmed(10));
    assert!(!canvas_inactive.is_node_dimmed(30));
    assert!(!canvas_inactive.is_node_dimmed(50));
    assert!(!canvas_inactive.is_link_dimmed(&EdgeKey {
        output_node_serial: 30,
        input_node_serial: 50,
    }));
}
