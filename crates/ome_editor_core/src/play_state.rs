//! Play/Stop mode — child game process management.
//!
//! [`PlayState`] manages the lifecycle of a separate game process that
//! the editor launches for testing. The game reads a serialized
//! `.ome_scene` file; when stopped, the process is killed and the
//! editor state remains untouched.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// PlayState
// ---------------------------------------------------------------------------

/// Manages the child game process lifecycle.
///
/// Stored as a resource in [`Resources`](ome_core::resource::Resources).
/// The editor inserts this automatically via [`EditorPlugin`](crate::EditorPlugin).
pub struct PlayState {
    child: Option<Child>,
    output: Arc<Mutex<Vec<String>>>,
}

impl PlayState {
    /// Creates a new idle `PlayState`.
    pub fn new() -> Self {
        Self {
            child: None,
            output: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Returns `true` if a game process is currently running.
    pub fn is_playing(&self) -> bool {
        self.child.is_some()
    }

    /// Launches the game process via `cargo run --manifest-path <project>`.
    ///
    /// Cargo handles the build (incremental, cached) and runs the resulting
    /// binary with `--scene <abs-path>`. The play binary picks up the scene
    /// through `oh_my_engine::SceneBootstrapPlugin`.
    ///
    /// `engine_root` is forwarded as `OME_ENGINE_ROOT` so the spawned
    /// binary's `DefaultPlugins::AssetPlugin` can resolve engine-shipped
    /// assets (Suzanne, sample materials) even though its CWD points at
    /// the project. Without it the asset database scans `<project>/assets`
    /// only, every engine GUID fails `load_by_guid`, and the game window
    /// renders a clear sky with no meshes.
    pub fn launch(
        &mut self,
        manifest_path: &Path,
        scene_path: &Path,
        engine_root: Option<&Path>,
    ) -> Result<(), PlayError> {
        if self.is_playing() {
            return Err(PlayError::AlreadyPlaying);
        }

        if let Ok(mut out) = self.output.lock() {
            out.clear();
        }

        let mut cmd = Command::new("cargo");
        cmd.arg("run")
            .arg("--manifest-path")
            .arg(manifest_path)
            .arg("--")
            // `--game` selects the project's game build (no embedded
            // editor) so Play tests the actual game, not a nested editor.
            .arg("--game")
            .arg("--scene")
            .arg(scene_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(engine_root) = engine_root {
            cmd.env("OME_ENGINE_ROOT", engine_root);
        }
        if let Some(project_root) = manifest_path.parent() {
            cmd.env("OME_PROJECT_ROOT", project_root);
        }
        if std::env::var_os("RUST_LOG").is_none() {
            cmd.env("RUST_LOG", "info");
        }
        let mut child = cmd.spawn().map_err(|e| PlayError::Spawn(e.to_string()))?;

        // Spawn reader threads for stdout/stderr.
        if let Some(stdout) = child.stdout.take() {
            let out = Arc::clone(&self.output);
            std::thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines().map_while(Result::ok) {
                    if let Ok(mut buf) = out.lock() {
                        buf.push(line);
                    }
                }
            });
        }

        if let Some(stderr) = child.stderr.take() {
            let out = Arc::clone(&self.output);
            std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    if let Ok(mut buf) = out.lock() {
                        buf.push(line);
                    }
                }
            });
        }

        self.child = Some(child);
        tracing::info!("game process launched");
        Ok(())
    }

    /// Kills the game process if running.
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
            tracing::info!("game process stopped");
        }
    }

    /// Checks if the child process has exited.
    ///
    /// Returns `true` if the process exited since the last check,
    /// in which case `is_playing()` will now return `false`.
    pub fn poll(&mut self) -> bool {
        let Some(child) = self.child.as_mut() else {
            return false;
        };
        match child.try_wait() {
            Ok(Some(status)) => {
                tracing::info!("game process exited: {status}");
                self.child = None;
                true
            }
            Ok(None) => false,
            Err(e) => {
                tracing::warn!("failed to poll game process: {e}");
                self.child = None;
                true
            }
        }
    }

    /// Drains captured output lines from the game process.
    pub fn drain_output(&self) -> Vec<String> {
        self.output
            .lock()
            .map(|mut buf| std::mem::take(&mut *buf))
            .unwrap_or_default()
    }
}

impl Drop for PlayState {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors from play mode operations.
#[derive(Debug)]
pub enum PlayError {
    /// A game process is already running.
    AlreadyPlaying,
    /// Failed to spawn the child process.
    Spawn(String),
}

impl std::fmt::Display for PlayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyPlaying => write!(f, "game process is already running"),
            Self::Spawn(e) => write!(f, "failed to spawn game process: {e}"),
        }
    }
}

impl std::error::Error for PlayError {}
