mod backend;
mod canvas_view;
mod model;

use backend::{Backend, PwEvent};
use canvas_view::{CanvasMessage, GraphCanvas, NODE_HEIGHT, NODE_WIDTH};
use iced::widget::{
    button, canvas::Canvas, checkbox, column, container, row, rule, scrollable, slider, space,
    text, Column, Row,
};
use iced::{border, font, Alignment, Color, Element, Length, Point, Subscription, Task, Theme, Vector};
use iced_fonts::{lucide, LUCIDE_FONT_BYTES};
use model::{EdgeKey, NodeCategory, PwGraph, PwNode, PwParamValue, PwPort, RoutingPreset};
use std::collections::{BTreeMap, HashSet};

pub fn main() -> iced::Result {
    iced::application(PipewireApp::new, PipewireApp::update, PipewireApp::view)
        .title("PipeWire Node Graph Control")
        .subscription(PipewireApp::subscription)
        .theme(PipewireApp::theme)
        .window_size((1380.0, 860.0))
        .run()
}

#[derive(Debug, Clone)]
pub enum Message {
    FontLoaded(Result<(), font::Error>),
    Pw(PwEvent),
    Canvas(CanvasMessage),
    RefreshGraph,
    AutoLayout,
    NudgeApart,
    ResetView,
    ToggleFilterAudio(bool),
    ToggleFilterMidi(bool),
    ToggleFilterVideo(bool),
    ToggleFilterInternal(bool),
    ToggleShowMonitorPorts(bool),
    ToggleSectionPerChannelVolumes {
        node_serial: u64,
        enabled: bool,
    },
    ToggleSectionNodeParams(bool),
    ToggleSectionNodeProps(bool),
    ToggleSectionPortProps(bool),
    ToggleSectionLinkProps(bool),
    ToggleNodeMute {
        node_serial: u64,
        muted: bool,
    },
    ChangeNodeVolume {
        node_serial: u64,
        volume: f32,
    },
    ChangeNodeChannelVolume {
        node_serial: u64,
        channel_index: usize,
        volume: f32,
    },
    SetNodeParamBool {
        node_serial: u64,
        key_id: u32,
        value: bool,
    },
    ChangeNodeParamFloat {
        node_serial: u64,
        key_id: u32,
        value: f32,
    },
    ChangeNodeParamDouble {
        node_serial: u64,
        key_id: u32,
        value: f64,
    },
    StepNodeParamInt {
        node_serial: u64,
        key_id: u32,
        current: i32,
        delta: i32,
    },
    StepNodeParamLong {
        node_serial: u64,
        key_id: u32,
        current: i64,
        delta: i64,
    },
    TogglePortConnection {
        output_node_serial: u64,
        output_port_serial: u64,
        input_node_serial: u64,
        input_port_serial: u64,
    },
    ApplyPreset {
        output_node_serial: u64,
        input_node_serial: u64,
        preset: RoutingPreset,
    },
    InspectNodePair {
        output_node_serial: u64,
        input_node_serial: u64,
    },
    DisconnectEdge {
        output_node_serial: u64,
        input_node_serial: u64,
    },
    SelectNode(u64),
    WindowClosed(iced::window::Id),
}

pub struct PipewireApp {
    backend: Backend,
    graph: PwGraph,
    node_positions: BTreeMap<u64, Point>,
    selected_edge: Option<EdgeKey>,
    selected_node: Option<u64>,
    show_audio: bool,
    show_midi: bool,
    show_video: bool,
    show_internal: bool,
    show_monitor_ports: bool,
    per_channel_volume_overrides: BTreeMap<u64, bool>,
    show_node_params: bool,
    show_node_props: bool,
    show_port_props: bool,
    show_link_props: bool,
    pan: Vector,
    zoom: f32,
    error_banner: Option<String>,
}

impl PipewireApp {
    pub fn new() -> (Self, Task<Message>) {
        let app = Self {
            backend: Backend::default(),
            graph: PwGraph::default(),
            node_positions: BTreeMap::new(),
            selected_edge: None,
            selected_node: None,
            show_audio: true,
            show_midi: false,
            show_video: false,
            show_internal: false,
            show_monitor_ports: false,
            per_channel_volume_overrides: BTreeMap::new(),
            show_node_params: false,
            show_node_props: false,
            show_port_props: false,
            show_link_props: false,
            pan: Vector::new(40.0, 40.0),
            zoom: 1.0,
            error_banner: None,
        };

        (app, font::load(LUCIDE_FONT_BYTES).map(Message::FontLoaded))
    }

    pub fn theme(&self) -> Theme {
        Theme::Dark
    }

