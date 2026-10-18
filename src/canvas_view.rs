use crate::model::{CollapsedEdge, EdgeKey, NodeCategory, PwGraph, PwNode, PwPort};
use iced::mouse;
use iced::widget::canvas::{
    self, Event, Frame, Geometry, LineCap, Path, Program, Stroke, Style, Text,
};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme, Vector};
use iced_fonts::LUCIDE_FONT;
use std::collections::{BTreeMap, HashSet};

pub const ICON_VOLUME_X: &str = "\u{e1ac}";
pub const ICON_VOLUME_2: &str = "\u{e1ab}";

pub const NODE_WIDTH: f32 = 235.0;
pub const NODE_HEIGHT: f32 = 96.0;
pub const SOCKET_RADIUS: f32 = 8.0;

#[derive(Debug, Clone)]
pub enum CanvasMessage {
    SelectEdge(EdgeKey),
    SelectNode(u64),
    ClearSelection,
    MoveNode(u64, Point),
    FinishMoveNode(u64),
    ConnectNodes {
        output_node_serial: u64,
        input_node_serial: u64,
    },
    DisconnectNodes {
        output_node_serial: u64,
        input_node_serial: u64,
    },
    PanCanvas(Vector),
    ZoomCanvas(f32, Point),
}

#[derive(Debug, Clone, Default)]
pub struct CanvasInteraction {
    pub dragging_node: Option<(u64, Vector)>,
    pub panning_from: Option<Point>,
    pub wiring_from_output: Option<(u64, Point)>,
    pub hovered_edge: Option<EdgeKey>,
    pub hovered_node: Option<u64>,
    pub hovered_input_socket: Option<u64>,
    pub hovered_output_socket: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanvasLayer {
    Base,
    Overlay,
}

pub struct GraphCanvas<'a> {
    pub graph: &'a PwGraph,
    pub visible_nodes: HashSet<u64>,
    pub node_positions: &'a BTreeMap<u64, Point>,
    pub selected_edge: Option<&'a EdgeKey>,
    pub selected_node: Option<u64>,
    pub pan: Vector,
    pub zoom: f32,
    pub layer: CanvasLayer,
}

impl<'a> GraphCanvas<'a> {
    fn screen_to_world(&self, p: Point) -> Point {
        Point::new(
            (p.x - self.pan.x) / self.zoom,
            (p.y - self.pan.y) / self.zoom,
        )
    }

    fn world_to_screen(&self, p: Point) -> Point {
        Point::new(p.x * self.zoom + self.pan.x, p.y * self.zoom + self.pan.y)
    }

    pub fn input_socket_world(node_pos: Point) -> Point {
        Point::new(node_pos.x, node_pos.y + NODE_HEIGHT * 0.55)
    }

    pub fn output_socket_world(node_pos: Point) -> Point {
        Point::new(node_pos.x + NODE_WIDTH, node_pos.y + NODE_HEIGHT * 0.55)
    }

    fn hit_test_output_socket(&self, world_pt: Point) -> Option<u64> {
        let hit_r = SOCKET_RADIUS * 1.6;
        for (&serial, &pos) in self.node_positions {
            if !self.visible_nodes.contains(&serial) {
                continue;
            }
            if let Some(node) = self.graph.nodes.get(&serial) {
                if !node.output_ports.is_empty() {
                    let sock = Self::output_socket_world(pos);
                    if distance(world_pt, sock) <= hit_r {
                        return Some(serial);
                    }
                }
            }
        }
        None
    }

    fn hit_test_input_socket(&self, world_pt: Point) -> Option<u64> {
        let hit_r = SOCKET_RADIUS * 1.6;
        for (&serial, &pos) in self.node_positions {
            if !self.visible_nodes.contains(&serial) {
                continue;
            }
            if let Some(node) = self.graph.nodes.get(&serial) {
                if !node.input_ports.is_empty() {
                    let sock = Self::input_socket_world(pos);
                    if distance(world_pt, sock) <= hit_r {
                        return Some(serial);
                    }
                }
            }
        }
        None
    }

    fn hit_test_node(&self, world_pt: Point) -> Option<u64> {
        // Test selected node first (it is drawn on top)
        if let Some(selected) = self.selected_node {
            if self.visible_nodes.contains(&selected) {
                if let Some(&pos) = self.node_positions.get(&selected) {
                    let rect = Rectangle::new(pos, Size::new(NODE_WIDTH, NODE_HEIGHT));
                    if rect.contains(world_pt) {
                        return Some(selected);
                    }
                }
            }
        }

        for (&serial, &pos) in self.node_positions.iter().rev() {
            if Some(serial) == self.selected_node || !self.visible_nodes.contains(&serial) {
                continue;
            }
            let rect = Rectangle::new(pos, Size::new(NODE_WIDTH, NODE_HEIGHT));
            if rect.contains(world_pt) {
                return Some(serial);
            }
        }
        None
    }

