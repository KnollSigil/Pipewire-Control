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
    PwDeviceRoute, PwGraph, PwLink, PwNode, PwPort,
};
use pipewire::spa::{
    param::ParamType,
    pod::{deserialize::PodDeserializer, serialize::PodSerializer, Object, Pod, Property, Value, ValueArray},
    sys as spa_sys,
    utils::SpaTypes,
};
use std::collections::BTreeMap;
use std::io::Cursor;

// ============================================================================
// Helpers
// ============================================================================

fn make_node(
    serial: u64,
    name: &str,
    category: NodeCategory,
    device_id: Option<u32>,
) -> PwNode {
    PwNode {
        id: serial as u32,
        object_serial: serial,
        name: name.to_string(),
        description: name.to_string(),
        app_name: None,
        media_name: None,
        media_class: match category {
            NodeCategory::AudioSource => "Audio/Source".to_string(),
            NodeCategory::AudioSink => "Audio/Sink".to_string(),
            NodeCategory::AudioOutputStream => "Stream/Output/Audio".to_string(),
            NodeCategory::AudioInputStream => "Stream/Input/Audio".to_string(),
            NodeCategory::AudioDuplex => "Audio/Duplex".to_string(),
            NodeCategory::Midi => "Midi/Bridge".to_string(),
            NodeCategory::Video => "Video/Source".to_string(),
            NodeCategory::Internal => "Internal".to_string(),
        },
        category,
        input_ports: Vec::new(),
        output_ports: Vec::new(),
        state: "running".to_string(),
        volume: Some(1.0),
        muted: Some(false),
        channel_names: vec!["FL".to_string(), "FR".to_string()],
        channel_volumes: vec![1.0, 1.0],
        props: BTreeMap::new(),
        params: Vec::new(),
        device_id,
        device_serial: None,
    }
}

// ============================================================================
// Section 1: State Transitions & Idempotency
// ============================================================================

/// Verifies that re-selecting an already-active profile is idempotent and safe.
#[test]
fn test_adversarial_profile_idempotent_toggle() {
    let mut graph = create_demo_graph();
    let mut backend = Backend::demo();

    let scarlett_serial = 1000;
    // Initial active profile is 1 (Analog Stereo)
    assert_eq!(
        graph.devices.get(&scarlett_serial).unwrap().active_profile,
        Some(1)
    );
    assert!(graph.devices.get(&scarlett_serial).unwrap().profiles[1].active);

    // Re-select active profile 1 repeatedly
    for _ in 0..10 {
        let res = backend.set_device_profile(&mut graph, scarlett_serial, 1);
        assert!(res.is_ok(), "Selecting already-active profile must succeed");

        let dev = graph.devices.get(&scarlett_serial).unwrap();
        assert_eq!(dev.active_profile, Some(1));

        let active_profiles: Vec<_> = dev.profiles.iter().filter(|p| p.active).collect();
        assert_eq!(
            active_profiles.len(),
            1,
            "Exactly one profile must be active after redundant toggle"
        );
        assert_eq!(active_profiles[0].index, 1);
        assert!(backend.last_status.contains("Analog Stereo"));
    }
}

/// Verifies that re-selecting an already-active route is idempotent and preserves directional exclusivity.
#[test]
fn test_adversarial_route_idempotent_toggle() {
    let mut graph = create_demo_graph();
    let mut backend = Backend::demo();

    let builtin_serial = 5000;
    // Built-in Audio initial state:
    // Route 0: Headphones (Output, active)
    // Route 3: Internal Mic (Input, active)
    assert!(graph.devices.get(&builtin_serial).unwrap().routes[0].active);
    assert!(graph.devices.get(&builtin_serial).unwrap().routes[3].active);

    // 1. Re-select active Output route 0 repeatedly
    for _ in 0..10 {
        let res = backend.set_device_route(&mut graph, builtin_serial, 0, Some(500));
        assert!(res.is_ok(), "Selecting already-active Output route must succeed");

        let dev = graph.devices.get(&builtin_serial).unwrap();
        assert!(dev.routes[0].active, "Output route 0 must remain active");
        assert!(!dev.routes[1].active, "Output route 1 must remain inactive");
        assert!(!dev.routes[2].active, "Output route 2 must remain inactive");
        assert!(dev.routes[3].active, "Input route 3 must remain active!");
        assert!(!dev.routes[4].active, "Input route 4 must remain inactive");
    }

    // 2. Re-select active Input route 3 repeatedly
    for _ in 0..10 {
        let res = backend.set_device_route(&mut graph, builtin_serial, 3, Some(500));
        assert!(res.is_ok(), "Selecting already-active Input route must succeed");

        let dev = graph.devices.get(&builtin_serial).unwrap();
        assert!(dev.routes[3].active, "Input route 3 must remain active");
        assert!(!dev.routes[4].active, "Input route 4 must remain inactive");
        assert!(dev.routes[0].active, "Output route 0 must remain active!");
        assert!(!dev.routes[1].active, "Output route 1 must remain inactive");
    }
}

