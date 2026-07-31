//! System scheduling and execution.
//!
//! The schedule organizes systems by stage and executes them in order.
//! Both CPU [`System`]s and GPU [`GpuSystem`]s are supported, with
//! consecutive GPU systems batched into a single command encoder.

mod any_system;
mod gpu_batch;
#[allow(clippy::module_inception)]
mod schedule;

#[cfg(test)]
mod tests;

pub use schedule::{Schedule, SystemFn};
