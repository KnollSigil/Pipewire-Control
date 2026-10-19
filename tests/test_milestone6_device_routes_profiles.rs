#![allow(dead_code, unused_imports, unused_variables)]

#[path = "../src/model.rs"]
mod model;

#[path = "../src/backend.rs"]
mod backend;

use backend::{
    apply_device_param_pod, serialize_device_profile_pod, serialize_device_route_pod, Backend,
};
use model::{
    create_demo_graph, NodeCategory, PortDirection, PwAvailability, PwDevice, PwDeviceProfile,
    PwDeviceRoute, PwGraph, PwNode,
};
use pipewire::spa::{
    param::ParamType,
    pod::{deserialize::PodDeserializer, Object, Pod, Property, Value, ValueArray},
    sys as spa_sys,
    utils::SpaTypes,
};
use std::collections::BTreeMap;

// ============================================================================
// 1. Device Inventory & Node Association Tests (R6.1, R6.2)
// ============================================================================

#[test]
fn test_r6_demo_graph_device_inventory_and_node_association() {
    let graph = create_demo_graph();

    // Verify exactly 2 devices are registered on the demo graph
    assert_eq!(graph.devices.len(), 2, "Demo graph should have exactly 2 devices");
    assert!(graph.devices.contains_key(&1000), "Should contain Scarlett Solo (serial 1000)");
    assert!(graph.devices.contains_key(&5000), "Should contain Built-in Audio (serial 5000)");

    let scarlett = graph.devices.get(&1000).unwrap();
    assert_eq!(scarlett.id, 100);
    assert_eq!(scarlett.title(), "Scarlett Solo");
    assert_eq!(scarlett.name, "alsa_card.usb-Focusrite_Scarlett_Solo");

    let builtin = graph.devices.get(&5000).unwrap();
    assert_eq!(builtin.id, 500);
    assert_eq!(builtin.title(), "HDA Intel PCH");
    assert_eq!(builtin.name, "alsa_card.pci-0000_00_1f.3");

    // Verify node 10 is linked to Device 1000
    let node10_dev = graph.device_for_node(10);
    assert!(node10_dev.is_some(), "Node 10 must be associated with a device");
    assert_eq!(node10_dev.unwrap().object_serial, 1000);

    // Verify node 50 is linked to Device 5000
    let node50_dev = graph.device_for_node(50);
    assert!(node50_dev.is_some(), "Node 50 must be associated with a device");
    assert_eq!(node50_dev.unwrap().object_serial, 5000);

    // Verify stream / non-hardware nodes (20 Firefox, 30 Spotify, 40 EasyEffects, 60 Discord) have NO device
    assert!(graph.device_for_node(20).is_none(), "Firefox (node 20) is not device-backed");
    assert!(graph.device_for_node(30).is_none(), "Spotify (node 30) is not device-backed");
    assert!(graph.device_for_node(40).is_none(), "EasyEffects (node 40) is not device-backed");
    assert!(graph.device_for_node(60).is_none(), "Discord (node 60) is not device-backed");
    assert!(graph.device_for_node(9999).is_none(), "Non-existent node returns None");
}

#[test]
fn test_r6_listing_profiles_for_device_with_active_and_available_states() {
    let graph = create_demo_graph();
    let scarlett = graph.devices.get(&1000).expect("Scarlett Solo must exist");

    // Scarlett Solo Profiles: Off (0), Analog Stereo (1, active), Pro Audio (2), S/PDIF (3, unavailable)
    assert_eq!(scarlett.profiles.len(), 4);
    assert_eq!(scarlett.active_profile, Some(1));

    let p0 = &scarlett.profiles[0];
    assert_eq!(p0.index, 0);
    assert_eq!(p0.name, "off");
    assert_eq!(p0.display_label(), "Off");
    assert_eq!(p0.available, PwAvailability::Yes);
    assert!(!p0.active);

    let p1 = &scarlett.profiles[1];
    assert_eq!(p1.index, 1);
    assert_eq!(p1.name, "analog-stereo");
    assert_eq!(p1.display_label(), "Analog Stereo");
    assert_eq!(p1.available, PwAvailability::Yes);
    assert!(p1.active, "Profile 1 must be active initially");

    let p2 = &scarlett.profiles[2];
    assert_eq!(p2.index, 2);
    assert_eq!(p2.name, "pro-audio");
    assert_eq!(p2.display_label(), "Pro Audio");
    assert_eq!(p2.available, PwAvailability::Yes);
    assert!(!p2.active);

    let p3 = &scarlett.profiles[3];
    assert_eq!(p3.index, 3);
    assert_eq!(p3.name, "iec958-stereo");
    assert_eq!(p3.display_label(), "S/PDIF");
    assert_eq!(p3.available, PwAvailability::No, "S/PDIF should be unavailable");
    assert!(!p3.active);
    assert!(!p3.available.is_available());

    // Built-in Audio Profiles
    let builtin = graph.devices.get(&5000).expect("Built-in Audio must exist");
    assert_eq!(builtin.profiles.len(), 4);
    assert_eq!(builtin.active_profile, Some(1));
    assert!(builtin.profiles.iter().any(|p| p.name == "hdmi-stereo" && p.available == PwAvailability::No));
}

