//! Hooks the ECS installs so plugins can reach it.
//!
//! `kooch_core` does not depend on `kooch_ecs`, so the operations a plugin
//! needs — spawning an entity, declaring a component type — arrive as
//! resources holding closures that `kooch_ecs` fills in. Absent, the
//! corresponding call fails with a reason rather than silently doing
//! nothing.

use kooch_plugin_api::component::{ComponentSchema, RegisterError};

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
mod tests;
