use crate::model::{CollapsedEdge, EdgeKey, NodeCategory, PwGraph, PwNode};
use iced::mouse;
use iced::widget::canvas::{
    self, Event, Frame, Geometry, LineCap, Path, Program, Stroke, Style, Text,
};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme, Vector};
use std::collections::{BTreeMap, HashSet};

pub const NODE_WIDTH: f32 = 235.0;
pub const NODE_HEIGHT: f32 = 96.0;
pub const SOCKET_RADIUS: f32 = 8.0;

#[derive(Debug, Clone)]
pub enum CanvasMessage {
    SelectEdge(EdgeKey),
    SelectNode(u64),
    ClearSelection,
    MoveNode(u64, Point),
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

pub struct GraphCanvas<'a> {
    pub graph: &'a PwGraph,
    pub visible_nodes: HashSet<u64>,
    pub node_positions: &'a BTreeMap<u64, Point>,
    pub selected_edge: Option<&'a EdgeKey>,
    pub selected_node: Option<u64>,
    pub pan: Vector,
    pub zoom: f32,
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
        for (&serial, &pos) in self.node_positions.iter().rev() {
            if !self.visible_nodes.contains(&serial) {
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

            // Check midpoint label badge rectangle first
            let mid = bezier_point(p0, p3, 0.5);
            let badge_w = (edge.summary.len() as f32 * 6.8 + 18.0).max(86.0);
            let badge_rect = Rectangle::new(
                Point::new(mid.x - badge_w * 0.5, mid.y - 12.0),
                Size::new(badge_w, 24.0),
            );
            if badge_rect.contains(world_pt) {
                return Some(edge.key);
            }

            // Sample cubic bezier curve distance
            if distance_to_bezier(world_pt, p0, p3) <= 10.0 {
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
        event: Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<CanvasMessage>) {
        let Some(cursor_pos) = cursor.position_in(bounds) else {
            return (canvas::event::Status::Ignored, None);
        };
        let world_pt = self.screen_to_world(cursor_pos);

        match event {
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let Some((node_serial, offset)) = state.dragging_node {
                    let new_pos = Point::new(world_pt.x - offset.x, world_pt.y - offset.y);
                    return (
                        canvas::event::Status::Captured,
                        Some(CanvasMessage::MoveNode(node_serial, new_pos)),
                    );
                }

                if let Some((out_node_serial, _)) = state.wiring_from_output {
                    state.wiring_from_output = Some((out_node_serial, world_pt));
                    state.hovered_input_socket = self.hit_test_input_socket(world_pt);
                    return (canvas::event::Status::Captured, None);
                }

                if let Some(last_pan) = state.panning_from {
                    let delta = Vector::new(cursor_pos.x - last_pan.x, cursor_pos.y - last_pan.y);
                    state.panning_from = Some(cursor_pos);
                    return (
                        canvas::event::Status::Captured,
                        Some(CanvasMessage::PanCanvas(delta)),
                    );
                }

                state.hovered_output_socket = self.hit_test_output_socket(world_pt);
                state.hovered_input_socket = self.hit_test_input_socket(world_pt);
                state.hovered_edge = if state.hovered_output_socket.is_none()
                    && state.hovered_input_socket.is_none()
                {
                    self.hit_test_edge(world_pt)
                } else {
                    None
                };
                state.hovered_node = if state.hovered_edge.is_none() {
                    self.hit_test_node(world_pt)
                } else {
                    None
                };

                (canvas::event::Status::Ignored, None)
            }

            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                // 1. Check output socket drag to create a new collapsed wire
                if let Some(out_node_serial) = self.hit_test_output_socket(world_pt) {
                    state.wiring_from_output = Some((out_node_serial, world_pt));
                    return (canvas::event::Status::Captured, None);
                }

                // 2. Check collapsed wire click -> open Port Routing Matrix
                if let Some(edge_key) = self.hit_test_edge(world_pt) {
                    return (
                        canvas::event::Status::Captured,
                        Some(CanvasMessage::SelectEdge(edge_key)),
                    );
                }

                // 3. Check node click/drag
                if let Some(node_serial) = self.hit_test_node(world_pt) {
                    if let Some(&node_pos) = self.node_positions.get(&node_serial) {
                        let offset = Vector::new(world_pt.x - node_pos.x, world_pt.y - node_pos.y);
                        state.dragging_node = Some((node_serial, offset));
                    }
                    return (
                        canvas::event::Status::Captured,
                        Some(CanvasMessage::SelectNode(node_serial)),
                    );
                }

                // 4. Otherwise start panning canvas and clear selection
                state.panning_from = Some(cursor_pos);
                (
                    canvas::event::Status::Captured,
                    Some(CanvasMessage::ClearSelection),
                )
            }

            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Middle)) => {
                state.panning_from = Some(cursor_pos);
                (canvas::event::Status::Captured, None)
            }

            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                // Right-click on a collapsed wire disconnects it
                if let Some(edge_key) = self.hit_test_edge(world_pt) {
                    return (
                        canvas::event::Status::Captured,
                        Some(CanvasMessage::DisconnectNodes {
                            output_node_serial: edge_key.output_node_serial,
                            input_node_serial: edge_key.input_node_serial,
                        }),
                    );
                }
                (canvas::event::Status::Ignored, None)
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
                            return (
                                canvas::event::Status::Captured,
                                Some(CanvasMessage::ConnectNodes {
                                    output_node_serial,
                                    input_node_serial,
                                }),
                            );
                        }
                    }
                    return (canvas::event::Status::Captured, None);
                }

                state.dragging_node = None;
                state.panning_from = None;
                (canvas::event::Status::Ignored, None)
            }

            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Middle)) => {
                state.panning_from = None;
                (canvas::event::Status::Ignored, None)
            }

            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let scroll_y = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => y,
                    mouse::ScrollDelta::Pixels { y, .. } => y / 40.0,
                };
                if scroll_y.abs() > 0.01 {
                    return (
                        canvas::event::Status::Captured,
                        Some(CanvasMessage::ZoomCanvas(scroll_y, cursor_pos)),
                    );
                }
                (canvas::event::Status::Ignored, None)
            }

            _ => (canvas::event::Status::Ignored, None),
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
        let mut frame = Frame::new(renderer, bounds.size());

        // Background fill
        frame.fill_rectangle(
            Point::ORIGIN,
            bounds.size(),
            Color::from_rgb(0.08, 0.09, 0.12),
        );

        // Draw subtle grid in screen space
        self.draw_grid(&mut frame, bounds.size());

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

            self.draw_collapsed_wire(&mut frame, p0, p3, edge, is_selected, is_hovered);
        }

        // 2. Draw active wire drag preview
        if let Some((out_node_serial, cursor_world)) = state.wiring_from_output {
            if let Some(&out_pos) = self.node_positions.get(&out_node_serial) {
                let p0 = self.world_to_screen(Self::output_socket_world(out_pos));
                let p3 = self.world_to_screen(cursor_world);
                let path = bezier_path(p0, p3);
                frame.stroke(
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

        // 3. Draw nodes
        for (&node_serial, &world_pos) in self.node_positions {
            if !self.visible_nodes.contains(&node_serial) {
                continue;
            }
            if let Some(node) = self.graph.nodes.get(&node_serial) {
                let is_selected = self.selected_node == Some(node_serial);
                let is_hovered = state.hovered_node == Some(node_serial);
                let out_sock_hovered = state.hovered_output_socket == Some(node_serial)
                    || state.wiring_from_output.map(|(s, _)| s) == Some(node_serial);
                let in_sock_hovered = state.hovered_input_socket == Some(node_serial);

                self.draw_node(
                    &mut frame,
                    node,
                    world_pos,
                    is_selected,
                    is_hovered,
                    in_sock_hovered,
                    out_sock_hovered,
                );
            }
        }

        // 4. Canvas overlay hint in bottom-left
        frame.fill_text(Text {
            content: String::from(
                "Drag Output ● → Input ● to connect  |  Click any wire to inspect/edit ports  |  Right-click wire to disconnect  |  Scroll to zoom",
            ),
            position: Point::new(14.0, bounds.height - 22.0),
            color: Color::from_rgba(0.75, 0.78, 0.85, 0.65),
            size: 12.0.into(),
            ..Text::default()
        });

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
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

        let base_color = if is_selected {
            Color::from_rgb(0.35, 0.88, 1.0)
        } else if is_hovered {
            Color::from_rgb(0.55, 0.92, 0.75)
        } else if edge.summary.contains("Mono → Stereo") {
            Color::from_rgb(0.95, 0.72, 0.30)
        } else if edge.summary.contains("Swap") {
            Color::from_rgb(0.95, 0.45, 0.65)
        } else {
            Color::from_rgb(0.38, 0.75, 0.55)
        };

        // Outer glow if selected or hovered
        if is_selected || is_hovered {
            frame.stroke(
                &path,
                Stroke {
                    style: Style::Solid(Color::from_rgba(
                        base_color.r,
                        base_color.g,
                        base_color.b,
                        0.28,
                    )),
                    width: (8.5 * self.zoom).max(5.0),
                    line_cap: LineCap::Round,
                    ..Stroke::default()
                },
            );
        }

        // Main collapsed wire stroke
        frame.stroke(
            &path,
            Stroke {
                style: Style::Solid(base_color),
                width: (3.2 * self.zoom).max(2.0),
                line_cap: LineCap::Round,
                ..Stroke::default()
            },
        );

        // Midpoint topology pill badge (e.g. "Stereo → Stereo", "Mono → Stereo")
        let mid = bezier_point(p0, p3, 0.5);
        let label = format!("{} ({})", edge.summary, edge.links.len());
        let badge_w = (label.len() as f32 * 6.5 + 16.0) * self.zoom.clamp(0.8, 1.25);
        let badge_h = 22.0 * self.zoom.clamp(0.8, 1.25);
        let badge_top_left = Point::new(mid.x - badge_w * 0.5, mid.y - badge_h * 0.5);

        let pill_bg = if is_selected {
            Color::from_rgb(0.14, 0.28, 0.38)
        } else if is_hovered {
            Color::from_rgb(0.16, 0.26, 0.24)
        } else {
            Color::from_rgba(0.12, 0.14, 0.19, 0.94)
        };

        frame.fill_rectangle(badge_top_left, Size::new(badge_w, badge_h), pill_bg);
        let border_rect = Path::rectangle(badge_top_left, Size::new(badge_w, badge_h));
        frame.stroke(
            &border_rect,
            Stroke {
                style: Style::Solid(base_color),
                width: if is_selected { 2.0 } else { 1.2 },
                ..Stroke::default()
            },
        );

        frame.fill_text(Text {
            content: label,
            position: Point::new(badge_top_left.x + 8.0, mid.y - 6.5 * self.zoom.clamp(0.8, 1.2)),
            color: Color::from_rgb(0.93, 0.95, 0.98),
            size: (11.5 * self.zoom.clamp(0.8, 1.2)).into(),
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

        // Port summary line at bottom of card
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