#[test]
fn test_r6_listing_routes_for_device_with_directions_and_availability() {
    let graph = create_demo_graph();
    let builtin = graph.devices.get(&5000).expect("Built-in Audio must exist");

    // Built-in Audio Routes:
    // 0: Headphones (Output, active, available)
    // 1: Internal Speakers (Output, inactive, available)
    // 2: Line Out (Output, inactive, unavailable)
    // 3: Internal Mic (Input, active, available)
    // 4: Headset Mic (Input, inactive, unavailable)
    assert_eq!(builtin.routes.len(), 5);

    let r_headphones = &builtin.routes[0];
    assert_eq!(r_headphones.index, 0);
    assert_eq!(r_headphones.direction, PortDirection::Output);
    assert_eq!(r_headphones.display_label(), "Headphones");
    assert!(r_headphones.active);
    assert!(r_headphones.available.is_available());

    let r_speakers = &builtin.routes[1];
    assert_eq!(r_speakers.index, 1);
    assert_eq!(r_speakers.direction, PortDirection::Output);
    assert_eq!(r_speakers.display_label(), "Internal Speakers");
    assert!(!r_speakers.active);
    assert!(r_speakers.available.is_available());

    let r_lineout = &builtin.routes[2];
    assert_eq!(r_lineout.index, 2);
    assert_eq!(r_lineout.direction, PortDirection::Output);
    assert_eq!(r_lineout.display_label(), "Line Out");
    assert!(!r_lineout.active);
    assert_eq!(r_lineout.available, PwAvailability::No);
    assert!(!r_lineout.available.is_available());

    let r_internal_mic = &builtin.routes[3];
    assert_eq!(r_internal_mic.index, 3);
    assert_eq!(r_internal_mic.direction, PortDirection::Input);
    assert_eq!(r_internal_mic.display_label(), "Internal Mic");
    assert!(r_internal_mic.active);
    assert!(r_internal_mic.available.is_available());

    let r_headset_mic = &builtin.routes[4];
    assert_eq!(r_headset_mic.index, 4);
    assert_eq!(r_headset_mic.direction, PortDirection::Input);
    assert_eq!(r_headset_mic.display_label(), "Headset Mic");
    assert!(!r_headset_mic.active);
    assert_eq!(r_headset_mic.available, PwAvailability::No);
}

// ============================================================================
// 2. Switching Active Profiles & Model Reflection (R6.2)
// ============================================================================

#[test]
fn test_r6_switching_active_profile_and_model_reflection() {
    let mut graph = create_demo_graph();
    let mut backend = Backend::demo();

    // Verify initial active profile is 1 (Analog Stereo)
    assert_eq!(graph.devices.get(&1000).unwrap().active_profile, Some(1));
    assert!(graph.devices.get(&1000).unwrap().profiles[1].active);
    assert!(!graph.devices.get(&1000).unwrap().profiles[2].active);

    // Switch Scarlett Solo to Pro Audio (profile index 2)
    let res = backend.set_device_profile(&mut graph, 1000, 2);
    assert!(res.is_ok(), "Switching to valid available profile must succeed");

    let scarlett = graph.devices.get(&1000).unwrap();
    assert_eq!(scarlett.active_profile, Some(2), "Active profile index must update to 2");
    assert!(!scarlett.profiles[1].active, "Old profile must be marked inactive");
    assert!(scarlett.profiles[2].active, "New profile must be marked active");
    assert!(!scarlett.profiles[0].active, "Other profiles remain inactive");
    assert!(backend.last_status.contains("Pro Audio"), "Status must reflect profile change: {}", backend.last_status);

    // Switch back to Off (index 0)
    let res2 = backend.set_device_profile(&mut graph, 1000, 0);
    assert!(res2.is_ok());
    let scarlett = graph.devices.get(&1000).unwrap();
    assert_eq!(scarlett.active_profile, Some(0));
    assert!(scarlett.profiles[0].active);
    assert!(!scarlett.profiles[2].active);
}

#[test]
fn test_r6_rejecting_unavailable_profile_selection() {
    let mut graph = create_demo_graph();
    let mut backend = Backend::demo();

    // Scarlett Solo profile 3 is S/PDIF (available = No)
    let initial_active = graph.devices.get(&1000).unwrap().active_profile;
    assert_eq!(initial_active, Some(1));

    let res = backend.set_device_profile(&mut graph, 1000, 3);
    assert!(res.is_err(), "Selecting an unavailable profile must return Err");
    let err_str = res.unwrap_err();
    assert!(err_str.contains("unavailable"), "Error message must state profile is unavailable: {err_str}");

    // Model must remain unchanged
    let scarlett = graph.devices.get(&1000).unwrap();
    assert_eq!(scarlett.active_profile, Some(1), "Active profile must not change on error");
    assert!(scarlett.profiles[1].active);
    assert!(!scarlett.profiles[3].active);

    // Built-in Audio profile 3 is HDMI Stereo (available = No)
    let res_builtin = backend.set_device_profile(&mut graph, 5000, 3);
    assert!(res_builtin.is_err());
    assert_eq!(graph.devices.get(&5000).unwrap().active_profile, Some(1));
}

// ============================================================================
// 3. Switching Active Routes & Directional Exclusivity (R6.2)
// ============================================================================

