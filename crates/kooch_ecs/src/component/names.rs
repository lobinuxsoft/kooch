//! Process-local interning of component type names.

use std::collections::HashMap;
use std::sync::Arc;

/// A process-local handle to an interned component type name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ComponentId(pub u32);

impl ComponentId {
    /// Sentinel for a component whose name is not (yet) interned.
    pub const INVALID: ComponentId = ComponentId(u32::MAX);

    /// `true` unless this is [`ComponentId::INVALID`].
    pub fn is_valid(self) -> bool {
        self != Self::INVALID
    }
}

/// Bidirectional interner mapping component type names to [`ComponentId`]s.
#[derive(Debug, Default, Clone)]
pub struct ComponentNames {
    /// `id.0` indexes this: the interned name for each id.
    names: Vec<Arc<str>>,
    /// Reverse lookup, name → id.
    lookup: HashMap<Arc<str>, ComponentId>,
}

impl ComponentNames {
    /// Creates an empty interner.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the id for `name`, assigning a fresh one if unseen.
    pub fn intern(&mut self, name: &str) -> ComponentId {
        if let Some(&id) = self.lookup.get(name) {
            return id;
        }
        let id = ComponentId(self.names.len() as u32);
        let shared: Arc<str> = Arc::from(name);
        self.names.push(Arc::clone(&shared));
        self.lookup.insert(shared, id);
        id
    }

    /// Returns the id for `name` if it has been interned, without
    /// assigning one. Used on read-only paths that must not mutate.
    pub fn id(&self, name: &str) -> Option<ComponentId> {
        self.lookup.get(name).copied()
    }

    /// Returns the name behind `id`, or `None` if it was never issued by
    /// this interner.
    pub fn name(&self, id: ComponentId) -> Option<&str> {
        self.names.get(id.0 as usize).map(|n| n.as_ref())
    }

    /// Number of distinct names interned so far.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// `true` when nothing has been interned.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

#[cfg(test)]
mod tests;
