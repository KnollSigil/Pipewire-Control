#![allow(dead_code, unused_imports, unused_variables)]

#[path = "../src/model.rs"]
mod model;

#[path = "../src/canvas_view.rs"]
mod canvas_view;

use canvas_view::*;
use iced::mouse;
use iced::widget::canvas::{self, Event, Program};
use iced::{Color, Point, Rectangle, Size, Vector};
use model::{EdgeKey, NodeCategory, PwGraph, PwNode, PwPort};
use std::collections::{BTreeMap, HashSet};

// ============================================================================
// AREA 1: Extreme Label Text Strings in compute_tooltip_width
// ============================================================================

#[test]
fn test_adversarial_tooltip_width_empty_string() {
    let width = compute_tooltip_width("", 12.0);
    assert_eq!(
        width, 16.0,
        "Empty label should strictly equal the horizontal padding (16.0px)"
    );
    assert!(width.is_finite());
}

#[test]
fn test_adversarial_tooltip_width_extreme_lengths() {
    let text_size = 11.5;

    // 100, 1,000, and 10,000 character strings
    let s_100 = "a".repeat(100);
    let s_1000 = "a".repeat(1000);
    let s_10000 = "a".repeat(10000);

    let w_100 = compute_tooltip_width(&s_100, text_size);
    let w_1000 = compute_tooltip_width(&s_1000, text_size);
    let w_10000 = compute_tooltip_width(&s_10000, text_size);

    assert!(w_100.is_finite());
    assert!(w_1000.is_finite());
    assert!(w_10000.is_finite());

    assert!(
        w_1000 > w_100 * 9.0,
        "1000-char width ({}) must scale roughly 10x from 100-char width ({})",
        w_1000,
        w_100
    );
    assert!(
        w_10000 > w_1000 * 9.0,
        "10000-char width ({}) must scale roughly 10x from 1000-char width ({})",
        w_10000,
        w_1000
    );
}

#[test]
fn test_adversarial_tooltip_width_unicode_emojis_and_scripts() {
    let text_size = 12.0;

    // Unicode emojis & ZWJ sequence
    let emojis = "🔥🚀🎉🦀👨‍👩‍👧‍👦✨🎶🎸🎹";
    let w_emojis = compute_tooltip_width(emojis, text_size);
    assert!(w_emojis.is_finite() && w_emojis > 16.0);

    // Multilingual scripts
    let cjk = "PipeWire 音声ルーティング設定 (デフォルト)";
    let cyrillic = "Аудио поток и конфигурация соединений";
    let arabic = "توجيه إشارات الصوت المتقدمة";
    let hebrew = "חיבורי אודיו מתקדמים ברשת";
    let devanagari = "ऑडियो सिग्नल रूटिंग और सेटिंग्स";

    for (name, script) in [
        ("CJK", cjk),
        ("Cyrillic", cyrillic),
        ("Arabic", arabic),
        ("Hebrew", hebrew),
        ("Devanagari", devanagari),
    ] {
        let w = compute_tooltip_width(script, text_size);
        assert!(
            w.is_finite() && w > 16.0,
            "{} width should be finite and > padding: got {}",
            name,
            w
        );
    }

    // Combining characters and Zero-Width Space
    let combining = "e\u{0301}\u{0302}\u{0303}a\u{0304}\u{0305}";
    let w_combining = compute_tooltip_width(combining, text_size);
    assert!(w_combining.is_finite() && w_combining > 16.0);

    let zws = "\u{200B}\u{200C}\u{200D}\u{FEFF}";
    let w_zws = compute_tooltip_width(zws, text_size);
    assert!(w_zws.is_finite() && w_zws > 16.0);
}

#[test]
fn test_adversarial_tooltip_width_control_and_injection_characters() {
    let text_size = 12.0;

    // Control characters (NUL, Bell, Backspace, Tab, LF, CR, Escape)
    let controls = "\0\x01\x07\x08\t\n\r\x1b";
    let w_controls = compute_tooltip_width(controls, text_size);
    assert!(w_controls.is_finite() && w_controls > 16.0);

    // Injection-like payloads and special symbols
    let payloads = [
        "<script>alert('xss')</script>",
        "../../../../etc/passwd",
        "' OR '1'='1'; DROP TABLE links; --",
        "%s%d%n%x%p",
        "!@#$%^&*()_+{}[]|\":;'<>?,./~`",
    ];

    for payload in payloads {
        let w = compute_tooltip_width(payload, text_size);
        assert!(
            w.is_finite() && w > 16.0,
            "Payload '{}' produced invalid width: {}",
            payload,
            w
        );
    }
}