    fn hit_test_edge(&self, world_pt: Point) -> Option<EdgeKey> {
        // If the world point is inside any visible node, do not hit-test edges.
        // Node interaction (click/hover/drag) has strict priority over wires behind them.
        if self.hit_test_node(world_pt).is_some() {
            return None;
        }

        let edges = self.graph.collapsed_edges();
        for edge in edges {
            if !self.visible_nodes.contains(&edge.key.output_node_serial)
                || !self.visible_nodes.contains(&edge.key.input_node_serial)
            {
                continue;
            }
            let (Some(&out_pos), Some(&in_pos)) = (
                self.node_positions.get(&edge.key.output_node_serial),
                self.node_positions.get(&edge.key.input_node_serial),
            ) else {
                continue;
            };
            let p0 = Self::output_socket_world(out_pos);
            let p3 = Self::input_socket_world(in_pos);

            // Midpoint check (condensed badge or hover tooltip)
            let mid = bezier_point(p0, p3, 0.5);
            let badge_rect = Rectangle::new(
                Point::new(mid.x - 24.0, mid.y - 18.0),
                Size::new(48.0, 36.0),
            );
            if badge_rect.contains(world_pt) {
                return Some(edge.key);
            }

            // Sample cubic bezier curve distance
            if distance_to_bezier(world_pt, p0, p3) <= 12.0 {
                return Some(edge.key);
            }
        }
        None
    }
}

impl<'a> Program<CanvasMessage> for GraphCanvas<'a> {
    type State = CanvasInteraction;

