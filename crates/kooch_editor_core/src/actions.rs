//! Editor actions collected during UI, applied after render.

pub(crate) mod asset_ops;
mod codegen;
mod dispatch;
pub(crate) mod entity_state;
pub(crate) mod handlers;
mod ide;

/// The IDE this machine would use, as a command string the Settings window can show and the user
/// can edit before applying.
pub(crate) fn detected_ide_command() -> Option<String> {
    let command = ide::from_desktop_defaults()?;
    let mut parts = vec![command.program];
    parts.extend(command.args);
    Some(parts.join(" "))
}
mod remote_edit;
pub(crate) mod remote_undo;
pub(crate) mod scene_io;

use std::any::TypeId;
use std::path::PathBuf;

use kooch_core::resource::Resources;
use kooch_ecs::component::ComponentId;
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::ReflectValue;
use kooch_ecs::transform::Transform;

use crate::undo::{CompoundCommand, EditorCommand, UndoStack};

use self::dispatch::{action_to_command, batch_description, same_ecs_variant};
use self::handlers::apply_non_ecs_action;

mod prefab_overrides;
pub(crate) mod prefab_propagate;

pub(crate) use self::asset_ops::main_scene_path;
pub(crate) use self::codegen::{
    initial_registrations, migrate_to_library, register_scripts, split_authoring,
};

/// What a collision bake produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BakeKind {
    /// One convex hull. The answer for a dynamic prop.
    Hull,
    /// Convex pieces that keep the hollows. Seconds of VHACD, which is
    /// why the result is a file.
    Parts,
    /// The triangles, decimated to a budget.
    Mesh,
}

impl BakeKind {
    /// The suffix its file takes, and the value its sidecar records.
    pub(crate) fn tag(self) -> &'static str {
        match self {
            Self::Hull => "hull",
            Self::Parts => "parts",
            Self::Mesh => "mesh",
        }
    }
}

pub(crate) enum EditorAction {
    /// Spawn an entity with Name + Transform + optional extra components.
    /// The optional String sets the Name component value.
    Spawn {
        extra: Vec<TypeId>,
        name: Option<String>,
        /// Which scene the new entity is authored into, and what it hangs off.
        into: SpawnTarget,
    },
    /// Spawn an entity bound to a meshlet asset. The asset path is resolved through the AssetServer
    /// (auto-generates a `.meta` sidecar at first import, registers the GUID in `AssetDatabase`)
    /// and the resulting GUID lands in `MeshRenderer.mesh`.
    SpawnMesh {
        path: PathBuf,
        name: String,
    },
    /// Spawn a block: writes a fresh `.block` holding `shape` into the project's assets and spawns
    /// an entity pointing at it.
    SpawnBlock {
        into: SpawnTarget,
        shape: kooch_blockmesh::Shape,
    },
    /// One block's shape, before and after an edit.
    BlockEdit {
        entity: Entity,
        source: kooch_core::Guid,
        before: Box<kooch_blockmesh::BlockMesh>,
        after: Box<kooch_blockmesh::BlockMesh>,
    },
    Despawn(Entity),
    /// Clones an existing entity's full component set (including reflected field values) into a new
    /// entity. The source stays untouched.
    Duplicate(Entity),
    /// Read entities into the editor's clipboard, replacing what was
    /// there. Carries the selection because the clipboard is filled from
    /// a panel that has one and the handler has not.
    CopyEntities(Vec<Entity>),
    /// Build the clipboard's contents as new entities, in `into`.
    PasteEntities {
        into: SpawnTarget,
    },
    /// Re-home an entity into another open scene.
    MoveToScene {
        entity: Entity,
        scene: kooch_core::Guid,
    },

