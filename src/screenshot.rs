use crate::model::{create_demo_graph, EdgeKey, NodeCategory, PwNode};
use crate::{Message, PipewireApp};
use iced::advanced::renderer::Headless;
use iced::mouse;
use iced::{Color, Point, Size};
use iced_fonts::LUCIDE_FONT_BYTES;
use iced_runtime::user_interface::Cache;
use iced_runtime::UserInterface;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs::{create_dir_all, File};
use std::path::{Path, PathBuf};

pub const WINDOW_WIDTH: u32 = 1380;
pub const WINDOW_HEIGHT: u32 = 860;

/// Initializes a headless `tiny-skia` renderer and registers the application's Lucide icon font.
pub async fn create_headless_renderer() -> Result<iced::Renderer, String> {
    let renderer = <iced::Renderer as Headless>::new(
        iced::Font::DEFAULT,
        iced::Pixels(14.0),
        Some("tiny-skia"),
    )
    .await
    .ok_or_else(|| "Failed to create headless tiny-skia renderer".to_string())?;

    // Load Lucide icon font into global font system
    iced_graphics::text::font_system()
        .write()
        .map_err(|e| format!("Failed to lock font system: {e}"))?
        .load_font(Cow::Borrowed(LUCIDE_FONT_BYTES));

    Ok(renderer)
}

/// Renders a single frame of `PipewireApp` headlessly to an RGBA byte buffer.
pub fn render_app_frame(
    renderer: &mut iced::Renderer,
    app: &PipewireApp,
    cursor_pos: Option<Point>,
) -> Result<Vec<u8>, String> {
    let window_size = Size::new(WINDOW_WIDTH as f32, WINDOW_HEIGHT as f32);
    let cache = Cache::default();

    let mut user_interface = UserInterface::build(app.view(), window_size, cache, renderer);

    let cursor = match cursor_pos {
        Some(pos) => {
            let mut messages = Vec::new();
            let mut clipboard = iced::advanced::clipboard::Null;
            user_interface.update(
                &[iced::Event::Mouse(iced::mouse::Event::CursorMoved {
                    position: pos,
                })],
                mouse::Cursor::Available(pos),
                renderer,
                &mut clipboard,
                &mut messages,
            );
            mouse::Cursor::Available(pos)
        }
        None => mouse::Cursor::Unavailable,
    };

    user_interface.draw(
        renderer,
        &iced::Theme::Dark,
        &iced::advanced::renderer::Style {
            text_color: Color::WHITE,
        },
        cursor,
    );

    let bytes = renderer.screenshot(
        Size::new(WINDOW_WIDTH, WINDOW_HEIGHT),
        1.0,
        Color::from_rgb(0.08, 0.09, 0.12),
    );

    let expected_len = (WINDOW_WIDTH * WINDOW_HEIGHT * 4) as usize;
    if bytes.len() != expected_len {
        return Err(format!(
            "Invalid screenshot byte length: expected {expected_len}, got {}",
            bytes.len()
        ));
    }

    Ok(bytes)
}

/// Encodes raw RGBA bytes into a PNG file on disk.
pub fn save_png(path: &Path, rgba: &[u8], width: u32, height: u32) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        create_dir_all(parent).map_err(|e| format!("Failed to create directory {parent:?}: {e}"))?;
    }

    let file = File::create(path).map_err(|e| format!("Failed to create file {path:?}: {e}"))?;
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);

    let mut writer = encoder
        .write_header()
        .map_err(|e| format!("Failed to write PNG header for {path:?}: {e}"))?;
    writer
        .write_image_data(rgba)
        .map_err(|e| format!("Failed to write PNG image data for {path:?}: {e}"))?;

    Ok(())
}

/// Creates a PipewireApp initialized in interactive demo mode with auto-layout computed.
fn create_demo_app() -> PipewireApp {
    let (mut app, _) = PipewireApp::new();
    app.backend = crate::backend::Backend::demo();
    app.graph = create_demo_graph();
    app.compute_auto_layout();
    app
}

