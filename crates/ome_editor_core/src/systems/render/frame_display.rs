//! [`FrameDisplayData`] — ECS data gathered once per frame, before the
//! egui context runs. Decouples the queries from the UI pass so the
//! borrow scopes don't collide.

use ome_core::resource::Resources;
use ome_ecs::archetype_registry::ArchetypeRegistry;

use crate::queries::{
    gather_archetype_data, gather_component_types, gather_entity_data, gather_reflected_types,
};
use crate::state::{
    ArchetypeDisplayInfo, ComponentTypeInfo, EntityDisplayInfo, ReflectedTypeInfo, SceneDisplayInfo,
};

pub(super) struct FrameDisplayData {
    pub(super) entities: Vec<EntityDisplayInfo>,
    pub(super) archetypes: Vec<ArchetypeDisplayInfo>,
    pub(super) component_types: Vec<ComponentTypeInfo>,
    pub(super) reflected_types: Vec<ReflectedTypeInfo>,
    pub(super) entity_count: usize,
    pub(super) archetype_count: usize,
    pub(super) active_archetype_count: usize,
    pub(super) scenes: Vec<SceneDisplayInfo>,
}

impl FrameDisplayData {
    pub(super) fn empty() -> Self {
        Self {
            entities: Vec::new(),
            archetypes: Vec::new(),
            component_types: Vec::new(),
            reflected_types: Vec::new(),
            entity_count: 0,
            archetype_count: 0,
            active_archetype_count: 0,
            scenes: Vec::new(),
        }
    }

    pub(super) fn gather(resources: &mut Resources) -> Self {
        // Intern every registry name first, so the read-only gather
        // functions below resolve each component to a stable id.
        crate::queries::intern_registry_names(resources);
        let entities = gather_entity_data(resources);
        let archetypes = gather_archetype_data(resources);
        let component_types = gather_component_types(resources);
        let reflected_types = gather_reflected_types(resources);
        let entity_count = entities.len();
        let archetype_count = resources
            .get::<ArchetypeRegistry>()
            .map_or(0, |a| a.archetype_count());
        let active_archetype_count = archetypes.iter().filter(|a| a.entity_count > 0).count();
        let scenes = gather_scenes(resources);
        Self {
            entities,
            archetypes,
            component_types,
            reflected_types,
            entity_count,
            archetype_count,
            active_archetype_count,
            scenes,
        }
    }
}

/// Snapshots the open scenes for the World panel.
fn gather_scenes(resources: &Resources) -> Vec<SceneDisplayInfo> {
    let Some(manager) = resources.get::<ome_ecs::SceneManager>() else {
        return Vec::new();
    };
    let active = manager.active_id();
    manager
        .scenes()
        .iter()
        .map(|scene| SceneDisplayInfo {
            id: scene.id,
            name: scene
                .path
                .as_ref()
                .and_then(|path| path.file_stem())
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Untitled".to_owned()),
            dirty: scene.dirty,
            active: active == Some(scene.id),
        })
        .collect()
}
