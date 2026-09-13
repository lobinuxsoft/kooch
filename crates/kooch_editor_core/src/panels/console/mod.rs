//! The Console tab — what the engine has been saying.

mod render;
mod state;

#[cfg(test)]
mod id_stability;

pub(crate) use render::draw_console;
pub(crate) use state::{ALL_LEVELS, ConsoleState};
