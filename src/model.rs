use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortDirection {
    Input,
    Output,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PwAvailability {
    Unknown,
    No,
    Yes,
}

impl PwAvailability {
    pub fn is_available(&self) -> bool {
        matches!(self, PwAvailability::Yes | PwAvailability::Unknown)
    }

    pub fn label(&self) -> &'static str {
        match self {
            PwAvailability::Unknown => "Unknown",
            PwAvailability::No => "Unavailable",
            PwAvailability::Yes => "Available",
        }
    }

    pub fn from_spa_id(id: u32) -> Self {
        match id {
            1 => PwAvailability::No,
            2 => PwAvailability::Yes,
            _ => PwAvailability::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PwDeviceProfile {
    pub index: i32,
    pub name: String,
    pub description: String,
    pub priority: u32,
    pub available: PwAvailability,
    pub active: bool,
}

impl PwDeviceProfile {
    pub fn display_label(&self) -> &str {
        if !self.description.is_empty() {
            &self.description
        } else {
            &self.name
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PwDeviceRoute {
    pub index: i32,
    pub name: String,
    pub description: String,
    pub direction: PortDirection,
    pub available: PwAvailability,
    pub active: bool,
    pub device_id: Option<i32>,
    pub devices: Vec<i32>,
    pub profiles: Vec<i32>,
    pub props: BTreeMap<String, String>,
}

impl PwDeviceRoute {
    pub fn display_label(&self) -> &str {
        if !self.description.is_empty() {
            &self.description
        } else {
            &self.name
        }
    }

    /// Resolves the card sub-device ID corresponding to a given profile index.
    ///
    /// If `devices` and `profiles` are parallel arrays of equal length, `devices[k]`
    /// corresponds to `profiles[k]`. Otherwise, falls back to `self.device_id` or the
    /// first device in `self.devices`.
    pub fn device_id_for_profile(&self, profile_index: Option<i32>) -> Option<i32> {
        if let Some(prof_idx) = profile_index {
            if self.devices.len() == self.profiles.len() && !self.devices.is_empty() {
                if let Some(pos) = self.profiles.iter().position(|&p| p == prof_idx) {
                    if let Some(&dev) = self.devices.get(pos) {
                        return Some(dev);
                    }
                }
            }
        }
        self.device_id.or_else(|| self.devices.first().copied())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PwDevice {
    pub id: u32,
    pub object_serial: u64,
    pub name: String,
    pub description: String,
    pub nick: Option<String>,
    pub media_class: Option<String>,
    pub props: BTreeMap<String, String>,
    pub profiles: Vec<PwDeviceProfile>,
    pub routes: Vec<PwDeviceRoute>,
    pub active_profile: Option<i32>,
}

impl PwDevice {
    pub fn title(&self) -> &str {
        if let Some(nick) = &self.nick {
            if !nick.is_empty() {
                return nick;
            }
        }
        if !self.description.is_empty() {
            return &self.description;
        }
        &self.name
    }

    /// Returns the currently active profile index for this device, if any.
    pub fn active_profile_index(&self) -> Option<i32> {
        self.active_profile.or_else(|| {
            self.profiles.iter().find(|p| p.active).map(|p| p.index)
        })
    }

    /// Returns an iterator over routes that are compatible with the currently active profile.
    ///
    /// - If the device profile is "Off" (index 0 or name "off"), no routes are compatible.
    /// - If an active profile is set, only routes whose `profiles` list is empty (valid for all)
    ///   or includes the active profile index are returned.
    /// - If no profile is set/known, all routes are returned.
    pub fn routes_for_active_profile(&self) -> impl Iterator<Item = &PwDeviceRoute> {
        let active_idx = self.active_profile_index();
        let is_off = active_idx == Some(0)
            || self
                .profiles
                .iter()
                .find(|p| p.active)
                .map(|p| p.index == 0 || p.name == "off")
                .unwrap_or(false);

        self.routes.iter().filter(move |r| {
            if is_off {
                return false;
            }
            match active_idx {
                Some(idx) => r.profiles.is_empty() || r.profiles.contains(&idx),
                None => true,
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelLayout {
    Empty,
    Mono,
    Stereo,
    Surround(usize),
    MultiChannel(usize),
}

impl ChannelLayout {
    pub fn from_ports(ports: &[&PwPort]) -> Self {
        match ports.len() {
            0 => ChannelLayout::Empty,
            1 => ChannelLayout::Mono,
            2 => ChannelLayout::Stereo,
            n => {
                let has_surround_tags = ports.iter().any(|p| {
                    matches!(
                        p.channel.as_str(),
                        "FL" | "FR" | "FC" | "LFE" | "RL" | "RR" | "SL" | "SR" | "FLC" | "FRC"
                    )
                });
                if has_surround_tags {
                    ChannelLayout::Surround(n)
                } else {
                    ChannelLayout::MultiChannel(n)
                }
            }
        }
    }

    pub fn label(&self, ports: &[&PwPort]) -> String {
        match *self {
            ChannelLayout::Empty => String::from("No Ports"),
            ChannelLayout::Mono => String::from("Mono"),
            ChannelLayout::Stereo => String::from("Stereo"),
            ChannelLayout::Surround(n) => {
                let has_lfe = ports.iter().any(|p| p.channel == "LFE");
                if n == 6 && has_lfe {
                    String::from("5.1 Surround")
                } else if n == 8 && has_lfe {
                    String::from("7.1 Surround")
                } else if n == 3 && has_lfe {
                    String::from("2.1 Surround")
                } else if n == 4 && !has_lfe {
                    String::from("4.0 Quad")
                } else if has_lfe {
                    format!("{}.1 Surround", n - 1)
                } else {
                    format!("{n}.0 Surround")
                }
            }
            ChannelLayout::MultiChannel(n) => format!("{n}-Ch Multi"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingPreset {
    /// Mono (1) -> Mono (1): direct 1:1 connection
    Mono1To1,
    /// Mono (1) -> Stereo (2): route mono output to both Left & Right inputs
    MonoToStereoBoth,
    /// Mono (1) -> Stereo (2): route mono output to Left input only
    MonoToLeftOnly,
    /// Mono (1) -> Stereo (2): route mono output to Right input only
    MonoToRightOnly,
    /// Stereo (2) -> Mono (1): sum/connect both Left & Right outputs into Mono input
    StereoToMonoBoth,
    /// Stereo (2) -> Mono (1): route Left output only to Mono input
    LeftOnlyToMono,
    /// Stereo (2) -> Mono (1): route Right output only to Mono input
    RightOnlyToMono,
    /// Stereo (2) -> Stereo (2): standard 1:1 (L -> L, R -> R)
    Stereo1To1,
    /// Swap Left and Right channels (L -> R, R -> L)
    SwapLeftRight,
    /// Stereo (2) -> Stereo (2): Left output to both L & R inputs
    StereoLeftToBoth,
    /// Stereo (2) -> Stereo (2): Right output to both L & R inputs
    StereoRightToBoth,
    /// Stereo (2) -> Stereo (2): Both L & R outputs to both L & R inputs (mono sum)
    StereoMonoSum,
    /// Mono (1) -> Surround/Multi (>2): route mono to Front Left & Right (or Ch 1+2)
    MonoToFrontStereo,
    /// Mono (1) -> Surround (>2): route mono to Front Center (FC)
    MonoToCenter,
    /// Mono (1) -> Surround/Multi (>2): route mono to all non-LFE input channels
    MonoToAllChannels,
    /// Stereo (2) -> Surround/Multi (>2): route L/R to Front Left & Right (or Ch 1+2)
    StereoToFront,
    /// Stereo (2) -> Surround/Multi (>2): upmix/clone L to all left channels (+FC) and R to all right channels (+FC)
    StereoUpmixSurround,
    /// Surround/Multi (>2) -> Stereo (2): route Front Left & Right (or Ch 1+2) to Stereo L/R
    SurroundFrontToStereo,
    /// Surround/Multi (>2) -> Stereo (2): downmix all left channels (+FC) to L and right channels (+FC) to R
    SurroundDownmixToStereo,
    /// Surround/Multi (>2) -> Mono (1): route Front Left & Right (or Ch 1+2) to Mono input
    SurroundFrontToMono,
    /// Surround (>2) -> Mono (1): route Front Center (FC) to Mono input
    CenterToMono,
    /// Surround/Multi (>2) -> Mono (1): sum all non-LFE output channels into Mono input
    AllChannelsToMono,
    /// Surround/Multi (>2) -> Surround/Multi (>2): match channels 1:1 by channel name or index
    MultiChannel1To1,
    /// Surround/Multi (>2) -> Surround/Multi (>2): connect Front Stereo (FL/FR or Ch 1+2) only
    MultiChannelFrontOnly,
    /// Surround/Multi (>2) -> Surround/Multi (>2): swap all Left/Right channel pairs
    MultiChannelSwapLeftRight,
    /// Disconnect all port links between the two nodes
    DisconnectAll,
}

impl RoutingPreset {
    pub fn label(&self) -> &'static str {
        match self {
            RoutingPreset::Mono1To1 => "Mono 1:1",
            RoutingPreset::MonoToStereoBoth => "Mono → Both (L+R)",
            RoutingPreset::MonoToLeftOnly => "Mono → Left Only",
            RoutingPreset::MonoToRightOnly => "Mono → Right Only",
            RoutingPreset::StereoToMonoBoth => "Stereo (L+R) → Mono",
            RoutingPreset::LeftOnlyToMono => "Left Only → Mono",
            RoutingPreset::RightOnlyToMono => "Right Only → Mono",
            RoutingPreset::Stereo1To1 => "Stereo 1:1",
            RoutingPreset::SwapLeftRight => "Swap L ↔ R",
            RoutingPreset::StereoLeftToBoth => "Left → Both (L+R)",
            RoutingPreset::StereoRightToBoth => "Right → Both (L+R)",
            RoutingPreset::StereoMonoSum => "Mono Sum (L+R)",
            RoutingPreset::MonoToFrontStereo => "Mono → Front (L+R)",
            RoutingPreset::MonoToCenter => "Mono → Center (FC)",
            RoutingPreset::MonoToAllChannels => "Mono → All Channels",
            RoutingPreset::StereoToFront => "Stereo → Front (L/R)",
            RoutingPreset::StereoUpmixSurround => "Upmix → Surround",
            RoutingPreset::SurroundFrontToStereo => "Front (L/R) → Stereo",
            RoutingPreset::SurroundDownmixToStereo => "Downmix → Stereo",
            RoutingPreset::SurroundFrontToMono => "Front (L+R) → Mono",
            RoutingPreset::CenterToMono => "Center (FC) → Mono",
            RoutingPreset::AllChannelsToMono => "All Channels → Mono",
            RoutingPreset::MultiChannel1To1 => "Match Channels 1:1",
            RoutingPreset::MultiChannelFrontOnly => "Front Stereo Only",
            RoutingPreset::MultiChannelSwapLeftRight => "Swap L ↔ R Channels",
            RoutingPreset::DisconnectAll => "Disconnect All",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeCategory {
    AudioSource,
    AudioOutputStream,
    AudioDuplex,
    AudioInputStream,
    AudioSink,
    Midi,
    Video,
    Internal,
}

impl NodeCategory {
    pub fn label(&self) -> &'static str {
        match self {
            NodeCategory::AudioSource => "Audio Source",
            NodeCategory::AudioOutputStream => "Playback Stream",
            NodeCategory::AudioDuplex => "Audio Duplex / Filter",
            NodeCategory::AudioInputStream => "Recording Stream",
            NodeCategory::AudioSink => "Audio Sink",
            NodeCategory::Midi => "MIDI",
            NodeCategory::Video => "Video",
            NodeCategory::Internal => "Internal / System",
        }
    }

    pub fn column_index(&self) -> usize {
        match self {
            NodeCategory::AudioSource => 0,
            NodeCategory::AudioOutputStream => 0,
            NodeCategory::AudioDuplex => 1,
            NodeCategory::Midi => 1,
            NodeCategory::Video => 1,
            NodeCategory::AudioInputStream => 2,
            NodeCategory::AudioSink => 2,
            NodeCategory::Internal => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PwParamValue {
    Bool(bool),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    String(String),
    FloatArray(Vec<f32>),
    DisplayOnly(String),
}

impl PwParamValue {
    pub fn display_value(&self) -> String {
        match self {
            PwParamValue::Bool(b) => b.to_string(),
            PwParamValue::Int(i) => i.to_string(),
            PwParamValue::Long(l) => l.to_string(),
            PwParamValue::Float(f) => format!("{f:.3}"),
            PwParamValue::Double(d) => format!("{d:.4}"),
            PwParamValue::String(s) => s.trim_end_matches('\0').to_string(),
            PwParamValue::FloatArray(vals) => {
                let inner: Vec<String> = vals.iter().map(|v| format!("{v:.3}")).collect();
                format!("[{}]", inner.join(", "))
            }
            PwParamValue::DisplayOnly(s) => s.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PwNodeParam {
    /// SPA param category name (e.g. "Props", "Format", "Latency", "ProcessLatency")
    pub param_type: String,
    /// Raw SPA property key ID (e.g. `SPA_PROP_mute`, `SPA_PROP_volume`, `SPA_PROP_rate`, etc.)
    pub key_id: u32,
    /// Human-readable property key name (e.g. "mute", "volume", "channelVolumes", "latencyOffsetNsec")
    pub name: String,
    /// Parsed parameter value
    pub value: PwParamValue,
    /// Whether this parameter is read-only (only `Props` without `READONLY` flag can be modified via `set_param`)
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PwPort {
    pub id: u32,
    pub object_serial: u64,
    pub node_id: u32,
    pub node_serial: u64,
    pub name: String,
    pub channel: String,
    pub direction: PortDirection,
    pub is_monitor: bool,
    pub props: BTreeMap<String, String>,
    pub params: Vec<PwNodeParam>,
}

impl PwPort {
    pub fn display_label(&self) -> String {
        if !self.channel.is_empty() && self.channel != "UNK" {
            if self.is_monitor {
                format!("{} (mon)", self.channel)
            } else {
                self.channel.clone()
            }
        } else if !self.name.is_empty() {
            self.name.clone()
        } else {
            format!("Port serial #{}", self.object_serial)
        }
    }

    pub fn channel_sort_key(&self) -> (u8, String) {
        let base = match self.channel.as_str() {
            "MONO" => 0,
            "FL" => 1,
            "FR" => 2,
            "FC" => 3,
            "LFE" => 4,
            "RL" => 5,
            "RR" => 6,
            "SL" => 7,
            "SR" => 8,
            "FLC" => 9,
            "FRC" => 10,
            _ => 20,
        };
        let mon_offset = if self.is_monitor { 50 } else { 0 };
        (base + mon_offset, self.name.clone())
    }

    pub fn is_left_channel(&self) -> bool {
        matches!(self.channel.as_str(), "FL" | "RL" | "SL" | "FLC" | "L")
    }

    pub fn is_right_channel(&self) -> bool {
        matches!(self.channel.as_str(), "FR" | "RR" | "SR" | "FRC" | "R")
    }

    pub fn is_center_channel(&self) -> bool {
        matches!(self.channel.as_str(), "FC" | "RC")
    }

    pub fn is_lfe_channel(&self) -> bool {
        self.channel == "LFE"
    }
}

#[derive(Debug, Clone)]
pub struct PwNode {
    pub id: u32,
    pub object_serial: u64,
    pub name: String,
    pub description: String,
    pub app_name: Option<String>,
    pub media_name: Option<String>,
    pub media_class: String,
    pub category: NodeCategory,
    pub state: String,
    pub input_ports: Vec<PwPort>,
    pub output_ports: Vec<PwPort>,
    /// Perceptual master volume (`0.0..=1.5`, where `1.0` = 100%)
    pub volume: Option<f32>,
    /// Mute state from `SPA_PROP_mute`
    pub muted: Option<bool>,
    /// Perceptual per-channel volumes (`0.0..=1.5`, cube-root of `SPA_PROP_channelVolumes`)
    pub channel_volumes: Vec<f32>,
    /// Channel names from `SPA_PROP_channelMap` (e.g. `["FL", "FR"]`)
    pub channel_names: Vec<String>,
    /// Raw PipeWire dictionary properties (`global.props` + `NodeInfoRef::props()`)
    pub props: BTreeMap<String, String>,
    /// Enumerated / subscribed SPA parameters (`Props`, `Format`, `Latency`, etc.)
    pub params: Vec<PwNodeParam>,
    /// Optional PipeWire device ID (`device.id`) backing this node
    pub device_id: Option<u32>,
    /// Optional object serial of the backing `PwDevice`
    pub device_serial: Option<u64>,
}

impl PwNode {
    pub fn title(&self) -> String {
        if let Some(app) = &self.app_name {
            if !app.is_empty() && app != &self.description && !self.description.is_empty() {
                return format!("{} — {}", app, self.description);
            }
            if !app.is_empty() {
                return app.clone();
            }
        }
        if !self.description.is_empty() {
            return self.description.clone();
        }
        if !self.name.is_empty() {
            return self.name.clone();
        }
        format!("Node serial #{}", self.object_serial)
    }

    pub fn subtitle(&self) -> String {
        if let Some(media) = &self.media_name {
            if !media.is_empty() && media != &self.title() {
                return media.clone();
            }
        }
        if !self.media_class.is_empty() {
            return self.media_class.clone();
        }
        self.name.clone()
    }

    pub fn supports_audio_controls(&self) -> bool {
        self.volume.is_some()
            || self.muted.is_some()
            || !self.channel_volumes.is_empty()
            || matches!(
                self.category,
                NodeCategory::AudioSource
                    | NodeCategory::AudioOutputStream
                    | NodeCategory::AudioDuplex
                    | NodeCategory::AudioInputStream
                    | NodeCategory::AudioSink
            )
    }

    pub fn effective_volume(&self) -> f32 {
        if let Some(v) = self.volume {
            v.clamp(0.0, 1.5)
        } else if !self.channel_volumes.is_empty() {
            let sum: f32 = self.channel_volumes.iter().copied().sum();
            (sum / self.channel_volumes.len() as f32).clamp(0.0, 1.5)
        } else {
            1.0
        }
    }

    /// Returns `true` when this node has 2 or more channel volumes that differ by more than `0.5%`.
    pub fn has_unequal_channel_volumes(&self) -> bool {
        let Some(&first) = self.channel_volumes.first() else {
            return false;
        };
        self.channel_volumes
            .iter()
            .skip(1)
            .any(|&v| (v - first).abs() > 0.005)
    }

    pub fn channel_label(&self, idx: usize) -> String {
        if let Some(ch) = self.channel_names.get(idx) {
            if !ch.is_empty() {
                return ch.clone();
            }
        }
        let ports = self.default_output_ports();
        if let Some(p) = ports.get(idx) {
            return p.display_label();
        }
        if let Some(p) = self.input_ports.get(idx) {
            return p.display_label();
        }
        format!("Ch {}", idx + 1)
    }

    /// Returns output ports suitable for default auto-wiring (prefers non-monitor ports unless only monitor ports exist).
    pub fn default_output_ports(&self) -> Vec<&PwPort> {
        let non_mon: Vec<&PwPort> = self.output_ports.iter().filter(|p| !p.is_monitor).collect();
        if !non_mon.is_empty() {
            non_mon
        } else {
            self.output_ports.iter().collect()
        }
    }

    pub fn output_layout(&self) -> ChannelLayout {
        let ports = self.default_output_ports();
        ChannelLayout::from_ports(&ports)
    }

    pub fn input_layout(&self) -> ChannelLayout {
        let ports: Vec<&PwPort> = self.input_ports.iter().collect();
        ChannelLayout::from_ports(&ports)
    }

    pub fn output_layout_label(&self) -> String {
        let ports = self.default_output_ports();
        ChannelLayout::from_ports(&ports).label(&ports)
    }

    pub fn input_layout_label(&self) -> String {
        let ports: Vec<&PwPort> = self.input_ports.iter().collect();
        ChannelLayout::from_ports(&ports).label(&ports)
    }

    /// Returns true if this node matches the search query over its name or description.
    pub fn matches_search(&self, query: &str) -> bool {
        matches_search(&self.name, &self.description, query)
    }
}

/// Returns true if `query` (when trimmed) is empty, or if `name` or `description`
/// contains the query as a case-insensitive substring.
pub fn matches_search(name: &str, description: &str, query: &str) -> bool {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return true;
    }
    let q_lower = trimmed.to_lowercase();
    name.to_lowercase().contains(&q_lower) || description.to_lowercase().contains(&q_lower)
}

/// Computes the set of node serials in `graph` that do NOT match `query`.
/// If `query` (trimmed) is empty, returns an empty set (no nodes are dimmed).
pub fn compute_dimmed_nodes(graph: &PwGraph, query: &str) -> HashSet<u64> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return HashSet::new();
    }
    graph
        .nodes
        .iter()
        .filter_map(|(&serial, node)| {
            if !matches_search(&node.name, &node.description, trimmed) {
                Some(serial)
            } else {
                None
            }
        })
        .collect()
}

/// Returns true if a collapsed edge is dimmed under an active search query.
/// A link is dimmed if search is active and either of its endpoint nodes is in `dimmed_nodes`.
pub fn is_link_dimmed(
    edge: &EdgeKey,
    dimmed_nodes: &HashSet<u64>,
    search_active: bool,
) -> bool {
    search_active
        && (dimmed_nodes.contains(&edge.output_node_serial)
            || dimmed_nodes.contains(&edge.input_node_serial))
}

/// Computes the set of collapsed edge keys in `graph` that are dimmed.
/// A collapsed edge is dimmed if search is active and either the output node or input node
/// is in `dimmed_nodes`.
pub fn compute_dimmed_links(
    graph: &PwGraph,
    dimmed_nodes: &HashSet<u64>,
    search_active: bool,
) -> HashSet<EdgeKey> {
    if !search_active {
        return HashSet::new();
    }
    graph
        .collapsed_edges()
        .into_iter()
        .filter_map(|edge| {
            if is_link_dimmed(&edge.key, dimmed_nodes, true) {
                Some(edge.key)
            } else {
                None
            }
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct PwLink {
    pub id: u32,
    pub object_serial: u64,
    pub output_node_id: u32,
    pub output_node_serial: u64,
    pub output_port_id: u32,
    pub output_port_serial: u64,
    pub input_node_id: u32,
    pub input_node_serial: u64,
    pub input_port_id: u32,
    pub input_port_serial: u64,
    pub state: String,
    pub props: BTreeMap<String, String>,
}

/// Represents a collapsed wire between an output node and an input node, identified by `object_serial`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EdgeKey {
    pub output_node_serial: u64,
    pub input_node_serial: u64,
}

#[derive(Debug, Clone)]
pub struct CollapsedEdge {
    pub key: EdgeKey,
    pub links: Vec<PwLink>,
    pub summary: String,
    pub is_default: bool,
}

/// The PipeWire graph snapshot used by the GUI.
/// All maps are keyed by `object_serial: u64` rather than global `id: u32`.
#[derive(Debug, Clone, Default)]
pub struct PwGraph {
    pub nodes: BTreeMap<u64, PwNode>,
    pub ports: BTreeMap<u64, PwPort>,
    pub links: BTreeMap<u64, PwLink>,
    pub devices: BTreeMap<u64, PwDevice>,
}

impl PwGraph {
    /// Returns a reference to the `PwDevice` associated with the given node serial, if any.
    pub fn device_for_node(&self, node_serial: u64) -> Option<&PwDevice> {
        let node = self.nodes.get(&node_serial)?;
        if let Some(ds) = node.device_serial {
            if let Some(dev) = self.devices.get(&ds) {
                return Some(dev);
            }
        }
        if let Some(dev_id) = node.device_id.or_else(|| {
            node.props
                .get("device.id")
                .and_then(|s| s.parse::<u32>().ok())
        }) {
            return self.devices.values().find(|d| d.id == dev_id);
        }
        None
    }

    /// Returns a mutable reference to the `PwDevice` associated with the given node serial, if any.
    pub fn device_for_node_mut(&mut self, node_serial: u64) -> Option<&mut PwDevice> {
        let node = self.nodes.get(&node_serial)?;
        let target_serial = node.device_serial.or_else(|| {
            let dev_id = node.device_id.or_else(|| {
                node.props
                    .get("device.id")
                    .and_then(|s| s.parse::<u32>().ok())
            })?;
            self.devices.values().find(|d| d.id == dev_id).map(|d| d.object_serial)
        })?;
        self.devices.get_mut(&target_serial)
    }

    /// Synchronizes `node_serial` on ports, `*_serial` on links, device serials on nodes, and rebuilds each node's
    /// `input_ports` and `output_ports` lists using the current global ID -> `object.serial` mappings.
    pub fn resolve_serials_and_rebuild_ports(&mut self) {
        let node_id_to_serial: HashMap<u32, u64> = self
            .nodes
            .values()
            .map(|n| (n.id, n.object_serial))
            .collect();
        let port_id_to_serial: HashMap<u32, u64> = self
            .ports
            .values()
            .map(|p| (p.id, p.object_serial))
            .collect();
        let device_id_to_serial: HashMap<u32, u64> = self
            .devices
            .values()
            .map(|d| (d.id, d.object_serial))
            .collect();

        for node in self.nodes.values_mut() {
            if node.device_id.is_none() {
                if let Some(dev_id) = node
                    .props
                    .get("device.id")
                    .and_then(|s| s.parse::<u32>().ok())
                {
                    node.device_id = Some(dev_id);
                }
            }
            if let Some(dev_id) = node.device_id {
                if let Some(&dev_serial) = device_id_to_serial.get(&dev_id) {
                    node.device_serial = Some(dev_serial);
                }
            }
        }

        for node in self.nodes.values_mut() {
            node.input_ports.clear();
            node.output_ports.clear();
        }

        for port in self.ports.values_mut() {
            if let Some(&ns) = node_id_to_serial.get(&port.node_id) {
                port.node_serial = ns;
                if let Some(node) = self.nodes.get_mut(&ns) {
                    match port.direction {
                        PortDirection::Input => node.input_ports.push(port.clone()),
                        PortDirection::Output => node.output_ports.push(port.clone()),
                    }
                }
            }
        }

        for node in self.nodes.values_mut() {
            node.input_ports.sort_by_key(|p| p.channel_sort_key());
            node.output_ports.sort_by_key(|p| p.channel_sort_key());
        }

        for link in self.links.values_mut() {
            if let Some(&ons) = node_id_to_serial.get(&link.output_node_id) {
                link.output_node_serial = ons;
            }
            if let Some(&ops) = port_id_to_serial.get(&link.output_port_id) {
                link.output_port_serial = ops;
            }
            if let Some(&ins) = node_id_to_serial.get(&link.input_node_id) {
                link.input_node_serial = ins;
            }
            if let Some(&ips) = port_id_to_serial.get(&link.input_port_id) {
                link.input_port_serial = ips;
            }
        }
    }

    /// Computes collapsed edges (one wire per `(output_node_serial, input_node_serial)` pair).
    pub fn collapsed_edges(&self) -> Vec<CollapsedEdge> {
        let mut grouped: BTreeMap<(u64, u64), Vec<PwLink>> = BTreeMap::new();
        for link in self.links.values() {
            if self.nodes.contains_key(&link.output_node_serial)
                && self.nodes.contains_key(&link.input_node_serial)
            {
                grouped
                    .entry((link.output_node_serial, link.input_node_serial))
                    .or_default()
                    .push(link.clone());
            }
        }

        grouped
            .into_iter()
            .map(|((output_node_serial, input_node_serial), links)| {
                let summary = self.describe_links(&links);
                let default_mapping =
                    self.compute_default_port_mapping(output_node_serial, input_node_serial);
                let active_set: BTreeSet<(u64, u64)> = links
                    .iter()
                    .map(|l| (l.output_port_serial, l.input_port_serial))
                    .collect();
                let default_set: BTreeSet<(u64, u64)> = default_mapping.into_iter().collect();
                let is_default = !active_set.is_empty() && active_set == default_set;

                CollapsedEdge {
                    key: EdgeKey {
                        output_node_serial,
                        input_node_serial,
                    },
                    links,
                    summary,
                    is_default,
                }
            })
            .collect()
    }

    /// Computes left-to-right column indices by signal flow depth (R4):
    /// - Column 0: Sources (in-degree 0 in signal flow, excluding pure unconnected sinks).
    /// - Column 1: Sinks fed by Column 0, PLUS sinks with no connected source (unconnected sinks).
    /// - Column 2: Sinks fed by Column 1.
    /// - Column k: Sinks fed by Column k-1 (depth(v) = max_{u in incoming(v)}(depth(u)) + 1).
    /// - Fallback for unreachable / unassigned nodes: Column 0.
    pub fn signal_flow_columns(
        &self,
        visible_nodes: &HashSet<u64>,
    ) -> BTreeMap<u64, usize> {
        compute_signal_flow_columns(self, visible_nodes)
    }

    /// Generates a clear description of the port mapping in a collapsed wire,
    /// e.g. "Stereo → Stereo", "Mono → Stereo", "Mono → Mono", "Stereo (Swap L/R)", etc.
    pub fn describe_links(&self, links: &[PwLink]) -> String {
        if links.is_empty() {
            return "No links".to_string();
        }

        let mut out_port_serials = Vec::new();
        let mut in_port_serials = Vec::new();
        let mut pairs = Vec::new();

        for link in links {
            let out_ch = self
                .ports
                .get(&link.output_port_serial)
                .map(|p| p.channel.as_str())
                .unwrap_or("");
            let in_ch = self
                .ports
                .get(&link.input_port_serial)
                .map(|p| p.channel.as_str())
                .unwrap_or("");
            if !out_port_serials.contains(&link.output_port_serial) {
                out_port_serials.push(link.output_port_serial);
            }
            if !in_port_serials.contains(&link.input_port_serial) {
                in_port_serials.push(link.input_port_serial);
            }
            pairs.push((out_ch, in_ch));
        }

        match (out_port_serials.len(), in_port_serials.len(), links.len()) {
            (1, 1, 1) => {
                let (o, i) = pairs[0];
                match (o, i) {
                    ("MONO", "FL") => "Mono → Left".to_string(),
                    ("MONO", "FR") => "Mono → Right".to_string(),
                    ("FL", "MONO") => "Left → Mono".to_string(),
                    ("FR", "MONO") => "Right → Mono".to_string(),
                    ("MONO", "FC") => "Mono → Center".to_string(),
                    ("FC", "MONO") => "Center → Mono".to_string(),
                    ("FL", "FL") => "Left → Left".to_string(),
                    ("FR", "FR") => "Right → Right".to_string(),
                    ("FL", "FR") => "Left → Right".to_string(),
                    ("FR", "FL") => "Right → Left".to_string(),
                    _ => "Mono → Mono".to_string(),
                }
            }
            (1, 2, 2) => {
                let (o, _) = pairs[0];
                match o {
                    "FL" => "Left → Stereo".to_string(),
                    "FR" => "Right → Stereo".to_string(),
                    _ => "Mono → Stereo".to_string(),
                }
            }
            (2, 1, 2) => "Stereo → Mono".to_string(),
            (2, 2, 2) => {
                let has_fl_fl = pairs.iter().any(|&(o, i)| o == "FL" && i == "FL");
                let has_fr_fr = pairs.iter().any(|&(o, i)| o == "FR" && i == "FR");
                let has_fl_fr = pairs.iter().any(|&(o, i)| o == "FL" && i == "FR");
                let has_fr_fl = pairs.iter().any(|&(o, i)| o == "FR" && i == "FL");

                if has_fl_fl && has_fr_fr {
                    "Stereo → Stereo".to_string()
                } else if has_fl_fr && has_fr_fl {
                    "Stereo (Swap L/R)".to_string()
                } else {
                    "Stereo → Stereo".to_string()
                }
            }
            (2, 2, 4) => "Stereo Mono Sum".to_string(),
            (_, _, n) => format!(
                "{} ch ({} links)",
                out_port_serials.len().max(in_port_serials.len()),
                n
            ),
        }
    }

    /// Returns the list of `RoutingPreset`s appropriate for the output and input channel layouts
    /// of `(output_node_serial, input_node_serial)`.
    ///
    /// For `Mono -> Mono` (1x1), returns an empty list because there is only a single port pair
    /// in the matrix and presets are redundant.
    pub fn available_presets(
        &self,
        output_node_serial: u64,
        input_node_serial: u64,
    ) -> Vec<RoutingPreset> {
        let (Some(out_node), Some(in_node)) = (
            self.nodes.get(&output_node_serial),
            self.nodes.get(&input_node_serial),
        ) else {
            return Vec::new();
        };

        let out_ports = out_node.default_output_ports();
        let in_ports: Vec<&PwPort> = in_node.input_ports.iter().collect();

        let out_layout = ChannelLayout::from_ports(&out_ports);
        let in_layout = ChannelLayout::from_ports(&in_ports);

        let out_has_fc = out_ports.iter().any(|p| p.is_center_channel());
        let in_has_fc = in_ports.iter().any(|p| p.is_center_channel());

        match (out_layout, in_layout) {
            (ChannelLayout::Empty, _) | (_, ChannelLayout::Empty) => Vec::new(),
            // Mono -> Mono has only 1 output port and 1 input port (1x1 cell in matrix);
            // quick routing presets don't make sense when there are no channel routing choices.
            (ChannelLayout::Mono, ChannelLayout::Mono) => Vec::new(),

            // Mono Source -> Stereo Target
            (ChannelLayout::Mono, ChannelLayout::Stereo) => vec![
                RoutingPreset::MonoToStereoBoth,
                RoutingPreset::MonoToLeftOnly,
                RoutingPreset::MonoToRightOnly,
                RoutingPreset::DisconnectAll,
            ],

            // Stereo Source -> Mono Target
            (ChannelLayout::Stereo, ChannelLayout::Mono) => vec![
                RoutingPreset::StereoToMonoBoth,
                RoutingPreset::LeftOnlyToMono,
                RoutingPreset::RightOnlyToMono,
                RoutingPreset::DisconnectAll,
            ],

            // Stereo Source -> Stereo Target
            (ChannelLayout::Stereo, ChannelLayout::Stereo) => vec![
                RoutingPreset::Stereo1To1,
                RoutingPreset::SwapLeftRight,
                RoutingPreset::StereoLeftToBoth,
                RoutingPreset::StereoRightToBoth,
                RoutingPreset::StereoMonoSum,
                RoutingPreset::DisconnectAll,
            ],

            // Mono Source -> Surround / Multi-Channel Target
            (
                ChannelLayout::Mono,
                ChannelLayout::Surround(_) | ChannelLayout::MultiChannel(_),
            ) => {
                let mut presets = vec![RoutingPreset::MonoToFrontStereo];
                if in_has_fc {
                    presets.push(RoutingPreset::MonoToCenter);
                }
                presets.push(RoutingPreset::MonoToAllChannels);
                presets.push(RoutingPreset::DisconnectAll);
                presets
            }

            // Surround / Multi-Channel Source -> Mono Target
            (
                ChannelLayout::Surround(_) | ChannelLayout::MultiChannel(_),
                ChannelLayout::Mono,
            ) => {
                let mut presets = vec![RoutingPreset::SurroundFrontToMono];
                if out_has_fc {
                    presets.push(RoutingPreset::CenterToMono);
                }
                presets.push(RoutingPreset::AllChannelsToMono);
                presets.push(RoutingPreset::DisconnectAll);
                presets
            }

            // Stereo Source -> Surround / Multi-Channel Target
            (
                ChannelLayout::Stereo,
                ChannelLayout::Surround(_) | ChannelLayout::MultiChannel(_),
            ) => vec![
                RoutingPreset::StereoToFront,
                RoutingPreset::StereoUpmixSurround,
                RoutingPreset::SwapLeftRight,
                RoutingPreset::DisconnectAll,
            ],

            // Surround / Multi-Channel Source -> Stereo Target
            (
                ChannelLayout::Surround(_) | ChannelLayout::MultiChannel(_),
                ChannelLayout::Stereo,
            ) => vec![
                RoutingPreset::SurroundFrontToStereo,
                RoutingPreset::SurroundDownmixToStereo,
                RoutingPreset::SwapLeftRight,
                RoutingPreset::DisconnectAll,
            ],

            // Surround / Multi-Channel Source -> Surround / Multi-Channel Target
            (
                ChannelLayout::Surround(_) | ChannelLayout::MultiChannel(_),
                ChannelLayout::Surround(_) | ChannelLayout::MultiChannel(_),
            ) => vec![
                RoutingPreset::MultiChannel1To1,
                RoutingPreset::MultiChannelFrontOnly,
                RoutingPreset::MultiChannelSwapLeftRight,
                RoutingPreset::DisconnectAll,
            ],
        }
    }

    /// Computes the exact target `(output_port_serial, input_port_serial)` links for a given `RoutingPreset`.
    pub fn compute_preset_port_mapping(
        &self,
        output_node_serial: u64,
        input_node_serial: u64,
        preset: RoutingPreset,
    ) -> Vec<(u64, u64)> {
        if preset == RoutingPreset::DisconnectAll {
            return Vec::new();
        }

        let (Some(out_node), Some(in_node)) = (
            self.nodes.get(&output_node_serial),
            self.nodes.get(&input_node_serial),
        ) else {
            return Vec::new();
        };

        let out_ports = out_node.default_output_ports();
        let in_ports: Vec<&PwPort> = in_node.input_ports.iter().collect();

        if out_ports.is_empty() || in_ports.is_empty() {
            return Vec::new();
        }

        let (out_fl, out_fr) = Self::find_front_lr(&out_ports);
        let (in_fl, in_fr) = Self::find_front_lr(&in_ports);

        let mut pairs = Vec::new();

        match preset {
            RoutingPreset::DisconnectAll => {}

            RoutingPreset::Mono1To1 => {
                pairs.push((out_ports[0].object_serial, in_ports[0].object_serial));
            }

            RoutingPreset::MonoToStereoBoth | RoutingPreset::MonoToFrontStereo => {
                let mono_out = out_ports[0].object_serial;
                if let (Some(l), Some(r)) = (in_fl, in_fr) {
                    pairs.push((mono_out, l));
                    if r != l {
                        pairs.push((mono_out, r));
                    }
                } else {
                    for in_p in in_ports.iter().take(2) {
                        pairs.push((mono_out, in_p.object_serial));
                    }
                }
            }

            RoutingPreset::MonoToLeftOnly => {
                let mono_out = out_ports[0].object_serial;
                if let Some(l) = in_fl {
                    pairs.push((mono_out, l));
                }
            }

            RoutingPreset::MonoToRightOnly => {
                let mono_out = out_ports[0].object_serial;
                if let Some(r) = in_fr.or(in_fl) {
                    pairs.push((mono_out, r));
                }
            }

            RoutingPreset::MonoToCenter => {
                let mono_out = out_ports[0].object_serial;
                if let Some(fc) = in_ports.iter().find(|p| p.is_center_channel()) {
                    pairs.push((mono_out, fc.object_serial));
                }
            }

            RoutingPreset::MonoToAllChannels => {
                let mono_out = out_ports[0].object_serial;
                let non_lfe: Vec<&PwPort> = in_ports
                    .iter()
                    .copied()
                    .filter(|p| !p.is_lfe_channel())
                    .collect();
                let targets = if non_lfe.is_empty() {
                    &in_ports
                } else {
                    &non_lfe
                };
                for in_p in targets {
                    pairs.push((mono_out, in_p.object_serial));
                }
            }

            RoutingPreset::StereoToMonoBoth | RoutingPreset::SurroundFrontToMono => {
                let mono_in = in_ports[0].object_serial;
                if let (Some(l), Some(r)) = (out_fl, out_fr) {
                    pairs.push((l, mono_in));
                    if r != l {
                        pairs.push((r, mono_in));
                    }
                } else {
                    for out_p in out_ports.iter().take(2) {
                        pairs.push((out_p.object_serial, mono_in));
                    }
                }
            }

            RoutingPreset::LeftOnlyToMono => {
                let mono_in = in_ports[0].object_serial;
                if let Some(l) = out_fl {
                    pairs.push((l, mono_in));
                }
            }

            RoutingPreset::RightOnlyToMono => {
                let mono_in = in_ports[0].object_serial;
                if let Some(r) = out_fr.or(out_fl) {
                    pairs.push((r, mono_in));
                }
            }

            RoutingPreset::CenterToMono => {
                let mono_in = in_ports[0].object_serial;
                if let Some(fc) = out_ports.iter().find(|p| p.is_center_channel()) {
                    pairs.push((fc.object_serial, mono_in));
                }
            }

            RoutingPreset::AllChannelsToMono => {
                let mono_in = in_ports[0].object_serial;
                let non_lfe: Vec<&PwPort> = out_ports
                    .iter()
                    .copied()
                    .filter(|p| !p.is_lfe_channel())
                    .collect();
                let sources = if non_lfe.is_empty() {
                    &out_ports
                } else {
                    &non_lfe
                };
                for out_p in sources {
                    pairs.push((out_p.object_serial, mono_in));
                }
            }

            RoutingPreset::Stereo1To1
            | RoutingPreset::StereoToFront
            | RoutingPreset::SurroundFrontToStereo
            | RoutingPreset::MultiChannelFrontOnly => {
                if let (Some(ol), Some(or), Some(il), Some(ir)) = (out_fl, out_fr, in_fl, in_fr) {
                    pairs.push((ol, il));
                    pairs.push((or, ir));
                } else {
                    for (o, i) in out_ports.iter().zip(in_ports.iter()).take(2) {
                        pairs.push((o.object_serial, i.object_serial));
                    }
                }
            }

            RoutingPreset::SwapLeftRight => {
                if let (Some(ol), Some(or), Some(il), Some(ir)) = (out_fl, out_fr, in_fl, in_fr) {
                    pairs.push((ol, ir));
                    pairs.push((or, il));
                }
            }

            RoutingPreset::StereoLeftToBoth => {
                if let (Some(ol), Some(il), Some(ir)) = (out_fl, in_fl, in_fr) {
                    pairs.push((ol, il));
                    pairs.push((ol, ir));
                }
            }

            RoutingPreset::StereoRightToBoth => {
                if let (Some(or), Some(il), Some(ir)) = (out_fr, in_fl, in_fr) {
                    pairs.push((or, il));
                    pairs.push((or, ir));
                }
            }

            RoutingPreset::StereoMonoSum => {
                if let (Some(ol), Some(or), Some(il), Some(ir)) = (out_fl, out_fr, in_fl, in_fr) {
                    pairs.push((ol, il));
                    pairs.push((ol, ir));
                    pairs.push((or, il));
                    pairs.push((or, ir));
                }
            }

            RoutingPreset::StereoUpmixSurround => {
                if let (Some(ol), Some(or)) = (out_fl, out_fr) {
                    let has_lr_tags = in_ports
                        .iter()
                        .any(|p| p.is_left_channel() || p.is_right_channel());
                    if has_lr_tags {
                        for in_p in &in_ports {
                            if in_p.is_left_channel() {
                                pairs.push((ol, in_p.object_serial));
                            } else if in_p.is_right_channel() {
                                pairs.push((or, in_p.object_serial));
                            } else if in_p.is_center_channel() {
                                pairs.push((ol, in_p.object_serial));
                                pairs.push((or, in_p.object_serial));
                            }
                        }
                    } else {
                        for (idx, in_p) in in_ports.iter().enumerate() {
                            let src = if idx % 2 == 0 { ol } else { or };
                            pairs.push((src, in_p.object_serial));
                        }
                    }
                }
            }

            RoutingPreset::SurroundDownmixToStereo => {
                if let (Some(il), Some(ir)) = (in_fl, in_fr) {
                    let has_lr_tags = out_ports
                        .iter()
                        .any(|p| p.is_left_channel() || p.is_right_channel());
                    if has_lr_tags {
                        for out_p in &out_ports {
                            if out_p.is_left_channel() {
                                pairs.push((out_p.object_serial, il));
                            } else if out_p.is_right_channel() {
                                pairs.push((out_p.object_serial, ir));
                            } else if out_p.is_center_channel() {
                                pairs.push((out_p.object_serial, il));
                                pairs.push((out_p.object_serial, ir));
                            }
                        }
                    } else {
                        for (idx, out_p) in out_ports.iter().enumerate() {
                            let dst = if idx % 2 == 0 { il } else { ir };
                            pairs.push((out_p.object_serial, dst));
                        }
                    }
                }
            }

            RoutingPreset::MultiChannel1To1 => {
                for out_p in &out_ports {
                    if !out_p.channel.is_empty() && out_p.channel != "UNK" {
                        if let Some(in_p) = in_ports.iter().find(|ip| ip.channel == out_p.channel) {
                            pairs.push((out_p.object_serial, in_p.object_serial));
                        }
                    }
                }
                if pairs.is_empty() {
                    for (o, i) in out_ports.iter().zip(in_ports.iter()) {
                        pairs.push((o.object_serial, i.object_serial));
                    }
                }
            }

            RoutingPreset::MultiChannelSwapLeftRight => {
                for out_p in &out_ports {
                    let target_ch = match out_p.channel.as_str() {
                        "FL" => Some("FR"),
                        "FR" => Some("FL"),
                        "RL" => Some("RR"),
                        "RR" => Some("RL"),
                        "SL" => Some("SR"),
                        "SR" => Some("SL"),
                        "FLC" => Some("FRC"),
                        "FRC" => Some("FLC"),
                        "FC" => Some("FC"),
                        "LFE" => Some("LFE"),
                        _ => None,
                    };
                    if let Some(tch) = target_ch {
                        if let Some(in_p) = in_ports.iter().find(|ip| ip.channel == tch) {
                            pairs.push((out_p.object_serial, in_p.object_serial));
                        }
                    }
                }
                if pairs.is_empty() {
                    // Fallback for unnamed multi-channel: swap adjacent pairs (0<->1, 2<->3, ...)
                    let count = out_ports.len().min(in_ports.len());
                    let mut idx = 0;
                    while idx + 1 < count {
                        pairs.push((
                            out_ports[idx].object_serial,
                            in_ports[idx + 1].object_serial,
                        ));
                        pairs.push((
                            out_ports[idx + 1].object_serial,
                            in_ports[idx].object_serial,
                        ));
                        idx += 2;
                    }
                    if idx < count {
                        pairs.push((out_ports[idx].object_serial, in_ports[idx].object_serial));
                    }
                }
            }
        }

        // Deduplicate while preserving order
        let mut seen = BTreeSet::new();
        pairs.retain(|pair| seen.insert(*pair));
        pairs
    }

    /// Checks whether the currently active links between `output_node_serial` and `input_node_serial`
    /// exactly match `preset`.
    pub fn is_preset_active(
        &self,
        output_node_serial: u64,
        input_node_serial: u64,
        preset: RoutingPreset,
    ) -> bool {
        let active_set: BTreeSet<(u64, u64)> = self
            .links
            .values()
            .filter(|l| {
                l.output_node_serial == output_node_serial
                    && l.input_node_serial == input_node_serial
            })
            .map(|l| (l.output_port_serial, l.input_port_serial))
            .collect();

        let target_set: BTreeSet<(u64, u64)> = self
            .compute_preset_port_mapping(output_node_serial, input_node_serial, preset)
            .into_iter()
            .collect();

        active_set == target_set
    }

    fn find_front_lr(ports: &[&PwPort]) -> (Option<u64>, Option<u64>) {
        let fl = ports
            .iter()
            .find(|p| p.channel == "FL" || p.channel == "L")
            .or_else(|| ports.first())
            .map(|p| p.object_serial);
        let fr = ports
            .iter()
            .find(|p| p.channel == "FR" || p.channel == "R")
            .or_else(|| ports.get(1))
            .map(|p| p.object_serial);
        (fl, fr)
    }

    /// Computes the smart default `(output_port_serial, input_port_serial)` pairs when connecting two nodes.
    pub fn compute_default_port_mapping(
        &self,
        output_node_serial: u64,
        input_node_serial: u64,
    ) -> Vec<(u64, u64)> {
        let (Some(out_node), Some(in_node)) = (
            self.nodes.get(&output_node_serial),
            self.nodes.get(&input_node_serial),
        ) else {
            return Vec::new();
        };

        let out_ports = out_node.default_output_ports();
        let in_ports: Vec<&PwPort> = in_node.input_ports.iter().collect();

        let out_layout = ChannelLayout::from_ports(&out_ports);
        let in_layout = ChannelLayout::from_ports(&in_ports);

        if out_layout == ChannelLayout::Mono && in_layout == ChannelLayout::Mono {
            return self.compute_preset_port_mapping(
                output_node_serial,
                input_node_serial,
                RoutingPreset::Mono1To1,
            );
        }

        if let Some(&first_preset) = self
            .available_presets(output_node_serial, input_node_serial)
            .first()
        {
            if first_preset != RoutingPreset::DisconnectAll {
                return self.compute_preset_port_mapping(
                    output_node_serial,
                    input_node_serial,
                    first_preset,
                );
            }
        }

        Vec::new()
    }
}

/// Computes left-to-right column indices by signal flow depth (R4):
/// - Column 0: Sources (in-degree 0 in signal flow, excluding pure unconnected sinks).
/// - Column 1: Sinks fed by Column 0, PLUS sinks with no connected source (unconnected sinks).
/// - Column 2: Sinks fed by Column 1.
/// - Column k: Sinks fed by Column k-1 (depth(v) = max_{u in incoming(v)}(depth(u)) + 1).
/// - Fallback for unreachable / unassigned nodes: Column 0.
pub fn compute_signal_flow_columns(
    graph: &PwGraph,
    visible_nodes: &HashSet<u64>,
) -> BTreeMap<u64, usize> {
    if visible_nodes.is_empty() {
        return BTreeMap::new();
    }

    // Filter to active visible nodes that exist in graph
    let active_nodes: HashSet<u64> = visible_nodes
        .iter()
        .copied()
        .filter(|id| graph.nodes.contains_key(id))
        .collect();

    if active_nodes.is_empty() {
        return BTreeMap::new();
    }

    let mut orig_in_edges: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut orig_out_edges: HashMap<u64, Vec<u64>> = HashMap::new();
    for &serial in &active_nodes {
        orig_in_edges.insert(serial, Vec::new());
        orig_out_edges.insert(serial, Vec::new());
    }

    for edge in graph.collapsed_edges() {
        let u = edge.key.output_node_serial;
        let v = edge.key.input_node_serial;
        if active_nodes.contains(&u) && active_nodes.contains(&v) && u != v {
            orig_in_edges.entry(v).or_default().push(u);
            orig_out_edges.entry(u).or_default().push(v);
        }
    }

    // Sort neighbor lists for deterministic traversal across runs
    for list in orig_in_edges.values_mut() {
        list.sort();
    }
    for list in orig_out_edges.values_mut() {
        list.sort();
    }

    // ------------------------------------------------------------------------
    // Step 1: Cycle Detection via 3-Color DFS to identify back-edges
    // ------------------------------------------------------------------------
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Color {
        White,
        Gray,
        Black,
    }

    let mut color: HashMap<u64, Color> = active_nodes.iter().map(|&id| (id, Color::White)).collect();
    let mut back_edges: HashSet<(u64, u64)> = HashSet::new();

    fn dfs_visit(
        u: u64,
        out_edges: &HashMap<u64, Vec<u64>>,
        color: &mut HashMap<u64, Color>,
        back_edges: &mut HashSet<(u64, u64)>,
    ) {
        color.insert(u, Color::Gray);
        if let Some(neighbors) = out_edges.get(&u) {
            for &v in neighbors {
                match color.get(&v).copied().unwrap_or(Color::White) {
                    Color::Gray => {
                        // Back-edge detected: v is an active ancestor on current DFS path
                        back_edges.insert((u, v));
                    }
                    Color::White => {
                        dfs_visit(v, out_edges, color, back_edges);
                    }
                    Color::Black => {}
                }
            }
        }
        color.insert(u, Color::Black);
    }

    // Phase 1A: Start DFS from natural sources (in-degree 0 in full graph)
    let mut natural_sources: Vec<u64> = active_nodes
        .iter()
        .copied()
        .filter(|&id| orig_in_edges.get(&id).map_or(0, |v| v.len()) == 0)
        .collect();
    natural_sources.sort();

    for s in natural_sources {
        if color.get(&s) == Some(&Color::White) {
            dfs_visit(s, &orig_out_edges, &mut color, &mut back_edges);
        }
    }

    // Phase 1B: Handle disconnected cycle components (nodes not reached from natural sources)
    let mut remaining: Vec<u64> = active_nodes
        .iter()
        .copied()
        .filter(|&id| color.get(&id) == Some(&Color::White))
        .collect();

    remaining.sort_by(|&a, &b| {
        let cat_rank = |node_opt: Option<&PwNode>| -> usize {
            match node_opt.map(|n| n.category) {
                Some(NodeCategory::AudioSource) | Some(NodeCategory::AudioOutputStream) => 0,
                Some(NodeCategory::AudioDuplex) => 1,
                Some(NodeCategory::AudioSink) | Some(NodeCategory::AudioInputStream) => 2,
                _ => 3,
            }
        };
        let cat_a = cat_rank(graph.nodes.get(&a));
        let cat_b = cat_rank(graph.nodes.get(&b));
        let in_a = orig_in_edges.get(&a).map_or(0, |v| v.len());
        let in_b = orig_in_edges.get(&b).map_or(0, |v| v.len());
        let out_a = orig_out_edges.get(&a).map_or(0, |v| v.len());
        let out_b = orig_out_edges.get(&b).map_or(0, |v| v.len());

        cat_a
            .cmp(&cat_b)
            .then_with(|| in_a.cmp(&in_b))
            .then_with(|| out_b.cmp(&out_a))
            .then_with(|| a.cmp(&b))
    });

    for s in remaining {
        if color.get(&s) == Some(&Color::White) {
            dfs_visit(s, &orig_out_edges, &mut color, &mut back_edges);
        }
    }

    // ------------------------------------------------------------------------
    // Step 2: Build Forward DAG (excluding back-edges)
    // ------------------------------------------------------------------------
    let mut dag_in_edges: HashMap<u64, Vec<u64>> = HashMap::new();
    for &serial in &active_nodes {
        dag_in_edges.insert(serial, Vec::new());
    }

    for edge in graph.collapsed_edges() {
        let u = edge.key.output_node_serial;
        let v = edge.key.input_node_serial;
        if active_nodes.contains(&u)
            && active_nodes.contains(&v)
            && u != v
            && !back_edges.contains(&(u, v))
        {
            dag_in_edges.entry(v).or_default().push(u);
        }
    }

    let mut columns: BTreeMap<u64, usize> = BTreeMap::new();

    // ------------------------------------------------------------------------
    // Step 3: Base Column Seeding (DAG roots and unconnected sinks)
    // ------------------------------------------------------------------------
    for &serial in &active_nodes {
        let node = match graph.nodes.get(&serial) {
            Some(n) => n,
            None => continue,
        };
        let orig_in_count = orig_in_edges.get(&serial).map_or(0, |v| v.len());
        let orig_out_count = orig_out_edges.get(&serial).map_or(0, |v| v.len());
        let dag_in_count = dag_in_edges.get(&serial).map_or(0, |v| v.len());

        let is_sink_type = matches!(
            node.category,
            NodeCategory::AudioSink | NodeCategory::AudioInputStream
        ) || (!node.input_ports.is_empty() && node.output_ports.is_empty());

        if dag_in_count == 0 {
            if orig_in_count == 0 && orig_out_count == 0 && is_sink_type {
                // Truly unconnected sink: R4 places in Column 1 alongside sinks fed by Column 0
                columns.insert(serial, 1);
            } else {
                columns.insert(serial, 0);
            }
        }
    }

    // ------------------------------------------------------------------------
    // Step 4: Iterative Longest DAG Path Relaxation (bounded & acyclic)
    // ------------------------------------------------------------------------
    let max_iterations = active_nodes.len() + 2;
    for _ in 0..max_iterations {
        let mut changed = false;
        for &serial in &active_nodes {
            if let Some(preds) = dag_in_edges.get(&serial) {
                if !preds.is_empty() {
                    let max_pred = preds
                        .iter()
                        .filter_map(|p| columns.get(p))
                        .max()
                        .copied();
                    if let Some(pred_depth) = max_pred {
                        let target_depth = pred_depth + 1;
                        let curr_depth = columns.get(&serial).copied();
                        match curr_depth {
                            Some(d) if d < target_depth => {
                                columns.insert(serial, target_depth);
                                changed = true;
                            }
                            None => {
                                columns.insert(serial, target_depth);
                                changed = true;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    // ------------------------------------------------------------------------
    // Step 5: Fallback for any unreachable nodes
    // ------------------------------------------------------------------------
    for &serial in &active_nodes {
        columns.entry(serial).or_insert(0);
    }

    // ------------------------------------------------------------------------
    // Step 6: Dense Column Compaction (eliminates empty column voids)
    // ------------------------------------------------------------------------
    let distinct_cols: BTreeSet<usize> = columns.values().copied().collect();
    let is_compact = distinct_cols.iter().enumerate().all(|(idx, &c)| idx == c);
    if !is_compact {
        let col_remap: HashMap<usize, usize> = distinct_cols
            .into_iter()
            .enumerate()
            .map(|(dense, orig)| (orig, dense))
            .collect();
        for c in columns.values_mut() {
            if let Some(&new_c) = col_remap.get(c) {
                *c = new_c;
            }
        }
    }

    columns
}

pub fn classify_node(media_class: &str, _name: &str) -> NodeCategory {
    let mc = media_class.to_lowercase();
    if mc.contains("audio/source") {
        NodeCategory::AudioSource
    } else if mc.contains("stream/output/audio") {
        NodeCategory::AudioOutputStream
    } else if mc.contains("stream/input/audio") {
        NodeCategory::AudioInputStream
    } else if mc.contains("audio/sink") {
        NodeCategory::AudioSink
    } else if mc.contains("audio/duplex") || mc.contains("audio") {
        NodeCategory::AudioDuplex
    } else if mc.contains("midi") {
        NodeCategory::Midi
    } else if mc.contains("video") {
        NodeCategory::Video
    } else {
        NodeCategory::Internal
    }
}

pub fn infer_channel_from_name(name: &str) -> String {
    let upper = name.to_uppercase();
    for ch in ["FL", "FR", "MONO", "FC", "LFE", "RL", "RR", "SL", "SR"] {
        if upper.ends_with(&format!("_{ch}")) || upper == ch {
            return ch.to_string();
        }
    }
    if upper.ends_with("_0") || upper.ends_with("_1") {
        return upper.rsplit('_').next().unwrap_or("").to_string();
    }
    String::new()
}

/// Provides a realistic demo/fallback graph when PipeWire daemon is unreachable,
/// and allows testing all features (Stereo->Stereo, Mono->Stereo, Mono->Mono, custom port editing).
pub fn create_demo_graph() -> PwGraph {
    let mut nodes = BTreeMap::new();
    let mut ports = BTreeMap::new();
    let mut links = BTreeMap::new();

    let mut add_node = |id: u32,
                        name: &str,
                        desc: &str,
                        app: Option<&str>,
                        media: Option<&str>,
                        mc: &str,
                        cat: NodeCategory,
                        in_chs: &[(&str, &str, bool)],
                        out_chs: &[(&str, &str, bool)]| {
        let node_serial = id as u64;
        let mut input_ports = Vec::new();
        let mut output_ports = Vec::new();

        for (idx, &(pname, ch, is_mon)) in in_chs.iter().enumerate() {
            let pid = id * 100 + idx as u32 + 1;
            let pserial = pid as u64;
            let mut port_props = BTreeMap::new();
            port_props.insert("object.serial".to_string(), pserial.to_string());
            port_props.insert("node.id".to_string(), id.to_string());
            port_props.insert("port.id".to_string(), idx.to_string());
            port_props.insert("port.name".to_string(), pname.to_string());
            port_props.insert("port.alias".to_string(), format!("{desc}:{pname}"));
            port_props.insert("port.direction".to_string(), "in".to_string());
            port_props.insert("audio.channel".to_string(), ch.to_string());
            port_props.insert("format.dsp".to_string(), "32 bit float mono audio".to_string());

            let port_params = vec![
                PwNodeParam {
                    param_type: "EnumFormat".to_string(),
                    key_id: 1,
                    name: "mediaType".to_string(),
                    value: PwParamValue::String("audio".to_string()),
                    read_only: true,
                },
                PwNodeParam {
                    param_type: "EnumFormat".to_string(),
                    key_id: 2,
                    name: "mediaSubtype".to_string(),
                    value: PwParamValue::String("raw".to_string()),
                    read_only: true,
                },
                PwNodeParam {
                    param_type: "EnumFormat".to_string(),
                    key_id: 3,
                    name: "format".to_string(),
                    value: PwParamValue::DisplayOnly("F32LE [choices: F32LE, S16LE, S24LE]".to_string()),
                    read_only: true,
                },
                PwNodeParam {
                    param_type: "EnumFormat".to_string(),
                    key_id: 4,
                    name: "rate".to_string(),
                    value: PwParamValue::DisplayOnly("48000 (range: 44100..192000)".to_string()),
                    read_only: true,
                },
                PwNodeParam {
                    param_type: "Format".to_string(),
                    key_id: 3,
                    name: "format".to_string(),
                    value: PwParamValue::DisplayOnly("F32LE".to_string()),
                    read_only: true,
                },
                PwNodeParam {
                    param_type: "Format".to_string(),
                    key_id: 4,
                    name: "rate".to_string(),
                    value: PwParamValue::Int(48000),
                    read_only: true,
                },
                PwNodeParam {
                    param_type: "Buffers".to_string(),
                    key_id: 1,
                    name: "buffers".to_string(),
                    value: PwParamValue::Int(16),
                    read_only: true,
                },
                PwNodeParam {
                    param_type: "Latency".to_string(),
                    key_id: 1,
                    name: "minQuantum".to_string(),
                    value: PwParamValue::Int(32),
                    read_only: true,
                },
                PwNodeParam {
                    param_type: "Latency".to_string(),
                    key_id: 2,
                    name: "maxQuantum".to_string(),
                    value: PwParamValue::Int(8192),
                    read_only: true,
                },
            ];

            let port = PwPort {
                id: pid,
                object_serial: pserial,
                node_id: id,
                node_serial,
                name: pname.to_string(),
                channel: ch.to_string(),
                direction: PortDirection::Input,
                is_monitor: is_mon,
                props: port_props,
                params: port_params,
            };
            ports.insert(pserial, port.clone());
            input_ports.push(port);
        }

        for (idx, &(pname, ch, is_mon)) in out_chs.iter().enumerate() {
            let pid = id * 100 + 50 + idx as u32 + 1;
            let pserial = pid as u64;
            let mut port_props = BTreeMap::new();
            port_props.insert("object.serial".to_string(), pserial.to_string());
            port_props.insert("node.id".to_string(), id.to_string());
            port_props.insert("port.id".to_string(), (idx + 50).to_string());
            port_props.insert("port.name".to_string(), pname.to_string());
            port_props.insert("port.alias".to_string(), format!("{desc}:{pname}"));
            port_props.insert("port.direction".to_string(), "out".to_string());
            port_props.insert("audio.channel".to_string(), ch.to_string());
            if is_mon {
                port_props.insert("port.monitor".to_string(), "true".to_string());
            }
            port_props.insert("format.dsp".to_string(), "32 bit float mono audio".to_string());

            let port_params = vec![
                PwNodeParam {
                    param_type: "EnumFormat".to_string(),
                    key_id: 1,
                    name: "mediaType".to_string(),
                    value: PwParamValue::String("audio".to_string()),
                    read_only: true,
                },
                PwNodeParam {
                    param_type: "EnumFormat".to_string(),
                    key_id: 2,
                    name: "mediaSubtype".to_string(),
                    value: PwParamValue::String("raw".to_string()),
                    read_only: true,
                },
                PwNodeParam {
                    param_type: "EnumFormat".to_string(),
                    key_id: 3,
                    name: "format".to_string(),
                    value: PwParamValue::DisplayOnly("F32LE [choices: F32LE, S16LE, S24LE]".to_string()),
                    read_only: true,
                },
                PwNodeParam {
                    param_type: "EnumFormat".to_string(),
                    key_id: 4,
                    name: "rate".to_string(),
                    value: PwParamValue::DisplayOnly("48000 (range: 44100..192000)".to_string()),
                    read_only: true,
                },
                PwNodeParam {
                    param_type: "Format".to_string(),
                    key_id: 3,
                    name: "format".to_string(),
                    value: PwParamValue::DisplayOnly("F32LE".to_string()),
                    read_only: true,
                },
                PwNodeParam {
                    param_type: "Format".to_string(),
                    key_id: 4,
                    name: "rate".to_string(),
                    value: PwParamValue::Int(48000),
                    read_only: true,
                },
                PwNodeParam {
                    param_type: "Buffers".to_string(),
                    key_id: 1,
                    name: "buffers".to_string(),
                    value: PwParamValue::Int(16),
                    read_only: true,
                },
                PwNodeParam {
                    param_type: "Latency".to_string(),
                    key_id: 1,
                    name: "minQuantum".to_string(),
                    value: PwParamValue::Int(32),
                    read_only: true,
                },
                PwNodeParam {
                    param_type: "Latency".to_string(),
                    key_id: 2,
                    name: "maxQuantum".to_string(),
                    value: PwParamValue::Int(8192),
                    read_only: true,
                },
            ];

            let port = PwPort {
                id: pid,
                object_serial: pserial,
                node_id: id,
                node_serial,
                name: pname.to_string(),
                channel: ch.to_string(),
                direction: PortDirection::Output,
                is_monitor: is_mon,
                props: port_props,
                params: port_params,
            };
            ports.insert(pserial, port.clone());
            output_ports.push(port);
        }

        let mut node_props = BTreeMap::new();
        node_props.insert("object.serial".to_string(), node_serial.to_string());
        node_props.insert("node.name".to_string(), name.to_string());
        node_props.insert("node.description".to_string(), desc.to_string());
        node_props.insert("media.class".to_string(), mc.to_string());
        node_props.insert("factory.mode".to_string(), "split".to_string());
        node_props.insert("audio.rate".to_string(), "48000".to_string());
        if let Some(a) = app {
            node_props.insert("application.name".to_string(), a.to_string());
        }
        if let Some(m) = media {
            node_props.insert("media.name".to_string(), m.to_string());
        }

        let non_mon_out: Vec<String> = out_chs
            .iter()
            .filter(|&&(_, _, mon)| !mon)
            .map(|&(_, ch, _)| ch.to_string())
            .collect();
        let channel_names: Vec<String> = if !non_mon_out.is_empty() {
            non_mon_out
        } else {
            in_chs.iter().map(|&(_, ch, _)| ch.to_string()).collect()
        };
        let n_channels = channel_names.len().max(1);
        let channel_volumes = vec![1.0_f32; n_channels];

        let params = vec![
            PwNodeParam {
                param_type: "Props".to_string(),
                key_id: 65539, // SPA_PROP_volume
                name: "volume".to_string(),
                value: PwParamValue::Float(1.0),
                read_only: false,
            },
            PwNodeParam {
                param_type: "Props".to_string(),
                key_id: 65540, // SPA_PROP_mute
                name: "mute".to_string(),
                value: PwParamValue::Bool(false),
                read_only: false,
            },
            PwNodeParam {
                param_type: "Props".to_string(),
                key_id: 65544, // SPA_PROP_channelVolumes
                name: "channelVolumes".to_string(),
                value: PwParamValue::FloatArray(channel_volumes.clone()),
                read_only: true,
            },
            PwNodeParam {
                param_type: "Props".to_string(),
                key_id: 65555, // SPA_PROP_latencyOffsetNsec
                name: "latencyOffsetNsec".to_string(),
                value: PwParamValue::Long(0),
                read_only: false,
            },
            PwNodeParam {
                param_type: "Format".to_string(),
                key_id: 1,
                name: "mediaType".to_string(),
                value: PwParamValue::DisplayOnly("audio".to_string()),
                read_only: true,
            },
            PwNodeParam {
                param_type: "Format".to_string(),
                key_id: 2,
                name: "mediaSubtype".to_string(),
                value: PwParamValue::DisplayOnly("raw".to_string()),
                read_only: true,
            },
            PwNodeParam {
                param_type: "Format".to_string(),
                key_id: 65537,
                name: "format".to_string(),
                value: PwParamValue::DisplayOnly("F32LE".to_string()),
                read_only: true,
            },
            PwNodeParam {
                param_type: "Format".to_string(),
                key_id: 65539,
                name: "rate".to_string(),
                value: PwParamValue::Int(48000),
                read_only: true,
            },
            PwNodeParam {
                param_type: "Format".to_string(),
                key_id: 65540,
                name: "channels".to_string(),
                value: PwParamValue::Int(n_channels as i32),
                read_only: true,
            },
        ];

        let (device_id, device_serial) = if node_serial == 10 {
            node_props.insert("device.id".to_string(), "100".to_string());
            (Some(100), Some(1000))
        } else if node_serial == 50 {
            node_props.insert("device.id".to_string(), "500".to_string());
            (Some(500), Some(5000))
        } else {
            (None, None)
        };

        nodes.insert(
            node_serial,
            PwNode {
                id,
                object_serial: node_serial,
                name: name.to_string(),
                description: desc.to_string(),
                app_name: app.map(|s| s.to_string()),
                media_name: media.map(|s| s.to_string()),
                media_class: mc.to_string(),
                category: cat,
                state: "running".to_string(),
                input_ports,
                output_ports,
                volume: Some(1.0),
                muted: Some(false),
                channel_volumes,
                channel_names,
                props: node_props,
                params,
                device_id,
                device_serial,
            },
        );
    };

    // 1. USB Condenser Mic (Mono Audio/Source)
    add_node(
        10,
        "alsa_input.usb-Focusrite_Scarlett_Solo",
        "Scarlett Solo Microphone",
        None,
        None,
        "Audio/Source",
        NodeCategory::AudioSource,
        &[],
        &[("capture_MONO", "MONO", false)],
    );

    // 2. Firefox Browser (Stereo Stream/Output/Audio)
    add_node(
        20,
        "Firefox",
        "Firefox",
        Some("Firefox"),
        Some("YouTube — Lo-Fi Beats"),
        "Stream/Output/Audio",
        NodeCategory::AudioOutputStream,
        &[],
        &[("output_FL", "FL", false), ("output_FR", "FR", false)],
    );

    // 3. Spotify (Stereo Stream/Output/Audio)
    add_node(
        30,
        "spotify",
        "Spotify",
        Some("Spotify"),
        Some("Daft Punk - Random Access Memories"),
        "Stream/Output/Audio",
        NodeCategory::AudioOutputStream,
        &[],
        &[("output_FL", "FL", false), ("output_FR", "FR", false)],
    );

    // 4. EasyEffects Filter / Equalizer (Stereo Audio/Duplex)
    add_node(
        40,
        "easyeffects_sink",
        "EasyEffects Equalizer",
        Some("EasyEffects"),
        None,
        "Audio/Duplex",
        NodeCategory::AudioDuplex,
        &[("playback_FL", "FL", false), ("playback_FR", "FR", false)],
        &[("output_FL", "FL", false), ("output_FR", "FR", false)],
    );

    // 5. Built-in Audio Stereo Headphones (Audio/Sink)
    add_node(
        50,
        "alsa_output.pci-0000_00_1f.3.analog-stereo",
        "Built-in Audio Headphones",
        None,
        None,
        "Audio/Sink",
        NodeCategory::AudioSink,
        &[("playback_FL", "FL", false), ("playback_FR", "FR", false)],
        &[("monitor_FL", "FL", true), ("monitor_FR", "FR", true)],
    );

    // 6. Discord Voice Capture (Stereo Stream/Input/Audio)
    add_node(
        60,
        "WEBRTC VoiceEngine",
        "Voice Chat Input",
        Some("Discord"),
        Some("Voice Call"),
        "Stream/Input/Audio",
        NodeCategory::AudioInputStream,
        &[("input_FL", "FL", false), ("input_FR", "FR", false)],
        &[],
    );

    let mut add_demo_link = |id: u32, on: u32, op: u32, inn: u32, ip: u32| {
        let serial = id as u64;
        let mut link_props = BTreeMap::new();
        link_props.insert("object.serial".to_string(), serial.to_string());
        link_props.insert("link.output.node".to_string(), on.to_string());
        link_props.insert("link.output.port".to_string(), op.to_string());
        link_props.insert("link.input.node".to_string(), inn.to_string());
        link_props.insert("link.input.port".to_string(), ip.to_string());
        link_props.insert("object.linger".to_string(), "1".to_string());

        links.insert(
            serial,
            PwLink {
                id,
                object_serial: serial,
                output_node_id: on,
                output_node_serial: on as u64,
                output_port_id: op,
                output_port_serial: op as u64,
                input_node_id: inn,
                input_node_serial: inn as u64,
                input_port_id: ip,
                input_port_serial: ip as u64,
                state: "active".to_string(),
                props: link_props,
            },
        );
    };

    // Mic (Mono: 1051) -> Discord (Stereo: 6001, 6002) => Mono -> Stereo collapsed link!
    add_demo_link(901, 10, 1051, 60, 6001);
    add_demo_link(902, 10, 1051, 60, 6002);

    // Firefox (Stereo: 2051, 2052) -> Headphones (Stereo: 5001, 5002) => Stereo -> Stereo
    add_demo_link(903, 20, 2051, 50, 5001);
    add_demo_link(904, 20, 2052, 50, 5002);

    // Spotify (3051, 3052) -> EasyEffects (4001, 4002) -> Headphones (5001, 5002)
    add_demo_link(905, 30, 3051, 40, 4001);
    add_demo_link(906, 30, 3052, 40, 4002);
    add_demo_link(907, 40, 4051, 50, 5001);
    add_demo_link(908, 40, 4052, 50, 5002);

    let mut scarlett_props = BTreeMap::new();
    scarlett_props.insert("object.serial".to_string(), "1000".to_string());
    scarlett_props.insert("device.api".to_string(), "alsa".to_string());
    scarlett_props.insert("device.bus".to_string(), "usb".to_string());
    scarlett_props.insert("device.name".to_string(), "alsa_card.usb-Focusrite_Scarlett_Solo".to_string());
    scarlett_props.insert("device.description".to_string(), "Scarlett Solo USB".to_string());
    scarlett_props.insert("device.nick".to_string(), "Scarlett Solo".to_string());
    scarlett_props.insert("media.class".to_string(), "Audio/Device".to_string());

    let scarlett_profiles = vec![
        PwDeviceProfile {
            index: 0,
            name: "off".to_string(),
            description: "Off".to_string(),
            priority: 0,
            available: PwAvailability::Yes,
            active: false,
        },
        PwDeviceProfile {
            index: 1,
            name: "analog-stereo".to_string(),
            description: "Analog Stereo".to_string(),
            priority: 100,
            available: PwAvailability::Yes,
            active: true,
        },
        PwDeviceProfile {
            index: 2,
            name: "pro-audio".to_string(),
            description: "Pro Audio".to_string(),
            priority: 80,
            available: PwAvailability::Yes,
            active: false,
        },
        PwDeviceProfile {
            index: 3,
            name: "iec958-stereo".to_string(),
            description: "S/PDIF".to_string(),
            priority: 20,
            available: PwAvailability::No,
            active: false,
        },
    ];

    let scarlett_routes = vec![
        PwDeviceRoute {
            index: 0,
            name: "analog-input-mic".to_string(),
            description: "Mic".to_string(),
            direction: PortDirection::Input,
            available: PwAvailability::Yes,
            active: true,
            device_id: Some(100),
            devices: vec![0, 0],
            profiles: vec![1, 2],
            props: BTreeMap::new(),
        },
        PwDeviceRoute {
            index: 1,
            name: "analog-input-linein".to_string(),
            description: "Line In".to_string(),
            direction: PortDirection::Input,
            available: PwAvailability::Yes,
            active: false,
            device_id: Some(100),
            devices: vec![0, 0],
            profiles: vec![1, 2],
            props: BTreeMap::new(),
        },
        PwDeviceRoute {
            index: 2,
            name: "analog-output-headphones".to_string(),
            description: "Headphones".to_string(),
            direction: PortDirection::Output,
            available: PwAvailability::Yes,
            active: true,
            device_id: Some(100),
            devices: vec![0, 0],
            profiles: vec![1, 2],
            props: BTreeMap::new(),
        },
        PwDeviceRoute {
            index: 3,
            name: "iec958-input".to_string(),
            description: "S/PDIF In".to_string(),
            direction: PortDirection::Input,
            available: PwAvailability::No,
            active: false,
            device_id: Some(100),
            devices: vec![1],
            profiles: vec![3],
            props: BTreeMap::new(),
        },
    ];

    let scarlett_device = PwDevice {
        id: 100,
        object_serial: 1000,
        name: "alsa_card.usb-Focusrite_Scarlett_Solo".to_string(),
        description: "Scarlett Solo USB".to_string(),
        nick: Some("Scarlett Solo".to_string()),
        media_class: Some("Audio/Device".to_string()),
        props: scarlett_props,
        profiles: scarlett_profiles,
        routes: scarlett_routes,
        active_profile: Some(1),
    };

    let mut builtin_props = BTreeMap::new();
    builtin_props.insert("object.serial".to_string(), "5000".to_string());
    builtin_props.insert("device.api".to_string(), "alsa".to_string());
    builtin_props.insert("device.bus".to_string(), "pci".to_string());
    builtin_props.insert("device.name".to_string(), "alsa_card.pci-0000_00_1f.3".to_string());
    builtin_props.insert("device.description".to_string(), "Built-in Audio".to_string());
    builtin_props.insert("device.nick".to_string(), "HDA Intel PCH".to_string());
    builtin_props.insert("media.class".to_string(), "Audio/Device".to_string());

    let builtin_profiles = vec![
        PwDeviceProfile {
            index: 0,
            name: "off".to_string(),
            description: "Off".to_string(),
            priority: 0,
            available: PwAvailability::Yes,
            active: false,
        },
        PwDeviceProfile {
            index: 1,
            name: "analog-stereo".to_string(),
            description: "Analog Stereo Duplex".to_string(),
            priority: 100,
            available: PwAvailability::Yes,
            active: true,
        },
        PwDeviceProfile {
            index: 2,
            name: "pro-audio".to_string(),
            description: "Pro Audio".to_string(),
            priority: 80,
            available: PwAvailability::Yes,
            active: false,
        },
        PwDeviceProfile {
            index: 3,
            name: "hdmi-stereo".to_string(),
            description: "HDMI Stereo".to_string(),
            priority: 20,
            available: PwAvailability::No,
            active: false,
        },
    ];

    let builtin_routes = vec![
        PwDeviceRoute {
            index: 0,
            name: "analog-output-headphones".to_string(),
            description: "Headphones".to_string(),
            direction: PortDirection::Output,
            available: PwAvailability::Yes,
            active: true,
            device_id: Some(500),
            devices: vec![0, 0],
            profiles: vec![1, 2],
            props: BTreeMap::new(),
        },
        PwDeviceRoute {
            index: 1,
            name: "analog-output-speaker".to_string(),
            description: "Internal Speakers".to_string(),
            direction: PortDirection::Output,
            available: PwAvailability::Yes,
            active: false,
            device_id: Some(500),
            devices: vec![0, 0],
            profiles: vec![1, 2],
            props: BTreeMap::new(),
        },
        PwDeviceRoute {
            index: 2,
            name: "analog-output-lineout".to_string(),
            description: "Line Out".to_string(),
            direction: PortDirection::Output,
            available: PwAvailability::No,
            active: false,
            device_id: Some(500),
            devices: vec![0, 0],
            profiles: vec![1, 2],
            props: BTreeMap::new(),
        },
        PwDeviceRoute {
            index: 3,
            name: "analog-input-internal-mic".to_string(),
            description: "Internal Mic".to_string(),
            direction: PortDirection::Input,
            available: PwAvailability::Yes,
            active: true,
            device_id: Some(500),
            devices: vec![0, 0],
            profiles: vec![1, 2],
            props: BTreeMap::new(),
        },
        PwDeviceRoute {
            index: 4,
            name: "analog-input-headset-mic".to_string(),
            description: "Headset Mic".to_string(),
            direction: PortDirection::Input,
            available: PwAvailability::No,
            active: false,
            device_id: Some(500),
            devices: vec![0, 0],
            profiles: vec![1, 2],
            props: BTreeMap::new(),
        },
    ];

    let builtin_device = PwDevice {
        id: 500,
        object_serial: 5000,
        name: "alsa_card.pci-0000_00_1f.3".to_string(),
        description: "Built-in Audio".to_string(),
        nick: Some("HDA Intel PCH".to_string()),
        media_class: Some("Audio/Device".to_string()),
        props: builtin_props,
        profiles: builtin_profiles,
        routes: builtin_routes,
        active_profile: Some(1),
    };

    let mut devices = BTreeMap::new();
    devices.insert(1000, scarlett_device);
    devices.insert(5000, builtin_device);

    PwGraph {
        nodes,
        ports,
        links,
        devices,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collapsed_edges_and_summaries() {
        let graph = create_demo_graph();
        let edges = graph.collapsed_edges();
        // 8 individual port links collapse into 4 node-to-node wires!
        assert_eq!(graph.links.len(), 8);
        assert_eq!(edges.len(), 4);

        let mic_to_discord = edges
            .iter()
            .find(|e| e.key.output_node_serial == 10 && e.key.input_node_serial == 60)
            .expect("Mic -> Discord edge should exist");
        assert_eq!(mic_to_discord.summary, "Mono → Stereo");
        assert_eq!(mic_to_discord.links.len(), 2);

        let ff_to_hp = edges
            .iter()
            .find(|e| e.key.output_node_serial == 20 && e.key.input_node_serial == 50)
            .expect("Firefox -> Headphones edge should exist");
        assert_eq!(ff_to_hp.summary, "Stereo → Stereo");
        assert_eq!(ff_to_hp.links.len(), 2);
    }

    #[test]
    fn test_smart_default_port_mapping() {
        let graph = create_demo_graph();

        // Mono (Node serial 10: port serial 1051) -> Stereo (Node serial 50: port serials 5001, 5002)
        let mono_to_stereo = graph.compute_default_port_mapping(10, 50);
        assert_eq!(mono_to_stereo, vec![(1051, 5001), (1051, 5002)]);

        // Stereo (Node serial 20: port serials 2051, 2052) -> Stereo (Node serial 60: port serials 6001, 6002)
        let stereo_to_stereo = graph.compute_default_port_mapping(20, 60);
        assert_eq!(stereo_to_stereo, vec![(2051, 6001), (2052, 6002)]);
    }

    #[test]
    fn test_context_dependent_presets_by_node_layout() {
        let mut graph = create_demo_graph();

        // Add a Mono Input Stream (serial 70, port serial 7001) and a 5.1 Surround Sink (serial 80)
        let mono_in_port = PwPort {
            id: 7001,
            object_serial: 7001,
            node_id: 70,
            node_serial: 70,
            name: "input_MONO".into(),
            channel: "MONO".into(),
            direction: PortDirection::Input,
            is_monitor: false,
            props: BTreeMap::new(),
            params: Vec::new(),
        };
        graph.ports.insert(7001, mono_in_port.clone());
        graph.nodes.insert(
            70,
            PwNode {
                id: 70,
                object_serial: 70,
                name: "mono_rec".into(),
                description: "Mono Recorder".into(),
                app_name: None,
                media_name: None,
                media_class: "Stream/Input/Audio".into(),
                category: NodeCategory::AudioInputStream,
                state: "running".into(),
                input_ports: vec![mono_in_port],
                output_ports: vec![],
                volume: Some(1.0),
                muted: Some(false),
                channel_volumes: vec![1.0],
                channel_names: vec!["MONO".into()],
                props: BTreeMap::new(),
                params: vec![],
                device_id: None,
                device_serial: None,
            },
        );

        let surround_chs = ["FL", "FR", "FC", "LFE", "RL", "RR"];
        let mut surround_in_ports = Vec::new();
        for (idx, ch) in surround_chs.iter().enumerate() {
            let pserial = 8001 + idx as u64;
            let p = PwPort {
                id: pserial as u32,
                object_serial: pserial,
                node_id: 80,
                node_serial: 80,
                name: format!("playback_{ch}"),
                channel: (*ch).into(),
                direction: PortDirection::Input,
                is_monitor: false,
                props: BTreeMap::new(),
                params: Vec::new(),
            };
            graph.ports.insert(pserial, p.clone());
            surround_in_ports.push(p);
        }
        graph.nodes.insert(
            80,
            PwNode {
                id: 80,
                object_serial: 80,
                name: "surround_sink".into(),
                description: "5.1 Surround Speakers".into(),
                app_name: None,
                media_name: None,
                media_class: "Audio/Sink".into(),
                category: NodeCategory::AudioSink,
                state: "running".into(),
                input_ports: surround_in_ports,
                output_ports: vec![],
                volume: Some(1.0),
                muted: Some(false),
                channel_volumes: vec![1.0; 6],
                channel_names: surround_chs.iter().map(|s| (*s).into()).collect(),
                props: BTreeMap::new(),
                params: vec![],
                device_id: None,
                device_serial: None,
            },
        );

        // 1. Mono -> Mono (10 -> 70): no redundant quick routing presets
        assert!(graph.available_presets(10, 70).is_empty());
        assert_eq!(graph.compute_default_port_mapping(10, 70), vec![(1051, 7001)]);

        // 2. Mono -> Stereo (10 -> 50): Mono->Both, Mono->Left, Mono->Right, DisconnectAll
        assert_eq!(
            graph.available_presets(10, 50),
            vec![
                RoutingPreset::MonoToStereoBoth,
                RoutingPreset::MonoToLeftOnly,
                RoutingPreset::MonoToRightOnly,
                RoutingPreset::DisconnectAll,
            ]
        );
        assert_eq!(
            graph.compute_preset_port_mapping(10, 50, RoutingPreset::MonoToLeftOnly),
            vec![(1051, 5001)]
        );
        assert_eq!(
            graph.compute_preset_port_mapping(10, 50, RoutingPreset::MonoToRightOnly),
            vec![(1051, 5002)]
        );

        // 3. Stereo -> Mono (20 -> 70): Stereo->Mono(L+R), LeftOnly->Mono, RightOnly->Mono, DisconnectAll
        assert_eq!(
            graph.available_presets(20, 70),
            vec![
                RoutingPreset::StereoToMonoBoth,
                RoutingPreset::LeftOnlyToMono,
                RoutingPreset::RightOnlyToMono,
                RoutingPreset::DisconnectAll,
            ]
        );
        assert_eq!(
            graph.compute_preset_port_mapping(20, 70, RoutingPreset::StereoToMonoBoth),
            vec![(2051, 7001), (2052, 7001)]
        );
        assert_eq!(
            graph.compute_preset_port_mapping(20, 70, RoutingPreset::RightOnlyToMono),
            vec![(2052, 7001)]
        );

        // 4. Stereo -> 5.1 Surround (20 -> 80)
        assert_eq!(
            graph.available_presets(20, 80),
            vec![
                RoutingPreset::StereoToFront,
                RoutingPreset::StereoUpmixSurround,
                RoutingPreset::SwapLeftRight,
                RoutingPreset::DisconnectAll,
            ]
        );
        // Upmix connects FL(2051) -> FL(8001), FC(8003), RL(8005) and FR(2052) -> FR(8002), FC(8003), RR(8006)
        assert_eq!(
            graph.compute_preset_port_mapping(20, 80, RoutingPreset::StereoUpmixSurround),
            vec![
                (2051, 8001),
                (2052, 8002),
                (2051, 8003),
                (2052, 8003),
                (2051, 8005),
                (2052, 8006),
            ]
        );
    }

    #[test]
    fn test_has_unequal_channel_volumes() {
        let mut graph = create_demo_graph();
        let node = graph.nodes.get_mut(&20).unwrap();

        // Initially equal stereo channel volumes [0.85, 0.85] -> defaults to Master volume
        assert!(!node.has_unequal_channel_volumes());

        // Slight floating point jitter within 0.5% tolerance is still considered equal
        node.channel_volumes = vec![0.850, 0.853];
        assert!(!node.has_unequal_channel_volumes());

        // Differing channel volumes -> defaults to split Per-Channel volume mixer
        node.channel_volumes = vec![0.85, 0.50];
        assert!(node.has_unequal_channel_volumes());

        // Mono node -> never unequal
        node.channel_volumes = vec![0.75];
        assert!(!node.has_unequal_channel_volumes());
    }

    #[test]
    fn test_r1_matches_search_case_insensitive_name_and_description() {
        let name = "alsa_output.pci-0000_00_1f.3";
        let desc = "Built-in Audio Analog Stereo";

        // 1. Empty or whitespace-only query matches everything
        assert!(matches_search(name, desc, ""));
        assert!(matches_search(name, desc, "   "));
        assert!(matches_search(name, desc, "\t\n"));

        // 2. Case-insensitive matches on name
        assert!(matches_search(name, desc, "alsa_output"));
        assert!(matches_search(name, desc, "ALSA_OUTPUT"));
        assert!(matches_search(name, desc, "pci-0000"));
        assert!(matches_search(name, desc, "1F.3"));

        // 3. Case-insensitive matches on description
        assert!(matches_search(name, desc, "built-in"));
        assert!(matches_search(name, desc, "BUILT-IN"));
        assert!(matches_search(name, desc, "audio"));
        assert!(matches_search(name, desc, "AUDIO"));
        assert!(matches_search(name, desc, "stereo"));
        assert!(matches_search(name, desc, "STEREO"));
        assert!(matches_search(name, desc, "Analog"));

        // 4. Leading / trailing whitespace in query is trimmed
        assert!(matches_search(name, desc, "  audio  "));
        assert!(matches_search(name, desc, "  pci  "));

        // 5. Non-matching query returns false
        assert!(!matches_search(name, desc, "bluetooth"));
        assert!(!matches_search(name, desc, "spotify"));
        assert!(!matches_search(name, desc, "microphone"));
    }

    #[test]
    fn test_r1_dimmed_nodes_set_is_exactly_non_matching() {
        let graph = create_demo_graph();
        // Demo graph contains:
        // 10: Scarlett Solo Microphone (name: alsa_input.usb-Focusrite_Scarlett_Solo)
        // 20: Firefox (name: Firefox, description: Firefox)
        // 30: Spotify (name: spotify, description: Spotify)
        // 40: EasyEffects Equalizer (name: easyeffects_sink)
        // 50: Built-in Audio Headphones (name: alsa_output.pci-0000_00_1f.3.analog-stereo)
        // 60: Discord Voice Capture (name: Discord)
        // 70: OBS Studio (name: obs)
        // 80: 5.1 Surround Speakers (name: alsa_output.pci-0000_00_1f.3.surround51)

        // Empty search: zero dimmed nodes
        let dimmed_empty = compute_dimmed_nodes(&graph, "");
        assert!(dimmed_empty.is_empty(), "Empty query should dim no nodes");

        let dimmed_spaces = compute_dimmed_nodes(&graph, "   ");
        assert!(dimmed_spaces.is_empty(), "Whitespace query should dim no nodes");

        // Search for "spotify": only node 30 matches; all others (10, 20, 40, 50, 60, etc.) must be dimmed
        let dimmed_spotify = compute_dimmed_nodes(&graph, "spotify");
        assert!(!dimmed_spotify.contains(&30), "Spotify (serial 30) must NOT be dimmed");
        assert!(dimmed_spotify.contains(&10), "Node 10 must be dimmed");
        assert!(dimmed_spotify.contains(&20), "Node 20 must be dimmed");
        assert!(dimmed_spotify.contains(&40), "Node 40 must be dimmed");
        assert!(dimmed_spotify.contains(&50), "Node 50 must be dimmed");
        assert!(dimmed_spotify.contains(&60), "Node 60 must be dimmed");

        // Verify exact set complement: matching count + dimmed count == total nodes
        let matching_count = graph
            .nodes
            .values()
            .filter(|n| matches_search(&n.name, &n.description, "spotify"))
            .count();
        assert_eq!(dimmed_spotify.len(), graph.nodes.len() - matching_count);

        // Search for "audio": matches Built-in Audio Headphones (50) and 5.1 Surround Speakers (80)
        let dimmed_audio = compute_dimmed_nodes(&graph, "audio");
        assert!(!dimmed_audio.contains(&50));
        assert!(dimmed_audio.contains(&30)); // Spotify has no "audio" in name or description

        // Search for non-existent query: all nodes are dimmed
        let dimmed_none = compute_dimmed_nodes(&graph, "nonexistent_node_xyz");
        assert_eq!(dimmed_none.len(), graph.nodes.len());
    }

    #[test]
    fn test_r1_dimmed_links_set_is_exactly_either_endpoint_non_matching() {
        let graph = create_demo_graph();

        // 1. When search is inactive, dimmed links must be completely empty
        let dimmed_empty = compute_dimmed_nodes(&graph, "");
        let dimmed_links_inactive = compute_dimmed_links(&graph, &dimmed_empty, false);
        assert!(
            dimmed_links_inactive.is_empty(),
            "Inactive search must have zero dimmed links"
        );

        // 2. Active search for "spotify":
        // In demo graph, Spotify (30) feeds EasyEffects (40) and Built-in Audio (50).
        // Since neither EasyEffects (40) nor Built-in Audio (50) matches "spotify",
        // all edges incident to non-matching nodes MUST be dimmed!
        let dimmed_nodes_spotify = compute_dimmed_nodes(&graph, "spotify");
        let dimmed_links_spotify = compute_dimmed_links(&graph, &dimmed_nodes_spotify, true);

        for edge in graph.collapsed_edges() {
            let out_dimmed = dimmed_nodes_spotify.contains(&edge.key.output_node_serial);
            let in_dimmed = dimmed_nodes_spotify.contains(&edge.key.input_node_serial);
            let expected_dimmed = out_dimmed || in_dimmed;

            assert_eq!(
                dimmed_links_spotify.contains(&edge.key),
                expected_dimmed,
                "Edge {:?} dimmed state must equal out_dimmed ({out_dimmed}) || in_dimmed ({in_dimmed})",
                edge.key
            );
        }
    }

    #[test]
    fn test_r4_model_demo_graph_signal_flow_depth_columns() {
        let graph = create_demo_graph();
        // Visible nodes for default audio filter (audio sources, streams, sinks, duplex)
        let visible: HashSet<u64> = [10, 20, 30, 40, 50, 60].into_iter().collect();
        let cols = compute_signal_flow_columns(&graph, &visible);

        // Column 0: Sources (Mic 10, Firefox 20, Spotify 30)
        assert_eq!(cols.get(&10), Some(&0), "Mic must be in Column 0");
        assert_eq!(cols.get(&20), Some(&0), "Firefox must be in Column 0");
        assert_eq!(cols.get(&30), Some(&0), "Spotify must be in Column 0");

        // Column 1: Sinks fed by Col 0 (EasyEffects 40 fed by Spotify 30, Discord 60 fed by Mic 10)
        assert_eq!(cols.get(&40), Some(&1), "EasyEffects must be in Column 1");
        assert_eq!(cols.get(&60), Some(&1), "Discord must be in Column 1");

        // Column 2: Sinks fed by Col 1 (Headphones 50 fed by EasyEffects 40 and Firefox 20)
        assert_eq!(cols.get(&50), Some(&2), "Headphones must be in Column 2");
    }

    fn make_test_node(
        serial: u64,
        name: &str,
        category: NodeCategory,
        in_ports: usize,
        out_ports: usize,
    ) -> PwNode {
        let mut input_ports = Vec::new();
        for i in 0..in_ports {
            input_ports.push(PwPort {
                id: (serial * 100 + i as u64) as u32,
                object_serial: serial * 1000 + i as u64,
                node_id: serial as u32,
                node_serial: serial,
                name: format!("input_{}", i + 1),
                direction: PortDirection::Input,
                channel: "FL".to_string(),
                is_monitor: false,
                props: BTreeMap::new(),
                params: Vec::new(),
            });
        }
        let mut output_ports = Vec::new();
        for i in 0..out_ports {
            output_ports.push(PwPort {
                id: (serial * 200 + i as u64) as u32,
                object_serial: serial * 2000 + i as u64,
                node_id: serial as u32,
                node_serial: serial,
                name: format!("output_{}", i + 1),
                direction: PortDirection::Output,
                channel: "FL".to_string(),
                is_monitor: false,
                props: BTreeMap::new(),
                params: Vec::new(),
            });
        }
        PwNode {
            id: serial as u32,
            object_serial: serial,
            name: name.to_string(),
            description: name.to_string(),
            app_name: None,
            media_name: None,
            media_class: "Audio/Sink".to_string(),
            category,
            input_ports,
            output_ports,
            state: "running".to_string(),
            volume: Some(1.0),
            muted: Some(false),
            channel_names: Vec::new(),
            channel_volumes: Vec::new(),
            props: BTreeMap::new(),
            params: Vec::new(),
            device_id: None,
            device_serial: None,
        }
    }

    fn make_test_link(
        serial: u64,
        out_serial: u64,
        out_port: u64,
        in_serial: u64,
        in_port: u64,
    ) -> PwLink {
        PwLink {
            id: serial as u32,
            object_serial: serial,
            output_node_id: out_serial as u32,
            output_node_serial: out_serial,
            output_port_id: out_port as u32,
            output_port_serial: out_port,
            input_node_id: in_serial as u32,
            input_node_serial: in_serial,
            input_port_id: in_port as u32,
            input_port_serial: in_port,
            state: "active".to_string(),
            props: BTreeMap::new(),
        }
    }

    #[test]
    fn test_r4_model_synthetic_chain_columns() {
        let mut graph = PwGraph::default();

        let n1 = make_test_node(1, "Synthetic Source", NodeCategory::AudioSource, 0, 1);
        graph.ports.insert(1000, n1.output_ports[0].clone());
        graph.nodes.insert(1, n1);

        let n2 = make_test_node(2, "Synthetic Filter", NodeCategory::AudioDuplex, 1, 1);
        graph.ports.insert(2000, n2.input_ports[0].clone());
        graph.ports.insert(2001, n2.output_ports[0].clone());
        graph.nodes.insert(2, n2);

        let n3 = make_test_node(3, "Synthetic Sink", NodeCategory::AudioSink, 1, 0);
        graph.ports.insert(3000, n3.input_ports[0].clone());
        graph.nodes.insert(3, n3);

        graph.links.insert(1001, make_test_link(1001, 1, 1000, 2, 2000));
        graph.links.insert(1002, make_test_link(1002, 2, 2001, 3, 3000));

        let visible: HashSet<u64> = [1, 2, 3].into_iter().collect();
        let cols = compute_signal_flow_columns(&graph, &visible);

        assert_eq!(cols.get(&1), Some(&0), "Chain source must be Col 0");
        assert_eq!(cols.get(&2), Some(&1), "Chain filter must be Col 1");
        assert_eq!(cols.get(&3), Some(&2), "Chain sink must be Col 2");
    }

    #[test]
    fn test_r4_model_synthetic_unconnected_sink_columns() {
        let mut graph = PwGraph::default();

        let n1 = make_test_node(1, "Isolated Source", NodeCategory::AudioSource, 0, 0);
        graph.nodes.insert(1, n1);

        let n2 = make_test_node(2, "Unconnected Headphones", NodeCategory::AudioSink, 1, 0);
        graph.ports.insert(2000, n2.input_ports[0].clone());
        graph.nodes.insert(2, n2);

        let visible: HashSet<u64> = [1, 2].into_iter().collect();
        let cols = compute_signal_flow_columns(&graph, &visible);

        assert_eq!(cols.get(&1), Some(&0), "Isolated source must be Col 0");
        assert_eq!(
            cols.get(&2),
            Some(&1),
            "Unconnected sink must be placed in Col 1 alongside sinks fed by Col 0"
        );
    }
}