/// Stress-tests rapid cyclic transitions across all available profiles.
#[test]
fn test_adversarial_profile_cyclic_transitions_rapid() {
    let mut graph = create_demo_graph();
    let mut backend = Backend::demo();

    let scarlett_serial = 1000;
    // Available profiles on Scarlett Solo: 0 (Off), 1 (Analog Stereo), 2 (Pro Audio)
    let sequence = [0, 1, 2, 0, 2, 1, 1, 0, 2, 0];

    for cycle in 0..20 {
        for &target_idx in &sequence {
            let res = backend.set_device_profile(&mut graph, scarlett_serial, target_idx);
            assert!(
                res.is_ok(),
                "Cycle {cycle}: Switching to available profile {target_idx} must succeed"
            );

            let dev = graph.devices.get(&scarlett_serial).unwrap();
            assert_eq!(dev.active_profile, Some(target_idx));

            let active_profiles: Vec<_> = dev.profiles.iter().filter(|p| p.active).collect();
            assert_eq!(
                active_profiles.len(),
                1,
                "Cycle {cycle}: Exactly one profile must be active, found {}",
                active_profiles.len()
            );
            assert_eq!(active_profiles[0].index, target_idx);
        }
    }
}

/// Verifies that switching device profiles does NOT inadvertently reset or mutate device route states.
#[test]
fn test_adversarial_interleaved_profile_and_route_transitions() {
    let mut graph = create_demo_graph();
    let mut backend = Backend::demo();

    let builtin_serial = 5000;

    // Switch route to Speakers (1)
    assert!(backend.set_device_route(&mut graph, builtin_serial, 1, Some(500)).is_ok());
    assert!(graph.devices.get(&builtin_serial).unwrap().routes[1].active);
    assert!(!graph.devices.get(&builtin_serial).unwrap().routes[0].active);

    // Switch profile to Off (0)
    assert!(backend.set_device_profile(&mut graph, builtin_serial, 0).is_ok());

    // Verify route states are preserved
    let dev = graph.devices.get(&builtin_serial).unwrap();
    assert_eq!(dev.active_profile, Some(0));
    assert!(dev.routes[1].active, "Speakers route should still be active");
    assert!(!dev.routes[0].active, "Headphones route should still be inactive");
    assert!(dev.routes[3].active, "Internal mic route should still be active");

    // Switch profile to Pro Audio (2)
    assert!(backend.set_device_profile(&mut graph, builtin_serial, 2).is_ok());

    // Switch route back to Headphones (0)
    assert!(backend.set_device_route(&mut graph, builtin_serial, 0, Some(500)).is_ok());

    let dev = graph.devices.get(&builtin_serial).unwrap();
    assert_eq!(dev.active_profile, Some(2));
    assert!(dev.routes[0].active);
    assert!(!dev.routes[1].active);
    assert!(dev.routes[3].active);
}

// ============================================================================
// Section 2: Boundary Conditions & Negative Inputs
// ============================================================================

/// Tests invalid device serials against profile and route setters, ensuring rejection and graph immutability.
#[test]
fn test_adversarial_invalid_device_serials() {
    let mut graph = create_demo_graph();
    let devices_before = graph.devices.clone();
    let nodes_count_before = graph.nodes.len();
    let links_count_before = graph.links.len();
    let mut backend = Backend::demo();

    let invalid_serials = [0, 1, 42, 9999, u64::MAX, 1000 - 1, 5000 + 1];

    for &bad_serial in &invalid_serials {
        // Profile setter with invalid serial
        let res_prof = backend.set_device_profile(&mut graph, bad_serial, 0);
        assert!(res_prof.is_err());
        let err_prof = res_prof.unwrap_err();
        assert!(
            err_prof.contains(&format!("Device (serial #{bad_serial}) not found")),
            "Expected device not found error, got: {err_prof}"
        );

        // Route setter with invalid serial
        let res_route = backend.set_device_route(&mut graph, bad_serial, 0, None);
        assert!(res_route.is_err());
        let err_route = res_route.unwrap_err();
        assert!(
            err_route.contains(&format!("Device (serial #{bad_serial}) not found")),
            "Expected device not found error, got: {err_route}"
        );
    }

    // Graph devices and structure must remain strictly identical
    assert_eq!(graph.devices, devices_before, "Devices were modified despite invalid device serials");
    assert_eq!(graph.nodes.len(), nodes_count_before);
    assert_eq!(graph.links.len(), links_count_before);
}

/// Tests nonexistent and out-of-range route indices (negative, overflow, foreign device indices).
#[test]
fn test_adversarial_invalid_route_indices() {
    let mut graph = create_demo_graph();
    let devices_before = graph.devices.clone();
    let mut backend = Backend::demo();

    let scarlett_serial = 1000;
    // Scarlett Solo only has routes 0, 1, 2, 3
    let bad_indices = [i32::MIN, -999, -1, 4, 5, 10, 100, i32::MAX];

    for &bad_idx in &bad_indices {
        let res = backend.set_device_route(&mut graph, scarlett_serial, bad_idx, Some(100));
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(
            err.contains(&format!("Route index {bad_idx} not found on device 'Scarlett Solo'")),
            "Unexpected error format: {err}"
        );
    }

    assert_eq!(graph.devices, devices_before, "Devices were modified despite invalid route indices");
}

