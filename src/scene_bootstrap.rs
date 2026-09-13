//! [`SceneBootstrapPlugin`]: [`SceneBootstrapPlugin::with_scene`], `--scene`, the manifest's
//! `main_scene` (#808), [`DEFAULT_SCENE_REL_PATH`] beside the executable, then the cwd. 🔴
//! Executable first: a double-clicked game's cwd is home.

use std::path::PathBuf;

use kooch_core::app::App;
use kooch_core::plugin::Plugin;
use kooch_core::resource::Resources;
use kooch_core::stage::Stage;
use kooch_ecs::SceneManager;

use kooch_core::scene_paths::{
    DEFAULT_SCENE_REL_PATH, PROJECT_MANIFEST_FILE, main_scene_of, normalise_main_scene,
};

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
        // `Stage::First` runs after every `Stage::Startup` registration, and consumes `BootScene`,
        // so it is one-shot.
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

/// Beside the executable, then the cwd; returns the cwd candidate when neither exists so the error
/// names the ordinary place.
pub(crate) fn default_scene_path() -> PathBuf {
    // The manifest's scene before the convention, which is wrong for any scene not called
    // `default.scene` (#808).
    if let Some(named) = manifest_scene() {
        return named;
    }
    let cwd = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(DEFAULT_SCENE_REL_PATH);
    // 🔴 A packaged scene is inside the pack, so `.exists()` rejected it and fell to the cwd. A pack
    // beside the executable decides the layout.
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

/// The manifest's scene (#808), beside the executable then the cwd. 🔴 Read from disk, never the
/// pack: it precedes the asset system, and a game that cannot open its pack still finds its scene.
fn manifest_scene() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(PathBuf::from));
    let cwd = std::env::current_dir().ok();

    for base in [exe_dir, cwd].into_iter().flatten() {
        match scene_named_by(&base) {
            Named::Scene(path) => {
                tracing::info!(
                    "SceneBootstrapPlugin: {PROJECT_MANIFEST_FILE} names {}",
                    path.display()
                );
                return Some(path);
            }
            // The manifest is there and names nothing. That is an answer:
            // stop looking and let the convention decide, rather than
            // reading a second project's manifest out of the cwd.
            Named::Nothing => return None,
            Named::NoManifest => continue,
        }
    }
    None
}

/// "No manifest" and "names nothing" are handled differently, hence an enum, not an `Option`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Named {
    Scene(PathBuf),
    Nothing,
    NoManifest,
}

/// The scene `base`'s manifest names — split from [`manifest_scene`], which reads process globals
/// tests cannot vary.
pub(crate) fn scene_named_by(base: &std::path::Path) -> Named {
    let Ok(text) = std::fs::read_to_string(base.join(PROJECT_MANIFEST_FILE)) else {
        return Named::NoManifest;
    };
    match main_scene_of(&text) {
        Some(named) => Named::Scene(base.join(normalise_main_scene(&named))),
        None => Named::Nothing,
    }
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
            // Refresh prefab copies left stale while the project was closed — the longest a scene
            // is ever closed.
            kooch_ecs::scene::propagate::refresh_all(resources);
        }
        // 🔴 A missing file is named as a packaging problem; the generic error reads like a corrupt
        // scene.
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
