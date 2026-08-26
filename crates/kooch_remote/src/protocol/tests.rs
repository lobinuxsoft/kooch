use super::*;

#[test]
fn entity_id_round_trips_through_entity() {
    let e = Entity::new(7, 3);
    let id: EntityId = e.into();
    assert_eq!(id.index, 7);
    assert_eq!(id.generation, 3);
    assert_eq!(Entity::from(id), e);
}

#[test]
fn request_deserializes_from_flat_json() {
    let json = r#"{"id":5,"method":"set_field","entity":{"index":1,"generation":0},"component":"game::Health","field":"hp","value":{"U32":42}}"#;
    let req: Request = serde_json::from_str(json).unwrap();
    assert_eq!(req.id, 5);
    match req.method {
        Method::SetField {
            entity,
            component,
            field,
            value,
        } => {
            assert_eq!(entity.index, 1);
            assert_eq!(component, "game::Health");
            assert_eq!(field, "hp");
            assert_eq!(value, ReflectValue::U32(42));
        }
        other => panic!("wrong method: {other:?}"),
    }
}

#[test]
fn ping_request_needs_no_params() {
    let req: Request = serde_json::from_str(r#"{"id":1,"method":"ping"}"#).unwrap();
    assert_eq!(req.method, Method::Ping);
}

#[test]
fn response_round_trips() {
    let resp = Response::ok(
        9,
        ResponseData::Spawned {
            entity: EntityId {
                index: 2,
                generation: 1,
            },
        },
    );
    let json = serde_json::to_string(&resp).unwrap();
    let back: Response = serde_json::from_str(&json).unwrap();
    assert_eq!(resp, back);
}

#[test]
fn error_response_round_trips() {
    let resp = Response::err(
        3,
        RemoteError::UnknownComponent {
            type_name: "game::Missing".into(),
        },
    );
    let json = serde_json::to_string(&resp).unwrap();
    let back: Response = serde_json::from_str(&json).unwrap();
    assert_eq!(resp, back);
}

/// A host built before this field existed sends a reply without it.
/// That has to keep parsing, and has to arrive as "nobody said"
/// rather than as a project running infinitely fast.
#[test]
fn a_reply_without_host_metrics_still_parses() {
    let json = r#"{"id":1,"result":{"kind":"entities","entities":[],"revision":7,"full":true}}"#;
    let parsed: Response = serde_json::from_str(json).expect("older host still understood");
    match parsed.payload {
        ResponsePayload::Result(ResponseData::Entities { host, revision, .. }) => {
            assert_eq!(host, None, "absent, not zeroed");
            assert_eq!(revision, 7);
        }
        other => panic!("expected entities, got {other:?}"),
    }
}

/// And a reply that carries them survives the round trip.
#[test]
fn host_metrics_round_trip() {
    let resp = Response::ok(
        9,
        ResponseData::Entities {
            entities: Vec::new(),
            removed: Vec::new(),
            revision: 2,
            full: false,
            host: Some(HostMetrics {
                frame_ms: 16.67,
                cpu_frame_ms: 4.2,
                ticks_instant: 59.99,
                ticks_per_second: 60.0,
            }),
            scenes: None,
        },
    );
    let json = serde_json::to_string(&resp).unwrap();
    let back: Response = serde_json::from_str(&json).unwrap();
    assert_eq!(resp, back);
}

/// Nothing to say costs nothing to send: the field is skipped, so a
/// host with no measurement yet does not widen every snapshot.
#[test]
fn absent_host_metrics_are_not_serialized() {
    let resp = Response::ok(
        1,
        ResponseData::Entities {
            entities: Vec::new(),
            removed: Vec::new(),
            revision: 1,
            full: true,
            host: None,
            scenes: None,
        },
    );
    let json = serde_json::to_string(&resp).unwrap();
    assert!(!json.contains("host"), "{json}");
}

/// The open scene set survives the wire.
#[test]
fn open_scenes_round_trip() {
    let resp = Response::ok(
        4,
        ResponseData::Entities {
            entities: Vec::new(),
            removed: Vec::new(),
            revision: 3,
            full: true,
            host: None,
            scenes: Some(vec![SceneEntry {
                id: Guid::new_v4(),
                path: Some("assets/scenes/many_lights.scene".into()),
                active: true,
                dirty: false,
            }]),
        },
    );
    let json = serde_json::to_string(&resp).unwrap();
    let back: Response = serde_json::from_str(&json).unwrap();
    assert_eq!(resp, back);
}

/// A host too old to send the open set has to arrive as "nobody said".
///
/// 🔴 Not as an empty list. The editor replaces its scene list from
/// this field, so an empty `Vec` here would blank the World panel of
/// every scene while the entities belonging to them kept arriving.
#[test]
fn a_reply_without_scenes_parses() {
    let json = r#"{"id":1,"result":{"kind":"entities","entities":[],"revision":7,"full":true}}"#;
    let parsed: Response = serde_json::from_str(json).expect("older host still understood");
    match parsed.payload {
        ResponsePayload::Result(ResponseData::Entities { scenes, .. }) => {
            assert_eq!(scenes, None, "absent, not empty");
        }
        other => panic!("expected entities, got {other:?}"),
    }
}

/// Nothing to say costs nothing to send.
#[test]
fn absent_scenes_are_not_serialized() {
    let resp = Response::ok(
        1,
        ResponseData::Entities {
            entities: Vec::new(),
            removed: Vec::new(),
            revision: 1,
            full: true,
            host: None,
            scenes: None,
        },
    );
    let json = serde_json::to_string(&resp).unwrap();
    assert!(!json.contains("scenes"), "{json}");
}
