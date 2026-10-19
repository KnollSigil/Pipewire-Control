#![allow(dead_code, unused_imports, unused_variables)]

#[path = "../src/model.rs"]
mod model;

#[path = "../src/backend.rs"]
mod backend;

use backend::{apply_port_param_pod, Backend};
use model::{
    create_demo_graph, NodeCategory, PortDirection, PwAvailability, PwGraph, PwNode, PwNodeParam,
    PwParamValue, PwPort,
};
use pipewire::spa::{
    param::ParamType,
    pod::{
        deserialize::PodDeserializer, serialize::PodSerializer, ChoiceValue, Object, Pod, Property,
        PropertyFlags, Value, ValueArray,
    },
    sys as spa_sys,
    utils::{Choice, ChoiceEnum, ChoiceFlags, Id, SpaTypes},
};
use std::collections::BTreeMap;
use std::io::Cursor;

fn make_test_port(id: u32, serial: u64, node_serial: u64, name: &str, dir: PortDirection) -> PwPort {
    PwPort {
        id,
        object_serial: serial,
        node_id: node_serial as u32,
        node_serial,
        name: name.to_string(),
        channel: "FL".to_string(),
        direction: dir,
        is_monitor: false,
        props: BTreeMap::new(),
        params: Vec::new(),
    }
}

// Helper to serialize an SPA Object into a byte vector
fn serialize_object_pod(obj_type: u32, param_id: u32, properties: Vec<Property>) -> Vec<u8> {
    let obj = Value::Object(Object {
        type_: obj_type,
        id: param_id,
        properties,
    });
    let (cursor, _) = PodSerializer::serialize(Cursor::new(Vec::new()), &obj)
        .expect("Failed to serialize Pod");
    cursor.into_inner()
}

#[test]
fn test_item38_port_params_demo_graph_population() {
    let graph = create_demo_graph();

    // Verify all ports in the demo graph have advertised SPA params populated
    assert!(!graph.ports.is_empty(), "Demo graph must have ports");
    for port in graph.ports.values() {
        assert!(
            !port.params.is_empty(),
            "Port {} (s#{}) should have populated SPA params",
            port.name,
            port.object_serial
        );

        // Check that Format, EnumFormat, Buffers, and Latency params are present
        let types: Vec<&str> = port.params.iter().map(|p| p.param_type.as_str()).collect();
        assert!(types.contains(&"EnumFormat"), "Must include EnumFormat");
        assert!(types.contains(&"Format"), "Must include Format");
        assert!(types.contains(&"Buffers"), "Must include Buffers");
        assert!(types.contains(&"Latency"), "Must include Latency");
    }
}

#[test]
fn test_item38_apply_port_param_pod_format_and_enum_format() {
    let mut port = make_test_port(1, 100, 10, "playback_FL", PortDirection::Input);
    assert!(port.params.is_empty());

    // Construct an SPA Format Object Pod
    let format_bytes = serialize_object_pod(
        SpaTypes::ObjectParamFormat.as_raw(),
        ParamType::Format.as_raw(),
        vec![
            Property {
                key: spa_sys::SPA_FORMAT_mediaType,
                flags: PropertyFlags::READONLY,
                value: Value::Id(Id(spa_sys::SPA_MEDIA_TYPE_audio)),
            },
            Property {
                key: spa_sys::SPA_FORMAT_mediaSubtype,
                flags: PropertyFlags::READONLY,
                value: Value::Id(Id(spa_sys::SPA_MEDIA_SUBTYPE_raw)),
            },
            Property {
                key: spa_sys::SPA_FORMAT_AUDIO_format,
                flags: PropertyFlags::READONLY,
                value: Value::Id(Id(spa_sys::SPA_AUDIO_FORMAT_F32P)),
            },
            Property {
                key: spa_sys::SPA_FORMAT_AUDIO_rate,
                flags: PropertyFlags::READONLY,
                value: Value::Int(48000),
            },
        ],
    );

    let format_pod = Pod::from_bytes(&format_bytes).unwrap();
    assert!(apply_port_param_pod(&mut port, ParamType::Format, format_pod));
    assert_eq!(port.params.len(), 4);

    let rate_param = port.params.iter().find(|p| p.name == "rate").expect("rate param should exist");
    assert_eq!(rate_param.param_type, "Format");
    assert_eq!(rate_param.value, PwParamValue::Int(48000));

    // Now apply EnumFormat with choices
    let enum_format_bytes = serialize_object_pod(
        SpaTypes::ObjectParamFormat.as_raw(),
        ParamType::EnumFormat.as_raw(),
        vec![
            Property {
                key: spa_sys::SPA_FORMAT_AUDIO_rate,
                flags: PropertyFlags::READONLY,
                value: Value::Choice(ChoiceValue::Int(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Range {
                        default: 48000,
                        min: 44100,
                        max: 192000,
                    },
                ))),
            },
        ],
    );

    let enum_format_pod = Pod::from_bytes(&enum_format_bytes).unwrap();
    assert!(apply_port_param_pod(&mut port, ParamType::EnumFormat, enum_format_pod));
    assert_eq!(port.params.len(), 5);

    let enum_rate = port.params.iter().find(|p| p.param_type == "EnumFormat" && p.name == "rate").unwrap();
    match &enum_rate.value {
        PwParamValue::DisplayOnly(s) => {
            assert!(s.contains("48000"), "Display string should contain default 48000: {s}");
            assert!(s.contains("44100"), "Display string should contain min 44100: {s}");
            assert!(s.contains("192000"), "Display string should contain max 192000: {s}");
        }
        other => panic!("Expected DisplayOnly for ChoiceValue, got: {other:?}"),
    }
}