    SetField {
        entity: Entity,
        component: ComponentId,
        field: String,
        value: ReflectValue,
    },
    AddComponent {
        entity: Entity,
        component: ComponentId,
    },
    RemoveComponent {
        entity: Entity,
        component: ComponentId,
    },
    /// Atomic Transform replacement, emitted by viewport gizmo handles
    /// at the end of a drag (one entry per drag, not per frame). The
    /// `desc` is the static label shown in the Edit menu's undo history.
    TransformEdit {
        entity: Entity,
        before: Transform,
        after: Transform,
        desc: &'static str,
    },
    /// Reverse the last edit **to one document**.
    Undo(crate::history::Document),
    Redo(crate::history::Document),
    SaveScene,
    /// Replace the world with a scene file.
    OpenScene {
        path: Option<std::path::PathBuf>,
    },
    /// Write an entity and its descendants to a scene file — a prefab.
    SavePrefab {
        entity: Entity,
        /// Folder to write into; `None` means the project's assets root.
        /// The drag-to-Assets path names the folder it was dropped on, the
        /// context menu does not.
        dest: Option<std::path::PathBuf>,
        /// Whether the user has already agreed to replace an existing file.
        overwrite: bool,
    },
    /// Replace a field on one component of one entity inside a prefab.
    EditPrefabField {
        prefab: kooch_core::Guid,
        entity_index: usize,
        component: String,
        field: String,
        value: kooch_ecs::reflect::ReflectValue,
    },
    /// Add or remove a component on one entity inside a prefab.
    EditPrefabComponent {
        prefab: kooch_core::Guid,
        entity_index: usize,
        /// The menu speaks `ComponentId`; the document stores a type name.
        /// Translating needs the registry, which the handler has and the
        /// panel does not.
        component: kooch_ecs::component::ComponentId,
        add: bool,
    },
    /// Write a prefab's edited document back to its file.
    SavePrefabAsset(kooch_core::Guid),
    /// Drop an instance's overrides so its fields follow the prefab again.
    RevertToPrefab {
        /// Any entity of the instance; the root is found from it.
        entity: Entity,
        /// `None` reverts the whole instance.
        component: Option<kooch_ecs::component::ComponentId>,
    },
    /// Push a saved prefab's values out to every instance of it, except the fields each instance
    /// overrode.
    PropagatePrefab(kooch_core::Guid),
    /// Tell the project a prefab file changed, so it stops instancing
    /// from the copy it read first.
    ReloadAssetOnHost(std::path::PathBuf),
    /// Dismiss the "replace this prefab?" prompt without saving.
    CancelPrefabOverwrite,
    /// Install the engine this editor ships over the one the project is
    /// building against. The next build of the project is a full one.
    UpdateEngine,
    /// Points a project at this editor's engine without opening it
    /// (#800). The launcher's version of [`Self::UpdateEngine`], which
    /// only ever ran as a side effect of opening a project.
    MoveProjectToEngine(std::path::PathBuf),
    /// Dismiss the engine notice and leave the installed engine alone.
    KeepEngine,
    /// Delete an installed engine by version. Never the one this editor
    /// ships, nor the one the open project builds against.
    RemoveEngine(String),
    /// Stamp a prefab into the open scene.
    InstantiatePrefab {
        /// The prefab asset. A guid rather than a path, so moving or
        /// renaming the file does not break whatever is holding it — the
        /// same reason `MeshRenderer.mesh` is one.
        prefab: kooch_core::Guid,
        /// Where to put the instance's root.
        at: crate::viewport_pick::DropPoint,
    },
    /// Open a scene beside the ones already loaded, rather than replacing them. The scene becomes
    /// the active one, so newly spawned entities land in it.
    OpenSceneAdditive {
        path: Option<std::path::PathBuf>,
    },
    /// Close one open scene, despawning only its entities.
    CloseScene(kooch_core::Guid),
    /// Make an already-open scene the one new entities are authored into.
    SetActiveScene(kooch_core::Guid),
    /// Write one open scene back to the file it came from.
    SaveOpenScene(kooch_core::Guid),
    /// Write one open scene to a path the user picks, and adopt it.
    SaveOpenSceneAs(kooch_core::Guid),
    /// Move an entity among its siblings: under `new_parent`, in front of `before`.
    MoveEntity {
        entity: Entity,
        /// `None` makes it a root of its scene.
        new_parent: Option<Entity>,
        /// The sibling it goes in front of; `None` puts it last.
        before: Option<Entity>,
    },
    /// Throw away one open scene's edits and read it back from its file.
    RevertOpenScene(kooch_core::Guid),
    Play,
    Stop,
    /// Open a project: launch its binary with `--remote` and drive its ECS over the wire.
    OpenProject(PathBuf),
    /// Rebuild the project and reconnect to the fresh binary. The only
    /// way to pick up code added since the session started — Rust is
    /// compiled ahead of time — and the way back from a dead session.
    RebuildAndRun,
    CreateProject {
        name: String,
        parent_path: PathBuf,
    },
    CloseProject,
    /// Run `cargo clean` on the open project.
    CleanProject,
    Reparent {
        entity: Entity,
        new_parent: Option<Entity>,
    },
    RemoveRecent(PathBuf),
    LaunchProject(PathBuf),
    CancelLaunch,
    /// Replace a `Material` asset's contents (PBR scalars + texture references). Emitted by the
    /// Asset Browser's material editor. Applied to `Assets<Material>` so the render sync picks it
    /// up live. Not undoable — an asset-level edit, distinct from the ECS field undo stack.
    EditMaterial {
        guid: kooch_core::Guid,
        material: kooch_render::material::Material,
        /// `false` while a drag is still in flight — update memory only.
        commit: bool,
    },
    /// Bakes a collision mesh out of a render mesh, into the project.
    BakeCollider {
        /// The mesh to derive from. Its own GUID is recorded in the
        /// result's sidecar, so a stale bake is detectable.
        source: kooch_core::Guid,
        kind: BakeKind,
        /// Face budget per piece. Zero keeps the exact hull, and is
        /// refused for [`BakeKind::Mesh`], which has nothing else to do.
        max_faces: u32,
    },
    /// Rewrites a texture's `[import]` table and re-imports it.
    SetImageImport {
        guid: kooch_core::Guid,
        import: kooch_render::texture::ImageImport,
    },
    /// Writes one field of a reflected asset (#744).
    EditAssetField {
        guid: kooch_core::Guid,
        field: String,
        value: kooch_ecs::reflect::ReflectValue,
        commit: bool,
    },
    /// Copy external files into a project folder and re-scan the asset
    /// database so they register as project assets. Emitted by the Asset
    /// Browser's drag-and-drop import. `dest` must be inside the project.
    ImportAssets {
        files: Vec<PathBuf>,
        dest: PathBuf,
    },
    /// Create an empty folder `<parent>/<name>`.
    CreateFolder {
        parent: PathBuf,
        name: String,
    },
    /// Create a new default `Material` asset `<folder>/<name>.material`, shading with `shader` when
    /// one is given.
    CreateMaterial {
        folder: PathBuf,
        name: String,
        shader: Option<kooch_core::Guid>,
    },
    /// Rename an asset file (and its `.meta` sidecar) to `new_name`,
    /// preserving the GUID so references survive.
    RenameAsset {
        path: PathBuf,
        new_name: String,
    },
    /// Rename a folder to `new_name`.
    RenameFolder {
        path: PathBuf,
        new_name: String,
    },
    /// Duplicate an asset file into a fresh copy (new GUID via re-import).
    DuplicateAsset {
        path: PathBuf,
    },
    /// Delete an asset file (and its `.meta` sidecar).
    DeleteAsset {
        path: PathBuf,
    },
    /// Delete a folder and everything under it.
    DeleteFolder {
        path: PathBuf,
    },
    /// Open the OS file manager at `path` (or its parent for a file).
    RevealInFileManager {
        path: PathBuf,
    },
    /// Make `path` the scene the project — and the game built from it — opens with (#808).
    SetSystemEnabled {
        name: String,
        nth: u32,
        enabled: bool,
    },
    SetMainScene {
        path: PathBuf,
    },
    /// Open `file` in an external IDE, with the project's **crate root** as the workspace, so the
    /// whole project (Rust source, `Cargo.toml`, …) is editable rather than the assets folder
    /// alone.
    OpenInIde {
        file: PathBuf,
    },
    /// Create a new source file (Rust / C# script) or an empty scene in
    /// `folder` from a stub template.
    CreateFile {
        folder: PathBuf,
        name: String,
        kind: NewFileKind,
    },
    /// Set (or clear, with `None`) the external IDE command used by
    /// [`OpenInIde`], persisted in the editor config.
    SetIdeCommand {
        command: Option<String>,
    },
    /// Set the environment the Play button launches the open project's
    /// game with, persisted in the editor config against that project's
    /// path. An empty line clears it.
    SetLaunchEnv {
        value: String,
    },
    /// Rescan the project's `src/` for components + systems and rewrite the editor-managed
    /// `src/registrations.rs` (regenerating `main.rs` if it is missing). Apply an edit to the open
    /// map, in memory.
    EditInputMap(crate::panels::input_map::InputMapAction),
    /// Write the open map back to its file.
    SaveInputMap,
    /// The dock has brought the Input Map panel forward; stop asking.
    InputMapFocused,
    /// Load an `.inputmap` and show it in the Input Map panel.
    OpenInputMap {
        path: std::path::PathBuf,
    },
    /// Read a generated `.shader` back into the Shader Graph panel (#1159).
    OpenShaderGraph {
        path: std::path::PathBuf,
    },
    /// Generate the `.shader` from the open graph and write it.
    SaveShaderGraph,
    /// The dock has brought the Shader Graph panel forward; stop asking.
    ShaderGraphFocused,
    /// Build and package the project with one of its presets (#758).
    BuildProject(kooch_core::Guid),
    /// Stop the running build.
    CancelBuild,
    RegisterScripts,
    /// Install what `preflight` found missing, and restart if this
    /// machine's package manager needs it. See [`crate::install`].
    InstallRequirements,
}

