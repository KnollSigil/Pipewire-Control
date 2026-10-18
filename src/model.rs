use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortDirection {
    Input,
    Output,
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

#[derive(Debug, Clone)]
pub struct PwPort {
    pub id: u32,
    pub object_serial: u64,
    pub node_id: u32,
    pub node_serial: u64,
    pub name: String,
    pub channel: String,
    pub direction: PortDirection,
    pub is_monitor: bool,
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
}

/// The PipeWire graph snapshot used by the GUI.
/// All maps are keyed by `object_serial: u64` rather than global `id: u32`.
#[derive(Debug, Clone, Default)]
pub struct PwGraph {
    pub nodes: BTreeMap<u64, PwNode>,
    pub ports: BTreeMap<u64, PwPort>,
    pub links: BTreeMap<u64, PwLink>,
}

impl PwGraph {
    /// Synchronizes `node_serial` on ports, `*_serial` on links, and rebuilds each node's
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
                CollapsedEdge {
                    key: EdgeKey {
                        output_node_serial,
                        input_node_serial,
                    },
                    links,
                    summary,
                }
            })
            .collect()
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

pub fn classify_node(media_class: &str, name: &str) -> NodeCategory {
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
    } else if name.contains("Dummy-Driver") || name.contains("Freewheel-Driver") {
        NodeCategory::Internal
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
            let port = PwPort {
                id: pid,
                object_serial: pserial,
                node_id: id,
                node_serial,
                name: pname.to_string(),
                channel: ch.to_string(),
                direction: PortDirection::Input,
                is_monitor: is_mon,
            };
            ports.insert(pserial, port.clone());
            input_ports.push(port);
        }

        for (idx, &(pname, ch, is_mon)) in out_chs.iter().enumerate() {
            let pid = id * 100 + 50 + idx as u32 + 1;
            let pserial = pid as u64;
            let port = PwPort {
                id: pid,
                object_serial: pserial,
                node_id: id,
                node_serial,
                name: pname.to_string(),
                channel: ch.to_string(),
                direction: PortDirection::Output,
                is_monitor: is_mon,
            };
            ports.insert(pserial, port.clone());
            output_ports.push(port);
        }

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

    PwGraph {
        nodes,
        ports,
        links,
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
}
