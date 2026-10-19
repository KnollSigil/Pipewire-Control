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
    pod::{deserialize::PodDeserializer, Object, Pod, Property, Value, ValueArray},
    sys as spa_sys,
    utils::SpaTypes,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Cursor;

// ============================================================================
// Deterministic Pseudo-Random Generator (LCG) for adversarial testing & fuzzing
// ============================================================================
struct TestRng {
    state: u64,
}

impl TestRng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u32(&mut self) -> u32 {
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.state >> 32) as u32
    }

    fn next_u8(&mut self) -> u8 {
        (self.next_u32() & 0xFF) as u8
    }

    fn next_range(&mut self, min: usize, max: usize) -> usize {
        assert!(min <= max);
        if min == max {
            return min;
        }
        let range = max - min + 1;
        min + (self.next_u32() as usize % range)
    }

    fn next_bool(&mut self) -> bool {
        (self.next_u32() & 1) == 1
    }
}

// ============================================================================
// SECTION 1: SPA Pod Serialization Byte Layout Verification
// ============================================================================

#[test]
fn test_profile_pod_exact_byte_layout_and_alignment() {
    let test_index = 42i32;
    let bytes = serialize_device_profile_pod(test_index)
        .expect("serialize_device_profile_pod must succeed");

    // SPA Pods MUST be padded and aligned to 8-byte boundaries
    assert_eq!(
        bytes.len() % 8,
        0,
        "Pod buffer length must be a multiple of 8"
    );
    assert_eq!(
        bytes.len(),
        64,
        "Profile pod with index and save must be exactly 64 bytes"
    );

    // 1. Pod Header (8 bytes: size: u32, type: u32)
    let pod_size = u32::from_ne_bytes(bytes[0..4].try_into().unwrap());
    let pod_type = u32::from_ne_bytes(bytes[4..8].try_into().unwrap());
    assert_eq!(
        pod_size as usize,
        bytes.len() - 8,
        "Pod header size must reflect body size (64 - 8 = 56)"
    );
    assert_eq!(
        pod_type,
        SpaTypes::Object.as_raw(),
        "Pod type must be SpaTypes::Object"
    );

    // 2. Object Header (8 bytes: type: u32, id: u32)
    let obj_type = u32::from_ne_bytes(bytes[8..12].try_into().unwrap());
    let obj_id = u32::from_ne_bytes(bytes[12..16].try_into().unwrap());
    assert_eq!(
        obj_type,
        SpaTypes::ObjectParamProfile.as_raw(),
        "Object type must be ObjectParamProfile"
    );
    assert_eq!(
        obj_id,
        ParamType::Profile.as_raw(),
        "Object id must be ParamType::Profile"
    );

    // 3. Property 1: SPA_PARAM_PROFILE_index
    // key: u32 (16..20), flags: u32 (20..24), pod size: u32 (24..28), pod type: u32 (28..32), value: i32 (32..36), pad: 4 (36..40)
    let prop1_key = u32::from_ne_bytes(bytes[16..20].try_into().unwrap());
    let prop1_flags = u32::from_ne_bytes(bytes[20..24].try_into().unwrap());
    let prop1_val_size = u32::from_ne_bytes(bytes[24..28].try_into().unwrap());
    let prop1_val_type = u32::from_ne_bytes(bytes[28..32].try_into().unwrap());
    let prop1_val = i32::from_ne_bytes(bytes[32..36].try_into().unwrap());
    let prop1_pad = &bytes[36..40];

    assert_eq!(
        prop1_key,
        spa_sys::SPA_PARAM_PROFILE_index,
        "Prop 1 key must be SPA_PARAM_PROFILE_index"
    );
    assert_eq!(prop1_flags, 0, "Prop 1 flags must be 0");
    assert_eq!(prop1_val_size, 4, "Prop 1 value pod size must be 4");
    assert_eq!(
        prop1_val_type,
        SpaTypes::Int.as_raw(),
        "Prop 1 type must be SpaTypes::Int"
    );
    assert_eq!(prop1_val, test_index, "Prop 1 value must match input index");
    assert_eq!(prop1_pad, &[0, 0, 0, 0], "Prop 1 4-byte padding must be zeroed");

    // 4. Property 2: SPA_PARAM_PROFILE_save
    // key: u32 (40..44), flags: u32 (44..48), pod size: u32 (48..52), pod type: u32 (52..56), value: i32 (56..60), pad: 4 (60..64)
    let prop2_key = u32::from_ne_bytes(bytes[40..44].try_into().unwrap());
    let prop2_flags = u32::from_ne_bytes(bytes[44..48].try_into().unwrap());
    let prop2_val_size = u32::from_ne_bytes(bytes[48..52].try_into().unwrap());
    let prop2_val_type = u32::from_ne_bytes(bytes[52..56].try_into().unwrap());
    let prop2_val = i32::from_ne_bytes(bytes[56..60].try_into().unwrap());
    let prop2_pad = &bytes[60..64];

    assert_eq!(
        prop2_key,
        spa_sys::SPA_PARAM_PROFILE_save,
        "Prop 2 key must be SPA_PARAM_PROFILE_save"
    );
    assert_eq!(prop2_flags, 0, "Prop 2 flags must be 0");
    assert_eq!(prop2_val_size, 4, "Prop 2 value pod size must be 4");
    assert_eq!(
        prop2_val_type,
        SpaTypes::Bool.as_raw(),
        "Prop 2 type must be SpaTypes::Bool"
    );
    assert_eq!(prop2_val, 1, "Prop 2 value must be 1 (true)");
    assert_eq!(prop2_pad, &[0, 0, 0, 0], "Prop 2 4-byte padding must be zeroed");
}