/// The kind of file created by [`EditorAction::CreateFile`]. The Rust
/// and Rhai kinds are scaffolded from `templates/` in the engine root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NewFileKind {
    RustComponent,
    RustSystem,
    Scene,
    /// One action on its own — what a component points at.
    InputAction,
    /// One way of building this project: target, output, packed (#758).
    BuildPreset,
    /// One editable block shape — a cube until somebody drags it (#946).
    BlockMesh,
    /// How the project looks: exposure, ambient, shadows (#744).
    RenderSettings,
    /// A surface shader, starting as a copy of the engine's PBR one (#1157).
    Shader,
    /// A shader the node graph owns (#1159): an empty graph, and the shader it generates.
    ShaderGraph,
}

/// Where a newly spawned entity goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpawnTarget {
    /// The scene new entities land in by default — the toolbar's Spawn
    /// button, and the World panel's empty area before this existed.
    Active,
    /// A named open scene, at its root.
    Scene(kooch_core::Guid),
    /// A child of an entity, in whatever scene that entity belongs to.
    ChildOf(Entity),
    /// A scene of its own, created empty and unsaved to hold it.
    NewScene,
}

impl EditorAction {
    /// Whether applying this needs the project's world to already be there.
    pub(crate) fn needs_a_live_world(&self) -> bool {
        match self {
            // Not the world's contents, but the project's schedule — and before the session
            // connects this would land on the editor's own instead, silently switching off the
            // wrong build's systems.
            Self::SetSystemEnabled { .. }
            // Everything that reads or writes the world, or persists it.
            | Self::Spawn { .. }
            | Self::SpawnMesh { .. }
            | Self::SpawnBlock { .. }
            | Self::BlockEdit { .. }
            | Self::Despawn(_)
            | Self::Duplicate(_)
            // Both read or write entities, so both wait for a world to
            // read them out of.
            | Self::CopyEntities(_)
            | Self::PasteEntities { .. }
            | Self::MoveToScene { .. }
            | Self::SetField { .. }
            | Self::AddComponent { .. }
            | Self::RemoveComponent { .. }
            | Self::TransformEdit { .. }
            | Self::Reparent { .. }
            | Self::SaveScene
            | Self::SavePrefab { .. }
            | Self::InstantiatePrefab { .. }
            | Self::OpenScene { .. }
            | Self::OpenSceneAdditive { .. }
            | Self::CloseScene(_)
            | Self::SetActiveScene(_)
            | Self::SaveOpenScene(_)
            | Self::SaveOpenSceneAs(_)
            | Self::RevertOpenScene(_)
            | Self::MoveEntity { .. }
            | Self::Play
            | Self::Stop
            | Self::RegisterScripts
            | Self::InstallRequirements
            => true,

            // Session and project lifecycle: these are how a user gets *out* of a stuck build, so
            // they must keep working.
            Self::BuildProject(_)
            | Self::CancelBuild
            | Self::OpenProject(_)
            | Self::RebuildAndRun
            | Self::CreateProject { .. }
            | Self::CloseProject
            | Self::LaunchProject(_)
            | Self::CancelLaunch
            | Self::RemoveRecent(_)
            // Cleaning is what you do *because* the world is not there,
            // and it disconnects the session itself before it starts.
            | Self::CleanProject
            // Answering a prompt is editor state; refusing it while a
            // project builds would leave the modal permanently up.
            | Self::CancelPrefabOverwrite
            // Dismissing the engine notice writes nothing, and
            // installing writes to disk outside the project rather than
            // to the world.
            | Self::KeepEngine
            | Self::MoveProjectToEngine(_)
            | Self::UpdateEngine
            | Self::RemoveEngine(_)
            // Nothing to do locally; it exists to reach the project.
            | Self::ReloadAssetOnHost(_)
            // Both write into the world, so they wait for one.
            | Self::PropagatePrefab(_)
            | Self::RevertToPrefab { .. }
            // A prefab is a file and a cached document. Neither is the
            // world, so editing one while a project builds is fine.
            | Self::EditPrefabField { .. }
            | Self::EditPrefabComponent { .. }
            | Self::SavePrefabAsset(_)
            // An input map is a file too. Editing bindings while a
            // project builds is exactly the half of #58 that works
            // without anything running.
            | Self::OpenInputMap { .. }
            | Self::EditInputMap(_)
            | Self::SaveInputMap
            | Self::InputMapFocused
            // The graph is a document this side owns, like the map above.
            | Self::OpenShaderGraph { .. }
            | Self::SaveShaderGraph
            | Self::ShaderGraphFocused => false,

            // Only the scene's history needs the world. A prefab or an
            // input map is a document this side owns, and undoing an edit
            // to one while the project compiles is fine.
            Self::Undo(document) | Self::Redo(document) => document.is_world(),

            // Editor preferences and things that act on files rather than
            // on the world. An asset edit is about a `.ron` on disk, and
            // the project is not holding it.
            | Self::SetIdeCommand { .. }
            | Self::SetLaunchEnv { .. }
            | Self::EditMaterial { .. }
            | Self::BakeCollider { .. }
            | Self::SetImageImport { .. }
            | Self::EditAssetField { .. }
            | Self::ImportAssets { .. }
            | Self::CreateFolder { .. }
            | Self::CreateMaterial { .. }
            | Self::RenameAsset { .. }
            | Self::RenameFolder { .. }
            | Self::DuplicateAsset { .. }
            | Self::DeleteAsset { .. }
            | Self::DeleteFolder { .. }
            | Self::RevealInFileManager { .. }
            | Self::OpenInIde { .. }
            // The manifest is a file beside the project, not the world.
            | Self::SetMainScene { .. }
            | Self::CreateFile { .. } => false,
        }
    }