#[test]
fn test_r6_switching_active_route_and_model_reflection() {
    let mut graph = create_demo_graph();
    let mut backend = Backend::demo();

    // Built-in Audio initial state:
    // Route 0 (Headphones, Output): active
    // Route 1 (Speakers, Output): inactive
    // Route 3 (Internal Mic, Input): active
    let builtin = graph.devices.get(&5000).unwrap();
    assert!(builtin.routes[0].active);
    assert!(!builtin.routes[1].active);
    assert!(builtin.routes[3].active);

    // Switch Output route from Headphones (0) to Internal Speakers (1)
    let res = backend.set_device_route(&mut graph, 5000, 1, Some(500));
    assert!(res.is_ok(), "Switching to valid available route must succeed");

    let builtin = graph.devices.get(&5000).unwrap();
    assert!(!builtin.routes[0].active, "Headphones must be deactivated");
    assert!(builtin.routes[1].active, "Speakers must be activated");
    assert!(builtin.routes[3].active, "Internal Mic (Input) must remain active!");
    assert!(backend.last_status.contains("Internal Speakers"));
}

#[test]
fn test_r6_directional_exclusivity_input_vs_output_routes() {
    let mut graph = create_demo_graph();
    let mut backend = Backend::demo();

    // Verify initial states on Scarlett Solo:
    // Route 0: Mic (Input, active)
    // Route 1: Line In (Input, inactive)
    // Route 2: Headphones (Output, active)
    let scarlett = graph.devices.get(&1000).unwrap();
    assert!(scarlett.routes[0].active);
    assert!(!scarlett.routes[1].active);
    assert!(scarlett.routes[2].active);

    // 1. Switch Input route from Mic (0) to Line In (1)
    let res = backend.set_device_route(&mut graph, 1000, 1, Some(100));
    assert!(res.is_ok());

    let scarlett = graph.devices.get(&1000).unwrap();
    assert!(!scarlett.routes[0].active, "Mic input should now be inactive");
    assert!(scarlett.routes[1].active, "Line In input should now be active");
    assert!(scarlett.routes[2].active, "Headphones OUTPUT must remain untouched and active!");

    // 2. Switch back Input route to Mic (0)
    let res2 = backend.set_device_route(&mut graph, 1000, 0, Some(100));
    assert!(res2.is_ok());

    let scarlett = graph.devices.get(&1000).unwrap();
    assert!(scarlett.routes[0].active, "Mic input is active again");
    assert!(!scarlett.routes[1].active, "Line In is inactive");
    assert!(scarlett.routes[2].active, "Headphones OUTPUT is still active!");
}

#[test]
fn test_r6_rejecting_unavailable_route_selection() {
    let mut graph = create_demo_graph();
    let mut backend = Backend::demo();

    // Built-in Audio route 2 is Line Out (Output, available = No)
    let res = backend.set_device_route(&mut graph, 5000, 2, Some(500));
    assert!(res.is_err(), "Selecting unavailable route 2 must fail");
    let err_str = res.unwrap_err();
    assert!(err_str.contains("unavailable"), "Error message should mention unavailable: {err_str}");

    // State must remain unchanged: Headphones active, Speakers inactive
    let builtin = graph.devices.get(&5000).unwrap();
    assert!(builtin.routes[0].active, "Headphones must remain active");
    assert!(!builtin.routes[1].active, "Speakers must remain inactive");
    assert!(!builtin.routes[2].active, "Line Out must remain inactive");

    // Built-in Audio route 4 is Headset Mic (Input, available = No)
    let res_mic = backend.set_device_route(&mut graph, 5000, 4, Some(500));
    assert!(res_mic.is_err(), "Selecting unavailable route 4 must fail");
    let builtin = graph.devices.get(&5000).unwrap();
    assert!(builtin.routes[3].active, "Internal mic remains active");
    assert!(!builtin.routes[4].active, "Headset mic remains inactive");

    // Scarlett Solo route 3 is S/PDIF In (Input, available = No)
    let res_spdif = backend.set_device_route(&mut graph, 1000, 3, Some(100));
    assert!(res_spdif.is_err());
    assert!(!graph.devices.get(&1000).unwrap().routes[3].active);
}

// ============================================================================
// 4. Node Convenience Methods & Non-Device Error Handling (R6.2)
// ============================================================================

#[test]
fn test_r6_node_convenience_methods_and_non_device_errors() {
    let mut graph = create_demo_graph();
    let mut backend = Backend::demo();

    // 1. set_node_device_profile on Node 10 (Scarlett Solo)
    let res = backend.set_node_device_profile(&mut graph, 10, 2);
    assert!(res.is_ok());
    assert_eq!(graph.devices.get(&1000).unwrap().active_profile, Some(2));

    // 2. set_node_device_route on Node 50 (Built-in Audio)
    let res = backend.set_node_device_route(&mut graph, 50, 1, Some(500));
    assert!(res.is_ok());
    assert!(graph.devices.get(&5000).unwrap().routes[1].active);

    // 3. Attempting to set profile on non-device node (20 = Firefox)
    let res_firefox = backend.set_node_device_profile(&mut graph, 20, 1);
    assert!(res_firefox.is_err());
    let err = res_firefox.unwrap_err();
    assert!(err.contains("not backed by a device"), "Expected error about non-device node: {err}");

    // 4. Attempting to set route on non-device node (30 = Spotify)
    let res_spotify = backend.set_node_device_route(&mut graph, 30, 0, None);
    assert!(res_spotify.is_err());
    assert!(res_spotify.unwrap_err().contains("not backed by a device"));

    // 5. Non-existent node
    let res_missing = backend.set_node_device_profile(&mut graph, 9999, 0);
    assert!(res_missing.is_err());
}

