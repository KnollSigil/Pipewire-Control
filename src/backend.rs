use crate::model::{
    classify_node, create_demo_graph, infer_channel_from_name, PortDirection, PwAvailability,
    PwDevice, PwDeviceProfile, PwDeviceRoute, PwGraph, PwLink, PwNode, PwNodeParam, PwParamValue,
    PwPort, RoutingPreset,
};
use iced::futures::{SinkExt, Stream};
use iced::stream;
use pipewire as pw;
use pw::{
    device::{Device, DeviceListener},
    keys,
    link::{Link, LinkListener, LinkState},
    node::{Node, NodeListener, NodeState},
    port::{Port, PortListener},
    properties::properties,
    spa::{
        param::ParamType,
        pod::{
            deserialize::PodDeserializer, serialize::PodSerializer, Object, Pod, Property,
            PropertyFlags, Value, ValueArray,
        },
        sys as spa_sys,
        utils::{dict::DictRef, SpaTypes},
    },
    types::ObjectType,
};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap, HashSet},
    ffi::CStr,
    io::Cursor,
    rc::Rc,
};
use tokio::sync::mpsc;

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
    SetNodeMute {
        node_serial: u64,
        muted: bool,
    },
    SetNodeChannelVolumes {
        node_serial: u64,
        channel_volumes: Vec<f32>,
    },
    SetNodePropParam {
        node_serial: u64,
        prop_key: u32,
        value: PwParamValue,
    },
    SetDeviceProfile {
        device_serial: u64,
        profile_index: i32,
    },
    SetDeviceRoute {
        device_serial: u64,
        route_index: i32,
        device_id: Option<i32>,
    },
    Sync,
    Quit,
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
    pub sync_request_count: usize,
}

impl Default for Backend {
    fn default() -> Self {
        Self {
            is_demo: false,
            last_status: String::from("Connecting to PipeWire via pipewire-rs..."),
            cmd_sender: None,
            next_demo_link_serial: 2000,
            sync_request_count: 0,
        }
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        self.quit();
    }
}

impl Backend {
    /// Creates a backend running in demo mode without background PipeWire threads.
    #[allow(dead_code)]
    pub fn demo() -> Self {
        let mut backend = Self::default();
        backend.is_demo = true;
        backend.last_status = String::from("Running in demo mode");
        backend
    }

    /// Returns true if the backend has an active live connection to PipeWire.
    pub fn is_healthy(&self) -> bool {
        !self.is_demo && self.cmd_sender.is_some()
    }

    /// Signals the PipeWire `MainLoop` thread to exit cleanly.
    pub fn quit(&mut self) {
        if let Some(tx) = self.cmd_sender.take() {
            let _ = tx.send(PwCommand::Quit);
        }
    }

    /// Requests a roundtrip sync from the PipeWire daemon.
    pub fn request_sync(&mut self) {
        self.sync_request_count = self.sync_request_count.wrapping_add(1);
        if let Some(tx) = &self.cmd_sender {
            let _ = tx.send(PwCommand::Sync);
        }
    }

    /// Sets the mute state on a node identified by `node_serial`.
    pub fn set_node_mute(
        &mut self,
        graph: &mut PwGraph,
        node_serial: u64,
        muted: bool,
    ) -> Result<(), String> {
        let Some(node) = graph.nodes.get_mut(&node_serial) else {
            return Err(format!("Node (serial #{node_serial}) not found"));
        };
        let title = node.title();
        node.muted = Some(muted);
        for p in &mut node.params {
            if p.param_type == "Props" && p.key_id == spa_sys::SPA_PROP_mute {
                p.value = PwParamValue::Bool(muted);
            }
        }

        if let Some(tx) = &self.cmd_sender {
            tx.send(PwCommand::SetNodeMute {
                node_serial,
                muted,
            })
            .map_err(|_| String::from("Failed to send SetNodeMute to PipeWire mainloop"))?;
            self.last_status = format!(
                "{} '{}' (serial #{node_serial})",
                if muted { "Muted" } else { "Unmuted" },
                title
            );
            return Ok(());
        }

        if self.is_demo {
            self.last_status = format!(
                "{} '{}' (serial #{node_serial})",
                if muted { "Muted" } else { "Unmuted" },
                title
            );
            return Ok(());
        }

        Err(String::from("PipeWire mainloop is not connected"))
    }

    /// Sets the master perceptual volume (`0.0..=1.5`) on a node identified by `node_serial`
    /// by updating `SPA_PROP_channelVolumes` (matching `pavucontrol` and KDE behavior).
    pub fn set_node_volume(
        &mut self,
        graph: &mut PwGraph,
        node_serial: u64,
        volume: f32,
    ) -> Result<(), String> {
        let vol = volume.clamp(0.0, 1.5);
        let Some(node) = graph.nodes.get_mut(&node_serial) else {
            return Err(format!("Node (serial #{node_serial}) not found"));
        };
        let title = node.title();
        node.volume = Some(vol);

        if node.channel_volumes.is_empty() {
            let inferred_channels = node
                .default_output_ports()
                .len()
                .max(node.input_ports.len())
                .max(1);
            node.channel_volumes = vec![vol; inferred_channels];
        } else {
            node.channel_volumes.fill(vol);
        }

        let ch_vols = node.channel_volumes.clone();
        let raw_cubic: Vec<f32> = ch_vols.iter().map(|v| v.powi(3)).collect();
        for p in &mut node.params {
            if p.param_type == "Props" && p.key_id == spa_sys::SPA_PROP_channelVolumes {
                p.value = PwParamValue::FloatArray(raw_cubic.clone());
            }
        }

        if let Some(tx) = &self.cmd_sender {
            tx.send(PwCommand::SetNodeChannelVolumes {
                node_serial,
                channel_volumes: ch_vols,
            })
            .map_err(|_| {
                String::from("Failed to send SetNodeChannelVolumes to PipeWire mainloop")
            })?;
            self.last_status = format!(
                "Set volume of '{}' (serial #{node_serial}) to {:.0}%",
                title,
                vol * 100.0
            );
            return Ok(());
        }

        if self.is_demo {
            self.last_status = format!(
                "Set volume of '{}' (serial #{node_serial}) to {:.0}%",
                title,
                vol * 100.0
            );
            return Ok(());
        }

        Err(String::from("PipeWire mainloop is not connected"))
    }

    /// Sets the volume (`0.0..=1.5`) of a single channel on a node identified by `node_serial`.
    pub fn set_node_channel_volume(
        &mut self,
        graph: &mut PwGraph,
        node_serial: u64,
        channel_index: usize,
        volume: f32,
    ) -> Result<(), String> {
        let vol = volume.clamp(0.0, 1.5);
        let Some(node) = graph.nodes.get_mut(&node_serial) else {
            return Err(format!("Node (serial #{node_serial}) not found"));
        };
        if channel_index >= node.channel_volumes.len() {
            return Err(format!("Channel index {channel_index} out of range"));
        }
        let title = node.title();
        let ch_label = node.channel_label(channel_index);
        node.channel_volumes[channel_index] = vol;
        let avg = node.channel_volumes.iter().copied().sum::<f32>()
            / node.channel_volumes.len() as f32;
        node.volume = Some(avg);

        let ch_vols = node.channel_volumes.clone();
        let raw_cubic: Vec<f32> = ch_vols.iter().map(|v| v.powi(3)).collect();
        for p in &mut node.params {
            if p.param_type == "Props" && p.key_id == spa_sys::SPA_PROP_channelVolumes {
                p.value = PwParamValue::FloatArray(raw_cubic.clone());
            }
        }

        if let Some(tx) = &self.cmd_sender {
            tx.send(PwCommand::SetNodeChannelVolumes {
                node_serial,
                channel_volumes: ch_vols,
            })
            .map_err(|_| {
                String::from("Failed to send SetNodeChannelVolumes to PipeWire mainloop")
            })?;
            self.last_status = format!(
                "Set {} channel volume of '{}' (serial #{node_serial}) to {:.0}%",
                ch_label,
                title,
                vol * 100.0
            );
            return Ok(());
        }

        if self.is_demo {
            self.last_status = format!(
                "Set {} channel volume of '{}' (serial #{node_serial}) to {:.0}%",
                ch_label,
                title,
                vol * 100.0
            );
            return Ok(());
        }

        Err(String::from("PipeWire mainloop is not connected"))
    }