    /// Whether this changes the project's WORLD, and so must be refused while the project is
    /// playing.
    pub(crate) fn is_a_world_edit(&self) -> bool {
        match self {
            // Structure and content of the world.
            Self::Spawn { .. }
            | Self::SpawnMesh { .. }
            | Self::SpawnBlock { .. }
            | Self::BlockEdit { .. }
            | Self::Despawn(_)
            | Self::Duplicate(_)
            | Self::PasteEntities { .. }
            | Self::MoveToScene { .. }
            | Self::SetField { .. }
            | Self::AddComponent { .. }
            | Self::RemoveComponent { .. }
            | Self::TransformEdit { .. }
            | Self::Reparent { .. }
            | Self::MoveEntity { .. }
            | Self::InstantiatePrefab { .. }
            | Self::PropagatePrefab(_)
            | Self::RevertToPrefab { .. }
            // Persisting the world is not a mutation of it, but it writes a FILE from a world
            // mid-simulation — the ball wherever it happened to roll. That is not the scene the
            // author saved, and it overwrites the one that was.
            | Self::SaveScene
            | Self::SavePrefab { .. }
            | Self::SaveOpenScene(_)
            | Self::SaveOpenSceneAs(_)
            | Self::RevertOpenScene(_)
            // Swapping what is loaded under a running simulation.
            | Self::OpenScene { .. }
            | Self::OpenSceneAdditive { .. }
            | Self::CloseScene(_)
            | Self::SetActiveScene(_) => true,

            // 🔴 Undo is refused rather than queued.
            Self::Undo(document) | Self::Redo(document) => document.is_world(),

            // Reading the world is fine — a copy takes nothing away, and
            // the clipboard is the editor's.
            Self::CopyEntities(_)
            // Play and Stop are the control itself.
            | Self::Play
            | Self::Stop
            // Everything below is a file, a preference, or session
            // lifecycle. None of them is the running world.
            | Self::RegisterScripts
            | Self::InstallRequirements
            | Self::BuildProject(_)
            | Self::CancelBuild
            | Self::OpenProject(_)
            | Self::RebuildAndRun
            | Self::CreateProject { .. }
            | Self::CloseProject
            | Self::LaunchProject(_)
            | Self::CancelLaunch
            | Self::RemoveRecent(_)
            | Self::CleanProject
            | Self::CancelPrefabOverwrite
            | Self::KeepEngine
            | Self::MoveProjectToEngine(_)
            | Self::UpdateEngine
            | Self::RemoveEngine(_)
            | Self::ReloadAssetOnHost(_)
            | Self::EditPrefabField { .. }
            | Self::EditPrefabComponent { .. }
            | Self::SavePrefabAsset(_)
            | Self::OpenInputMap { .. }
            | Self::EditInputMap(_)
            | Self::SaveInputMap
            | Self::InputMapFocused
            | Self::OpenShaderGraph { .. }
            | Self::SaveShaderGraph
            | Self::ShaderGraphFocused
            | Self::SetIdeCommand { .. }
            | Self::SetLaunchEnv { .. }
            | Self::EditMaterial { .. }
            | Self::BakeCollider { .. }
            | Self::SetImageImport { .. }
            | Self::EditAssetField { .. }
            | Self::ImportAssets { .. }
            | Self::CreateFolder { .. }
            | Self::CreateMaterial { .. }
            | Self::RenameAsset { .. }
            | Self::RenameFolder { .. }
            | Self::DuplicateAsset { .. }
            | Self::DeleteAsset { .. }
            | Self::DeleteFolder { .. }
            | Self::RevealInFileManager { .. }
            | Self::OpenInIde { .. }
            | Self::SetMainScene { .. }
            // 🔴 NOT a world edit, on purpose. Switching a system off
            // while it runs is the whole point of having the switch, so
            // the Play guard must not block it.
            | Self::SetSystemEnabled { .. }
            | Self::CreateFile { .. } => false,
        }
    }
}

