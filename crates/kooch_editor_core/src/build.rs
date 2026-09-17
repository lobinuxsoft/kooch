//! Producing a game from a project (#758).

pub mod compile;
/// - [`dlss`] — the two obligations a build with NVIDIA's upscaler
///   carries: the SDK before cargo, the runtime and the notices after.
pub mod dlss;
pub mod key;
pub mod mingw;
pub mod package;
pub mod platform;
pub mod preset;
pub mod scene_features;

pub use compile::{BuildJob, BuildStatus};
pub use key::project_key;
pub use package::PACK_FILE;
pub use package::{Package, PackageError, assemble};
pub use platform::Platform;
pub use preset::{BUILD_PRESET_EXTENSION, BuildPreset, BuildPresetLoader};

/// The editor's build state: the running job and what it has said.
#[derive(Default)]
pub struct BuildState {
    /// The running job, or the finished one — kept after it ends so the
    /// panel can go on showing where the output landed.
    pub job: Option<BuildJob>,
    /// Everything cargo has said this run.
    pub log: Vec<String>,
}

impl BuildState {
    /// Moves the job along and collects its output. Once a frame.
    pub fn poll(&mut self) {
        let Some(job) = self.job.as_mut() else {
            return;
        };
        // Drained every frame whatever the status: the reader threads
        // keep filling the buffer, and a build that failed has its reason
        // in the last few lines.
        self.log.extend(job.drain_output());
        job.poll();
    }

    /// Whether a build is running right now.
    pub fn busy(&self) -> bool {
        matches!(
            self.job.as_ref().map(BuildJob::status),
            Some(BuildStatus::Compiling { .. } | BuildStatus::Packaging),
        )
    }
}
