//! Play and Stop.

use ome_core::resource::Resources;

use crate::play_state::PlayState;
use crate::project_state::ProjectState;

pub(super) fn handle_play(resources: &mut Resources) {
    let (manifest_path, engine_root) = match resources.get::<ProjectState>() {
        Some(ps) => (
            ps.active_project
                .as_ref()
                .map(|p| p.root_path.join("Cargo.toml")),
            ps.engine_root.clone(),
        ),
        None => (None, None),
    };
    let Some(manifest_path) = manifest_path else {
        tracing::error!("Play: no active project — open a project first");
        return;
    };
    if !manifest_path.exists() {
        tracing::error!(
            "Play: project has no Cargo.toml at {} — Play only works on crate-projects",
            manifest_path.display()
        );
        return;
    }
    let doc = ome_ecs::SceneDocument::from_ecs(resources);
    let scene_path = std::env::temp_dir().join("ome_play_scene.scene");
    if let Err(e) = doc.save(&scene_path) {
        tracing::error!("failed to save play scene: {e}");
    } else if let Some(play_state) = resources.get_mut::<PlayState>()
        && let Err(e) = play_state.launch(&manifest_path, &scene_path, engine_root.as_deref())
    {
        tracing::error!("failed to launch game: {e}");
    }
}

pub(super) fn handle_stop(resources: &mut Resources) {
    if let Some(play_state) = resources.get_mut::<PlayState>() {
        play_state.stop();
    }
}
