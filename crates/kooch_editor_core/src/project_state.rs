//! Project state resource for the editor.
//!
//! Tracks the currently active project, transient UI state
//! for the launch screen (new project form), and the launcher
//! child process (compilation + execution of project binaries).

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use crate::project::{EditorConfig, ProjectManifest, sanitize_crate_name};

// ---------------------------------------------------------------------------
// Active project
// ---------------------------------------------------------------------------

/// Information about the currently loaded project.
#[derive(Debug, Clone)]
pub struct ActiveProject {
    pub manifest: ProjectManifest,
    pub root_path: PathBuf,
}

// ---------------------------------------------------------------------------
// New project form (transient UI state)
// ---------------------------------------------------------------------------

/// Transient UI state for the "New Project" inline form.
#[derive(Debug, Clone, Default)]
pub struct NewProjectForm {
    pub name: String,
    pub parent_path: String,
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Launcher process (compile + run project binary)
// ---------------------------------------------------------------------------

/// Status of the launcher child process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LauncherStatus {
    Compiling,
    /// The binary was launched as an independent process — the launcher can exit.
    Launched,
    Failed(String),
    Exited,
}

/// Manages the two-phase launcher: compile with `cargo build`, then
/// launch the resulting binary directly.
pub struct LauncherProcess {
    /// The currently active child process (cargo build or the binary).
    child: Child,
    output: Arc<Mutex<Vec<String>>>,
    pub status: LauncherStatus,
    /// Path to the project binary (computed from project root + crate name).
    binary_path: PathBuf,
    /// Root of the project directory (needed to launch the binary).
    project_root: PathBuf,
    /// Root of the engine repository / install. Forwarded to the
    /// game binary as `KOOCH_ENGINE_ROOT` so its asset plugin can
    /// locate engine-shipped assets even when the binary's CWD
    /// points at the project root.
    engine_root: Option<PathBuf>,
}

/// Spawns reader threads that drain stdout/stderr into the shared buffer.
fn spawn_output_readers(child: &mut Child, output: &Arc<Mutex<Vec<String>>>) {
    if let Some(stdout) = child.stdout.take() {
        let out = Arc::clone(output);
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
        let out = Arc::clone(output);
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                if let Ok(mut buf) = out.lock() {
                    buf.push(line);
                }
            }
        });
    }
}

impl LauncherProcess {
    /// Starts phase 1: `cargo build --manifest-path <path>`.
    ///
    /// `engine_root` is forwarded to the spawned game binary as the
    /// `KOOCH_ENGINE_ROOT` env var so its `DefaultPlugins` can locate
    /// the engine-shipped assets (Suzanne, default sky textures, the
    /// committed sample materials). Without it the binary would only
    /// see `<project>/assets/`, miss every engine asset the scene
    /// references, and render nothing.
    pub fn spawn(project_root: &Path, engine_root: Option<&Path>) -> Result<Self, String> {
        let manifest_path = project_root.join("Cargo.toml");
        let output = Arc::new(Mutex::new(Vec::new()));

        // Determine the binary name from project.kooch or directory name.
        let crate_name = project_root
            .file_name()
            .map(|n| sanitize_crate_name(&n.to_string_lossy()))
            .unwrap_or_else(|| "project".to_owned());

        let binary_name = if cfg!(windows) {
            format!("{crate_name}.exe")
        } else {
            crate_name
        };

        let binary_path = project_root.join("target").join("debug").join(binary_name);

        let mut cmd = Command::new("cargo");
        cmd.args(["build", "--manifest-path"]).arg(&manifest_path);
        // Authoring, not a game build: this produces the `dylib` the
        // editor loads to list the project's components, and that is
        // compiled out of a game (#558).
        crate::cargo_args::authoring(&mut cmd);
        let mut child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to spawn cargo build: {e}"))?;

        spawn_output_readers(&mut child, &output);

        Ok(Self {
            child,
            output,
            status: LauncherStatus::Compiling,
            binary_path,
            project_root: project_root.to_owned(),
            engine_root: engine_root.map(|p| p.to_owned()),
        })
    }

    /// Polls the child process and handles phase transitions.
    pub fn poll(&mut self) {
        match &self.status {
            LauncherStatus::Failed(_) | LauncherStatus::Exited | LauncherStatus::Launched => return,
            LauncherStatus::Compiling => self.poll_compiling(),
        }
    }

