//! Chunks to load and unload from the [`StreamingFocus`](crate::focus::StreamingFocus) union and [`LodRingConfig`](crate::lod::LodRingConfig):
//! [`activate_chunks`] is pure, [`activation_system`] wraps it. Universe coordinates, so rebases
//! need no remapping.

mod helpers;
mod public;

#[cfg(test)]
mod tests;

pub use public::{activate_chunks, activate_chunks_cached, activation_system};
