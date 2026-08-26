//! Fetching NVIDIA's DLSS SDK, which the engine may not ship.
//!
//! # 🔴 Why a button and not a dependency
//!
//! The SDK's licence is explicit in both directions. It forbids
//! redistributing it — *"you may not distribute or sublicense the SDK as
//! a stand-alone product"* — so an editor that carried a copy, or served
//! one from a mirror of ours, would be doing exactly that. And it is
//! accepted **by use**: *"By using the SDK, you affirm that you have
//! reached the legal age of majority, you accept the terms of this
//! license."*
//!
//! So this fetches from **NVIDIA and nowhere else**, and only after the
//! person at the keyboard has said they accept those terms. A button
//! that downloaded on the first click would be accepting a licence on
//! somebody's behalf, which is not a thing an editor gets to do.
//!
//! # What ships with a game is not this
//!
//! The SDK is headers and a static library, used while compiling. What a
//! game distributes is the **runtime** — one `.so` or `.dll` — plus the
//! copyright text from section 9.5 of the SDK's programming guide. The
//! same licence that forbids the first permits the second: *"Distribute
//! any software and materials within the SDK … as incorporated in object
//! code format into a software application."*
//!
//! # And it does not enable DLSS
//!
//! Nothing in this engine calls DLSS yet. This puts the SDK where a
//! build could find it; the backend behind `UpscaleTechnique` is #536 and
//! is not this.

use std::path::{Path, PathBuf};

/// The version `dlss_wgpu` 4.0.0 is built against.
///
/// 🔴 Pinned, not "latest". The crate's own version chart lines up
/// `dlss_wgpu 4.0.0` with SDK `v310.5.3` and wgpu 29 — which is the wgpu
/// this engine uses. A newer SDK is a different row of that table.
pub const VERSION: &str = "310.5.3";

/// The tag that `VERSION` names in NVIDIA's repository.
pub const TAG: &str = "v310.5.3";

/// 🔴 NVIDIA's repository, and never a mirror of ours: hosting a copy is
/// the "stand-alone product" the licence forbids.
pub const REPO: &str = "https://github.com/NVIDIA/DLSS";

/// The terms, at the exact tag being fetched.
pub const LICENSE: &str = "https://github.com/NVIDIA/DLSS/blob/v310.5.3/LICENSE.txt";

/// Where this machine keeps it.
pub fn sdk_dir() -> Option<PathBuf> {
    crate::engine_vendor::shared_sdk_dir("dlss", VERSION)
}

/// Whether `dir` holds a usable SDK.
///
/// Checks what the BUILD needs rather than that the directory exists: an
/// interrupted clone leaves a directory, and `dlss_wgpu`'s build script
/// wants `include/` for bindgen and the static library it links. The
/// runtime is checked too because that is what a game ships.
pub fn is_installed(dir: &Path) -> bool {
    dir.join("include/nvsdk_ngx_helpers.h").is_file()
        && dir.join("lib/Linux_x86_64/libnvsdk_ngx.a").is_file()
        && runtime_path(dir).is_file()
}

/// The one file a game distributes beside its executable.
pub fn runtime_path(dir: &Path) -> PathBuf {
    dir.join(format!(
        "lib/Linux_x86_64/rel/libnvidia-ngx-dlss.so.{VERSION}"
    ))
}

/// The runtime a build for `triple` ships, inside `dir`.
///
/// Empty means this machine, which is the only case where the host's own
/// name is the right answer.
pub fn runtime_for(dir: &Path, platform: crate::build::Platform) -> PathBuf {
    match platform {
        crate::build::Platform::Windows => dir.join("lib/Windows_x86_64/rel/nvngx_dlss.dll"),
        crate::build::Platform::Linux => runtime_path(dir),
    }
}

/// The document whose section 9.5 a shipped game has to carry.
pub fn notices_path(dir: &Path) -> PathBuf {
    dir.join("doc/DLSS_Programming_Guide_Release.pdf")
}

/// The clone, as arguments.
///
/// `--depth 1` because the history is not wanted and the checkout is
/// large; `-b TAG` because the version is pinned to the crate.
pub fn clone_args(dest: &Path) -> Vec<String> {
    vec![
        "clone".to_owned(),
        "--depth".to_owned(),
        "1".to_owned(),
        "-b".to_owned(),
        TAG.to_owned(),
        REPO.to_owned(),
        dest.to_string_lossy().into_owned(),
    ]
}