/// Tests nonexistent and out-of-range profile indices (negative, overflow, foreign device indices).
#[test]
fn test_adversarial_invalid_profile_indices() {
    let mut graph = create_demo_graph();
    let devices_before = graph.devices.clone();
    let mut backend = Backend::demo();

    let builtin_serial = 5000;
    // Built-in Audio only has profiles 0, 1, 2, 3
    let bad_indices = [i32::MIN, -999, -1, 4, 10, 9999, i32::MAX];

    for &bad_idx in &bad_indices {
        let res = backend.set_device_profile(&mut graph, builtin_serial, bad_idx);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(
            err.contains(&format!("Profile index {bad_idx} not found on device 'HDA Intel PCH'")),
            "Unexpected error format: {err}"
        );
    }

    assert_eq!(graph.devices, devices_before, "Devices were modified despite invalid profile indices");
}

/// Tests variations of `device_id` argument in `set_device_route`.
#[test]
fn test_adversarial_device_id_boundary_values() {
    let mut graph = create_demo_graph();
    let mut backend = Backend::demo();

    let builtin_serial = 5000;
    let dev_ids = [None, Some(0), Some(1), Some(-1), Some(i32::MIN), Some(i32::MAX)];

    for (step, &dev_id) in dev_ids.iter().enumerate() {
        // Toggle between route 0 and route 1
        let target_idx = (step % 2) as i32;
        let res = backend.set_device_route(&mut graph, builtin_serial, target_idx, dev_id);
        assert!(
            res.is_ok(),
            "Setting route with device_id={dev_id:?} must succeed"
        );
        let dev = graph.devices.get(&builtin_serial).unwrap();
        assert!(dev.routes[target_idx as usize].active);
    }
}

/// Tests degenerate and empty devices (0 profiles, 0 routes, empty string names).
#[test]
fn test_adversarial_empty_and_degenerate_devices() {
    let mut graph = PwGraph::default();
    let mut backend = Backend::demo();

    let empty_device_serial = 7777;
    let empty_device = PwDevice {
        id: 77,
        object_serial: empty_device_serial,
        name: String::new(),
        description: String::new(),
        nick: None,
        media_class: None,
        props: BTreeMap::new(),
        profiles: Vec::new(),
        routes: Vec::new(),
        active_profile: None,
    };
    graph.devices.insert(empty_device_serial, empty_device);

    let dev = graph.devices.get(&empty_device_serial).unwrap();
    assert_eq!(dev.title(), "", "Empty device title should safely evaluate to empty string");

    // Setting profile on empty device must return error gracefully
    let res_p = backend.set_device_profile(&mut graph, empty_device_serial, 0);
    assert!(res_p.is_err());
    assert!(res_p.unwrap_err().contains("Profile index 0 not found"));

    // Setting route on empty device must return error gracefully
    let res_r = backend.set_device_route(&mut graph, empty_device_serial, 0, None);
    assert!(res_r.is_err());
    assert!(res_r.unwrap_err().contains("Route index 0 not found"));

    // Verify empty profile and route display labels
    let empty_profile = PwDeviceProfile {
        index: 0,
        name: String::new(),
        description: String::new(),
        priority: 0,
        available: PwAvailability::Unknown,
        active: false,
    };
    assert_eq!(empty_profile.display_label(), "");

    let empty_route = PwDeviceRoute {
        index: 0,
        name: String::new(),
        description: String::new(),
        direction: PortDirection::Output,
        available: PwAvailability::Unknown,
        active: false,
        device_id: None,
        profiles: Vec::new(),
        props: BTreeMap::new(),
    };
    assert_eq!(empty_route.display_label(), "");
}

// ============================================================================
// Section 3: Directional Exclusivity & Rapid Interleaved Switches
// ============================================================================

/// Rapidly interleaves switches between Output route on Built-in Audio and Input route on Scarlett Solo
/// for 1,000 iterations, verifying directional exclusivity and device isolation on every step.
#[test]
fn test_adversarial_rapid_interleaved_route_switching_1000_iterations() {
    let mut graph = create_demo_graph();
    let mut backend = Backend::demo();

    let builtin_serial = 5000;
    let scarlett_serial = 1000;

    let mut last_builtin_out = 0;
    let mut last_scarlett_in = 0;

    for i in 0..1000 {
        if i % 2 == 0 {
            // Switch Built-in Audio Output route between 0 (Headphones) and 1 (Speakers)
            let target_out = if last_builtin_out == 0 { 1 } else { 0 };
            let res = backend.set_device_route(&mut graph, builtin_serial, target_out, Some(500));
            assert!(res.is_ok(), "Iteration {i}: Failed setting Built-in Audio Output to {target_out}");
            last_builtin_out = target_out;
        } else {
            // Switch Scarlett Solo Input route between 0 (Mic) and 1 (Line In)
            let target_in = if last_scarlett_in == 0 { 1 } else { 0 };
            let res = backend.set_device_route(&mut graph, scarlett_serial, target_in, Some(100));
            assert!(res.is_ok(), "Iteration {i}: Failed setting Scarlett Input to {target_in}");
            last_scarlett_in = target_in;
        }

        // Rigorous invariant checks at every iteration:
        let builtin = graph.devices.get(&builtin_serial).unwrap();
        assert_eq!(
            builtin.routes[0].active,
            last_builtin_out == 0,
            "Built-in route 0 active mismatch at iteration {i}"
        );
        assert_eq!(
            builtin.routes[1].active,
            last_builtin_out == 1,
            "Built-in route 1 active mismatch at iteration {i}"
        );
        assert!(!builtin.routes[2].active, "Unavailable Line Out must NEVER be active");
        assert!(
            builtin.routes[3].active,
            "Built-in Input route 3 (Internal Mic) MUST remain active at iteration {i}!"
        );
        assert!(!builtin.routes[4].active, "Unavailable Headset Mic must NEVER be active");

        let scarlett = graph.devices.get(&scarlett_serial).unwrap();
        assert_eq!(
            scarlett.routes[0].active,
            last_scarlett_in == 0,
            "Scarlett route 0 active mismatch at iteration {i}"
        );
        assert_eq!(
            scarlett.routes[1].active,
            last_scarlett_in == 1,
            "Scarlett route 1 active mismatch at iteration {i}"
        );
        assert!(
            scarlett.routes[2].active,
            "Scarlett Output route 2 (Headphones) MUST remain active at iteration {i}!"
        );
        assert!(!scarlett.routes[3].active, "Unavailable S/PDIF In must NEVER be active");
    }
}

