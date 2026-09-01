//! System scheduling and execution.
//!
//! The schedule organizes systems by stage and executes them in order.
//! Both CPU [`System`]s and GPU [`GpuSystem`]s are supported, with
//! consecutive GPU systems batched into a single command encoder.

mod any_system;
mod gpu_batch;
mod identity;
#[allow(clippy::module_inception)]
mod schedule;
mod system_scope;
mod toggles;

#[cfg(test)]
mod tests;

pub use identity::{SystemInfo, SystemKey, SystemSource, canonical, short_name};
pub use schedule::{RUN_ORDER, Schedule, SystemFn};
pub use toggles::SystemToggles;