/// What the editor shows about the SDK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SdkState {
    /// Present and usable.
    Installed(PathBuf),
    /// Not here, and this is where it would go.
    Missing(PathBuf),
    /// Fetching, with whatever git last said.
    Fetching(String),
    /// The last attempt failed, with the reason.
    Failed(String),
    /// No data directory on this platform to put it in.
    Nowhere,
}

impl SdkState {
    /// Looks at the disk. Cheap enough to call while drawing a panel:
    /// three `is_file`s.
    pub fn detect() -> Self {
        match sdk_dir() {
            Some(dir) if is_installed(&dir) => Self::Installed(dir),
            Some(dir) => Self::Missing(dir),
            None => Self::Nowhere,
        }
    }
}

/// The fetch, and the acceptance that has to precede it.
///
/// 🔴 The `accepted` flag is not a formality and not ours to default to
/// true. NVIDIA's licence is accepted by USE, so the moment this editor
/// puts the SDK on the disk somebody has accepted it — and it must be
/// the person at the keyboard, having been shown where the terms are.
#[derive(Debug, Default)]
pub struct SdkInstall {
    pub state: Option<SdkState>,
    /// Set by a tick box next to the licence link. Reset on failure so a
    /// retry is a second deliberate act.
    pub accepted: bool,
    /// Where the fetch thread leaves its answer.
    ///
    /// `Arc<Mutex<..>>` rather than a channel because this lives in
    /// `Resources`, which requires `Sync`, and `mpsc::Receiver` is not —
    /// the same shape `PlayState` uses for the output it collects.
    progress: Option<std::sync::Arc<std::sync::Mutex<Option<Result<PathBuf, String>>>>>,
}

impl SdkInstall {
    /// The state, looked up once and then remembered.
    pub fn state(&mut self) -> &SdkState {
        self.state.get_or_insert_with(SdkState::detect)
    }

    /// Whether a fetch may start: terms accepted, somewhere to put it,
    /// and nothing already running.
    pub fn can_fetch(&mut self) -> bool {
        self.accepted
            && self.progress.is_none()
            && matches!(self.state(), SdkState::Missing(_) | SdkState::Failed(_))
    }

    /// Starts the clone on a thread. The editor keeps drawing.
    pub fn fetch(&mut self) {
        if !matches!(self.state(), SdkState::Missing(_) | SdkState::Failed(_)) {
            return;
        }
        let Some(dir) = sdk_dir() else {
            self.state = Some(SdkState::Nowhere);
            return;
        };
        let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
        self.progress = Some(std::sync::Arc::clone(&slot));
        self.state = Some(SdkState::Fetching("cloning from NVIDIA…".to_owned()));
        std::thread::spawn(move || {
            let result = clone_into(&dir);
            if let Ok(mut slot) = slot.lock() {
                *slot = Some(result);
            }
        });
    }

    /// Picks up a finished fetch. Called while drawing; does nothing
    /// until the thread reports.
    pub fn poll(&mut self) {
        let Some(slot) = &self.progress else { return };
        let finished = slot.lock().ok().and_then(|mut slot| slot.take());
        let Some(result) = finished else { return };
        self.progress = None;
        match result {
            Ok(dir) => self.state = Some(SdkState::Installed(dir)),
            Err(problem) => {
                // 🔴 Acceptance is cleared with the failure: a retry is a
                // second deliberate act rather than a click that inherits
                // consent given for an attempt that did not happen.
                self.accepted = false;
                self.state = Some(SdkState::Failed(problem));
            }
        }
    }
}

/// Clones, then checks what arrived is usable.
///
/// ⚠️ A clone that exits 0 is not proof: a partial checkout, a tag that
/// moved, a layout NVIDIA changed. The directory is verified against what
/// the build actually needs and removed when it does not hold it, so a
/// broken attempt cannot look installed.
fn clone_into(dir: &Path) -> Result<PathBuf, String> {
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let _ = std::fs::remove_dir_all(dir);
    let output = std::process::Command::new("git")
        .args(clone_args(dir))
        .output()
        .map_err(|e| format!("git could not be run: {e}"))?;
    if !output.status.success() {
        let _ = std::fs::remove_dir_all(dir);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr.lines().last().unwrap_or("git failed").to_owned());
    }
    if !is_installed(dir) {
        let _ = std::fs::remove_dir_all(dir);
        return Err("the clone finished but the SDK is not in it".to_owned());
    }
    Ok(dir.to_path_buf())
}

#[cfg(test)]
mod tests;
