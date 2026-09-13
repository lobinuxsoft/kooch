//! The engine's side of the plugin API.

use kooch_plugin_api::component::{ComponentSchema, RegisterError};
use kooch_plugin_api::engine_api::{Engine, PluginSystem};
use kooch_plugin_api::types::Stage as PluginStage;

use crate::resource::Resources;
use crate::schedule::Schedule;
use crate::stage::Stage;

use super::bridges::{ComponentBridge, EntityBridge};
use super::plugin_data::PluginData;

/// Translates a plugin's stage into the engine's.
pub(crate) const fn map_stage(stage: PluginStage) -> Stage {
    match stage {
        PluginStage::Startup => Stage::Startup,
        PluginStage::First => Stage::First,
        PluginStage::Input => Stage::Input,
        PluginStage::PreUpdate => Stage::PreUpdate,
        PluginStage::Update => Stage::Update,
        PluginStage::PostUpdate => Stage::PostUpdate,
        PluginStage::GpuSync => Stage::GpuSync,
        PluginStage::Gpu => Stage::Gpu,
        PluginStage::Physics => Stage::Physics,
        PluginStage::PostPhysics => Stage::PostPhysics,
        PluginStage::PreRender => Stage::PreRender,
        PluginStage::Render => Stage::Render,
        PluginStage::PostRender => Stage::PostRender,
        PluginStage::Last => Stage::Last,
    }
}

/// What a plugin talks to.
pub struct EngineHost<'a> {
    resources: &'a mut Resources,
    schedule: Option<&'a mut Schedule>,
}

impl<'a> EngineHost<'a> {
    /// A host for `KoochPlugin::build`, where registering systems is
    /// allowed.
    pub fn building(resources: &'a mut Resources, schedule: &'a mut Schedule) -> Self {
        Self {
            resources,
            schedule: Some(schedule),
        }
    }

    /// A host for a running system, where the schedule is off limits.
    pub fn running(resources: &'a mut Resources) -> Self {
        Self {
            resources,
            schedule: None,
        }
    }
}

impl Engine for EngineHost<'_> {
    fn spawn_entity(&mut self) -> Option<u64> {
        // Remove-use-reinsert: the bridge's closures take `&mut
        // Resources`, which it is itself stored in.
        let bridge = self.resources.remove::<EntityBridge>()?;
        let entity = bridge.spawn(self.resources);
        self.resources.insert(bridge);
        Some(entity)
    }

    fn despawn_entity(&mut self, entity: u64) -> bool {
        let Some(bridge) = self.resources.remove::<EntityBridge>() else {
            return false;
        };
        let removed = bridge.despawn(self.resources, entity);
        self.resources.insert(bridge);
        removed
    }

    fn register_component(&mut self, schema: ComponentSchema) -> Result<(), RegisterError> {
        if schema.type_name.trim().is_empty() {
            return Err(RegisterError::EmptyName);
        }
        if let Some(index) = schema.fields.iter().position(|f| f.name.trim().is_empty()) {
            return Err(RegisterError::EmptyFieldName { index });
        }

        let Some(bridge) = self.resources.remove::<ComponentBridge>() else {
            return Err(RegisterError::NoRegistry);
        };
        let result = bridge.register(self.resources, &schema);
        self.resources.insert(bridge);

        match &result {
            Ok(()) => tracing::info!(
                component = schema.type_name,
                fields = schema.fields.len(),
                "plugin component registered"
            ),
            Err(e) => tracing::warn!(component = schema.type_name, "{e}"),
        }
        result
    }

    fn add_system(&mut self, stage: PluginStage, mut system: PluginSystem) {
        let Some(schedule) = self.schedule.as_mut() else {
            tracing::error!(
                "a plugin tried to add a system outside build(); the schedule is running"
            );
            return;
        };
        schedule.add_system(map_stage(stage), move |resources: &mut Resources| {
            let mut host = EngineHost::running(resources);
            system(&mut host);
        });
    }

    fn log(&self, message: &str) {
        tracing::info!(target: "plugin", "{message}");
    }

    fn set_data(&mut self, key: &str, data: &[u8]) {
        if let Some(store) = self.resources.get_mut::<PluginData>() {
            store.set(key, data);
        } else {
            tracing::warn!(key, "set_data called but the host has no PluginData");
        }
    }

    fn get_data(&self, key: &str) -> Option<&[u8]> {
        self.resources.get::<PluginData>()?.get(key)
    }
}

#[cfg(test)]
mod tests;
