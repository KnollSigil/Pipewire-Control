use crate::model::{
    classify_node, create_demo_graph, infer_channel_from_name, PortDirection, PwGraph, PwLink,
    PwNode, PwPort,
};
use iced::futures::{SinkExt, Stream};
use iced::stream;
use pipewire as pw;
use pw::{
    keys,
    link::{Link, LinkListener, LinkState},
    node::{Node, NodeListener, NodeState},
    properties::properties,
    types::ObjectType,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingPreset {
    DefaultAuto,
    Stereo1To1,
    SwapLeftRight,
    MonoLeftToBoth,
    MonoRightToBoth,
    DisconnectAll,
}

/// Commands sent from the Iced GUI thread to the PipeWire `MainLoop` thread via `pipewire::channel`.
/// All GUI operations reference objects strictly by `object_serial: u64`.
#[derive(Debug, Clone)]
pub enum PwCommand {
    CreateLink {
        output_node_serial: u64,
        output_port_serial: u64,
        input_node_serial: u64,
        input_port_serial: u64,
    },
    DestroyLink {
        link_serial: u64,
    },
    Sync,
}

#[derive(Clone)]
pub struct PwCommandSender(pub pw::channel::Sender<PwCommand>);

impl std::fmt::Debug for PwCommandSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PwCommandSender(..)")
    }
}

/// Events emitted by the PipeWire `MainLoop` thread back to the Iced GUI via `iced::Subscription`.
#[derive(Debug, Clone)]
pub enum PwEvent {
    Connected(PwCommandSender),
    GraphUpdated(PwGraph),
    Status(String),
    Disconnected(String),
}

pub struct Backend {
    pub is_demo: bool,
    pub last_status: String,
    pub cmd_sender: Option<pw::channel::Sender<PwCommand>>,
    next_demo_link_serial: u64,
}

impl Default for Backend {
    fn default() -> Self {
        Self {
            is_demo: false,
            last_status: String::from("Connecting to PipeWire via pipewire-rs..."),
            cmd_sender: None,
            next_demo_link_serial: 2000,
        }
    }
}

impl Backend {
    /// Requests a roundtrip sync from the PipeWire daemon.
    pub fn request_sync(&mut self) {
        if let Some(tx) = &self.cmd_sender {
            let _ = tx.send(PwCommand::Sync);
        }
    }

    /// Creates a link between `output_port_serial` and `input_port_serial` using `pipewire-rs`.
    pub fn connect_ports(
        &mut self,
        graph: &mut PwGraph,
        output_node_serial: u64,
        output_port_serial: u64,
        input_node_serial: u64,
        input_port_serial: u64,
    ) -> Result<(), String> {
        if graph.links.values().any(|l| {
            l.output_port_serial == output_port_serial && l.input_port_serial == input_port_serial
        }) {
            return Ok(());
        }

        if let Some(tx) = &self.cmd_sender {
            tx.send(PwCommand::CreateLink {
                output_node_serial,
                output_port_serial,
                input_node_serial,
                input_port_serial,
            })
            .map_err(|_| String::from("Failed to send CreateLink to PipeWire mainloop"))?;
            self.last_status = format!(
                "Requested PipeWire link (serial {output_port_serial} → {input_port_serial})"
            );
            return Ok(());
        }

        if self.is_demo {
            let serial = self.next_demo_link_serial;
            self.next_demo_link_serial += 1;
            let id = serial as u32;
            let out_node_id = graph
                .nodes
                .get(&output_node_serial)
                .map(|n| n.id)
                .unwrap_or(output_node_serial as u32);
            let out_port_id = graph
                .ports
                .get(&output_port_serial)
                .map(|p| p.id)
                .unwrap_or(output_port_serial as u32);
            let in_node_id = graph
                .nodes
                .get(&input_node_serial)
                .map(|n| n.id)
                .unwrap_or(input_node_serial as u32);
            let in_port_id = graph
                .ports
                .get(&input_port_serial)
                .map(|p| p.id)
                .unwrap_or(input_port_serial as u32);

            graph.links.insert(
                serial,
                PwLink {
                    id,
                    object_serial: serial,
                    output_node_id: out_node_id,
                    output_node_serial,
                    output_port_id: out_port_id,
                    output_port_serial,
                    input_node_id: in_node_id,
                    input_node_serial,
                    input_port_id: in_port_id,
                    input_port_serial,
                    state: "active".to_string(),
                },
            );
            self.last_status =
                format!("Connected port serial {output_port_serial} → {input_port_serial}");
            return Ok(());
        }

        Err(String::from("PipeWire mainloop is not connected"))
    }

