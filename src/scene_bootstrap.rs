//! [`SceneBootstrapPlugin`] — loads the initial scene at startup.
//!
//! Resolution order:
//! 1. Explicit override via [`SceneBootstrapPlugin::with_scene`].
//! 2. `--scene <path>` CLI argument (used by the editor when launching
//!    the play binary).
//! 3. [`DEFAULT_SCENE_REL_PATH`] **beside the executable**.
//! 4. The same, relative to the current working directory.
//!
//! # Why the executable comes first
//!
//! 🔴 A shipped game is opened by double-clicking it, and that leaves the
//! working directory wherever the desktop felt like — the user's home,
//! or `/`. Resolved against the cwd alone, a released game starts with
//! **an empty scene and no error**: the file it looked for is not missing
//! from the package, it was never looked for in the package.
//!
//! Beside the executable is where a packaged game keeps its content, so
//! that is asked first. The cwd stays as the fallback because it is what
//! a plain `cargo run` inside a project relies on — there the executable
//! lives in `target/debug/` and has no `scenes/` beside it.
//!
//! The cwd convention does **not** hold for `cargo run --manifest-path …`,
//! which leaves the child's working directory at the caller's — a
//! launcher using that form must set the child's working directory itself
//! (see `RemoteSession::launch`) or pass `--scene` with an absolute path
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

/// Where a game looks for its scene when nobody said.
///
/// Beside the executable first, then the working directory. Returns the
/// cwd candidate when neither exists, so the error names the ordinary
/// place rather than a path inside an install directory.
pub(crate) fn default_scene_path() -> PathBuf {
    let cwd = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(DEFAULT_SCENE_REL_PATH);
    // 🔴 `.exists()` asks the **disk**, and a packaged game's scene is
    // inside the pack — so the check that was meant to pick the shipped
    // layout rejected it and fell back to the working directory, which
    // for a double-clicked game is the user's home.
    //
    // The same mistake as the asset root in `default_asset_plugin`, made
    // twice: a shipped game's files are not on disk, so "does it exist"
    // is the wrong question. A pack beside the executable is what says
    // this is a packaged game, and then the layout is the package's.
    let packaged = crate::shipped::shipped_pack().is_some();
    beside_exe()
        .filter(|p| packaged || p.exists())
        .unwrap_or(cwd)
}

/// [`DEFAULT_SCENE_REL_PATH`] beside the running executable.
pub(crate) fn beside_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join(DEFAULT_SCENE_REL_PATH))
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
        // 🔴 Named as a packaging problem when the file is simply not
        // there, because that is what it is and the generic error reads
        // like a corrupt scene. A released game whose content was not
        // copied beside it starts empty, and this line is the only thing
        // standing between that and half an hour of looking at the
        // renderer.
        Err(err) if !path.exists() => tracing::error!(
            "SceneBootstrapPlugin: no scene at {} — a packaged game keeps its \
             `{DEFAULT_SCENE_REL_PATH}` and `assets/` beside the executable; \
             run it from its own folder or pass --scene ({err})",
            path.display(),
        ),
        Err(err) => tracing::error!(
            "SceneBootstrapPlugin: failed to load {}: {err}",
            path.display()
        ),
    }
    resources.insert(manager);
}
