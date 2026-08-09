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

pub mod key;
pub mod preset;

pub use key::project_key;
pub use preset::{BUILD_PRESET_EXTENSION, BuildPreset, BuildPresetLoader};