/// Stress-tests directional exclusivity on a synthetic complex device with 10 Inputs and 10 Outputs.
#[test]
fn test_adversarial_synthetic_multi_route_matrix_stress() {
    let mut graph = PwGraph::default();
    let mut backend = Backend::demo();

    let test_serial = 9999;
    let mut routes = Vec::new();

    // 10 Output routes (indices 0..10)
    for i in 0..10 {
        routes.push(PwDeviceRoute {
            index: i,
            name: format!("out_{i}"),
            description: format!("Output Port {i}"),
            direction: PortDirection::Output,
            available: PwAvailability::Yes,
            active: i == 0,
            device_id: Some(99),
            profiles: vec![1],
            props: BTreeMap::new(),
        });
    }

    // 10 Input routes (indices 10..20)
    for i in 10..20 {
        routes.push(PwDeviceRoute {
            index: i,
            name: format!("in_{i}"),
            description: format!("Input Port {i}"),
            direction: PortDirection::Input,
            available: PwAvailability::Yes,
            active: i == 10,
            device_id: Some(99),
            profiles: vec![1],
            props: BTreeMap::new(),
        });
    }

    let dev = PwDevice {
        id: 99,
        object_serial: test_serial,
        name: String::from("synthetic_card"),
        description: String::from("Synthetic Multi-Route Card"),
        nick: None,
        media_class: None,
        props: BTreeMap::new(),
        profiles: vec![PwDeviceProfile {
            index: 1,
            name: String::from("pro-audio"),
            description: String::from("Pro Audio"),
            priority: 1,
            available: PwAvailability::Yes,
            active: true,
        }],
        routes,
        active_profile: Some(1),
    };
    graph.devices.insert(test_serial, dev);

    let mut current_active_out = 0;
    let mut current_active_in = 10;

    // Pseudo-random sequence of route switches
    for step in 1..200 {
        let is_output_switch = step % 3 != 0;
        if is_output_switch {
            let target_out = (step * 7) % 10;
            let res = backend.set_device_route(&mut graph, test_serial, target_out, Some(99));
            assert!(res.is_ok());
            current_active_out = target_out;
        } else {
            let target_in = 10 + ((step * 11) % 10);
            let res = backend.set_device_route(&mut graph, test_serial, target_in, Some(99));
            assert!(res.is_ok());
            current_active_in = target_in;
        }

        let dev = graph.devices.get(&test_serial).unwrap();

        // Verify Output routes
        let active_outputs: Vec<_> = dev
            .routes
            .iter()
            .filter(|r| r.direction == PortDirection::Output && r.active)
            .map(|r| r.index)
            .collect();
        assert_eq!(
            active_outputs,
            vec![current_active_out],
            "Step {step}: Output route active set mismatch"
        );

        // Verify Input routes
        let active_inputs: Vec<_> = dev
            .routes
            .iter()
            .filter(|r| r.direction == PortDirection::Input && r.active)
            .map(|r| r.index)
            .collect();
        assert_eq!(
            active_inputs,
            vec![current_active_in],
            "Step {step}: Input route active set mismatch"
        );
    }
}

// ============================================================================
// Section 4: Rejection and Error Messaging for Unavailable Items
// ============================================================================

/// Exhaustively tests every unavailable profile and route in the demo graph, verifying rejection,
/// exact error message contents, and backend status reflection.
#[test]
fn test_adversarial_exhaustive_unavailable_rejection_across_devices() {
    let mut graph = create_demo_graph();
    let devices_snapshot = graph.devices.clone();
    let mut backend = Backend::demo();

    // 1. Scarlett Solo Profile 3: S/PDIF (available = No)
    let res = backend.set_device_profile(&mut graph, 1000, 3);
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert_eq!(
        err,
        "Cannot select unavailable profile 'S/PDIF' on 'Scarlett Solo'"
    );
    assert_eq!(backend.last_status, err);

    // 2. Scarlett Solo Route 3: S/PDIF In (available = No)
    let res = backend.set_device_route(&mut graph, 1000, 3, Some(100));
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert_eq!(
        err,
        "Cannot select unavailable route 'S/PDIF In' on 'Scarlett Solo'"
    );
    assert_eq!(backend.last_status, err);

    // 3. Built-in Audio Profile 3: HDMI Stereo (available = No)
    let res = backend.set_device_profile(&mut graph, 5000, 3);
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert_eq!(
        err,
        "Cannot select unavailable profile 'HDMI Stereo' on 'HDA Intel PCH'"
    );
    assert_eq!(backend.last_status, err);

    // 4. Built-in Audio Route 2: Line Out (available = No)
    let res = backend.set_device_route(&mut graph, 5000, 2, Some(500));
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert_eq!(
        err,
        "Cannot select unavailable route 'Line Out' on 'HDA Intel PCH'"
    );
    assert_eq!(backend.last_status, err);

    // 5. Built-in Audio Route 4: Headset Mic (available = No)
    let res = backend.set_device_route(&mut graph, 5000, 4, Some(500));
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert_eq!(
        err,
        "Cannot select unavailable route 'Headset Mic' on 'HDA Intel PCH'"
    );
    assert_eq!(backend.last_status, err);

    // Verify zero mutations across device inventories
    assert_eq!(
        graph.devices, devices_snapshot,
        "Devices were modified by rejected unavailable selections"
    );
}

