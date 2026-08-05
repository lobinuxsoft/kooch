//! The engine's side of the plugin API.
//!
//! [`EngineHost`] implements [`Engine`] over the real `Resources` and
//! `Schedule`. It is handed to a plugin as `&mut dyn Engine` and borrows
//! for exactly as long as the call — no raw pointers, no context struct
//! outliving what it points at.

use kooch_plugin_api::component::{ComponentSchema, RegisterError};
use kooch_plugin_api::engine_api::{Engine, PluginSystem};
use kooch_plugin_api::types::Stage as PluginStage;

use crate::resource::Resources;
use crate::schedule::Schedule;
use crate::stage::Stage;

use super::bridges::{ComponentBridge, EntityBridge};
use super::plugin_data::PluginData;

/// Translates a plugin's stage into the engine's.
///
/// Exhaustive on purpose: adding a stage to either side without the
/// other fails the build here rather than running a plugin's system at
/// the wrong point in the frame.
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
///
/// The schedule is only present while the plugin is being built —
/// registering a system mid-frame would mutate the schedule that is
/// running, so a system that tries is refused with a log line instead.
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
mod tests {
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
}