#[test]
fn test_r6_missing_device_or_route_index_error_handling() {
    let mut graph = create_demo_graph();
    let mut backend = Backend::demo();

    // Non-existent device serial
    let res = backend.set_device_profile(&mut graph, 9999, 1);
    assert!(res.is_err());
    assert!(res.unwrap_err().contains("not found"));

    let res_route = backend.set_device_route(&mut graph, 9999, 0, None);
    assert!(res_route.is_err());
    assert!(res_route.unwrap_err().contains("not found"));

    // Existent device, non-existent profile index
    let res_p_idx = backend.set_device_profile(&mut graph, 1000, 99);
    assert!(res_p_idx.is_err());
    assert!(res_p_idx.unwrap_err().contains("Profile index 99 not found"));

    // Existent device, non-existent route index
    let res_r_idx = backend.set_device_route(&mut graph, 1000, 99, None);
    assert!(res_r_idx.is_err());
    assert!(res_r_idx.unwrap_err().contains("Route index 99 not found"));
}

// ============================================================================
// 5. Data Model Helpers & Availability Logic (R6.1)
// ============================================================================

#[test]
fn test_r6_availability_predicates_and_labels() {
    // PwAvailability::Yes
    assert!(PwAvailability::Yes.is_available());
    assert_eq!(PwAvailability::Yes.label(), "Available");
    assert_eq!(PwAvailability::from_spa_id(2), PwAvailability::Yes);

    // PwAvailability::Unknown
    assert!(PwAvailability::Unknown.is_available(), "Unknown availability is treated as available");
    assert_eq!(PwAvailability::Unknown.label(), "Unknown");
    assert_eq!(PwAvailability::from_spa_id(0), PwAvailability::Unknown);
    assert_eq!(PwAvailability::from_spa_id(999), PwAvailability::Unknown);

    // PwAvailability::No
    assert!(!PwAvailability::No.is_available());
    assert_eq!(PwAvailability::No.label(), "Unavailable");
    assert_eq!(PwAvailability::from_spa_id(1), PwAvailability::No);
}

#[test]
fn test_r6_device_profile_and_route_display_labels() {
    let p_with_desc = PwDeviceProfile {
        index: 1,
        name: "analog-stereo".to_string(),
        description: "Analog Stereo Duplex".to_string(),
        priority: 100,
        available: PwAvailability::Yes,
        active: true,
    };
    assert_eq!(p_with_desc.display_label(), "Analog Stereo Duplex");

    let p_no_desc = PwDeviceProfile {
        index: 2,
        name: "pro-audio".to_string(),
        description: "".to_string(),
        priority: 80,
        available: PwAvailability::Yes,
        active: false,
    };
    assert_eq!(p_no_desc.display_label(), "pro-audio");

    let r_with_desc = PwDeviceRoute {
        index: 0,
        name: "analog-output-headphones".to_string(),
        description: "Headphones".to_string(),
        direction: PortDirection::Output,
        available: PwAvailability::Yes,
        active: true,
        device_id: Some(100),
        devices: vec![0],
        profiles: vec![1],
        props: BTreeMap::new(),
    };
    assert_eq!(r_with_desc.display_label(), "Headphones");

    let r_no_desc = PwDeviceRoute {
        index: 1,
        name: "analog-input-linein".to_string(),
        description: "".to_string(),
        direction: PortDirection::Input,
        available: PwAvailability::Yes,
        active: false,
        device_id: Some(100),
        devices: vec![0],
        profiles: vec![1],
        props: BTreeMap::new(),
    };
    assert_eq!(r_no_desc.display_label(), "analog-input-linein");
}

#[test]
fn test_r6_device_title_resolution() {
    let dev_with_nick = PwDevice {
        id: 1,
        object_serial: 10,
        name: "alsa_card.usb-1".to_string(),
        description: "USB Audio Card".to_string(),
        nick: Some("Focusrite Solo".to_string()),
        media_class: None,
        props: BTreeMap::new(),
        profiles: Vec::new(),
        routes: Vec::new(),
        active_profile: None,
    };
    assert_eq!(dev_with_nick.title(), "Focusrite Solo");

    let dev_with_desc_only = PwDevice {
        id: 2,
        object_serial: 20,
        name: "alsa_card.pci-1".to_string(),
        description: "High Definition Audio Controller".to_string(),
        nick: None,
        media_class: None,
        props: BTreeMap::new(),
        profiles: Vec::new(),
        routes: Vec::new(),
        active_profile: None,
    };
    assert_eq!(dev_with_desc_only.title(), "High Definition Audio Controller");

    let dev_name_only = PwDevice {
        id: 3,
        object_serial: 30,
        name: "alsa_card.fallback".to_string(),
        description: "".to_string(),
        nick: None,
        media_class: None,
        props: BTreeMap::new(),
        profiles: Vec::new(),
        routes: Vec::new(),
        active_profile: None,
    };
    assert_eq!(dev_name_only.title(), "alsa_card.fallback");
}

