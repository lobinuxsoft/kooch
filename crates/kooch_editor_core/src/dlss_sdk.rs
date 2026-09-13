//! Fetching NVIDIA's DLSS SDK, which the engine may not ship.

use std::path::{Path, PathBuf};

/// The version `dlss_wgpu` 4.0.0 is built against.
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
#[derive(Debug, Default)]
pub struct SdkInstall {
    pub state: Option<SdkState>,
    /// Set by a tick box next to the licence link. Reset on failure so a
    /// retry is a second deliberate act.
    pub accepted: bool,
    /// Where the fetch thread leaves its answer.
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