    /// Disconnects an existing link between `output_port_serial` and `input_port_serial` using `pipewire-rs`.
    pub fn disconnect_ports(
        &mut self,
        graph: &mut PwGraph,
        output_port_serial: u64,
        input_port_serial: u64,
    ) -> Result<(), String> {
        let matching_link_serials: Vec<u64> = graph
            .links
            .values()
            .filter(|l| {
                l.output_port_serial == output_port_serial
                    && l.input_port_serial == input_port_serial
            })
            .map(|l| l.object_serial)
            .collect();

        if matching_link_serials.is_empty() {
            return Ok(());
        }

        if let Some(tx) = &self.cmd_sender {
            for link_serial in matching_link_serials {
                tx.send(PwCommand::DestroyLink { link_serial })
                    .map_err(|_| String::from("Failed to send DestroyLink to PipeWire mainloop"))?;
            }
            self.last_status = format!(
                "Requested PipeWire unlink (serial {output_port_serial} ↛ {input_port_serial})"
            );
            return Ok(());
        }

        if self.is_demo {
            for link_serial in matching_link_serials {
                graph.links.remove(&link_serial);
            }
            self.last_status =
                format!("Disconnected port serial {output_port_serial} ↛ {input_port_serial}");
            return Ok(());
        }

        Err(String::from("PipeWire mainloop is not connected"))
    }

    /// Toggles a single port-to-port link inside the Port Routing Matrix.
    pub fn toggle_port_link(
        &mut self,
        graph: &mut PwGraph,
        output_node_serial: u64,
        output_port_serial: u64,
        input_node_serial: u64,
        input_port_serial: u64,
    ) -> Result<(), String> {
        let exists = graph.links.values().any(|l| {
            l.output_port_serial == output_port_serial && l.input_port_serial == input_port_serial
        });

        if exists {
            self.disconnect_ports(graph, output_port_serial, input_port_serial)
        } else {
            self.connect_ports(
                graph,
                output_node_serial,
                output_port_serial,
                input_node_serial,
                input_port_serial,
            )
        }
    }

    /// Connects two nodes using the smart default collapsed wire mapping
    /// (Stereo->Stereo, Mono->Stereo, Mono->Mono, etc.).
    pub fn connect_nodes_default(
        &mut self,
        graph: &mut PwGraph,
        output_node_serial: u64,
        input_node_serial: u64,
    ) -> Result<(), String> {
        if output_node_serial == input_node_serial {
            return Err("Cannot connect a node to itself".to_string());
        }

        let pairs = graph.compute_default_port_mapping(output_node_serial, input_node_serial);
        if pairs.is_empty() {
            return Err("No compatible output/input ports found between these nodes".to_string());
        }

        for (out_port_serial, in_port_serial) in pairs {
            self.connect_ports(
                graph,
                output_node_serial,
                out_port_serial,
                input_node_serial,
                in_port_serial,
            )?;
        }

        let out_title = graph
            .nodes
            .get(&output_node_serial)
            .map(|n| n.title())
            .unwrap_or_default();
        let in_title = graph
            .nodes
            .get(&input_node_serial)
            .map(|n| n.title())
            .unwrap_or_default();
        self.last_status = format!("Connected {out_title} → {in_title}");
        Ok(())
    }

    /// Disconnects all links in a collapsed wire between `output_node_serial` and `input_node_serial`.
    pub fn disconnect_nodes(
        &mut self,
        graph: &mut PwGraph,
        output_node_serial: u64,
        input_node_serial: u64,
    ) -> Result<(), String> {
        let pairs: Vec<(u64, u64)> = graph
            .links
            .values()
            .filter(|l| {
                l.output_node_serial == output_node_serial
                    && l.input_node_serial == input_node_serial
            })
            .map(|l| (l.output_port_serial, l.input_port_serial))
            .collect();

        for (op_serial, ip_serial) in pairs {
            self.disconnect_ports(graph, op_serial, ip_serial)?;
        }

        self.last_status =
            format!("Disconnected wire (serial {output_node_serial} ↛ {input_node_serial})");
        Ok(())
    }

