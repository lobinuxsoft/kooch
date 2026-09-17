//! Where a system runs inside its stage (#392).
//!
//! 🔴 Constraints name systems rather than holding handles: a plugin cannot get a handle to a
//! system the engine registered, and two plugins that both want to run late must be able to say so
//! without knowing which one loaded first.

use super::any_system::AnySystem;
use super::identity::{canonical, short_name};

/// What a system's position in its stage is pinned against, by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Order {
    /// Names this system runs before.
    pub before: Vec<String>,
    /// Names this system runs after.
    pub after: Vec<String>,
}

impl Order {
    /// Runs before every system answering to `name`.
    pub fn before(name: impl Into<String>) -> Self {
        Self::default().and_before(name)
    }

    /// Runs after every system answering to `name`.
    pub fn after(name: impl Into<String>) -> Self {
        Self::default().and_after(name)
    }

    pub fn and_before(mut self, name: impl Into<String>) -> Self {
        self.before.push(name.into());
        self
    }

    pub fn and_after(mut self, name: impl Into<String>) -> Self {
        self.after.push(name.into());
        self
    }

    pub fn is_empty(&self) -> bool {
        self.before.is_empty() && self.after.is_empty()
    }

    /// Whether `system` is one of the systems `name` addresses. A full `type_name` path matches,
    /// and so does the short name a reader writes — the same name the Systems panel shows.
    fn names(name: &str, system: &AnySystem) -> bool {
        let key = &system.key().name;
        key == canonical(name) || short_name(key) == short_name(name)
    }
}

/// Sorts a stage so every constraint holds, keeping registration order wherever it does not decide.
///
/// A cycle leaves the stage as it was: an order that contradicts itself is an author's mistake, and
/// a frame that runs its systems in the order they were added is the one thing known to work.
pub(super) fn sort(systems: &mut Vec<AnySystem>, stage: crate::stage::Stage) {
    if systems.iter().all(|system| system.order().is_empty()) {
        return;
    }
    let edges = edges(systems);
    match kahn(systems.len(), &edges) {
        Some(order) => {
            let mut taken: Vec<Option<AnySystem>> = systems.drain(..).map(Some).collect();
            systems.extend(order.into_iter().filter_map(|i| taken[i].take()));
        }
        None => tracing::error!(
            stage = stage.name(),
            "systems in this stage order themselves in a cycle; running them as registered"
        ),
    }
}

/// `(earlier, later)` pairs the sort has to respect. A name nothing answers to is dropped: the
/// plugin that owns it may simply not be loaded.
fn edges(systems: &[AnySystem]) -> Vec<(usize, usize)> {
    let mut edges = Vec::new();
    for (i, system) in systems.iter().enumerate() {
        for name in &system.order().after {
            edges.extend(matching(systems, name, i).map(|j| (j, i)));
        }
        for name in &system.order().before {
            edges.extend(matching(systems, name, i).map(|j| (i, j)));
        }
    }
    edges
}

fn matching<'a>(
    systems: &'a [AnySystem],
    name: &'a str,
    besides: usize,
) -> impl Iterator<Item = usize> + 'a {
    systems
        .iter()
        .enumerate()
        .filter(move |(j, system)| *j != besides && Order::names(name, system))
        .map(|(j, _)| j)
}

/// Kahn's algorithm, taking the lowest ready index first so registration order breaks every tie.
/// `None` on a cycle.
fn kahn(count: usize, edges: &[(usize, usize)]) -> Option<Vec<usize>> {
    let mut incoming = vec![0usize; count];
    for &(_, later) in edges {
        incoming[later] += 1;
    }
    let mut order = Vec::with_capacity(count);
    let mut ready: Vec<usize> = (0..count).filter(|i| incoming[*i] == 0).collect();
    while let Some(next) = ready.iter().copied().min() {
        ready.retain(|i| *i != next);
        order.push(next);
        for &(_, later) in edges.iter().filter(|(e, _)| *e == next) {
            incoming[later] -= 1;
            if incoming[later] == 0 {
                ready.push(later);
            }
        }
    }
    (order.len() == count).then_some(order)
}

#[cfg(test)]
mod tests;