/// Prefabs edited in the Inspector whose file is behind the cache.
#[derive(Default)]
pub(crate) struct DirtyPrefabs(std::collections::HashSet<kooch_core::Guid>);

impl DirtyPrefabs {
    pub(crate) fn contains(&self, prefab: kooch_core::Guid) -> bool {
        self.0.contains(&prefab)
    }

    pub(crate) fn mark(&mut self, prefab: kooch_core::Guid) {
        self.0.insert(prefab);
    }

    pub(crate) fn clear(&mut self, prefab: kooch_core::Guid) {
        self.0.remove(&prefab);
    }
}

/// A prefab save waiting on the user's answer about replacing a file.
#[derive(Clone)]
pub(crate) struct PendingPrefabOverwrite {
    pub(crate) entity: Entity,
    pub(crate) dest: Option<std::path::PathBuf>,
    /// The file that would be replaced. Shown to the user, so they are
    /// answering about a name they recognise rather than about "a prefab".
    pub(crate) path: std::path::PathBuf,
}

/// Holds back any `SavePrefab` that would replace an existing file.
fn intercept_prefab_overwrites<'a>(
    resources: &mut Resources,
    actions: &'a [EditorAction],
) -> Vec<&'a EditorAction> {
    let mut out = Vec::with_capacity(actions.len());
    for action in actions {
        // The answer arrived; the prompt has done its job either way.
        if matches!(
            action,
            EditorAction::CancelPrefabOverwrite
                | EditorAction::SavePrefab {
                    overwrite: true,
                    ..
                }
        ) {
            resources.remove::<PendingPrefabOverwrite>();
        }
        let EditorAction::SavePrefab {
            entity,
            dest,
            overwrite: false,
        } = action
        else {
            out.push(action);
            continue;
        };
        // No project open: the handler says so. Not this function's job to
        // report, and holding the action back would swallow the message.
        let Some(root) = crate::actions::handlers::prefab_root(resources) else {
            out.push(action);
            continue;
        };
        let name = crate::actions::handlers::entity_name(resources, *entity);
        let path = crate::actions::handlers::prefab_path(&root, &name, dest.as_deref());
        if !path.exists() {
            out.push(action);
            continue;
        }
        resources.insert(PendingPrefabOverwrite {
            entity: *entity,
            dest: dest.clone(),
            path,
        });
    }
    out
}

