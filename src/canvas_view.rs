use crate::model::{CollapsedEdge, EdgeKey, NodeCategory, PwGraph, PwNode, PwPort};
use iced::mouse;
use iced::widget::canvas::{
    self, Event, Frame, Geometry, LineCap, Path, Program, Stroke, Style, Text,
};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme, Vector};
use iced_fonts::LUCIDE_FONT;
use std::collections::{BTreeMap, HashMap, HashSet};

pub const ICON_VOLUME_X: &str = "\u{e1ac}";
pub const ICON_VOLUME_2: &str = "\u{e1ab}";

pub const NODE_WIDTH: f32 = 235.0;
pub const NODE_HEIGHT: f32 = 96.0;
pub const SOCKET_RADIUS: f32 = 8.0;
pub const ROW_HEIGHT: f32 = 130.0;
pub const COL_SPACING: f32 = 340.0;
pub const INITIAL_X: f32 = 40.0;
pub const INITIAL_Y: f32 = 40.0;

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
    pub cursor_pos: Option<Point>,
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
    pub search_query: &'a str,
}

impl<'a> GraphCanvas<'a> {
    pub fn is_search_active(&self) -> bool {
        !self.search_query.trim().is_empty()
    }

    pub fn is_node_dimmed(&self, node_serial: u64) -> bool {
        let trimmed = self.search_query.trim();
        if trimmed.is_empty() {
            return false;
        }
        if let Some(node) = self.graph.nodes.get(&node_serial) {
            !crate::model::matches_search(&node.name, &node.description, trimmed)
        } else {
            true
        }
    }

    pub fn is_link_dimmed(&self, edge: &EdgeKey) -> bool {
        let trimmed = self.search_query.trim();
        if trimmed.is_empty() {
            return false;
        }
        self.is_node_dimmed(edge.output_node_serial) || self.is_node_dimmed(edge.input_node_serial)
    }

    #[allow(dead_code)]
    pub fn dimmed_nodes(&self) -> HashSet<u64> {
        crate::model::compute_dimmed_nodes(self.graph, self.search_query)
    }