    /// Sets an editable `Props` parameter on a node identified by `node_serial`.
    pub fn set_node_prop_param(
        &mut self,
        graph: &mut PwGraph,
        node_serial: u64,
        prop_key: u32,
        value: PwParamValue,
    ) -> Result<(), String> {
        let Some(node) = graph.nodes.get_mut(&node_serial) else {
            return Err(format!("Node (serial #{node_serial}) not found"));
        };
        let title = node.title();
        let mut param_name = format!("key #{prop_key}");
        for p in &mut node.params {
            if p.param_type == "Props" && p.key_id == prop_key {
                param_name = p.name.clone();
                p.value = value.clone();
            }
        }

        // Sync dedicated fields if the user edited mute or volume via the Params editor
        if prop_key == spa_sys::SPA_PROP_mute {
            if let PwParamValue::Bool(b) = value {
                node.muted = Some(b);
            }
        } else if prop_key == spa_sys::SPA_PROP_volume {
            if let PwParamValue::Float(f) = value {
                node.volume = Some(f.max(0.0).cbrt().clamp(0.0, 1.5));
            }
        }

        if let Some(tx) = &self.cmd_sender {
            tx.send(PwCommand::SetNodePropParam {
                node_serial,
                prop_key,
                value: value.clone(),
            })
            .map_err(|_| String::from("Failed to send SetNodePropParam to PipeWire mainloop"))?;
            self.last_status = format!(
                "Updated param '{}' = {} on '{}' (serial #{node_serial})",
                param_name,
                value.display_value(),
                title
            );
            return Ok(());
        }

        if self.is_demo {
            self.last_status = format!(
                "Updated param '{}' = {} on '{}' (serial #{node_serial})",
                param_name,
                value.display_value(),
                title
            );
            return Ok(());
        }

        Err(String::from("PipeWire mainloop is not connected"))
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

            let mut link_props = BTreeMap::new();
            link_props.insert("object.serial".to_string(), serial.to_string());
            link_props.insert("link.output.node".to_string(), out_node_id.to_string());
            link_props.insert("link.output.port".to_string(), out_port_id.to_string());
            link_props.insert("link.input.node".to_string(), in_node_id.to_string());
            link_props.insert("link.input.port".to_string(), in_port_id.to_string());
            link_props.insert("object.linger".to_string(), "1".to_string());

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
                    props: link_props,
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
                graph.links.remove(&link_serial);
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

    /// Reconciles the port links between `output_node_serial` and `input_node_serial` so that the
    /// resulting active links match `target_pairs` (`(output_port_serial, input_port_serial)`).
    ///
    /// - Links already present in `graph.links` that belong to `target_pairs` are kept intact
    ///   (never destroyed or toggled off).
    /// - Links between the two nodes that are NOT in `target_pairs` (or duplicate links) are destroyed.
    /// - Target pairs that are not currently connected are created.
    pub fn set_node_links(
        &mut self,
        graph: &mut PwGraph,
        output_node_serial: u64,
        input_node_serial: u64,
        target_pairs: &[(u64, u64)],
    ) -> Result<(), String> {
        let target_set: HashSet<(u64, u64)> = target_pairs.iter().copied().collect();
        let mut already_connected: HashSet<(u64, u64)> = HashSet::new();
        let mut link_serials_to_destroy: Vec<u64> = Vec::new();

        for link in graph.links.values() {
            if link.output_node_serial == output_node_serial
                && link.input_node_serial == input_node_serial
            {
                let pair = (link.output_port_serial, link.input_port_serial);
                if target_set.contains(&pair) && already_connected.insert(pair) {
                    // Already active and part of target route — keep it untouched!
                } else {
                    link_serials_to_destroy.push(link.object_serial);
                }
            }
        }

        let pairs_to_create: Vec<(u64, u64)> = target_pairs
            .iter()
            .copied()
            .filter(|pair| !already_connected.contains(pair))
            .collect();

        if let Some(tx) = &self.cmd_sender {
            for link_serial in link_serials_to_destroy {
                tx.send(PwCommand::DestroyLink { link_serial })
                    .map_err(|_| String::from("Failed to send DestroyLink to PipeWire mainloop"))?;
                graph.links.remove(&link_serial);
            }
            for (output_port_serial, input_port_serial) in pairs_to_create {
                tx.send(PwCommand::CreateLink {
                    output_node_serial,
                    output_port_serial,
                    input_node_serial,
                    input_port_serial,
                })
                .map_err(|_| String::from("Failed to send CreateLink to PipeWire mainloop"))?;
            }
            return Ok(());
        }

        if self.is_demo {
            for link_serial in link_serials_to_destroy {
                graph.links.remove(&link_serial);
            }
            for (output_port_serial, input_port_serial) in pairs_to_create {
                self.connect_ports(
                    graph,
                    output_node_serial,
                    output_port_serial,
                    input_node_serial,
                    input_port_serial,
                )?;
            }
            return Ok(());
        }

        Err(String::from("PipeWire mainloop is not connected"))
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

        self.set_node_links(graph, output_node_serial, input_node_serial, &pairs)?;

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
        self.set_node_links(graph, output_node_serial, input_node_serial, &[])?;
        self.last_status =
            format!("Disconnected wire (serial {output_node_serial} ↛ {input_node_serial})");
        Ok(())
    }

    /// Applies a routing preset between two nodes identified by `object_serial` by reconciling
    /// existing links with the preset's desired port pairs.
    pub fn apply_preset(
        &mut self,
        graph: &mut PwGraph,
        output_node_serial: u64,
        input_node_serial: u64,
        preset: RoutingPreset,
    ) -> Result<(), String> {
        if !graph.nodes.contains_key(&output_node_serial)
            || !graph.nodes.contains_key(&input_node_serial)
        {
            return Err("Node not found".to_string());
        }

        let target_pairs =
            graph.compute_preset_port_mapping(output_node_serial, input_node_serial, preset);

        self.set_node_links(graph, output_node_serial, input_node_serial, &target_pairs)?;
        self.last_status = format!(
            "Applied preset '{}' (serial {output_node_serial} → {input_node_serial})",
            preset.label()
        );
        Ok(())
    }

    /// Selects a device profile by `device_serial` and `profile_index`.
    /// Rejects selection if the profile availability is `PwAvailability::No`.
    pub fn set_device_profile(
        &mut self,
        graph: &mut PwGraph,
        device_serial: u64,
        profile_index: i32,
    ) -> Result<(), String> {
        let device = graph
            .devices
            .get_mut(&device_serial)
            .ok_or_else(|| format!("Device (serial #{device_serial}) not found"))?;

        let target_profile = device
            .profiles
            .iter()
            .find(|p| p.index == profile_index)
            .ok_or_else(|| {
                format!(
                    "Profile index {profile_index} not found on device '{}'",
                    device.title()
                )
            })?;

        if target_profile.available == PwAvailability::No {
            let err_msg = format!(
                "Cannot select unavailable profile '{}' on '{}'",
                target_profile.display_label(),
                device.title()
            );
            self.last_status = err_msg.clone();
            return Err(err_msg);
        }

        let dev_title = device.title().to_string();
        let profile_name = target_profile.display_label().to_string();

        for p in &mut device.profiles {
            p.active = p.index == profile_index;
        }
        device.active_profile = Some(profile_index);

        if let Some(sender) = &self.cmd_sender {
            let _ = sender.send(PwCommand::SetDeviceProfile {
                device_serial,
                profile_index,
            });
        }

        self.last_status = format!("Switched device '{dev_title}' to profile '{profile_name}'");
        Ok(())
    }

    /// Selects a device route (port) by `device_serial`, `route_index`, and optional `device_id`.
    /// Preserves directional exclusivity: changing an Output route only updates Output routes,
    /// leaving Input routes untouched (and vice-versa).
    /// Rejects selection if the route availability is `PwAvailability::No`.
    pub fn set_device_route(
        &mut self,
        graph: &mut PwGraph,
        device_serial: u64,
        route_index: i32,
        device_id: Option<i32>,
    ) -> Result<(), String> {
        let device = graph
            .devices
            .get_mut(&device_serial)
            .ok_or_else(|| format!("Device (serial #{device_serial}) not found"))?;

        let target_route = device
            .routes
            .iter()
            .find(|r| r.index == route_index)
            .ok_or_else(|| {
                format!(
                    "Route index {route_index} not found on device '{}'",
                    device.title()
                )
            })?;

        if target_route.available == PwAvailability::No {
            let err_msg = format!(
                "Cannot select unavailable route '{}' on '{}'",
                target_route.display_label(),
                device.title()
            );
            self.last_status = err_msg.clone();
            return Err(err_msg);
        }

        let dev_title = device.title().to_string();
        let route_name = target_route.display_label().to_string();
        let target_dir = target_route.direction;

        for r in &mut device.routes {
            if r.direction == target_dir {
                r.active = r.index == route_index;
            }
        }

        if let Some(sender) = &self.cmd_sender {
            let _ = sender.send(PwCommand::SetDeviceRoute {
                device_serial,
                route_index,
                device_id,
            });
        }

        self.last_status = format!("Switched device '{dev_title}' to route '{route_name}'");
        Ok(())
    }

    /// Convenience helper to switch a device profile for a node backed by a device.
    #[allow(dead_code)]
    pub fn set_node_device_profile(
        &mut self,
        graph: &mut PwGraph,
        node_serial: u64,
        profile_index: i32,
    ) -> Result<(), String> {
        let dev_serial = graph
            .device_for_node(node_serial)
            .map(|d| d.object_serial)
            .ok_or_else(|| format!("Node (serial #{node_serial}) is not backed by a device"))?;
        self.set_device_profile(graph, dev_serial, profile_index)
    }

    /// Convenience helper to switch a device route for a node backed by a device.
    #[allow(dead_code)]
    pub fn set_node_device_route(
        &mut self,
        graph: &mut PwGraph,
        node_serial: u64,
        route_index: i32,
        device_id: Option<i32>,
    ) -> Result<(), String> {
        let dev_serial = graph
            .device_for_node(node_serial)
            .map(|d| d.object_serial)
            .ok_or_else(|| format!("Node (serial #{node_serial}) is not backed by a device"))?;
        self.set_device_route(graph, dev_serial, route_index, device_id)
    }
}

struct PwLoopState {
    graph: PwGraph,
    link_factory: String,
    /// Maps live PipeWire global `id: u32` -> authoritative `object.serial: u64`.
    id_to_serial: HashMap<u32, u64>,
    created_links: Vec<Link>,
    bound_links: HashMap<u64, (Link, LinkListener)>,
    bound_ports: HashMap<u64, (Port, PortListener)>,
    bound_nodes: HashMap<u64, (Node, NodeListener)>,
    bound_devices: HashMap<u64, (Device, DeviceListener)>,
}

impl Default for PwLoopState {
    fn default() -> Self {
        Self {
            graph: PwGraph::default(),
            link_factory: String::from("link-factory"),
            id_to_serial: HashMap::new(),
            created_links: Vec::new(),
            bound_links: HashMap::new(),
            bound_ports: HashMap::new(),
            bound_nodes: HashMap::new(),
            bound_devices: HashMap::new(),
        }
    }
}

impl PwLoopState {
    /// Verifies that the node identified by `node_serial` still exists in the live PipeWire
    /// registry with its expected `object.serial`.
    fn verify_node_serial(&self, node_serial: u64) -> Result<u32, String> {
        let Some(live_node) = self.graph.nodes.get(&node_serial) else {
            return Err(format!(
                "Refused node operation: node (serial {node_serial}) no longer exists"
            ));
        };
        let live_serial = self.id_to_serial.get(&live_node.id).copied();
        if live_serial == Some(node_serial) {
            Ok(live_node.id)
        } else {
            Err(format!(
                "Refused node operation on global #{}: object.serial mismatch (expected {node_serial}, live is {:?})",
                live_node.id, live_serial
            ))
        }
    }

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

struct QuitOnDrop(Option<pw::channel::Sender<PwCommand>>);

impl Drop for QuitOnDrop {
    fn drop(&mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(PwCommand::Quit);
        }
    }
}

/// Iced `Subscription` stream that spawns the dedicated `pipewire-rs` `MainLoop` thread
/// via `tokio::task::spawn_blocking` and forwards real-time PipeWire registry events to the UI
/// using `tokio::sync::mpsc` (`blocking_send` -> async `recv`).
pub fn pipewire_subscription() -> impl Stream<Item = PwEvent> {
    stream::channel(
        128,
        |mut output: iced::futures::channel::mpsc::Sender<PwEvent>| async move {
            let (tx, mut rx) = mpsc::channel::<PwEvent>(128);
            let mut quit_guard = QuitOnDrop(None);

        let join_handle = tokio::task::spawn_blocking(move || run_pipewire_loop(tx));

        while let Some(event) = rx.recv().await {
            if let PwEvent::Connected(ref sender) = event {
                quit_guard.0 = Some(sender.0.clone());
            }
            if output.send(event).await.is_err() {
                break;
            }
        }

        // Ensure the PipeWire mainloop is told to quit before awaiting join_handle
        drop(quit_guard);

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
    if gui_tx
        .blocking_send(PwEvent::Connected(PwCommandSender(cmd_tx)))
        .is_err()
    {
        return Ok(());
    }

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
        let mainloop_done = mainloop.clone();
        let mainloop_err = mainloop.clone();

        core.add_listener_local()
            .done(move |id, seq| {
                if id == pw::core::PW_ID_CORE && Some(seq) == pending_sync.get() {
                    pending_sync.set(None);
                    let mut st = state.borrow_mut();
                    st.graph.resolve_serials_and_rebuild_ports();
                    if gui_tx_done
                        .blocking_send(PwEvent::GraphUpdated(st.graph.clone()))
                        .is_err()
                    {
                        mainloop_done.quit();
                    }
                }
            })
            .error(move |id, _seq, res, message| {
                if gui_tx_err
                    .blocking_send(PwEvent::Status(format!(
                        "PipeWire error (id {id}, code {res}): {message}"
                    )))
                    .is_err()
                {
                    mainloop_err.quit();
                }
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
                        let initial_props = dict_to_btreemap(props);

                        let node_id = global.id;
                        {
                            let mut st = state_global.borrow_mut();
                            let device_id = initial_props
                                .get("device.id")
                                .and_then(|s| s.parse::<u32>().ok());
                            let device_serial =
                                device_id.and_then(|did| st.id_to_serial.get(&did).copied());

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
                                    volume: None,
                                    muted: None,
                                    channel_volumes: Vec::new(),
                                    channel_names: Vec::new(),
                                    props: initial_props,
                                    params: Vec::new(),
                                    input_ports: Vec::new(),
                                    output_ports: Vec::new(),
                                    device_id,
                                    device_serial,
                                },
                            );
                        }

                        // Bind the Node proxy to track live state changes, properties, and SPA params
                        if let Some(reg) = registry_weak.upgrade() {
                            if let Ok(node_proxy) = reg.bind::<Node, _>(global) {
                                let state_info = state_global.clone();
                                let sync_info = trigger_sync_global.clone();
                                let state_param = state_global.clone();
                                let sync_param = trigger_sync_global.clone();

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
                                            if let Some(info_props) = info.props() {
                                                if merge_dict_into_btreemap(
                                                    &mut node_entry.props,
                                                    info_props,
                                                ) {
                                                    changed = true;
                                                }
                                            }
                                        }
                                        if changed {
                                            sync_info();
                                        }
                                    })
                                    .param(move |_seq, param_type, _index, _next, pod| {
                                        let Some(pod) = pod else {
                                            return;
                                        };
                                        let mut changed = false;
                                        if let Some(node_entry) = state_param
                                            .borrow_mut()
                                            .graph
                                            .nodes
                                            .get_mut(&object_serial)
                                        {
                                            changed =
                                                apply_node_param_pod(node_entry, param_type, pod);
                                        }
                                        if changed {
                                            sync_param();
                                        }
                                    })
                                    .register();

                                // Subscribe to live updates of Props, Format, Latency, and ProcessLatency
                                node_proxy.subscribe_params(&[
                                    ParamType::Props,
                                    ParamType::Format,
                                    ParamType::Latency,
                                    ParamType::ProcessLatency,
                                ]);

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
                        let initial_props = dict_to_btreemap(props);

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
                                props: initial_props,
                            },
                        );
                        drop(st);