#[test]
fn test_adversarial_tooltip_width_boundary_font_sizes() {
    let label = "Stereo Out -> Main (default)";

    // Zero font size -> pure padding (16.0)
    let w_zero = compute_tooltip_width(label, 0.0);
    assert_eq!(w_zero, 16.0);

    // Negative font size -> clamped to 0.0 by text_size.max(0.0) -> pure padding (16.0)
    let w_neg = compute_tooltip_width(label, -100.0);
    assert_eq!(w_neg, 16.0);

    // Sub-pixel fractional font size
    let w_tiny = compute_tooltip_width(label, 0.001);
    assert!(w_tiny > 16.0 && w_tiny < 17.0);

    // Giant font size
    let w_giant = compute_tooltip_width(label, 500.0);
    assert!(w_giant.is_finite() && w_giant > 5000.0);
}

// ============================================================================
// AREA 2: Tooltip Rect Clamping under Fractional/Extreme Zoom & Canvas Bounds
// ============================================================================

#[test]
fn test_adversarial_tooltip_rect_zoom_extremes_and_text_size_clamping() {
    let bounds = Rectangle::new(Point::ORIGIN, Size::new(1920.0, 1080.0));
    let anchor = Point::new(960.0, 540.0);
    let label = "Built-in Audio Analog Stereo -> Main Output (default)";

    // Test a wide sweep of zoom levels: 0.001x to 100.0x
    let zooms = [
        0.001_f32, 0.01, 0.1, 0.25, 0.5, 0.8, 1.0, 1.25, 1.5, 2.0, 5.0, 10.0, 50.0, 100.0,
    ];

    for &zoom in &zooms {
        // UI formula: (11.5 * zoom.clamp(0.8, 1.25)).max(9.5)
        let ui_text_size = (11.5 * zoom.clamp(0.8, 1.25)).max(9.5);
        assert!(
            (9.5..=14.375).contains(&ui_text_size),
            "UI text size {} at zoom {} must be bounded in [9.5, 14.375]",
            ui_text_size,
            zoom
        );

        let rect = compute_tooltip_rect(label, ui_text_size, anchor, bounds);

        // Clamping invariants
        assert!(rect.x >= bounds.x + 6.0, "rect.x must be >= bounds.x + margin");
        assert!(
            rect.x + rect.width <= bounds.x + bounds.width - 6.0,
            "rect right edge must be <= bounds right edge - margin"
        );
        assert!(rect.y >= bounds.y + 6.0, "rect.y must be >= bounds.y + margin");
        assert!(
            rect.y + rect.height <= bounds.y + bounds.height - 6.0,
            "rect bottom edge must be <= bounds bottom edge - margin"
        );
    }
}

#[test]
fn test_adversarial_tooltip_rect_canvas_bounds_extremes() {
    let label = "Playback FL, FR -> Sink FL, FR (custom)";
    let text_size = 11.5;
    let anchor = Point::new(5.0, 5.0);

    // 1. Tiny Canvas: 10x10 px (Tooltip is much larger than canvas: ~230x21 px)
    let tiny_bounds = Rectangle::new(Point::ORIGIN, Size::new(10.0, 10.0));
    let tiny_rect = compute_tooltip_rect(label, text_size, anchor, tiny_bounds);
    // Must not panic, must fall back safely to bounds origin
    assert_eq!(tiny_rect.x, tiny_bounds.x);
    assert_eq!(tiny_rect.y, tiny_bounds.y);
    assert!(tiny_rect.width > 0.0 && tiny_rect.height > 0.0);

    // 2. Zero-Width and Zero-Height Canvas: (100.0, 200.0, 0.0, 0.0)
    let zero_bounds = Rectangle::new(Point::new(100.0, 200.0), Size::ZERO);
    let zero_rect = compute_tooltip_rect(label, text_size, Point::new(100.0, 200.0), zero_bounds);
    // Must not panic, must clamp safely to bounds origin
    assert_eq!(zero_rect.x, zero_bounds.x);
    assert_eq!(zero_rect.y, zero_bounds.y);

    // 3. Negative Dimensions: (50.0, 50.0, -100.0, -100.0)
    let neg_bounds = Rectangle::new(Point::new(50.0, 50.0), Size::new(-100.0, -100.0));
    let neg_rect = compute_tooltip_rect(label, text_size, Point::new(50.0, 50.0), neg_bounds);
    assert_eq!(neg_rect.x, neg_bounds.x);
    assert_eq!(neg_rect.y, neg_bounds.y);

    // 4. Unusual Aspect Ratios: ultra-wide (100,000 x 5) and ultra-tall (5 x 100,000)
    let ultra_wide = Rectangle::new(Point::ORIGIN, Size::new(100_000.0, 5.0));
    let wide_rect = compute_tooltip_rect(label, text_size, Point::new(500.0, 2.0), ultra_wide);
    assert!(wide_rect.x >= ultra_wide.x);
    assert_eq!(wide_rect.y, ultra_wide.y); // height is too small (5 < 21.5), so y falls back safely to bounds.y

    let ultra_tall = Rectangle::new(Point::ORIGIN, Size::new(5.0, 100_000.0));
    let tall_rect = compute_tooltip_rect(label, text_size, Point::new(2.0, 500.0), ultra_tall);
    assert_eq!(tall_rect.x, ultra_tall.x); // width is too small (5 < 230), so x falls back safely to bounds.x
    assert!(tall_rect.y >= ultra_tall.y);

    // 5. Negative Origin Canvas: (-1000.0, -800.0, 2000.0, 1600.0)
    let offset_bounds = Rectangle::new(Point::new(-1000.0, -800.0), Size::new(2000.0, 1600.0));
    let offset_anchor = Point::new(-950.0, -750.0);
    let offset_rect = compute_tooltip_rect(label, text_size, offset_anchor, offset_bounds);
    assert!(offset_rect.x >= offset_bounds.x + 6.0);
    assert!(offset_rect.x + offset_rect.width <= offset_bounds.x + offset_bounds.width - 6.0);
    assert!(offset_rect.y >= offset_bounds.y + 6.0);
    assert!(offset_rect.y + offset_rect.height <= offset_bounds.y + offset_bounds.height - 6.0);
}

