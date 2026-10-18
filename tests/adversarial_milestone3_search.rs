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
// Deterministic PRNG for Reproducible Stress Testing
// ============================================================================
struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 0xCAFE_BABE_DEAD_BEEF } else { seed },
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn next_range(&mut self, min: usize, max: usize) -> usize {
        assert!(max >= min);
        if max == min {
            return min;
        }
        let span = max - min + 1;
        min + (self.next_u64() as usize % span)
    }

    fn next_char(&mut self) -> char {
        const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_./\\ \t\n!@#$%^&*()[]{}|;:'\",<>?";
        let idx = (self.next_u64() as usize) % CHARS.len();
        CHARS[idx] as char
    }

    fn next_string(&mut self, len: usize) -> String {
        let mut s = String::with_capacity(len);
        for _ in 0..len {
            s.push(self.next_char());
        }
        s
    }
}

// ============================================================================
// HELPER: Synthetic Graph Builder with Complete Fields
// ============================================================================
fn build_synthetic_node(
    serial: u64,
    name: &str,
    desc: &str,
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
            name: format!("playback_{}", i + 1),
            direction: PortDirection::Input,
            channel: format!("FL_{}", i),
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
            name: format!("capture_{}", i + 1),
            direction: PortDirection::Output,
            channel: format!("FR_{}", i),
            is_monitor: false,
            props: BTreeMap::new(),
        });
    }

    PwNode {
        id: serial as u32,
        object_serial: serial,
        name: name.to_string(),
        description: desc.to_string(),
        app_name: None,
        media_name: None,
        media_class: "Audio/Sink".to_string(),
        category,
        input_ports,
        output_ports,
        state: "running".to_string(),
        volume: Some(1.0),
        muted: Some(false),
        channel_names: vec!["FL".to_string(), "FR".to_string()],
        channel_volumes: vec![1.0, 1.0],
        props: BTreeMap::new(),
        params: Vec::new(),
    }
}

fn link_nodes(graph: &mut PwGraph, link_serial: u64, out_node: &PwNode, in_node: &PwNode) {
    assert!(!out_node.output_ports.is_empty(), "Source node must have output port");
    assert!(!in_node.input_ports.is_empty(), "Sink node must have input port");
    let out_p = &out_node.output_ports[0];
    let in_p = &in_node.input_ports[0];

    graph.links.insert(
        link_serial,
        PwLink {
            id: link_serial as u32,
            object_serial: link_serial,
            output_node_id: out_node.id,
            output_node_serial: out_node.object_serial,
            output_port_id: out_p.id,
            output_port_serial: out_p.object_serial,
            input_node_id: in_node.id,
            input_node_serial: in_node.object_serial,
            input_port_id: in_p.id,
            input_port_serial: in_p.object_serial,
            state: "active".to_string(),
            props: BTreeMap::new(),
        },
    );
}

// ============================================================================
// SUITE 1: Adversarial Search Query Inputs
// ============================================================================

#[test]
fn test_adversarial_search_regex_and_metacharacters() {
    let name = "alsa_output.pci-0000_00_1f.3";
    let desc = "Built-in Audio Analog Stereo (Realtek ALC892)";

    let regex_payloads = [
        ".*",
        "^.*$",
        "[a-z]+",
        "\\d+",
        "\\w+",
        "(audio|alsa)",
        "^alsa",
        "3$",
        "\\bAnalog\\b",
        "(?i)builtin",
        "a{1,5}",
        "pci-\\d+",
        "[0-9_]+",
        // Unclosed / syntactically invalid regexes (which would panic in regex::Regex::new)
        "[",
        "[]",
        "[a-",
        "(",
        "((",
        ")",
        "{",
        "\\",
        "*",
        "+",
        "?",
        "??",
        "*+",
        "+?",
        "^$",
    ];

    for payload in regex_payloads {
        // Must NEVER panic under invalid regex tokens — string matching is literal substring!
        let matches = matches_search(name, desc, payload);
        // Literal tokens that actually appear in name or desc:
        if payload == "3$" {
            // desc doesn't have 3$, so it should be false
            assert!(!matches);
        } else if payload == ".*" {
            assert!(!matches);
        } else if payload == "pci-" {
            assert!(matches);
        }
    }

    // Explicitly verify literal matching of metacharacters:
    let node_with_symbols_name = "test_node_[dsp]*+(v2.0)?";
    let node_with_symbols_desc = "Channel split: {L|R} -> (Out^1 & Out$2)";

    assert!(matches_search(node_with_symbols_name, node_with_symbols_desc, "[dsp]"));
    assert!(matches_search(node_with_symbols_name, node_with_symbols_desc, "*+"));
    assert!(matches_search(node_with_symbols_name, node_with_symbols_desc, "(v2.0)?"));
    assert!(matches_search(node_with_symbols_name, node_with_symbols_desc, "{L|R}"));
    assert!(matches_search(node_with_symbols_name, node_with_symbols_desc, "Out^1"));
    assert!(matches_search(node_with_symbols_name, node_with_symbols_desc, "Out$2"));
}

