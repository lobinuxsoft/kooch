//! Editor actions collected during UI, applied after render.

mod asset_ops;
mod codegen;
mod dispatch;
pub(crate) mod entity_state;
pub(crate) mod handlers;
mod ide;

/// The IDE this machine would use, as a command string the Settings
/// window can show and the user can edit before applying.
///
/// `None` when nothing could be resolved — on a system without
/// `xdg-mime`, or with no handler registered for source files.
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
    SyncOutcome, initial_registrations, migrate_to_library, register_scripts, split_authoring,
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
    ///
    /// The only bake that can be *wrong*. Collapsing an edge moves the
    /// surface, so a decimated floor is a floor in a slightly different
    /// place — and slightly lower is a floor a character sinks into. The
    /// two convex ones can only ever enclose more than they were given.
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
        /// Which scene the new entity is authored into, and what it hangs
        /// off.
        ///
        /// 🔴 Carried rather than inferred. Every spawn used to land in
        /// the active scene, which is the right answer for the toolbar
        /// button and the wrong one for a menu opened on a scene, or on
        /// an entity, that is not the active one — the entity would
        /// appear somewhere other than where it was asked for, and the
        /// only sign of it is a row in the wrong group.
        into: SpawnTarget,
    },
    /// Spawn an entity bound to a meshlet asset. The asset path is
    /// resolved through the AssetServer (auto-generates a `.meta`
    /// sidecar at first import, registers the GUID in `AssetDatabase`)
    /// and the resulting GUID lands in `MeshRenderer.mesh`.
    SpawnMesh {
        path: PathBuf,
        name: String,
    },
    Despawn(Entity),
    /// Clones an existing entity's full component set (including
    /// reflected field values) into a new entity. The source stays
    /// untouched. Used by the World panel "Duplicate" button to
    /// quickly bring up parallel test entities (e.g. the LOD-stack
    /// inspector workflow needs N copies of one mesh entity).
    Duplicate(Entity),
    /// Read entities into the editor's clipboard, replacing what was
    /// there. Carries the selection because the clipboard is filled from
    /// a panel that has one and the handler has not.
    CopyEntities(Vec<Entity>),
    /// Build the clipboard's contents as new entities, in `into`.
    ///
    /// 🔴 It names the DESTINATION and not the source. What to paste is
    /// whatever was copied — a paste that named its own source would be
    /// a duplicate — but where it lands is a choice, and leaving it
    /// unnamed is what made entities copied out of one scene appear
    /// under "Unsaved" instead of in the scene somebody right-clicked.
    PasteEntities {
        into: SpawnTarget,
    },
    /// Re-home an entity into another open scene.
    ///
    /// 🔴 A move, not a copy. Dragging a row onto a scene header is the
    /// direct-manipulation form of the paste target above, and an entity
    /// belongs to exactly one scene — so the source stops holding it and
    /// both files are dirty afterwards.
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
    ///
    /// The document travels with the chord because only the UI knows
    /// which one is being looked at, and the whole point of #813 is that
    /// a Ctrl+Z in the Input Map panel must not reach the scene.
    Undo(crate::history::Document),
    Redo(crate::history::Document),
    SaveScene,
    /// Replace the world with a scene file.
    ///
    /// `None` raises a file dialog — the File menu, which has no file in
    /// mind. `Some` is a caller that already named one: an Assets panel
    /// row IS the path, and asking for a file the click just identified
    /// is the same fault as having no way to name it at all.
    OpenScene {
        path: Option<std::path::PathBuf>,
    },
    /// Write an entity and its descendants to a scene file — a prefab.
    ///
    /// A prefab is a scene; see
    /// [`SceneDocument::from_ecs_subtree`](kooch_ecs::scene::SceneDocument::from_ecs_subtree)
    /// for why there is no separate format (#611).
    ///
    /// The file is named after the entity, read from the entity itself
    /// rather than passed in: the name lives on a `Name` component, and a
    /// caller that had to look it up first — the Assets panel, which only
    /// receives a dragged handle — would need the world it does not have.
    SavePrefab {
        entity: Entity,
        /// Folder to write into; `None` means the project's assets root.
        /// The drag-to-Assets path names the folder it was dropped on, the
        /// context menu does not.
        dest: Option<std::path::PathBuf>,
        /// Whether the user has already agreed to replace an existing file.
        ///
        /// Emitted `false` by every trigger. `apply_actions` turns a
        /// collision into a confirmation prompt and re-emits with `true`
        /// once answered, so the check lives in one place for both the
        /// local and the remote path.
        overwrite: bool,
    },
    /// Replace a field on one component of one entity inside a prefab.
    ///
    /// Addresses the entity by its index in the document rather than by a
    /// handle: a prefab's entities do not exist, which is the whole
    /// difference between editing one and editing a scene.
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
    ///
    /// `component` is `None` for the whole instance. Without this an
    /// override is permanent: an accidental gizmo drag detaches that
    /// transform from the prefab forever, and the only way back is
    /// deleting the instance and placing a new one.
    RevertToPrefab {
        /// Any entity of the instance; the root is found from it.
        entity: Entity,
        /// `None` reverts the whole instance.
        ///
        /// A `ComponentId` because that is what a panel has; the document
        /// stores type names, and the registry that translates lives with
        /// the handler.
        component: Option<kooch_ecs::component::ComponentId>,
    },
    /// Push a saved prefab's values out to every instance of it, except
    /// the fields each instance overrode.
    ///
    /// Carries its own writes rather than expanding into `SetField`s: an
    /// edit on an instance is recorded as an override, so propagating that
    /// way would pin every field it touched and the instance would never
    /// follow the prefab again.
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
        ///
        /// Unresolved on purpose: a viewport drop names a place on
        /// *screen*, and turning that into a world position needs the
        /// camera, which the panel that reported the drop cannot read. See
        /// [`DropPoint`](crate::viewport_pick::DropPoint).
        at: crate::viewport_pick::DropPoint,
    },
    /// Open a scene beside the ones already loaded, rather than replacing
    /// them. The scene becomes the active one, so newly spawned entities
    /// land in it.
    ///
    /// `None` asks; `Some` is a caller that already named the file. See
    /// [`Self::OpenScene`].
    OpenSceneAdditive {
        path: Option<std::path::PathBuf>,
    },
    /// Close one open scene, despawning only its entities.
    CloseScene(kooch_core::Guid),
    /// Make an already-open scene the one new entities are authored into.
    SetActiveScene(kooch_core::Guid),
    /// Write one open scene back to the file it came from.
    ///
    /// Named, not implied. The File menu's [`Self::SaveScene`] saves the
    /// active scene, and with several open the one somebody right-clicked
    /// is routinely not that — saving the wrong file is not a mistake the
    /// user can see until the next load.
    ///
    /// Falls back to asking for a path when the scene has never been
    /// saved, which is the only case where there is nothing to write to.
    SaveOpenScene(kooch_core::Guid),
    /// Write one open scene to a path the user picks, and adopt it.
    SaveOpenSceneAs(kooch_core::Guid),
    /// Move an entity among its siblings: under `new_parent`, in front of
    /// `before`.
    ///
    /// Where, not what number. "Before that one" is what a drag means,
    /// and the numbering that expresses it is the engine's
    /// (`kooch_ecs::order::place`) — a caller that picked values would
    /// put the renumbering rule in every caller, and they would disagree
    /// the first time a gap ran out.
    MoveEntity {
        entity: Entity,
        /// `None` makes it a root of its scene.
        new_parent: Option<Entity>,
        /// The sibling it goes in front of; `None` puts it last.
        before: Option<Entity>,
    },
    /// Throw away one open scene's edits and read it back from its file.
    ///
    /// Only that scene. With several open, "discard changes" that threw
    /// away every scene's would destroy work in files the user never
    /// touched.
    RevertOpenScene(kooch_core::Guid),
    Play,
    Stop,
    /// Open a project: launch its binary with `--remote` and drive its
    /// ECS over the wire. The project owns its own component types, so
    /// this is the only way the hub can edit a project it was never
    /// compiled against — and it is the only mode, so Play always runs
    /// gameplay in the editor's viewport.
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
    ///
    /// `cargo clean` rather than deleting `target/` by hand: the
    /// directory is not always there. `CARGO_TARGET_DIR` and
    /// `.cargo/config.toml` can move it, and a `rm -rf ./target` against
    /// a redirected build would remove nothing while reporting success.
    CleanProject,
    Reparent {
        entity: Entity,
        new_parent: Option<Entity>,
    },
    RemoveRecent(PathBuf),
    LaunchProject(PathBuf),
    CancelLaunch,
    /// Replace a `Material` asset's contents (PBR scalars + texture
    /// references). Emitted by the Asset Browser's material editor.
    /// Applied to `Assets<Material>` so the render sync picks it up live.
    /// Not undoable — an asset-level edit, distinct from the ECS field
    /// undo stack.
    ///
    /// `commit` separates *what the user is seeing* from *what is worth
    /// writing down*. A slider reports a change every frame it is
    /// dragged: persisting each one wrote the file, read it back and made
    /// a round trip to the project — 29 times for one drag, measured. In
    /// between, the live copy is enough; the file is written when the
    /// drag ends.
    EditMaterial {
        guid: kooch_core::Guid,
        material: kooch_render::material::Material,
        /// `false` while a drag is still in flight — update memory only.
        commit: bool,
    },
    /// Bakes a collision mesh out of a render mesh, into the project.
    ///
    /// A file rather than a runtime cache because the concave case is
    /// seconds of VHACD per body build, because an artist has to be able
    /// to open what the solver collides against, and because a bake is
    /// the only place a hull may be simplified below its exact form.
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
    ///
    /// No `commit` flag, unlike the two below: this is a checkbox, and a
    /// checkbox has no drag to be in the middle of.
    SetImageImport {
        guid: kooch_core::Guid,
        import: kooch_render::texture::ImageImport,
    },
    /// Writes one field of a reflected asset (#744).
    ///
    /// The generic counterpart to `EditMaterial`: any type registered
    /// with `register_reflected_asset!` is edited through this, so a new
    /// asset type needs no new action and no new handler.
    ///
    /// `commit` carries the same meaning it does there — `false` while a
    /// drag is in flight, so the file is written once per gesture rather
    /// than once per frame.
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
    /// Create a new default `Material` asset `<folder>/<name>.ron`.
    CreateMaterial {
        folder: PathBuf,
        name: String,
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
    /// Make `path` the scene the project — and the game built from it —
    /// opens with (#808).
    ///
    /// `path` is absolute, the way the asset tree carries it; the handler
    /// is what turns it into the project-relative form the manifest
    /// stores. An absolute path written into `project.kooch` would work
    /// on the machine that clicked and nowhere else, and nothing would
    /// report it until the game opened an empty scene.
    /// Stop or restart one scheduled system, from its next frame.
    ///
    /// Addressed by name and occurrence, never by index: an index moves
    /// the moment a plugin is added, and two anonymous closures in one
    /// module share a name (#982).
    SetSystemEnabled {
        name: String,
        nth: u32,
        enabled: bool,
    },
    SetMainScene {
        path: PathBuf,
    },
    /// Open `file` in an external IDE, with the project's **crate root**
    /// as the workspace, so the whole project (Rust source,
    /// `Cargo.toml`, …) is editable rather than the assets folder alone.
    ///
    /// # Why the workspace is not a parameter
    ///
    /// It was, and all three places that build this action passed the
    /// asset browser's root — the `assets/` directory — so the IDE
    /// opened a workspace with no source in it. The workspace is not a
    /// property of the click; it is a property of where the file lives,
    /// and the handler is the one place that knows.
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
    /// Rescan the project's `src/` for components + systems and rewrite
    /// the editor-managed `src/registrations.rs` (regenerating `main.rs`
    /// if it is missing).
    /// Apply an edit to the open map, in memory.
    ///
    /// The file is not touched. `SaveInputMap` is what reaches disk, so
    /// closing without saving discards — which is what every other
    /// document editor does and what a prefab already does here.
    EditInputMap(crate::panels::input_map::InputMapAction),
    /// Write the open map back to its file.
    SaveInputMap,
    /// The dock has brought the Input Map panel forward; stop asking.
    ///
    /// A one-shot rather than the panel clearing the flag itself: the
    /// draw borrows the resource immutably, and a frame that both reads
    /// and writes the same state is where a "why does this flicker" bug
    /// comes from.
    InputMapFocused,
    /// Load an `.inputmap` and show it in the Input Map panel.
    ///
    /// A dedicated panel rather than the Inspector. An input map is a
    /// document — action maps, actions, bindings, a live column — and the
    /// Inspector draws *component fields on the selected entity*. Putting
    /// every asset type through one panel is how it stops being good at
    /// any of them.
    OpenInputMap {
        path: std::path::PathBuf,
    },
    /// Build and package the project with one of its presets (#758).
    ///
    /// Carries the preset's guid rather than the preset: the panel has a
    /// handle, and what it points at may have been edited in the
    /// Inspector since — the handler reads the current one.
    BuildProject(kooch_core::Guid),
    /// Stop the running build.
    ///
    /// cargo is killed rather than asked: it has no "stop when
    /// convenient", and a build still compiling after the button said it
    /// stopped is worse than an interrupted one.
    CancelBuild,
    RegisterScripts,
    /// Install what `preflight` found missing, and restart if this
    /// machine's package manager needs it. See [`crate::install`].
    InstallRequirements,
    /// The author saw the resync notice. Clears it; the rebuild is
    /// theirs to run.
    AcknowledgeScriptSync,
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
    ///
    /// **Several per project**, unlike settings — "Windows release" and
    /// "Linux debug" are two presets, not one with a switch.
    BuildPreset,
    /// How the project looks: exposure, ambient, shadows (#744).
    ///
    /// **One per project.** The menu hides this once the project has
    /// one — a second file is read by nothing and produces a warning
    /// nobody sees.
    RenderSettings,
}