#[test]
fn test_r6_graph_resolve_serials_links_device_id_to_serial() {
    let mut graph = PwGraph::default();

    // Insert a synthetic device with id: 42, serial: 4200
    graph.devices.insert(
        4200,
        PwDevice {
            id: 42,
            object_serial: 4200,
            name: "test_dev".to_string(),
            description: "Test Device".to_string(),
            nick: None,
            media_class: None,
            props: BTreeMap::new(),
            profiles: Vec::new(),
            routes: Vec::new(),
            active_profile: None,
        },
    );

    // Insert a node with props["device.id"] = "42", but device_id and device_serial None
    let mut props = BTreeMap::new();
    props.insert("device.id".to_string(), "42".to_string());
    graph.nodes.insert(
        999,
        PwNode {
            id: 10,
            object_serial: 999,
            name: "node_test".to_string(),
            description: "Node Test".to_string(),
            app_name: None,
            media_name: None,
            media_class: "Audio/Sink".to_string(),
            category: NodeCategory::AudioSink,
            state: "running".to_string(),
            input_ports: Vec::new(),
            output_ports: Vec::new(),
            volume: None,
            muted: None,
            channel_volumes: Vec::new(),
            channel_names: Vec::new(),
            props,
            params: Vec::new(),
            device_id: None,
            device_serial: None,
        },
    );

    // Run resolution
    graph.resolve_serials_and_rebuild_ports();

    let node = graph.nodes.get(&999).unwrap();
    assert_eq!(node.device_id, Some(42), "device_id should be parsed from props");
    assert_eq!(node.device_serial, Some(4200), "device_serial should be mapped from device with id 42");

    // Also test graph.device_for_node query
    let dev = graph.device_for_node(999);
    assert!(dev.is_some());
    assert_eq!(dev.unwrap().object_serial, 4200);
}

// ============================================================================
// 6. SPA Pod Serialization & Roundtrip Tests (R6.3)
// ============================================================================

#[test]
fn test_r6_serialize_device_profile_pod_roundtrip() {
    // Serialize profile index 2
    let pod_bytes = serialize_device_profile_pod(2).expect("Serialization must succeed");
    assert!(!pod_bytes.is_empty(), "Pod bytes should not be empty");

    // Deserialize via PodDeserializer
    let res = PodDeserializer::deserialize_from::<Value>(&pod_bytes);
    assert!(res.is_ok(), "Deserialization must succeed");

    let (_, val) = res.unwrap();
    let Value::Object(obj) = val else {
        panic!("Expected Value::Object");
    };

    assert_eq!(obj.type_, SpaTypes::ObjectParamProfile.as_raw(), "Type must be ObjectParamProfile");
    assert_eq!(obj.id, ParamType::Profile.as_raw(), "Id must be ParamType::Profile");

    // Check properties: index = 2, save = true
    let mut found_index = None;
    let mut found_save = None;
    for prop in &obj.properties {
        if prop.key == spa_sys::SPA_PARAM_PROFILE_index {
            if let Value::Int(i) = prop.value {
                found_index = Some(i);
            }
        } else if prop.key == spa_sys::SPA_PARAM_PROFILE_save {
            if let Value::Bool(b) = prop.value {
                found_save = Some(b);
            }
        }
    }

    assert_eq!(found_index, Some(2), "Deserialized profile index must be 2");
    assert_eq!(found_save, Some(true), "Deserialized save flag must be true");
}

#[test]
fn test_r6_serialize_device_route_pod_roundtrip_with_and_without_device_id() {
    // 1. With device_id = Some(500)
    let bytes_with_dev = serialize_device_route_pod(1, Some(500)).expect("Serialization must succeed");
    let (_, val) = PodDeserializer::deserialize_from::<Value>(&bytes_with_dev).unwrap();
    let Value::Object(obj) = val else {
        panic!("Expected Value::Object");
    };

    assert_eq!(obj.type_, SpaTypes::ObjectParamRoute.as_raw());
    assert_eq!(obj.id, ParamType::Route.as_raw());

    let mut found_index = None;
    let mut found_save = None;
    let mut found_dev = None;
    for prop in &obj.properties {
        if prop.key == spa_sys::SPA_PARAM_ROUTE_index {
            if let Value::Int(i) = prop.value {
                found_index = Some(i);
            }
        } else if prop.key == spa_sys::SPA_PARAM_ROUTE_save {
            if let Value::Bool(b) = prop.value {
                found_save = Some(b);
            }
        } else if prop.key == spa_sys::SPA_PARAM_ROUTE_device {
            if let Value::Int(d) = prop.value {
                found_dev = Some(d);
            }
        }
    }

    assert_eq!(found_index, Some(1));
    assert_eq!(found_save, Some(true));
    assert_eq!(found_dev, Some(500));

    // 2. Without device_id (None)
    let bytes_no_dev = serialize_device_route_pod(3, None).expect("Serialization must succeed");
    let (_, val2) = PodDeserializer::deserialize_from::<Value>(&bytes_no_dev).unwrap();
    let Value::Object(obj2) = val2 else {
        panic!("Expected Value::Object");
    };

    let mut found_index2 = None;
    let mut has_dev = false;
    for prop in &obj2.properties {
        if prop.key == spa_sys::SPA_PARAM_ROUTE_index {
            if let Value::Int(i) = prop.value {
                found_index2 = Some(i);
            }
        } else if prop.key == spa_sys::SPA_PARAM_ROUTE_device {
            has_dev = true;
        }
    }

    assert_eq!(found_index2, Some(3));
    assert!(!has_dev, "Should not contain device property when None");
}

