//! What each action is: whether it needs the world, what it edits, what undo calls it.

use super::*;

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
            | Self::SaveScene { .. }
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
            | Self::RenameLayer { .. }
            | Self::SetLayerPair { .. }
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
            | Self::SaveScene { .. }
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
            | Self::RenameLayer { .. }
            | Self::SetLayerPair { .. }
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