#[test]
fn test_route_pod_exact_byte_layout_with_device_id() {
    let test_route = 7i32;
    let test_device = 500i32;
    let bytes = serialize_device_route_pod(test_route, Some(test_device))
        .expect("serialize_device_route_pod must succeed");

    assert_eq!(bytes.len() % 8, 0, "Pod buffer length must be multiple of 8");
    assert_eq!(
        bytes.len(),
        88,
        "Route pod with index, save, and device must be exactly 88 bytes"
    );

    // Pod header
    let pod_size = u32::from_ne_bytes(bytes[0..4].try_into().unwrap());
    let pod_type = u32::from_ne_bytes(bytes[4..8].try_into().unwrap());
    assert_eq!(pod_size as usize, bytes.len() - 8);
    assert_eq!(pod_type, SpaTypes::Object.as_raw());

    // Object header
    let obj_type = u32::from_ne_bytes(bytes[8..12].try_into().unwrap());
    let obj_id = u32::from_ne_bytes(bytes[12..16].try_into().unwrap());
    assert_eq!(obj_type, SpaTypes::ObjectParamRoute.as_raw());
    assert_eq!(obj_id, ParamType::Route.as_raw());

    // Prop 1: index
    let p1_key = u32::from_ne_bytes(bytes[16..20].try_into().unwrap());
    let p1_type = u32::from_ne_bytes(bytes[28..32].try_into().unwrap());
    let p1_val = i32::from_ne_bytes(bytes[32..36].try_into().unwrap());
    assert_eq!(p1_key, spa_sys::SPA_PARAM_ROUTE_index);
    assert_eq!(p1_type, SpaTypes::Int.as_raw());
    assert_eq!(p1_val, test_route);
    assert_eq!(&bytes[36..40], &[0, 0, 0, 0]);

    // Prop 2: save
    let p2_key = u32::from_ne_bytes(bytes[40..44].try_into().unwrap());
    let p2_type = u32::from_ne_bytes(bytes[52..56].try_into().unwrap());
    let p2_val = i32::from_ne_bytes(bytes[56..60].try_into().unwrap());
    assert_eq!(p2_key, spa_sys::SPA_PARAM_ROUTE_save);
    assert_eq!(p2_type, SpaTypes::Bool.as_raw());
    assert_eq!(p2_val, 1);
    assert_eq!(&bytes[60..64], &[0, 0, 0, 0]);

    // Prop 3: device
    let p3_key = u32::from_ne_bytes(bytes[64..68].try_into().unwrap());
    let p3_type = u32::from_ne_bytes(bytes[76..80].try_into().unwrap());
    let p3_val = i32::from_ne_bytes(bytes[80..84].try_into().unwrap());
    assert_eq!(p3_key, spa_sys::SPA_PARAM_ROUTE_device);
    assert_eq!(p3_type, SpaTypes::Int.as_raw());
    assert_eq!(p3_val, test_device);
    assert_eq!(&bytes[84..88], &[0, 0, 0, 0]);
}

#[test]
fn test_route_pod_exact_byte_layout_without_device_id() {
    let test_route = 15i32;
    let bytes = serialize_device_route_pod(test_route, None)
        .expect("serialize_device_route_pod must succeed");

    assert_eq!(bytes.len() % 8, 0);
    assert_eq!(
        bytes.len(),
        64,
        "Route pod without device must be exactly 64 bytes (no 24-byte device prop)"
    );

    let pod_size = u32::from_ne_bytes(bytes[0..4].try_into().unwrap());
    assert_eq!(pod_size, 56);

    let obj_type = u32::from_ne_bytes(bytes[8..12].try_into().unwrap());
    let obj_id = u32::from_ne_bytes(bytes[12..16].try_into().unwrap());
    assert_eq!(obj_type, SpaTypes::ObjectParamRoute.as_raw());
    assert_eq!(obj_id, ParamType::Route.as_raw());

    let p1_val = i32::from_ne_bytes(bytes[32..36].try_into().unwrap());
    assert_eq!(p1_val, test_route);
    let p2_val = i32::from_ne_bytes(bytes[56..60].try_into().unwrap());
    assert_eq!(p2_val, 1);
}

// ============================================================================
// SECTION 2: Roundtrip Deserialization & Boundary Conditions
// ============================================================================