/// Stress-tests repeated rejections (50 consecutive calls) ensuring state remains completely intact.
#[test]
fn test_adversarial_repeated_rejection_stress() {
    let mut graph = create_demo_graph();
    let initial_devices = graph.devices.clone();
    let mut backend = Backend::demo();

    for _ in 0..50 {
        assert!(backend.set_device_profile(&mut graph, 1000, 3).is_err());
        assert!(backend.set_device_route(&mut graph, 1000, 3, Some(100)).is_err());
        assert!(backend.set_device_profile(&mut graph, 5000, 3).is_err());
        assert!(backend.set_device_route(&mut graph, 5000, 2, Some(500)).is_err());
        assert!(backend.set_device_route(&mut graph, 5000, 4, Some(500)).is_err());
    }

    assert_eq!(
        graph.devices, initial_devices,
        "Devices were corrupted after repeated rejection attempts"
    );
}

/// Tests that `PwAvailability::Unknown` is selectable while `PwAvailability::No` is strictly rejected.
#[test]
fn test_adversarial_availability_enum_semantics() {
    let mut graph = PwGraph::default();
    let mut backend = Backend::demo();

    let dev_serial = 8888;
    let dev = PwDevice {
        id: 88,
        object_serial: dev_serial,
        name: String::from("test_device"),
        description: String::from("Test Device"),
        nick: None,
        media_class: None,
        props: BTreeMap::new(),
        profiles: vec![
            PwDeviceProfile {
                index: 0,
                name: String::from("prof_unknown"),
                description: String::from("Unknown Profile"),
                priority: 1,
                available: PwAvailability::Unknown,
                active: false,
            },
            PwDeviceProfile {
                index: 1,
                name: String::from("prof_no"),
                description: String::from("Unavailable Profile"),
                priority: 2,
                available: PwAvailability::No,
                active: false,
            },
            PwDeviceProfile {
                index: 2,
                name: String::from("prof_yes"),
                description: String::from("Available Profile"),
                priority: 3,
                available: PwAvailability::Yes,
                active: false,
            },
        ],
        routes: vec![
            PwDeviceRoute {
                index: 0,
                name: String::from("route_unknown"),
                description: String::from("Unknown Route"),
                direction: PortDirection::Output,
                available: PwAvailability::Unknown,
                active: false,
                device_id: None,
                profiles: vec![],
                props: BTreeMap::new(),
            },
            PwDeviceRoute {
                index: 1,
                name: String::from("route_no"),
                description: String::from("Unavailable Route"),
                direction: PortDirection::Output,
                available: PwAvailability::No,
                active: false,
                device_id: None,
                profiles: vec![],
                props: BTreeMap::new(),
            },
            PwDeviceRoute {
                index: 2,
                name: String::from("route_yes"),
                description: String::from("Available Route"),
                direction: PortDirection::Output,
                available: PwAvailability::Yes,
                active: false,
                device_id: None,
                profiles: vec![],
                props: BTreeMap::new(),
            },
        ],
        active_profile: None,
    };
    graph.devices.insert(dev_serial, dev);

    // Unknown Profile: selectable
    assert!(backend.set_device_profile(&mut graph, dev_serial, 0).is_ok());
    assert_eq!(graph.devices.get(&dev_serial).unwrap().active_profile, Some(0));

    // Unavailable Profile: rejected
    assert!(backend.set_device_profile(&mut graph, dev_serial, 1).is_err());
    assert_eq!(graph.devices.get(&dev_serial).unwrap().active_profile, Some(0));

    // Available Profile: selectable
    assert!(backend.set_device_profile(&mut graph, dev_serial, 2).is_ok());
    assert_eq!(graph.devices.get(&dev_serial).unwrap().active_profile, Some(2));

    // Unknown Route: selectable
    assert!(backend.set_device_route(&mut graph, dev_serial, 0, None).is_ok());
    assert!(graph.devices.get(&dev_serial).unwrap().routes[0].active);

    // Unavailable Route: rejected
    assert!(backend.set_device_route(&mut graph, dev_serial, 1, None).is_err());
    assert!(graph.devices.get(&dev_serial).unwrap().routes[0].active);
    assert!(!graph.devices.get(&dev_serial).unwrap().routes[1].active);

    // Available Route: selectable
    assert!(backend.set_device_route(&mut graph, dev_serial, 2, None).is_ok());
    assert!(!graph.devices.get(&dev_serial).unwrap().routes[0].active);
    assert!(graph.devices.get(&dev_serial).unwrap().routes[2].active);
}