    /// Phase 1: wait for `cargo build` to finish.
    fn poll_compiling(&mut self) {
        match self.child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    // Build succeeded — launch the binary as an independent process.
                    if let Err(e) = self.launch_binary() {
                        self.status = LauncherStatus::Failed(e);
                    } else {
                        self.status = LauncherStatus::Launched;
                    }
                } else {
                    self.status = LauncherStatus::Failed(format!("cargo build failed ({status})"));
                }
            }
            Ok(None) => {} // Still compiling.
            Err(e) => {
                self.status = LauncherStatus::Failed(format!("failed to poll cargo: {e}"));
            }
        }
    }

    /// Launches the compiled binary as a fully independent process.
    ///
    /// The binary runs detached — no piped IO, no child tracking. The
    /// launcher can safely exit without affecting it.
    fn launch_binary(&mut self) -> Result<(), String> {
        if !self.binary_path.exists() {
            return Err(format!("binary not found: {}", self.binary_path.display()));
        }

        if let Ok(mut buf) = self.output.lock() {
            buf.push(format!("--- Launching {} ---", self.binary_path.display()));
        }

        // Spawn the binary with stdout/stderr inherited so engine
        // logs (asset scan summaries, render-stage warnings, …)
        // surface in whatever terminal the editor itself is running
        // in. Trade-off: closing the editor before the binary exits
        // can leave the binary writing to a vanished pipe; we accept
        // that until the launcher routes the output through its own
        // captured-output channel like the cargo-build phase does.
        let mut cmd = Command::new(&self.binary_path);
        cmd.current_dir(&self.project_root)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        if let Some(engine_root) = &self.engine_root {
            cmd.env("KOOCH_ENGINE_ROOT", engine_root);
        }
        cmd.env("KOOCH_PROJECT_ROOT", &self.project_root);
        // Default to info-level logs unless the user already set
        // RUST_LOG; gives us asset_scan, render warnings, and the
        // editor camera trace without forcing the user to know the
        // tracing-subscriber env var.
        if std::env::var_os("RUST_LOG").is_none() {
            cmd.env("RUST_LOG", "info");
        }
        cmd.spawn()
            .map_err(|e| format!("failed to launch binary: {e}"))?;

        Ok(())
    }

    /// Drains captured output lines.
    pub fn drain_output(&self) -> Vec<String> {
        self.output
            .lock()
            .map(|mut buf| std::mem::take(&mut *buf))
            .unwrap_or_default()
    }

    /// Kills the child process.
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for LauncherProcess {
    fn drop(&mut self) {
        // Only kill the cargo build process if still compiling.
        // Once the binary is launched, it's independent.
        if self.status == LauncherStatus::Compiling {
            self.kill();
        }
    }
}

// ---------------------------------------------------------------------------
// ProjectState resource
// ---------------------------------------------------------------------------

/// Editor resource that tracks the active project and launch screen state.
pub struct ProjectState {
    pub active_project: Option<ActiveProject>,
    pub editor_config: EditorConfig,
    pub new_project_form: NewProjectForm,
    /// Whether the "New Project" form is currently visible.
    pub show_new_project_form: bool,
    /// Path to the engine repository root (set by the launcher binary).
    pub engine_root: Option<PathBuf>,
    /// Active launcher process (compiling/running a project binary).
    pub launcher_process: Option<LauncherProcess>,
    /// Accumulated output lines from the launcher process for display.
    pub launcher_output: Vec<String>,
    /// How the engine on disk compares to the one this editor ships,
    /// as of the last time a project was opened.
    ///
    /// `Some` with a difference that
    /// [`wants_a_decision`](crate::engine_vendor::Difference::wants_a_decision)
    /// is what puts the notice on screen. Cleared when it is answered —
    /// keeping it would draw the notice forever.
    pub engine_status: Option<crate::engine_vendor::EngineStatus>,
}

impl ProjectState {
    /// Creates a new `ProjectState`, loading the persisted editor config.
    pub fn new() -> Self {
        Self {
            active_project: None,
            editor_config: EditorConfig::load(),
            new_project_form: NewProjectForm::default(),
            show_new_project_form: false,
            engine_root: None,
            launcher_process: None,
            launcher_output: Vec::new(),
            engine_status: None,
        }
    }

