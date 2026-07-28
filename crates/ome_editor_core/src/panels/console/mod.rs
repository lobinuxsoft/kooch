//! The Console tab — what the engine has been saying.
//!
//! Everything reaches this through `tracing`, including a running
//! project's output, which the editor forwards as `[game] ...`. So there is
//! one log rather than a panel and a terminal that drift apart.
//!
//! # Why filters are not optional
//!
//! At `info` the engine is chatty — asset scans, meshlet LOD chains, one
//! line per pipeline. The line that matters is a `warn` about a joint with
//! no bodies, three hundred lines up. A log nobody can narrow is a log
//! nobody reads, which is the state this panel replaces.
//!
//! # Why it costs nothing to leave open
//!
//! [`state`] holds the panel's own copy of the log and its own filtered
//! view, both updated only by what changed. [`render`] draws only the rows
//! on screen. The first version did neither, and an open Console cloned
//! two thousand lines and laid out two thousand rows every frame.

mod render;
mod state;

#[cfg(test)]
mod id_stability;

pub(crate) use render::draw_console;
pub(crate) use state::ConsoleState;
