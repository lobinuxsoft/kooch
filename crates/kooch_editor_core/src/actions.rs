//! Editor actions collected during UI, applied after render.

mod asset_ops;
mod codegen;
mod dispatch;
mod handlers;
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
pub(crate) mod scene_io;

use std::any::TypeId;
use std::path::PathBuf;

use kooch_core::power::PowerProfile;
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

pub(crate) use self::codegen::{
    initial_registrations, migrate_to_library, register_scripts, split_authoring,
};

pub(crate) enum EditorAction {
    /// Spawn an entity with Name + Transform + optional extra components.
    /// The optional String sets the Name component value.
    Spawn {
        extra: Vec<TypeId>,
        name: Option<String>,
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
    Undo,
    Redo,
    SaveScene,
    OpenScene,
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
    OpenSceneAdditive,
    /// Close one open scene, despawning only its entities.
    CloseScene(kooch_core::Guid),
    /// Make an already-open scene the one new entities are authored into.
    SetActiveScene(kooch_core::Guid),
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
    RebuildRemote,
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
    SetPowerProfile(PowerProfile),
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
            // Everything that reads or writes the world, or persists it.
            Self::Spawn { .. }
            | Self::SpawnMesh { .. }
            | Self::Despawn(_)
            | Self::Duplicate(_)
            | Self::SetField { .. }
            | Self::AddComponent { .. }
            | Self::RemoveComponent { .. }
            | Self::TransformEdit { .. }
            | Self::Reparent { .. }
            | Self::Undo
            | Self::Redo
            | Self::SaveScene
            | Self::SavePrefab { .. }
            | Self::InstantiatePrefab { .. }
            | Self::OpenScene
            | Self::OpenSceneAdditive
            | Self::CloseScene(_)
            | Self::SetActiveScene(_)
            | Self::Play
            | Self::Stop
            | Self::RegisterScripts => true,

            // Session and project lifecycle: these are how a user gets
            // *out* of a stuck build, so they must keep working.
            //
            // Building belongs here rather than above: it reads the
            // project from disk and never touches the ECS, so it works
            // while a project is still compiling and its world is empty.
            Self::BuildProject(_)
            | Self::CancelBuild
            | Self::OpenProject(_)
            | Self::RebuildRemote
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

            // Editor preferences and things that act on files rather than
            // on the world. An asset edit is about a `.ron` on disk, and
            // the project is not holding it.
            Self::SetPowerProfile(_)
            | Self::SetIdeCommand { .. }
            | Self::EditMaterial { .. }
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
    if !queued.is_empty() {
        tracing::info!(
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

    if remote {
        for action in actions.iter().copied() {
            if !remote_edit::dispatch(resources, action) {
                apply_non_ecs_action(action, resources, undo_stack);
            }
        }
        return;
    }

    let mut i = 0;
    while i < actions.len() {
        let action = actions[i];

        // Undo/Redo are handled directly.
        if matches!(action, EditorAction::Undo) {
            undo_stack.undo(resources);
            i += 1;
            continue;
        }
        if matches!(action, EditorAction::Redo) {
            undo_stack.redo(resources);
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
