//! [`FrameDisplayData`] — ECS data gathered once per frame, before the
//! egui context runs. Decouples the queries from the UI pass so the
//! borrow scopes don't collide.

use std::collections::HashSet;

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
    /// The project's layer names, gathered once a frame for every mask the Inspector draws.
    pub(super) layer_labels: Vec<String>,
    pub(super) entity_count: usize,
    pub(super) archetype_count: usize,
    pub(super) active_archetype_count: usize,
    /// How many faces are painted on the block being edited, and on which entity. The Inspector
    /// draws from this snapshot and cannot read `BlockSelection` itself.
    pub(super) picked_faces: Option<(kooch_ecs::Entity, usize)>,
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
            picked_faces: None,
            layer_labels: kooch_core::layers::LayerNames::default().labels(),
            entity_count: 0,
            archetype_count: 0,
            active_archetype_count: 0,
            scenes: Vec::new(),
            systems: Vec::new(),
        }
    }

    /// Gathers the frame's data, and says what each part of it cost.
    pub(super) fn gather(
        resources: &mut Resources,
        detail_for: &HashSet<kooch_ecs::Entity>,
    ) -> (Self, GatherStages) {
        let mut stages = GatherStages::default();

        // Intern every registry name first, so the read-only gather
        // functions below resolve each component to a stable id.
        let t = std::time::Instant::now();
        crate::queries::intern_registry_names(resources);
        stages.intern_ms = ms_since(t);

        let t = std::time::Instant::now();
        let entities = gather_entity_data(resources, detail_for);
        stages.entities_ms = ms_since(t);

        let t = std::time::Instant::now();
        let archetypes = gather_archetype_data(resources);
        stages.archetypes_ms = ms_since(t);

        let t = std::time::Instant::now();
        let component_types = gather_component_types(resources);
        let reflected_types = gather_reflected_types(resources);
        let layer_labels = layer_labels(resources);
        stages.types_ms = ms_since(t);

        let entity_count = entities.len();
        let archetype_count = resources
            .get::<ArchetypeRegistry>()
            .map_or(0, |a| a.archetype_count());
        let active_archetype_count = archetypes.iter().filter(|a| a.entity_count > 0).count();
        // Face mode only: an edge or corner count would read as faces in the panel below.
        let picked_faces = picked_faces(resources);
        let scenes = gather_scenes(resources);
        let systems = gather_systems(resources);
        (
            Self {
                entities,
                archetypes,
                component_types,
                reflected_types,
                layer_labels,
                entity_count,
                archetype_count,
                active_archetype_count,
                picked_faces,
                scenes,
                systems,
            },
            stages,
        )
    }
}

/// What the project schedules, or what this editor does when there is no project.
/// The project's layer names, for every mask the Inspector draws.
fn layer_labels(resources: &Resources) -> Vec<String> {
    resources
        .get::<kooch_core::layers::LayerNames>()
        .map(|names| names.labels())
        .unwrap_or_else(|| kooch_core::layers::LayerNames::default().labels())
}

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

/// How many faces are painted on the block being edited, and on which entity.
///
/// Face mode only: an edge or corner count would read as faces in the panel below.
fn picked_faces(resources: &Resources) -> Option<(kooch_ecs::Entity, usize)> {
    let selection = resources.get::<crate::block_edit::BlockSelection>()?;
    match selection.mode == crate::block_edit::ElementMode::Face {
        true => Some((selection.entity?, selection.elements.len())),
        false => None,
    }
}

/// The snapshot plus what it was read from, so a frame that changed nothing reuses it.
///
/// 🔴 The gather walks every entity and cost 29.9 of the 40.4 ms of the main thread at 4,129
/// entities (#1561). Almost every frame of a session changes nothing it reads: the world's shape
/// moves when something spawns, is reparented or renamed, not when a body falls.
#[derive(Default)]
pub(super) struct FrameDisplayCache {
    data: Option<FrameDisplayData>,
    key: Key,
}

/// Everything the snapshot's rows are read from, as revisions.
///
/// 🔴 `GlobalTransform` is deliberately absent: it moves every frame in Play, and the only rows
/// that need it live are the selected ones, which [`refresh_detail`] re-reads anyway.
#[derive(Default, PartialEq)]
struct Key {
    archetypes: u64,
    names: u64,
    parents: u64,
    members: u64,
    order: u64,
    dynamic: usize,
    /// Registered component types, so a plugin reload rebuilds the "Add Component" lists.
    types: usize,
    detail: Vec<kooch_ecs::Entity>,
}

impl Key {
    fn read(resources: &Resources, detail_for: &HashSet<kooch_ecs::Entity>) -> Self {
        let components = resources.get::<kooch_ecs::component::ComponentRegistry>();
        let revision = |read: fn(&kooch_ecs::component::ComponentRegistry) -> Option<u64>| {
            components.as_deref().and_then(read).unwrap_or(0)
        };
        let mut detail: Vec<kooch_ecs::Entity> = detail_for.iter().copied().collect();
        detail.sort_unstable_by_key(|entity| (entity.index(), entity.generation()));
        Self {
            archetypes: resources
                .get::<ArchetypeRegistry>()
                .map_or(0, |archetypes| archetypes.revision()),
            names: revision(|r| Some(r.get_cpu::<kooch_ecs::Name>()?.revision())),
            parents: revision(|r| Some(r.get_cpu::<kooch_ecs::hierarchy::Parent>()?.revision())),
            members: revision(|r| Some(r.get_cpu::<kooch_ecs::SceneMember>()?.revision())),
            order: revision(|r| Some(r.get_cpu::<kooch_ecs::Order>()?.revision())),
            dynamic: resources
                .get::<kooch_ecs::dynamic_components::DynamicComponents>()
                .map_or(0, |dynamic| dynamic.len()),
            types: components.as_deref().map_or(0, |r| r.registered_count()),
            detail,
        }
    }
}

impl FrameDisplayCache {
    /// The frame's snapshot: the cached one with the selection re-read, or a fresh gather.
    pub(super) fn frame(
        &mut self,
        resources: &mut Resources,
        detail_for: &HashSet<kooch_ecs::Entity>,
    ) -> (&FrameDisplayData, GatherStages) {
        let key = Key::read(resources, detail_for);
        let mut stages = GatherStages::default();
        if self.data.is_some() && key == self.key {
            let t = std::time::Instant::now();
            let data = self.data.as_mut().expect("checked above");
            crate::queries::refresh_detail(resources, &mut data.entities, detail_for);
            // Re-read every frame rather than watched: each is bounded by the scene count or by
            // one selection, not by the world, and watching them would cost more than reading.
            data.picked_faces = picked_faces(resources);
            data.layer_labels = layer_labels(resources);
            data.scenes = gather_scenes(resources);
            data.systems = gather_systems(resources);
            stages.entities_ms = ms_since(t);
            stages.reused = true;
            return (self.data.as_ref().expect("checked above"), stages);
        }
        let (data, gathered) = FrameDisplayData::gather(resources, detail_for);
        self.key = key;
        self.data = Some(data);
        (self.data.as_ref().expect("just stored"), gathered)
    }
}