#[test]
fn test_profile_pod_roundtrip_boundaries_and_extremes() {
    let edge_cases = [
        0,
        1,
        2,
        -1,
        -100,
        i32::MAX,
        i32::MIN,
        1000000,
        -1000000,
        127,
        -128,
        32767,
        -32768,
    ];

    for &index in &edge_cases {
        let bytes = serialize_device_profile_pod(index)
            .unwrap_or_else(|e| panic!("Failed to serialize index {index}: {e}"));

        let (rem, val) = PodDeserializer::deserialize_from::<Value>(&bytes)
            .unwrap_or_else(|e| panic!("Failed to deserialize index {index}: {e:?}"));
        assert!(rem.is_empty(), "Should consume entire buffer for index {index}");

        let Value::Object(obj) = val else {
            panic!("Expected Value::Object for index {index}");
        };

        assert_eq!(obj.type_, SpaTypes::ObjectParamProfile.as_raw());
        assert_eq!(obj.id, ParamType::Profile.as_raw());

        let mut roundtripped_index = None;
        let mut roundtripped_save = None;

        for prop in &obj.properties {
            if prop.key == spa_sys::SPA_PARAM_PROFILE_index {
                if let Value::Int(i) = prop.value {
                    roundtripped_index = Some(i);
                }
            } else if prop.key == spa_sys::SPA_PARAM_PROFILE_save {
                if let Value::Bool(b) = prop.value {
                    roundtripped_save = Some(b);
                }
            }
        }

        assert_eq!(
            roundtripped_index,
            Some(index),
            "Profile index must roundtrip losslessly for {index}"
        );
        assert_eq!(
            roundtripped_save,
            Some(true),
            "Save flag must remain true for {index}"
        );
    }
}

#[test]
fn test_route_pod_roundtrip_boundaries_and_extremes() {
    let route_indices = [0, 1, -1, i32::MAX, i32::MIN, 42, -500];
    let device_ids = [
        None,
        Some(0),
        Some(1),
        Some(-1),
        Some(i32::MAX),
        Some(i32::MIN),
        Some(500),
    ];

    for &route_idx in &route_indices {
        for &dev_id in &device_ids {
            let bytes = serialize_device_route_pod(route_idx, dev_id).unwrap_or_else(|e| {
                panic!("Failed to serialize route {route_idx}, dev {dev_id:?}: {e}")
            });

            let (rem, val) = PodDeserializer::deserialize_from::<Value>(&bytes).unwrap_or_else(|e| {
                panic!("Failed to deserialize route {route_idx}, dev {dev_id:?}: {e:?}")
            });
            assert!(rem.is_empty());

            let Value::Object(obj) = val else {
                panic!("Expected Value::Object");
            };

            assert_eq!(obj.type_, SpaTypes::ObjectParamRoute.as_raw());
            assert_eq!(obj.id, ParamType::Route.as_raw());

            let mut roundtripped_index = None;
            let mut roundtripped_save = None;
            let mut roundtripped_dev = None;

            for prop in &obj.properties {
                if prop.key == spa_sys::SPA_PARAM_ROUTE_index {
                    if let Value::Int(i) = prop.value {
                        roundtripped_index = Some(i);
                    }
                } else if prop.key == spa_sys::SPA_PARAM_ROUTE_save {
                    if let Value::Bool(b) = prop.value {
                        roundtripped_save = Some(b);
                    }
                } else if prop.key == spa_sys::SPA_PARAM_ROUTE_device {
                    if let Value::Int(d) = prop.value {
                        roundtripped_dev = Some(d);
                    }
                }
            }

            assert_eq!(roundtripped_index, Some(route_idx));
            assert_eq!(roundtripped_save, Some(true));
            assert_eq!(roundtripped_dev, dev_id);
        }
    }
}

// ============================================================================
// SECTION 3: Robustness Under Malformed, Corrupted, and Unexpected SPA Pods
// ============================================================================

#[test]
fn test_apply_device_param_pod_with_truncated_buffers_never_panics() {
    let mut device = PwDevice {
        id: 1,
        object_serial: 100,
        name: "test_dev".to_string(),
        description: "Test Device".to_string(),
        nick: None,
        media_class: None,
        props: BTreeMap::new(),
        profiles: Vec::new(),
        routes: Vec::new(),
        active_profile: None,
    };

    let valid_profile_bytes = serialize_device_profile_pod(1).unwrap();
    let valid_route_bytes = serialize_device_route_pod(0, Some(1)).unwrap();

    // Test all prefix lengths from 0 to len - 1
    for len in 0..valid_profile_bytes.len() {
        let prefix = &valid_profile_bytes[..len];
        if let Some(pod) = Pod::from_bytes(prefix) {
            let res = apply_device_param_pod(&mut device, ParamType::EnumProfile, pod);
            let _ = apply_device_param_pod(&mut device, ParamType::Profile, pod);
        }
    }

    for len in 0..valid_route_bytes.len() {
        let prefix = &valid_route_bytes[..len];
        if let Some(pod) = Pod::from_bytes(prefix) {
            let res = apply_device_param_pod(&mut device, ParamType::EnumRoute, pod);
            let _ = apply_device_param_pod(&mut device, ParamType::Route, pod);
        }
    }
}