    /// Applies a routing preset between two nodes identified by `object_serial`.
    pub fn apply_preset(
        &mut self,
        graph: &mut PwGraph,
        output_node_serial: u64,
        input_node_serial: u64,
        preset: RoutingPreset,
    ) -> Result<(), String> {
        self.disconnect_nodes(graph, output_node_serial, input_node_serial)?;

        if preset == RoutingPreset::DisconnectAll {
            return Ok(());
        }
        if preset == RoutingPreset::DefaultAuto {
            return self.connect_nodes_default(graph, output_node_serial, input_node_serial);
        }

        let (Some(out_node), Some(in_node)) = (
            graph.nodes.get(&output_node_serial).cloned(),
            graph.nodes.get(&input_node_serial).cloned(),
        ) else {
            return Err("Node not found".to_string());
        };

        let out_ports: Vec<u64> = out_node
            .default_output_ports()
            .into_iter()
            .map(|p| p.object_serial)
            .collect();
        let in_ports: Vec<u64> = in_node
            .input_ports
            .iter()
            .map(|p| p.object_serial)
            .collect();

        if out_ports.is_empty() || in_ports.is_empty() {
            return Err("Nodes do not have ports for this preset".to_string());
        }

        let mut pairs = Vec::new();
        match preset {
            RoutingPreset::Stereo1To1 => {
                for (o, i) in out_ports.iter().zip(in_ports.iter()) {
                    pairs.push((*o, *i));
                }
            }
            RoutingPreset::SwapLeftRight => {
                if out_ports.len() >= 2 && in_ports.len() >= 2 {
                    pairs.push((out_ports[0], in_ports[1]));
                    pairs.push((out_ports[1], in_ports[0]));
                } else if out_ports.len() == 1 && in_ports.len() >= 2 {
                    pairs.push((out_ports[0], in_ports[1]));
                }
            }
            RoutingPreset::MonoLeftToBoth => {
                let first_out = out_ports[0];
                for &in_p in &in_ports {
                    pairs.push((first_out, in_p));
                }
            }
            RoutingPreset::MonoRightToBoth => {
                let right_out = *out_ports.get(1).unwrap_or(&out_ports[0]);
                for &in_p in &in_ports {
                    pairs.push((right_out, in_p));
                }
            }
            RoutingPreset::DefaultAuto | RoutingPreset::DisconnectAll => {}
        }

        for (op_serial, ip_serial) in pairs {
            self.connect_ports(
                graph,
                output_node_serial,
                op_serial,
                input_node_serial,
                ip_serial,
            )?;
        }

        Ok(())
    }
}

struct PwLoopState {
    graph: PwGraph,
    link_factory: String,
    /// Maps live PipeWire global `id: u32` -> authoritative `object.serial: u64`.
    id_to_serial: HashMap<u32, u64>,
    created_links: Vec<Link>,
    bound_links: HashMap<u64, (Link, LinkListener)>,
    bound_nodes: HashMap<u64, (Node, NodeListener)>,
}

impl Default for PwLoopState {
    fn default() -> Self {
        Self {
            graph: PwGraph::default(),
            link_factory: String::from("link-factory"),
            id_to_serial: HashMap::new(),
            created_links: Vec::new(),
            bound_links: HashMap::new(),
            bound_nodes: HashMap::new(),
        }
    }
}

impl PwLoopState {
    /// Verifies that the requested output/input nodes and ports still exist in the live PipeWire
    /// registry with their exact expected `object.serial`s, returning their current `(out_node_id, out_port_id, in_node_id, in_port_id)`.
    fn verify_create_link(
        &self,
        output_node_serial: u64,
        output_port_serial: u64,
        input_node_serial: u64,
        input_port_serial: u64,
    ) -> Result<(u32, u32, u32, u32), String> {
        let Some(out_node) = self.graph.nodes.get(&output_node_serial) else {
            return Err(format!(
                "Refused to create link: output node (serial {output_node_serial}) no longer exists"
            ));
        };
        if self.id_to_serial.get(&out_node.id).copied() != Some(output_node_serial) {
            return Err(format!(
                "Refused to create link: output node #{} object.serial changed (expected {output_node_serial})",
                out_node.id
            ));
        }

        let Some(out_port) = self.graph.ports.get(&output_port_serial) else {
            return Err(format!(
                "Refused to create link: output port (serial {output_port_serial}) no longer exists"
            ));
        };
        if self.id_to_serial.get(&out_port.id).copied() != Some(output_port_serial) {
            return Err(format!(
                "Refused to create link: output port #{} object.serial changed (expected {output_port_serial})",
                out_port.id
            ));
        }
        if out_port.node_id != out_node.id {
            return Err(format!(
                "Refused to create link: output port (serial {output_port_serial}) does not belong to output node (serial {output_node_serial})"
            ));
        }

        let Some(in_node) = self.graph.nodes.get(&input_node_serial) else {
            return Err(format!(
                "Refused to create link: input node (serial {input_node_serial}) no longer exists"
            ));
        };
        if self.id_to_serial.get(&in_node.id).copied() != Some(input_node_serial) {
            return Err(format!(
                "Refused to create link: input node #{} object.serial changed (expected {input_node_serial})",
                in_node.id
            ));
        }

        let Some(in_port) = self.graph.ports.get(&input_port_serial) else {
            return Err(format!(
                "Refused to create link: input port (serial {input_port_serial}) no longer exists"
            ));
        };
        if self.id_to_serial.get(&in_port.id).copied() != Some(input_port_serial) {
            return Err(format!(
                "Refused to create link: input port #{} object.serial changed (expected {input_port_serial})",
                in_port.id
            ));
        }
        if in_port.node_id != in_node.id {
            return Err(format!(
                "Refused to create link: input port (serial {input_port_serial}) does not belong to input node (serial {input_node_serial})"
            ));
        }

        Ok((out_node.id, out_port.id, in_node.id, in_port.id))
    }