    fn update(
        &self,
        state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<CanvasMessage>> {
        if self.layer == CanvasLayer::Overlay {
            if let Event::Mouse(mouse::Event::CursorMoved { .. }) = event {
                let cursor_pos = cursor.position_in(bounds)?;
                let world_pt = self.screen_to_world(cursor_pos);
                state.hovered_output_socket = self.hit_test_output_socket(world_pt);
                state.hovered_input_socket = self.hit_test_input_socket(world_pt);
            }
            return None;
        }

        let cursor_pos = cursor.position_in(bounds)?;
        let world_pt = self.screen_to_world(cursor_pos);

        match event {
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let Some((node_serial, offset)) = state.dragging_node {
                    let new_pos = Point::new(world_pt.x - offset.x, world_pt.y - offset.y);
                    return Some(
                        canvas::Action::publish(CanvasMessage::MoveNode(node_serial, new_pos))
                            .and_capture(),
                    );
                }

                if let Some((out_node_serial, _)) = state.wiring_from_output {
                    state.wiring_from_output = Some((out_node_serial, world_pt));
                    state.hovered_input_socket = self.hit_test_input_socket(world_pt);
                    return Some(canvas::Action::request_redraw().and_capture());
                }

                if let Some(last_pan) = state.panning_from {
                    let delta = Vector::new(cursor_pos.x - last_pan.x, cursor_pos.y - last_pan.y);
                    state.panning_from = Some(cursor_pos);
                    return Some(
                        canvas::Action::publish(CanvasMessage::PanCanvas(delta)).and_capture(),
                    );
                }

                let prev_out = state.hovered_output_socket;
                let prev_in = state.hovered_input_socket;
                let prev_edge = state.hovered_edge.clone();
                let prev_node = state.hovered_node;

                state.hovered_output_socket = self.hit_test_output_socket(world_pt);
                state.hovered_input_socket = self.hit_test_input_socket(world_pt);

                // Priority: Sockets > Nodes > Edges
                if state.hovered_output_socket.is_none() && state.hovered_input_socket.is_none() {
                    state.hovered_node = self.hit_test_node(world_pt);
                    if state.hovered_node.is_none() {
                        state.hovered_edge = self.hit_test_edge(world_pt);
                    } else {
                        state.hovered_edge = None;
                    }
                } else {
                    state.hovered_node = None;
                    state.hovered_edge = None;
                }

                if prev_out != state.hovered_output_socket
                    || prev_in != state.hovered_input_socket
                    || prev_edge != state.hovered_edge
                    || prev_node != state.hovered_node
                {
                    Some(canvas::Action::request_redraw())
                } else {
                    None
                }
            }

            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                // 1. Check output socket drag to create a new collapsed wire
                if let Some(out_node_serial) = self.hit_test_output_socket(world_pt) {
                    state.wiring_from_output = Some((out_node_serial, world_pt));
                    return Some(canvas::Action::request_redraw().and_capture());
                }

                // 2. Check node click/drag (PRIORITY over wires so wires behind nodes do not intercept clicks)
                if let Some(node_serial) = self.hit_test_node(world_pt) {
                    if let Some(&node_pos) = self.node_positions.get(&node_serial) {
                        let offset = Vector::new(world_pt.x - node_pos.x, world_pt.y - node_pos.y);
                        state.dragging_node = Some((node_serial, offset));
                    }
                    return Some(
                        canvas::Action::publish(CanvasMessage::SelectNode(node_serial))
                            .and_capture(),
                    );
                }

                // 3. Check collapsed wire click -> open Port Routing Matrix
                if let Some(edge_key) = self.hit_test_edge(world_pt) {
                    return Some(
                        canvas::Action::publish(CanvasMessage::SelectEdge(edge_key)).and_capture(),
                    );
                }

                // 4. Otherwise start panning canvas and clear selection
                state.panning_from = Some(cursor_pos);
                Some(canvas::Action::publish(CanvasMessage::ClearSelection).and_capture())
            }

            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Middle)) => {
                state.panning_from = Some(cursor_pos);
                Some(canvas::Action::capture())
            }

            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                // Right-click on a collapsed wire disconnects it (only if not clicking on a node)
                if self.hit_test_node(world_pt).is_none() {
                    if let Some(edge_key) = self.hit_test_edge(world_pt) {
                        return Some(
                            canvas::Action::publish(CanvasMessage::DisconnectNodes {
                                output_node_serial: edge_key.output_node_serial,
                                input_node_serial: edge_key.input_node_serial,
                            })
                            .and_capture(),
                        );
                    }
                }
                None
            }

            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if let Some((output_node_serial, _)) = state.wiring_from_output.take() {
                    state.hovered_input_socket = None;
                    // Connect if released on an input socket or anywhere on a target node with input ports
                    let target_node = self
                        .hit_test_input_socket(world_pt)
                        .or_else(|| self.hit_test_node(world_pt));
                    if let Some(input_node_serial) = target_node {
                        if input_node_serial != output_node_serial {
                            return Some(
                                canvas::Action::publish(CanvasMessage::ConnectNodes {
                                    output_node_serial,
                                    input_node_serial,
                                })
                                .and_capture(),
                            );
                        }
                    }
                    return Some(canvas::Action::request_redraw().and_capture());
                }

                if let Some((node_serial, _)) = state.dragging_node.take() {
                    state.panning_from = None;
                    return Some(
                        canvas::Action::publish(CanvasMessage::FinishMoveNode(node_serial))
                            .and_capture(),
                    );
                }

                state.dragging_node = None;
                state.panning_from = None;
                None
            }

            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Middle)) => {
                state.panning_from = None;
                None
            }

            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let scroll_y = match *delta {
                    mouse::ScrollDelta::Lines { y, .. } => y,
                    mouse::ScrollDelta::Pixels { y, .. } => y / 40.0,
                };
                if scroll_y.abs() > 0.01 {
                    return Some(
                        canvas::Action::publish(CanvasMessage::ZoomCanvas(scroll_y, cursor_pos))
                            .and_capture(),
                    );
                }
                None
            }

            _ => None,
        }
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut geometries = Vec::new();

        match self.layer {
            CanvasLayer::Base => {
                let mut base_frame = Frame::new(renderer, bounds.size());

                // Background fill
                base_frame.fill_rectangle(
                    Point::ORIGIN,
                    bounds.size(),
                    Color::from_rgb(0.08, 0.09, 0.12),
                );

                // Draw subtle grid in screen space
                self.draw_grid(&mut base_frame, bounds.size());

                // 1. Draw all collapsed edges (single wire per node pair)
                let edges = self.graph.collapsed_edges();
                for edge in &edges {
                    if !self.visible_nodes.contains(&edge.key.output_node_serial)
                        || !self.visible_nodes.contains(&edge.key.input_node_serial)
                    {
                        continue;
                    }
                    let (Some(&out_pos), Some(&in_pos)) = (
                        self.node_positions.get(&edge.key.output_node_serial),
                        self.node_positions.get(&edge.key.input_node_serial),
                    ) else {
                        continue;
                    };

                    let p0 = self.world_to_screen(Self::output_socket_world(out_pos));
                    let p3 = self.world_to_screen(Self::input_socket_world(in_pos));

                    let is_selected = self.selected_edge == Some(&edge.key);
                    let is_hovered = state.hovered_edge.as_ref() == Some(&edge.key);

                    self.draw_collapsed_wire(&mut base_frame, p0, p3, edge, is_selected, is_hovered);
                }

                let active_serial = state.dragging_node.map(|(s, _)| s).or(self.selected_node);

                // Draw unselected nodes in base layer with ALL text fully rendered
                for (&node_serial, &world_pos) in self.node_positions {
                    if !self.visible_nodes.contains(&node_serial) {
                        continue;
                    }
                    if Some(node_serial) == active_serial {
                        continue;
                    }
                    if let Some(node) = self.graph.nodes.get(&node_serial) {
                        let is_hovered = state.hovered_node == Some(node_serial);
                        let out_sock_hovered = state.hovered_output_socket == Some(node_serial)
                            || state.wiring_from_output.map(|(s, _)| s) == Some(node_serial);
                        let in_sock_hovered = state.hovered_input_socket == Some(node_serial);

                        self.draw_node(
                            &mut base_frame,
                            node,
                            world_pos,
                            false,
                            is_hovered,
                            in_sock_hovered,
                            out_sock_hovered,
                        );
                    }
                }

                // Active wire drag preview
                if let Some((out_node_serial, cursor_world)) = state.wiring_from_output {
                    if let Some(&out_pos) = self.node_positions.get(&out_node_serial) {
                        let p0 = self.world_to_screen(Self::output_socket_world(out_pos));
                        let p3 = self.world_to_screen(cursor_world);
                        let path = bezier_path(p0, p3);
                        base_frame.stroke(
                            &path,
                            Stroke {
                                style: Style::Solid(Color::from_rgb(0.35, 0.85, 0.95)),
                                width: 3.0 * self.zoom,
                                line_cap: LineCap::Round,
                                ..Stroke::default()
                            },
                        );
                    }
                }

                // Hovered/selected wire detail tooltip above nodes
                for edge in &edges {
                    let is_selected = self.selected_edge == Some(&edge.key);
                    let is_hovered = state.hovered_edge.as_ref() == Some(&edge.key);
                    if !is_hovered && !is_selected {
                        continue;
                    }
                    if !self.visible_nodes.contains(&edge.key.output_node_serial)
                        || !self.visible_nodes.contains(&edge.key.input_node_serial)
                    {
                        continue;
                    }
                    let (Some(&out_pos), Some(&in_pos)) = (
                        self.node_positions.get(&edge.key.output_node_serial),
                        self.node_positions.get(&edge.key.input_node_serial),
                    ) else {
                        continue;
                    };

                    let p0 = self.world_to_screen(Self::output_socket_world(out_pos));
                    let p3 = self.world_to_screen(Self::input_socket_world(in_pos));
                    self.draw_wire_tooltip(&mut base_frame, p0, p3, edge, is_selected);
                }

                // Canvas overlay hint in bottom-left
                base_frame.fill_text(Text {
                    content: String::from(
                        "Drag Output socket -> Input socket to connect  |  Click wire to inspect ports  |  Right-click wire to disconnect  |  Scroll to zoom",
                    ),
                    position: Point::new(14.0, bounds.height - 22.0),
                    color: Color::from_rgba(0.75, 0.78, 0.85, 0.65),
                    size: 12.0.into(),
                    ..Text::default()
                });

                geometries.push(base_frame.into_geometry());
            }

            CanvasLayer::Overlay => {
                // Overlay layer: renders active/selected/dragged node card on top of the base layer.
                // In iced::widget::stack, this layer is drawn using with_layer, physically occluding
                // any text and geometry rendered in the base layer beneath this card.
                if let Some(act_serial) = self.selected_node {
                    if self.visible_nodes.contains(&act_serial) {
                        if let (Some(node), Some(&world_pos)) = (
                            self.graph.nodes.get(&act_serial),
                            self.node_positions.get(&act_serial),
                        ) {
                            let mut act_frame = Frame::new(renderer, bounds.size());
                            let out_sock_hovered = state.hovered_output_socket == Some(act_serial);
                            let in_sock_hovered = state.hovered_input_socket == Some(act_serial);

                            self.draw_node(
                                &mut act_frame,
                                node,
                                world_pos,
                                true,
                                false,
                                in_sock_hovered,
                                out_sock_hovered,
                            );
                            geometries.push(act_frame.into_geometry());
                        }
                    }
                }
            }
        }

        geometries
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if self.layer == CanvasLayer::Overlay {
            return mouse::Interaction::None;
        }

        if cursor.position_in(bounds).is_none() {
            return mouse::Interaction::default();
        }
        if state.dragging_node.is_some() || state.panning_from.is_some() {
            return mouse::Interaction::Grabbing;
        }
        if state.wiring_from_output.is_some()
            || state.hovered_output_socket.is_some()
            || state.hovered_input_socket.is_some()
        {
            return mouse::Interaction::Crosshair;
        }
        if state.hovered_edge.is_some() {
            return mouse::Interaction::Pointer;
        }
        if state.hovered_node.is_some() {
            return mouse::Interaction::Grab;
        }
        mouse::Interaction::default()
    }
}