#[test]
fn test_apply_device_param_pod_fuzzing_byte_corruptions_never_panics() {
    let mut rng = TestRng::new(0xDEADBEEFCAFE);
    let mut device = PwDevice {
        id: 42,
        object_serial: 4200,
        name: "fuzz_dev".to_string(),
        description: "Fuzz Device".to_string(),
        nick: None,
        media_class: None,
        props: BTreeMap::new(),
        profiles: Vec::new(),
        routes: Vec::new(),
        active_profile: None,
    };

    let base_profile = serialize_device_profile_pod(1).unwrap();
    let base_route = serialize_device_route_pod(0, Some(42)).unwrap();

    let param_types = [
        ParamType::EnumProfile,
        ParamType::Profile,
        ParamType::EnumRoute,
        ParamType::Route,
    ];

    let mut panic_count = 0;
    let mut total_tested = 0;

    // Fuzz profile pod with 1000 randomized corruptions
    for _ in 0..1000 {
        let mut corrupted = base_profile.clone();
        let num_mutations = rng.next_range(1, 5);
        for _ in 0..num_mutations {
            let offset = rng.next_range(0, corrupted.len() - 1);
            let val = rng.next_u8();
            corrupted[offset] = val;
        }

        if let Some(pod) = Pod::from_bytes(&corrupted) {
            for &pt in &param_types {
                total_tested += 1;
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    apply_device_param_pod(&mut device, pt, pod)
                }));
                if res.is_err() {
                    panic_count += 1;
                    if panic_count == 1 {
                        eprintln!("First panic pod bytes: {corrupted:?}");
                    }
                }
            }
        }
    }

    // Fuzz route pod with 1000 randomized corruptions
    for _ in 0..1000 {
        let mut corrupted = base_route.clone();
        let num_mutations = rng.next_range(1, 5);
        for _ in 0..num_mutations {
            let offset = rng.next_range(0, corrupted.len() - 1);
            let val = rng.next_u8();
            corrupted[offset] = val;
        }

        if let Some(pod) = Pod::from_bytes(&corrupted) {
            for &pt in &param_types {
                total_tested += 1;
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    apply_device_param_pod(&mut device, pt, pod)
                }));
                if res.is_err() {
                    panic_count += 1;
                }
            }
        }
    }

    println!("Fuzzing complete: tested {total_tested} valid Pod representations, encountered {panic_count} panics in apply_device_param_pod");
    assert_eq!(
        panic_count, 0,
        "apply_device_param_pod panicked {panic_count} times out of {total_tested} on corrupted SPA pods due to unhandled libspa PodDeserializer panics"
    );
}

#[test]
fn test_apply_device_param_pod_panics_on_nested_malformed_object() {
    let mut device = PwDevice {
        id: 1,
        object_serial: 10,
        name: "dev".to_string(),
        description: "dev".to_string(),
        nick: None,
        media_class: None,
        props: BTreeMap::new(),
        profiles: Vec::new(),
        routes: Vec::new(),
        active_profile: None,
    };

    // Minimal 64-byte Object pod containing a property whose value is an Object pod with size 4 (< 8).
    let raw: [u8; 64] = [
        56, 0, 0, 0, 15, 0, 0, 0, // Outer Pod: size=56, type=SPA_TYPE_Object (15)
        1, 0, 0, 0, 1, 0, 0, 0,   // Outer Object: type=1, id=1
        1, 0, 0, 0, 0, 0, 0, 0,   // Prop 1: key=1, flags=0
        4, 0, 0, 0, 4, 0, 0, 0,   // Prop 1 val pod: size=4, type=SPA_TYPE_Int (4)
        0, 0, 0, 0, 0, 0, 0, 0,   // Prop 1 val: 0, padding
        2, 0, 0, 0, 0, 0, 0, 0,   // Prop 2: key=2, flags=0
        4, 0, 0, 0, 15, 0, 0, 0,  // Prop 2 val pod: size=4, type=SPA_TYPE_Object (15) -- MALFORMED!
        0, 0, 0, 0, 0, 0, 0, 0,   // Trailing 8 bytes in outer buffer
    ];

    let pod = Pod::from_bytes(&raw).expect("Valid outer Pod structure");

    // Uncaught panic occurs inside libspa PodDeserializer: attempt to subtract with overflow (len - 8 => 4 - 8)
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        apply_device_param_pod(&mut device, ParamType::EnumProfile, pod)
    }));

    assert!(
        res.is_ok(),
        "CRITICAL VULNERABILITY: apply_device_param_pod must not panic when PodDeserializer encounters a malformed property object"
    );
}

#[test]
fn test_apply_device_param_pod_with_non_object_pods_gracefully_fails() {
    let mut device = PwDevice {
        id: 1,
        object_serial: 10,
        name: "dev".to_string(),
        description: "dev".to_string(),
        nick: None,
        media_class: None,
        props: BTreeMap::new(),
        profiles: Vec::new(),
        routes: Vec::new(),
        active_profile: None,
    };

    let non_objects = [
        Value::Int(42),
        Value::Long(999999999),
        Value::Float(std::f32::consts::PI),
        Value::Double(std::f64::consts::E),
        Value::Bool(true),
        Value::String("hello world".to_string()),
        Value::None,
    ];

    for val in non_objects {
        let (cursor, _) =
            pipewire::spa::pod::serialize::PodSerializer::serialize(Cursor::new(Vec::new()), &val)
                .unwrap();
        let buf = cursor.into_inner();
        let pod = Pod::from_bytes(&buf).unwrap();

        assert!(!apply_device_param_pod(&mut device, ParamType::EnumProfile, pod));
        assert!(!apply_device_param_pod(&mut device, ParamType::Profile, pod));
        assert!(!apply_device_param_pod(&mut device, ParamType::EnumRoute, pod));
        assert!(!apply_device_param_pod(&mut device, ParamType::Route, pod));
    }
}

