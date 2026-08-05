//! [`FrameDisplayData`] — ECS data gathered once per frame, before the
//! egui context runs. Decouples the queries from the UI pass so the
//! borrow scopes don't collide.

use kooch_core::resource::Resources;
use kooch_ecs::archetype_registry::ArchetypeRegistry;

use crate::perf::{GatherStages, ms_since};

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

    /// Gathers the frame's data, and says what each part of it cost.
    ///
    /// The timings are returned rather than written into the perf
    /// Resource here: the render system already assembles its stages as
    /// a local and publishes them once, and a second writer would make
    /// the order of two writes decide what the HUD shows.
    pub(super) fn gather(resources: &mut Resources) -> (Self, GatherStages) {
        let mut stages = GatherStages::default();

        // Intern every registry name first, so the read-only gather
        // functions below resolve each component to a stable id.
        let t = std::time::Instant::now();
        crate::queries::intern_registry_names(resources);
        stages.intern_ms = ms_since(t);

        // Read before the gather so the entities the Inspector will show
        // are known while their values are still being decided. The
        // overlay is still in `Resources` at this point in the frame;
        // the render system removes it further down.
        let detail_for: std::collections::HashSet<kooch_ecs::Entity> = resources
            .get::<crate::state::EditorOverlay>()
            .map(|overlay| overlay.selected_entities.iter().copied().collect())
            .unwrap_or_default();

        let t = std::time::Instant::now();
        let entities = gather_entity_data(resources, &detail_for);
        stages.entities_ms = ms_since(t);

        let t = std::time::Instant::now();
        let archetypes = gather_archetype_data(resources);
        stages.archetypes_ms = ms_since(t);

        let t = std::time::Instant::now();
        let component_types = gather_component_types(resources);
        let reflected_types = gather_reflected_types(resources);
        stages.types_ms = ms_since(t);

        let entity_count = entities.len();
        let archetype_count = resources
            .get::<ArchetypeRegistry>()
            .map_or(0, |a| a.archetype_count());
        let active_archetype_count = archetypes.iter().filter(|a| a.entity_count > 0).count();
        let scenes = gather_scenes(resources);
        (
            Self {
                entities,
                archetypes,
                component_types,
                reflected_types,
                entity_count,
                archetype_count,
                active_archetype_count,
                scenes,
            },
            stages,
        )
    }
}

/// Snapshots the open scenes for the World panel.
fn gather_scenes(resources: &Resources) -> Vec<SceneDisplayInfo> {
    let Some(manager) = resources.get::<kooch_ecs::SceneManager>() else {
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