impl<'a> GraphCanvas<'a> {
    fn draw_grid(&self, frame: &mut Frame, size: Size) {
        let step = (36.0 * self.zoom).max(14.0);
        let grid_color = Color::from_rgba(1.0, 1.0, 1.0, 0.035);
        let stroke = Stroke {
            style: Style::Solid(grid_color),
            width: 1.0,
            ..Stroke::default()
        };

        let start_x = self.pan.x.rem_euclid(step);
        let mut x = start_x;
        while x < size.width {
            let line = Path::line(Point::new(x, 0.0), Point::new(x, size.height));
            frame.stroke(&line, stroke);
            x += step;
        }

        let start_y = self.pan.y.rem_euclid(step);
        let mut y = start_y;
        while y < size.height {
            let line = Path::line(Point::new(0.0, y), Point::new(size.width, y));
            frame.stroke(&line, stroke);
            y += step;
        }
    }

    fn draw_collapsed_wire(
        &self,
        frame: &mut Frame,
        p0: Point,
        p3: Point,
        edge: &CollapsedEdge,
        is_selected: bool,
        is_hovered: bool,
    ) {
        let path = bezier_path(p0, p3);

        // Styling: Non-default connections stand out with an amber/coral hue,
        // while default connections use a clean teal/green hue.
        let base_color = if is_selected {
            Color::from_rgb(0.35, 0.88, 1.0)
        } else if is_hovered {
            Color::from_rgb(0.55, 0.95, 0.75)
        } else if !edge.is_default {
            Color::from_rgb(0.98, 0.68, 0.28)
        } else {
            Color::from_rgb(0.35, 0.75, 0.58)
        };

        // Outer glow if selected or hovered, or subtle highlight for custom non-default routing
        if is_selected || is_hovered {
            frame.stroke(
                &path,
                Stroke {
                    style: Style::Solid(Color::from_rgba(
                        base_color.r,
                        base_color.g,
                        base_color.b,
                        0.32,
                    )),
                    width: (9.0 * self.zoom).max(5.0),
                    line_cap: LineCap::Round,
                    ..Stroke::default()
                },
            );
        } else if !edge.is_default {
            frame.stroke(
                &path,
                Stroke {
                    style: Style::Solid(Color::from_rgba(0.98, 0.68, 0.28, 0.16)),
                    width: (6.5 * self.zoom).max(4.0),
                    line_cap: LineCap::Round,
                    ..Stroke::default()
                },
            );
        }

        // Main collapsed wire stroke (non-default connections are slightly thicker and distinctive)
        let stroke_width = if is_selected || is_hovered {
            (3.8 * self.zoom).max(2.4)
        } else if !edge.is_default {
            (3.4 * self.zoom).max(2.2)
        } else {
            (2.6 * self.zoom).max(1.8)
        };

        frame.stroke(
            &path,
            Stroke {
                style: Style::Solid(base_color),
                width: stroke_width,
                line_cap: LineCap::Round,
                ..Stroke::default()
            },
        );

        // Wire midpoint in screen space
        let mid = bezier_point(p0, p3, 0.5);

        // Gather out and in ports for this edge to build the condensed port routing matrix
        let out_node = self.graph.nodes.get(&edge.key.output_node_serial);
        let in_node = self.graph.nodes.get(&edge.key.input_node_serial);

        let out_ports: Vec<&PwPort> = out_node
            .map(|n| {
                if n.output_ports.iter().any(|p| !p.is_monitor) {
                    n.output_ports.iter().filter(|p| !p.is_monitor).collect()
                } else {
                    n.output_ports.iter().collect()
                }
            })
            .unwrap_or_default();
        let in_ports: Vec<&PwPort> = in_node
            .map(|n| n.input_ports.iter().collect())
            .unwrap_or_default();

        let num_out = out_ports.len().max(1);
        let num_in = in_ports.len().max(1);

        // 1. CONDENSED MATRIX REPRESENTATION:
        // Always drawn at the wire midpoint. Displays a mini graphical grid representing
        // the port routing matrix with small dots indicating connected port pairs.
        // For non-default connections, this mini-matrix is framed in an amber/coral badge.
        let dot_cell = (5.0 * self.zoom).clamp(3.5, 7.0);
        let dot_r = (1.5 * self.zoom).clamp(1.0, 2.2);
        let grid_w = num_in as f32 * dot_cell;
        let grid_h = num_out as f32 * dot_cell;
        let pad_x = (5.0 * self.zoom).clamp(3.0, 6.0);
        let pad_y = (4.0 * self.zoom).clamp(2.5, 5.0);

        let badge_w = grid_w + pad_x * 2.0;
        let badge_h = grid_h + pad_y * 2.0;
        let badge_top_left = Point::new(mid.x - badge_w * 0.5, mid.y - badge_h * 0.5);

        // Badge background
        let pill_bg = if is_selected {
            Color::from_rgb(0.14, 0.28, 0.38)
        } else if is_hovered {
            Color::from_rgb(0.16, 0.26, 0.24)
        } else if !edge.is_default {
            Color::from_rgba(0.20, 0.16, 0.12, 0.94)
        } else {
            Color::from_rgba(0.11, 0.13, 0.17, 0.92)
        };

        frame.fill_rectangle(badge_top_left, Size::new(badge_w, badge_h), pill_bg);

        // Badge border
        let border_rect = Path::rectangle(badge_top_left, Size::new(badge_w, badge_h));
        frame.stroke(
            &border_rect,
            Stroke {
                style: Style::Solid(base_color),
                width: if is_selected {
                    2.0
                } else if !edge.is_default {
                    1.4
                } else {
                    1.0
                },
                ..Stroke::default()
            },
        );

        // Draw condensed matrix grid dots
        let grid_origin_x = badge_top_left.x + pad_x + dot_cell * 0.5;
        let grid_origin_y = badge_top_left.y + pad_y + dot_cell * 0.5;

        for (row_idx, out_p) in out_ports.iter().enumerate() {
            for (col_idx, in_p) in in_ports.iter().enumerate() {
                let is_connected = edge.links.iter().any(|l| {
                    l.output_port_serial == out_p.object_serial
                        && l.input_port_serial == in_p.object_serial
                });

                let cx = grid_origin_x + col_idx as f32 * dot_cell;
                let cy = grid_origin_y + row_idx as f32 * dot_cell;
                let dot_circle = Path::circle(Point::new(cx, cy), dot_r);

                if is_connected {
                    frame.fill(&dot_circle, base_color);
                } else {
                    frame.fill(
                        &dot_circle,
                        Color::from_rgba(1.0, 1.0, 1.0, if !edge.is_default { 0.12 } else { 0.08 }),
                    );
                }
            }
        }

    }