#[test]
fn test_adversarial_search_unicode_scripts_and_normalization() {
    let name = "dac_Über_Kopfhörer_München";
    let desc = "Studio Монитор (Аудио) — 24bit / 日本語テスト / 🎧 🔊";

    // 1. Accented German
    assert!(matches_search(name, desc, "über"));
    assert!(matches_search(name, desc, "ÜBER"));
    assert!(matches_search(name, desc, "kopfhörer"));
    assert!(matches_search(name, desc, "KOPFHÖRER"));
    assert!(matches_search(name, desc, "münchen"));
    assert!(matches_search(name, desc, "MÜNCHEN"));

    // 2. Cyrillic case-insensitive
    assert!(matches_search(name, desc, "монитор"));
    assert!(matches_search(name, desc, "МОНИТОР"));
    assert!(matches_search(name, desc, "аудио"));
    assert!(matches_search(name, desc, "АУДИО"));

    // 3. CJK
    assert!(matches_search(name, desc, "日本語"));
    assert!(matches_search(name, desc, "テスト"));

    // 4. Emojis
    assert!(matches_search(name, desc, "🎧"));
    assert!(matches_search(name, desc, "🔊"));
    assert!(!matches_search(name, desc, "🎸"));

    // 5. Special Punctuation & Em-dash
    assert!(matches_search(name, desc, "—"));
    assert!(matches_search(name, desc, "/"));
    assert!(matches_search(name, desc, "("));
    assert!(matches_search(name, desc, ")"));

    // 6. Zero-width and invisible characters
    let zwsp_query = "\u{200B}";
    // Searching for a zero-width space in a string without it should cleanly return false without panic
    assert!(!matches_search(name, desc, zwsp_query));

    // Zero-width space inside name:
    let zwsp_node_name = "secret\u{200B}audio";
    let zwsp_node_desc = "invisible\u{FEFF}marker";
    assert!(matches_search(zwsp_node_name, zwsp_node_desc, "\u{200B}"));
    assert!(matches_search(zwsp_node_name, zwsp_node_desc, "\u{FEFF}"));
}

#[test]
fn test_adversarial_search_null_bytes_and_control_characters() {
    let name = "alsa_output.pci";
    let desc = "Built-in Audio";

    // Null bytes embedded in query
    let null_query = "alsa\0output";
    // Neither name nor desc has a null byte, so this should safely return false without crash
    assert!(!matches_search(name, desc, null_query));

    // Node containing a null byte:
    let node_with_null = "alsa\0output";
    assert!(matches_search(node_with_null, desc, "alsa"));
    assert!(matches_search(node_with_null, desc, "\0"));

    // Control characters \x01 through \x1F and \x7F
    for byte in 1u8..=31u8 {
        let ch = byte as char;
        let ctrl_str = ch.to_string();
        if ch.is_whitespace() {
            // Whitespace control chars (\t, \n, \r, \x0C) trim to empty string, matching everything
            assert!(
                matches_search(name, desc, &ctrl_str),
                "Whitespace control byte {byte} should be trimmed to empty and match all"
            );
        } else {
            // Non-whitespace control queries against normal node return false cleanly
            assert!(
                !matches_search(name, desc, &ctrl_str),
                "Non-whitespace control byte {byte} should not match normal node"
            );
        }
    }

    // DEL character \x7F
    let del_str = "\x7F";
    assert!(!matches_search(name, desc, del_str));
}

