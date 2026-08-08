use super::*;
use std::sync::{Arc, Mutex};

#[test]
fn an_entity_bridge_forwards_both_operations() {
    let bridge = EntityBridge::new(|_| 7, |_, e| e == 7);
    let mut resources = Resources::new();

    assert_eq!(bridge.spawn(&mut resources), 7);
    assert!(bridge.despawn(&mut resources, 7));
    assert!(!bridge.despawn(&mut resources, 8));
}

#[test]
fn a_component_bridge_forwards_the_schema() {
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let recorder = Arc::clone(&seen);
    let bridge = ComponentBridge::new(move |_, schema| {
        recorder.lock().unwrap().push(schema.type_name.clone());
        Ok(())
    });
    let mut resources = Resources::new();

    let schema = ComponentSchema::new("my_game::Health");
    assert!(bridge.register(&mut resources, &schema).is_ok());
    assert_eq!(seen.lock().unwrap().as_slice(), ["my_game::Health"]);
}

/// The ECS decides what a name collision means, and the error has to
/// come back rather than being swallowed by the bridge.
#[test]
fn a_rejection_reaches_the_caller() {
    let bridge = ComponentBridge::new(|_, schema| {
        Err(RegisterError::NameTaken {
            type_name: schema.type_name.clone(),
        })
    });
    let mut resources = Resources::new();

    let err = bridge
        .register(&mut resources, &ComponentSchema::new("my_game::Health"))
        .unwrap_err();
    assert_eq!(
        err,
        RegisterError::NameTaken {
            type_name: "my_game::Health".into()
        }
    );
}