    /// Verifies that the link identified by `link_serial` still exists in the live PipeWire
    /// registry and that its current global `id` still maps to `link_serial`, returning the verified `link_id: u32`.
    fn verify_destroy_link(&self, link_serial: u64) -> Result<u32, String> {
        let Some(live_link) = self.graph.links.get(&link_serial) else {
            return Err(format!(
                "Skipped destroying link (serial {link_serial}): link was already removed"
            ));
        };

        let live_serial = self.id_to_serial.get(&live_link.id).copied();
        if live_serial == Some(link_serial) {
            Ok(live_link.id)
        } else {
            Err(format!(
                "Refused to destroy global #{}: object.serial mismatch (expected {link_serial}, live is {:?})",
                live_link.id, live_serial
            ))
        }
    }
}

/// Iced `Subscription` stream that spawns the dedicated `pipewire-rs` `MainLoop` thread
/// via `tokio::task::spawn_blocking` and forwards real-time PipeWire registry events to the UI
/// using `tokio::sync::mpsc` (`blocking_send` -> async `recv`).
pub fn pipewire_subscription() -> impl Stream<Item = PwEvent> {
    stream::channel(128, |mut output| async move {
        let (tx, mut rx) = mpsc::channel::<PwEvent>(128);

        let join_handle = tokio::task::spawn_blocking(move || run_pipewire_loop(tx));

        while let Some(event) = rx.recv().await {
            let _ = output.send(event).await;
        }

        match join_handle.await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                let _ = output.send(PwEvent::Disconnected(err)).await;
                let _ = output.send(PwEvent::GraphUpdated(create_demo_graph())).await;
            }
            Err(join_err) => {
                let _ = output
                    .send(PwEvent::Disconnected(format!(
                        "PipeWire task terminated unexpectedly: {join_err}"
                    )))
                    .await;
                let _ = output.send(PwEvent::GraphUpdated(create_demo_graph())).await;
            }
        }

        // Keep the subscription stream alive for the lifetime of the application
        std::future::pending::<()>().await;
    })
}