    /// Event-driven PipeWire subscription via `pipewire-rs` MainLoop (0% idle CPU usage).
    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            Subscription::run(backend::pipewire_subscription).map(Message::Pw),
            iced::window::close_events().map(Message::WindowClosed),
        ])
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

    fn ensure_positions_for_new_nodes(&mut self) {
        let missing: Vec<u64> = self
            .graph
            .nodes
            .keys()
            .copied()
            .filter(|serial| !self.node_positions.contains_key(serial))
            .collect();
        if !missing.is_empty() {
            self.compute_auto_layout();
        }
    }

    fn compute_auto_layout(&mut self) {
        let visible = self.visible_node_serials();
        let mut columns: [Vec<u64>; 3] = [Vec::new(), Vec::new(), Vec::new()];

        for (&serial, node) in &self.graph.nodes {
            if !visible.contains(&serial) {
                continue;
            }
            let col = node.category.column_index().min(2);
            columns[col].push(serial);
        }

        let col_x = [40.0, 380.0, 720.0];
        let row_spacing = NODE_HEIGHT + 34.0;

        for (col_idx, node_serials) in columns.iter().enumerate() {
            for (row_idx, &node_serial) in node_serials.iter().enumerate() {
                let x = col_x[col_idx];
                let y = 40.0 + row_idx as f32 * row_spacing;
                self.node_positions.insert(node_serial, Point::new(x, y));
            }
        }
        self.resolve_node_overlaps(None);
    }

    pub fn resolve_node_overlaps(&mut self, priority_serial: Option<u64>) {
        let visible = self.visible_node_serials();
        // Halved padding (12px padding around nodes)
        let min_dist_x = NODE_WIDTH + 12.0; // 247.0 px
        let min_dist_y = NODE_HEIGHT + 12.0; // 108.0 px

        let active_serials: Vec<u64> = self
            .node_positions
            .keys()
            .copied()
            .filter(|s| visible.contains(s))
            .collect();

        // The grabbed/priority node must be pinned at its drop position.
        // It must NEVER be nudged away by other nodes; other nodes must yield to it.
        let pinned_pos = priority_serial.and_then(|prio| self.node_positions.get(&prio).copied());

        for _pass in 0..30 {
            let mut any_overlap = false;

            for i in 0..active_serials.len() {
                for j in (i + 1)..active_serials.len() {
                    let s_a = active_serials[i];
                    let s_b = active_serials[j];

                    let (Some(&pos_a), Some(&pos_b)) = (
                        self.node_positions.get(&s_a),
                        self.node_positions.get(&s_b),
                    ) else {
                        continue;
                    };

                    let dx = pos_b.x - pos_a.x;
                    let dy = pos_b.y - pos_a.y;
                    let overlap_x = min_dist_x - dx.abs();
                    let overlap_y = min_dist_y - dy.abs();

                    if overlap_x > 0.0 && overlap_y > 0.0 {
                        any_overlap = true;

                        // Dimension-aware choice between row (X) and column (Y) nudging:
                        // Compare normalized displacement relative to the node dimensions:
                        // ux = |dx| / min_dist_x, uy = |dy| / min_dist_y.
                        // If uy > ux, the relative vertical displacement is larger, indicating
                        // column alignment. If ux >= uy, the relative horizontal displacement is larger,
                        // indicating row alignment.
                        let ux = dx.abs() / min_dist_x;
                        let uy = dy.abs() / min_dist_y;
                        let push_along_y = uy > ux;

                        if push_along_y {
                            let dir = if dy >= 0.0 { 1.0 } else { -1.0 };
                            let shift = overlap_y + 2.0;

                            if Some(s_a) == priority_serial {
                                // s_a is pinned; only move s_b
                                let mut new_b_y = if dir >= 0.0 {
                                    pos_a.y + min_dist_y
                                } else {
                                    pos_a.y - min_dist_y
                                };
                                if new_b_y < 20.0 {
                                    new_b_y = pos_a.y + min_dist_y;
                                }
                                self.node_positions.insert(s_b, Point::new(pos_b.x, new_b_y));
                            } else if Some(s_b) == priority_serial {
                                // s_b is pinned; only move s_a
                                let mut new_a_y = if dir >= 0.0 {
                                    pos_b.y - min_dist_y
                                } else {
                                    pos_b.y + min_dist_y
                                };
                                if new_a_y < 20.0 {
                                    new_a_y = pos_b.y + min_dist_y;
                                }
                                self.node_positions.insert(s_a, Point::new(pos_a.x, new_a_y));
                            } else {
                                // Neither is pinned; nudge both apart
                                let half = shift * 0.5;
                                let new_b_y = (pos_b.y + dir * half).max(20.0);
                                let new_a_y = (pos_a.y - dir * half).max(20.0);
                                self.node_positions.insert(s_b, Point::new(pos_b.x, new_b_y));
                                self.node_positions.insert(s_a, Point::new(pos_a.x, new_a_y));
                            }
                        } else {
                            let dir = if dx >= 0.0 { 1.0 } else { -1.0 };
                            let shift = overlap_x + 2.0;

                            if Some(s_a) == priority_serial {
                                // s_a is pinned; only move s_b
                                let mut new_b_x = if dir >= 0.0 {
                                    pos_a.x + min_dist_x
                                } else {
                                    pos_a.x - min_dist_x
                                };
                                if new_b_x < 20.0 {
                                    new_b_x = pos_a.x + min_dist_x;
                                }
                                self.node_positions.insert(s_b, Point::new(new_b_x, pos_b.y));
                            } else if Some(s_b) == priority_serial {
                                // s_b is pinned; only move s_a
                                let mut new_a_x = if dir >= 0.0 {
                                    pos_b.x - min_dist_x
                                } else {
                                    pos_b.x + min_dist_x
                                };
                                if new_a_x < 20.0 {
                                    new_a_x = pos_b.x + min_dist_x;
                                }
                                self.node_positions.insert(s_a, Point::new(new_a_x, pos_a.y));
                            } else {
                                // Neither is pinned; nudge both apart
                                let half = shift * 0.5;
                                let new_b_x = (pos_b.x + dir * half).max(20.0);
                                let new_a_x = (pos_a.x - dir * half).max(20.0);
                                self.node_positions.insert(s_b, Point::new(new_b_x, pos_b.y));
                                self.node_positions.insert(s_a, Point::new(new_a_x, pos_a.y));
                            }
                        }
                    }
                }
            }

            // Always enforce that the grabbed/priority node was not moved
            if let (Some(prio), Some(orig_pos)) = (priority_serial, pinned_pos) {
                self.node_positions.insert(prio, orig_pos);
            }

            if !any_overlap {
                break;
            }
        }

        // Final enforcement
        if let (Some(prio), Some(orig_pos)) = (priority_serial, pinned_pos) {
            self.node_positions.insert(prio, orig_pos);
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        self.error_banner = None;

        match message {
            Message::FontLoaded(_) => {}
            Message::Pw(pw_event) => match pw_event {
                PwEvent::Connected(sender) => {
                    self.backend.is_demo = false;
                    self.backend.cmd_sender = Some(sender.0);
                    self.backend.last_status =
                        String::from("Connected to PipeWire via pipewire-rs");
                }
                PwEvent::GraphUpdated(new_graph) => {
                    self.graph = new_graph;
                    self.ensure_positions_for_new_nodes();
                    if !self.backend.is_demo {
                        self.backend.last_status = format!(
                            "PipeWire Live ({} nodes, {} ports, {} links)",
                            self.graph.nodes.len(),
                            self.graph.ports.len(),
                            self.graph.links.len()
                        );
                    }
                }
                PwEvent::Status(status) => {
                    self.backend.last_status = status;
                }
                PwEvent::Disconnected(err) => {
                    self.backend.is_demo = true;
                    self.backend.cmd_sender = None;
                    self.backend.last_status = format!(
                        "PipeWire daemon unreachable ({err}) — showing interactive Demo Graph"
                    );
                }
            },
            Message::RefreshGraph => {
                self.backend.request_sync();
            }
            Message::AutoLayout => {
                self.compute_auto_layout();
            }
            Message::NudgeApart => {
                self.resolve_node_overlaps(None);
            }
            Message::ResetView => {
                self.pan = Vector::new(40.0, 40.0);
                self.zoom = 1.0;
            }
            Message::ToggleFilterAudio(val) => {
                self.show_audio = val;
                self.compute_auto_layout();
            }
            Message::ToggleFilterMidi(val) => {
                self.show_midi = val;
                self.compute_auto_layout();
            }
            Message::ToggleFilterVideo(val) => {
                self.show_video = val;
                self.compute_auto_layout();
            }
            Message::ToggleFilterInternal(val) => {
                self.show_internal = val;
                self.compute_auto_layout();
            }
            Message::ToggleShowMonitorPorts(val) => {
                self.show_monitor_ports = val;
            }
            Message::ToggleSectionPerChannelVolumes {
                node_serial,
                enabled,
            } => {
                self.per_channel_volume_overrides
                    .insert(node_serial, enabled);
                if !enabled {
                    if let Some(master_vol) = self
                        .graph
                        .nodes
                        .get(&node_serial)
                        .map(|n| n.effective_volume())
                    {
                        if let Err(e) =
                            self.backend
                                .set_node_volume(&mut self.graph, node_serial, master_vol)
                        {
                            self.error_banner = Some(e);
                        }
                    }
                }
            }
            Message::ToggleSectionNodeParams(val) => {
                self.show_node_params = val;
            }
            Message::ToggleSectionNodeProps(val) => {
                self.show_node_props = val;
            }
            Message::ToggleSectionPortProps(val) => {
                self.show_port_props = val;
            }
            Message::ToggleSectionLinkProps(val) => {
                self.show_link_props = val;
            }
            Message::ToggleNodeMute { node_serial, muted } => {
                if let Err(e) = self.backend.set_node_mute(&mut self.graph, node_serial, muted) {
                    self.error_banner = Some(e);
                }
            }
            Message::ChangeNodeVolume {
                node_serial,
                volume,
            } => {
                if let Err(e) = self
                    .backend
                    .set_node_volume(&mut self.graph, node_serial, volume)
                {
                    self.error_banner = Some(e);
                }
            }
            Message::ChangeNodeChannelVolume {
                node_serial,
                channel_index,
                volume,
            } => {
                if let Err(e) = self.backend.set_node_channel_volume(
                    &mut self.graph,
                    node_serial,
                    channel_index,
                    volume,
                ) {
                    self.error_banner = Some(e);
                }
            }
            Message::SetNodeParamBool {
                node_serial,
                key_id,
                value,
            } => {
                if let Err(e) = self.backend.set_node_prop_param(
                    &mut self.graph,
                    node_serial,
                    key_id,
                    PwParamValue::Bool(value),
                ) {
                    self.error_banner = Some(e);
                }
            }
            Message::ChangeNodeParamFloat {
                node_serial,
                key_id,
                value,
            } => {
                if let Err(e) = self.backend.set_node_prop_param(
                    &mut self.graph,
                    node_serial,
                    key_id,
                    PwParamValue::Float(value),
                ) {
                    self.error_banner = Some(e);
                }
            }
            Message::ChangeNodeParamDouble {
                node_serial,
                key_id,
                value,
            } => {
                if let Err(e) = self.backend.set_node_prop_param(
                    &mut self.graph,
                    node_serial,
                    key_id,
                    PwParamValue::Double(value),
                ) {
                    self.error_banner = Some(e);
                }
            }
            Message::StepNodeParamInt {
                node_serial,
                key_id,
                current,
                delta,
            } => {
                let next_val = current.saturating_add(delta);
                if let Err(e) = self.backend.set_node_prop_param(
                    &mut self.graph,
                    node_serial,
                    key_id,
                    PwParamValue::Int(next_val),
                ) {
                    self.error_banner = Some(e);
                }
            }
            Message::StepNodeParamLong {
                node_serial,
                key_id,
                current,
                delta,
            } => {
                let next_val = current.saturating_add(delta);
                if let Err(e) = self.backend.set_node_prop_param(
                    &mut self.graph,
                    node_serial,
                    key_id,
                    PwParamValue::Long(next_val),
                ) {
                    self.error_banner = Some(e);
                }
            }
            Message::SelectNode(node_serial) => {
                self.per_channel_volume_overrides.remove(&node_serial);
                self.selected_node = Some(node_serial);
                self.selected_edge = None;
            }
            Message::WindowClosed(_id) => {
                self.backend.quit();
            }
            Message::InspectNodePair {
                output_node_serial,
                input_node_serial,
            } => {
                self.selected_edge = Some(EdgeKey {
                    output_node_serial,
                    input_node_serial,
                });
                self.selected_node = None;
            }
            Message::DisconnectEdge {
                output_node_serial,
                input_node_serial,
            } => {
                if let Err(e) = self.backend.disconnect_nodes(
                    &mut self.graph,
                    output_node_serial,
                    input_node_serial,
                ) {
                    self.error_banner = Some(e);
                }
            }
            Message::TogglePortConnection {
                output_node_serial,
                output_port_serial,
                input_node_serial,
                input_port_serial,
            } => {
                if let Err(e) = self.backend.toggle_port_link(
                    &mut self.graph,
                    output_node_serial,
                    output_port_serial,
                    input_node_serial,
                    input_port_serial,
                ) {
                    self.error_banner = Some(e);
                }
            }
            Message::ApplyPreset {
                output_node_serial,
                input_node_serial,
                preset,
            } => {
                if let Err(e) = self.backend.apply_preset(
                    &mut self.graph,
                    output_node_serial,
                    input_node_serial,
                    preset,
                ) {
                    self.error_banner = Some(e);
                }
            }
            Message::Canvas(canvas_msg) => match canvas_msg {
                CanvasMessage::SelectEdge(edge_key) => {
                    self.selected_edge = Some(edge_key);
                    self.selected_node = None;
                }
                CanvasMessage::SelectNode(node_serial) => {
                    self.per_channel_volume_overrides.remove(&node_serial);
                    self.selected_node = Some(node_serial);
                    self.selected_edge = None;
                }
                CanvasMessage::ClearSelection => {
                    self.selected_edge = None;
                    self.selected_node = None;
                }
                CanvasMessage::MoveNode(node_serial, new_pos) => {
                    self.node_positions.insert(node_serial, new_pos);
                }
                CanvasMessage::FinishMoveNode(node_serial) => {
                    self.resolve_node_overlaps(Some(node_serial));
                }
                CanvasMessage::ConnectNodes {
                    output_node_serial,
                    input_node_serial,
                } => {
                    match self.backend.connect_nodes_default(
                        &mut self.graph,
                        output_node_serial,
                        input_node_serial,
                    ) {
                        Ok(()) => {
                            self.selected_edge = Some(EdgeKey {
                                output_node_serial,
                                input_node_serial,
                            });
                            self.selected_node = None;
                        }
                        Err(e) => {
                            self.error_banner = Some(e);
                        }
                    }
                }
                CanvasMessage::DisconnectNodes {
                    output_node_serial,
                    input_node_serial,
                } => {
                    if let Err(e) = self.backend.disconnect_nodes(
                        &mut self.graph,
                        output_node_serial,
                        input_node_serial,
                    ) {
                        self.error_banner = Some(e);
                    }
                }
                CanvasMessage::PanCanvas(delta) => {
                    self.pan = self.pan + delta;
                }
                CanvasMessage::ZoomCanvas(scroll_y, cursor_pos) => {
                    let old_zoom = self.zoom;
                    let factor = if scroll_y > 0.0 { 1.1 } else { 0.9 };
                    let new_zoom = (old_zoom * factor).clamp(0.45, 2.2);

                    let world_before = Point::new(
                        (cursor_pos.x - self.pan.x) / old_zoom,
                        (cursor_pos.y - self.pan.y) / old_zoom,
                    );
                    self.zoom = new_zoom;
                    self.pan = Vector::new(
                        cursor_pos.x - world_before.x * new_zoom,
                        cursor_pos.y - world_before.y * new_zoom,
                    );
                }
            },
        }

        Task::none()
    }

    pub fn view(&self) -> Element<'_, Message> {
        let visible_nodes = self.visible_node_serials();

        let top_bar = self.view_top_bar();

        let canvas_widget: Element<'_, CanvasMessage> = Canvas::new(GraphCanvas {
            graph: &self.graph,
            visible_nodes: visible_nodes.clone(),
            node_positions: &self.node_positions,
            selected_edge: self.selected_edge.as_ref(),
            selected_node: self.selected_node,
            pan: self.pan,
            zoom: self.zoom,
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
        let canvas_widget = canvas_widget.map(Message::Canvas);

        let inspector_panel = self.view_inspector_panel(&visible_nodes);

        let main_row = row![
            container(canvas_widget)
                .width(Length::FillPortion(7))
                .height(Length::Fill),
            container(inspector_panel)
                .width(Length::Fixed(460.0))
                .height(Length::Fill)
                .padding(14)
        ]
        .height(Length::Fill);

        let status_bar = self.view_status_bar();

        column![
            top_bar,
            rule::horizontal(1),
            main_row,
            rule::horizontal(1),
            status_bar
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    /// Renders a parameter or property value, replacing an empty string with a rounded `empty` badge.
    fn view_value_or_empty_badge<'a>(val: String, text_color: Color) -> Element<'a, Message> {
        if val.is_empty() {
            let badge = container(
                text("empty")
                    .size(10)
                    .color(Color::from_rgb(0.68, 0.74, 0.84)),
            )
            .padding([1, 7])
            .style(|_theme| container::Style {
                background: Some(Color::from_rgba(0.24, 0.28, 0.36, 0.85).into()),
                border: border::rounded(6)
                    .width(1.0)
                    .color(Color::from_rgba(0.45, 0.52, 0.65, 0.45)),
                ..Default::default()
            });

            container(badge)
                .width(Length::FillPortion(3))
                .align_x(Alignment::Start)
                .into()
        } else {
            text(val)
                .size(11)
                .width(Length::FillPortion(3))
                .color(text_color)
                .into()
        }
    }

    fn view_top_bar(&self) -> Element<'_, Message> {
        let mode_badge = if self.backend.is_demo {
            text("● Demo Graph").color(Color::from_rgb(0.96, 0.74, 0.30))
        } else {
            text("● PipeWire Live").color(Color::from_rgb(0.38, 0.88, 0.56))
        };

        let filters = row![
            text("Show:").size(13).color(Color::from_rgb(0.7, 0.74, 0.82)),
            checkbox(self.show_audio)
                .label("Audio")
                .on_toggle(Message::ToggleFilterAudio)
                .size(16)
                .text_size(13),
            checkbox(self.show_midi)
                .label("MIDI")
                .on_toggle(Message::ToggleFilterMidi)
                .size(16)
                .text_size(13),
            checkbox(self.show_video)
                .label("Video")
                .on_toggle(Message::ToggleFilterVideo)
                .size(16)
                .text_size(13),
            checkbox(self.show_internal)
                .label("Internal")
                .on_toggle(Message::ToggleFilterInternal)
                .size(16)
                .text_size(13),
        ]
        .spacing(14)
        .align_y(Alignment::Center);

        let actions = row![
            button(text("Auto-Layout").size(13))
                .on_press(Message::AutoLayout)
                .padding([6, 12]),
            button(text("Nudge Apart").size(13))
                .on_press(Message::NudgeApart)
                .padding([6, 12]),
            button(text("Reset View").size(13))
                .on_press(Message::ResetView)
                .padding([6, 12]),
            button(text("Refresh PipeWire").size(13))
                .on_press(Message::RefreshGraph)
                .padding([6, 12]),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        container(
            row![
                text("PipeWire Graph Control").size(18),
                mode_badge.size(13),
                space::horizontal(),
                filters,
                space::horizontal(),
                actions,
            ]
            .spacing(16)
            .align_y(Alignment::Center),
        )
        .padding([10, 16])
        .into()
    }

    fn view_status_bar(&self) -> Element<'_, Message> {
        let status_text = if let Some(err) = &self.error_banner {
            text(format!("⚠ {err}"))
                .size(12)
                .color(Color::from_rgb(0.98, 0.45, 0.45))
        } else {
            text(&self.backend.last_status)
                .size(12)
                .color(Color::from_rgb(0.68, 0.72, 0.80))
        };

        let edge_count = self.graph.collapsed_edges().len();
        let raw_link_count = self.graph.links.len();

        container(
            row![
                status_text,
                space::horizontal(),
                text(format!(
                    "Connections: {edge_count}   |   Port Links: {raw_link_count}   |   Zoom: {:.0}%",
                    self.zoom * 100.0
                ))
                .size(12)
                .color(Color::from_rgb(0.60, 0.65, 0.74)),
            ]
            .align_y(Alignment::Center),
        )
        .padding([6, 16])
        .into()
    }

    fn view_inspector_panel(&self, visible_nodes: &HashSet<u64>) -> Element<'_, Message> {
        let content: Element<'_, Message> = if let Some(edge_key) = &self.selected_edge {
            self.view_edge_routing_matrix(edge_key)
        } else if let Some(node_serial) = self.selected_node {
            self.view_node_inspector(node_serial, visible_nodes)
        } else {
            self.view_overview_inspector()
        };

        // Reserve 18px right padding inside the scrollable container so the vertical scrollbar
        // never overlaps or clips right-aligned controls and text.
        let padded = container(content)
            .width(Length::Fill)
            .padding(iced::Padding {
                top: 2.0,
                right: 18.0,
                bottom: 8.0,
                left: 2.0,
            });

        scrollable(padded)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    /// Returns whether the split Per-Channel volume mixer should be shown for `node`:
    /// defaults to `false` (Master volume) when all channel volumes are equal, and `true`
    /// (Split Per-Channel volume mixer) when channel volumes differ, unless manually toggled.
    fn is_per_channel_volume_active(&self, node: &PwNode) -> bool {
        if node.channel_volumes.is_empty() {
            return false;
        }
        self.per_channel_volume_overrides
            .get(&node.object_serial)
            .copied()
            .unwrap_or_else(|| node.has_unequal_channel_volumes())
    }

    /// Renders a compact volume & mute bar for a node (used inside the Edge Inspector).
    fn view_compact_node_audio_row<'a>(
        &self,
        label_prefix: &str,
        node: &'a PwNode,
    ) -> Element<'a, Message> {
        let node_serial = node.object_serial;
        let is_muted = node.muted.unwrap_or(false);
        let vol = node.effective_volume();
        let show_per_channel = self.is_per_channel_volume_active(node);

        let mute_color = if is_muted {
            Color::from_rgb(0.98, 0.48, 0.48)
        } else {
            Color::from_rgb(0.85, 0.90, 0.96)
        };
        let mute_icon = if is_muted {
            lucide::volume_x().size(12).color(mute_color)
        } else {
            lucide::volume_two().size(12).color(mute_color)
        };
        let mute_btn = button(
            row![
                mute_icon,
                text(if is_muted { "Muted" } else { "Mute" })
                    .size(11)
                    .color(mute_color),
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        )
        .padding([3, 8])
        .on_press(Message::ToggleNodeMute {
            node_serial,
            muted: !is_muted,
        });

        let inspect_btn = button(text("Inspect").size(11))
            .padding([3, 7])
            .on_press(Message::SelectNode(node_serial));

        let mut col = Column::new().width(Length::Fill).spacing(4);
        let mut header_row = Row::new().spacing(6).align_y(Alignment::Center);
        header_row = header_row.push(
            text(format!("{label_prefix}: {}", node.title()))
                .size(12)
                .width(Length::Fill)
                .color(Color::from_rgb(0.86, 0.90, 0.96)),
        );
        if !node.channel_volumes.is_empty() {
            header_row = header_row.push(
                checkbox(show_per_channel)
                    .label("Split")
                    .on_toggle(move |enabled| Message::ToggleSectionPerChannelVolumes {
                        node_serial,
                        enabled,
                    })
                    .size(13)
                    .text_size(11),
            );
        }
        header_row = header_row.push(mute_btn);
        header_row = header_row.push(inspect_btn);
        col = col.push(header_row);

        if show_per_channel {
            for (ch_idx, &ch_vol) in node.channel_volumes.iter().enumerate() {
                let ch_label = node.channel_label(ch_idx);
                col = col.push(
                    row![
                        container(
                            text(ch_label)
                                .size(11)
                                .color(Color::from_rgb(0.55, 0.85, 0.96)),
                        )
                        .width(Length::Fixed(42.0)),
                        slider(0.0..=1.5, ch_vol, move |v| {
                            Message::ChangeNodeChannelVolume {
                                node_serial,
                                channel_index: ch_idx,
                                volume: v,
                            }
                        })
                        .step(0.01_f32)
                        .width(Length::Fill),
                        container(
                            text(format!("{:>3.0}%", ch_vol * 100.0))
                                .size(11)
                                .color(if is_muted {
                                    Color::from_rgb(0.75, 0.50, 0.50)
                                } else {
                                    Color::from_rgb(0.78, 0.86, 0.95)
                                }),
                        )
                        .width(Length::Fixed(42.0)),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                );
            }
        } else {
            let vol_slider = slider(0.0..=1.5, vol, move |v| Message::ChangeNodeVolume {
                node_serial,
                volume: v,
            })
            .step(0.01_f32)
            .width(Length::Fill);

            col = col.push(
                row![
                    container(
                        text("Master")
                            .size(11)
                            .color(Color::from_rgb(0.68, 0.74, 0.82)),
                    )
                    .width(Length::Fixed(42.0)),
                    vol_slider,
                    container(
                        text(format!("{:>3.0}%", vol * 100.0))
                            .size(12)
                            .color(if is_muted {
                                Color::from_rgb(0.75, 0.50, 0.50)
                            } else {
                                Color::from_rgb(0.78, 0.86, 0.95)
                            }),
                    )
                    .width(Length::Fixed(42.0)),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        }

        col.into()
    }

    /// Renders the Port Routing Matrix & Inspector for a selected collapsed wire (or node pair).
    fn view_edge_routing_matrix(&self, edge_key: &EdgeKey) -> Element<'_, Message> {
        let (Some(out_node), Some(in_node)) = (
            self.graph.nodes.get(&edge_key.output_node_serial),
            self.graph.nodes.get(&edge_key.input_node_serial),
        ) else {
            return column![text("Selected wire nodes are no longer present.").size(14)]
                .spacing(10)
                .into();
        };

        let active_links: Vec<_> = self
            .graph
            .links
            .values()
            .filter(|l| {
                l.output_node_serial == edge_key.output_node_serial
                    && l.input_node_serial == edge_key.input_node_serial
            })
            .cloned()
            .collect();

        let topology_summary = self.graph.describe_links(&active_links);

        let out_layout_label = out_node.output_layout_label();
        let in_layout_label = in_node.input_layout_label();

        // Header info
        let header = column![
            text("Connection Port Inspector")
                .size(17)
                .color(Color::from_rgb(0.45, 0.88, 1.0)),
            text(format!(
                "Source: {} [{}] (serial #{})",
                out_node.title(),
                out_layout_label,
                out_node.object_serial
            ))
            .size(14)
            .width(Length::Fill),
            text(format!(
                "Target:  {} [{}] (serial #{})",
                in_node.title(),
                in_layout_label,
                in_node.object_serial
            ))
            .size(14)
            .width(Length::Fill),
            text(format!(
                "Current Topology: {} ({} port link{})",
                topology_summary,
                active_links.len(),
                if active_links.len() == 1 { "" } else { "s" }
            ))
            .size(13)
            .width(Length::Fill)
            .color(Color::from_rgb(0.48, 0.90, 0.66)),
        ]
        .width(Length::Fill)
        .spacing(5);

        // Quick Volume & Mute controls for Source and Target nodes if they support audio
        let mut endpoint_audio_col = Column::new().width(Length::Fill).spacing(8);
        let mut has_endpoint_audio = false;
        if out_node.supports_audio_controls() {
            endpoint_audio_col =
                endpoint_audio_col.push(self.view_compact_node_audio_row("Source", out_node));
            has_endpoint_audio = true;
        }
        if in_node.supports_audio_controls() {
            endpoint_audio_col =
                endpoint_audio_col.push(self.view_compact_node_audio_row("Target", in_node));
            has_endpoint_audio = true;
        }

        // Context-dependent Quick Routing Presets based on source/target channel layouts
        let out_serial = out_node.object_serial;
        let in_serial = in_node.object_serial;
        let available_presets = self.graph.available_presets(out_serial, in_serial);

        // Filter output ports (whether to show monitor ports)
        let has_monitor_ports = out_node.output_ports.iter().any(|p| p.is_monitor);
        let out_ports: Vec<&PwPort> =
            if self.show_monitor_ports || !out_node.output_ports.iter().any(|p| !p.is_monitor) {
                out_node.output_ports.iter().collect()
            } else {
                out_node
                    .output_ports
                    .iter()
                    .filter(|p| !p.is_monitor)
                    .collect()
            };
        let in_ports: Vec<&PwPort> = in_node.input_ports.iter().collect();

        // Build the interactive Port Routing Matrix (Output Ports × Input Ports)
        let mut matrix_col = Column::new().width(Length::Fill).spacing(8);
        matrix_col = matrix_col.push(
            text("Port-to-Port Routing Matrix (Click to Connect / Disconnect)")
                .size(14)
                .width(Length::Fill)
                .color(Color::from_rgb(0.82, 0.85, 0.92)),
        );

        if has_monitor_ports {
            matrix_col = matrix_col.push(
                checkbox(self.show_monitor_ports)
                    .label("Include Monitor Output Ports")
                    .on_toggle(Message::ToggleShowMonitorPorts)
                    .size(15)
                    .text_size(12),
            );
        }

        if out_ports.is_empty() || in_ports.is_empty() {
            matrix_col = matrix_col.push(
                text("One of the selected nodes has no compatible ports.")
                    .size(13)
                    .color(Color::from_rgb(0.75, 0.65, 0.55)),
            );
        } else {
            let mut grid_col = Column::new().spacing(6);

            // Column headers (Input Ports of Target Node)
            let mut header_row = Row::new().spacing(6).align_y(Alignment::Center);
            header_row = header_row.push(
                container(
                    text("Out \\ In")
                        .size(12)
                        .color(Color::from_rgb(0.60, 0.65, 0.75)),
                )
                .width(Length::Fixed(105.0)),
            );

            for in_port in &in_ports {
                header_row = header_row.push(
                    container(
                        column![
                            text(in_port.display_label())
                                .size(12)
                                .color(Color::from_rgb(0.45, 0.85, 0.98)),
                            text(&in_port.name)
                                .size(10)
                                .color(Color::from_rgb(0.58, 0.62, 0.70)),
                        ]
                        .align_x(Alignment::Center),
                    )
                    .width(Length::Fixed(88.0)),
                );
            }
            grid_col = grid_col.push(header_row);

            // One row per Output Port of Source Node
            for out_port in &out_ports {
                let mut port_row = Row::new().spacing(6).align_y(Alignment::Center);

                port_row = port_row.push(
                    container(column![
                        text(out_port.display_label())
                            .size(12)
                            .color(Color::from_rgb(0.45, 0.92, 0.62)),
                        text(&out_port.name)
                            .size(10)
                            .color(Color::from_rgb(0.58, 0.62, 0.70)),
                    ])
                    .width(Length::Fixed(105.0)),
                );

                for in_port in &in_ports {
                    let is_connected = active_links.iter().any(|l| {
                        l.output_port_serial == out_port.object_serial
                            && l.input_port_serial == in_port.object_serial
                    });

                    let cell_label = if is_connected {
                        format!(
                            "● {}→{}",
                            out_port.display_label(),
                            in_port.display_label()
                        )
                    } else {
                        String::from("○  —")
                    };

                    let btn = button(
                        text(cell_label).size(12).color(if is_connected {
                            Color::from_rgb(0.35, 0.98, 0.65)
                        } else {
                            Color::from_rgb(0.55, 0.60, 0.68)
                        }),
                    )
                    .width(Length::Fixed(88.0))
                    .padding([8, 6])
                    .on_press(Message::TogglePortConnection {
                        output_node_serial: out_node.object_serial,
                        output_port_serial: out_port.object_serial,
                        input_node_serial: in_node.object_serial,
                        input_port_serial: in_port.object_serial,
                    });

                    port_row = port_row.push(btn);
                }

                grid_col = grid_col.push(port_row);
            }

            // Allow horizontal scrolling when a node has many ports (e.g. surround 5.1/7.1)
            let matrix_scroll = scrollable(
                container(grid_col).padding(iced::Padding {
                    top: 0.0,
                    right: 0.0,
                    bottom: 6.0,
                    left: 0.0,
                }),
            )
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::new(),
            ));
            matrix_col = matrix_col.push(matrix_scroll);
        }

        // Collapsible Link Properties viewer
        let mut link_props_col = Column::new().width(Length::Fill).spacing(6);
        link_props_col = link_props_col.push(
            checkbox(self.show_link_props)
                .label(format!("Show Active Link Properties ({})", active_links.len()))
                .on_toggle(Message::ToggleSectionLinkProps)
                .size(15)
                .text_size(13),
        );
        if self.show_link_props {
            if active_links.is_empty() {
                link_props_col = link_props_col.push(
                    text("No active links between these nodes.")
                        .size(12)
                        .color(Color::from_rgb(0.60, 0.64, 0.72)),
                );
            } else {
                for link in &active_links {
                    let out_port_lbl = self
                        .graph
                        .ports
                        .get(&link.output_port_serial)
                        .map(|p| p.display_label())
                        .unwrap_or_else(|| String::from("?"));
                    let in_port_lbl = self
                        .graph
                        .ports
                        .get(&link.input_port_serial)
                        .map(|p| p.display_label())
                        .unwrap_or_else(|| String::from("?"));
                    let mut single_link_col = Column::new().width(Length::Fill).spacing(2);
                    single_link_col = single_link_col.push(
                        text(format!(
                            "Link s#{} (ID #{}) — {} → {} [{}]",
                            link.object_serial, link.id, out_port_lbl, in_port_lbl, link.state
                        ))
                        .size(12)
                        .width(Length::Fill)
                        .color(Color::from_rgb(0.45, 0.88, 1.0)),
                    );
                    for (k, v) in &link.props {
                        single_link_col = single_link_col.push(
                            row![
                                text(format!("  {k}:"))
                                    .size(11)
                                    .width(Length::FillPortion(2))
                                    .color(Color::from_rgb(0.62, 0.68, 0.78)),
                                Self::view_value_or_empty_badge(
                                    v.clone(),
                                    Color::from_rgb(0.86, 0.90, 0.96),
                                ),
                            ]
                            .spacing(8)
                            .align_y(Alignment::Center),
                        );
                    }
                    link_props_col = link_props_col.push(single_link_col);
                }
            }
        }

        let mut content = Column::new().width(Length::Fill).spacing(14);
        content = content.push(header);
        if has_endpoint_audio {
            content = content.push(rule::horizontal(1));
            content = content.push(endpoint_audio_col);
        }
        content = content.push(rule::horizontal(1));

        if !available_presets.is_empty() {
            let mut presets_col = Column::new().width(Length::Fill).spacing(8);
            presets_col = presets_col.push(
                text(format!(
                    "Quick Routing Presets ({out_layout_label} → {in_layout_label})"
                ))
                .size(14)
                .width(Length::Fill)
                .color(Color::from_rgb(0.82, 0.85, 0.92)),
            );

            for chunk in available_presets.chunks(2) {
                let mut preset_row = Row::new().spacing(6).align_y(Alignment::Center);
                for &preset in chunk {
                    let is_active = self.graph.is_preset_active(out_serial, in_serial, preset);
                    let btn_label = if is_active {
                        format!("● {}", preset.label())
                    } else {
                        preset.label().to_string()
                    };
                    let btn = button(
                        text(btn_label).size(12).color(if is_active {
                            Color::from_rgb(0.35, 0.98, 0.65)
                        } else {
                            Color::from_rgb(0.88, 0.90, 0.95)
                        }),
                    )
                    .width(Length::FillPortion(1))
                    .on_press(Message::ApplyPreset {
                        output_node_serial: out_serial,
                        input_node_serial: in_serial,
                        preset,
                    })
                    .padding([5, 8]);
                    preset_row = preset_row.push(btn);
                }
                presets_col = presets_col.push(preset_row);
            }

            content = content.push(presets_col);
            content = content.push(rule::horizontal(1));
        }

        content = content.push(matrix_col);
        content = content.push(rule::horizontal(1));
        content = content.push(link_props_col);
        content.into()
    }

    /// Renders the Node Inspector when a single node is clicked.
    fn view_node_inspector(
        &self,
        node_serial: u64,
        visible_nodes: &HashSet<u64>,
    ) -> Element<'_, Message> {
        let Some(node) = self.graph.nodes.get(&node_serial) else {
            return text("Node not found.").size(14).into();
        };

        let header = column![
            text(format!("Node Inspector: {}", node.title()))
                .size(17)
                .width(Length::Fill)
                .color(Color::from_rgb(0.45, 0.88, 1.0)),
            text(format!(
                "Serial: #{} (ID #{})   |   Class: {}   |   State: {}",
                node.object_serial, node.id, node.media_class, node.state
            ))
            .size(12)
            .width(Length::Fill)
            .color(Color::from_rgb(0.68, 0.72, 0.80)),
            text(format!("Internal Name: {}", node.name))
                .size(12)
                .width(Length::Fill)
                .color(Color::from_rgb(0.58, 0.62, 0.70)),
        ]
        .width(Length::Fill)
        .spacing(4);

        let mut main_col = Column::new().width(Length::Fill).spacing(14);
        main_col = main_col.push(header);

        // 1. Per-Node Volume & Mute Controls (for audio nodes or nodes with volume/mute params)
        if node.supports_audio_controls() {
            main_col = main_col.push(rule::horizontal(1));
            let is_muted = node.muted.unwrap_or(false);
            let master_vol = node.effective_volume();
            let show_per_channel = self.is_per_channel_volume_active(node);

            let mut vol_col = Column::new().width(Length::Fill).spacing(8);

            let mut title_row = Row::new().spacing(8).align_y(Alignment::Center);
            title_row = title_row.push(
                text("Volume & Mute")
                    .size(14)
                    .width(Length::Fill)
                    .color(Color::from_rgb(0.82, 0.85, 0.92)),
            );
            if !node.channel_volumes.is_empty() {
                title_row = title_row.push(
                    checkbox(show_per_channel)
                        .label(format!("Per-Channel ({} ch)", node.channel_volumes.len()))
                        .on_toggle(move |enabled| Message::ToggleSectionPerChannelVolumes {
                            node_serial,
                            enabled,
                        })
                        .size(14)
                        .text_size(12),
                );
            }
            let mute_color = if is_muted {
                Color::from_rgb(0.98, 0.45, 0.45)
            } else {
                Color::from_rgb(0.45, 0.95, 0.65)
            };
            let mute_icon = if is_muted {
                lucide::volume_x().size(13).color(mute_color)
            } else {
                lucide::volume_two().size(13).color(mute_color)
            };
            title_row = title_row.push(
                button(
                    row![
                        mute_icon,
                        text(if is_muted { "Muted" } else { "Unmuted" })
                            .size(12)
                            .color(mute_color),
                    ]
                    .spacing(5)
                    .align_y(Alignment::Center),
                )
                .padding([4, 10])
                .on_press(Message::ToggleNodeMute {
                    node_serial,
                    muted: !is_muted,
                }),
            );
            vol_col = vol_col.push(title_row);

            // When Per-Channel mode is enabled, split channel sliders replace the Master volume slider
            if show_per_channel {
                for (ch_idx, &ch_vol) in node.channel_volumes.iter().enumerate() {
                    let ch_label = node.channel_label(ch_idx);
                    vol_col = vol_col.push(
                        row![
                            container(
                                text(ch_label)
                                    .size(11)
                                    .color(Color::from_rgb(0.55, 0.85, 0.96)),
                            )
                            .width(Length::Fixed(46.0)),
                            slider(0.0..=1.5, ch_vol, move |v| {
                                Message::ChangeNodeChannelVolume {
                                    node_serial,
                                    channel_index: ch_idx,
                                    volume: v,
                                }
                            })
                            .step(0.01_f32)
                            .width(Length::Fill),
                            container(
                                text(format!("{:>3.0}%", ch_vol * 100.0))
                                    .size(11)
                                    .color(Color::from_rgb(0.80, 0.84, 0.92)),
                            )
                            .width(Length::Fixed(44.0)),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    );
                }
            } else {
                // Master volume slider + quick level buttons
                vol_col = vol_col.push(
                    row![
                        container(
                            text("Master")
                                .size(12)
                                .color(Color::from_rgb(0.70, 0.75, 0.84)),
                        )
                        .width(Length::Fixed(46.0)),
                        slider(0.0..=1.5, master_vol, move |v| Message::ChangeNodeVolume {
                            node_serial,
                            volume: v,
                        })
                        .step(0.01_f32)
                        .width(Length::Fill),
                        container(
                            text(format!("{:>3.0}%", master_vol * 100.0))
                                .size(12)
                                .color(Color::from_rgb(0.90, 0.94, 0.98)),
                        )
                        .width(Length::Fixed(44.0)),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                );

                vol_col = vol_col.push(
                    row![
                        button(text("0%").size(11))
                            .padding([2, 8])
                            .on_press(Message::ChangeNodeVolume {
                                node_serial,
                                volume: 0.0,
                            }),
                        button(text("50%").size(11))
                            .padding([2, 8])
                            .on_press(Message::ChangeNodeVolume {
                                node_serial,
                                volume: 0.5,
                            }),
                        button(text("100%").size(11))
                            .padding([2, 8])
                            .on_press(Message::ChangeNodeVolume {
                                node_serial,
                                volume: 1.0,
                            }),
                        button(text("125%").size(11))
                            .padding([2, 8])
                            .on_press(Message::ChangeNodeVolume {
                                node_serial,
                                volume: 1.25,
                            }),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                );
            }

            main_col = main_col.push(vol_col);
        }

        // 2. Connected collapsed wires involving this node
        let edges: Vec<_> = self
            .graph
            .collapsed_edges()
            .into_iter()
            .filter(|e| {
                e.key.output_node_serial == node_serial || e.key.input_node_serial == node_serial
            })
            .collect();

        let mut wires_col = Column::new().width(Length::Fill).spacing(6);
        wires_col = wires_col.push(
            text("Connected Links")
                .size(14)
                .color(Color::from_rgb(0.82, 0.85, 0.92)),
        );

        if edges.is_empty() {
            wires_col = wires_col.push(
                text("No wires connected to this node.")
                    .size(12)
                    .color(Color::from_rgb(0.62, 0.66, 0.74)),
            );
        } else {
            for edge in edges {
                let other_label = if edge.key.output_node_serial == node_serial {
                    let target = self
                        .graph
                        .nodes
                        .get(&edge.key.input_node_serial)
                        .map(|n| n.title())
                        .unwrap_or_default();
                    format!("──► {target}  [{}]", edge.summary)
                } else {
                    let source = self
                        .graph
                        .nodes
                        .get(&edge.key.output_node_serial)
                        .map(|n| n.title())
                        .unwrap_or_default();
                    format!("◄── {source}  [{}]", edge.summary)
                };

                wires_col = wires_col.push(
                    row![
                        text(other_label).size(12).width(Length::Fill),
                        button(text("Edit Ports").size(11))
                            .padding([4, 8])
                            .on_press(Message::InspectNodePair {
                                output_node_serial: edge.key.output_node_serial,
                                input_node_serial: edge.key.input_node_serial,
                            }),
                        button(text("×").size(11))
                            .padding([4, 8])
                            .on_press(Message::DisconnectEdge {
                                output_node_serial: edge.key.output_node_serial,
                                input_node_serial: edge.key.input_node_serial,
                            }),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                );
            }
        }
        main_col = main_col.push(rule::horizontal(1));
        main_col = main_col.push(wires_col);

        // 3. Quick "Connect / Edit Port Matrix with another node" list
        if !node.output_ports.is_empty() {
            let mut quick_connect_col = Column::new().width(Length::Fill).spacing(6);
            quick_connect_col = quick_connect_col.push(
                text("Route Output From This Node To:")
                    .size(14)
                    .color(Color::from_rgb(0.82, 0.85, 0.92)),
            );
            for (&other_serial, other_node) in &self.graph.nodes {
                if other_serial == node_serial
                    || !visible_nodes.contains(&other_serial)
                    || other_node.input_ports.is_empty()
                {
                    continue;
                }
                quick_connect_col = quick_connect_col.push(
                    row![
                        text(other_node.title()).size(12).width(Length::Fill),
                        button(text("Connect").size(11))
                            .padding([3, 8])
                            .on_press(Message::Canvas(CanvasMessage::ConnectNodes {
                                output_node_serial: node_serial,
                                input_node_serial: other_serial,
                            })),
                        button(text("Port Matrix").size(11))
                            .padding([3, 8])
                            .on_press(Message::InspectNodePair {
                                output_node_serial: node_serial,
                                input_node_serial: other_serial,
                            }),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                );
            }
            main_col = main_col.push(rule::horizontal(1));
            main_col = main_col.push(quick_connect_col);
        }

        // 4. Node SPA Params (collapsed by default, placed below Routing sections and above Node Ports)
        main_col = main_col.push(rule::horizontal(1));
        let mut params_col = Column::new().width(Length::Fill).spacing(6);
        params_col = params_col.push(
            checkbox(self.show_node_params)
                .label(format!("Node SPA Params ({})", node.params.len()))
                .on_toggle(Message::ToggleSectionNodeParams)
                .size(15)
                .text_size(14),
        );

        if self.show_node_params {
            if node.params.is_empty() {
                params_col = params_col.push(
                    text("No SPA params advertised by this node.")
                        .size(12)
                        .color(Color::from_rgb(0.60, 0.64, 0.72)),
                );
            } else {
                for param in &node.params {
                    let key_id = param.key_id;
                    let badge_color = match param.param_type.as_str() {
                        "Props" => Color::from_rgb(0.45, 0.88, 0.66),
                        "Format" => Color::from_rgb(0.48, 0.82, 0.98),
                        "Latency" | "ProcessLatency" => Color::from_rgb(0.95, 0.76, 0.42),
                        _ => Color::from_rgb(0.72, 0.76, 0.84),
                    };

                    if !param.read_only {
                        let label_widget = row![
                            text(format!("[{}]", param.param_type))
                                .size(11)
                                .color(badge_color),
                            text(&param.name)
                                .size(12)
                                .width(Length::Fill)
                                .color(Color::from_rgb(0.86, 0.90, 0.96)),
                        ]
                        .width(Length::Fill)
                        .spacing(6)
                        .align_y(Alignment::Center);

                        match &param.value {
                            PwParamValue::Bool(b) => {
                                let b_val = *b;
                                params_col = params_col.push(
                                    row![
                                        label_widget,
                                        checkbox(b_val)
                                            .on_toggle(move |new_b| Message::SetNodeParamBool {
                                                node_serial,
                                                key_id,
                                                value: new_b,
                                            })
                                            .size(15),
                                        text(if b_val { "true" } else { "false" })
                                            .size(11)
                                            .color(Color::from_rgb(0.75, 0.82, 0.92)),
                                    ]
                                    .spacing(6)
                                    .align_y(Alignment::Center),
                                );
                            }
                            PwParamValue::Float(f) => {
                                let f_val = *f;
                                let max_range = if f_val > 2.0 { f_val * 2.0 } else { 2.0 };
                                params_col = params_col.push(
                                    column![
                                        row![
                                            label_widget,
                                            text(format!("{f_val:.3}"))
                                                .size(11)
                                                .color(Color::from_rgb(0.45, 0.92, 0.68)),
                                        ]
                                        .spacing(6),
                                        slider(0.0..=max_range, f_val, move |new_f| {
                                            Message::ChangeNodeParamFloat {
                                                node_serial,
                                                key_id,
                                                value: new_f,
                                            }
                                        })
                                        .step(0.01_f32)
                                        .width(Length::Fill),
                                    ]
                                    .spacing(3),
                                );
                            }
                            PwParamValue::Double(d) => {
                                let d_val = *d;
                                let f_val = d_val as f32;
                                let max_range = if f_val > 2.0 { f_val * 2.0 } else { 2.0 };
                                params_col = params_col.push(
                                    column![
                                        row![
                                            label_widget,
                                            text(format!("{d_val:.4}"))
                                                .size(11)
                                                .color(Color::from_rgb(0.45, 0.92, 0.68)),
                                        ]
                                        .spacing(6),
                                        slider(0.0..=max_range, f_val, move |new_f| {
                                            Message::ChangeNodeParamDouble {
                                                node_serial,
                                                key_id,
                                                value: f64::from(new_f),
                                            }
                                        })
                                        .step(0.01_f32)
                                        .width(Length::Fill),
                                    ]
                                    .spacing(3),
                                );
                            }
                            PwParamValue::Int(i) => {
                                let cur = *i;
                                let step = if cur.abs() >= 128 { 64 } else { 1 };
                                params_col = params_col.push(
                                    row![
                                        label_widget,
                                        button(text("-").size(11))
                                            .padding([2, 7])
                                            .on_press(Message::StepNodeParamInt {
                                                node_serial,
                                                key_id,
                                                current: cur,
                                                delta: -step,
                                            }),
                                        text(format!("{cur}"))
                                            .size(11)
                                            .color(Color::from_rgb(0.45, 0.92, 0.68)),
                                        button(text("+").size(11))
                                            .padding([2, 7])
                                            .on_press(Message::StepNodeParamInt {
                                                node_serial,
                                                key_id,
                                                current: cur,
                                                delta: step,
                                            }),
                                    ]
                                    .spacing(6)
                                    .align_y(Alignment::Center),
                                );
                            }
                            PwParamValue::Long(l) => {
                                let cur = *l;
                                let step = if cur.abs() >= 1_000_000 { 100_000 } else { 1 };
                                params_col = params_col.push(
                                    row![
                                        label_widget,
                                        button(text("-").size(11))
                                            .padding([2, 7])
                                            .on_press(Message::StepNodeParamLong {
                                                node_serial,
                                                key_id,
                                                current: cur,
                                                delta: -step,
                                            }),
                                        text(format!("{cur}"))
                                            .size(11)
                                            .color(Color::from_rgb(0.45, 0.92, 0.68)),
                                        button(text("+").size(11))
                                            .padding([2, 7])
                                            .on_press(Message::StepNodeParamLong {
                                                node_serial,
                                                key_id,
                                                current: cur,
                                                delta: step,
                                            }),
                                    ]
                                    .spacing(6)
                                    .align_y(Alignment::Center),
                                );
                            }
                            _ => {
                                params_col = params_col.push(
                                    row![
                                        container(label_widget).width(Length::FillPortion(2)),
                                        Self::view_value_or_empty_badge(
                                            param.value.display_value(),
                                            Color::from_rgb(0.76, 0.80, 0.88),
                                        ),
                                    ]
                                    .spacing(8)
                                    .align_y(Alignment::Center),
                                );
                            }
                        }
                    } else {
                        let ro_label = row![
                            text(format!("[{}]", param.param_type))
                                .size(11)
                                .color(badge_color),
                            text(&param.name)
                                .size(12)
                                .width(Length::Fill)
                                .color(Color::from_rgb(0.86, 0.90, 0.96)),
                        ]
                        .width(Length::FillPortion(2))
                        .spacing(6)
                        .align_y(Alignment::Center);

                        params_col = params_col.push(
                            row![
                                ro_label,
                                Self::view_value_or_empty_badge(
                                    param.value.display_value(),
                                    Color::from_rgb(0.72, 0.76, 0.84),
                                ),
                            ]
                            .spacing(8)
                            .align_y(Alignment::Center),
                        );
                    }
                }
            }
        }
        main_col = main_col.push(params_col);

        // 5. Port inventory + collapsible Port Properties
        let mut ports_col = Column::new().width(Length::Fill).spacing(5);
        ports_col = ports_col.push(
            row![
                text(format!(
                    "Node Ports ({} In, {} Out)",
                    node.input_ports.len(),
                    node.output_ports.len()
                ))
                .size(14)
                .width(Length::Fill)
                .color(Color::from_rgb(0.82, 0.85, 0.92)),
                checkbox(self.show_port_props)
                    .label("Port Props")
                    .on_toggle(Message::ToggleSectionPortProps)
                    .size(14)
                    .text_size(12),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
        for p in &node.input_ports {
            ports_col = ports_col.push(
                text(format!(
                    "  [IN]  s#{} (ID #{})  {} ({})",
                    p.object_serial,
                    p.id,
                    p.name,
                    p.display_label()
                ))
                .size(12)
                .width(Length::Fill)
                .color(Color::from_rgb(0.50, 0.82, 0.96)),
            );
            if self.show_port_props {
                for (k, v) in &p.props {
                    ports_col = ports_col.push(
                        row![
                            text(format!("      {k}:"))
                                .size(11)
                                .width(Length::FillPortion(2))
                                .color(Color::from_rgb(0.58, 0.64, 0.74)),
                            Self::view_value_or_empty_badge(
                                v.clone(),
                                Color::from_rgb(0.82, 0.86, 0.94),
                            ),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    );
                }
            }
        }
        for p in &node.output_ports {
            ports_col = ports_col.push(
                text(format!(
                    "  [OUT] s#{} (ID #{})  {} ({})",
                    p.object_serial,
                    p.id,
                    p.name,
                    p.display_label()
                ))
                .size(12)
                .width(Length::Fill)
                .color(Color::from_rgb(0.50, 0.90, 0.65)),
            );
            if self.show_port_props {
                for (k, v) in &p.props {
                    ports_col = ports_col.push(
                        row![
                            text(format!("      {k}:"))
                                .size(11)
                                .width(Length::FillPortion(2))
                                .color(Color::from_rgb(0.58, 0.64, 0.74)),
                            Self::view_value_or_empty_badge(
                                v.clone(),
                                Color::from_rgb(0.82, 0.86, 0.94),
                            ),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    );
                }
            }
        }
        main_col = main_col.push(rule::horizontal(1));
        main_col = main_col.push(ports_col);

        // 6. Node Properties dictionary
        let mut props_col = Column::new().width(Length::Fill).spacing(4);
        props_col = props_col.push(
            checkbox(self.show_node_props)
                .label(format!("Node Properties ({})", node.props.len()))
                .on_toggle(Message::ToggleSectionNodeProps)
                .size(15)
                .text_size(14),
        );
        if self.show_node_props {
            for (k, v) in &node.props {
                props_col = props_col.push(
                    row![
                        text(format!("{k}:"))
                            .size(11)
                            .width(Length::FillPortion(2))
                            .color(Color::from_rgb(0.62, 0.68, 0.78)),
                        Self::view_value_or_empty_badge(
                            v.clone(),
                            Color::from_rgb(0.88, 0.91, 0.96),
                        ),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                );
            }
        }
        main_col = main_col.push(rule::horizontal(1));
        main_col = main_col.push(props_col);

        main_col.into()
    }

    /// Renders the Overview Inspector when no wire or node is currently selected.
    fn view_overview_inspector(&self) -> Element<'_, Message> {
        let edges = self.graph.collapsed_edges();

        let mut list_col = Column::new().width(Length::Fill).spacing(8);
        list_col = list_col.push(
            text("Active Connections")
                .size(16)
                .color(Color::from_rgb(0.45, 0.88, 1.0)),
        );
        list_col = list_col.push(
            text("Click any node on the canvas to adjust its Volume, Mute, SPA Params, and view Node/Port Properties, or click a wire to edit Port Routing.")
                .size(12)
                .width(Length::Fill)
                .color(Color::from_rgb(0.68, 0.72, 0.80)),
        );

        if edges.is_empty() {
            list_col = list_col.push(
                text("No active connections. Drag from any node's green Output socket (right) to an Input socket (left) to connect.")
                    .size(13)
                    .width(Length::Fill)
                    .color(Color::from_rgb(0.75, 0.75, 0.80)),
            );
        } else {
            for edge in edges {
                let out_title = self
                    .graph
                    .nodes
                    .get(&edge.key.output_node_serial)
                    .map(|n| n.title())
                    .unwrap_or_default();
                let in_title = self
                    .graph
                    .nodes
                    .get(&edge.key.input_node_serial)
                    .map(|n| n.title())
                    .unwrap_or_default();

                let card = container(
                    column![
                        text(format!("{out_title}  ──►  {in_title}"))
                            .size(13)
                            .width(Length::Fill),
                        row![
                            text(format!(
                                "{} ({} port links)",
                                edge.summary,
                                edge.links.len()
                            ))
                            .size(12)
                            .width(Length::Fill)
                            .color(Color::from_rgb(0.48, 0.90, 0.66)),
                            button(text("Inspect / Edit Ports").size(11))
                                .padding([4, 8])
                                .on_press(Message::InspectNodePair {
                                    output_node_serial: edge.key.output_node_serial,
                                    input_node_serial: edge.key.input_node_serial,
                                }),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    ]
                    .width(Length::Fill)
                    .spacing(6),
                )
                .width(Length::Fill)
                .padding(8);

                list_col = list_col.push(card);
                list_col = list_col.push(rule::horizontal(1));
            }
        }

        list_col.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NodeCategory;

    fn make_test_node(serial: u64, cat: NodeCategory) -> PwNode {
        PwNode {
            id: serial as u32,
            object_serial: serial,
            name: format!("Node {serial}"),
            description: format!("Description {serial}"),
            app_name: None,
            media_name: None,
            media_class: String::from("Audio/Sink"),
            category: cat,
            state: String::from("running"),
            volume: None,
            muted: None,
            channel_volumes: Vec::new(),
            channel_names: Vec::new(),
            input_ports: Vec::new(),
            output_ports: Vec::new(),
            params: Vec::new(),
            props: BTreeMap::new(),
        }
    }

    #[test]
    fn test_resolve_node_overlaps_pins_grabbed_node() {
        let (mut app, _) = PipewireApp::new();
        app.graph.nodes.insert(1, make_test_node(1, NodeCategory::AudioSource));
        app.graph.nodes.insert(2, make_test_node(2, NodeCategory::AudioSource));

        // Node 1 was already at (40.0, 40.0). User grabbed and dropped Node 2 at (40.0, 60.0).
        app.node_positions.insert(1, Point::new(40.0, 40.0));
        app.node_positions.insert(2, Point::new(40.0, 60.0));

        app.resolve_node_overlaps(Some(2));

        let p1 = app.node_positions[&1];
        let p2 = app.node_positions[&2];

        // Grabbed node (2) MUST remain exactly where the user dropped it!
        assert_eq!(p2, Point::new(40.0, 60.0), "Grabbed node must never be moved!");
        // Node 1 must have been nudged out of the way
        assert!(
            (p2.x - p1.x).abs() >= NODE_WIDTH + 10.0 || (p2.y - p1.y).abs() >= NODE_HEIGHT + 10.0,
            "Node 1 should be nudged away to satisfy clearance: p1={p1:?}, p2={p2:?}"
        );
    }

    #[test]
    fn test_resolve_node_overlaps_horizontal_decision() {
        let (mut app, _) = PipewireApp::new();
        app.graph.nodes.insert(1, make_test_node(1, NodeCategory::AudioSource));
        app.graph.nodes.insert(2, make_test_node(2, NodeCategory::AudioSource));

        // Node 1 at (40.0, 100.0). Node 2 dropped at (120.0, 110.0):
        // dx = 80.0, dy = 10.0.
        // ux = 80/247 = 0.324 > uy = 10/108 = 0.092.
        // Dimension-aware logic must choose horizontal row nudging (X) rather than column (Y)!
        app.node_positions.insert(1, Point::new(40.0, 100.0));
        app.node_positions.insert(2, Point::new(120.0, 110.0));

        app.resolve_node_overlaps(Some(2));

        let p1 = app.node_positions[&1];
        let p2 = app.node_positions[&2];

        assert_eq!(p2, Point::new(120.0, 110.0), "Grabbed node 2 must stay pinned");
        // Separated along X (p1 pushed to the left of p2)
        assert!(
            (p2.x - p1.x).abs() >= NODE_WIDTH + 10.0,
            "Must be separated horizontally along X: p1={p1:?}, p2={p2:?}"
        );
    }

    #[test]
    fn test_resolve_node_overlaps_cascades_multiple_nodes() {
        let (mut app, _) = PipewireApp::new();
        app.graph.nodes.insert(1, make_test_node(1, NodeCategory::AudioSource));
        app.graph.nodes.insert(2, make_test_node(2, NodeCategory::AudioSource));
        app.graph.nodes.insert(3, make_test_node(3, NodeCategory::AudioSource));

        // 3 nodes placed on top of each other
        app.node_positions.insert(1, Point::new(40.0, 40.0));
        app.node_positions.insert(2, Point::new(40.0, 50.0));
        app.node_positions.insert(3, Point::new(40.0, 60.0));

        app.resolve_node_overlaps(None);

        let p1 = app.node_positions[&1];
        let p2 = app.node_positions[&2];
        let p3 = app.node_positions[&3];

        let pairs = [(p1, p2), (p1, p3), (p2, p3)];
        for (a, b) in pairs {
            let cx = (b.x - a.x).abs();
            let cy = (b.y - a.y).abs();
            assert!(
                cx >= NODE_WIDTH + 10.0 || cy >= NODE_HEIGHT + 10.0,
                "Nodes {a:?} and {b:?} must not overlap"
            );
        }
    }
}

