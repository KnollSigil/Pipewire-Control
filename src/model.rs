use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortDirection {
    Input,
    Output,
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
            "FL" | "SL" => 1,
            "FR" | "SR" => 2,
            "FC" => 3,
            "LFE" => 4,
            "RL" => 5,
            "RR" => 6,
            _ => 10,
        };
        let mon_offset = if self.is_monitor { 50 } else { 0 };
        (base + mon_offset, self.name.clone())
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
            (1, 1, 1) => "Mono → Mono".to_string(),
            (1, 2, 2) => "Mono → Stereo".to_string(),
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
            (_, _, n) => format!(
                "{} ch ({} links)",
                out_port_serials.len().max(in_port_serials.len()),
                n
            ),
        }
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

        if out_ports.is_empty() || in_ports.is_empty() {
            return Vec::new();
        }

        // Case 1: Mono -> Stereo (1 output port, 2 input ports)
        if out_ports.len() == 1 && in_ports.len() == 2 {
            return vec![
                (out_ports[0].object_serial, in_ports[0].object_serial),
                (out_ports[0].object_serial, in_ports[1].object_serial),
            ];
        }

        // Case 2: Stereo -> Mono (2 output ports, 1 input port)
        if out_ports.len() == 2 && in_ports.len() == 1 {
            return vec![
                (out_ports[0].object_serial, in_ports[0].object_serial),
                (out_ports[1].object_serial, in_ports[0].object_serial),
            ];
        }

        // Case 3: Try matching by channel name (FL->FL, FR->FR, etc.)
        let mut matched = Vec::new();
        for out_p in &out_ports {
            if !out_p.channel.is_empty() && out_p.channel != "UNK" {
                if let Some(in_p) = in_ports.iter().find(|ip| ip.channel == out_p.channel) {
                    matched.push((out_p.object_serial, in_p.object_serial));
                }
            }
        }
        if !matched.is_empty() {
            return matched;
        }

        // Fallback: zip by index
        out_ports
            .iter()
            .zip(in_ports.iter())
            .map(|(o, i)| (o.object_serial, i.object_serial))
            .collect()
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
}
