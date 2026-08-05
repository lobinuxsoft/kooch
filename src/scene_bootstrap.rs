//! [`SceneBootstrapPlugin`] — loads the initial scene at startup.
//!
//! Resolution order:
//! 1. Explicit override via [`SceneBootstrapPlugin::with_scene`].
//! 2. `--scene <path>` CLI argument (used by the editor when launching
//!    the play binary).
//! 3. [`DEFAULT_SCENE_REL_PATH`] relative to the current working directory.
//!
//! The cwd convention holds for a plain `cargo run` inside the project.
//! It does **not** hold for `cargo run --manifest-path …`, which leaves
//! the child's working directory at the caller's — a launcher using that
//! form must set the child's working directory itself (see
//! `RemoteSession::launch`) or pass `--scene` with an absolute path
//! (see `PlayState::launch`).

use std::path::PathBuf;

use kooch_core::app::App;
use kooch_core::plugin::Plugin;
use kooch_core::resource::Resources;
use kooch_core::stage::Stage;
use kooch_ecs::SceneManager;

use kooch_core::scene_paths::DEFAULT_SCENE_REL_PATH;

/// Resource holding the path queued for the startup loader.
struct BootScene(PathBuf);

/// Plugin that loads the boot scene into the live ECS at `Stage::Startup`.
#[derive(Default)]
pub struct SceneBootstrapPlugin {
    scene_override: Option<PathBuf>,
}

impl SceneBootstrapPlugin {
    /// Forces the plugin to load the given path, ignoring CLI args and
    /// the default convention.
    pub fn with_scene(path: impl Into<PathBuf>) -> Self {
        Self {
            scene_override: Some(path.into()),
        }
    }
}

impl Plugin for SceneBootstrapPlugin {
    fn build(&self, app: &mut App) {
        let path = self
            .scene_override
            .clone()
            .or_else(parse_scene_cli_arg)
            .unwrap_or_else(default_scene_path);
        app.insert_resource(BootScene(path));
        // `Stage::First` runs once-per-frame AFTER `Stage::Startup` completes,
        // so any user-defined `register_components` system at Stage::Startup
        // is guaranteed to have run before the scene is deserialized.
        // The `BootScene` resource is consumed on first invocation, so this
        // is effectively a one-shot.
        app.add_system(Stage::First, load_boot_scene);
    }

    fn name(&self) -> &str {
        "SceneBootstrapPlugin"
    }
}

fn parse_scene_cli_arg() -> Option<PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    let i = args.iter().position(|a| a == "--scene")?;
    args.get(i + 1).map(PathBuf::from)
}

fn default_scene_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(DEFAULT_SCENE_REL_PATH)
}

fn load_boot_scene(resources: &mut Resources) {
    let Some(boot) = resources.remove::<BootScene>() else {
        return;
    };
    let Some(mut manager) = resources.remove::<SceneManager>() else {
        tracing::error!(
            "SceneBootstrapPlugin: SceneManager missing — add EcsPlugin before SceneBootstrapPlugin"
        );
        return;
    };
    let path = if boot.0.is_absolute() {
        boot.0.clone()
    } else {
        std::env::current_dir().unwrap_or_default().join(&boot.0)
    };
    match manager.load(&path, resources) {
        Ok(()) => {
            tracing::info!("SceneBootstrapPlugin: loaded {}", path.display());
            // The scene holds its prefab instances in full, so a prefab
            // edited while it was closed left stale copies. This is the
            // case that motivates it: opening a project is the longest a
            // scene is ever closed for.
            kooch_ecs::scene::propagate::refresh_all(resources);
        }
        Err(err) => tracing::error!(
            "SceneBootstrapPlugin: failed to load {}: {err}",
            path.display()
        ),
    }
    resources.insert(manager);
}