                        // Bind the Port proxy to receive authoritative PortInfoRef properties
                        if let Some(reg) = registry_weak.upgrade() {
                            if let Ok(port_proxy) = reg.bind::<Port, _>(global) {
                                let state_info = state_global.clone();
                                let sync_info = trigger_sync_global.clone();
                                let listener = port_proxy
                                    .add_listener_local()
                                    .info(move |info| {
                                        let mut changed = false;
                                        if let Some(info_props) = info.props() {
                                            if let Some(port_entry) = state_info
                                                .borrow_mut()
                                                .graph
                                                .ports
                                                .get_mut(&object_serial)
                                            {
                                                if merge_dict_into_btreemap(
                                                    &mut port_entry.props,
                                                    info_props,
                                                ) {
                                                    changed = true;
                                                }
                                            }
                                        }
                                        if changed {
                                            sync_info();
                                        }
                                    })
                                    .register();
                                state_global
                                    .borrow_mut()
                                    .bound_ports
                                    .insert(object_serial, (port_proxy, listener));
                            }
                        }

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
                        let initial_props = dict_to_btreemap(props);

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
                                    props: initial_props,
                                },
                            );
                        }

                        // Bind the Link proxy to receive authoritative LinkInfoRef updates (including state, serial & props)
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

                                        let mut merged_props = st
                                            .graph
                                            .links
                                            .get(&info_serial)
                                            .map(|l| l.props.clone())
                                            .unwrap_or_default();
                                        if let Some(info_props) = info.props() {
                                            merge_dict_into_btreemap(&mut merged_props, info_props);
                                        }

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
                                                props: merged_props,
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
                    ObjectType::Device => {
                        let object_serial = props
                            .get(*keys::OBJECT_SERIAL)
                            .and_then(|s| s.parse::<u64>().ok())
                            .unwrap_or(global.id as u64);

                        let name = props
                            .get(*keys::DEVICE_NAME)
                            .or_else(|| props.get(*keys::DEVICE_NICK))
                            .or_else(|| props.get(*keys::DEVICE_DESCRIPTION))
                            .unwrap_or_default()
                            .to_string();
                        let description = props
                            .get(*keys::DEVICE_DESCRIPTION)
                            .unwrap_or_default()
                            .to_string();
                        let nick = props.get(*keys::DEVICE_NICK).map(str::to_string);
                        let media_class = props.get(*keys::MEDIA_CLASS).map(str::to_string);
                        let initial_props = dict_to_btreemap(props);

                        let device_id = global.id;
                        {
                            let mut st = state_global.borrow_mut();
                            st.id_to_serial.insert(device_id, object_serial);
                            st.graph.devices.insert(
                                object_serial,
                                PwDevice {
                                    id: device_id,
                                    object_serial,
                                    name,
                                    description,
                                    nick,
                                    media_class,
                                    props: initial_props,
                                    profiles: Vec::new(),
                                    routes: Vec::new(),
                                    active_profile: None,
                                },
                            );
                        }

                        // Bind the Device proxy to track live state changes, properties, and SPA params
                        if let Some(reg) = registry_weak.upgrade() {
                            if let Ok(device_proxy) = reg.bind::<Device, _>(global) {
                                let state_info = state_global.clone();
                                let sync_info = trigger_sync_global.clone();
                                let state_param = state_global.clone();
                                let sync_param = trigger_sync_global.clone();

                                let listener = device_proxy
                                    .add_listener_local()
                                    .info(move |info| {
                                        let mut changed = false;
                                        if let Some(props) = info.props() {
                                            let mut st = state_info.borrow_mut();
                                            if let Some(device) =
                                                st.graph.devices.get_mut(&object_serial)
                                            {
                                                changed = merge_dict_into_btreemap(
                                                    &mut device.props,
                                                    props,
                                                );
                                            }
                                        }
                                        if changed {
                                            sync_info();
                                        }
                                    })
                                    .param(move |_seq, param_type, _index, _next, param| {
                                        if let Some(pod) = param {
                                            let mut st = state_param.borrow_mut();
                                            if let Some(device) =
                                                st.graph.devices.get_mut(&object_serial)
                                            {
                                                if apply_device_param_pod(
                                                    device, param_type, pod,
                                                ) {
                                                    drop(st);
                                                    sync_param();
                                                }
                                            }
                                        }
                                    })
                                    .register();

                                device_proxy.subscribe_params(&[
                                    ParamType::EnumProfile,
                                    ParamType::Profile,
                                    ParamType::EnumRoute,
                                    ParamType::Route,
                                ]);
                                device_proxy.enum_params(
                                    0,
                                    Some(ParamType::EnumProfile),
                                    0,
                                    u32::MAX,
                                );
                                device_proxy.enum_params(0, Some(ParamType::Profile), 0, u32::MAX);
                                device_proxy.enum_params(0, Some(ParamType::EnumRoute), 0, u32::MAX);
                                device_proxy.enum_params(0, Some(ParamType::Route), 0, u32::MAX);

                                state_global
                                    .borrow_mut()
                                    .bound_devices
                                    .insert(object_serial, (device_proxy, listener));
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
                    st.bound_ports.remove(&serial);
                    changed = true;
                }
                if st.graph.links.remove(&serial).is_some() {
                    st.bound_links.remove(&serial);
                    changed = true;
                }
                if st.graph.devices.remove(&serial).is_some() {
                    st.bound_devices.remove(&serial);
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
        let mainloop_cmd = mainloop.clone();

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
                        let already_exists = state.borrow().graph.links.values().any(|l| {
                            l.output_port_id == output_port_id && l.input_port_id == input_port_id
                        });
                        if already_exists {
                            trigger_sync();
                            return;
                        }

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
            PwCommand::SetNodeMute {
                node_serial,
                muted,
            } => {
                let verification = state.borrow().verify_node_serial(node_serial);
                match verification {
                    Ok(_node_id) => {
                        let res = serialize_node_prop_pod(spa_sys::SPA_PROP_mute, Value::Bool(muted))
                            .and_then(|bytes| {
                                let st = state.borrow();
                                let Some((node_proxy, _)) = st.bound_nodes.get(&node_serial) else {
                                    return Err(format!(
                                        "Bound proxy for node serial {node_serial} not found"
                                    ));
                                };
                                let Some(pod) = Pod::from_bytes(&bytes) else {
                                    return Err(String::from("Failed to construct SPA Pod"));
                                };
                                node_proxy.set_param(ParamType::Props, 0, pod);
                                Ok(())
                            });
                        if let Err(err) = res {
                            let _ = gui_tx_cmd.blocking_send(PwEvent::Status(err));
                        }
                        trigger_sync();
                    }
                    Err(reason) => {
                        let _ = gui_tx_cmd.blocking_send(PwEvent::Status(reason));
                        trigger_sync();
                    }
                }
            }
            PwCommand::SetNodeChannelVolumes {
                node_serial,
                channel_volumes,
            } => {
                let verification = state.borrow().verify_node_serial(node_serial);
                match verification {
                    Ok(_node_id) => {
                        let raw_cubic: Vec<f32> = channel_volumes
                            .iter()
                            .map(|v| v.clamp(0.0, 1.5).powi(3))
                            .collect();
                        let res = serialize_node_prop_pod(
                            spa_sys::SPA_PROP_channelVolumes,
                            Value::ValueArray(ValueArray::Float(raw_cubic)),
                        )
                        .and_then(|bytes| {
                            let st = state.borrow();
                            let Some((node_proxy, _)) = st.bound_nodes.get(&node_serial) else {
                                return Err(format!(
                                    "Bound proxy for node serial {node_serial} not found"
                                ));
                            };
                            let Some(pod) = Pod::from_bytes(&bytes) else {
                                return Err(String::from("Failed to construct SPA Pod"));
                            };
                            node_proxy.set_param(ParamType::Props, 0, pod);
                            Ok(())
                        });
                        if let Err(err) = res {
                            let _ = gui_tx_cmd.blocking_send(PwEvent::Status(err));
                        }
                        trigger_sync();
                    }
                    Err(reason) => {
                        let _ = gui_tx_cmd.blocking_send(PwEvent::Status(reason));
                        trigger_sync();
                    }
                }
            }
            PwCommand::SetNodePropParam {
                node_serial,
                prop_key,
                value,
            } => {
                let verification = state.borrow().verify_node_serial(node_serial);
                match verification {
                    Ok(_node_id) => {
                        let spa_val_res = param_value_to_spa_value(&value);
                        let res = spa_val_res
                            .and_then(|spa_val| serialize_node_prop_pod(prop_key, spa_val))
                            .and_then(|bytes| {
                                let st = state.borrow();
                                let Some((node_proxy, _)) = st.bound_nodes.get(&node_serial) else {
                                    return Err(format!(
                                        "Bound proxy for node serial {node_serial} not found"
                                    ));
                                };
                                let Some(pod) = Pod::from_bytes(&bytes) else {
                                    return Err(String::from("Failed to construct SPA Pod"));
                                };
                                node_proxy.set_param(ParamType::Props, 0, pod);
                                Ok(())
                            });
                        if let Err(err) = res {
                            let _ = gui_tx_cmd.blocking_send(PwEvent::Status(err));
                        }
                        trigger_sync();
                    }
                    Err(reason) => {
                        let _ = gui_tx_cmd.blocking_send(PwEvent::Status(reason));
                        trigger_sync();
                    }
                }
            }
            PwCommand::SetDeviceProfile {
                device_serial,
                profile_index,
            } => {
                let res = serialize_device_profile_pod(profile_index).and_then(|bytes| {
                    let st = state.borrow();
                    let Some((device_proxy, _)) = st.bound_devices.get(&device_serial) else {
                        return Err(format!(
                            "Bound proxy for device serial {device_serial} not found"
                        ));
                    };
                    let Some(pod) = Pod::from_bytes(&bytes) else {
                        return Err(String::from("Failed to construct SPA Pod"));
                    };
                    device_proxy.set_param(ParamType::Profile, 0, pod);
                    Ok(())
                });
                if let Err(err) = res {
                    let _ = gui_tx_cmd.blocking_send(PwEvent::Status(err));
                }
                trigger_sync();
            }
            PwCommand::SetDeviceRoute {
                device_serial,
                route_index,
                device_id,
            } => {
                let res = serialize_device_route_pod(route_index, device_id).and_then(|bytes| {
                    let st = state.borrow();
                    let Some((device_proxy, _)) = st.bound_devices.get(&device_serial) else {
                        return Err(format!(
                            "Bound proxy for device serial {device_serial} not found"
                        ));
                    };
                    let Some(pod) = Pod::from_bytes(&bytes) else {
                        return Err(String::from("Failed to construct SPA Pod"));
                    };
                    device_proxy.set_param(ParamType::Route, 0, pod);
                    Ok(())
                });
                if let Err(err) = res {
                    let _ = gui_tx_cmd.blocking_send(PwEvent::Status(err));
                }
                trigger_sync();
            }
            PwCommand::Sync => {
                trigger_sync();
            }
            PwCommand::Quit => {
                mainloop_cmd.quit();
            }
        })
    };

    // Perform initial sync roundtrip so the initial graph is delivered
    trigger_sync();

    mainloop.run();
    Ok(())
}

