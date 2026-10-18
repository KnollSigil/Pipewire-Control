mod backend;
mod canvas_view;
mod model;

use backend::{Backend, PwEvent, RoutingPreset};
use canvas_view::{CanvasMessage, GraphCanvas, NODE_HEIGHT};
use iced::widget::{
    button, canvas::Canvas, checkbox, column, container, horizontal_rule, horizontal_space, row,
    scrollable, text, Column, Row,
};
use iced::{Alignment, Color, Element, Length, Point, Subscription, Task, Theme, Vector};
use model::{EdgeKey, NodeCategory, PwGraph, PwPort};
use std::collections::{BTreeMap, HashSet};

pub fn main() -> iced::Result {
    iced::application(
        "PipeWire Node Graph Control",
        PipewireApp::update,
        PipewireApp::view,
    )
    .subscription(PipewireApp::subscription)
    .theme(PipewireApp::theme)
    .window_size((1360.0, 840.0))
    .run_with(PipewireApp::new)
}

#[derive(Debug, Clone)]
pub enum Message {
    Pw(PwEvent),
    Canvas(CanvasMessage),
    RefreshGraph,
    AutoLayout,
    ResetView,
    ToggleFilterAudio(bool),
    ToggleFilterMidi(bool),
    ToggleFilterVideo(bool),
    ToggleFilterInternal(bool),
    ToggleShowMonitorPorts(bool),
    TogglePortConnection {
        output_node_id: u32,
        output_port_id: u32,
        input_node_id: u32,
        input_port_id: u32,
    },
    ApplyPreset {
        output_node_id: u32,
        input_node_id: u32,
        preset: RoutingPreset,
    },
    InspectNodePair {
        output_node_id: u32,
        input_node_id: u32,
    },
    DisconnectEdge {
        output_node_id: u32,
        input_node_id: u32,
    },
    SelectNode(u32),
}

