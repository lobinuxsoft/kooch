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
    /// Write the active scene: to its own file, or to one asked for when `as_new` or never saved.
    SaveScene {
        as_new: bool,
    },
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
    /// Renames one of the project's layers (#1218). `guid` is the `.layers` file, or `None` when
    /// the project has none yet and one has to be written first.
    RenameLayer {
        guid: Option<kooch_core::Guid>,
        index: usize,
        name: String,
    },
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
    /// What the project calls its 32 layers, read by renderers and colliders alike (#1218).
    Layers,
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

/// Prefabs edited in the Inspector whose file is behind the cache.
mod apply;
mod describe;

pub(crate) use apply::{DirtyPrefabs, PendingPrefabOverwrite, apply_actions};

#[cfg(test)]
mod tests;