    fn draw_wire_tooltip(
        &self,
        frame: &mut Frame,
        p0: Point,
        p3: Point,
        edge: &CollapsedEdge,
        is_selected: bool,
    ) {
        let base_color = if is_selected {
            Color::from_rgb(0.35, 0.88, 1.0)
        } else if !edge.is_default {
            Color::from_rgb(0.98, 0.68, 0.28)
        } else {
            Color::from_rgb(0.55, 0.95, 0.75)
        };

        let mid = bezier_point(p0, p3, 0.5);
        let text_size = (11.5 * self.zoom.clamp(0.8, 1.25)).max(9.5);

        let label = if !edge.is_default {
            format!("{} (custom)", edge.summary)
        } else {
            format!("{} (default)", edge.summary)
        };

        let tip_w = (label.len() as f32 * (text_size * 0.58) + 16.0).max(75.0);
        let tip_h = text_size + 10.0;
        let tip_x = mid.x - tip_w * 0.5;
        let tip_y = mid.y - tip_h - 14.0;

        let tip_rect = Rectangle::new(Point::new(tip_x, tip_y), Size::new(tip_w, tip_h));
        frame.fill_rectangle(
            tip_rect.position(),
            tip_rect.size(),
            Color::from_rgba(0.08, 0.10, 0.15, 0.97),
        );

        let tip_border = Path::rectangle(tip_rect.position(), tip_rect.size());
        frame.stroke(
            &tip_border,
            Stroke {
                style: Style::Solid(base_color),
                width: 1.2,
                ..Stroke::default()
            },
        );

        frame.fill_text(Text {
            content: label,
            position: Point::new(tip_x + 8.0, tip_y + 4.5),
            color: Color::from_rgb(0.94, 0.96, 0.98),
            size: text_size.into(),
            ..Text::default()
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_node(
        &self,
        frame: &mut Frame,
        node: &PwNode,
        world_pos: Point,
        is_selected: bool,
        is_hovered: bool,
        in_sock_hovered: bool,
        out_sock_hovered: bool,
    ) {
        let top_left = self.world_to_screen(world_pos);
        let size = Size::new(NODE_WIDTH * self.zoom, NODE_HEIGHT * self.zoom);
        let accent = category_color(node.category);

        // Card background
        let bg_color = if is_selected {
            Color::from_rgb(0.17, 0.20, 0.27)
        } else if is_hovered {
            Color::from_rgb(0.15, 0.17, 0.23)
        } else {
            Color::from_rgb(0.12, 0.14, 0.18)
        };
        frame.fill_rectangle(top_left, size, bg_color);

        // Top category header bar
        let header_h = 24.0 * self.zoom;
        frame.fill_rectangle(
            top_left,
            Size::new(size.width, header_h),
            Color::from_rgba(accent.r, accent.g, accent.b, 0.25),
        );

        // Card border
        let border_path = Path::rectangle(top_left, size);
        let border_color = if is_selected {
            Color::from_rgb(0.40, 0.85, 1.0)
        } else if is_hovered {
            accent
        } else {
            Color::from_rgba(accent.r, accent.g, accent.b, 0.55)
        };
        frame.stroke(
            &border_path,
            Stroke {
                style: Style::Solid(border_color),
                width: if is_selected { 2.2 } else { 1.3 },
                ..Stroke::default()
            },
        );

        // Header category text + serial
        let header_label = format!("{}  s#{}", node.category.label(), node.object_serial);
        frame.fill_text(Text {
            content: truncate_str(&header_label, 30),
            position: Point::new(top_left.x + 10.0 * self.zoom, top_left.y + 5.0 * self.zoom),
            color: accent,
            size: (11.0 * self.zoom).into(),
            ..Text::default()
        });

        // Node primary title
        frame.fill_text(Text {
            content: truncate_str(&node.title(), 26),
            position: Point::new(top_left.x + 12.0 * self.zoom, top_left.y + 31.0 * self.zoom),
            color: Color::from_rgb(0.95, 0.96, 0.98),
            size: (13.5 * self.zoom).into(),
            ..Text::default()
        });

        // Node subtitle
        frame.fill_text(Text {
            content: truncate_str(&node.subtitle(), 30),
            position: Point::new(top_left.x + 12.0 * self.zoom, top_left.y + 50.0 * self.zoom),
            color: Color::from_rgb(0.62, 0.66, 0.74),
            size: (11.0 * self.zoom).into(),
            ..Text::default()
        });

        // Port summary line and Volume / Mute badge at bottom of card
        let in_summary = if node.input_ports.is_empty() {
            String::new()
        } else {
            format!("In: {}p", node.input_ports.len())
        };
        let out_summary = if node.output_ports.is_empty() {
            String::new()
        } else {
            let non_mon = node.default_output_ports().len();
            format!("Out: {}p", non_mon)
        };
        let ports_line = match (in_summary.is_empty(), out_summary.is_empty()) {
            (false, false) => format!("{in_summary}   •   {out_summary}"),
            (false, true) => in_summary,
            (true, false) => out_summary,
            (true, true) => String::from("No ports"),
        };

        frame.fill_text(Text {
            content: ports_line,
            position: Point::new(top_left.x + 12.0 * self.zoom, top_left.y + 72.0 * self.zoom),
            color: Color::from_rgb(0.52, 0.76, 0.68),
            size: (11.0 * self.zoom).into(),
            ..Text::default()
        });

        if node.muted == Some(true) {
            let mute_color = Color::from_rgb(0.95, 0.42, 0.42);
            frame.fill_text(Text {
                content: ICON_VOLUME_X.to_string(),
                position: Point::new(
                    top_left.x + (NODE_WIDTH - 66.0) * self.zoom,
                    top_left.y + 71.5 * self.zoom,
                ),
                color: mute_color,
                size: (12.0 * self.zoom).into(),
                font: LUCIDE_FONT,
                ..Text::default()
            });
            frame.fill_text(Text {
                content: String::from("MUTED"),
                position: Point::new(
                    top_left.x + (NODE_WIDTH - 50.0) * self.zoom,
                    top_left.y + 72.0 * self.zoom,
                ),
                color: mute_color,
                size: (10.5 * self.zoom).into(),
                ..Text::default()
            });
        } else if let Some(vol) = node.volume {
            let vol_color = Color::from_rgb(0.75, 0.85, 0.96);
            frame.fill_text(Text {
                content: ICON_VOLUME_2.to_string(),
                position: Point::new(
                    top_left.x + (NODE_WIDTH - 62.0) * self.zoom,
                    top_left.y + 71.5 * self.zoom,
                ),
                color: vol_color,
                size: (12.0 * self.zoom).into(),
                font: LUCIDE_FONT,
                ..Text::default()
            });
            frame.fill_text(Text {
                content: format!("{:>3.0}%", vol * 100.0),
                position: Point::new(
                    top_left.x + (NODE_WIDTH - 46.0) * self.zoom,
                    top_left.y + 72.0 * self.zoom,
                ),
                color: vol_color,
                size: (10.5 * self.zoom).into(),
                ..Text::default()
            });
        }

        // Collapsed Input Socket (left edge)
        if !node.input_ports.is_empty() {
            let sock_screen = self.world_to_screen(Self::input_socket_world(world_pos));
            let r = SOCKET_RADIUS * self.zoom * if in_sock_hovered { 1.35 } else { 1.0 };
            let circle = Path::circle(sock_screen, r);
            frame.fill(
                &circle,
                if in_sock_hovered {
                    Color::from_rgb(0.40, 0.95, 0.75)
                } else {
                    Color::from_rgb(0.25, 0.78, 0.95)
                },
            );
            frame.stroke(
                &circle,
                Stroke {
                    style: Style::Solid(Color::from_rgb(0.08, 0.10, 0.14)),
                    width: 2.0,
                    ..Stroke::default()
                },
            );
        }

        // Collapsed Output Socket (right edge)
        if !node.output_ports.is_empty() {
            let sock_screen = self.world_to_screen(Self::output_socket_world(world_pos));
            let r = SOCKET_RADIUS * self.zoom * if out_sock_hovered { 1.35 } else { 1.0 };
            let circle = Path::circle(sock_screen, r);
            frame.fill(
                &circle,
                if out_sock_hovered {
                    Color::from_rgb(0.98, 0.80, 0.32)
                } else {
                    Color::from_rgb(0.38, 0.86, 0.56)
                },
            );
            frame.stroke(
                &circle,
                Stroke {
                    style: Style::Solid(Color::from_rgb(0.08, 0.10, 0.14)),
                    width: 2.0,
                    ..Stroke::default()
                },
            );
        }
    }
}

pub fn category_color(cat: NodeCategory) -> Color {
    match cat {
        NodeCategory::AudioSource => Color::from_rgb(0.32, 0.82, 0.56),
        NodeCategory::AudioOutputStream => Color::from_rgb(0.36, 0.68, 0.96),
        NodeCategory::AudioDuplex => Color::from_rgb(0.75, 0.52, 0.96),
        NodeCategory::AudioInputStream => Color::from_rgb(0.96, 0.68, 0.32),
        NodeCategory::AudioSink => Color::from_rgb(0.94, 0.45, 0.52),
        NodeCategory::Midi => Color::from_rgb(0.92, 0.82, 0.34),
        NodeCategory::Video => Color::from_rgb(0.35, 0.86, 0.86),
        NodeCategory::Internal => Color::from_rgb(0.58, 0.62, 0.68),
    }
}

fn bezier_controls(p0: Point, p3: Point) -> (Point, Point) {
    let dx = (p3.x - p0.x).abs().max(60.0) * 0.5;
    (Point::new(p0.x + dx, p0.y), Point::new(p3.x - dx, p3.y))
}

fn bezier_path(p0: Point, p3: Point) -> Path {
    let (c0, c1) = bezier_controls(p0, p3);
    Path::new(|builder| {
        builder.move_to(p0);
        builder.bezier_curve_to(c0, c1, p3);
    })
}


fn bezier_point(p0: Point, p3: Point, t: f32) -> Point {
    let (p1, p2) = bezier_controls(p0, p3);
    let u = 1.0 - t;
    let tt = t * t;
    let uu = u * u;
    let uuu = uu * u;
    let ttt = tt * t;

    let x = uuu * p0.x + 3.0 * uu * t * p1.x + 3.0 * u * tt * p2.x + ttt * p3.x;
    let y = uuu * p0.y + 3.0 * uu * t * p1.y + 3.0 * u * tt * p2.y + ttt * p3.y;
    Point::new(x, y)
}

fn distance_to_bezier(pt: Point, p0: Point, p3: Point) -> f32 {
    let steps = 28;
    let mut min_d = f32::MAX;
    let mut prev = p0;
    for i in 1..=steps {
        let t = i as f32 / steps as f32;
        let curr = bezier_point(p0, p3, t);
        let d = distance_to_segment(pt, prev, curr);
        if d < min_d {
            min_d = d;
        }
        prev = curr;
    }
    min_d
}

fn distance_to_segment(p: Point, a: Point, b: Point) -> f32 {
    let l2 = (b.x - a.x).powi(2) + (b.y - a.y).powi(2);
    if l2 == 0.0 {
        return distance(p, a);
    }
    let t = (((p.x - a.x) * (b.x - a.x) + (p.y - a.y) * (b.y - a.y)) / l2).clamp(0.0, 1.0);
    let proj = Point::new(a.x + t * (b.x - a.x), a.y + t * (b.y - a.y));
    distance(p, proj)
}

fn distance(a: Point, b: Point) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