// ============================================================================
// 7. Live Param Pod Application Emulation (R6.3)
// ============================================================================

#[test]
fn test_r6_apply_device_param_pod_live_emulation() {
    let mut device = PwDevice {
        id: 77,
        object_serial: 7700,
        name: "live_card".to_string(),
        description: "Live Sound Card".to_string(),
        nick: None,
        media_class: None,
        props: BTreeMap::new(),
        profiles: Vec::new(),
        routes: Vec::new(),
        active_profile: None,
    };

    // 1. Simulate incoming EnumProfile pod for profile 1 (Analog Stereo, available: Yes)
    let p1_obj = Value::Object(Object {
        type_: SpaTypes::ObjectParamProfile.as_raw(),
        id: ParamType::EnumProfile.as_raw(),
        properties: vec![
            Property::new(spa_sys::SPA_PARAM_PROFILE_index, Value::Int(1)),
            Property::new(spa_sys::SPA_PARAM_PROFILE_name, Value::String("analog-stereo".to_string())),
            Property::new(spa_sys::SPA_PARAM_PROFILE_description, Value::String("Analog Stereo Duplex".to_string())),
            Property::new(spa_sys::SPA_PARAM_PROFILE_priority, Value::Int(100)),
            Property::new(spa_sys::SPA_PARAM_PROFILE_available, Value::Int(2)), // 2 = Yes
        ],
    });
    let (c1, _) = pipewire::spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &p1_obj).unwrap();
    let buf1 = c1.into_inner();
    let pod1 = Pod::from_bytes(&buf1).unwrap();
    assert!(apply_device_param_pod(&mut device, ParamType::EnumProfile, pod1));
    assert_eq!(device.profiles.len(), 1);
    assert_eq!(device.profiles[0].name, "analog-stereo");
    assert_eq!(device.profiles[0].available, PwAvailability::Yes);

    // 2. Simulate incoming EnumProfile pod for profile 2 (S/PDIF, available: No)
    let p2_obj = Value::Object(Object {
        type_: SpaTypes::ObjectParamProfile.as_raw(),
        id: ParamType::EnumProfile.as_raw(),
        properties: vec![
            Property::new(spa_sys::SPA_PARAM_PROFILE_index, Value::Int(2)),
            Property::new(spa_sys::SPA_PARAM_PROFILE_name, Value::String("iec958-stereo".to_string())),
            Property::new(spa_sys::SPA_PARAM_PROFILE_description, Value::String("Digital S/PDIF".to_string())),
            Property::new(spa_sys::SPA_PARAM_PROFILE_priority, Value::Int(50)),
            Property::new(spa_sys::SPA_PARAM_PROFILE_available, Value::Int(1)), // 1 = No
        ],
    });
    let (c2, _) = pipewire::spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &p2_obj).unwrap();
    let buf2 = c2.into_inner();
    let pod2 = Pod::from_bytes(&buf2).unwrap();
    assert!(apply_device_param_pod(&mut device, ParamType::EnumProfile, pod2));
    assert_eq!(device.profiles.len(), 2);
    assert_eq!(device.profiles[1].available, PwAvailability::No);

    // 3. Simulate active Profile pod selecting profile 1
    let act_prof_bytes = serialize_device_profile_pod(1).unwrap();
    let pod_act_prof = Pod::from_bytes(&act_prof_bytes).unwrap();
    assert!(apply_device_param_pod(&mut device, ParamType::Profile, pod_act_prof));
    assert_eq!(device.active_profile, Some(1));
    assert!(device.profiles[0].active);
    assert!(!device.profiles[1].active);

    // 4. Simulate incoming EnumRoute pod for route 0 (Headphones, Output, available: Yes)
    let r0_obj = Value::Object(Object {
        type_: SpaTypes::ObjectParamRoute.as_raw(),
        id: ParamType::EnumRoute.as_raw(),
        properties: vec![
            Property::new(spa_sys::SPA_PARAM_ROUTE_index, Value::Int(0)),
            Property::new(spa_sys::SPA_PARAM_ROUTE_direction, Value::Int(1)), // 1 = Output
            Property::new(spa_sys::SPA_PARAM_ROUTE_name, Value::String("analog-output-headphones".to_string())),
            Property::new(spa_sys::SPA_PARAM_ROUTE_description, Value::String("Headphones".to_string())),
            Property::new(spa_sys::SPA_PARAM_ROUTE_priority, Value::Int(100)),
            Property::new(spa_sys::SPA_PARAM_ROUTE_available, Value::Int(2)), // 2 = Yes
            Property::new(spa_sys::SPA_PARAM_ROUTE_profiles, Value::ValueArray(ValueArray::Int(vec![1]))),
        ],
    });
    let (cr0, _) = pipewire::spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &r0_obj).unwrap();
    let buf_r0 = cr0.into_inner();
    let pod_r0 = Pod::from_bytes(&buf_r0).unwrap();
    assert!(apply_device_param_pod(&mut device, ParamType::EnumRoute, pod_r0));
    assert_eq!(device.routes.len(), 1);
    assert_eq!(device.routes[0].display_label(), "Headphones");
    assert_eq!(device.routes[0].direction, PortDirection::Output);

    // 5. Simulate active Route pod selecting route 0
    let act_route_bytes = serialize_device_route_pod(0, Some(77)).unwrap();
    let pod_act_route = Pod::from_bytes(&act_route_bytes).unwrap();
    assert!(apply_device_param_pod(&mut device, ParamType::Route, pod_act_route));
    assert!(device.routes[0].active);
}