#[test]
fn test_adversarial_search_injection_payloads() {
    let name = "alsa_output.pci-0000_00_1f.3";
    let desc = "Realtek ALC892 Analog Stereo";

    let injection_payloads = [
        // SQL Injection
        "' OR '1'='1",
        "' OR 1=1 --",
        "'; DROP TABLE nodes; --",
        "admin'--",
        "1' UNION SELECT 1, 'pw', 3 --",
        // Path Traversal
        "../../../../../../etc/passwd",
        "/proc/self/mem",
        "C:\\Windows\\System32\\cmd.exe",
        // Shell Injection
        "; rm -rf / ;",
        "$(whoami)",
        "`id`",
        "| cat /etc/shadow",
        "& calc.exe",
        // Cross-site Scripting / HTML
        "<script>alert('pwned')</script>",
        "<img src=x onerror=alert(1)>",
        "<svg/onload=alert`1`>",
        // Format Strings
        "%s%s%s%s%s%s%s%s%s%s%n",
        "%p%p%p%p",
        "{0}{1}{2}",
        "{{7*7}}",
    ];

    for payload in injection_payloads {
        // Query must never panic or execute; it is treated as a plain string
        let res = matches_search(name, desc, payload);
        assert!(!res, "Payload '{payload}' should not match standard node");
    }
}

#[test]
fn test_adversarial_search_extreme_lengths() {
    let name = "alsa_output.pci-0000_00_1f.3";
    let desc = "Built-in Audio Analog Stereo";

    // 1. Length 1,000
    let query_1k = "a".repeat(1_000);
    assert!(!matches_search(name, desc, &query_1k));

    // 2. Length 10,000
    let query_10k = "x".repeat(10_000);
    assert!(!matches_search(name, desc, &query_10k));

    // 3. Length 100,000
    let query_100k = " ".repeat(50_000) + "audio" + &" ".repeat(50_000);
    // When trimmed, "audio" is extracted and matches!
    assert!(matches_search(name, desc, &query_100k));

    // 4. Length 1,000,000 (1 MB query)
    let query_1m = "q".repeat(1_000_000);
    let start = std::time::Instant::now();
    let res = matches_search(name, desc, &query_1m);
    let duration = start.elapsed();
    assert!(!res);
    assert!(
        duration.as_millis() < 500,
        "1MB search query took too long: {:?}",
        duration
    );

    // 5. Huge node name and description (10,000 chars each)
    let huge_name = "prefix_".to_string() + &"n".repeat(9_980) + "_suffix";
    let huge_desc = "desc_".to_string() + &"d".repeat(9_980) + "_target";

    assert!(matches_search(&huge_name, &huge_desc, "suffix"));
    assert!(matches_search(&huge_name, &huge_desc, "target"));
    assert!(matches_search(&huge_name, &huge_desc, "prefix"));
    assert!(!matches_search(&huge_name, &huge_desc, "nonexistent"));
}

// ============================================================================
// SUITE 2: Edge Cases (0 matches, all match, partial matches)
// ============================================================================

#[test]
fn test_adversarial_zero_matches_dims_everything() {
    let graph = create_demo_graph();
    let total_nodes = graph.nodes.len();
    let total_edges = graph.collapsed_edges().len();

    let non_matching_query = "completely_impossible_token_999999999";
    let dimmed_nodes = compute_dimmed_nodes(&graph, non_matching_query);

    // 1. Every single node in the graph MUST be in dimmed_nodes
    assert_eq!(
        dimmed_nodes.len(),
        total_nodes,
        "When 0 nodes match, ALL nodes must be dimmed"
    );
    for &serial in graph.nodes.keys() {
        assert!(dimmed_nodes.contains(&serial));
    }

    // 2. Every single link in the graph MUST be in dimmed_links
    let dimmed_links = compute_dimmed_links(&graph, &dimmed_nodes, true);
    assert_eq!(
        dimmed_links.len(),
        total_edges,
        "When 0 nodes match, ALL links must be dimmed"
    );

    // 3. Canvas rendering invariants
    let node_positions = BTreeMap::new();
    let canvas = GraphCanvas {
        graph: &graph,
        visible_nodes: graph.nodes.keys().copied().collect(),
        node_positions: &node_positions,
        selected_edge: None,
        selected_node: None,
        pan: Vector::ZERO,
        zoom: 1.0,
        layer: CanvasLayer::Base,
        search_query: non_matching_query,
    };

    assert!(canvas.is_search_active());
    for &serial in graph.nodes.keys() {
        assert!(canvas.is_node_dimmed(serial));
    }
    for edge in graph.collapsed_edges() {
        assert!(canvas.is_link_dimmed(&edge.key));
    }
}