/// Generates all visual verification screenshots (scenarios a through h) and writes them to `output_dir`.
pub fn generate_all_screenshots(output_dir: &Path) -> Result<Vec<PathBuf>, String> {
    create_dir_all(output_dir)
        .map_err(|e| format!("Failed to create output dir {output_dir:?}: {e}"))?;

    iced::futures::executor::block_on(async {
        let mut renderer = create_headless_renderer().await?;
        let mut generated_paths = Vec::new();

        let save_both_names = |paths: &mut Vec<PathBuf>,
                               rgba: &[u8],
                               short_name: &str,
                               long_name: &str|
         -> Result<(), String> {
            let p1 = output_dir.join(short_name);
            save_png(&p1, rgba, WINDOW_WIDTH, WINDOW_HEIGHT)?;
            paths.push(p1);

            let p2 = output_dir.join(long_name);
            save_png(&p2, rgba, WINDOW_WIDTH, WINDOW_HEIGHT)?;
            paths.push(p2);
            Ok(())
        };

        // ====================================================================
        // Scenario (a): Clean startup after auto-layout
        // Columns arranged left-to-right by signal flow depth, no overlapping
        // node cards, no cards on link curves, overview inspector visible.
        // ====================================================================
        println!("Generating scenario (a): startup after auto-layout...");
        let app_a = create_demo_app();
        let bytes_a = render_app_frame(&mut renderer, &app_a, None)?;
        save_both_names(
            &mut generated_paths,
            &bytes_a,
            "a_startup_after_auto_layout.png",
            "scenario_a_startup_auto_layout.png",
        )?;

        // ====================================================================
        // Scenario (b): Search active with some nodes dimmed
        // Filter query "scarlett" active. Node 10 matches; all non-matching
        // nodes and their links are dimmed (opacity 0.28). Layout unchanged.
        // ====================================================================
        println!("Generating scenario (b): search active with some nodes dimmed...");
        let mut app_b = create_demo_app();
        let _ = app_b.update(Message::SearchQueryChanged("scarlett".to_string()));
        let bytes_b = render_app_frame(&mut renderer, &app_b, None)?;
        save_both_names(
            &mut generated_paths,
            &bytes_b,
            "b_search_active_dimmed.png",
            "scenario_b_search_dimmed.png",
        )?;

        // ====================================================================
        // Scenario (c): Hovered default link showing its tooltip near cursor
        // Hover pointer over default wire Firefox (Node 20) -> Built-in Audio (Node 50).
        // Tooltip is tightly fit, anchored near cursor, inside canvas bounds.
        // ====================================================================
        println!("Generating scenario (c): hovered default link showing tooltip near cursor...");
        let app_c = create_demo_app();
        let hover_pos_c = Point::new(530.0, 186.0);
        let bytes_c = render_app_frame(&mut renderer, &app_c, Some(hover_pos_c))?;
        save_both_names(
            &mut generated_paths,
            &bytes_c,
            "c_hovered_default_link_tooltip.png",
            "scenario_c_hovered_default_link_tooltip.png",
        )?;

        // ====================================================================
        // Scenario (d): Hovered/selected non-default link showing mini routing-matrix badge
        // Firefox (20) -> Built-in Audio (50) link is made non-default (single channel FL->FL),
        // selected and hovered, showing mini routing-matrix badge and edge inspector.
        // ====================================================================
        println!("Generating scenario (d): hovered/selected non-default link...");
        let mut app_d = create_demo_app();
        // Remove link 904 (FR->FR), leaving only 903 (FL->FL) -> custom non-default link
        app_d.graph.links.retain(|_, link| link.id != 904);
        let non_default_edge = EdgeKey {
            output_node_serial: 20,
            input_node_serial: 50,
        };
        app_d.selected_edge = Some(non_default_edge);
        let hover_pos_d = Point::new(530.0, 186.0);
        let bytes_d = render_app_frame(&mut renderer, &app_d, Some(hover_pos_d))?;
        save_both_names(
            &mut generated_paths,
            &bytes_d,
            "d_hovered_selected_non_default_link.png",
            "scenario_d_non_default_link_badge.png",
        )?;

        // ====================================================================
        // Scenario (e): Multi-selection of 3+ nodes
        // Nodes 10 (Scarlett Solo), 20 (Firefox), and 30 (Spotify) are selected
        // and visibly highlighted on the canvas overlay.
        // ====================================================================
        println!("Generating scenario (e): multi-selection of 3+ nodes...");
        let mut app_e = create_demo_app();
        app_e.selected_nodes.insert(10);
        app_e.selected_nodes.insert(20);
        app_e.selected_nodes.insert(30);
        let bytes_e = render_app_frame(&mut renderer, &app_e, None)?;
        save_both_names(
            &mut generated_paths,
            &bytes_e,
            "e_multi_selection_3_nodes.png",
            "scenario_e_multi_selection.png",
        )?;

        // ====================================================================
        // Scenario (f): Inspector for demo device showing routes and profiles
        // Inspecting Node 50 (Built-in Audio Headphones) showing device routes
        // and profiles with active, available, and unavailable entries distinct.
        // ====================================================================
        println!("Generating scenario (f): inspector for demo device...");
        let mut app_f = create_demo_app();
        app_f.selected_nodes.insert(50);
        let bytes_f = render_app_frame(&mut renderer, &app_f, None)?;
        save_both_names(
            &mut generated_paths,
            &bytes_f,
            "f_inspector_demo_device.png",
            "scenario_f_device_inspector_routes_profiles.png",
        )?;

        // ====================================================================
        // Scenario (g): Layout before and after adding a node
        // Demonstrating that newly added node is placed in a free spot and
        // all pre-existing node positions are strictly unchanged.
        // ====================================================================
        println!("Generating scenario (g): layout before and after adding a node...");
        let mut app_g = create_demo_app();
        let original_positions = app_g.node_positions.clone();

        // g1: before adding node
        let bytes_g1 = render_app_frame(&mut renderer, &app_g, None)?;
        save_both_names(
            &mut generated_paths,
            &bytes_g1,
            "g1_layout_before_adding_node.png",
            "scenario_g1_layout_before_add_node.png",
        )?;

        // Add node 70: OBS Studio
        let mut obs_props = BTreeMap::new();
        obs_props.insert("object.serial".to_string(), "70".to_string());
        obs_props.insert("node.name".to_string(), "obs_studio".to_string());
        obs_props.insert("node.description".to_string(), "OBS Studio Audio".to_string());
        obs_props.insert("media.class".to_string(), "Stream/Output/Audio".to_string());
        let obs_node = PwNode {
            id: 70,
            object_serial: 70,
            name: "obs_studio".to_string(),
            description: "OBS Studio Audio".to_string(),
            app_name: Some("OBS Studio".to_string()),
            media_name: Some("Broadcast Stream".to_string()),
            media_class: "Stream/Output/Audio".to_string(),
            category: NodeCategory::AudioOutputStream,
            input_ports: Vec::new(),
            output_ports: Vec::new(),
            state: "running".to_string(),
            volume: Some(1.0),
            muted: Some(false),
            channel_volumes: vec![1.0, 1.0],
            channel_names: vec!["FL".to_string(), "FR".to_string()],
            props: obs_props,
            params: Vec::new(),
            device_id: None,
            device_serial: None,
        };
        app_g.graph.nodes.insert(70, obs_node);
        app_g.ensure_positions_for_new_nodes();

        // Verify position stability
        for (&serial, &orig_pos) in &original_positions {
            assert_eq!(
                app_g.node_positions.get(&serial),
                Some(&orig_pos),
                "Pre-existing node {serial} moved after adding new node!"
            );
        }
        assert!(
            app_g.node_positions.contains_key(&70),
            "New node 70 must be assigned a position"
        );

        // g2: after adding node
        let bytes_g2 = render_app_frame(&mut renderer, &app_g, None)?;
        save_both_names(
            &mut generated_paths,
            &bytes_g2,
            "g2_layout_after_adding_node.png",
            "scenario_g2_layout_after_add_node.png",
        )?;

        // ====================================================================
        // Scenario (h): Top bar while connected
        // Toolbar with connected health: green "PipeWire Live" indicator,
        // no "Nudge Apart" button, no "Refresh PipeWire" button.
        // ====================================================================
        println!("Generating scenario (h): top bar while connected...");
        let (mut app_h, _) = PipewireApp::new();
        app_h.graph = create_demo_graph();
        app_h.compute_auto_layout();
        app_h.backend.is_demo = false;
        let (cmd_tx, _cmd_rx) = pipewire::channel::channel::<crate::backend::PwCommand>();
        app_h.backend.cmd_sender = Some(cmd_tx);
        app_h.backend.last_status = "PipeWire Live (6 nodes, 10 ports, 8 links)".to_string();

        assert!(
            app_h.is_connection_healthy(),
            "Connection must evaluate to healthy"
        );
        assert!(
            !app_h.should_show_refresh_button(),
            "Refresh button must not be shown when connected"
        );

        let bytes_h = render_app_frame(&mut renderer, &app_h, None)?;
        save_both_names(
            &mut generated_paths,
            &bytes_h,
            "h_top_bar_connected.png",
            "scenario_h_top_bar_connected.png",
        )?;

        println!(
            "Successfully generated all {} screenshot files in {:?}",
            generated_paths.len(),
            output_dir
        );

        Ok(generated_paths)
    })
}