fn dict_to_btreemap(dict: &DictRef) -> BTreeMap<String, String> {
    dict.iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn merge_dict_into_btreemap(target: &mut BTreeMap<String, String>, dict: &DictRef) -> bool {
    let mut changed = false;
    for (k, v) in dict.iter() {
        match target.get(k) {
            Some(existing) if existing == v => {}
            _ => {
                target.insert(k.to_string(), v.to_string());
                changed = true;
            }
        }
    }
    changed
}

fn serialize_node_prop_pod(prop_key: u32, spa_val: Value) -> Result<Vec<u8>, String> {
    let obj = Value::Object(Object {
        type_: SpaTypes::ObjectParamProps.as_raw(),
        id: ParamType::Props.as_raw(),
        properties: vec![Property::new(prop_key, spa_val)],
    });
    let (cursor, _) = PodSerializer::serialize(Cursor::new(Vec::new()), &obj)
        .map_err(|e| format!("Failed to serialize SPA Pod: {e}"))?;
    Ok(cursor.into_inner())
}

/// Serializes a device profile SPA Pod (`SpaTypes::ObjectParamProfile`, `ParamType::Profile`).
pub fn serialize_device_profile_pod(profile_index: i32) -> Result<Vec<u8>, String> {
    let obj = Value::Object(Object {
        type_: SpaTypes::ObjectParamProfile.as_raw(),
        id: ParamType::Profile.as_raw(),
        properties: vec![
            Property::new(spa_sys::SPA_PARAM_PROFILE_index, Value::Int(profile_index)),
            Property::new(spa_sys::SPA_PARAM_PROFILE_save, Value::Bool(true)),
        ],
    });
    let (cursor, _) = PodSerializer::serialize(Cursor::new(Vec::new()), &obj)
        .map_err(|e| format!("Failed to serialize SPA Pod: {e}"))?;
    Ok(cursor.into_inner())
}

/// Serializes a device route SPA Pod (`SpaTypes::ObjectParamRoute`, `ParamType::Route`).
pub fn serialize_device_route_pod(
    route_index: i32,
    device_id: Option<i32>,
) -> Result<Vec<u8>, String> {
    let mut properties = vec![
        Property::new(spa_sys::SPA_PARAM_ROUTE_index, Value::Int(route_index)),
        Property::new(spa_sys::SPA_PARAM_ROUTE_save, Value::Bool(true)),
    ];
    if let Some(dev) = device_id {
        properties.push(Property::new(spa_sys::SPA_PARAM_ROUTE_device, Value::Int(dev)));
    }
    let obj = Value::Object(Object {
        type_: SpaTypes::ObjectParamRoute.as_raw(),
        id: ParamType::Route.as_raw(),
        properties,
    });
    let (cursor, _) = PodSerializer::serialize(Cursor::new(Vec::new()), &obj)
        .map_err(|e| format!("Failed to serialize SPA Pod: {e}"))?;
    Ok(cursor.into_inner())
}

fn param_value_to_spa_value(val: &PwParamValue) -> Result<Value, String> {
    match val {
        PwParamValue::Bool(b) => Ok(Value::Bool(*b)),
        PwParamValue::Int(i) => Ok(Value::Int(*i)),
        PwParamValue::Long(l) => Ok(Value::Long(*l)),
        PwParamValue::Float(f) => Ok(Value::Float(*f)),
        PwParamValue::Double(d) => Ok(Value::Double(*d)),
        PwParamValue::String(s) => Ok(Value::String(s.clone())),
        PwParamValue::FloatArray(v) => Ok(Value::ValueArray(ValueArray::Float(v.clone()))),
        PwParamValue::DisplayOnly(_) => Err(String::from("Cannot serialize a display-only param")),
    }
}

fn param_type_label(param_type: ParamType) -> &'static str {
    if param_type == ParamType::Props {
        "Props"
    } else if param_type == ParamType::Format {
        "Format"
    } else if param_type == ParamType::Latency {
        "Latency"
    } else if param_type == ParamType::ProcessLatency {
        "ProcessLatency"
    } else {
        "Param"
    }
}

