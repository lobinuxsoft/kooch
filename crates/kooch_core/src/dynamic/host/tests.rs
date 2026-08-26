use super::*;
use kooch_plugin_api::component::FieldKind;

fn host_resources() -> Resources {
    let mut resources = Resources::new();
    resources.insert(PluginData::new());
    resources
}

#[test]
fn every_stage_maps() {
    // Exhaustive by construction — this asserts the table is total
    // rather than that any particular pair is right.
    for stage in PluginStage::ALL {
        let _ = map_stage(*stage);
    }
    assert_eq!(map_stage(PluginStage::Update), Stage::Update);
    assert_eq!(map_stage(PluginStage::Last), Stage::Last);
}

#[test]
fn data_survives_a_round_trip() {
    let mut resources = host_resources();
    let mut host = EngineHost::running(&mut resources);

    host.set_data("progress", b"level-3");
    assert_eq!(host.get_data("progress"), Some(&b"level-3"[..]));
    assert_eq!(host.get_data("absent"), None);
}

#[test]
fn an_empty_component_name_is_refused_before_the_registry() {
    let mut resources = host_resources();
    let mut host = EngineHost::running(&mut resources);

    // No ComponentBridge installed: if the name check did not come
    // first this would report NoRegistry instead.
    assert_eq!(
        host.register_component(ComponentSchema::new("   ")),
        Err(RegisterError::EmptyName)
    );
}

#[test]
fn an_empty_field_name_names_its_index() {
    let mut resources = host_resources();
    let mut host = EngineHost::running(&mut resources);

    let schema = ComponentSchema::new("my_game::Health")
        .with_field("current", FieldKind::U32)
        .with_field("", FieldKind::F32);

    assert_eq!(
        host.register_component(schema),
        Err(RegisterError::EmptyFieldName { index: 1 })
    );
}

/// A host built without the ECS says so, rather than accepting a
/// registration that goes nowhere.
#[test]
fn without_a_registry_it_says_so() {
    let mut resources = host_resources();
    let mut host = EngineHost::running(&mut resources);

    assert_eq!(
        host.register_component(ComponentSchema::new("my_game::Health")),
        Err(RegisterError::NoRegistry)
    );
}

#[test]
fn a_registered_component_reaches_the_bridge() {
    use std::sync::{Arc, Mutex};

    let seen: Arc<Mutex<Vec<ComponentSchema>>> = Arc::default();
    let recorder = Arc::clone(&seen);
    let mut resources = host_resources();
    resources.insert(ComponentBridge::new(move |_, schema| {
        recorder.lock().unwrap().push(schema.clone());
        Ok(())
    }));

    let mut host = EngineHost::running(&mut resources);
    let schema = ComponentSchema::new("my_game::Health").with_field("current", FieldKind::U32);
    assert!(host.register_component(schema).is_ok());

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].fields[0].kind, FieldKind::U32);
}

/// The bridge is put back after use, so a second plugin can register.
#[test]
fn the_bridge_survives_a_registration() {
    let mut resources = host_resources();
    resources.insert(ComponentBridge::new(|_, _| Ok(())));

    let mut host = EngineHost::running(&mut resources);
    assert!(
        host.register_component(ComponentSchema::new("a::A"))
            .is_ok()
    );
    assert!(
        host.register_component(ComponentSchema::new("b::B"))
            .is_ok()
    );
}

/// Registering a system mid-frame would mutate the schedule that is
/// running it, so it is refused rather than deferred silently.
#[test]
fn a_running_host_refuses_to_add_systems() {
    let mut resources = host_resources();
    let mut host = EngineHost::running(&mut resources);

    // Must not panic; the refusal is logged.
    host.add_system(PluginStage::Update, Box::new(|_| {}));
}

#[test]
fn spawning_without_an_entity_bridge_is_none() {
    let mut resources = host_resources();
    let mut host = EngineHost::running(&mut resources);

    assert_eq!(host.spawn_entity(), None);
    assert!(!host.despawn_entity(1));
}

#[test]
fn spawning_goes_through_the_entity_bridge() {
    let mut resources = host_resources();
    resources.insert(EntityBridge::new(|_| 42, |_, e| e == 42));

    let mut host = EngineHost::running(&mut resources);
    assert_eq!(host.spawn_entity(), Some(42));
    assert!(host.despawn_entity(42));
    assert!(!host.despawn_entity(1));
}
