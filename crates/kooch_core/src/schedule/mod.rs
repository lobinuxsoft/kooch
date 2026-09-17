//! System scheduling and execution.

mod any_system;
mod catalog;
mod gpu_batch;
mod identity;
mod order;
#[allow(clippy::module_inception)]
mod schedule;
mod system_scope;
mod toggles;

#[cfg(test)]
mod tests;

pub use catalog::{SystemCatalog, SystemRecord};
pub use identity::{SystemInfo, SystemKey, SystemSource, canonical, short_name};
pub use order::Order;
pub use schedule::{RUN_ORDER, Schedule, SystemFn};
pub use toggles::SystemToggles;