fn param_type_rank(param_type: &str) -> u8 {
    match param_type {
        "Props" => 0,
        "Format" => 1,
        "Latency" => 2,
        "ProcessLatency" => 3,
        _ => 4,
    }
}

fn spa_short_name(type_table: *const spa_sys::spa_type_info, id: u32) -> Option<String> {
    if type_table.is_null() {
        return None;
    }
    // SAFETY: `spa_debug_type_find_short_name` inspects static null-terminated SPA type tables
    // exported by `libspa-sys` and returns either null or a static C string pointer.
    unsafe {
        let ptr = spa_sys::spa_debug_type_find_short_name(type_table, id);
        if ptr.is_null() {
            None
        } else {
            CStr::from_ptr(ptr).to_str().ok().map(str::to_string)
        }
    }
}

fn lookup_spa_prop_name(obj_type: u32, key: u32) -> String {
    let name_opt = unsafe {
        if obj_type == SpaTypes::ObjectParamProps.as_raw() {
            spa_short_name(spa_sys::spa_type_props, key)
        } else if obj_type == SpaTypes::ObjectParamFormat.as_raw() {
            spa_short_name(spa_sys::spa_type_format, key)
        } else if obj_type == SpaTypes::ObjectParamLatency.as_raw() {
            spa_short_name(spa_sys::spa_type_param_latency, key)
        } else if obj_type == SpaTypes::ObjectParamProcessLatency.as_raw() {
            spa_short_name(spa_sys::spa_type_param_process_latency, key)
        } else {
            None
        }
    };
    name_opt.unwrap_or_else(|| format!("0x{key:x}"))
}

