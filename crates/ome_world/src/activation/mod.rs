//! Chunk activation — decides which chunks the manager should load /
//! unload based on the union of active [`StreamingFocus`] regions and
//! the global [`LodRingConfig`].
//!
//! Two layers:
//! - [`activate_chunks`] is the pure algorithm: takes focus positions
//!   in universe coords and updates the manager's queues. Trivial to
//!   unit-test without an ECS.
//! - [`activation_system`] is the ECS-aware wrapper: reads
//!   `ActiveOrigin` + iterates `(StreamingFocus, GlobalTransform)`
//!   entities, then delegates to the pure function.
//!
//! Coordinate convention: focus positions and chunk grid indices both
//! work in **absolute world / universe coordinates**, not the
//! simulation frame. A focus at `GlobalTransform.translation = (5,5,5)`
//! against `ActiveOrigin = (1000, 0, 0)` produces a focus universe
//! position of `(1005, 5, 5)`. This keeps the activation logic
//! correct across origin rebases without per-frame remapping of the
//! grid.

mod helpers;
mod public;

#[cfg(test)]
mod tests;

pub use public::{activate_chunks, activate_chunks_cached, activation_system};