fn run_pipewire_loop(gui_tx: mpsc::Sender<PwEvent>) -> Result<(), String> {
    pw::init();

    let mainloop = pw::main_loop::MainLoopRc::new(None)
        .map_err(|e| format!("Failed to create PipeWire MainLoop: {e}"))?;
    let context = pw::context::ContextRc::new(&mainloop, None)
        .map_err(|e| format!("Failed to create PipeWire Context: {e}"))?;
    let core = context
        .connect_rc(None)
        .map_err(|e| format!("Failed to connect to PipeWire daemon: {e}"))?;
    let registry = core
        .get_registry_rc()
        .map_err(|e| format!("Failed to get PipeWire Registry: {e}"))?;

    let (cmd_tx, cmd_rx) = pw::channel::channel::<PwCommand>();
    let _ = gui_tx.blocking_send(PwEvent::Connected(PwCommandSender(cmd_tx)));

    let state = Rc::new(RefCell::new(PwLoopState::default()));
    let pending_sync = Rc::new(Cell::new(None));

    // Helper closure to schedule a coalesced `core.sync(0)` roundtrip
    let trigger_sync = {
        let core = core.clone();
        let pending_sync = pending_sync.clone();
        move || {
            if let Ok(seq) = core.sync(0) {
                pending_sync.set(Some(seq));
            }
        }
    };

    // Listen for `core.done` events so we emit a single coalesced `PwGraph` snapshot
    // after initial registry enumeration or after any batch of graph changes.
    let _core_listener = {
        let state = state.clone();
        let pending_sync = pending_sync.clone();
        let gui_tx_done = gui_tx.clone();
        let gui_tx_err = gui_tx.clone();

        core.add_listener_local()
            .done(move |id, seq| {
                if id == pw::core::PW_ID_CORE && Some(seq) == pending_sync.get() {
                    pending_sync.set(None);
                    let mut st = state.borrow_mut();
                    st.graph.resolve_serials_and_rebuild_ports();
                    let _ = gui_tx_done.blocking_send(PwEvent::GraphUpdated(st.graph.clone()));
                }
            })
            .error(move |id, _seq, res, message| {
                let _ = gui_tx_err.blocking_send(PwEvent::Status(format!(
                    "PipeWire error (id {id}, code {res}): {message}"
                )));
            })
            .register()
    };

    // Listen for PipeWire Registry global additions and removals
    let _registry_listener = {
        let state_global = state.clone();
        let state_remove = state.clone();
        let registry_weak = registry.downgrade();
        let trigger_sync_global = trigger_sync.clone();
        let trigger_sync_remove = trigger_sync.clone();

        registry
            .add_listener_local()
            .global(move |global| {
                let Some(props) = global.props.as_ref() else {
                    return;
                };

                if global.type_ == ObjectType::Factory {
                    if props.get(*keys::FACTORY_TYPE_NAME) == Some(ObjectType::Link.to_str()) {
                        if let Some(factory_name) = props.get(*keys::FACTORY_NAME) {
                            state_global.borrow_mut().link_factory = factory_name.to_string();
                        }
                    }
                    return;
                }

                let Some(object_serial) = props
                    .get(*keys::OBJECT_SERIAL)
                    .and_then(|s| s.parse::<u64>().ok())
                else {
                    return;
                };

                state_global
                    .borrow_mut()
                    .id_to_serial
                    .insert(global.id, object_serial);

                match global.type_ {
                    ObjectType::Node => {
                        let name = props.get(*keys::NODE_NAME).unwrap_or_default().to_string();
                        let description = props
                            .get(*keys::NODE_DESCRIPTION)
                            .or_else(|| props.get(*keys::NODE_NICK))
                            .unwrap_or_default()
                            .to_string();
                        let app_name = props.get(*keys::APP_NAME).map(str::to_string);
                        let media_name = props.get(*keys::MEDIA_NAME).map(str::to_string);
                        let media_class = props
                            .get(*keys::MEDIA_CLASS)
                            .unwrap_or_default()
                            .to_string();
                        let category = classify_node(&media_class, &name);

                        let node_id = global.id;
                        {
                            let mut st = state_global.borrow_mut();
                            st.graph.nodes.insert(
                                object_serial,
                                PwNode {
                                    id: node_id,
                                    object_serial,
                                    name,
                                    description,
                                    app_name,
                                    media_name,
                                    media_class,
                                    category,
                                    state: String::from("idle"),
                                    input_ports: Vec::new(),
                                    output_ports: Vec::new(),
                                },
                            );
                        }

                        // Bind the Node proxy to track live state changes (Running, Idle, Suspended)
                        if let Some(reg) = registry_weak.upgrade() {
                            if let Ok(node_proxy) = reg.bind::<Node, _>(global) {
                                let state_info = state_global.clone();
                                let sync_info = trigger_sync_global.clone();
                                let listener = node_proxy
                                    .add_listener_local()
                                    .info(move |info| {
                                        let new_state = format_node_state(info.state());
                                        let mut changed = false;
                                        if let Some(node_entry) = state_info
                                            .borrow_mut()
                                            .graph
                                            .nodes
                                            .get_mut(&object_serial)
                                        {
                                            if node_entry.state != new_state {
                                                node_entry.state = new_state;
                                                changed = true;
                                            }
                                        }
                                        if changed {
                                            sync_info();
                                        }
                                    })
                                    .register();
                                state_global
                                    .borrow_mut()
                                    .bound_nodes
                                    .insert(object_serial, (node_proxy, listener));
                            }
                        }

                        trigger_sync_global();
                    }
                    ObjectType::Port => {
                        let Some(node_id) = props
                            .get(*keys::NODE_ID)
                            .and_then(|s| s.parse::<u32>().ok())
                        else {
                            return;
                        };
                        let name = props.get(*keys::PORT_NAME).unwrap_or_default().to_string();
                        let direction = if props.get(*keys::PORT_DIRECTION) == Some("in") {
                            PortDirection::Input
                        } else {
                            PortDirection::Output
                        };
                        let channel = props
                            .get(*keys::AUDIO_CHANNEL)
                            .map(str::to_string)
                            .unwrap_or_else(|| infer_channel_from_name(&name));
                        let is_monitor = props
                            .get(*keys::PORT_MONITOR)
                            .map(|s| s == "true" || s == "1")
                            .unwrap_or_else(|| name.starts_with("monitor"));

                        let mut st = state_global.borrow_mut();
                        let node_serial = st.id_to_serial.get(&node_id).copied().unwrap_or(0);
                        st.graph.ports.insert(
                            object_serial,
                            PwPort {
                                id: global.id,
                                object_serial,
                                node_id,
                                node_serial,
                                name,
                                channel,
                                direction,
                                is_monitor,
                            },
                        );
                        drop(st);

                        trigger_sync_global();
                    }
                    ObjectType::Link => {
                        let out_node = props
                            .get(*keys::LINK_OUTPUT_NODE)
                            .and_then(|s| s.parse::<u32>().ok());
                        let out_port = props
                            .get(*keys::LINK_OUTPUT_PORT)
                            .and_then(|s| s.parse::<u32>().ok());
                        let in_node = props
                            .get(*keys::LINK_INPUT_NODE)
                            .and_then(|s| s.parse::<u32>().ok());
                        let in_port = props
                            .get(*keys::LINK_INPUT_PORT)
                            .and_then(|s| s.parse::<u32>().ok());

                        let link_id = global.id;
                        if let (Some(on), Some(op), Some(inn), Some(ip)) =
                            (out_node, out_port, in_node, in_port)
                        {
                            let mut st = state_global.borrow_mut();
                            let output_node_serial = st.id_to_serial.get(&on).copied().unwrap_or(0);
                            let output_port_serial = st.id_to_serial.get(&op).copied().unwrap_or(0);
                            let input_node_serial = st.id_to_serial.get(&inn).copied().unwrap_or(0);
                            let input_port_serial = st.id_to_serial.get(&ip).copied().unwrap_or(0);

                            st.graph.links.insert(
                                object_serial,
                                PwLink {
                                    id: link_id,
                                    object_serial,
                                    output_node_id: on,
                                    output_node_serial,
                                    output_port_id: op,
                                    output_port_serial,
                                    input_node_id: inn,
                                    input_node_serial,
                                    input_port_id: ip,
                                    input_port_serial,
                                    state: String::from("active"),
                                },
                            );
                        }

                        // Bind the Link proxy to receive authoritative LinkInfoRef updates (including state & serial)
                        if let Some(reg) = registry_weak.upgrade() {
                            if let Ok(link_proxy) = reg.bind::<Link, _>(global) {
                                let state_info = state_global.clone();
                                let sync_info = trigger_sync_global.clone();
                                let listener = link_proxy
                                    .add_listener_local()
                                    .info(move |info| {
                                        let state_str = format_link_state(info.state());
                                        let info_serial = info
                                            .props()
                                            .and_then(|p| p.get(*keys::OBJECT_SERIAL))
                                            .and_then(|s| s.parse::<u64>().ok())
                                            .unwrap_or(object_serial);

                                        let mut st = state_info.borrow_mut();
                                        st.id_to_serial.insert(link_id, info_serial);

                                        let on = info.output_node_id();
                                        let op = info.output_port_id();
                                        let inn = info.input_node_id();
                                        let ip = info.input_port_id();
                                        let output_node_serial =
                                            st.id_to_serial.get(&on).copied().unwrap_or(0);
                                        let output_port_serial =
                                            st.id_to_serial.get(&op).copied().unwrap_or(0);
                                        let input_node_serial =
                                            st.id_to_serial.get(&inn).copied().unwrap_or(0);
                                        let input_port_serial =
                                            st.id_to_serial.get(&ip).copied().unwrap_or(0);

                                        st.graph.links.insert(
                                            info_serial,
                                            PwLink {
                                                id: link_id,
                                                object_serial: info_serial,
                                                output_node_id: on,
                                                output_node_serial,
                                                output_port_id: op,
                                                output_port_serial,
                                                input_node_id: inn,
                                                input_node_serial,
                                                input_port_id: ip,
                                                input_port_serial,
                                                state: state_str,
                                            },
                                        );
                                        drop(st);
                                        sync_info();
                                    })
                                    .register();
                                state_global
                                    .borrow_mut()
                                    .bound_links
                                    .insert(object_serial, (link_proxy, listener));
                            }
                        }

                        trigger_sync_global();
                    }
                    _ => {}
                }
            })
            .global_remove(move |id| {
                let mut st = state_remove.borrow_mut();
                let Some(serial) = st.id_to_serial.remove(&id) else {
                    return;
                };
                let mut changed = false;
                if st.graph.nodes.remove(&serial).is_some() {
                    st.bound_nodes.remove(&serial);
                    changed = true;
                }
                if st.graph.ports.remove(&serial).is_some() {
                    changed = true;
                }
                if st.graph.links.remove(&serial).is_some() {
                    st.bound_links.remove(&serial);
                    changed = true;
                }
                drop(st);
                if changed {
                    trigger_sync_remove();
                }
            })
            .register()
    };

    // Attach the GUI command channel receiver to the PipeWire MainLoop
    let _attached_cmd_rx = {
        let core = core.clone();
        let registry = registry.clone();
        let state = state.clone();
        let trigger_sync = trigger_sync.clone();
        let gui_tx_cmd = gui_tx.clone();

        cmd_rx.attach(mainloop.loop_(), move |cmd| match cmd {
            PwCommand::CreateLink {
                output_node_serial,
                output_port_serial,
                input_node_serial,
                input_port_serial,
            } => {
                // Verify that both nodes and both ports still exist with their expected `object.serial`s
                // before creating a PipeWire link.
                let verification = state.borrow().verify_create_link(
                    output_node_serial,
                    output_port_serial,
                    input_node_serial,
                    input_port_serial,
                );

                match verification {
                    Ok((output_node_id, output_port_id, input_node_id, input_port_id)) => {
                        let factory_name = state.borrow().link_factory.clone();
                        let out_node_s = output_node_id.to_string();
                        let out_port_s = output_port_id.to_string();
                        let in_node_s = input_node_id.to_string();
                        let in_port_s = input_port_id.to_string();

                        let props = properties! {
                            *keys::LINK_OUTPUT_NODE => out_node_s.as_str(),
                            *keys::LINK_OUTPUT_PORT => out_port_s.as_str(),
                            *keys::LINK_INPUT_NODE => in_node_s.as_str(),
                            *keys::LINK_INPUT_PORT => in_port_s.as_str(),
                            *keys::OBJECT_LINGER => "1"
                        };

                        match core.create_object::<Link>(&factory_name, &props) {
                            Ok(link_proxy) => {
                                state.borrow_mut().created_links.push(link_proxy);
                                let _ = gui_tx_cmd.blocking_send(PwEvent::Status(format!(
                                    "Created PipeWire link: port serial {output_port_serial} (#{output_port_id}) → {input_port_serial} (#{input_port_id})"
                                )));
                                trigger_sync();
                            }
                            Err(err) => {
                                let _ = gui_tx_cmd.blocking_send(PwEvent::Status(format!(
                                    "Failed to create PipeWire link: {err}"
                                )));
                            }
                        }
                    }
                    Err(reason) => {
                        let _ = gui_tx_cmd.blocking_send(PwEvent::Status(reason));
                        trigger_sync();
                    }
                }
            }
            PwCommand::DestroyLink { link_serial } => {
                // Verify that the link with `link_serial` still exists and its live global ID
                // still maps to `link_serial` before calling `registry.destroy_global(link_id)`.
                let verification = state.borrow().verify_destroy_link(link_serial);
                match verification {
                    Ok(link_id) => {
                        let res = registry.destroy_global(link_id).into_result();
                        {
                            let mut st = state.borrow_mut();
                            st.id_to_serial.remove(&link_id);
                            st.bound_links.remove(&link_serial);
                            st.graph.links.remove(&link_serial);
                        }
                        match res {
                            Ok(_) => {
                                let _ = gui_tx_cmd.blocking_send(PwEvent::Status(format!(
                                    "Destroyed PipeWire link serial {link_serial} (global #{link_id})"
                                )));
                            }
                            Err(err) => {
                                let _ = gui_tx_cmd.blocking_send(PwEvent::Status(format!(
                                    "Destroy link serial {link_serial} (global #{link_id}) returned: {err}"
                                )));
                            }
                        }
                    }
                    Err(reason) => {
                        let _ = gui_tx_cmd.blocking_send(PwEvent::Status(reason));
                    }
                }
                trigger_sync();
            }
            PwCommand::Sync => {
                trigger_sync();
            }
        })
    };

    // Perform initial sync roundtrip so the initial graph is delivered
    trigger_sync();

    mainloop.run();
    Ok(())
}