#[test]
fn test_apply_device_param_pod_with_unexpected_param_types_gracefully_fails() {
    let mut device = PwDevice {
        id: 1,
        object_serial: 10,
        name: "dev".to_string(),
        description: "dev".to_string(),
        nick: None,
        media_class: None,
        props: BTreeMap::new(),
        profiles: Vec::new(),
        routes: Vec::new(),
        active_profile: None,
    };

    let profile_bytes = serialize_device_profile_pod(1).unwrap();
    let pod = Pod::from_bytes(&profile_bytes).unwrap();

    // Calling with non-device param types: Props, Format, Latency
    assert!(!apply_device_param_pod(&mut device, ParamType::Props, pod));
    assert!(!apply_device_param_pod(&mut device, ParamType::Format, pod));
    assert!(!apply_device_param_pod(&mut device, ParamType::Latency, pod));
    assert!(!apply_device_param_pod(&mut device, ParamType::ProcessLatency, pod));
}

#[test]
fn test_apply_device_param_pod_with_missing_mandatory_properties() {
    let mut device = PwDevice {
        id: 1,
        object_serial: 10,
        name: "dev".to_string(),
        description: "dev".to_string(),
        nick: None,
        media_class: None,
        props: BTreeMap::new(),
        profiles: Vec::new(),
        routes: Vec::new(),
        active_profile: None,
    };

    // 1. EnumProfile without index (only has name and description)
    let p_no_index = Value::Object(Object {
        type_: SpaTypes::ObjectParamProfile.as_raw(),
        id: ParamType::EnumProfile.as_raw(),
        properties: vec![
            Property::new(spa_sys::SPA_PARAM_PROFILE_name, Value::String("no-index".to_string())),
            Property::new(spa_sys::SPA_PARAM_PROFILE_description, Value::String("No Index Desc".to_string())),
        ],
    });
    let (c, _) = pipewire::spa::pod::serialize::PodSerializer::serialize(Cursor::new(Vec::new()), &p_no_index).unwrap();
    let buf1 = c.into_inner();
    let pod1 = Pod::from_bytes(&buf1).unwrap();
    assert!(!apply_device_param_pod(&mut device, ParamType::EnumProfile, pod1), "Must return false when index is missing");
    assert!(device.profiles.is_empty(), "Profiles must not be modified");

    // 2. Active Profile without index
    let p_act_no_index = Value::Object(Object {
        type_: SpaTypes::ObjectParamProfile.as_raw(),
        id: ParamType::Profile.as_raw(),
        properties: vec![
            Property::new(spa_sys::SPA_PARAM_PROFILE_name, Value::String("pro-audio".to_string())),
        ],
    });
    let (c2, _) = pipewire::spa::pod::serialize::PodSerializer::serialize(Cursor::new(Vec::new()), &p_act_no_index).unwrap();
    let buf2 = c2.into_inner();
    let pod2 = Pod::from_bytes(&buf2).unwrap();
    assert!(!apply_device_param_pod(&mut device, ParamType::Profile, pod2));
    assert_eq!(device.active_profile, None);

    // 3. EnumRoute without index
    let r_no_index = Value::Object(Object {
        type_: SpaTypes::ObjectParamRoute.as_raw(),
        id: ParamType::EnumRoute.as_raw(),
        properties: vec![
            Property::new(spa_sys::SPA_PARAM_ROUTE_name, Value::String("headphones".to_string())),
            Property::new(spa_sys::SPA_PARAM_ROUTE_direction, Value::Int(1)),
        ],
    });
    let (c3, _) = pipewire::spa::pod::serialize::PodSerializer::serialize(Cursor::new(Vec::new()), &r_no_index).unwrap();
    let buf3 = c3.into_inner();
    let pod3 = Pod::from_bytes(&buf3).unwrap();
    assert!(!apply_device_param_pod(&mut device, ParamType::EnumRoute, pod3));
    assert!(device.routes.is_empty());

    // 4. Active Route without index
    let r_act_no_index = Value::Object(Object {
        type_: SpaTypes::ObjectParamRoute.as_raw(),
        id: ParamType::Route.as_raw(),
        properties: vec![
            Property::new(spa_sys::SPA_PARAM_ROUTE_name, Value::String("headphones".to_string())),
        ],
    });
    let (c4, _) = pipewire::spa::pod::serialize::PodSerializer::serialize(Cursor::new(Vec::new()), &r_act_no_index).unwrap();
    let buf4 = c4.into_inner();
    let pod4 = Pod::from_bytes(&buf4).unwrap();
    assert!(!apply_device_param_pod(&mut device, ParamType::Route, pod4));
}

#[test]
fn test_apply_device_param_pod_with_mismatched_property_types() {
    let mut device = PwDevice {
        id: 1,
        object_serial: 10,
        name: "dev".to_string(),
        description: "dev".to_string(),
        nick: None,
        media_class: None,
        props: BTreeMap::new(),
        profiles: Vec::new(),
        routes: Vec::new(),
        active_profile: None,
    };

    // Index is a String instead of Int!
    let malformed = Value::Object(Object {
        type_: SpaTypes::ObjectParamProfile.as_raw(),
        id: ParamType::EnumProfile.as_raw(),
        properties: vec![
            Property::new(spa_sys::SPA_PARAM_PROFILE_index, Value::String("not an int".to_string())),
            Property::new(spa_sys::SPA_PARAM_PROFILE_name, Value::Int(12345)), // Name is an Int instead of String!
            Property::new(spa_sys::SPA_PARAM_PROFILE_available, Value::Float(1.5)), // Available is Float!
        ],
    });
    let (c, _) = pipewire::spa::pod::serialize::PodSerializer::serialize(Cursor::new(Vec::new()), &malformed).unwrap();
    let buf_mal = c.into_inner();
    let pod = Pod::from_bytes(&buf_mal).unwrap();

    // Must safely reject without panic
    assert!(!apply_device_param_pod(&mut device, ParamType::EnumProfile, pod));
}