// ============================================================================
// Section 5: SPA Pod Serialization & Live Emulation Adversarial Stress
// ============================================================================

/// Stress-tests SPA pod serialization with boundary index and device_id values.
#[test]
fn test_adversarial_pod_serializer_extreme_values() {
    let test_indices = [i32::MIN, -100, -1, 0, 1, 42, 9999, i32::MAX];

    // 1. Profile pods
    for &idx in &test_indices {
        let bytes = serialize_device_profile_pod(idx).expect("Profile serialization must succeed");
        assert!(!bytes.is_empty());

        let Ok((_, Value::Object(obj))) =
            PodDeserializer::deserialize_from::<Value>(&bytes)
        else {
            panic!("Deserialization of profile pod must yield Object");
        };
        assert_eq!(obj.type_, SpaTypes::ObjectParamProfile.as_raw());
        assert_eq!(obj.id, ParamType::Profile.as_raw());

        let idx_prop = obj
            .properties
            .iter()
            .find(|p| p.key == spa_sys::SPA_PARAM_PROFILE_index)
            .expect("Index prop must be present");
        assert_eq!(idx_prop.value, Value::Int(idx));

        let save_prop = obj
            .properties
            .iter()
            .find(|p| p.key == spa_sys::SPA_PARAM_PROFILE_save)
            .expect("Save prop must be present");
        assert_eq!(save_prop.value, Value::Bool(true));
    }

    // 2. Route pods with extreme device_id options
    let test_dev_ids = [None, Some(i32::MIN), Some(-1), Some(0), Some(i32::MAX)];
    for &idx in &test_indices {
        for &dev_id in &test_dev_ids {
            let bytes = serialize_device_route_pod(idx, dev_id).expect("Route serialization must succeed");
            assert!(!bytes.is_empty());

            let Ok((_, Value::Object(obj))) =
                PodDeserializer::deserialize_from::<Value>(&bytes)
            else {
                panic!("Deserialization of route pod must yield Object");
            };
            assert_eq!(obj.type_, SpaTypes::ObjectParamRoute.as_raw());
            assert_eq!(obj.id, ParamType::Route.as_raw());

            let idx_prop = obj
                .properties
                .iter()
                .find(|p| p.key == spa_sys::SPA_PARAM_ROUTE_index)
                .expect("Index prop must be present");
            assert_eq!(idx_prop.value, Value::Int(idx));

            let dev_prop = obj
                .properties
                .iter()
                .find(|p| p.key == spa_sys::SPA_PARAM_ROUTE_device);
            match dev_id {
                Some(expected_dev) => {
                    assert!(dev_prop.is_some());
                    assert_eq!(dev_prop.unwrap().value, Value::Int(expected_dev));
                }
                None => {
                    assert!(dev_prop.is_none());
                }
            }
        }
    }
}

/// Tests `apply_device_param_pod` against corrupt, empty, and non-device pods.
#[test]
fn test_adversarial_apply_device_param_pod_malformed_input() {
    let mut dev = PwDevice {
        id: 100,
        object_serial: 1000,
        name: String::from("dev"),
        description: String::from("dev"),
        nick: None,
        media_class: None,
        props: BTreeMap::new(),
        profiles: Vec::new(),
        routes: Vec::new(),
        active_profile: None,
    };

    // 1. Pod with irrelevant ParamType (e.g. Props or Format)
    let profile_bytes = serialize_device_profile_pod(1).unwrap();
    let pod = Pod::from_bytes(&profile_bytes).unwrap();
    let res_props = apply_device_param_pod(&mut dev, ParamType::Props, pod);
    assert!(!res_props, "Applying profile pod with ParamType::Props must return false");

    let res_format = apply_device_param_pod(&mut dev, ParamType::Format, pod);
    assert!(!res_format, "Applying profile pod with ParamType::Format must return false");

    // 2. Pod without index property (EnumProfile)
    let no_index_obj = Value::Object(Object {
        type_: SpaTypes::ObjectParamProfile.as_raw(),
        id: ParamType::EnumProfile.as_raw(),
        properties: vec![
            Property::new(spa_sys::SPA_PARAM_PROFILE_name, Value::String(String::from("test"))),
        ],
    });
    let (cursor, _) = PodSerializer::serialize(Cursor::new(Vec::new()), &no_index_obj).unwrap();
    let buf_no_idx = cursor.into_inner();
    let pod_no_idx = Pod::from_bytes(&buf_no_idx).unwrap();
    let res_no_idx = apply_device_param_pod(&mut dev, ParamType::EnumProfile, pod_no_idx);
    assert!(!res_no_idx, "Pod missing index must return false");
    assert!(dev.profiles.is_empty());

    // 3. Pod with corrupt property types (e.g. String for index instead of Int)
    let bad_type_obj = Value::Object(Object {
        type_: SpaTypes::ObjectParamProfile.as_raw(),
        id: ParamType::EnumProfile.as_raw(),
        properties: vec![
            Property::new(spa_sys::SPA_PARAM_PROFILE_index, Value::String(String::from("not_an_int"))),
        ],
    });
    let (cursor_bad, _) = PodSerializer::serialize(Cursor::new(Vec::new()), &bad_type_obj).unwrap();
    let buf_bad = cursor_bad.into_inner();
    let pod_bad_type = Pod::from_bytes(&buf_bad).unwrap();
    let res_bad = apply_device_param_pod(&mut dev, ParamType::EnumProfile, pod_bad_type);
    assert!(!res_bad, "Pod with wrong type for index must return false");
}