pub struct PipewireApp {
    backend: Backend,
    graph: PwGraph,
    node_positions: BTreeMap<u32, Point>,
    selected_edge: Option<EdgeKey>,
    selected_node: Option<u32>,
    show_audio: bool,
    show_midi: bool,
    show_video: bool,
    show_internal: bool,
    show_monitor_ports: bool,
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
            pan: Vector::new(40.0, 40.0),
            zoom: 1.0,
            error_banner: None,
        };

        (app, Task::none())
    }

    pub fn theme(&self) -> Theme {
        Theme::Dark
    }

    /// Event-driven PipeWire subscription via `pipewire-rs` MainLoop (0% idle CPU usage).
    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::run(backend::pipewire_subscription).map(Message::Pw)
    }

    fn visible_node_ids(&self) -> HashSet<u32> {
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
            .map(|n| n.id)
            .collect()
    }

    fn ensure_positions_for_new_nodes(&mut self) {
        let missing: Vec<u32> = self
            .graph
            .nodes
            .keys()
            .copied()
            .filter(|id| !self.node_positions.contains_key(id))
            .collect();
        if !missing.is_empty() {
            self.compute_auto_layout();
        }
    }

    fn compute_auto_layout(&mut self) {
        let visible = self.visible_node_ids();
        let mut columns: [Vec<u32>; 3] = [Vec::new(), Vec::new(), Vec::new()];

        for (&id, node) in &self.graph.nodes {
            if !visible.contains(&id) {
                continue;
            }
            let col = node.category.column_index().min(2);
            columns[col].push(id);
        }

        let col_x = [40.0, 380.0, 720.0];
        let row_spacing = NODE_HEIGHT + 34.0;

        for (col_idx, node_ids) in columns.iter().enumerate() {
            for (row_idx, &node_id) in node_ids.iter().enumerate() {
                let x = col_x[col_idx];
                let y = 40.0 + row_idx as f32 * row_spacing;
                self.node_positions.insert(node_id, Point::new(x, y));
            }
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        self.error_banner = None;

        match message {
            Message::Pw(pw_event) => match pw_event {
                PwEvent::Connected(sender) => {
                    self.backend.is_demo = false;
                    self.backend.cmd_sender = Some(sender.0);
                    self.backend.last_status =
                        String::from("Connected to PipeWire via pipewire-rs");
                }
                PwEvent::GraphUpdated(new_graph) => {
                    let was_empty = self.graph.nodes.is_empty();
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
                    if was_empty && self.selected_edge.is_none() {
                        if let Some(first_edge) = self.graph.collapsed_edges().first() {
                            self.selected_edge = Some(first_edge.key.clone());
                        }
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
            Message::SelectNode(node_id) => {
                self.selected_node = Some(node_id);
                self.selected_edge = None;
            }
            Message::InspectNodePair {
                output_node_id,
                input_node_id,
            } => {
                self.selected_edge = Some(EdgeKey {
                    output_node_id,
                    input_node_id,
                });
                self.selected_node = None;
            }
            Message::DisconnectEdge {
                output_node_id,
                input_node_id,
            } => {
                if let Err(e) =
                    self.backend
                        .disconnect_nodes(&mut self.graph, output_node_id, input_node_id)
                {
                    self.error_banner = Some(e);
                }
            }
            Message::TogglePortConnection {
                output_node_id,
                output_port_id,
                input_node_id,
                input_port_id,
            } => {
                if let Err(e) = self.backend.toggle_port_link(
                    &mut self.graph,
                    output_node_id,
                    output_port_id,
                    input_node_id,
                    input_port_id,
                ) {
                    self.error_banner = Some(e);
                }
            }
            Message::ApplyPreset {
                output_node_id,
                input_node_id,
                preset,
            } => {
                if let Err(e) = self.backend.apply_preset(
                    &mut self.graph,
                    output_node_id,
                    input_node_id,
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
                CanvasMessage::SelectNode(node_id) => {
                    self.selected_node = Some(node_id);
                    self.selected_edge = None;
                }
                CanvasMessage::ClearSelection => {
                    self.selected_edge = None;
                    self.selected_node = None;
                }
                CanvasMessage::MoveNode(node_id, new_pos) => {
                    self.node_positions.insert(node_id, new_pos);
                }
                CanvasMessage::ConnectNodes {
                    output_node_id,
                    input_node_id,
                } => {
                    match self.backend.connect_nodes_default(
                        &mut self.graph,
                        output_node_id,
                        input_node_id,
                    ) {
                        Ok(()) => {
                            self.selected_edge = Some(EdgeKey {
                                output_node_id,
                                input_node_id,
                            });
                            self.selected_node = None;
                        }
                        Err(e) => {
                            self.error_banner = Some(e);
                        }
                    }
                }
                CanvasMessage::DisconnectNodes {
                    output_node_id,
                    input_node_id,
                } => {
                    if let Err(e) = self.backend.disconnect_nodes(
                        &mut self.graph,
                        output_node_id,
                        input_node_id,
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
        let visible_nodes = self.visible_node_ids();

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
                .width(Length::Fixed(420.0))
                .height(Length::Fill)
                .padding(14)
        ]
        .height(Length::Fill);

        let status_bar = self.view_status_bar();

        column![top_bar, horizontal_rule(1), main_row, horizontal_rule(1), status_bar]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn view_top_bar(&self) -> Element<'_, Message> {
        let mode_badge = if self.backend.is_demo {
            text("● Demo Graph").color(Color::from_rgb(0.96, 0.74, 0.30))
        } else {
            text("● PipeWire Live").color(Color::from_rgb(0.38, 0.88, 0.56))
        };

        let filters = row![
            text("Show:").size(13).color(Color::from_rgb(0.7, 0.74, 0.82)),
            checkbox("Audio", self.show_audio)
                .on_toggle(Message::ToggleFilterAudio)
                .size(16)
                .text_size(13),
            checkbox("MIDI", self.show_midi)
                .on_toggle(Message::ToggleFilterMidi)
                .size(16)
                .text_size(13),
            checkbox("Video", self.show_video)
                .on_toggle(Message::ToggleFilterVideo)
                .size(16)
                .text_size(13),
            checkbox("Internal", self.show_internal)
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
                horizontal_space(),
                filters,
                horizontal_space(),
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
                horizontal_space(),
                text(format!(
                    "Collapsed Wires: {edge_count}   |   Underlying Port Links: {raw_link_count}   |   Zoom: {:.0}%",
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

    fn view_inspector_panel(&self, visible_nodes: &HashSet<u32>) -> Element<'_, Message> {
        let content: Element<'_, Message> = if let Some(edge_key) = &self.selected_edge {
            self.view_edge_routing_matrix(edge_key)
        } else if let Some(node_id) = self.selected_node {
            self.view_node_inspector(node_id, visible_nodes)
        } else {
            self.view_overview_inspector()
        };

        scrollable(content).height(Length::Fill).into()
    }

    /// Renders the Port Routing Matrix & Inspector for a selected collapsed wire (or node pair).
    fn view_edge_routing_matrix(&self, edge_key: &EdgeKey) -> Element<'_, Message> {
        let (Some(out_node), Some(in_node)) = (
            self.graph.nodes.get(&edge_key.output_node_id),
            self.graph.nodes.get(&edge_key.input_node_id),
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
                l.output_node_id == edge_key.output_node_id
                    && l.input_node_id == edge_key.input_node_id
            })
            .cloned()
            .collect();

        let topology_summary = self.graph.describe_links(&active_links);

        // Header info
        let header = column![
            text("Collapsed Wire Port Inspector")
                .size(17)
                .color(Color::from_rgb(0.45, 0.88, 1.0)),
            text(format!("Source: {} (#{})", out_node.title(), out_node.id)).size(14),
            text(format!("Target:  {} (#{})", in_node.title(), in_node.id)).size(14),
            text(format!(
                "Current Topology: {} ({} port link{})",
                topology_summary,
                active_links.len(),
                if active_links.len() == 1 { "" } else { "s" }
            ))
            .size(13)
            .color(Color::from_rgb(0.48, 0.90, 0.66)),
        ]
        .spacing(5);

        // Quick Topology Presets
        let out_id = out_node.id;
        let in_id = in_node.id;
        let presets = column![
            text("Quick Routing Presets")
                .size(14)
                .color(Color::from_rgb(0.82, 0.85, 0.92)),
            row![
                button(text("Auto Default").size(12))
                    .on_press(Message::ApplyPreset {
                        output_node_id: out_id,
                        input_node_id: in_id,
                        preset: RoutingPreset::DefaultAuto,
                    })
                    .padding([5, 10]),
                button(text("Stereo 1:1").size(12))
                    .on_press(Message::ApplyPreset {
                        output_node_id: out_id,
                        input_node_id: in_id,
                        preset: RoutingPreset::Stereo1To1,
                    })
                    .padding([5, 10]),
                button(text("Swap L ↔ R").size(12))
                    .on_press(Message::ApplyPreset {
                        output_node_id: out_id,
                        input_node_id: in_id,
                        preset: RoutingPreset::SwapLeftRight,
                    })
                    .padding([5, 10]),
            ]
            .spacing(6),
            row![
                button(text("Mono (L) → Stereo").size(12))
                    .on_press(Message::ApplyPreset {
                        output_node_id: out_id,
                        input_node_id: in_id,
                        preset: RoutingPreset::MonoLeftToBoth,
                    })
                    .padding([5, 10]),
                button(text("Mono (R) → Stereo").size(12))
                    .on_press(Message::ApplyPreset {
                        output_node_id: out_id,
                        input_node_id: in_id,
                        preset: RoutingPreset::MonoRightToBoth,
                    })
                    .padding([5, 10]),
                button(text("Disconnect All").size(12))
                    .on_press(Message::ApplyPreset {
                        output_node_id: out_id,
                        input_node_id: in_id,
                        preset: RoutingPreset::DisconnectAll,
                    })
                    .padding([5, 10]),
            ]
            .spacing(6),
        ]
        .spacing(8);

        // Filter output ports (whether to show monitor ports)
        let has_monitor_ports = out_node.output_ports.iter().any(|p| p.is_monitor);
        let out_ports: Vec<&PwPort> = if self.show_monitor_ports || !out_node.output_ports.iter().any(|p| !p.is_monitor) {
            out_node.output_ports.iter().collect()
        } else {
            out_node.output_ports.iter().filter(|p| !p.is_monitor).collect()
        };
        let in_ports: Vec<&PwPort> = in_node.input_ports.iter().collect();

        // Build the interactive Port Routing Matrix (Output Ports × Input Ports)
        let mut matrix_col = Column::new().spacing(8);
        matrix_col = matrix_col.push(
            text("Port-to-Port Routing Matrix (Click to Connect / Disconnect)")
                .size(14)
                .color(Color::from_rgb(0.82, 0.85, 0.92)),
        );

        if has_monitor_ports {
            matrix_col = matrix_col.push(
                checkbox("Include Monitor Output Ports", self.show_monitor_ports)
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
            // Column headers (Input Ports of Target Node)
            let mut header_row = Row::new().spacing(6).align_y(Alignment::Center);
            header_row = header_row.push(
                container(
                    text("Out \\ In")
                        .size(12)
                        .color(Color::from_rgb(0.60, 0.65, 0.75)),
                )
                .width(Length::Fixed(115.0)),
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
                    .width(Length::Fixed(92.0)),
                );
            }
            matrix_col = matrix_col.push(header_row);

            // One row per Output Port of Source Node
            for out_port in &out_ports {
                let mut port_row = Row::new().spacing(6).align_y(Alignment::Center);

                port_row = port_row.push(
                    container(
                        column![
                            text(out_port.display_label())
                                .size(12)
                                .color(Color::from_rgb(0.45, 0.92, 0.62)),
                            text(&out_port.name)
                                .size(10)
                                .color(Color::from_rgb(0.58, 0.62, 0.70)),
                        ],
                    )
                    .width(Length::Fixed(115.0)),
                );

                for in_port in &in_ports {
                    let is_connected = active_links.iter().any(|l| {
                        l.output_port_id == out_port.id && l.input_port_id == in_port.id
                    });

                    let cell_label = if is_connected {
                        format!("● {}→{}", out_port.display_label(), in_port.display_label())
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
                    .width(Length::Fixed(92.0))
                    .padding([8, 6])
                    .on_press(Message::TogglePortConnection {
                        output_node_id: out_node.id,
                        output_port_id: out_port.id,
                        input_node_id: in_node.id,
                        input_port_id: in_port.id,
                    });

                    port_row = port_row.push(btn);
                }

                matrix_col = matrix_col.push(port_row);
            }
        }

        // Individual active port links list
        let mut links_col = Column::new().spacing(6);
        links_col = links_col.push(
            text("Active Port-Level Links in this Wire")
                .size(14)
                .color(Color::from_rgb(0.82, 0.85, 0.92)),
        );

        if active_links.is_empty() {
            links_col = links_col.push(
                text("No active port links between these two nodes. Click any cell in the matrix above or choose a preset to connect.")
                    .size(12)
                    .color(Color::from_rgb(0.65, 0.68, 0.75)),
            );
        } else {
            for link in &active_links {
                let out_name = self
                    .graph
                    .ports
                    .get(&link.output_port_id)
                    .map(|p| format!("{} ({})", p.name, p.display_label()))
                    .unwrap_or_else(|| format!("Port #{}", link.output_port_id));
                let in_name = self
                    .graph
                    .ports
                    .get(&link.input_port_id)
                    .map(|p| format!("{} ({})", p.name, p.display_label()))
                    .unwrap_or_else(|| format!("Port #{}", link.input_port_id));

                links_col = links_col.push(
                    row![
                        text(format!("{out_name}  ──►  {in_name}  [{}]", link.state))
                            .size(12)
                            .color(Color::from_rgb(0.85, 0.90, 0.95)),
                        horizontal_space(),
                        button(text("Unlink").size(11))
                            .padding([3, 8])
                            .on_press(Message::TogglePortConnection {
                                output_node_id: link.output_node_id,
                                output_port_id: link.output_port_id,
                                input_node_id: link.input_node_id,
                                input_port_id: link.input_port_id,
                            }),
                    ]
                    .align_y(Alignment::Center),
                );
            }
        }

        column![
            header,
            horizontal_rule(1),
            presets,
            horizontal_rule(1),
            matrix_col,
            horizontal_rule(1),
            links_col,
        ]
        .spacing(14)
        .into()
    }

    /// Renders the Node Inspector when a single node is clicked.
    fn view_node_inspector(
        &self,
        node_id: u32,
        visible_nodes: &HashSet<u32>,
    ) -> Element<'_, Message> {
        let Some(node) = self.graph.nodes.get(&node_id) else {
            return text("Node not found.").size(14).into();
        };

        let header = column![
            text(format!("Node Inspector: {}", node.title()))
                .size(17)
                .color(Color::from_rgb(0.45, 0.88, 1.0)),
            text(format!(
                "ID: #{}   |   Class: {}   |   State: {}",
                node.id, node.media_class, node.state
            ))
            .size(12)
            .color(Color::from_rgb(0.68, 0.72, 0.80)),
            text(format!("Internal Name: {}", node.name))
                .size(12)
                .color(Color::from_rgb(0.58, 0.62, 0.70)),
        ]
        .spacing(4);

        // Connected collapsed wires involving this node
        let edges: Vec<_> = self
            .graph
            .collapsed_edges()
            .into_iter()
            .filter(|e| e.key.output_node_id == node_id || e.key.input_node_id == node_id)
            .collect();

        let mut wires_col = Column::new().spacing(6);
        wires_col = wires_col.push(
            text("Connected Collapsed Wires")
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
                let other_label = if edge.key.output_node_id == node_id {
                    let target = self
                        .graph
                        .nodes
                        .get(&edge.key.input_node_id)
                        .map(|n| n.title())
                        .unwrap_or_default();
                    format!("──► {target}  [{}]", edge.summary)
                } else {
                    let source = self
                        .graph
                        .nodes
                        .get(&edge.key.output_node_id)
                        .map(|n| n.title())
                        .unwrap_or_default();
                    format!("◄── {source}  [{}]", edge.summary)
                };

                wires_col = wires_col.push(
                    row![
                        text(other_label).size(12),
                        horizontal_space(),
                        button(text("Edit Ports").size(11))
                            .padding([4, 8])
                            .on_press(Message::InspectNodePair {
                                output_node_id: edge.key.output_node_id,
                                input_node_id: edge.key.input_node_id,
                            }),
                        button(text("×").size(11))
                            .padding([4, 8])
                            .on_press(Message::DisconnectEdge {
                                output_node_id: edge.key.output_node_id,
                                input_node_id: edge.key.input_node_id,
                            }),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                );
            }
        }

        // Quick "Connect / Edit Port Matrix with another node" list
        let mut quick_connect_col = Column::new().spacing(6);
        if !node.output_ports.is_empty() {
            quick_connect_col = quick_connect_col.push(
                text("Route Output From This Node To:")
                    .size(14)
                    .color(Color::from_rgb(0.82, 0.85, 0.92)),
            );
            for (&other_id, other_node) in &self.graph.nodes {
                if other_id == node_id
                    || !visible_nodes.contains(&other_id)
                    || other_node.input_ports.is_empty()
                {
                    continue;
                }
                quick_connect_col = quick_connect_col.push(
                    row![
                        text(other_node.title()).size(12),
                        horizontal_space(),
                        button(text("Connect").size(11))
                            .padding([3, 8])
                            .on_press(Message::Canvas(CanvasMessage::ConnectNodes {
                                output_node_id: node_id,
                                input_node_id: other_id,
                            })),
                        button(text("Port Matrix").size(11))
                            .padding([3, 8])
                            .on_press(Message::InspectNodePair {
                                output_node_id: node_id,
                                input_node_id: other_id,
                            }),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                );
            }
        }

        // Port inventory
        let mut ports_col = Column::new().spacing(4);
        ports_col = ports_col.push(
            text(format!(
                "Node Ports ({} In, {} Out)",
                node.input_ports.len(),
                node.output_ports.len()
            ))
            .size(14)
            .color(Color::from_rgb(0.82, 0.85, 0.92)),
        );
        for p in &node.input_ports {
            ports_col = ports_col.push(
                text(format!("  [IN]  #{}  {} ({})", p.id, p.name, p.display_label()))
                    .size(12)
                    .color(Color::from_rgb(0.50, 0.82, 0.96)),
            );
        }
        for p in &node.output_ports {
            ports_col = ports_col.push(
                text(format!("  [OUT] #{}  {} ({})", p.id, p.name, p.display_label()))
                    .size(12)
                    .color(Color::from_rgb(0.50, 0.90, 0.65)),
            );
        }

        column![
            header,
            horizontal_rule(1),
            wires_col,
            horizontal_rule(1),
            quick_connect_col,
            horizontal_rule(1),
            ports_col,
        ]
        .spacing(14)
        .into()
    }

    /// Renders the Overview Inspector when no wire or node is currently selected.
    fn view_overview_inspector(&self) -> Element<'_, Message> {
        let edges = self.graph.collapsed_edges();

        let mut list_col = Column::new().spacing(8);
        list_col = list_col.push(
            text("Active Collapsed Wires")
                .size(16)
                .color(Color::from_rgb(0.45, 0.88, 1.0)),
        );
        list_col = list_col.push(
            text("Click any collapsed wire on the canvas (or below) to view and edit which individual output ports connect to which input ports.")
                .size(12)
                .color(Color::from_rgb(0.68, 0.72, 0.80)),
        );

        if edges.is_empty() {
            list_col = list_col.push(
                text("No active connections. Drag from any node's green Output socket (right) to an Input socket (left) to connect.")
                    .size(13)
                    .color(Color::from_rgb(0.75, 0.75, 0.80)),
            );
        } else {
            for edge in edges {
                let out_title = self
                    .graph
                    .nodes
                    .get(&edge.key.output_node_id)
                    .map(|n| n.title())
                    .unwrap_or_default();
                let in_title = self
                    .graph
                    .nodes
                    .get(&edge.key.input_node_id)
                    .map(|n| n.title())
                    .unwrap_or_default();

                let card = container(
                    column![
                        row![
                            text(format!("{out_title}  ──►  {in_title}")).size(13),
                            horizontal_space(),
                        ],
                        row![
                            text(format!(
                                "{} ({} port links)",
                                edge.summary,
                                edge.links.len()
                            ))
                            .size(12)
                            .color(Color::from_rgb(0.48, 0.90, 0.66)),
                            horizontal_space(),
                            button(text("Inspect / Edit Ports").size(11))
                                .padding([4, 8])
                                .on_press(Message::InspectNodePair {
                                    output_node_id: edge.key.output_node_id,
                                    input_node_id: edge.key.input_node_id,
                                }),
                        ]
                        .align_y(Alignment::Center),
                    ]
                    .spacing(6),
                )
                .padding(8);

                list_col = list_col.push(card);
                list_col = list_col.push(horizontal_rule(1));
            }
        }

        list_col.into()
    }
}