#[test]
fn test_adversarial_all_nodes_match_dims_nothing() {
    let mut graph = PwGraph::default();
    // Construct 4 nodes where all contain the token "apollo" in name or description
    let n1 = build_synthetic_node(1, "apollo_source_1", "Core mic", NodeCategory::AudioSource, 0, 1);
    let n2 = build_synthetic_node(2, "apollo_filter", "DSP stage", NodeCategory::AudioSink, 1, 1);
    let n3 = build_synthetic_node(3, "monitor_out", "Apollo studio monitors", NodeCategory::AudioSink, 1, 0);
    let n4 = build_synthetic_node(4, "headphones_out", "Headphones (Apollo)", NodeCategory::AudioSink, 1, 0);

    link_nodes(&mut graph, 10, &n1, &n2);
    link_nodes(&mut graph, 20, &n2, &n3);
    link_nodes(&mut graph, 30, &n2, &n4);

    graph.nodes.insert(n1.object_serial, n1);
    graph.nodes.insert(n2.object_serial, n2);
    graph.nodes.insert(n3.object_serial, n3);
    graph.nodes.insert(n4.object_serial, n4);

    let query = "apollo";
    let dimmed_nodes = compute_dimmed_nodes(&graph, query);

    // 1. dimmed_nodes MUST be empty because all 4 nodes match!
    assert!(
        dimmed_nodes.is_empty(),
        "When all nodes match, dimmed_nodes must be empty, but got {:?}",
        dimmed_nodes
    );

    // 2. dimmed_links MUST be empty because both endpoints of every link match!
    let dimmed_links = compute_dimmed_links(&graph, &dimmed_nodes, true);
    assert!(
        dimmed_links.is_empty(),
        "When all nodes match, dimmed_links must be empty, but got {:?}",
        dimmed_links
    );

    // 3. Canvas helpers verify no node or link is dimmed
    let node_positions = BTreeMap::new();
    let canvas = GraphCanvas {
        graph: &graph,
        visible_nodes: graph.nodes.keys().copied().collect(),
        node_positions: &node_positions,
        selected_edge: None,
        selected_node: None,
        pan: Vector::ZERO,
        zoom: 1.0,
        layer: CanvasLayer::Base,
        search_query: query,
    };

    assert!(canvas.is_search_active());
    for &serial in graph.nodes.keys() {
        assert!(!canvas.is_node_dimmed(serial));
    }
    for edge in graph.collapsed_edges() {
        assert!(!canvas.is_link_dimmed(&edge.key));
    }
}

