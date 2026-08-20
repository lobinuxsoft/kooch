//! Producing a game from a project (#758).
//!
//! The editor could play a project and compile it by hand, and had no way
//! to *make a build* — which is the one thing an editor exists to do that
//! a text editor and cargo do not do together.
//!
//! - [`preset`] — `.buildpreset`, what "a build" means for one target.
//!   A reflected asset, so the Inspector edits it with no editor code.
//! - [`key`] — the key a project's packs are sealed with, deliberately
//!   outside the preset and outside version control.
//! - [`compile`] — running cargo for a preset, and packaging what
//!   comes out.
//! - [`package`] — laying out the folder a player receives: the
//!   executable, its scenes, and one asset pack merged from the two
//!   trees the editor keeps apart.

pub mod compile;
/// - [`dlss`] — the two obligations a build with NVIDIA's upscaler
///   carries: the SDK before cargo, the runtime and the notices after.
pub mod dlss;
pub mod key;
pub mod package;
pub mod preset;

pub use compile::{BuildJob, BuildStatus};
pub use key::project_key;
pub use package::PACK_FILE;
pub use package::{Package, PackageError, assemble};
pub use preset::{BUILD_PRESET_EXTENSION, BuildPreset, BuildPresetLoader};

/// The editor's build state: the running job and what it has said.
///
/// A resource rather than panel state, because a build outlives the
/// frame that started it and has to keep going while the Build tab is
/// not even visible.
#[derive(Default)]
pub struct BuildState {
    /// The running job, or the finished one — kept after it ends so the
    /// panel can go on showing where the output landed.
    pub job: Option<BuildJob>,
    /// Everything cargo has said this run.
    ///
    /// Held here rather than drained into the Console: a build's output
    /// is long, mostly `Compiling`, and burying the project's own logs
    /// under it is how the Console stops being useful.
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