#[test]
fn test_adversarial_tooltip_rect_anchor_extremes_and_flipping() {
    let bounds = Rectangle::new(Point::ORIGIN, Size::new(800.0, 600.0));
    let label = "Wire Label";
    let text_size = 11.5;
    let margin = 6.0;

    // Anchor at exact top edge (y = 0.0)
    // Placed above would be y = 0.0 - 21.5 - 10.0 = -31.5 < margin (6.0).
    // Must flip below anchor: y = 0.0 + 16.0 = 16.0.
    let top_rect = compute_tooltip_rect(label, text_size, Point::new(400.0, 0.0), bounds);
    assert_eq!(
        top_rect.y, 16.0,
        "Tooltip must flip below anchor when placed above canvas top"
    );

    // Anchor at exact bottom edge (y = 600.0)
    // Placed above: y = 600.0 - 21.5 - 10.0 = 568.5.
    // Bottom edge = 568.5 + 21.5 = 590.0 <= bounds.height - margin (594.0).
    let bottom_rect = compute_tooltip_rect(label, text_size, Point::new(400.0, 600.0), bounds);
    assert!(bottom_rect.y + bottom_rect.height <= bounds.height - margin);

    // Anchor way outside canvas: (-10000.0, -5000.0) and (50000.0, 30000.0)
    let neg_far = compute_tooltip_rect(label, text_size, Point::new(-10000.0, -5000.0), bounds);
    assert_eq!(neg_far.x, bounds.x + margin);
    assert!(neg_far.y >= bounds.y + margin);

    let pos_far = compute_tooltip_rect(label, text_size, Point::new(50000.0, 30000.0), bounds);
    assert_eq!(pos_far.x + pos_far.width, bounds.width - margin);
    assert_eq!(pos_far.y + pos_far.height, bounds.height - margin);
}

#[test]
fn test_adversarial_tooltip_rect_floating_point_special_values() {
    let bounds = Rectangle::new(Point::ORIGIN, Size::new(800.0, 600.0));
    let label = "Test Label";
    let text_size = 11.5;

    // Anchor with Infinity
    let inf_anchor = Point::new(f32::INFINITY, f32::INFINITY);
    let inf_rect = compute_tooltip_rect(label, text_size, inf_anchor, bounds);
    assert!(inf_rect.x.is_finite());
    assert!(inf_rect.y.is_finite());
    assert!(inf_rect.x <= bounds.width);
    assert!(inf_rect.y <= bounds.height);

    // Anchor with NegInfinity
    let neg_inf_anchor = Point::new(f32::NEG_INFINITY, f32::NEG_INFINITY);
    let neg_inf_rect = compute_tooltip_rect(label, text_size, neg_inf_anchor, bounds);
    assert!(neg_inf_rect.x.is_finite());
    assert!(neg_inf_rect.y.is_finite());
    assert!(neg_inf_rect.x >= bounds.x);
    assert!(neg_inf_rect.y >= bounds.y);
}

