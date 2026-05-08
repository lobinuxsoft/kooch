//! Dense GPU-backed component storage with CPU shadow copy.
//!
//! [`GpuComponentStorage<T>`] stores component data in a `Vec<T>` indexed by
//! `entity.index()`, with a lazy [`GpuBuffer<T>`] that is created on first
//! sync. Dirty tracking uses a min/max range for efficient partial uploads.

mod storage;

#[cfg(test)]
mod tests;

pub use storage::GpuComponentStorage;