#[test]
fn test_adversarial_partial_matches_diamond_graph() {
    let mut graph = PwGraph::default();

    // S (Source) -> F1 (Filter 1) -> K (Sink)
    //            -> F2 (Filter 2) ->
    let s = build_synthetic_node(1, "usb_mic_source", "Vocal input", NodeCategory::AudioSource, 0, 2);
    let f1 = build_synthetic_node(2, "eq_equalizer_dsp", "10-band parametric", NodeCategory::AudioSink, 1, 1);
    let f2 = build_synthetic_node(3, "compressor_dsp", "Dynamic limiter", NodeCategory::AudioSink, 1, 1);
    let k = build_synthetic_node(4, "headphones_sink", "Stereo output", NodeCategory::AudioSink, 2, 0);

    link_nodes(&mut graph, 10, &s, &f1);
    link_nodes(&mut graph, 20, &s, &f2);
    link_nodes(&mut graph, 30, &f1, &k);
    link_nodes(&mut graph, 40, &f2, &k);

    graph.nodes.insert(s.object_serial, s);
    graph.nodes.insert(f1.object_serial, f1);
    graph.nodes.insert(f2.object_serial, f2);
    graph.nodes.insert(k.object_serial, k);

    // Scenario A: Query matches only F1 ("equalizer")
    {
        let dimmed_nodes = compute_dimmed_nodes(&graph, "equalizer");
        assert_eq!(dimmed_nodes, HashSet::from([1, 3, 4]));

        let dimmed_links = compute_dimmed_links(&graph, &dimmed_nodes, true);
        // All links touch at least one non-matching node (1, 3, or 4), so ALL links are dimmed!
        assert_eq!(dimmed_links.len(), 4);
    }

    // Scenario B: Query matches S and F1 ("usb_mic" and "dsp" -> query "e" appears in "usb_mic_source" and "equalizer" and "compressor" and "stereo")
    // Let's use a query that matches S (1) and F1 (2) exclusively: "parametric" or "vocal" -> test disjunction:
    // Query "1" appears in "playback_1", but search is over name and description!
    // F1 name has "eq", description has "parametric".
    // S description has "vocal".
    // Query "DSP": matches F1 and F2!
    {
        let dimmed_nodes = compute_dimmed_nodes(&graph, "dsp");
        // Matches F1 (2) and F2 (3). Dimmed: S (1) and K (4).
        assert_eq!(dimmed_nodes, HashSet::from([1, 4]));

        let dimmed_links = compute_dimmed_links(&graph, &dimmed_nodes, true);
        // Link 1->2 (S->F1): S is dimmed -> link is dimmed
        // Link 1->3 (S->F2): S is dimmed -> link is dimmed
        // Link 2->4 (F1->K): K is dimmed -> link is dimmed
        // Link 3->4 (F2->K): K is dimmed -> link is dimmed
        assert_eq!(dimmed_links.len(), 4);
    }

    // Scenario C: Link between two MATCHING nodes
    // Add link from F1 (2) to F2 (3): both match "dsp"!
    {
        let mut graph_with_internal_edge = graph.clone();
        let n2_clone = graph_with_internal_edge.nodes[&2].clone();
        let n3_clone = graph_with_internal_edge.nodes[&3].clone();
        link_nodes(&mut graph_with_internal_edge, 50, &n2_clone, &n3_clone);

        let dimmed_nodes = compute_dimmed_nodes(&graph_with_internal_edge, "dsp");
        let dimmed_links = compute_dimmed_links(&graph_with_internal_edge, &dimmed_nodes, true);

        let internal_edge = EdgeKey {
            output_node_serial: 2,
            input_node_serial: 3,
        };

        // Edge 2->3 connects two matching nodes, so it MUST NOT be dimmed!
        assert!(
            !dimmed_links.contains(&internal_edge),
            "Edge between two matching nodes (2 -> 3) must NOT be dimmed!"
        );
        assert!(!is_link_dimmed(&internal_edge, &dimmed_nodes, true));
    }
}

// ============================================================================
// SUITE 3: Layout Stability Invariant Under 1,000 Randomized Queries
// ============================================================================