pub(crate) fn apply_actions(
    resources: &mut Resources,
    actions: &[EditorAction],
    undo_stack: &mut UndoStack,
) {
    // Dual-sink: with a connected remote session the editor's ECS is a mirror of a project that
    // owns the real state, so ECS edits route over the wire instead of mutating the mirror (which
    // the next refresh would overwrite).
    let mut queued: Vec<EditorAction> = resources
        .get_mut::<prefab_propagate::PendingPropagation>()
        .map(|pending| pending.drain())
        .unwrap_or_default()
        .into_iter()
        .map(EditorAction::PropagatePrefab)
        .collect();
    // Ahead of the propagation, so the project has dropped its stale copy
    // before anything asks it to rebuild from one.
    let reloads: Vec<EditorAction> = resources
        .get_mut::<handlers::PendingHostReloads>()
        .map(|pending| std::mem::take(&mut pending.0))
        .unwrap_or_default()
        .into_iter()
        .map(EditorAction::ReloadAssetOnHost)
        .collect();
    if !reloads.is_empty() {
        queued.splice(0..0, reloads);
    }
    // Ahead of everything: a world held across a rebuild has to be back
    // before anything else acts on the scene it is supposed to be in.
    let resumed = crate::carry::resume(resources);
    if !resumed.is_empty() {
        queued.splice(0..0, resumed);
    }
    if !queued.is_empty() {
        // 🔴 `debug`, not `info`. A live prefab drains every frame, so at `info` this printed sixty
        // identical lines a second and buried every other message in the Console — including the
        // ones a measurement run is there to read.
        tracing::debug!(
            target: "kooch_editor_core::prefab",
            drained = queued.len(),
            "propagation drained into actions",
        );
    }

    // Asked before the local/remote split so the prompt appears once
    // regardless of which path would have written the file.
    let mut actions = intercept_prefab_overwrites(resources, actions);
    actions.extend(queued.iter());
    let actions = &actions;

    // Recorded before the edits are applied, while the instance still holds the values the user is
    // changing away from — and appended so the write that persists the set travels the same path as
    // the edit that caused it.
    let recorded = prefab_overrides::record(resources, actions);
    let mut owned: Vec<&EditorAction>;
    let actions = match recorded.is_empty() {
        true => actions,
        false => {
            owned = actions.clone();
            owned.extend(recorded.iter());
            &owned
        }
    };

    let remote = resources
        .get::<crate::remote_session::RemoteState>()
        .is_some_and(|s| s.is_connected());

    // A session that exists but has not answered yet: the project is still building and its world
    // has not arrived. Dropping the actions that need it is what stops a Ctrl+S from writing the
    // empty mirror over the project's scene — see `needs_a_live_world`.
    let awaiting_world = !remote
        && resources
            .get::<crate::remote_session::RemoteState>()
            .is_some_and(|state| state.session.is_some());
    if awaiting_world {
        let held = actions
            .iter()
            .filter(|action| action.needs_a_live_world())
            .count();
        if held > 0 {
            tracing::warn!(
                refused = held,
                "the project is still starting — edits are refused until its world arrives",
            );
        }
        for action in actions.iter().copied().filter(|a| !a.needs_a_live_world()) {
            apply_non_ecs_action(action, resources, undo_stack);
        }
        return;
    }

    // 🔴 A playing project owns its world and the editor does not get to touch it.
    let playing = resources
        .get::<crate::remote_session::RemoteState>()
        .is_some_and(|state| state.playing);
    if playing {
        let refused = actions.iter().filter(|a| a.is_a_world_edit()).count();
        if refused > 0 {
            tracing::warn!(
                refused,
                "the project is playing — stop it to edit the world",
            );
        }
    }

    if remote {
        for action in actions.iter().copied() {
            if playing && action.is_a_world_edit() {
                continue;
            }
            if !remote_edit::dispatch(resources, action) {
                apply_non_ecs_action(action, resources, undo_stack);
            }
        }
        return;
    }

    let mut i = 0;
    while i < actions.len() {
        let action = actions[i];

        // Undo/Redo are handled directly — the scene's here, and every
        // other document by the handler below.
        if let EditorAction::Undo(document) | EditorAction::Redo(document) = action {
            let undo = matches!(action, EditorAction::Undo(_));
            match (document.is_world(), undo) {
                (true, true) => undo_stack.undo(resources),
                (true, false) => undo_stack.redo(resources),
                (false, _) => {
                    crate::history::documents::step(resources, document, undo);
                }
            }
            i += 1;
            continue;
        }

        // Check if this is an ECS action that can be batched.
        if action_to_command(action, resources).is_some() {
            // Find the run of consecutive same-variant ECS actions.
            let run_start = i;
            let mut run_end = i + 1;
            while run_end < actions.len() && same_ecs_variant(action, &actions[run_end]) {
                run_end += 1;
            }
            let run = &actions[run_start..run_end];

            if run.len() == 1 {
                // Single action — execute directly (snapshot already captured above
                // was discarded; re-capture since resources may have changed).
                if let Some(cmd) = action_to_command(run[0], resources) {
                    undo_stack.execute(cmd, resources);
                }
            } else {
                // Multiple same-type actions — batch into a CompoundCommand.
                let desc = batch_description(run);
                let mut cmds: Vec<Box<dyn EditorCommand>> = Vec::with_capacity(run.len());
                for a in run.iter().copied() {
                    // Snapshot must be taken sequentially: each command's
                    // before-state depends on the previous command's execution.
                    if let Some(cmd) = action_to_command(a, resources) {
                        cmds.push(cmd);
                    }
                }
                let compound = CompoundCommand::new(desc, cmds);
                undo_stack.execute(Box::new(compound), resources);
            }

            i = run_end;
            continue;
        }

        // Non-ECS actions: process directly (no undo).
        apply_non_ecs_action(action, resources, undo_stack);
        i += 1;
    }
}

#[cfg(test)]
mod tests;
