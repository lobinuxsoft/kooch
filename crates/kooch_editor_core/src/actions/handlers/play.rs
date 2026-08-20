//! Play and Stop.

use kooch_core::resource::Resources;

use crate::play_state::PlayState;
use crate::project_state::ProjectState;

pub(super) fn handle_play(resources: &mut Resources) {
    let (manifest_path, engine_root, launch_env) = match resources.get::<ProjectState>() {
        Some(ps) => {
            let root = ps.active_project.as_ref().map(|p| p.root_path.clone());
            let env = root
                .as_deref()
                .map(|root| {
                    crate::play_state::parse_launch_env(ps.editor_config.launch_env_for(root))
                })
                .unwrap_or_default();
            (
                root.map(|root| root.join("Cargo.toml")),
                ps.engine_root.clone(),
                env,
            )
        }
        None => (None, None, Vec::new()),
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
    let doc = kooch_ecs::SceneDocument::from_ecs(resources);
    let scene_path = std::env::temp_dir().join("kooch_play_scene.scene");
    if let Err(e) = doc.save(&scene_path) {
        tracing::error!("failed to save play scene: {e}");
    } else if let Some(play_state) = resources.get_mut::<PlayState>()
        && let Err(e) = play_state.launch(
            &manifest_path,
            &scene_path,
            engine_root.as_deref(),
            &launch_env,
        )
    {
        tracing::error!("failed to launch game: {e}");
    }
}

pub(super) fn handle_stop(resources: &mut Resources) {
    if let Some(play_state) = resources.get_mut::<PlayState>() {
        play_state.stop();
    }
}
