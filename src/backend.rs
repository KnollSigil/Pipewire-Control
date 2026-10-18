use crate::model::{
    classify_node, create_demo_graph, infer_channel_from_name, PortDirection, PwGraph, PwLink,
    PwNode, PwPort,
};
use iced::futures::channel::mpsc;
use iced::futures::Stream;
use iced::stream;
use pipewire as pw;
use pw::{
    link::{Link, LinkListener, LinkState},
    node::{Node, NodeListener, NodeState},
    properties::properties,
    types::ObjectType,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    thread,
};

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
#[derive(Debug, Clone)]
pub enum PwCommand {
    CreateLink {
        output_node_id: u32,
        output_port_id: u32,
        input_node_id: u32,
        input_port_id: u32,
    },
    DestroyLink {
        link_id: u32,
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
    next_demo_link_id: u32,
}

impl Default for Backend {
    fn default() -> Self {
        Self {
            is_demo: false,
            last_status: String::from("Connecting to PipeWire via pipewire-rs..."),
            cmd_sender: None,
            next_demo_link_id: 2000,
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

    /// Creates a link between `output_port_id` and `input_port_id` using `pipewire-rs`.
    pub fn connect_ports(
        &mut self,
        graph: &mut PwGraph,
        output_node_id: u32,
        output_port_id: u32,
        input_node_id: u32,
        input_port_id: u32,
    ) -> Result<(), String> {
        if graph.links.values().any(|l| {
            l.output_port_id == output_port_id && l.input_port_id == input_port_id
        }) {
            return Ok(());
        }

        if let Some(tx) = &self.cmd_sender {
            tx.send(PwCommand::CreateLink {
                output_node_id,
                output_port_id,
                input_node_id,
                input_port_id,
            })
            .map_err(|_| String::from("Failed to send CreateLink to PipeWire mainloop"))?;
            self.last_status =
                format!("Requested PipeWire link #{output_port_id} → #{input_port_id}");
            return Ok(());
        }

        if self.is_demo {
            let id = self.next_demo_link_id;
            self.next_demo_link_id += 1;
            graph.links.insert(
                id,
                PwLink {
                    id,
                    output_node_id,
                    output_port_id,
                    input_node_id,
                    input_port_id,
                    state: "active".to_string(),
                },
            );
            self.last_status = format!("Connected port #{output_port_id} → #{input_port_id}");
            return Ok(());
        }

        Err(String::from("PipeWire mainloop is not connected"))
    }

    /// Disconnects an existing link between `output_port_id` and `input_port_id` using `pipewire-rs`.
    pub fn disconnect_ports(
        &mut self,
        graph: &mut PwGraph,
        output_port_id: u32,
        input_port_id: u32,
    ) -> Result<(), String> {
        let matching_ids: Vec<u32> = graph
            .links
            .values()
            .filter(|l| l.output_port_id == output_port_id && l.input_port_id == input_port_id)
            .map(|l| l.id)
            .collect();

        if matching_ids.is_empty() {
            return Ok(());
        }

        if let Some(tx) = &self.cmd_sender {
            for link_id in matching_ids {
                tx.send(PwCommand::DestroyLink { link_id })
                    .map_err(|_| String::from("Failed to send DestroyLink to PipeWire mainloop"))?;
            }
            self.last_status =
                format!("Requested PipeWire unlink #{output_port_id} ↛ #{input_port_id}");
            return Ok(());
        }

        if self.is_demo {
            for id in matching_ids {
                graph.links.remove(&id);
            }
            self.last_status = format!("Disconnected port #{output_port_id} ↛ #{input_port_id}");
            return Ok(());
        }

        Err(String::from("PipeWire mainloop is not connected"))
    }

    /// Toggles a single port-to-port link inside the Port Routing Matrix.
    pub fn toggle_port_link(
        &mut self,
        graph: &mut PwGraph,
        output_node_id: u32,
        output_port_id: u32,
        input_node_id: u32,
        input_port_id: u32,
    ) -> Result<(), String> {
        let exists = graph
            .links
            .values()
            .any(|l| l.output_port_id == output_port_id && l.input_port_id == input_port_id);

        if exists {
            self.disconnect_ports(graph, output_port_id, input_port_id)
        } else {
            self.connect_ports(
                graph,
                output_node_id,
                output_port_id,
                input_node_id,
                input_port_id,
            )
        }
    }

    /// Connects two nodes using the smart default collapsed wire mapping
    /// (Stereo->Stereo, Mono->Stereo, Mono->Mono, etc.).
    pub fn connect_nodes_default(
        &mut self,
        graph: &mut PwGraph,
        output_node_id: u32,
        input_node_id: u32,
    ) -> Result<(), String> {
        if output_node_id == input_node_id {
            return Err("Cannot connect a node to itself".to_string());
        }

        let pairs = graph.compute_default_port_mapping(output_node_id, input_node_id);
        if pairs.is_empty() {
            return Err("No compatible output/input ports found between these nodes".to_string());
        }

        for (out_port, in_port) in pairs {
            self.connect_ports(graph, output_node_id, out_port, input_node_id, in_port)?;
        }

        let out_title = graph
            .nodes
            .get(&output_node_id)
            .map(|n| n.title())
            .unwrap_or_default();
        let in_title = graph
            .nodes
            .get(&input_node_id)
            .map(|n| n.title())
            .unwrap_or_default();
        self.last_status = format!("Connected {out_title} → {in_title}");
        Ok(())
    }

    /// Disconnects all links in a collapsed wire between `output_node_id` and `input_node_id`.
    pub fn disconnect_nodes(
        &mut self,
        graph: &mut PwGraph,
        output_node_id: u32,
        input_node_id: u32,
    ) -> Result<(), String> {
        let pairs: Vec<(u32, u32)> = graph
            .links
            .values()
            .filter(|l| l.output_node_id == output_node_id && l.input_node_id == input_node_id)
            .map(|l| (l.output_port_id, l.input_port_id))
            .collect();

        for (op, ip) in pairs {
            self.disconnect_ports(graph, op, ip)?;
        }

        self.last_status = format!("Disconnected wire #{output_node_id} ↛ #{input_node_id}");
        Ok(())
    }

    /// Applies a routing preset between two nodes.
    pub fn apply_preset(
        &mut self,
        graph: &mut PwGraph,
        output_node_id: u32,
        input_node_id: u32,
        preset: RoutingPreset,
    ) -> Result<(), String> {
        self.disconnect_nodes(graph, output_node_id, input_node_id)?;

        if preset == RoutingPreset::DisconnectAll {
            return Ok(());
        }
        if preset == RoutingPreset::DefaultAuto {
            return self.connect_nodes_default(graph, output_node_id, input_node_id);
        }

        let (Some(out_node), Some(in_node)) = (
            graph.nodes.get(&output_node_id).cloned(),
            graph.nodes.get(&input_node_id).cloned(),
        ) else {
            return Err("Node not found".to_string());
        };

        let out_ports: Vec<u32> = out_node
            .default_output_ports()
            .into_iter()
            .map(|p| p.id)
            .collect();
        let in_ports: Vec<u32> = in_node.input_ports.iter().map(|p| p.id).collect();

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

        for (op, ip) in pairs {
            self.connect_ports(graph, output_node_id, op, input_node_id, ip)?;
        }

        Ok(())
    }
}

struct PwLoopState {
    graph: PwGraph,
    link_factory: String,
    created_links: Vec<Link>,
    bound_links: HashMap<u32, (Link, LinkListener)>,
    bound_nodes: HashMap<u32, (Node, NodeListener)>,
}

impl Default for PwLoopState {
    fn default() -> Self {
        Self {
            graph: PwGraph::default(),
            link_factory: String::from("link-factory"),
            created_links: Vec::new(),
            bound_links: HashMap::new(),
            bound_nodes: HashMap::new(),
        }
    }
}

/// Iced `Subscription` stream that spawns the dedicated `pipewire-rs` `MainLoop` thread
/// and forwards real-time PipeWire registry events to the UI without polling.
pub fn pipewire_subscription() -> impl Stream<Item = PwEvent> {
    stream::channel(128, |output| async move {
        thread::spawn(move || {
            if let Err(err) = run_pipewire_loop(output.clone()) {
                let mut out = output;
                let _ = out.try_send(PwEvent::Disconnected(err));
                let _ = out.try_send(PwEvent::GraphUpdated(create_demo_graph()));
            }
        });

        // Keep the subscription stream alive for the lifetime of the application
        std::future::pending::<()>().await;
    })
}

fn run_pipewire_loop(mut gui_tx: mpsc::Sender<PwEvent>) -> Result<(), String> {
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
    let _ = gui_tx.try_send(PwEvent::Connected(PwCommandSender(cmd_tx)));

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
                    st.graph.rebuild_node_ports();
                    let _ = gui_tx_done
                        .clone()
                        .try_send(PwEvent::GraphUpdated(st.graph.clone()));
                }
            })
            .error(move |id, _seq, res, message| {
                let _ = gui_tx_err.clone().try_send(PwEvent::Status(format!(
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

                match global.type_ {
                    ObjectType::Factory => {
                        if props.get("factory.type.name") == Some(ObjectType::Link.to_str()) {
                            if let Some(factory_name) = props.get("factory.name") {
                                state_global.borrow_mut().link_factory = factory_name.to_string();
                            }
                        }
                    }
                    ObjectType::Node => {
                        let name = props.get("node.name").unwrap_or_default().to_string();
                        let description = props
                            .get("node.description")
                            .or_else(|| props.get("node.nick"))
                            .unwrap_or_default()
                            .to_string();
                        let app_name = props.get("application.name").map(str::to_string);
                        let media_name = props.get("media.name").map(str::to_string);
                        let media_class = props.get("media.class").unwrap_or_default().to_string();
                        let category = classify_node(&media_class, &name);

                        let node_id = global.id;
                        {
                            let mut st = state_global.borrow_mut();
                            st.graph.nodes.insert(
                                node_id,
                                PwNode {
                                    id: node_id,
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
                                        if let Some(node_entry) =
                                            state_info.borrow_mut().graph.nodes.get_mut(&node_id)
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
                                    .insert(node_id, (node_proxy, listener));
                            }
                        }

                        trigger_sync_global();
                    }
                    ObjectType::Port => {
                        let Some(node_id) =
                            props.get("node.id").and_then(|s| s.parse::<u32>().ok())
                        else {
                            return;
                        };
                        let name = props.get("port.name").unwrap_or_default().to_string();
                        let direction = if props.get("port.direction") == Some("in") {
                            PortDirection::Input
                        } else {
                            PortDirection::Output
                        };
                        let channel = props
                            .get("audio.channel")
                            .map(str::to_string)
                            .unwrap_or_else(|| infer_channel_from_name(&name));
                        let is_monitor = props
                            .get("port.monitor")
                            .map(|s| s == "true" || s == "1")
                            .unwrap_or_else(|| name.starts_with("monitor"));

                        state_global.borrow_mut().graph.ports.insert(
                            global.id,
                            PwPort {
                                id: global.id,
                                node_id,
                                name,
                                channel,
                                direction,
                                is_monitor,
                            },
                        );

                        trigger_sync_global();
                    }
                    ObjectType::Link => {
                        let out_node = props
                            .get("link.output.node")
                            .and_then(|s| s.parse::<u32>().ok());
                        let out_port = props
                            .get("link.output.port")
                            .and_then(|s| s.parse::<u32>().ok());
                        let in_node = props
                            .get("link.input.node")
                            .and_then(|s| s.parse::<u32>().ok());
                        let in_port = props
                            .get("link.input.port")
                            .and_then(|s| s.parse::<u32>().ok());

                        let link_id = global.id;
                        if let (Some(on), Some(op), Some(inn), Some(ip)) =
                            (out_node, out_port, in_node, in_port)
                        {
                            state_global.borrow_mut().graph.links.insert(
                                link_id,
                                PwLink {
                                    id: link_id,
                                    output_node_id: on,
                                    output_port_id: op,
                                    input_node_id: inn,
                                    input_port_id: ip,
                                    state: String::from("active"),
                                },
                            );
                        }

                        // Bind the Link proxy to receive authoritative LinkInfoRef updates (including state)
                        if let Some(reg) = registry_weak.upgrade() {
                            if let Ok(link_proxy) = reg.bind::<Link, _>(global) {
                                let state_info = state_global.clone();
                                let sync_info = trigger_sync_global.clone();
                                let listener = link_proxy
                                    .add_listener_local()
                                    .info(move |info| {
                                        let state_str = format_link_state(info.state());
                                        let mut st = state_info.borrow_mut();
                                        st.graph.links.insert(
                                            link_id,
                                            PwLink {
                                                id: link_id,
                                                output_node_id: info.output_node_id(),
                                                output_port_id: info.output_port_id(),
                                                input_node_id: info.input_node_id(),
                                                input_port_id: info.input_port_id(),
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
                                    .insert(link_id, (link_proxy, listener));
                            }
                        }

                        trigger_sync_global();
                    }
                    _ => {}
                }
            })
            .global_remove(move |id| {
                let mut st = state_remove.borrow_mut();
                let mut changed = false;
                if st.graph.nodes.remove(&id).is_some() {
                    st.bound_nodes.remove(&id);
                    changed = true;
                }
                if st.graph.ports.remove(&id).is_some() {
                    changed = true;
                }
                if st.graph.links.remove(&id).is_some() {
                    st.bound_links.remove(&id);
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
                output_node_id,
                output_port_id,
                input_node_id,
                input_port_id,
            } => {
                let factory_name = state.borrow().link_factory.clone();
                let out_node_s = output_node_id.to_string();
                let out_port_s = output_port_id.to_string();
                let in_node_s = input_node_id.to_string();
                let in_port_s = input_port_id.to_string();

                let props = properties! {
                    "link.output.node" => out_node_s.as_str(),
                    "link.output.port" => out_port_s.as_str(),
                    "link.input.node" => in_node_s.as_str(),
                    "link.input.port" => in_port_s.as_str(),
                    "object.linger" => "1"
                };

                match core.create_object::<Link>(&factory_name, &props) {
                    Ok(link_proxy) => {
                        state.borrow_mut().created_links.push(link_proxy);
                        let _ = gui_tx_cmd.clone().try_send(PwEvent::Status(format!(
                            "Created PipeWire link: port #{output_port_id} → #{input_port_id}"
                        )));
                        trigger_sync();
                    }
                    Err(err) => {
                        let _ = gui_tx_cmd.clone().try_send(PwEvent::Status(format!(
                            "Failed to create PipeWire link: {err}"
                        )));
                    }
                }
            }
            PwCommand::DestroyLink { link_id } => {
                let res = registry.destroy_global(link_id).into_result();
                {
                    let mut st = state.borrow_mut();
                    st.bound_links.remove(&link_id);
                    st.graph.links.remove(&link_id);
                }
                match res {
                    Ok(_) => {
                        let _ = gui_tx_cmd.clone().try_send(PwEvent::Status(format!(
                            "Destroyed PipeWire link #{link_id}"
                        )));
                    }
                    Err(err) => {
                        let _ = gui_tx_cmd.clone().try_send(PwEvent::Status(format!(
                            "Destroy link #{link_id} returned: {err}"
                        )));
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
