//! Editor actions collected during UI, applied after render.

mod asset_ops;
mod codegen;
mod dispatch;
mod handlers;
mod remote_edit;
pub(crate) mod scene_io;

use std::any::TypeId;
use std::path::PathBuf;

use ome_core::power::PowerProfile;
use ome_core::resource::Resources;
use ome_ecs::component::ComponentId;
use ome_ecs::entity::Entity;
use ome_ecs::reflect::ReflectValue;
use ome_ecs::transform::Transform;

use crate::undo::{CompoundCommand, EditorCommand, UndoStack};

use self::dispatch::{action_to_command, batch_description, same_ecs_variant};
use self::handlers::apply_non_ecs_action;

pub(crate) use self::codegen::{migrate_to_library, register_scripts};

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
    /// Open a scene beside the ones already loaded, rather than replacing
    /// them. The scene becomes the active one, so newly spawned entities
    /// land in it.
    OpenSceneAdditive,
    /// Close one open scene, despawning only its entities.
    CloseScene(ome_core::Guid),
    /// Make an already-open scene the one new entities are authored into.
    SetActiveScene(ome_core::Guid),
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
    Reparent {
        entity: Entity,
        new_parent: Option<Entity>,
    },
    RemoveRecent(PathBuf),
    LaunchProject(PathBuf),
    CancelLaunch,
    SetPowerProfile(PowerProfile),
    /// Replace a `Material` asset's contents (PBR scalars + texture
    /// references) and persist it to its `.ron` on disk. Emitted by the
    /// Asset Browser's material editor. Applied to `Assets<Material>` so
    /// the render sync picks it up live. Not undoable — an asset-level
    /// edit, distinct from the ECS field undo stack.
    EditMaterial {
        guid: ome_core::Guid,
        material: ome_render::material::Material,
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
    /// Open `file` in an external IDE with `root` as the workspace, so
    /// the whole project (Rust source, `Cargo.toml`, …) is editable.
    OpenInIde {
        root: PathBuf,
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
    RegisterScripts,
}

/// The kind of file created by [`EditorAction::CreateFile`]. The Rust
/// and Rhai kinds are scaffolded from `templates/` in the engine root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NewFileKind {
    RustComponent,
    RustSystem,
    Scene,
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
            | Self::OpenScene
            | Self::OpenSceneAdditive
            | Self::CloseScene(_)
            | Self::SetActiveScene(_)
            | Self::Play
            | Self::Stop
            | Self::RegisterScripts => true,

            // Session and project lifecycle: these are how a user gets
            // *out* of a stuck build, so they must keep working.
            Self::OpenProject(_)
            | Self::RebuildRemote
            | Self::CreateProject { .. }
            | Self::CloseProject
            | Self::LaunchProject(_)
            | Self::CancelLaunch
            | Self::RemoveRecent(_) => false,

            // Editor preferences and things that act on files rather than
            // on the world. An asset edit is about a `.ron` on disk, and
            // the project is not holding it.
            Self::SetPowerProfile(_)
            | Self::SetIdeCommand { .. }
            | Self::EditMaterial { .. }
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
        for action in actions.iter().filter(|a| !a.needs_a_live_world()) {
            apply_non_ecs_action(action, resources, undo_stack);
        }
        return;
    }

    if remote {
        for action in actions {
            if !remote_edit::dispatch(resources, action) {
                apply_non_ecs_action(action, resources, undo_stack);
            }
        }
        return;
    }

    let mut i = 0;
    while i < actions.len() {
        let action = &actions[i];

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
                if let Some(cmd) = action_to_command(&run[0], resources) {
                    undo_stack.execute(cmd, resources);
                }
            } else {
                // Multiple same-type actions — batch into a CompoundCommand.
                let desc = batch_description(run);
                let mut cmds: Vec<Box<dyn EditorCommand>> = Vec::with_capacity(run.len());
                for a in run {
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
mod tests {
    use super::*;

    /// The one that motivated the guard: a Save during a build would
    /// write the empty mirror over the project's real scene. Not a panic
    /// — a deleted scene.
    #[test]
    fn saving_needs_a_live_world() {
        assert!(EditorAction::SaveScene.needs_a_live_world());
    }

    /// Every escape hatch has to keep working, or a build that never
    /// finishes leaves the editor with no way out.
    #[test]
    fn the_ways_out_of_a_stuck_build_are_not_blocked() {
        for (name, action) in [
            ("CancelLaunch", EditorAction::CancelLaunch),
            ("RebuildRemote", EditorAction::RebuildRemote),
            ("CloseProject", EditorAction::CloseProject),
        ] {
            assert!(
                !action.needs_a_live_world(),
                "{name} is how a user recovers; blocking it traps them",
            );
        }
    }

    /// Play asks the *project* to start simulating. Sent before it can
    /// answer, it is a message into a socket nobody is reading yet.
    #[test]
    fn play_and_stop_wait_for_the_project() {
        assert!(EditorAction::Play.needs_a_live_world());
        assert!(EditorAction::Stop.needs_a_live_world());
    }

    /// An asset edit is about a file on disk, and the project is not
    /// holding it — refusing these would block work that is perfectly
    /// safe during a build.
    #[test]
    fn preferences_and_file_work_stay_available() {
        assert!(!EditorAction::SetPowerProfile(PowerProfile::Battery).needs_a_live_world());
    }
}