    /// Opens a project from the given root directory.
    pub fn open_project(&mut self, root_path: &Path) -> Result<(), crate::project::ProjectError> {
        let mut manifest = ProjectManifest::load(root_path)?;
        self.editor_config.add_recent(&manifest.name, root_path);
        if let Err(e) = self.editor_config.save() {
            tracing::warn!("failed to save editor config: {e}");
        }

        // Self-heal default scene: keep `scenes/default.scene` on disk
        // and point `main_scene` at it whenever the manifest has no entry.
        // Existing main_scene values are respected even if the file is
        // missing — that's a load error, not a manifest problem.
        if let Err(e) = crate::project::ensure_default_scene(root_path) {
            tracing::warn!("failed to ensure default scene: {e}");
        }
        // 🔴 The engine lives ONCE on the machine, in
        // ~/.local/share/kooch/<version>/engine, and every project's
        // manifest points at it (#754). This is the first moment anything
        // knows which version the project wants.
        //
        // 🔴 Looking is not the same as replacing, and this used to do
        // both. Installing this editor's engine over the one a project
        // was building against makes the next build a full one, minutes
        // long, and leaves the project's compiled `.so` behind an engine
        // it was not compiled against — announced in a log line nobody
        // reads. Now the difference is reported and the answer is the
        // user's.
        //
        // ⚠️ A missing engine is still installed without asking: there is
        // nothing to keep, and a project that cannot build at all is not
        // a choice worth offering.
        let source = crate::engine_vendor::vendor_source(self.engine_root.as_deref());
        let status = crate::engine_vendor::status(&manifest.engine_version, source.as_deref());
        if status.difference.wants_a_decision() {
            tracing::info!(
                summary = %status.headline(),
                "the engine this editor ships is not the one this project uses",
            );
            if let Some(dir) = status.installed.clone()
                && let Err(e) = crate::project::point_manifest_at_engine(root_path, &dir)
            {
                tracing::warn!("could not point the project at the engine: {e}");
            }
            self.engine_status = Some(status);
            return self.finish_open(root_path, manifest);
        }
        self.engine_status = Some(status);

        match crate::engine_vendor::ensure_current(&manifest.engine_version, source.as_deref()) {
            Ok((state, Some(engine_dir))) => {
                use crate::engine_vendor::VendorState;
                match state {
                    VendorState::UpToDate => {}
                    // 🔴 Said plainly because the next build is a full
                    // one: every engine source file is newer than the
                    // project's `target/`, so cargo rebuilds all of it.
                    // Minutes of silence with no explanation is how this
                    // reads otherwise (#761).
                    VendorState::Replaced => tracing::info!(
                        path = %engine_dir.display(),
                        "the engine was replaced with the one this editor ships — \
                         the next build of this project is a full rebuild",
                    ),
                    _ => tracing::info!(
                        ?state,
                        path = %engine_dir.display(),
                        "engine materialised",
                    ),
                }
                // The manifest carries an absolute path and `$HOME`
                // differs per user, so a project moved between machines
                // points somewhere that does not exist. The editor owns
                // that line — it owns the directory it names — so it
                // rewrites it rather than letting cargo fail on it.
                if let Err(e) = crate::project::point_manifest_at_engine(root_path, &engine_dir) {
                    tracing::warn!("could not point the project at the engine: {e}");
                }
            }
            // Never fails an open: a project already pointing at a good
            // engine still builds, and one that is not says so at build
            // time with cargo's own error.
            Ok((_, None)) => tracing::warn!(
                "no engine source available to materialise; the project keeps whatever \
                 its manifest points at",
            ),
            Err(e) => tracing::warn!("could not materialise the engine: {e}"),
        }

        self.finish_open(root_path, manifest)
    }

    /// The rest of opening a project, once the engine question is settled
    /// one way or the other.
    ///
    /// Its own method because that question has two answers — install, or
    /// leave it to the user — and both of them open the project.
    fn finish_open(
        &mut self,
        root_path: &Path,
        mut manifest: ProjectManifest,
    ) -> Result<(), crate::project::ProjectError> {
        if manifest.main_scene.is_none() {
            manifest.main_scene = Some(crate::project::DEFAULT_SCENE_REL_PATH.to_owned());
            if let Err(e) = manifest.save(root_path) {
                tracing::warn!("failed to persist updated manifest: {e}");
            }
        }

        self.active_project = Some(ActiveProject {
            manifest,
            root_path: root_path.to_owned(),
        });
        Ok(())
    }

