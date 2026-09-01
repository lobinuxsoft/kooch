//! What is scheduled, published where a system can read it.
//!
//! [`Schedule`](super::Schedule) lives on the `App`, not in `Resources`,
//! so a panel — which is itself a system — cannot reach it. The catalog
//! is the schedule's own description of itself, copied into `Resources`
//! once every plugin has been built.
//!
//! ⚠️ A snapshot, not a live view. Everything the engine and a project
//! schedule is added before `App::run`, which is where this is written;
//! a system added after that will not appear until something calls
//! [`App::publish_systems`](crate::app::App::publish_systems) again.

use super::identity::{SystemKey, SystemSource, short_name};
use crate::stage::Stage;

/// One scheduled system, owned so it can outlive the borrow it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemRecord {
    pub stage: Stage,
    /// The full path, kept because it is what a log or a profile says.
    pub name: String,
    pub key: SystemKey,
    pub source: SystemSource,
    pub gpu: bool,
}

impl SystemRecord {
    /// The name to put in a list.
    pub fn short_name(&self) -> &str {
        short_name(&self.name)
    }
}

/// Every system the schedule holds, in the order a frame runs them.
#[derive(Debug, Default, Clone)]
pub struct SystemCatalog {
    systems: Vec<SystemRecord>,
}

impl SystemCatalog {
    pub fn new(systems: Vec<SystemRecord>) -> Self {
        Self { systems }
    }

    /// Every system, in run order.
    pub fn all(&self) -> &[SystemRecord] {
        &self.systems
    }

    /// The systems one half of the build scheduled, in run order.
    pub fn from(&self, source: SystemSource) -> impl Iterator<Item = &SystemRecord> {
        self.systems
            .iter()
            .filter(move |system| system.source == source)
    }

    /// How many systems are listed.
    pub fn len(&self) -> usize {
        self.systems.len()
    }

    /// `true` when nothing has been published yet.
    pub fn is_empty(&self) -> bool {
        self.systems.is_empty()
    }
}

#[cfg(test)]
mod tests;