#[test]
fn test_adversarial_1000_randomized_queries_layout_stability_invariant() {
    let graph = create_demo_graph();
    let mut rng = XorShift64::new(0xDEAD_BEEF_CAFE_1234);

    // Initial node positions (deterministic synthetic grid representing app.node_positions)
    let mut node_positions: BTreeMap<u64, Point> = BTreeMap::new();
    let col_x = [40.0, 380.0, 720.0];
    let row_spacing = NODE_HEIGHT + 34.0;

    for (idx, &serial) in graph.nodes.keys().enumerate() {
        let col = idx % 3;
        let row = idx / 3;
        let x = col_x[col];
        let y = 40.0 + row as f32 * row_spacing;
        node_positions.insert(serial, Point::new(x, y));
    }

    let initial_positions_snapshot = node_positions.clone();
    assert!(!initial_positions_snapshot.is_empty());

    // Pool of token sources for random queries
    let known_tokens = [
        "spotify",
        "audio",
        "alsa",
        "Focusrite",
        "Scarlett",
        "EasyEffects",
        "Built-in",
        "Headphones",
        "Speakers",
        "Microphone",
        "pci",
        "Analog",
        "Stereo",
        "Surround",
        "Discord",
        "Firefox",
        "OBS",
    ];

    let hostile_tokens = [
        "'; DROP TABLE nodes; --",
        "<script>alert(1)</script>",
        "../../../../etc/passwd",
        ".*+?[a-z]",
        "\\w+\\d+^$",
        "🎧 🔊 🎵",
        "über Kopfhörer",
        "München",
        "\0\x01\x1F",
        "%s%s%n",
        "   \t\r\n   ",
        "\" OR \"\"=\"",
    ];

    for iteration in 0..1_000 {
        // Generate query
        let query_type = rng.next_range(0, 5);
        let query = match query_type {
            0 => {
                // Inactive / blank query
                match rng.next_range(0, 3) {
                    0 => String::new(),
                    1 => "   ".to_string(),
                    2 => "\t\n  \r\n ".to_string(),
                    _ => "        ".to_string(),
                }
            }
            1 => {
                // Known token with random case mutation and padding
                let tok = known_tokens[rng.next_range(0, known_tokens.len() - 1)];
                let pad_left = " ".repeat(rng.next_range(0, 4));
                let pad_right = " ".repeat(rng.next_range(0, 4));
                let mutated: String = tok
                    .chars()
                    .map(|c| {
                        if rng.next_range(0, 1) == 0 {
                            c.to_uppercase().to_string()
                        } else {
                            c.to_lowercase().to_string()
                        }
                    })
                    .collect();
                format!("{pad_left}{mutated}{pad_right}")
            }
            2 => {
                // Hostile token
                let tok = hostile_tokens[rng.next_range(0, hostile_tokens.len() - 1)];
                tok.to_string()
            }
            3 => {
                // Random alphanumeric garbage
                let len = rng.next_range(1, 40);
                rng.next_string(len)
            }
            4 => {
                // Long query (500 to 2,000 characters)
                let len = rng.next_range(500, 2_000);
                rng.next_string(len)
            }
            _ => {
                // Single character / edge token
                rng.next_char().to_string()
            }
        };

        // 1. Compute dimmed nodes
        let dimmed_nodes = compute_dimmed_nodes(&graph, &query);
        let search_active = !query.trim().is_empty();
        let dimmed_links = compute_dimmed_links(&graph, &dimmed_nodes, search_active);

        // 2. Instantiate GraphCanvas (which models how app renders canvas)
        let canvas = GraphCanvas {
            graph: &graph,
            visible_nodes: graph.nodes.keys().copied().collect(),
            node_positions: &node_positions,
            selected_edge: None,
            selected_node: None,
            pan: Vector::new(40.0, 40.0),
            zoom: 1.0,
            layer: CanvasLayer::Base,
            search_query: &query,
        };

        // Invariant 1: Canvas must report correct search active state
        assert_eq!(
            canvas.is_search_active(),
            search_active,
            "Canvas search_active flag mismatch for query '{query}' (iteration {iteration})"
        );

        // Invariant 2: Canvas dimming methods must match compute_dimmed_nodes exactly
        for &serial in graph.nodes.keys() {
            let expected_dim = if search_active {
                dimmed_nodes.contains(&serial)
            } else {
                false
            };
            assert_eq!(
                canvas.is_node_dimmed(serial),
                expected_dim,
                "Node {serial} dimming mismatch on query '{query}' (iteration {iteration})"
            );
        }

        // Invariant 3: node_positions is NEVER mutated or wiped
        assert_eq!(
            node_positions, initial_positions_snapshot,
            "CRITICAL INVARIANT VIOLATION: node_positions was mutated during iteration {iteration} with query '{query}'"
        );

        // Invariant 4: Layout point coordinates are strictly bit-for-bit identical
        for (&serial, &pos) in &node_positions {
            let orig = initial_positions_snapshot[&serial];
            assert_eq!(
                pos.x.to_bits(),
                orig.x.to_bits(),
                "Node {serial} X coordinate altered bitwise on iteration {iteration}"
            );
            assert_eq!(
                pos.y.to_bits(),
                orig.y.to_bits(),
                "Node {serial} Y coordinate altered bitwise on iteration {iteration}"
            );
        }
    }
}