#[test]
fn test_r6_enum_route_devices_array_and_direction_exclusivity() {
    let mut device = PwDevice {
        id: 88,
        object_serial: 8800,
        name: "test_card".to_string(),
        description: "Test Card".to_string(),
        nick: None,
        media_class: None,
        props: BTreeMap::new(),
        profiles: Vec::new(),
        routes: Vec::new(),
        active_profile: Some(1),
    };

    // 1. Output route 0: Headphones (using SPA_PARAM_ROUTE_devices array [0])
    let r0_obj = Value::Object(Object {
        type_: SpaTypes::ObjectParamRoute.as_raw(),
        id: ParamType::EnumRoute.as_raw(),
        properties: vec![
            Property::new(spa_sys::SPA_PARAM_ROUTE_index, Value::Int(0)),
            Property::new(spa_sys::SPA_PARAM_ROUTE_direction, Value::Int(1)), // Output
            Property::new(spa_sys::SPA_PARAM_ROUTE_name, Value::String("analog-output-headphones".to_string())),
            Property::new(spa_sys::SPA_PARAM_ROUTE_description, Value::String("Headphones".to_string())),
            Property::new(spa_sys::SPA_PARAM_ROUTE_available, Value::Int(2)),
            Property::new(spa_sys::SPA_PARAM_ROUTE_devices, Value::ValueArray(ValueArray::Int(vec![0]))),
        ],
    });
    let (cr0, _) = pipewire::spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &r0_obj).unwrap();
    let buf0 = cr0.into_inner();
    let pod_r0 = Pod::from_bytes(&buf0).unwrap();
    assert!(apply_device_param_pod(&mut device, ParamType::EnumRoute, pod_r0));
    assert_eq!(device.routes[0].device_id, Some(0), "device_id must be parsed from SPA_PARAM_ROUTE_devices");

    // 2. Output route 1: Speakers
    let r1_obj = Value::Object(Object {
        type_: SpaTypes::ObjectParamRoute.as_raw(),
        id: ParamType::EnumRoute.as_raw(),
        properties: vec![
            Property::new(spa_sys::SPA_PARAM_ROUTE_index, Value::Int(1)),
            Property::new(spa_sys::SPA_PARAM_ROUTE_direction, Value::Int(1)), // Output
            Property::new(spa_sys::SPA_PARAM_ROUTE_name, Value::String("analog-output-speakers".to_string())),
            Property::new(spa_sys::SPA_PARAM_ROUTE_description, Value::String("Internal Speakers".to_string())),
            Property::new(spa_sys::SPA_PARAM_ROUTE_available, Value::Int(2)),
            Property::new(spa_sys::SPA_PARAM_ROUTE_devices, Value::ValueArray(ValueArray::Int(vec![0]))),
        ],
    });
    let (cr1, _) = pipewire::spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &r1_obj).unwrap();
    let buf1 = cr1.into_inner();
    let pod_r1 = Pod::from_bytes(&buf1).unwrap();
    assert!(apply_device_param_pod(&mut device, ParamType::EnumRoute, pod_r1));

    // 3. Input route 2: Internal Mic
    let r2_obj = Value::Object(Object {
        type_: SpaTypes::ObjectParamRoute.as_raw(),
        id: ParamType::EnumRoute.as_raw(),
        properties: vec![
            Property::new(spa_sys::SPA_PARAM_ROUTE_index, Value::Int(2)),
            Property::new(spa_sys::SPA_PARAM_ROUTE_direction, Value::Int(0)), // Input
            Property::new(spa_sys::SPA_PARAM_ROUTE_name, Value::String("analog-input-mic".to_string())),
            Property::new(spa_sys::SPA_PARAM_ROUTE_description, Value::String("Internal Mic".to_string())),
            Property::new(spa_sys::SPA_PARAM_ROUTE_available, Value::Int(2)),
            Property::new(spa_sys::SPA_PARAM_ROUTE_devices, Value::ValueArray(ValueArray::Int(vec![0]))),
        ],
    });
    let (cr2, _) = pipewire::spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &r2_obj).unwrap();
    let buf2 = cr2.into_inner();
    let pod_r2 = Pod::from_bytes(&buf2).unwrap();
    assert!(apply_device_param_pod(&mut device, ParamType::EnumRoute, pod_r2));

    // 4. Activate Output Route 0 (Headphones)
    let act_out0 = serialize_device_route_pod(0, Some(0)).unwrap();
    assert!(apply_device_param_pod(&mut device, ParamType::Route, Pod::from_bytes(&act_out0).unwrap()));

    // 5. Activate Input Route 2 (Internal Mic)
    let act_in2 = serialize_device_route_pod(2, Some(0)).unwrap();
    assert!(apply_device_param_pod(&mut device, ParamType::Route, Pod::from_bytes(&act_in2).unwrap()));

    // Verify BOTH route 0 (Output) AND route 2 (Input) are concurrently active!
    let r0 = device.routes.iter().find(|r| r.index == 0).unwrap();
    let r1 = device.routes.iter().find(|r| r.index == 1).unwrap();
    let r2 = device.routes.iter().find(|r| r.index == 2).unwrap();
    assert!(r0.active, "Output route 0 must be active");
    assert!(!r1.active, "Output route 1 must be inactive");
    assert!(r2.active, "Input route 2 must be active concurrently (direction exclusivity)");

    // 6. Switch Output route to route 1 (Speakers)
    let act_out1 = serialize_device_route_pod(1, Some(0)).unwrap();
    assert!(apply_device_param_pod(&mut device, ParamType::Route, Pod::from_bytes(&act_out1).unwrap()));

    let r0_after = device.routes.iter().find(|r| r.index == 0).unwrap();
    let r1_after = device.routes.iter().find(|r| r.index == 1).unwrap();
    let r2_after = device.routes.iter().find(|r| r.index == 2).unwrap();
    assert!(!r0_after.active, "Output route 0 must become inactive");
    assert!(r1_after.active, "Output route 1 must become active");
    assert!(r2_after.active, "Input route 2 must remain active untouched by output route switch");
}

