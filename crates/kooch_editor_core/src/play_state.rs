//! Play/Stop mode — child game process management.
//!
//! [`PlayState`] manages the lifecycle of a separate game process that
//! the editor launches for testing. The game reads a serialized
//! `.scene` file; when stopped, the process is killed and the
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
/// Stored as a resource in [`Resources`](kooch_core::resource::Resources).
/// The editor inserts this automatically via [`EditorPlugin`](crate::EditorPlugin).
pub struct PlayState {
    child: Option<Child>,
    output: Arc<Mutex<Vec<String>>>,
}

/// Splits a launch line into environment pairs.
///
/// Whitespace-separated `KEY=VALUE`, split at the FIRST `=` so a value
/// may contain one — `RUST_LOG=kooch_render=debug` is a real thing
/// somebody types.
///
/// 🔴 A token that does not parse is dropped **with a warning**, never
/// in silence. Silently ignoring a misspelling looks exactly like the
/// feature not existing, and this one is typed in a text field minutes
/// before a measurement run.
///
/// No quoting, deliberately. A value with a space in it would need a
/// shell's rules, and the variables this exists for — every `KOOCH_*`
/// knob in the engine — are single words. Pretending to support quotes
/// and getting them subtly wrong is worse than not offering them.
pub fn parse_launch_env(raw: &str) -> Vec<(String, String)> {
    raw.split_whitespace()
        .filter_map(|token| match token.split_once('=') {
            Some((key, value)) if !key.is_empty() => Some((key.to_owned(), value.to_owned())),
            _ => {
                tracing::warn!(
                    token,
                    "launch environment: not a KEY=VALUE pair — ignored for this launch",
                );
                None
            }
        })
        .collect()
}

/// The environment a launched game gets, in the order it is applied.
///
/// Later entries overwrite earlier ones, which is what `Command::env`
/// does — so the ORDER of this list is the policy:
///
/// 1. **The author's launch line**, first, so everything below can beat
///    it.
/// 2. **`KOOCH_ENGINE_ROOT` and `KOOCH_PROJECT_ROOT`**, which the editor
///    knows and a text field does not.
/// 3. **`RUST_LOG`**, and only as a default: a launch line that names it
///    keeps what it asked for, and so does an editor started with one.
/// 4. 🔴 **`KOOCH_LOG_FORMAT=json`, unconditionally.** The Console
///    parses the game's output; handed anything else every line arrives
///    as one opaque string that has lost the level and target it filters
///    on. A launch option that could replace this would look exactly
///    like the Console breaking.
///
/// `inherited_logs` is whether this process already has `RUST_LOG`, read
/// by the caller so the rule itself stays testable.
fn game_env(
    launch_env: &[(String, String)],
    engine_root: Option<&Path>,
    project_root: Option<&Path>,
    inherited_logs: bool,
) -> Vec<(String, std::ffi::OsString)> {
    let mut env: Vec<(String, std::ffi::OsString)> = launch_env
        .iter()
        .map(|(key, value)| (key.clone(), value.into()))
        .collect();
    if let Some(engine_root) = engine_root {
        env.push(("KOOCH_ENGINE_ROOT".to_owned(), engine_root.into()));
    }
    if let Some(project_root) = project_root {
        env.push(("KOOCH_PROJECT_ROOT".to_owned(), project_root.into()));
    }
    let asked_for_logs = launch_env.iter().any(|(key, _)| key == "RUST_LOG");
    if !asked_for_logs && !inherited_logs {
        env.push(("RUST_LOG".to_owned(), "info".into()));
    }
    env.push(("KOOCH_LOG_FORMAT".to_owned(), "json".into()));
    env
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
    /// through `kooch::SceneBootstrapPlugin`.
    ///
    /// `engine_root` is forwarded as `KOOCH_ENGINE_ROOT` so the spawned
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
        launch_env: &[(String, String)],
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
            // No mode flag any more: the project's default binary IS the
            // game (#558). Play therefore runs the same artefact a player
            // would, which is the point — it used to run a build that also
            // contained the editor and merely declined to open it.
            .arg("--scene")
            .arg(scene_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in game_env(
            launch_env,
            engine_root,
            manifest_path.parent(),
            std::env::var_os("RUST_LOG").is_some(),
        ) {
            cmd.env(key, value);
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

#[cfg(test)]
mod tests;
