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
    pub(super) systems: Vec<kooch_remote::protocol::SystemEntry>,
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
            systems: Vec::new(),
        }
    }

    /// Gathers the frame's data, and says what each part of it cost.
    pub(super) fn gather(resources: &mut Resources) -> (Self, GatherStages) {
        let mut stages = GatherStages::default();

        // Intern every registry name first, so the read-only gather
        // functions below resolve each component to a stable id.
        let t = std::time::Instant::now();
        crate::queries::intern_registry_names(resources);
        stages.intern_ms = ms_since(t);

        // Read before the gather so the entities the Inspector will show are known while their
        // values are still being decided. The overlay is still in `Resources` at this point in the
        // frame; the render system removes it further down.
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
        let systems = gather_systems(resources);
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
                systems,
            },
            stages,
        )
    }
}

/// What the project schedules, or what this editor does when there is no project.
fn gather_systems(resources: &Resources) -> Vec<kooch_remote::protocol::SystemEntry> {
    if let Some(systems) = resources
        .get::<crate::remote_session::RemoteState>()
        .and_then(|state| state.session.as_ref())
        .and_then(|session| session.systems())
    {
        return systems.to_vec();
    }
    local_systems(resources)
}

/// The editor's own schedule, read from the catalog it published.
fn local_systems(resources: &Resources) -> Vec<kooch_remote::protocol::SystemEntry> {
    use kooch_core::schedule::{SystemCatalog, SystemSource, SystemToggles};

    let Some(catalog) = resources.get::<SystemCatalog>() else {
        return Vec::new();
    };
    let toggles = resources.get::<SystemToggles>();
    catalog
        .all()
        .iter()
        .map(|system| kooch_remote::protocol::SystemEntry {
            stage: format!("{:?}", system.stage),
            name: system.name.clone(),
            short: system.short_name().to_owned(),
            nth: system.key.nth,
            project: system.source == SystemSource::Project,
            gpu: system.gpu,
            enabled: !toggles
                .as_ref()
                .is_some_and(|toggles| toggles.is_disabled(&system.key)),
        })
        .collect()
}

/// Snapshots the open scenes for the World panel.
fn gather_scenes(resources: &Resources) -> Vec<SceneDisplayInfo> {
    if let Some(scenes) = remote_scenes(resources) {
        return scenes;
    }
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
            path: scene.path.clone(),
            dirty: scene.dirty,
            active: active == Some(scene.id),
        })
        .collect()
}

/// The open scenes as the connected project reports them.
fn remote_scenes(resources: &Resources) -> Option<Vec<SceneDisplayInfo>> {
    let session = resources
        .get::<crate::remote_session::RemoteState>()?
        .session
        .as_ref()?;
    let scenes = session.open_scenes()?;
    Some(
        scenes
            .iter()
            .map(|scene| SceneDisplayInfo {
                id: scene.id,
                name: scene
                    .path
                    .as_deref()
                    .map(std::path::Path::new)
                    .and_then(|path| path.file_stem())
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Untitled".to_owned()),
                // The project's path, on the project's disk. Both
                // processes see the same filesystem, so it is meaningful
                // here — the same reason the scene dialog is.
                path: scene.path.as_deref().map(std::path::PathBuf::from),
                dirty: scene.dirty,
                active: scene.active,
            })
            .collect(),
    )
}
