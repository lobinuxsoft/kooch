//! Hooks the ECS installs so plugins can reach it.
//!
//! `ome_core` does not depend on `ome_ecs`, so the operations a plugin
//! needs — spawning an entity, declaring a component type — arrive as
//! resources holding closures that `ome_ecs` fills in. Absent, the
//! corresponding call fails with a reason rather than silently doing
//! nothing.

use ome_plugin_api::component::{ComponentSchema, RegisterError};

use crate::resource::Resources;

/// Entity operations, installed by the ECS.
pub struct EntityBridge {
    spawn_fn: Box<dyn Fn(&mut Resources) -> u64 + Send + Sync>,
    despawn_fn: Box<dyn Fn(&mut Resources, u64) -> bool + Send + Sync>,
}

impl EntityBridge {
    /// Creates the bridge from the ECS's spawn and despawn logic.
    pub fn new(
        spawn: impl Fn(&mut Resources) -> u64 + Send + Sync + 'static,
        despawn: impl Fn(&mut Resources, u64) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            spawn_fn: Box::new(spawn),
            despawn_fn: Box::new(despawn),
        }
    }

    pub(crate) fn spawn(&self, resources: &mut Resources) -> u64 {
        (self.spawn_fn)(resources)
    }

    pub(crate) fn despawn(&self, resources: &mut Resources, entity: u64) -> bool {
        (self.despawn_fn)(resources, entity)
    }
}

/// Component-type registration, installed by the ECS.
///
/// A plugin's component types do not exist in this binary, so they are
/// registered by name and field list rather than by Rust type — the same
/// form `DynamicComponents` already stores them in.
pub struct ComponentBridge {
    register_fn:
        Box<dyn Fn(&mut Resources, &ComponentSchema) -> Result<(), RegisterError> + Send + Sync>,
}

impl ComponentBridge {
    /// Creates the bridge from the ECS's registration logic.
    pub fn new(
        register: impl Fn(&mut Resources, &ComponentSchema) -> Result<(), RegisterError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            register_fn: Box::new(register),
        }
    }

    pub(crate) fn register(
        &self,
        resources: &mut Resources,
        schema: &ComponentSchema,
    ) -> Result<(), RegisterError> {
        (self.register_fn)(resources, schema)
    }
}

#[cfg(test)]
mod tests {
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
}