#[test]
fn test_apply_device_param_pod_with_adversarial_strings_and_extreme_priorities() {
    let mut device = PwDevice {
        id: 1,
        object_serial: 10,
        name: "dev".to_string(),
        description: "dev".to_string(),
        nick: None,
        media_class: None,
        props: BTreeMap::new(),
        profiles: Vec::new(),
        routes: Vec::new(),
        active_profile: None,
    };

    // Strings with special symbols, Unicode/emojis, very large strings, extreme priority
    let huge_desc = "x".repeat(10_000);
    let adversarial = Value::Object(Object {
        type_: SpaTypes::ObjectParamProfile.as_raw(),
        id: ParamType::EnumProfile.as_raw(),
        properties: vec![
            Property::new(spa_sys::SPA_PARAM_PROFILE_index, Value::Int(10)),
            Property::new(
                spa_sys::SPA_PARAM_PROFILE_name,
                Value::String("analog-stereo-🎧-!@#$%^&*()_+".to_string()),
            ),
            Property::new(spa_sys::SPA_PARAM_PROFILE_description, Value::String(huge_desc)),
            Property::new(spa_sys::SPA_PARAM_PROFILE_priority, Value::Int(-9999)), // Negative priority
            Property::new(spa_sys::SPA_PARAM_PROFILE_available, Value::Int(99999)), // Unknown availability ID
        ],
    });

    let (c, _) = pipewire::spa::pod::serialize::PodSerializer::serialize(Cursor::new(Vec::new()), &adversarial).unwrap();
    let buf_adv = c.into_inner();
    let pod = Pod::from_bytes(&buf_adv).unwrap();

    assert!(apply_device_param_pod(&mut device, ParamType::EnumProfile, pod));
    assert_eq!(device.profiles.len(), 1);
    let prof = &device.profiles[0];
    assert_eq!(prof.index, 10);
    assert_eq!(prof.name, "analog-stereo-🎧-!@#$%^&*()_+");
    assert_eq!(prof.priority, 0, "Negative priority clamped to 0");
    assert_eq!(prof.available, PwAvailability::Unknown, "Unknown ID maps to PwAvailability::Unknown");
}

// ============================================================================
// SECTION 4: Graph Model Invariants Under Churn, Removal, and Reassignment
// ============================================================================

#[test]
fn test_graph_invariants_device_and_node_lookup_consistency() {
    let graph = create_demo_graph();

    // Verify consistency for all nodes
    for (node_serial, node) in &graph.nodes {
        let dev_opt = graph.device_for_node(*node_serial);
        if let Some(dev) = dev_opt {
            assert!(node.device_id.is_some() || node.props.contains_key("device.id"));
            assert_eq!(dev.object_serial, node.device_serial.unwrap());
        } else {
            // If device_for_node is None, device_serial should either be None or point to missing device
            if let Some(ds) = node.device_serial {
                assert!(!graph.devices.contains_key(&ds));
            }
        }
    }
}

#[test]
fn test_graph_invariants_device_removal_cleans_up_lookups() {
    let mut graph = create_demo_graph();
    let mut backend = Backend::demo();

    // Node 10 is backed by Device 1000 (Scarlett Solo)
    assert!(graph.device_for_node(10).is_some());
    assert_eq!(graph.device_for_node(10).unwrap().object_serial, 1000);

    // Remove device 1000
    let removed = graph.devices.remove(&1000);
    assert!(removed.is_some());

    // Invariant: device_for_node MUST return None now!
    assert!(
        graph.device_for_node(10).is_none(),
        "device_for_node must return None when device is removed"
    );
    assert!(
        graph.device_for_node_mut(10).is_none(),
        "device_for_node_mut must return None when device is removed"
    );

    // Calling resolve_serials_and_rebuild_ports must not panic
    graph.resolve_serials_and_rebuild_ports();
    assert!(graph.device_for_node(10).is_none());

    // Calling backend operations for that node must return clean Err, not panic
    let res_prof = backend.set_node_device_profile(&mut graph, 10, 0);
    assert!(res_prof.is_err());
    assert!(res_prof.unwrap_err().contains("not backed by a device"));

    let res_route = backend.set_node_device_route(&mut graph, 10, 0, None);
    assert!(res_route.is_err());
}