/// Where a newly spawned entity goes.
///
/// A scene and a parent are one question, not two: an entity's scene is
/// its parent's, so naming a parent already names the scene. Splitting
/// them into separate fields would let a caller ask for a child of an
/// entity in one scene and a member of another, which nothing can honour.
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
    ///
    /// What right-clicking the panel's empty space means: not "put this
    /// somewhere" but "start something new". An entity has to belong to a
    /// scene, so starting one is what makes the request answerable.
    NewScene,
}

impl EditorAction {
    /// Whether applying this needs the project's world to already be
    /// there.
    ///
    /// # Why this exists
    ///
    /// Opening a project builds it, which takes tens of seconds, and the
    /// dock is up and clickable for all of them while the mirror is still
    /// empty. `apply_actions` routes over the wire only once the session
    /// is *connected*; before that every edit falls through to the local
    /// path and mutates the empty mirror instead.
    ///
    /// `SaveScene` is the one that hurts: it would write that empty ECS
    /// over the project's real scene file. Not a panic — a scene deleted
    /// by pressing Ctrl+S during a build.
    ///
    /// # Why an exhaustive match
    ///
    /// No wildcard arm on purpose. A new variant has to come here and say
    /// which side it is on, which is the only way this stays true as the
    /// list grows past forty.
    pub(crate) fn needs_a_live_world(&self) -> bool {
        match self {
            // Not the world's contents, but the project's schedule —
            // and before the session connects this would land on the
            // editor's own instead, silently switching off the wrong
            // build's systems.
            Self::SetSystemEnabled { .. }
            // Everything that reads or writes the world, or persists it.
            | Self::Spawn { .. }
            | Self::SpawnMesh { .. }
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
            | Self::AcknowledgeScriptSync => true,

            // Session and project lifecycle: these are how a user gets
            // *out* of a stuck build, so they must keep working.
            //
            // Building belongs here rather than above: it reads the
            // project from disk and never touches the ECS, so it works
            // while a project is still compiling and its world is empty.
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
            | Self::InputMapFocused => false,

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

    /// Whether this changes the project's WORLD, and so must be refused
    /// while the project is playing.
    ///
    /// # Why the editor goes read-only under Play
    ///
    /// The engine accepts edits from the editor only while it is not
    /// simulating. An edit sent mid-play lands in a world the game is
    /// already stepping, so the next tick either overwrites it or
    /// simulates from a state the author never saw — and the editor
    /// showed neither as an error.
    ///
    /// It is also what makes the frame affordable. The mirror carries
    /// editing machinery — a reflected copy of every component of every
    /// entity — that exists so a field can be typed into. Nothing can be
    /// typed into while this returns `true`, so nothing has to be
    /// carried, and #1012's thin play pull is that consequence rather
    /// than a separate optimisation.
    ///
    /// # Why an exhaustive match
    ///
    /// The same reason [`Self::needs_a_live_world`] has one, and the
    /// same failure if it did not: a wildcard would let the next variant
    /// added default to "allowed", and an edit that slips through under
    /// Play is invisible until a simulation diverges.
    pub(crate) fn is_a_world_edit(&self) -> bool {
        match self {
            // Structure and content of the world.
            Self::Spawn { .. }
            | Self::SpawnMesh { .. }
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
            // Persisting the world is not a mutation of it, but it
            // writes a FILE from a world mid-simulation — the ball
            // wherever it happened to roll. That is not the scene the
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

            // 🔴 Undo is refused rather than queued. A stack whose
            // entries describe a world that has since been simulated
            // cannot be replayed onto it, and holding the presses to
            // apply on Stop would undo several steps at once, at a
            // moment the user is not looking at the thing being undone.
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
            | Self::AcknowledgeScriptSync
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
///
/// The edits themselves live in `Assets<SceneDocument>` — which is what
/// `spawn_prefab` reads — so an unsaved prefab is already live for anything
/// spawning it. This is what lets the Inspector say so.
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
///
/// A resource rather than a field on the overlay: it is set by the action
/// layer and read by the renderer, and neither owns the other.
#[derive(Clone)]
pub(crate) struct PendingPrefabOverwrite {
    pub(crate) entity: Entity,
    pub(crate) dest: Option<std::path::PathBuf>,
    /// The file that would be replaced. Shown to the user, so they are
    /// answering about a name they recognise rather than about "a prefab".
    pub(crate) path: std::path::PathBuf,
}

/// Holds back any `SavePrefab` that would replace an existing file.
///
/// # Why overwriting rather than a numeric suffix
///
/// Suffixing never destroyed anything, which sounds safe and made the
/// common case impossible: saving a prefab again after editing the entity
/// is how a prefab is *iterated on*, and it produced `Enemy_1`, `Enemy_2`,
/// `Enemy_3` instead of an updated `Enemy`. Replacing is what the user
/// means; the prompt is what makes it safe.
///
/// Re-saving keeps the file's guid — see `kooch_ecs::scene::prefab::save` —
/// so every component already pointing at that prefab still does.
/// Borrows rather than clones: this runs on every batch of actions, and
/// `EditorAction` carries paths, names and reflected values that nothing
/// here needs a copy of.
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
    // Dual-sink: with a connected remote session the editor's ECS is a
    // mirror of a project that owns the real state, so ECS edits route
    // over the wire instead of mutating the mirror (which the next
    // refresh would overwrite). Actions remote mode does not own fall
    // through to the local path below. This is the one place the two
    // modes diverge.
    // A prefab saved while the previous batch was being handled. Drained
    // into actions here so propagation goes through the same dispatch as
    // everything else, one frame after the save rather than in the middle
    // of it.
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
        // 🔴 `debug`, not `info`. A live prefab drains every frame, so at
        // `info` this printed sixty identical lines a second and buried
        // every other message in the Console — including the ones a
        // measurement run is there to read.
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

    // Recorded before the edits are applied, while the instance still
    // holds the values the user is changing away from — and appended so
    // the write that persists the set travels the same path as the edit
    // that caused it.
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

    // A session that exists but has not answered yet: the project is
    // still building and its world has not arrived. Dropping the actions
    // that need it is what stops a Ctrl+S from writing the empty mirror
    // over the project's scene — see `needs_a_live_world`.
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

    // 🔴 A playing project owns its world and the editor does not get to
    // touch it. Refused here rather than greyed out in each panel: there
    // are a dozen ways to reach a world edit — a chord, a context menu, a
    // dragged handle, a typed field — and disabling them one at a time is
    // how one of them stays live. See `is_a_world_edit`.
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
