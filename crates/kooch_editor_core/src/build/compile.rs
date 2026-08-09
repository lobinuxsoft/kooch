//! Running cargo for a build preset, and packaging what comes out (#758).
//!
//! A build takes minutes, so this is a child process polled each frame —
//! the same shape as `PlayState` and the project launcher, for the same
//! reason: the editor has to stay drawable while it happens.
//!
//! # 🔴 What is checked before cargo runs
//!
//! A missing cross-compilation target fails **ten minutes in**, with a
//! linker error that says nothing about targets. Both of the things that
//! can be known up front are checked up front, and the message names the
//! command that fixes it.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use kooch_pack::PackKey;

use super::{BuildPreset, Package, PackageError};

/// Where a build has got to.
#[derive(Debug, Clone)]
pub enum BuildStatus {
    /// cargo is running.
    Compiling,
    /// cargo finished; the folder is being laid out.
    Packaging,
    /// Everything worked.
    Done(Package),
    Failed(String),
    /// Stopped on purpose.
    ///
    /// Separate from `Failed` because it is not one: nothing went wrong,
    /// and a red error where someone pressed a button reads as a bug.
    Cancelled,
}

/// A build in progress.
pub struct BuildJob {
    child: Option<Child>,
    output: Arc<Mutex<Vec<String>>>,
    status: BuildStatus,
    preset: BuildPreset,
    project_root: PathBuf,
    engine_root: Option<PathBuf>,
    crate_name: String,
    key: PackKey,
}

impl BuildJob {
    /// Checks what can be checked, then starts cargo.
    pub fn start(
        preset: &BuildPreset,
        project_root: &Path,
        engine_root: Option<&Path>,
        crate_name: &str,
        key: PackKey,
    ) -> Result<Self, String> {
        if let Some(problem) = missing_toolchain(preset) {
            return Err(problem);
        }

        let mut command = cargo_command(preset, project_root, crate_name);
        if preset.pack_assets {
            // The shares the game reassembles its key from. Through the
            // environment, so nothing is written into the project — see
            // `kooch::shipped`.
            command.env(
                kooch_core::asset_loader::SHARES_ENV,
                kooch_core::asset_loader::shares_for_build(&key),
            );
        }
        command.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = command
            .spawn()
            .map_err(|e| format!("could not start cargo: {e}"))?;
        let output = Arc::new(Mutex::new(Vec::new()));
        capture(child.stdout.take(), &output);
        capture(child.stderr.take(), &output);

        Ok(Self {
            child: Some(child),
            output,
            status: BuildStatus::Compiling,
            preset: preset.clone(),
            project_root: project_root.to_path_buf(),
            engine_root: engine_root.map(Path::to_path_buf),
            crate_name: crate_name.to_owned(),
            key,
        })
    }

    /// Where the build is. Call once a frame.
    pub fn status(&self) -> &BuildStatus {
        &self.status
    }

    /// Lines cargo has produced, taken out of the buffer.
    pub fn drain_output(&self) -> Vec<String> {
        self.output
            .lock()
            .map(|mut buffer| std::mem::take(&mut *buffer))
            .unwrap_or_default()
    }

    /// Advances the job. Packaging happens here, on the frame cargo
    /// exits, because it is fast enough not to need its own process and
    /// slow enough to be worth a status of its own.
    pub fn poll(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        match child.try_wait() {
            Ok(None) => {}
            Ok(Some(exit)) => {
                self.child = None;
                if !exit.success() {
                    // cargo's own words are in the log; this is the line
                    // that says which step failed.
                    self.status =
                        BuildStatus::Failed(format!("cargo exited with {exit} — see the log"));
                    return;
                }
                self.status = BuildStatus::Packaging;
                self.package();
            }
            Err(e) => {
                self.child = None;
                self.status = BuildStatus::Failed(format!("lost track of cargo: {e}"));
            }
        }
    }