/// Verifies that re-enumerating profiles/routes via `apply_device_param_pod` preserves the active status.
#[test]
fn test_adversarial_apply_device_param_pod_preserves_active_state_on_update() {
    let mut dev = PwDevice {
        id: 100,
        object_serial: 1000,
        name: String::from("dev"),
        description: String::from("dev"),
        nick: None,
        media_class: None,
        props: BTreeMap::new(),
        profiles: Vec::new(),
        routes: Vec::new(),
        active_profile: Some(2),
    };

    // 1. Initial EnumProfile pod for profile 2
    let obj1 = Value::Object(Object {
        type_: SpaTypes::ObjectParamProfile.as_raw(),
        id: ParamType::EnumProfile.as_raw(),
        properties: vec![
            Property::new(spa_sys::SPA_PARAM_PROFILE_index, Value::Int(2)),
            Property::new(spa_sys::SPA_PARAM_PROFILE_name, Value::String(String::from("pro-audio"))),
            Property::new(spa_sys::SPA_PARAM_PROFILE_description, Value::String(String::from("Pro Audio"))),
        ],
    });
    let (c1, _) = PodSerializer::serialize(Cursor::new(Vec::new()), &obj1).unwrap();
    let b1 = c1.into_inner();
    assert!(apply_device_param_pod(&mut dev, ParamType::EnumProfile, Pod::from_bytes(&b1).unwrap()));
    assert!(dev.profiles[0].active, "Profile 2 should be active because dev.active_profile == Some(2)");

    // 2. Updated EnumProfile pod arrives (e.g. description updated)
    let obj2 = Value::Object(Object {
        type_: SpaTypes::ObjectParamProfile.as_raw(),
        id: ParamType::EnumProfile.as_raw(),
        properties: vec![
            Property::new(spa_sys::SPA_PARAM_PROFILE_index, Value::Int(2)),
            Property::new(spa_sys::SPA_PARAM_PROFILE_name, Value::String(String::from("pro-audio"))),
            Property::new(spa_sys::SPA_PARAM_PROFILE_description, Value::String(String::from("Pro Audio Updated"))),
        ],
    });
    let (c2, _) = PodSerializer::serialize(Cursor::new(Vec::new()), &obj2).unwrap();
    let b2 = c2.into_inner();
    assert!(apply_device_param_pod(&mut dev, ParamType::EnumProfile, Pod::from_bytes(&b2).unwrap()));
    assert_eq!(dev.profiles[0].description, "Pro Audio Updated");
    assert!(dev.profiles[0].active, "Active status must NOT be lost on EnumProfile update");

    // 3. Route enumeration: route index 0 (Output)
    let route_obj = Value::Object(Object {
        type_: SpaTypes::ObjectParamRoute.as_raw(),
        id: ParamType::EnumRoute.as_raw(),
        properties: vec![
            Property::new(spa_sys::SPA_PARAM_ROUTE_index, Value::Int(0)),
            Property::new(spa_sys::SPA_PARAM_ROUTE_direction, Value::Int(1)), // Output
            Property::new(spa_sys::SPA_PARAM_ROUTE_name, Value::String(String::from("headphones"))),
        ],
    });
    let (cr, _) = PodSerializer::serialize(Cursor::new(Vec::new()), &route_obj).unwrap();
    let br = cr.into_inner();
    assert!(apply_device_param_pod(&mut dev, ParamType::EnumRoute, Pod::from_bytes(&br).unwrap()));

    // Make route active via ParamType::Route
    let active_route_obj = Value::Object(Object {
        type_: SpaTypes::ObjectParamRoute.as_raw(),
        id: ParamType::Route.as_raw(),
        properties: vec![
            Property::new(spa_sys::SPA_PARAM_ROUTE_index, Value::Int(0)),
            Property::new(spa_sys::SPA_PARAM_ROUTE_direction, Value::Int(1)),
        ],
    });
    let (car, _) = PodSerializer::serialize(Cursor::new(Vec::new()), &active_route_obj).unwrap();
    let bar = car.into_inner();
    assert!(apply_device_param_pod(&mut dev, ParamType::Route, Pod::from_bytes(&bar).unwrap()));
    assert!(dev.routes[0].active);

    // Re-send EnumRoute pod with updated description
    let route_update = Value::Object(Object {
        type_: SpaTypes::ObjectParamRoute.as_raw(),
        id: ParamType::EnumRoute.as_raw(),
        properties: vec![
            Property::new(spa_sys::SPA_PARAM_ROUTE_index, Value::Int(0)),
            Property::new(spa_sys::SPA_PARAM_ROUTE_direction, Value::Int(1)),
            Property::new(spa_sys::SPA_PARAM_ROUTE_name, Value::String(String::from("headphones"))),
            Property::new(spa_sys::SPA_PARAM_ROUTE_description, Value::String(String::from("Studio Headphones"))),
        ],
    });
    let (cru, _) = PodSerializer::serialize(Cursor::new(Vec::new()), &route_update).unwrap();
    let bru = cru.into_inner();
    assert!(apply_device_param_pod(&mut dev, ParamType::EnumRoute, Pod::from_bytes(&bru).unwrap()));
    assert_eq!(dev.routes[0].description, "Studio Headphones");
    assert!(dev.routes[0].active, "Route active status must NOT be reset to false on EnumRoute update");
}