fn format_link_state(state: LinkState<'_>) -> String {
    match state {
        LinkState::Error(e) => format!("error ({e})"),
        LinkState::Unlinked => String::from("unlinked"),
        LinkState::Init => String::from("init"),
        LinkState::Negotiating => String::from("negotiating"),
        LinkState::Allocating => String::from("allocating"),
        LinkState::Paused => String::from("paused"),
        LinkState::Active => String::from("active"),
    }
}

fn format_node_state(state: NodeState<'_>) -> String {
    match state {
        NodeState::Error(e) => format!("error ({e})"),
        NodeState::Creating => String::from("creating"),
        NodeState::Suspended => String::from("suspended"),
        NodeState::Idle => String::from("idle"),
        NodeState::Running => String::from("running"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NodeCategory;

    #[test]
    fn test_verify_destroy_link_checks_object_serial() {
        let mut st = PwLoopState::default();
        st.id_to_serial.insert(77, 1042);
        st.graph.links.insert(
            1042,
            PwLink {
                id: 77,
                object_serial: 1042,
                output_node_id: 10,
                output_node_serial: 100,
                output_port_id: 11,
                output_port_serial: 110,
                input_node_id: 20,
                input_node_serial: 200,
                input_port_id: 21,
                input_port_serial: 210,
                state: String::from("active"),
            },
        );

        // Matching object_serial succeeds and returns the live global ID (77)
        assert_eq!(st.verify_destroy_link(1042), Ok(77));

        // Simulate global ID #77 being reused by a new object with serial 2099 before stale command arrives
        st.id_to_serial.insert(77, 2099);
        assert!(st.verify_destroy_link(1042).is_err());

        // Non-existent serial is also rejected
        assert!(st.verify_destroy_link(9999).is_err());
    }

    #[test]
    fn test_verify_create_link_checks_node_and_port_serials() {
        let mut st = PwLoopState::default();

        // Register output node (id 10, serial 100) and port (id 11, serial 110)
        st.id_to_serial.insert(10, 100);
        st.id_to_serial.insert(11, 110);
        st.graph.nodes.insert(
            100,
            PwNode {
                id: 10,
                object_serial: 100,
                name: "out_node".into(),
                description: "Out".into(),
                app_name: None,
                media_name: None,
                media_class: "Audio/Source".into(),
                category: NodeCategory::AudioSource,
                state: "running".into(),
                input_ports: vec![],
                output_ports: vec![],
            },
        );
        st.graph.ports.insert(
            110,
            PwPort {
                id: 11,
                object_serial: 110,
                node_id: 10,
                node_serial: 100,
                name: "out_FL".into(),
                channel: "FL".into(),
                direction: PortDirection::Output,
                is_monitor: false,
            },
        );

        // Register input node (id 20, serial 200) and port (id 21, serial 210)
        st.id_to_serial.insert(20, 200);
        st.id_to_serial.insert(21, 210);
        st.graph.nodes.insert(
            200,
            PwNode {
                id: 20,
                object_serial: 200,
                name: "in_node".into(),
                description: "In".into(),
                app_name: None,
                media_name: None,
                media_class: "Audio/Sink".into(),
                category: NodeCategory::AudioSink,
                state: "running".into(),
                input_ports: vec![],
                output_ports: vec![],
            },
        );
        st.graph.ports.insert(
            210,
            PwPort {
                id: 21,
                object_serial: 210,
                node_id: 20,
                node_serial: 200,
                name: "in_FL".into(),
                channel: "FL".into(),
                direction: PortDirection::Input,
                is_monitor: false,
            },
        );

        // Valid serials resolve to live global IDs (10, 11, 20, 21)
        assert_eq!(st.verify_create_link(100, 110, 200, 210), Ok((10, 11, 20, 21)));

        // If port global #21 is replaced by a new object with serial 999, verification rejects link creation
        st.id_to_serial.insert(21, 999);
        assert!(st.verify_create_link(100, 110, 200, 210).is_err());
    }
}