#[test]
fn test_adversarial_tooltip_rect_nan_anchor() {
    let bounds = Rectangle::new(Point::ORIGIN, Size::new(800.0, 600.0));
    let label = "Test Label";
    let text_size = 11.5;

    let nan_anchor = Point::new(f32::NAN, f32::NAN);
    let _nan_rect = compute_tooltip_rect(label, text_size, nan_anchor, bounds);
    assert!(!bounds.contains(nan_anchor));
}

// ============================================================================
// AREA 3: Edge Hover Transitions and Cursor Movement Fuzzing
// ============================================================================

#[test]
fn test_adversarial_cursor_lifecycle_and_hover_state_fuzzing() {
    let graph = PwGraph::default();
    let visible_nodes = HashSet::new();
    let node_positions = BTreeMap::new();
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
        search_query: "",
    };

    let bounds = Rectangle::new(Point::ORIGIN, Size::new(800.0, 600.0));
    let mut state = CanvasInteraction::default();

    let dummy_edge = EdgeKey {
        output_node_serial: 10,
        input_node_serial: 20,
    };

    // 1. Cursor moves inside bounds over an edge
    state.hovered_edge = Some(dummy_edge.clone());
    let pos1 = Point::new(200.0, 300.0);
    let event1 = Event::Mouse(mouse::Event::CursorMoved { position: pos1 });
    let action1 = canvas.update(&mut state, &event1, bounds, mouse::Cursor::Available(pos1));

    assert_eq!(state.cursor_pos, Some(pos1));
    assert!(action1.is_some(), "Hovering edge must request redraw on move");

    // 2. Cursor leaves canvas (CursorLeft)
    let left_event = Event::Mouse(mouse::Event::CursorLeft);
    let action_left = canvas.update(&mut state, &left_event, bounds, mouse::Cursor::Unavailable);

    // State must be completely wiped
    assert_eq!(state.cursor_pos, None, "Cursor pos must be cleared on CursorLeft");
    assert_eq!(state.hovered_edge, None, "Hovered edge must be cleared on CursorLeft");
    assert_eq!(state.hovered_node, None, "Hovered node must be cleared on CursorLeft");
    assert_eq!(state.hovered_input_socket, None, "Input socket must be cleared on CursorLeft");
    assert_eq!(state.hovered_output_socket, None, "Output socket must be cleared on CursorLeft");
    assert!(
        action_left.is_some(),
        "CursorLeft must request redraw to erase lingering tooltip/hover outline"
    );

    // 3. Fuzzing 5,000 rapid cursor transitions: inside, outside, edge, corners, unavailable
    let test_points = [
        Point::new(0.0, 0.0),
        Point::new(800.0, 600.0),
        Point::new(-1.0, 300.0),
        Point::new(801.0, 300.0),
        Point::new(400.0, -1.0),
        Point::new(400.0, 601.0),
        Point::new(100.0, 100.0),
        Point::new(500.0, 450.0),
    ];

    for i in 0..5000 {
        let pt = test_points[i % test_points.len()];
        let is_inside = bounds.contains(pt);

        if i % 7 == 0 {
            let _action = canvas.update(&mut state, &Event::Mouse(mouse::Event::CursorLeft), bounds, mouse::Cursor::Unavailable);
            assert_eq!(state.cursor_pos, None);
            assert_eq!(state.hovered_edge, None);
        } else {
            let cursor = if is_inside || i % 2 == 0 {
                mouse::Cursor::Available(pt)
            } else {
                mouse::Cursor::Unavailable
            };

            if i % 3 == 0 {
                state.hovered_edge = Some(dummy_edge.clone());
            }

            let event = Event::Mouse(mouse::Event::CursorMoved { position: pt });
            let _action = canvas.update(&mut state, &event, bounds, cursor);

            if is_inside {
                assert_eq!(state.cursor_pos, Some(pt));
            }
        }
    }
}

// ============================================================================
// AREA 4: Link Badge Visibility Truth Table & Invariants
// ============================================================================

#[test]
fn test_adversarial_badge_visibility_exhaustive_truth_table() {
    for is_default in [false, true] {
        for is_hovered in [false, true] {
            for is_selected in [false, true] {
                let show = should_show_link_badge(is_default, is_hovered, is_selected);
                let expected = !(is_default && !is_hovered && !is_selected);

                assert_eq!(
                    show, expected,
                    "Mismatch for is_default={}, is_hovered={}, is_selected={}: expected {}, got {}",
                    is_default, is_hovered, is_selected, expected, show
                );
            }
        }
    }
}