fn lookup_audio_channel_name(channel_id: u32) -> String {
    unsafe {
        spa_short_name(spa_sys::spa_type_audio_channel, channel_id)
            .unwrap_or_else(|| format!("CH{channel_id}"))
    }
}

fn format_spa_id_value(obj_type: u32, key: u32, id_val: u32) -> String {
    let resolved = unsafe {
        if obj_type == SpaTypes::ObjectParamFormat.as_raw() {
            if key == spa_sys::SPA_FORMAT_mediaType {
                spa_short_name(spa_sys::spa_type_media_type, id_val)
            } else if key == spa_sys::SPA_FORMAT_mediaSubtype {
                spa_short_name(spa_sys::spa_type_media_subtype, id_val)
            } else if key == spa_sys::SPA_FORMAT_AUDIO_format {
                spa_short_name(spa_sys::spa_type_audio_format, id_val)
            } else {
                None
            }
        } else {
            None
        }
    };
    resolved.unwrap_or_else(|| format!("Id({id_val})"))
}

fn apply_node_param_pod(node: &mut PwNode, param_type: ParamType, pod: &Pod) -> bool {
    let Ok(Ok((_, Value::Object(obj)))) =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            PodDeserializer::deserialize_from::<Value>(pod.as_bytes())
        }))
    else {
        return false;
    };

    let ptype_str = param_type_label(param_type).to_string();
    let is_props = param_type == ParamType::Props;

    let mut extracted: Vec<PwNodeParam> = Vec::new();

    for prop in &obj.properties {
        let key_id = prop.key;
        let name = lookup_spa_prop_name(obj.type_, key_id);
        let flag_ro = prop.flags.contains(PropertyFlags::READONLY);
        let non_editable = !is_props || flag_ro;

        if is_props {
            if key_id == spa_sys::SPA_PROP_mute {
                if let Value::Bool(b) = prop.value {
                    node.muted = Some(b);
                }
            } else if key_id == spa_sys::SPA_PROP_volume {
                if let Value::Float(f) = prop.value {
                    if node.channel_volumes.is_empty() {
                        node.volume = Some(f.max(0.0).cbrt().clamp(0.0, 1.5));
                    }
                }
            } else if key_id == spa_sys::SPA_PROP_channelVolumes {
                if let Value::ValueArray(ValueArray::Float(ref vols)) = prop.value {
                    let user_vols: Vec<f32> = vols
                        .iter()
                        .map(|&v| v.max(0.0).cbrt().clamp(0.0, 1.5))
                        .collect();
                    if !user_vols.is_empty() {
                        let avg = user_vols.iter().copied().sum::<f32>() / user_vols.len() as f32;
                        node.volume = Some(avg);
                        node.channel_volumes = user_vols;
                    }
                }
            } else if key_id == spa_sys::SPA_PROP_channelMap {
                if let Value::ValueArray(ValueArray::Id(ref ids)) = prop.value {
                    node.channel_names = ids
                        .iter()
                        .map(|id| lookup_audio_channel_name(id.0))
                        .collect();
                }
            }
        }

        let (param_val, read_only) = match &prop.value {
            Value::Bool(b) => (PwParamValue::Bool(*b), non_editable),
            Value::Int(i) => (PwParamValue::Int(*i), non_editable),
            Value::Long(l) => (PwParamValue::Long(*l), non_editable),
            Value::Float(f) => (PwParamValue::Float(*f), non_editable),
            Value::Double(d) => (PwParamValue::Double(*d), non_editable),
            Value::String(s) => {
                let clean = s
                    .split('\0')
                    .next()
                    .unwrap_or_default()
                    .trim_matches('\0')
                    .to_string();
                (PwParamValue::String(clean), true)
            }
            Value::Id(id) => (
                PwParamValue::DisplayOnly(format_spa_id_value(obj.type_, key_id, id.0)),
                true,
            ),
            Value::Rectangle(rect) => (
                PwParamValue::DisplayOnly(format!("{}x{}", rect.width, rect.height)),
                true,
            ),
            Value::Fraction(frac) => (
                PwParamValue::DisplayOnly(format!("{}/{}", frac.num, frac.denom)),
                true,
            ),
            Value::ValueArray(arr) => match arr {
                ValueArray::Float(fs) => (PwParamValue::FloatArray(fs.clone()), true),
                ValueArray::Int(is) => (
                    PwParamValue::DisplayOnly(format!(
                        "[{}]",
                        is.iter()
                            .map(i32::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                    true,
                ),
                ValueArray::Long(ls) => (
                    PwParamValue::DisplayOnly(format!(
                        "[{}]",
                        ls.iter()
                            .map(i64::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                    true,
                ),
                ValueArray::Id(ids) => {
                    if is_props && key_id == spa_sys::SPA_PROP_channelMap {
                        let names: Vec<String> = ids
                            .iter()
                            .map(|id| lookup_audio_channel_name(id.0))
                            .collect();
                        (
                            PwParamValue::DisplayOnly(format!("[{}]", names.join(", "))),
                            true,
                        )
                    } else {
                        let names: Vec<String> = ids
                            .iter()
                            .map(|id| format_spa_id_value(obj.type_, key_id, id.0))
                            .collect();
                        (
                            PwParamValue::DisplayOnly(format!("[{}]", names.join(", "))),
                            true,
                        )
                    }
                }
                _ => (PwParamValue::DisplayOnly(format!("{arr:?}")), true),
            },
            other => (PwParamValue::DisplayOnly(format!("{other:?}")), true),
        };

        if let Some(existing) = node
            .params
            .iter_mut()
            .find(|p| p.param_type == ptype_str && p.key_id == key_id)
        {
            existing.name = name;
            existing.value = param_val;
            existing.read_only = read_only;
        } else {
            extracted.push(PwNodeParam {
                param_type: ptype_str.clone(),
                key_id,
                name,
                value: param_val,
                read_only,
            });
        }
    }

    node.params.extend(extracted);
    node.params
        .sort_by_key(|p| (param_type_rank(&p.param_type), p.key_id));
    true
}

/// Applies an incoming SPA Pod parameter for a device (handling profiles and routes).
pub fn apply_device_param_pod(
    device: &mut PwDevice,
    param_type: ParamType,
    pod: &Pod,
) -> bool {
    let Ok(Ok((_, Value::Object(obj)))) =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            PodDeserializer::deserialize_from::<Value>(pod.as_bytes())
        }))
    else {
        return false;
    };

    if param_type == ParamType::EnumProfile {
        let mut index = None;
        let mut name = String::new();
        let mut description = String::new();
        let mut priority = 0u32;
        let mut available = PwAvailability::Unknown;

        for prop in &obj.properties {
            if prop.key == spa_sys::SPA_PARAM_PROFILE_index {
                if let Value::Int(i) = prop.value {
                    index = Some(i);
                }
            } else if prop.key == spa_sys::SPA_PARAM_PROFILE_name {
                if let Value::String(ref s) = prop.value {
                    name = s.trim_matches('\0').to_string();
                }
            } else if prop.key == spa_sys::SPA_PARAM_PROFILE_description {
                if let Value::String(ref s) = prop.value {
                    description = s.trim_matches('\0').to_string();
                }
            } else if prop.key == spa_sys::SPA_PARAM_PROFILE_priority {
                if let Value::Int(p) = prop.value {
                    priority = p.max(0) as u32;
                }
            } else if prop.key == spa_sys::SPA_PARAM_PROFILE_available {
                let avail_id = match prop.value {
                    Value::Id(id) => id.0,
                    Value::Int(i) => i as u32,
                    _ => 0,
                };
                available = PwAvailability::from_spa_id(avail_id);
            }
        }

        if let Some(idx) = index {
            if let Some(existing) = device.profiles.iter_mut().find(|p| p.index == idx) {
                existing.name = name;
                existing.description = description;
                existing.priority = priority;
                existing.available = available;
            } else {
                let is_active = device.active_profile == Some(idx);
                device.profiles.push(PwDeviceProfile {
                    index: idx,
                    name,
                    description,
                    priority,
                    available,
                    active: is_active,
                });
            }
            device.profiles.sort_by_key(|p| p.index);
            return true;
        }
    } else if param_type == ParamType::Profile {
        let mut active_idx = None;
        let mut active_name = String::new();

        for prop in &obj.properties {
            if prop.key == spa_sys::SPA_PARAM_PROFILE_index {
                if let Value::Int(i) = prop.value {
                    active_idx = Some(i);
                }
            } else if prop.key == spa_sys::SPA_PARAM_PROFILE_name {
                if let Value::String(ref s) = prop.value {
                    active_name = s.trim_matches('\0').to_string();
                }
            }
        }

        if let Some(idx) = active_idx {
            device.active_profile = Some(idx);
            for p in &mut device.profiles {
                p.active = p.index == idx || (!active_name.is_empty() && p.name == active_name);
            }
            return true;
        }
    } else if param_type == ParamType::EnumRoute {
        let mut index = None;
        let mut direction = PortDirection::Output;
        let mut device_id = None;
        let mut name = String::new();
        let mut description = String::new();
        let mut _priority = 0u32;
        let mut available = PwAvailability::Unknown;
        let mut profiles = Vec::new();

        for prop in &obj.properties {
            if prop.key == spa_sys::SPA_PARAM_ROUTE_index {
                if let Value::Int(i) = prop.value {
                    index = Some(i);
                }
            } else if prop.key == spa_sys::SPA_PARAM_ROUTE_direction {
                let dir_val = match prop.value {
                    Value::Id(id) => id.0,
                    Value::Int(i) => i as u32,
                    _ => 1,
                };
                direction = if dir_val == 0 {
                    PortDirection::Input
                } else {
                    PortDirection::Output
                };
            } else if prop.key == spa_sys::SPA_PARAM_ROUTE_device {
                if let Value::Int(d) = prop.value {
                    device_id = Some(d);
                }
            } else if prop.key == spa_sys::SPA_PARAM_ROUTE_name {
                if let Value::String(ref s) = prop.value {
                    name = s.trim_matches('\0').to_string();
                }
            } else if prop.key == spa_sys::SPA_PARAM_ROUTE_description {
                if let Value::String(ref s) = prop.value {
                    description = s.trim_matches('\0').to_string();
                }
            } else if prop.key == spa_sys::SPA_PARAM_ROUTE_priority {
                if let Value::Int(p) = prop.value {
                    _priority = p.max(0) as u32;
                }
            } else if prop.key == spa_sys::SPA_PARAM_ROUTE_available {
                let avail_id = match prop.value {
                    Value::Id(id) => id.0,
                    Value::Int(i) => i as u32,
                    _ => 0,
                };
                available = PwAvailability::from_spa_id(avail_id);
            } else if prop.key == spa_sys::SPA_PARAM_ROUTE_profiles {
                if let Value::ValueArray(ValueArray::Int(ref profs)) = prop.value {
                    profiles = profs.clone();
                }
            }
        }

        if let Some(idx) = index {
            if let Some(existing) = device
                .routes
                .iter_mut()
                .find(|r| r.index == idx && r.direction == direction)
            {
                existing.name = name;
                existing.description = description;
                existing.available = available;
                existing.device_id = device_id;
                existing.profiles = profiles;
            } else {
                device.routes.push(PwDeviceRoute {
                    index: idx,
                    name,
                    description,
                    direction,
                    available,
                    active: false,
                    device_id,
                    profiles,
                    props: BTreeMap::new(),
                });
            }
            device.routes.sort_by_key(|r| (r.direction == PortDirection::Output, r.index));
            return true;
        }
    } else if param_type == ParamType::Route {
        let mut active_idx = None;
        let mut active_name = String::new();
        let mut direction = None;

        for prop in &obj.properties {
            if prop.key == spa_sys::SPA_PARAM_ROUTE_index {
                if let Value::Int(i) = prop.value {
                    active_idx = Some(i);
                }
            } else if prop.key == spa_sys::SPA_PARAM_ROUTE_direction {
                let dir_val = match prop.value {
                    Value::Id(id) => id.0,
                    Value::Int(i) => i as u32,
                    _ => 1,
                };
                direction = Some(if dir_val == 0 {
                    PortDirection::Input
                } else {
                    PortDirection::Output
                });
            } else if prop.key == spa_sys::SPA_PARAM_ROUTE_name {
                if let Value::String(ref s) = prop.value {
                    active_name = s.trim_matches('\0').to_string();
                }
            }
        }

        if let Some(idx) = active_idx {
            for r in &mut device.routes {
                if let Some(dir) = direction {
                    if r.direction == dir {
                        r.active = r.index == idx
                            || (!active_name.is_empty() && r.name == active_name);
                    }
                } else {
                    r.active = r.index == idx || (!active_name.is_empty() && r.name == active_name);
                }
            }
            return true;
        }
    }

    false
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
                props: BTreeMap::new(),
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
    fn test_verify_node_serial_checks_object_serial() {
        let mut st = PwLoopState::default();
        st.id_to_serial.insert(40, 10);
        st.graph.nodes.insert(
            10,
            PwNode {
                id: 40,
                object_serial: 10,
                name: "node".into(),
                description: "Node".into(),
                app_name: None,
                media_name: None,
                media_class: "Audio/Sink".into(),
                category: NodeCategory::AudioSink,
                state: "running".into(),
                volume: Some(1.0),
                muted: Some(false),
                channel_volumes: vec![1.0, 1.0],
                channel_names: vec!["FL".into(), "FR".into()],
                props: BTreeMap::new(),
                params: vec![],
                input_ports: vec![],
                output_ports: vec![],
                device_id: None,
                device_serial: None,
            },
        );

        assert_eq!(st.verify_node_serial(10), Ok(40));
        st.id_to_serial.insert(40, 999);
        assert!(st.verify_node_serial(10).is_err());
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
                volume: None,
                muted: None,
                channel_volumes: vec![],
                channel_names: vec![],
                props: BTreeMap::new(),
                params: vec![],
                input_ports: vec![],
                output_ports: vec![],
                device_id: None,
                device_serial: None,
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
                props: BTreeMap::new(),
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
                volume: None,
                muted: None,
                channel_volumes: vec![],
                channel_names: vec![],
                props: BTreeMap::new(),
                params: vec![],
                input_ports: vec![],
                output_ports: vec![],
                device_id: None,
                device_serial: None,
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
                props: BTreeMap::new(),
            },
        );

        // Valid serials resolve to live global IDs (10, 11, 20, 21)
        assert_eq!(
            st.verify_create_link(100, 110, 200, 210),
            Ok((10, 11, 20, 21))
        );

        // If port global #21 is replaced by a new object with serial 999, verification rejects link creation
        st.id_to_serial.insert(21, 999);
        assert!(st.verify_create_link(100, 110, 200, 210).is_err());
    }

    #[test]
    fn test_apply_preset_reconciles_without_toggling_existing_links() {
        let mut graph = create_demo_graph();
        let mut backend = Backend::default();
        backend.is_demo = true;

        // Initially Firefox (20: 2051 FL, 2052 FR) -> Headphones (50: 5001 FL, 5002 FR)
        // has links 903 (2051->5001) and 904 (2052->5002).
        // Toggle off 2052->5002 and toggle on 2051->5002 so we have (2051->5001) and (2051->5002).
        backend
            .toggle_port_link(&mut graph, 20, 2052, 50, 5002)
            .unwrap();
        backend
            .toggle_port_link(&mut graph, 20, 2051, 50, 5002)
            .unwrap();

        // Link 903 (2051->5001) is still present with serial 903.
        assert!(graph.links.contains_key(&903));

        // Now apply Stereo1To1 preset:
        // - Existing link 903 (2051->5001) must be KEPT (same serial 903, not destroyed/toggled!)
        // - Extra link (2051->5002) must be DESTROYED
        // - Missing link (2052->5002) must be CREATED
        backend
            .apply_preset(&mut graph, 20, 50, RoutingPreset::Stereo1To1)
            .unwrap();

        assert!(
            graph.links.contains_key(&903),
            "Existing link in target preset should be preserved untouched"
        );
        assert!(graph.is_preset_active(20, 50, RoutingPreset::Stereo1To1));
    }

    #[test]
    fn test_pod_roundtrip_updates_node_volume_mute_and_params() {
        let mut graph = create_demo_graph();
        let node = graph.nodes.get_mut(&20).unwrap();

        // Serialize a mute = true SPA Pod and apply it to the node
        let mute_bytes =
            serialize_node_prop_pod(spa_sys::SPA_PROP_mute, Value::Bool(true)).unwrap();
        let mute_pod = Pod::from_bytes(&mute_bytes).unwrap();
        assert!(apply_node_param_pod(node, ParamType::Props, mute_pod));
        assert_eq!(node.muted, Some(true));

        // Serialize channelVolumes = [0.125, 0.125] (which is 0.5^3 = 50% perceptual volume)
        let vol_bytes = serialize_node_prop_pod(
            spa_sys::SPA_PROP_channelVolumes,
            Value::ValueArray(ValueArray::Float(vec![0.125, 0.125])),
        )
        .unwrap();
        let vol_pod = Pod::from_bytes(&vol_bytes).unwrap();
        assert!(apply_node_param_pod(node, ParamType::Props, vol_pod));
        let v = node.volume.unwrap();
        assert!((v - 0.5).abs() < 1e-4, "Expected ~0.5, got {v}");
        assert_eq!(node.channel_volumes.len(), 2);

        // Verify that updating a Props param (such as volume) never causes [Latency] or [Format]
        // to jump above [Props], and keeps params deterministically ordered by (group, key_id).
        node.params.push(PwNodeParam {
            param_type: "Latency".into(),
            key_id: 1,
            name: "direction".into(),
            value: PwParamValue::String("Output".into()),
            read_only: true,
        });
        let vol_bytes2 = serialize_node_prop_pod(
            spa_sys::SPA_PROP_channelVolumes,
            Value::ValueArray(ValueArray::Float(vec![1.0, 1.0])),
        )
        .unwrap();
        let vol_pod2 = Pod::from_bytes(&vol_bytes2).unwrap();
        assert!(apply_node_param_pod(node, ParamType::Props, vol_pod2));
        assert_eq!(node.params.first().unwrap().param_type, "Props");
        assert_eq!(node.params.last().unwrap().param_type, "Latency");
    }
}