    #[allow(dead_code)]
    pub fn dimmed_links(&self) -> HashSet<EdgeKey> {
        crate::model::compute_dimmed_links(
            self.graph,
            &self.dimmed_nodes(),
            self.is_search_active(),
        )
    }
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
                if let Some(cursor_pos) = cursor.position_in(bounds) {
                    state.cursor_pos = Some(cursor_pos);
                    let world_pt = self.screen_to_world(cursor_pos);
                    state.hovered_output_socket = self.hit_test_output_socket(world_pt);
                    state.hovered_input_socket = self.hit_test_input_socket(world_pt);
                }
            }
            return None;
        }

        if let Event::Mouse(mouse::Event::CursorLeft) = event {
            state.cursor_pos = None;
            state.hovered_edge = None;
            state.hovered_node = None;
            state.hovered_input_socket = None;
            state.hovered_output_socket = None;
            return Some(canvas::Action::request_redraw());
        }

        let cursor_pos = cursor.position_in(bounds)?;
        let world_pt = self.screen_to_world(cursor_pos);

        match event {
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                state.cursor_pos = Some(cursor_pos);

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
                    || state.hovered_edge.is_some()
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
                    let is_dimmed = self.is_link_dimmed(&edge.key);

                    self.draw_collapsed_wire(
                        &mut base_frame,
                        p0,
                        p3,
                        edge,
                        is_selected,
                        is_hovered,
                        is_dimmed,
                    );
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
                        let is_dimmed = self.is_node_dimmed(node_serial);

                        self.draw_node(
                            &mut base_frame,
                            node,
                            world_pos,
                            false,
                            is_hovered,
                            in_sock_hovered,
                            out_sock_hovered,
                            is_dimmed,
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
                    let canvas_bounds = Rectangle::new(Point::ORIGIN, bounds.size());
                    self.draw_wire_tooltip(
                        &mut base_frame,
                        p0,
                        p3,
                        edge,
                        is_selected,
                        is_hovered,
                        state.cursor_pos,
                        canvas_bounds,
                    );
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
                            let is_dimmed = self.is_node_dimmed(act_serial);

                            self.draw_node(
                                &mut act_frame,
                                node,
                                world_pos,
                                true,
                                false,
                                in_sock_hovered,
                                out_sock_hovered,
                                is_dimmed,
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

    #[allow(clippy::too_many_arguments)]
    fn draw_collapsed_wire(
        &self,
        frame: &mut Frame,
        p0: Point,
        p3: Point,
        edge: &CollapsedEdge,
        is_selected: bool,
        is_hovered: bool,
        is_dimmed: bool,
    ) {
        let path = bezier_path(p0, p3);
        let wire_alpha = if is_dimmed { 0.18 } else { 1.0 };

        // Styling: Non-default connections stand out with an amber/coral hue,
        // while default connections use a clean teal/green hue.
        let base_color = if is_selected {
            Color::from_rgba(0.35, 0.88, 1.0, wire_alpha)
        } else if is_hovered {
            Color::from_rgba(0.55, 0.95, 0.75, wire_alpha)
        } else if !edge.is_default {
            Color::from_rgba(0.98, 0.68, 0.28, wire_alpha)
        } else {
            Color::from_rgba(0.35, 0.75, 0.58, wire_alpha)
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
                        0.32 * wire_alpha,
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
                    style: Style::Solid(Color::from_rgba(0.98, 0.68, 0.28, 0.16 * wire_alpha)),
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

        if should_show_link_badge(edge.is_default, is_hovered, is_selected) {
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
                Color::from_rgba(0.14, 0.28, 0.38, wire_alpha)
            } else if is_hovered {
                Color::from_rgba(0.16, 0.26, 0.24, wire_alpha)
            } else if !edge.is_default {
                Color::from_rgba(0.20, 0.16, 0.12, 0.94 * wire_alpha)
            } else {
                Color::from_rgba(0.11, 0.13, 0.17, 0.92 * wire_alpha)
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
                        let dot_alpha = (if !edge.is_default { 0.12 } else { 0.08 }) * wire_alpha;
                        frame.fill(
                            &dot_circle,
                            Color::from_rgba(1.0, 1.0, 1.0, dot_alpha),
                        );
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_wire_tooltip(
        &self,
        frame: &mut Frame,
        p0: Point,
        p3: Point,
        edge: &CollapsedEdge,
        is_selected: bool,
        is_hovered: bool,
        cursor_pos: Option<Point>,
        bounds: Rectangle,
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

        let anchor = if is_hovered {
            cursor_pos.filter(|pt| bounds.contains(*pt)).unwrap_or(mid)
        } else {
            mid
        };

        let tip_rect = compute_tooltip_rect(&label, text_size, anchor, bounds);

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
            position: Point::new(tip_rect.x + 8.0, tip_rect.y + (tip_rect.height - text_size) * 0.5),
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
        is_dimmed: bool,
    ) {
        let top_left = self.world_to_screen(world_pos);
        let size = Size::new(NODE_WIDTH * self.zoom, NODE_HEIGHT * self.zoom);
        let accent = category_color(node.category);
        let dim_alpha = if is_dimmed { 0.28 } else { 1.0 };

        // Card background
        let bg_color = if is_selected {
            Color::from_rgba(0.17, 0.20, 0.27, dim_alpha)
        } else if is_hovered {
            Color::from_rgba(0.15, 0.17, 0.23, dim_alpha)
        } else {
            Color::from_rgba(0.12, 0.14, 0.18, dim_alpha)
        };
        frame.fill_rectangle(top_left, size, bg_color);

        // Top category header bar
        let header_h = 24.0 * self.zoom;
        frame.fill_rectangle(
            top_left,
            Size::new(size.width, header_h),
            Color::from_rgba(accent.r, accent.g, accent.b, 0.25 * dim_alpha),
        );

        // Card border
        let border_path = Path::rectangle(top_left, size);
        let border_color = if is_selected {
            Color::from_rgba(0.40, 0.85, 1.0, dim_alpha)
        } else if is_hovered {
            Color::from_rgba(accent.r, accent.g, accent.b, dim_alpha)
        } else {
            Color::from_rgba(accent.r, accent.g, accent.b, 0.55 * dim_alpha)
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
            color: Color::from_rgba(accent.r, accent.g, accent.b, dim_alpha),
            size: (11.0 * self.zoom).into(),
            ..Text::default()
        });

        // Node primary title
        frame.fill_text(Text {
            content: truncate_str(&node.title(), 26),
            position: Point::new(top_left.x + 12.0 * self.zoom, top_left.y + 31.0 * self.zoom),
            color: Color::from_rgba(0.95, 0.96, 0.98, dim_alpha),
            size: (13.5 * self.zoom).into(),
            ..Text::default()
        });

        // Node subtitle
        frame.fill_text(Text {
            content: truncate_str(&node.subtitle(), 30),
            position: Point::new(top_left.x + 12.0 * self.zoom, top_left.y + 50.0 * self.zoom),
            color: Color::from_rgba(0.62, 0.66, 0.74, dim_alpha),
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
            color: Color::from_rgba(0.52, 0.76, 0.68, dim_alpha),
            size: (11.0 * self.zoom).into(),
            ..Text::default()
        });

        if node.muted == Some(true) {
            let mute_color = Color::from_rgba(0.95, 0.42, 0.42, dim_alpha);
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
            let vol_color = Color::from_rgba(0.75, 0.85, 0.96, dim_alpha);
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
            let fill_col = if in_sock_hovered {
                Color::from_rgba(0.40, 0.95, 0.75, dim_alpha)
            } else {
                Color::from_rgba(0.25, 0.78, 0.95, dim_alpha)
            };
            frame.fill(&circle, fill_col);
            frame.stroke(
                &circle,
                Stroke {
                    style: Style::Solid(Color::from_rgba(0.08, 0.10, 0.14, dim_alpha)),
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
            let fill_col = if out_sock_hovered {
                Color::from_rgba(0.98, 0.80, 0.32, dim_alpha)
            } else {
                Color::from_rgba(0.38, 0.86, 0.56, dim_alpha)
            };
            frame.fill(&circle, fill_col);
            frame.stroke(
                &circle,
                Stroke {
                    style: Style::Solid(Color::from_rgba(0.08, 0.10, 0.14, dim_alpha)),
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

pub fn bezier_controls(p0: Point, p3: Point) -> (Point, Point) {
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


pub fn bezier_point(p0: Point, p3: Point, t: f32) -> Point {
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

pub fn node_input_socket(node_pos: Point) -> Point {
    Point::new(node_pos.x, node_pos.y + NODE_HEIGHT * 0.55)
}

pub fn node_output_socket(node_pos: Point) -> Point {
    Point::new(node_pos.x + NODE_WIDTH, node_pos.y + NODE_HEIGHT * 0.55)
}

fn line_segment_intersects_rect(
    p1: Point,
    p2: Point,
    min_x: f32,
    max_x: f32,
    min_y: f32,
    max_y: f32,
) -> bool {
    let seg_min_x = p1.x.min(p2.x);
    let seg_max_x = p1.x.max(p2.x);
    let seg_min_y = p1.y.min(p2.y);
    let seg_max_y = p1.y.max(p2.y);

    if seg_max_x < min_x || seg_min_x > max_x || seg_max_y < min_y || seg_min_y > max_y {
        return false;
    }

    let dx = p2.x - p1.x;
    let dy = p2.y - p1.y;

    let mut t0 = 0.0f32;
    let mut t1 = 1.0f32;

    let p = [-dx, dx, -dy, dy];
    let q = [p1.x - min_x, max_x - p1.x, p1.y - min_y, max_y - p1.y];

    for i in 0..4 {
        if p[i] == 0.0 {
            if q[i] < 0.0 {
                return false;
            }
        } else {
            let t = q[i] / p[i];
            if p[i] < 0.0 {
                if t > t1 {
                    return false;
                }
                if t > t0 {
                    t0 = t;
                }
            } else {
                if t < t0 {
                    return false;
                }
                if t < t1 {
                    t1 = t;
                }
            }
        }
    }
    t0 <= t1
}

/// Returns true if the node bounding rectangle (expanded with 4px margin) intersects
/// the cubic bezier curve between p0 (output socket) and p3 (input socket).
/// Uses N=40 sampled points along the curve plus segment-rectangle intersection tests.
pub fn node_intersects_link_curve(node_rect: Rectangle, p0: Point, p3: Point) -> bool {
    let steps = 40;
    let margin = 4.0;
    let min_x = node_rect.x - margin;
    let max_x = node_rect.x + node_rect.width + margin;
    let min_y = node_rect.y - margin;
    let max_y = node_rect.y + node_rect.height + margin;

    let mut prev = p0;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let pt = bezier_point(p0, p3, t);

        if pt.x >= min_x && pt.x <= max_x && pt.y >= min_y && pt.y <= max_y {
            return true;
        }

        if i > 0 && line_segment_intersects_rect(prev, pt, min_x, max_x, min_y, max_y) {
            return true;
        }
        prev = pt;
    }
    false
}

/// Computes signal-flow auto-layout positions for all visible nodes:
/// 1. Assign columns by topological signal-flow depth (R4).
/// 2. Barycentric vertical ordering across columns to minimize link crossings.
/// 3. Sampled bezier curve collision check with row slot adjustment so no node is placed on crossing wires.
/// 4. Ensures no two nodes overlap (maintaining standard separation).
pub fn compute_auto_layout_positions(
    graph: &PwGraph,
    visible_nodes: &HashSet<u64>,
) -> BTreeMap<u64, Point> {
    if visible_nodes.is_empty() {
        return BTreeMap::new();
    }

    let columns_map = crate::model::compute_signal_flow_columns(graph, visible_nodes);
    let mut col_groups: BTreeMap<usize, Vec<u64>> = BTreeMap::new();
    for &serial in visible_nodes {
        if graph.nodes.contains_key(&serial) {
            let col = columns_map.get(&serial).copied().unwrap_or(0);
            col_groups.entry(col).or_default().push(serial);
        }
    }

    if col_groups.is_empty() {
        return BTreeMap::new();
    }

    let max_col = *col_groups.keys().max().unwrap_or(&0);

    // Initial deterministic sorting and row placement
    for nodes in col_groups.values_mut() {
        nodes.sort();
    }

    let mut positions: BTreeMap<u64, Point> = BTreeMap::new();
    for (&col, nodes) in &col_groups {
        let x = INITIAL_X + col as f32 * COL_SPACING;
        for (row_idx, &serial) in nodes.iter().enumerate() {
            let y = INITIAL_Y + row_idx as f32 * ROW_HEIGHT;
            positions.insert(serial, Point::new(x, y));
        }
    }

    // Build adjacency for barycentric ordering
    let mut in_edges: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut out_edges: HashMap<u64, Vec<u64>> = HashMap::new();
    for edge in graph.collapsed_edges() {
        let u = edge.key.output_node_serial;
        let v = edge.key.input_node_serial;
        if visible_nodes.contains(&u) && visible_nodes.contains(&v) && u != v {
            in_edges.entry(v).or_default().push(u);
            out_edges.entry(u).or_default().push(v);
        }
    }

    // Barycentric vertical ordering passes
    // Pass 1: Forward (columns 1..=max_col sort by upstream barycenter)
    for col in 1..=max_col {
        if let Some(nodes) = col_groups.get_mut(&col) {
            nodes.sort_by(|&a, &b| {
                let bary_a = match in_edges.get(&a) {
                    Some(preds) if !preds.is_empty() => {
                        let count = preds.iter().filter(|p| positions.contains_key(p)).count();
                        if count > 0 {
                            let sum: f32 = preds.iter().filter_map(|p| positions.get(p)).map(|p| p.y).sum();
                            sum / count as f32
                        } else {
                            1_000_000.0 + a as f32
                        }
                    }
                    _ => 1_000_000.0 + a as f32, // Unconnected sinks placed after connected
                };
                let bary_b = match in_edges.get(&b) {
                    Some(preds) if !preds.is_empty() => {
                        let count = preds.iter().filter(|p| positions.contains_key(p)).count();
                        if count > 0 {
                            let sum: f32 = preds.iter().filter_map(|p| positions.get(p)).map(|p| p.y).sum();
                            sum / count as f32
                        } else {
                            1_000_000.0 + b as f32
                        }
                    }
                    _ => 1_000_000.0 + b as f32,
                };
                bary_a.partial_cmp(&bary_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.cmp(&b))
            });

            // Reassign y positions in this column
            for (row_idx, &serial) in nodes.iter().enumerate() {
                if let Some(p) = positions.get_mut(&serial) {
                    p.y = INITIAL_Y + row_idx as f32 * ROW_HEIGHT;
                }
            }
        }
    }

    // Pass 2: Backward (columns max_col..=0 sort by downstream barycenter)
    for col in (0..=max_col).rev() {
        if let Some(nodes) = col_groups.get_mut(&col) {
            nodes.sort_by(|&a, &b| {
                let bary_a = match out_edges.get(&a) {
                    Some(succs) if !succs.is_empty() => {
                        let count = succs.iter().filter(|s| positions.contains_key(s)).count();
                        if count > 0 {
                            let sum: f32 = succs.iter().filter_map(|s| positions.get(s)).map(|p| p.y).sum();
                            sum / count as f32
                        } else {
                            1_000_000.0 + a as f32
                        }
                    }
                    _ => {
                        match in_edges.get(&a) {
                            Some(preds) if !preds.is_empty() => {
                                let count = preds.iter().filter(|p| positions.contains_key(p)).count();
                                if count > 0 {
                                    let sum: f32 = preds.iter().filter_map(|p| positions.get(p)).map(|p| p.y).sum();
                                    sum / count as f32
                                } else {
                                    1_000_000.0 + a as f32
                                }
                            }
                            _ => 1_000_000.0 + a as f32,
                        }
                    }
                };
                let bary_b = match out_edges.get(&b) {
                    Some(succs) if !succs.is_empty() => {
                        let count = succs.iter().filter(|s| positions.contains_key(s)).count();
                        if count > 0 {
                            let sum: f32 = succs.iter().filter_map(|s| positions.get(s)).map(|p| p.y).sum();
                            sum / count as f32
                        } else {
                            1_000_000.0 + b as f32
                        }
                    }
                    _ => {
                        match in_edges.get(&b) {
                            Some(preds) if !preds.is_empty() => {
                                let count = preds.iter().filter(|p| positions.contains_key(p)).count();
                                if count > 0 {
                                    let sum: f32 = preds.iter().filter_map(|p| positions.get(p)).map(|p| p.y).sum();
                                    sum / count as f32
                                } else {
                                    1_000_000.0 + b as f32
                                }
                            }
                            _ => 1_000_000.0 + b as f32,
                        }
                    }
                };
                bary_a.partial_cmp(&bary_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.cmp(&b))
            });

            for (row_idx, &serial) in nodes.iter().enumerate() {
                if let Some(p) = positions.get_mut(&serial) {
                    p.y = INITIAL_Y + row_idx as f32 * ROW_HEIGHT;
                }
            }
        }
    }

    // Pass 3: Final forward pass (columns 1..=max_col sort by upstream barycenter)
    for col in 1..=max_col {
        if let Some(nodes) = col_groups.get_mut(&col) {
            nodes.sort_by(|&a, &b| {
                let bary_a = match in_edges.get(&a) {
                    Some(preds) if !preds.is_empty() => {
                        let count = preds.iter().filter(|p| positions.contains_key(p)).count();
                        if count > 0 {
                            let sum: f32 = preds.iter().filter_map(|p| positions.get(p)).map(|p| p.y).sum();
                            sum / count as f32
                        } else {
                            1_000_000.0 + a as f32
                        }
                    }
                    _ => 1_000_000.0 + a as f32,
                };
                let bary_b = match in_edges.get(&b) {
                    Some(preds) if !preds.is_empty() => {
                        let count = preds.iter().filter(|p| positions.contains_key(p)).count();
                        if count > 0 {
                            let sum: f32 = preds.iter().filter_map(|p| positions.get(p)).map(|p| p.y).sum();
                            sum / count as f32
                        } else {
                            1_000_000.0 + b as f32
                        }
                    }
                    _ => 1_000_000.0 + b as f32,
                };
                bary_a.partial_cmp(&bary_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.cmp(&b))
            });

            for (row_idx, &serial) in nodes.iter().enumerate() {
                if let Some(p) = positions.get_mut(&serial) {
                    p.y = INITIAL_Y + row_idx as f32 * ROW_HEIGHT;
                }
            }
        }
    }

    // Sampled Bezier Curve Collision Check & Row Slot Adjustment
    let collapsed_edges = graph.collapsed_edges();

    // Sort edges by column span descending: long-span bypass edges establish
    // clearance corridors first, avoiding cascading oscillations.
    let mut sorted_edges = collapsed_edges;
    sorted_edges.sort_by(|a, b| {
        let u_a = a.key.output_node_serial;
        let v_a = a.key.input_node_serial;
        let u_b = b.key.output_node_serial;
        let v_b = b.key.input_node_serial;

        let col_u_a = columns_map.get(&u_a).copied().unwrap_or(0) as isize;
        let col_v_a = columns_map.get(&v_a).copied().unwrap_or(0) as isize;
        let span_a = (col_v_a - col_u_a).abs();

        let col_u_b = columns_map.get(&u_b).copied().unwrap_or(0) as isize;
        let col_v_b = columns_map.get(&v_b).copied().unwrap_or(0) as isize;
        let span_b = (col_v_b - col_u_b).abs();

        span_b
            .cmp(&span_a)
            .then_with(|| u_a.cmp(&u_b))
            .then_with(|| v_a.cmp(&v_b))
    });

    for _pass in 0..50 {
        let mut any_shift = false;

        for edge in &sorted_edges {
            let u = edge.key.output_node_serial;
            let v = edge.key.input_node_serial;
            let (Some(&pos_u), Some(&pos_v)) = (positions.get(&u), positions.get(&v)) else {
                continue;
            };
            let p0 = node_output_socket(pos_u);
            let p3 = node_input_socket(pos_v);

            for (&w_serial, pos_w) in positions.iter_mut() {
                if w_serial == u || w_serial == v {
                    continue;
                }
                let w_rect = Rectangle::new(*pos_w, Size::new(NODE_WIDTH, NODE_HEIGHT));
                if node_intersects_link_curve(w_rect, p0, p3) {
                    let target_y = f32::max(
                        pos_w.y + ROW_HEIGHT,
                        f32::max(pos_u.y, pos_v.y) + ROW_HEIGHT,
                    );
                    pos_w.y = target_y;
                    any_shift = true;
                }
            }
        }

        // Maintain column separation and ordering without vertical overlap
        for nodes in col_groups.values_mut() {
            nodes.sort_by(|&a, &b| {
                let ya = positions.get(&a).map_or(0.0, |p| p.y);
                let yb = positions.get(&b).map_or(0.0, |p| p.y);
                ya.partial_cmp(&yb)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.cmp(&b))
            });

            let mut min_y = INITIAL_Y;
            for &serial in nodes.iter() {
                if let Some(pos) = positions.get_mut(&serial) {
                    if pos.y < min_y {
                        pos.y = min_y;
                        any_shift = true;
                    }
                    min_y = pos.y + ROW_HEIGHT;
                }
            }
        }

        if !any_shift {
            break;
        }
    }

    positions
}

/// Finds a sensible free spot for newly appearing nodes without moving existing nodes (R4):
/// - Computes target column from connected upstream/downstream nodes or node category.
/// - Probes row coordinates (y = 40.0, 170.0, 300.0, 430.0...).
/// - Validates candidate: no overlap with any existing node, no intersection with existing sampled link curves.
/// - Leaves all existing node positions strictly unchanged.
pub fn find_sensible_free_spot(
    graph: &PwGraph,
    existing_positions: &BTreeMap<u64, Point>,
    new_node: &PwNode,
) -> Point {
    if existing_positions.is_empty() {
        return Point::new(INITIAL_X, INITIAL_Y);
    }

    let col_from_x = |x: f32| -> usize {
        if x <= INITIAL_X {
            0
        } else {
            ((x - INITIAL_X + COL_SPACING * 0.5) / COL_SPACING).floor().max(0.0) as usize
        }
    };

    let mut upstream_cols = Vec::new();
    let mut downstream_cols = Vec::new();

    for edge in graph.collapsed_edges() {
        if edge.key.input_node_serial == new_node.object_serial {
            if let Some(&u_pos) = existing_positions.get(&edge.key.output_node_serial) {
                upstream_cols.push(col_from_x(u_pos.x));
            }
        }
        if edge.key.output_node_serial == new_node.object_serial {
            if let Some(&v_pos) = existing_positions.get(&edge.key.input_node_serial) {
                downstream_cols.push(col_from_x(v_pos.x));
            }
        }
    }

    let target_col = if !upstream_cols.is_empty() {
        upstream_cols.into_iter().max().unwrap_or(0) + 1
    } else if !downstream_cols.is_empty() {
        downstream_cols.into_iter().min().unwrap_or(1).saturating_sub(1)
    } else {
        match new_node.category {
            NodeCategory::AudioSource | NodeCategory::AudioOutputStream => 0,
            _ => 1,
        }
    };

    let target_x = INITIAL_X + target_col as f32 * COL_SPACING;
    let collapsed_edges = graph.collapsed_edges();

    // Probe candidate row coordinates: y = 40.0, 170.0, 300.0, 430.0...
    for r in 0..1000 {
        let candidate_y = INITIAL_Y + r as f32 * ROW_HEIGHT;
        let candidate_pt = Point::new(target_x, candidate_y);
        let candidate_rect = Rectangle::new(candidate_pt, Size::new(NODE_WIDTH, NODE_HEIGHT));

        // 1. Check no overlap with any existing node
        let mut overlaps = false;
        for (&serial, &pos) in existing_positions {
            if serial == new_node.object_serial {
                continue;
            }
            let dx = (pos.x - candidate_pt.x).abs();
            let dy = (pos.y - candidate_pt.y).abs();
            if dx < NODE_WIDTH + 12.0 && dy < NODE_HEIGHT + 12.0 {
                overlaps = true;
                break;
            }
        }
        if overlaps {
            continue;
        }

        // 2. Check no intersection with any existing sampled link curve
        let mut collides_wire = false;
        for edge in &collapsed_edges {
            let u = edge.key.output_node_serial;
            let v = edge.key.input_node_serial;
            if u == new_node.object_serial || v == new_node.object_serial {
                continue;
            }
            let (Some(&pos_u), Some(&pos_v)) = (existing_positions.get(&u), existing_positions.get(&v)) else {
                continue;
            };
            let p0 = node_output_socket(pos_u);
            let p3 = node_input_socket(pos_v);
            if node_intersects_link_curve(candidate_rect, p0, p3) {
                collides_wire = true;
                break;
            }
        }
        if collides_wire {
            continue;
        }

        return candidate_pt;
    }

    Point::new(target_x, INITIAL_Y + 1000.0 * ROW_HEIGHT)
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

/// Determines whether to render the mini condensed routing matrix badge on a link wire.
/// Only non-default, hovered, or selected links display the badge.
pub fn should_show_link_badge(is_default: bool, is_hovered: bool, is_selected: bool) -> bool {
    !is_default || is_hovered || is_selected
}

/// Computes a tight proportional width for a link tooltip based on character glyph widths and font size.
pub fn compute_tooltip_width(label: &str, text_size: f32) -> f32 {
    let text_size = text_size.max(0.0);
    let char_units: f32 = label
        .chars()
        .map(|c| match c {
            'i' | 'l' | '!' | '|' | ':' | ';' | '.' | ',' | '\'' | '`' => 0.28,
            'r' | 't' | 'f' | 'j' | '(' | ')' | '[' | ']' | '{' | '}' | '-' => 0.38,
            'm' | 'w' | 'M' | 'W' => 0.82,
            'A'..='Z' => 0.65,
            ' ' => 0.32,
            _ => 0.54,
        })
        .sum();
    char_units * text_size + 16.0
}

/// Computes a clamped bounding rectangle for a link tooltip anchored near a target point.
/// Ensures the tooltip is positioned tightly near the anchor and stays completely within canvas bounds.
pub fn compute_tooltip_rect(
    label: &str,
    text_size: f32,
    anchor: Point,
    bounds: Rectangle,
) -> Rectangle {
    let w = compute_tooltip_width(label, text_size);
    let h = text_size + 10.0;
    let margin = 6.0;

    // Anchor horizontally centered on anchor
    let x = anchor.x - w * 0.5;
    // Anchor vertically above anchor
    let mut y = anchor.y - h - 10.0;

    // If placed above canvas top margin, flip below anchor
    if y < bounds.y + margin {
        y = anchor.y + 16.0;
    }

    // Clamp strictly inside canvas bounds
    let min_x = bounds.x + margin;
    let max_x = (bounds.x + bounds.width - w - margin).max(bounds.x);
    let clamped_x = if max_x >= min_x {
        x.clamp(min_x, max_x)
    } else {
        bounds.x
    };

    let min_y = bounds.y + margin;
    let max_y = (bounds.y + bounds.height - h - margin).max(bounds.y);
    let clamped_y = if max_y >= min_y {
        y.clamp(min_y, max_y)
    } else {
        bounds.y
    };

    Rectangle::new(Point::new(clamped_x, clamped_y), Size::new(w, h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::widget::canvas::Program;

    #[test]
    fn test_r3_badge_visibility_truth_table() {
        let mut combinations_tested = 0;
        for is_default in [false, true] {
            for is_hovered in [false, true] {
                for is_selected in [false, true] {
                    let show = should_show_link_badge(is_default, is_hovered, is_selected);
                    let expected = !is_default || is_hovered || is_selected;
                    assert_eq!(
                        show, expected,
                        "Failed for permutation: is_default={}, is_hovered={}, is_selected={}",
                        is_default, is_hovered, is_selected
                    );
                    // Specifically verify that only (true, false, false) returns false
                    if is_default && !is_hovered && !is_selected {
                        assert!(!show, "Default unhovered unselected link must NOT show badge");
                    } else {
                        assert!(show, "Non-default, hovered, or selected link MUST show badge");
                    }
                    combinations_tested += 1;
                }
            }
        }
        assert_eq!(combinations_tested, 8, "Must test all 8 permutations");
    }

    #[test]
    fn test_r2_tooltip_width_proportionality() {
        let text_size = 12.0;

        // 1. Monotonicity and proportional scaling with label length
        let short_label = "2ch";
        let medium_label = "2ch (default)";
        let long_label = "Playback / Built-in Audio Analog Stereo -> System (custom)";

        let w_short = compute_tooltip_width(short_label, text_size);
        let w_medium = compute_tooltip_width(medium_label, text_size);
        let w_long = compute_tooltip_width(long_label, text_size);

        assert!(
            w_short < w_medium,
            "Short label width {} should be less than medium {}",
            w_short,
            w_medium
        );
        assert!(
            w_medium < w_long,
            "Medium label width {} should be less than long {}",
            w_medium,
            w_long
        );

        // 2. Strict character length proportionality on uniform strings
        let pad = 16.0;
        let s1 = "x";
        let s5 = "xxxxx";
        let s10 = "xxxxxxxxxx";

        let w1 = compute_tooltip_width(s1, text_size);
        let w5 = compute_tooltip_width(s5, text_size);
        let w10 = compute_tooltip_width(s10, text_size);

        let text_w1 = w1 - pad;
        let text_w5 = w5 - pad;
        let text_w10 = w10 - pad;

        assert!(
            (text_w5 - 5.0 * text_w1).abs() < 1e-4,
            "5-char text width ({}) should be 5x 1-char width ({})",
            text_w5,
            text_w1
        );
        assert!(
            (text_w10 - 10.0 * text_w1).abs() < 1e-4,
            "10-char text width ({}) should be 10x 1-char width ({})",
            text_w10,
            text_w1
        );

        // 3. No large artificial clamping / huge empty margins on short strings
        assert!(
            w1 < 30.0,
            "Single character label should have tight width without huge empty margins, got {}",
            w1
        );
    }

    #[test]
    fn test_r2_tooltip_bounds_clamping_corners() {
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(800.0, 600.0));
        let text_size = 11.5;
        let label = "FL, FR -> playback_FL, playback_FR (default)";

        let test_corners = [
            ("Top-Left (0, 0)", Point::new(0.0, 0.0)),
            ("Top-Right (bounds.width, 0)", Point::new(bounds.width, 0.0)),
            ("Bottom-Left (0, bounds.height)", Point::new(0.0, bounds.height)),
            (
                "Bottom-Right (bounds.width, bounds.height)",
                Point::new(bounds.width, bounds.height),
            ),
        ];

        for (name, corner) in test_corners {
            let rect = compute_tooltip_rect(label, text_size, corner, bounds);

            assert!(
                rect.x >= bounds.x,
                "{}: rect.x ({}) must be >= bounds.x ({})",
                name,
                rect.x,
                bounds.x
            );
            assert!(
                rect.y >= bounds.y,
                "{}: rect.y ({}) must be >= bounds.y ({})",
                name,
                rect.y,
                bounds.y
            );
            assert!(
                rect.x + rect.width <= bounds.x + bounds.width,
                "{}: rect right edge ({}) must be <= bounds right edge ({})",
                name,
                rect.x + rect.width,
                bounds.x + bounds.width
            );
            assert!(
                rect.y + rect.height <= bounds.y + bounds.height,
                "{}: rect bottom edge ({}) must be <= bounds bottom edge ({})",
                name,
                rect.y + rect.height,
                bounds.y + bounds.height
            );
        }
    }

    #[test]
    fn test_r2_tooltip_zoom_scaling() {
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(800.0, 600.0));
        let anchor = Point::new(400.0, 300.0);
        let label = "Stereo Out -> Main (default)";

        let zoom_levels = [0.5_f32, 1.0_f32, 2.0_f32];
        let mut rects = Vec::new();

        for &zoom in &zoom_levels {
            let text_size = (11.5 * zoom.clamp(0.8, 1.25)).max(9.5);
            let rect = compute_tooltip_rect(label, text_size, anchor, bounds);

            // Clamping check at every zoom level
            assert!(rect.x >= bounds.x && rect.x + rect.width <= bounds.x + bounds.width);
            assert!(rect.y >= bounds.y && rect.y + rect.height <= bounds.y + bounds.height);

            rects.push((zoom, text_size, rect));
        }

        // Text size at zoom 0.5 <= zoom 1.0 < zoom 2.0
        assert!(rects[0].1 <= rects[1].1);
        assert!(rects[1].1 < rects[2].1);

        // Tooltip dimensions scale proportionally with zoom / text size
        assert!(rects[1].2.width < rects[2].2.width);
        assert!(rects[1].2.height < rects[2].2.height);

        // Extreme zoom / large text_size inside small bounds: remains strictly clamped
        let small_bounds = Rectangle::new(Point::ORIGIN, Size::new(120.0, 80.0));
        let big_text_size = 24.0;
        let clamped = compute_tooltip_rect(label, big_text_size, Point::new(10.0, 10.0), small_bounds);
        assert!(clamped.x >= small_bounds.x);
        assert!(clamped.y >= small_bounds.y);
        assert!(clamped.x + clamped.width <= small_bounds.x + small_bounds.width || clamped.x == small_bounds.x);
        assert!(clamped.y + clamped.height <= small_bounds.y + small_bounds.height || clamped.y == small_bounds.y);
    }

    #[test]
    fn test_r2_tooltip_anchor_near_cursor() {
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(1000.0, 800.0));
        let cursor = Point::new(500.0, 400.0);
        let label = "Test Label";
        let text_size = 12.0;

        let rect = compute_tooltip_rect(label, text_size, cursor, bounds);

        // Centered horizontally on cursor
        let actual_center_x = rect.x + rect.width * 0.5;
        assert!(
            (actual_center_x - cursor.x).abs() < 1e-4,
            "Tooltip must be centered horizontally on cursor: actual {}, expected {}",
            actual_center_x,
            cursor.x
        );

        // Placed tightly above cursor by offset
        assert!(
            rect.y + rect.height < cursor.y,
            "Tooltip rect bottom {} should be above cursor y {}",
            rect.y + rect.height,
            cursor.y
        );
        assert!(
            cursor.y - (rect.y + rect.height) <= 15.0,
            "Tooltip should be tightly offset above cursor"
        );
    }

    #[test]
    fn test_r2_canvas_interaction_tracks_cursor_and_requests_redraw() {
        let graph = PwGraph::default();
        let visible_nodes = HashSet::new();
        let node_positions = BTreeMap::new();
        let canvas = GraphCanvas {
            graph: &graph,
            visible_nodes,
            node_positions: &node_positions,
            selected_edge: None,
            selected_node: None,
            pan: Vector::ZERO,
            zoom: 1.0,
            layer: CanvasLayer::Base,
            search_query: "",
        };

        let bounds = Rectangle::new(Point::ORIGIN, Size::new(800.0, 600.0));
        let mut state = CanvasInteraction::default();
        assert_eq!(state.cursor_pos, None);

        let cursor_point = Point::new(150.0, 200.0);
        let cursor = mouse::Cursor::Available(cursor_point);
        let event = Event::Mouse(mouse::Event::CursorMoved {
            position: cursor_point,
        });

        // With no hovered edge, moving cursor updates cursor_pos but does not request redraw
        let action = canvas.update(&mut state, &event, bounds, cursor);
        assert_eq!(state.cursor_pos, Some(cursor_point));
        assert!(action.is_none());

        // When an edge is hovered, moving cursor updates cursor_pos AND requests redraw for smooth tooltip tracking
        state.hovered_edge = Some(crate::model::EdgeKey {
            output_node_serial: 1,
            input_node_serial: 2,
        });
        let next_cursor_point = Point::new(155.0, 205.0);
        let next_cursor = mouse::Cursor::Available(next_cursor_point);
        let next_event = Event::Mouse(mouse::Event::CursorMoved {
            position: next_cursor_point,
        });

        let action = canvas.update(&mut state, &next_event, bounds, next_cursor);
        assert_eq!(state.cursor_pos, Some(next_cursor_point));
        assert!(
            action.is_some(),
            "Must request redraw when edge is hovered during cursor move"
        );
    }

    #[test]
    fn test_r1_graph_canvas_dimming_helpers() {
        let graph = crate::model::create_demo_graph();
        let visible_nodes: HashSet<u64> = graph.nodes.keys().copied().collect();
        let node_positions = BTreeMap::new();

        // Canvas with empty search query
        let canvas_empty = GraphCanvas {
            graph: &graph,
            visible_nodes: visible_nodes.clone(),
            node_positions: &node_positions,
            selected_edge: None,
            selected_node: None,
            pan: Vector::ZERO,
            zoom: 1.0,
            layer: CanvasLayer::Base,
            search_query: "",
        };
        assert!(!canvas_empty.is_search_active());
        assert!(!canvas_empty.is_node_dimmed(30));
        assert!(!canvas_empty.is_node_dimmed(50));
        assert!(canvas_empty.dimmed_nodes().is_empty());
        assert!(canvas_empty.dimmed_links().is_empty());

        // Canvas with active search query "spotify"
        let canvas_active = GraphCanvas {
            graph: &graph,
            visible_nodes,
            node_positions: &node_positions,
            selected_edge: None,
            selected_node: None,
            pan: Vector::ZERO,
            zoom: 1.0,
            layer: CanvasLayer::Base,
            search_query: "spotify",
        };
        assert!(canvas_active.is_search_active());
        assert!(!canvas_active.is_node_dimmed(30), "Spotify (30) matches query");
        assert!(canvas_active.is_node_dimmed(50), "Built-in Audio (50) does not match");
        assert!(
            canvas_active.is_link_dimmed(&crate::model::EdgeKey {
                output_node_serial: 30,
                input_node_serial: 50,
            }),
            "Link 30 -> 50 has a dimmed endpoint (50), so it must be dimmed"
        );
    }
}