// ============================================================================
// Section 6: Node Convenience Methods & Graph Resolution Stress
// ============================================================================

/// Tests `resolve_serials_and_rebuild_ports` when multiple nodes map to the same device,
/// and verifies node convenience methods operate on the shared device.
#[test]
fn test_adversarial_node_device_resolution_and_chaining() {
    let mut graph = PwGraph::default();
    let mut backend = Backend::demo();

    // Create a device with id 42, serial 4200
    let dev = PwDevice {
        id: 42,
        object_serial: 4200,
        name: String::from("alsa_card.shared"),
        description: String::from("Shared Audio Device"),
        nick: None,
        media_class: None,
        props: BTreeMap::new(),
        profiles: vec![
            PwDeviceProfile {
                index: 0,
                name: String::from("off"),
                description: String::from("Off"),
                priority: 0,
                available: PwAvailability::Yes,
                active: false,
            },
            PwDeviceProfile {
                index: 1,
                name: String::from("stereo"),
                description: String::from("Stereo Duplex"),
                priority: 1,
                available: PwAvailability::Yes,
                active: true,
            },
        ],
        routes: vec![
            PwDeviceRoute {
                index: 0,
                name: String::from("speaker"),
                description: String::from("Speaker"),
                direction: PortDirection::Output,
                available: PwAvailability::Yes,
                active: true,
                device_id: Some(42),
                profiles: vec![1],
                props: BTreeMap::new(),
            },
            PwDeviceRoute {
                index: 1,
                name: String::from("headphones"),
                description: String::from("Headphones"),
                direction: PortDirection::Output,
                available: PwAvailability::Yes,
                active: false,
                device_id: Some(42),
                profiles: vec![1],
                props: BTreeMap::new(),
            },
            PwDeviceRoute {
                index: 2,
                name: String::from("mic"),
                description: String::from("Microphone"),
                direction: PortDirection::Input,
                available: PwAvailability::Yes,
                active: true,
                device_id: Some(42),
                profiles: vec![1],
                props: BTreeMap::new(),
            },
        ],
        active_profile: Some(1),
    };
    graph.devices.insert(4200, dev);

    // Node 1: Sink Node (device_id 42)
    let sink_node = make_node(1010, "alsa_output", NodeCategory::AudioSink, Some(42));
    graph.nodes.insert(1010, sink_node);

    // Node 2: Source Node (device_id 42)
    let source_node = make_node(1020, "alsa_input", NodeCategory::AudioSource, Some(42));
    graph.nodes.insert(1020, source_node);

    // Node 3: Unconnected Stream Node (no device_id)
    let stream_node = make_node(1030, "stream_client", NodeCategory::AudioOutputStream, None);
    graph.nodes.insert(1030, stream_node);

    // Node 4: Orphan Node with non-existent device_id (999)
    let orphan_node = make_node(1040, "orphan_client", NodeCategory::AudioOutputStream, Some(999));
    graph.nodes.insert(1040, orphan_node);

    // Resolve serials
    graph.resolve_serials_and_rebuild_ports();

    // Verify serial resolutions
    assert_eq!(graph.nodes.get(&1010).unwrap().device_serial, Some(4200));
    assert_eq!(graph.nodes.get(&1020).unwrap().device_serial, Some(4200));
    assert_eq!(graph.nodes.get(&1030).unwrap().device_serial, None);
    assert_eq!(graph.nodes.get(&1040).unwrap().device_serial, None);

    // Both node 1010 and 1020 resolve to the shared device
    assert_eq!(graph.device_for_node(1010).unwrap().object_serial, 4200);
    assert_eq!(graph.device_for_node(1020).unwrap().object_serial, 4200);

    // Change profile via Sink node (1010)
    assert!(backend.set_node_device_profile(&mut graph, 1010, 0).is_ok());
    assert_eq!(graph.devices.get(&4200).unwrap().active_profile, Some(0));

    // Verify Source node (1020) also observes the new profile
    assert_eq!(
        graph.device_for_node(1020).unwrap().active_profile,
        Some(0)
    );

    // Switch route via Source node (1020)
    assert!(backend.set_node_device_route(&mut graph, 1020, 1, Some(42)).is_ok());
    assert!(graph.devices.get(&4200).unwrap().routes[1].active);
    assert!(!graph.devices.get(&4200).unwrap().routes[0].active);
    assert!(graph.devices.get(&4200).unwrap().routes[2].active, "Microphone (Input) stays active");

    // Convenience setters on non-device nodes must return error
    assert!(backend.set_node_device_profile(&mut graph, 1030, 0).is_err());
    assert!(backend.set_node_device_route(&mut graph, 1030, 0, None).is_err());
    assert!(backend.set_node_device_profile(&mut graph, 1040, 0).is_err());
    assert!(backend.set_node_device_route(&mut graph, 1040, 0, None).is_err());
}