#[test]
fn test_graph_invariants_device_hotplug_serial_reassignment() {
    let mut graph = PwGraph::default();

    // 1. Initial Device: id 88, serial 8800
    graph.devices.insert(
        8800,
        PwDevice {
            id: 88,
            object_serial: 8800,
            name: "usb_dac".to_string(),
            description: "USB DAC".to_string(),
            nick: None,
            media_class: None,
            props: BTreeMap::new(),
            profiles: vec![PwDeviceProfile {
                index: 1,
                name: "stereo".to_string(),
                description: "Stereo".to_string(),
                priority: 10,
                available: PwAvailability::Yes,
                active: true,
            }],
            routes: Vec::new(),
            active_profile: Some(1),
        },
    );

    // Node referencing device.id = 88
    let mut props = BTreeMap::new();
    props.insert("device.id".to_string(), "88".to_string());
    graph.nodes.insert(
        55,
        PwNode {
            id: 888,
            object_serial: 55,
            name: "dac_sink".to_string(),
            description: "DAC Sink".to_string(),
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

    graph.resolve_serials_and_rebuild_ports();
    assert_eq!(graph.device_for_node(55).unwrap().object_serial, 8800);

    // 2. Unplug: remove serial 8800
    graph.devices.remove(&8800);
    assert!(graph.device_for_node(55).is_none());

    // 3. Re-plug: device comes back with SAME id 88, but NEW PipeWire serial 8801
    graph.devices.insert(
        8801,
        PwDevice {
            id: 88,
            object_serial: 8801,
            name: "usb_dac".to_string(),
            description: "USB DAC".to_string(),
            nick: None,
            media_class: None,
            props: BTreeMap::new(),
            profiles: vec![PwDeviceProfile {
                index: 1,
                name: "stereo".to_string(),
                description: "Stereo".to_string(),
                priority: 10,
                available: PwAvailability::Yes,
                active: true,
            }],
            routes: Vec::new(),
            active_profile: Some(1),
        },
    );

    graph.resolve_serials_and_rebuild_ports();

    // Invariant: node 55 must now resolve to the new serial 8801!
    let dev = graph.device_for_node(55).expect("Should find re-plugged device");
    assert_eq!(dev.object_serial, 8801);
}

#[test]
fn test_graph_invariants_directional_exclusivity_state_machine() {
    let mut graph = create_demo_graph();
    let mut backend = Backend::demo();

    let builtin_serial = 5000;

    // Invariant verification closure
    let verify_exclusivity = |g: &PwGraph| {
        let dev = g.devices.get(&builtin_serial).unwrap();
        let active_outputs = dev.routes.iter().filter(|r| r.direction == PortDirection::Output && r.active).count();
        let active_inputs = dev.routes.iter().filter(|r| r.direction == PortDirection::Input && r.active).count();
        assert!(active_outputs <= 1, "At most 1 active output route allowed");
        assert!(active_inputs <= 1, "At most 1 active input route allowed");
    };

    verify_exclusivity(&graph);

    // Initial state: route 0 (Output Headphones) active, route 3 (Input Mic) active
    let dev = graph.devices.get(&builtin_serial).unwrap();
    assert!(dev.routes[0].active && dev.routes[0].direction == PortDirection::Output);
    assert!(dev.routes[3].active && dev.routes[3].direction == PortDirection::Input);

    // 1. Try to switch Output to unavailable route 2 (Line Out) -> MUST FAIL, state untouched
    let res = backend.set_device_route(&mut graph, builtin_serial, 2, Some(500));
    assert!(res.is_err());
    verify_exclusivity(&graph);
    let dev = graph.devices.get(&builtin_serial).unwrap();
    assert!(dev.routes[0].active, "Output route must remain 0");
    assert!(dev.routes[3].active, "Input route must remain 3");

    // 2. Switch Output to available route 1 (Speakers) -> MUST SUCCEED, Input untouched!
    let res2 = backend.set_device_route(&mut graph, builtin_serial, 1, Some(500));
    assert!(res2.is_ok());
    verify_exclusivity(&graph);
    let dev = graph.devices.get(&builtin_serial).unwrap();
    assert!(!dev.routes[0].active);
    assert!(dev.routes[1].active);
    assert!(dev.routes[3].active, "Input route 3 must remain active!");

    // 3. Try to switch Input to unavailable route 4 (Headset Mic) -> MUST FAIL, state untouched
    let res3 = backend.set_device_route(&mut graph, builtin_serial, 4, Some(500));
    assert!(res3.is_err());
    verify_exclusivity(&graph);
    let dev = graph.devices.get(&builtin_serial).unwrap();
    assert!(dev.routes[1].active, "Output route 1 must still be active");
    assert!(dev.routes[3].active, "Input route 3 must still be active");

    // 4. Switch Output back to Headphones (route 0)
    let res4 = backend.set_device_route(&mut graph, builtin_serial, 0, Some(500));
    assert!(res4.is_ok());
    verify_exclusivity(&graph);
    let dev = graph.devices.get(&builtin_serial).unwrap();
    assert!(dev.routes[0].active);
    assert!(!dev.routes[1].active);
    assert!(dev.routes[3].active);
}

#[test]
fn test_graph_invariants_profile_exclusivity_state_machine() {
    let mut graph = create_demo_graph();
    let mut backend = Backend::demo();

    let scarlett_serial = 1000;

    let verify_profile_exclusivity = |g: &PwGraph| {
        let dev = g.devices.get(&scarlett_serial).unwrap();
        let active_profiles = dev.profiles.iter().filter(|p| p.active).count();
        assert_eq!(active_profiles, 1, "Exactly 1 profile must be active");
        let active_p = dev.profiles.iter().find(|p| p.active).unwrap();
        assert_eq!(dev.active_profile, Some(active_p.index));
    };

    verify_profile_exclusivity(&graph);

    // Initial active: 1
    // Try to switch to unavailable profile 3 -> MUST FAIL, active remains 1
    assert!(backend.set_device_profile(&mut graph, scarlett_serial, 3).is_err());
    verify_profile_exclusivity(&graph);
    assert_eq!(graph.devices.get(&scarlett_serial).unwrap().active_profile, Some(1));

    // Switch to profile 2 -> SUCCEEDS
    assert!(backend.set_device_profile(&mut graph, scarlett_serial, 2).is_ok());
    verify_profile_exclusivity(&graph);
    assert_eq!(graph.devices.get(&scarlett_serial).unwrap().active_profile, Some(2));

    // Switch to profile 0 -> SUCCEEDS
    assert!(backend.set_device_profile(&mut graph, scarlett_serial, 0).is_ok());
    verify_profile_exclusivity(&graph);
    assert_eq!(graph.devices.get(&scarlett_serial).unwrap().active_profile, Some(0));

    // Switch back to profile 1 -> SUCCEEDS
    assert!(backend.set_device_profile(&mut graph, scarlett_serial, 1).is_ok());
    verify_profile_exclusivity(&graph);
    assert_eq!(graph.devices.get(&scarlett_serial).unwrap().active_profile, Some(1));
}

#[test]
fn test_graph_invariants_massive_graph_churn_stress_harness() {
    let mut rng = TestRng::new(0x1337BEEF);
    let mut graph = PwGraph::default();
    let mut backend = Backend::demo();

    let mut device_ids = Vec::new();
    let mut node_ids = Vec::new();

    for iter in 0..600 {
        let op = rng.next_range(0, 5);
        match op {
            0 => {
                // Add Device
                let d_id = (iter + 1) as u32;
                let d_serial = (iter + 1) as u64 * 1000;
                let dev = PwDevice {
                    id: d_id,
                    object_serial: d_serial,
                    name: format!("device_{d_id}"),
                    description: format!("Device {d_id}"),
                    nick: None,
                    media_class: None,
                    props: BTreeMap::new(),
                    profiles: vec![
                        PwDeviceProfile {
                            index: 0,
                            name: "off".to_string(),
                            description: "Off".to_string(),
                            priority: 0,
                            available: PwAvailability::Yes,
                            active: true,
                        },
                        PwDeviceProfile {
                            index: 1,
                            name: "on".to_string(),
                            description: "On".to_string(),
                            priority: 10,
                            available: if rng.next_bool() {
                                PwAvailability::Yes
                            } else {
                                PwAvailability::No
                            },
                            active: false,
                        },
                    ],
                    routes: vec![
                        PwDeviceRoute {
                            index: 0,
                            name: "out".to_string(),
                            description: "Out".to_string(),
                            direction: PortDirection::Output,
                            available: PwAvailability::Yes,
                            active: true,
                            device_id: Some(d_id as i32),
                            profiles: vec![1],
                            props: BTreeMap::new(),
                        },
                        PwDeviceRoute {
                            index: 1,
                            name: "in".to_string(),
                            direction: PortDirection::Input,
                            description: "In".to_string(),
                            available: if rng.next_bool() {
                                PwAvailability::Yes
                            } else {
                                PwAvailability::No
                            },
                            active: true,
                            device_id: Some(d_id as i32),
                            profiles: vec![1],
                            props: BTreeMap::new(),
                        },
                    ],
                    active_profile: Some(0),
                };
                graph.devices.insert(d_serial, dev);
                device_ids.push((d_id, d_serial));
            }
            1 => {
                // Add Node referencing random existing device or no device
                let n_id = (iter + 1) as u32;
                let n_serial = (iter + 1) as u64;
                let mut props = BTreeMap::new();
                let dev_id_opt = if !device_ids.is_empty() && rng.next_bool() {
                    let idx = rng.next_range(0, device_ids.len() - 1);
                    let target_id = device_ids[idx].0;
                    props.insert("device.id".to_string(), target_id.to_string());
                    Some(target_id)
                } else {
                    None
                };

                let node = PwNode {
                    id: n_id,
                    object_serial: n_serial,
                    name: format!("node_{n_id}"),
                    description: format!("Node {n_id}"),
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
                    device_id: dev_id_opt,
                    device_serial: None,
                };
                graph.nodes.insert(n_serial, node);
                node_ids.push(n_serial);
            }
            2 => {
                // Remove a random device
                if !device_ids.is_empty() {
                    let idx = rng.next_range(0, device_ids.len() - 1);
                    let (_did, dserial) = device_ids.swap_remove(idx);
                    graph.devices.remove(&dserial);
                }
            }
            3 => {
                // Remove a random node
                if !node_ids.is_empty() {
                    let idx = rng.next_range(0, node_ids.len() - 1);
                    let nserial = node_ids.swap_remove(idx);
                    graph.nodes.remove(&nserial);
                }
            }
            4 => {
                // Route/Profile operation on random existing device
                if !device_ids.is_empty() {
                    let idx = rng.next_range(0, device_ids.len() - 1);
                    let (_did, dserial) = device_ids[idx];
                    let _ = backend.set_device_profile(&mut graph, dserial, 1);
                    let _ = backend.set_device_route(&mut graph, dserial, 1, None);
                }
            }
            _ => {
                // Reconcile and rebuild
                graph.resolve_serials_and_rebuild_ports();
                let _ = graph.collapsed_edges();
            }
        }

        // Periodically verify invariants
        if iter % 50 == 0 {
            graph.resolve_serials_and_rebuild_ports();
            for (&nserial, node) in &graph.nodes {
                let dev_lookup = graph.device_for_node(nserial);
                if let Some(dev) = dev_lookup {
                    assert!(graph.devices.contains_key(&dev.object_serial));
                }
            }
        }
    }
}