    fn package(&mut self) {
        let binary = built_binary(&self.preset, &self.project_root, &self.crate_name);
        let result = super::assemble(
            &self.preset,
            &self.project_root,
            self.engine_root.as_deref(),
            &binary,
            &self.crate_name,
            &self.key,
        );
        self.status = match result {
            Ok(package) => {
                for name in &package.shadowed {
                    tracing::warn!(
                        asset = %name,
                        "a project asset replaced the engine's of the same name",
                    );
                }
                BuildStatus::Done(package)
            }
            Err(PackageError::NoBinary(path)) => BuildStatus::Failed(format!(
                "cargo succeeded but produced no executable at {} — check the \
                 preset's target triple",
                path.display(),
            )),
            Err(e) => BuildStatus::Failed(e.to_string()),
        };
    }

    /// Stops the build.
    ///
    /// ⚠️ cargo is killed, not asked. It has no "stop when convenient",
    /// and a build that keeps compiling after the button says it stopped
    /// is worse than an interrupted one — cargo recovers from a kill by
    /// redoing whatever crate it was on.
    ///
    /// Nothing is packaged, so a half-built executable never reaches the
    /// output folder: `assemble` only runs when cargo exits clean.
    pub fn cancel(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.child = None;
        self.status = BuildStatus::Cancelled;
    }
}

impl Drop for BuildJob {
    /// A build outliving the editor would keep a `target/` lock and half
    /// a `dist/` behind it.
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// The cargo invocation a preset describes.
///
/// 🔴 `--bin <crate>` names the **game**, never `<crate>_editor`. The
/// authoring binary is gated behind a feature a shipped build does not
/// enable (#558), and asking for it by accident is the one way to put the
/// editor back into a release.
pub fn cargo_command(preset: &BuildPreset, project_root: &Path, crate_name: &str) -> Command {
    let mut command = Command::new("cargo");
    command
        .arg("build")
        .arg("--manifest-path")
        .arg(project_root.join("Cargo.toml"))
        .arg("--bin")
        .arg(crate_name);
    if preset.release {
        command.arg("--release");
    }
    if !preset.target_triple.trim().is_empty() {
        command.arg("--target").arg(preset.target_triple.trim());
    }
    let features = preset.feature_list();
    if !features.is_empty() {
        command.arg("--features").arg(features.join(","));
    }
    // 🔴 mingw's gcc defaults to C23, where `false` is a keyword, and
    // GKlib declares an enum member with that name — so metis-sys fails
    // to build for Windows without this. Measured, not guessed: the
    // engine cross-compiles with it and does not without.
    if preset.target_triple.contains("windows-gnu") {
        command.env("CFLAGS_x86_64_pc_windows_gnu", "-std=gnu17");
    }
    command
}

/// Where cargo leaves the executable for this preset.
pub fn built_binary(preset: &BuildPreset, project_root: &Path, crate_name: &str) -> PathBuf {
    let mut path = project_root.join("target");
    if !preset.is_host() {
        path = path.join(preset.target_triple.trim());
    }
    // The name cargo writes, which is the crate's — the preset's own
    // name is what the *copy* is called.
    let produced = match preset.target_triple.contains("windows") {
        true => format!("{crate_name}.exe"),
        false => crate_name.to_owned(),
    };
    path.join(preset.profile_dir()).join(produced)
}

/// A reason this build cannot start, or `None`.
///
/// Only what is knowable without compiling. A missing C toolchain is not
/// — that surfaces from cargo, and guessing at it would mean refusing
/// builds that would have worked.
fn missing_toolchain(preset: &BuildPreset) -> Option<String> {
    if preset.is_host() {
        return None;
    }
    let triple = preset.target_triple.trim();
    let installed = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .ok()?;
    let installed = String::from_utf8_lossy(&installed.stdout);
    if installed.lines().any(|line| line.trim() == triple) {
        return None;
    }
    // 🔴 The whole reason this check exists: without it cargo runs for
    // ten minutes and fails with a linker error that never says the word
    // "target".
    Some(format!(
        "the target {triple} is not installed — run:\n  rustup target add {triple}",
    ))
}

/// Forwards a pipe into the shared buffer.
fn capture(stream: Option<impl std::io::Read + Send + 'static>, into: &Arc<Mutex<Vec<String>>>) {
    let Some(stream) = stream else { return };
    let into = Arc::clone(into);
    std::thread::spawn(move || {
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            if let Ok(mut buffer) = into.lock() {
                buffer.push(kooch_core::strip_ansi(&line));
            }
        }
    });
}

#[cfg(test)]
mod compile_tests;