#[test]
fn test_r6_profile_off_device_state_and_restoration() {
    let mut graph = create_demo_graph();
    let mut backend = Backend::default();
    backend.is_demo = true;

    // Simulate Built-in Audio device (serial 5000) switching to Profile 0 ("off")
    backend.set_device_profile(&mut graph, 5000, 0).unwrap();

    let builtin = graph.devices.get(&5000).unwrap();
    assert_eq!(builtin.active_profile, Some(0));
    assert!(builtin.profiles.iter().find(|p| p.index == 0).unwrap().active);
    assert!(!builtin.profiles.iter().find(|p| p.index == 1).unwrap().active);

    // Now switch Built-in Audio device back to Profile 1 ("analog-stereo")
    backend.set_device_profile(&mut graph, 5000, 1).unwrap();
    let builtin_restored = graph.devices.get(&5000).unwrap();
    assert_eq!(builtin_restored.active_profile, Some(1));
    assert!(builtin_restored.profiles.iter().find(|p| p.index == 1).unwrap().active);
    assert!(!builtin_restored.profiles.iter().find(|p| p.index == 0).unwrap().active);
}

#[test]
fn test_r6_routes_for_active_profile_filtering() {
    let mut graph = create_demo_graph();

    let scarlett = graph.devices.get_mut(&1000).unwrap();
    // Initially active profile is 1 (Analog Stereo)
    assert_eq!(scarlett.active_profile_index(), Some(1));

    // Routes with profiles: [1, 2] should be included; S/PDIF with profiles: [3] must be excluded!
    let active_routes: Vec<_> = scarlett.routes_for_active_profile().collect();
    assert_eq!(active_routes.len(), 3);
    assert!(active_routes.iter().any(|r| r.name == "analog-input-mic"));
    assert!(active_routes.iter().any(|r| r.name == "analog-input-linein"));
    assert!(active_routes.iter().any(|r| r.name == "analog-output-headphones"));
    assert!(!active_routes.iter().any(|r| r.name == "iec958-input"));

    // When switched to profile 3 (S/PDIF)
    scarlett.active_profile = Some(3);
    for p in &mut scarlett.profiles {
        p.active = p.index == 3;
    }
    let spdif_routes: Vec<_> = scarlett.routes_for_active_profile().collect();
    assert_eq!(spdif_routes.len(), 1);
    assert_eq!(spdif_routes[0].name, "iec958-input");

    // When switched to profile 0 (Off)
    scarlett.active_profile = Some(0);
    for p in &mut scarlett.profiles {
        p.active = p.index == 0;
    }
    assert_eq!(scarlett.routes_for_active_profile().count(), 0);
}

#[test]
fn test_r6_unavailable_routes_and_profiles_filtering() {
    let graph = create_demo_graph();
    let builtin = graph.devices.get(&5000).unwrap();

    // Built-in device has profiles:
    // 0: Off (available: Yes)
    // 1: Analog Stereo (available: Yes, active)
    // 2: Pro Audio (available: Yes)
    // 3: HDMI Stereo (available: No)
    let available_profiles: Vec<_> = builtin
        .profiles
        .iter()
        .filter(|p| p.available != PwAvailability::No || p.active)
        .collect();
    assert_eq!(available_profiles.len(), 3);
    assert!(!available_profiles.iter().any(|p| p.name == "hdmi-stereo"));

    // Built-in device active profile routes:
    // 0: Headphones (Output, available: Yes, active: true)
    // 1: Speakers (Output, available: Yes)
    // 2: Line Out (Output, available: No)
    // 3: Internal Mic (Input, available: Yes, active: true)
    // 4: Headset Mic (Input, available: No)
    let active_out_routes: Vec<_> = builtin
        .routes_for_active_profile()
        .filter(|r| r.direction == PortDirection::Output)
        .filter(|r| r.available != PwAvailability::No || r.active)
        .collect();
    assert_eq!(active_out_routes.len(), 2);
    assert_eq!(active_out_routes[0].name, "analog-output-headphones");
    assert_eq!(active_out_routes[1].name, "analog-output-speaker");

    let active_in_routes: Vec<_> = builtin
        .routes_for_active_profile()
        .filter(|r| r.direction == PortDirection::Input)
        .filter(|r| r.available != PwAvailability::No || r.active)
        .collect();
    assert_eq!(active_in_routes.len(), 1);
    assert_eq!(active_in_routes[0].name, "analog-input-internal-mic");
}