#[test]
fn test_item38_apply_port_param_pod_buffers_and_latency() {
    let mut port = make_test_port(2, 200, 10, "capture_FL", PortDirection::Output);

    // Construct Buffers param
    let buffers_bytes = serialize_object_pod(
        SpaTypes::ObjectParamBuffers.as_raw(),
        ParamType::Buffers.as_raw(),
        vec![
            Property {
                key: spa_sys::SPA_PARAM_BUFFERS_buffers,
                flags: PropertyFlags::READONLY,
                value: Value::Int(32),
            },
            Property {
                key: spa_sys::SPA_PARAM_BUFFERS_size,
                flags: PropertyFlags::READONLY,
                value: Value::Int(4096),
            },
        ],
    );

    let buffers_pod = Pod::from_bytes(&buffers_bytes).unwrap();
    assert!(apply_port_param_pod(&mut port, ParamType::Buffers, buffers_pod));
    assert_eq!(port.params.len(), 2);
    assert!(port.params.iter().all(|p| p.param_type == "Buffers"));

    // Construct Latency param
    let latency_bytes = serialize_object_pod(
        SpaTypes::ObjectParamLatency.as_raw(),
        ParamType::Latency.as_raw(),
        vec![
            Property {
                key: spa_sys::SPA_PARAM_LATENCY_minQuantum,
                flags: PropertyFlags::READONLY,
                value: Value::Float(128.0),
            },
            Property {
                key: spa_sys::SPA_PARAM_LATENCY_maxQuantum,
                flags: PropertyFlags::READONLY,
                value: Value::Float(1024.0),
            },
        ],
    );

    let latency_pod = Pod::from_bytes(&latency_bytes).unwrap();
    assert!(apply_port_param_pod(&mut port, ParamType::Latency, latency_pod));
    assert_eq!(port.params.len(), 4);

    let min_q = port.params.iter().find(|p| p.name == "minQuantum").unwrap();
    assert_eq!(min_q.param_type, "Latency");
    assert_eq!(min_q.value, PwParamValue::Float(128.0));
}

#[test]
fn test_item39_dynamic_params_and_props_no_hardcoding() {
    let mut port = make_test_port(3, 300, 10, "in_custom", PortDirection::Input);

    // Apply an arbitrary SPA object type with arbitrary keys
    let custom_bytes = serialize_object_pod(
        SpaTypes::ObjectParamPortConfig.as_raw(),
        ParamType::PortConfig.as_raw(),
        vec![
            Property {
                key: 999, // Unknown / vendor-specific key
                flags: PropertyFlags::empty(),
                value: Value::String("CustomValue123".to_string()),
            },
        ],
    );

    let custom_pod = Pod::from_bytes(&custom_bytes).unwrap();
    assert!(apply_port_param_pod(&mut port, ParamType::PortConfig, custom_pod));
    let custom_param = port.params.iter().find(|p| p.key_id == 999).expect("Custom param should be retained");
    assert_eq!(custom_param.param_type, "PortConfig");
    assert_eq!(custom_param.value, PwParamValue::String("CustomValue123".to_string()));

    // Verify properties dictionary allows any key/value without filtering
    port.props.insert("custom.prop.x".to_string(), "foo".to_string());
    port.props.insert("pipewire.custom.arbitrary".to_string(), "bar".to_string());
    assert_eq!(port.props.len(), 2);
    assert_eq!(port.props.get("custom.prop.x").unwrap(), "foo");
    assert_eq!(port.props.get("pipewire.custom.arbitrary").unwrap(), "bar");
}