    /// Installs the engine this editor ships, answering the notice.
    ///
    /// ⚠️ The next build of the project is a **full** one: every engine
    /// source file is newer than the project's `target/`, so cargo
    /// rebuilds all of it. Minutes of silence with no explanation is how
    /// that reads otherwise (#761).
    pub fn update_engine(&mut self) {
        // 🔴 Not while cargo is reading it. Installing renames the whole
        // directory out from under a compile in progress, and what comes
        // back is an error about a missing file in a crate nobody
        // touched. The notice stays up; pressing it again after the
        // build works.
        if self.launcher_process.is_some() {
            tracing::warn!("a build is running — the engine is not replaced while cargo reads it");
            return;
        }

        // 🔴 The version comes from the notice, not from the open
        // project. The notice outlives closing a project — it is about
        // the machine — and reading the version off `active_project`
        // made Install a button that returned in silence from the
        // project manager, which is exactly where it is easiest to
        // press.
        //
        // 🔴🔴 And it is the version this editor **ships**, not the one
        // the project asks for — see `EngineStatus::version_to_install`.
        // Asking for the project's version returns the engine already on
        // the machine under that name, which is the right answer for
        // opening a project and the wrong one for a button that says
        // *"Installing moves the project onto it."*
        let Some(version) = self
            .engine_status
            .as_ref()
            .map(|s| s.version_to_install().to_owned())
        else {
            return;
        };
        let root_path = self.active_project.as_ref().map(|p| p.root_path.clone());

        let source = crate::engine_vendor::vendor_source(self.engine_root.as_deref());
        match crate::engine_vendor::ensure_current(&version, source.as_deref()) {
            Ok((state, Some(engine_dir))) => {
                tracing::info!(
                    ?state,
                    path = %engine_dir.display(),
                    "engine installed — the next build of a project on it is a full rebuild",
                );
                // Only when one is open. With none, the engine is still
                // installed and the next project to open is pointed at
                // it by the usual path.
                // 🔴 One call, because two files record which engine a
                // project uses and writing one without the other is
                // exactly what made this prompt return for ever (#801).
                // `move_project_to_engine` is the only place either is
                // written.
                if let Some(root) = root_path {
                    match crate::project::move_project_to_engine(&root, &engine_dir, &version) {
                        Ok(()) => {
                            if let Some(project) = self.active_project.as_mut() {
                                project.manifest.engine_version = version.clone();
                            }
                        }
                        Err(e) => {
                            tracing::warn!("could not point the project at the engine: {e}")
                        }
                    }
                }
                self.engine_status =
                    Some(crate::engine_vendor::status(&version, source.as_deref()));
            }
            Ok((_, None)) => tracing::warn!("no engine source available to install"),
            Err(e) => tracing::warn!("could not install the engine: {e}"),
        }
    }

    /// Dismisses the engine notice, leaving the installed engine alone.
    ///
    /// 🔴 What this cannot promise: engines are named by version and
    /// replaced in place, so keeping one here holds only until something
    /// else installs over it — updating from another project, for
    /// instance. Two engines with the same version have nowhere separate
    /// to live.
    pub fn keep_engine(&mut self) {
        self.engine_status = None;
    }

    /// Closes the current project, returning to the launch screen.
    pub fn close_project(&mut self) {
        self.active_project = None;
    }

    /// Returns `true` if a project is currently loaded.
    pub fn is_project_loaded(&self) -> bool {
        self.active_project.is_some()
    }

    /// Spawns the launcher process for a project.
    pub fn spawn_launcher(&mut self, project_root: &Path) {
        self.launcher_output.clear();
        match LauncherProcess::spawn(project_root, self.engine_root.as_deref()) {
            Ok(proc) => {
                tracing::info!("launcher: building {}", project_root.display());
                self.launcher_process = Some(proc);
            }
            Err(e) => {
                tracing::error!("launcher: {e}");
                self.launcher_output.push(format!("ERROR: {e}"));
            }
        }
    }

    /// Polls the launcher process and drains its output.
    pub fn poll_launcher(&mut self) {
        let Some(proc) = self.launcher_process.as_mut() else {
            return;
        };
        proc.poll();
        let lines = proc.drain_output();
        self.launcher_output.extend(lines);
    }

    /// Returns the current launcher status, if any.
    pub fn launcher_status(&self) -> Option<&LauncherStatus> {
        self.launcher_process.as_ref().map(|p| &p.status)
    }

    /// Kills the launcher process and clears it.
    pub fn kill_launcher(&mut self) {
        if let Some(mut proc) = self.launcher_process.take() {
            proc.kill();
        }
        self.launcher_output.clear();
    }
}
