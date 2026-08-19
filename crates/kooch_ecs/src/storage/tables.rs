//! The tables of a world, and the component set each one serves.

use std::collections::HashMap;

use crate::component::{ComponentRegistry, StorageId};
use crate::storage::column::Column;
use crate::storage::table::Table;

/// Which table of a world a row lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TableId(pub u32);

impl TableId {
    /// The table as an index.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Every [`Table`] in a world, addressed by [`TableId`].
///
/// # One table per component set, and why that is not one per archetype
///
/// A table is looked up by the **set of components it stores**, and two
/// callers asking for the same set get the same id. Today that is the same
/// thing as one table per archetype, because every component is stored in
/// a table.
///
/// 🎯 It stops being the same thing the moment a component opts into
/// sparse-set storage (stage 7 of #891): two archetypes that differ *only*
/// in a sparse-set component must then share one table, so that gaining or
/// losing that component **does not move the row**. That is the entire
/// payoff of the sparse-set kind, and it only works if the table's
/// identity was never the archetype's identity.
///
/// So the lookup is by component set from the start. Building it the other
/// way would work today and would have to be undone later.
pub struct Tables {
    tables: Vec<Table>,
    /// Sorted component set → the table serving it.
    by_components: HashMap<Box<[StorageId]>, TableId>,
}

impl Default for Tables {
    fn default() -> Self {
        Self::new()
    }
}

impl Tables {
    /// An empty collection.
    pub fn new() -> Self {
        Self {
            tables: Vec::new(),
            by_components: HashMap::new(),
        }
    }

    /// How many tables exist.
    #[inline]
    pub fn len(&self) -> usize {
        self.tables.len()
    }

    /// Whether no table exists yet.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }

    /// The table serving `components`, creating it if this is the first
    /// time that set is asked for.
    ///
    /// The order of `components` does not matter: the set is sorted before
    /// it is looked up, so `[A, B]` and `[B, A]` are one table and not two.
    ///
    /// # Panics
    ///
    /// If a component is not registered. A table needs the concrete type to
    /// build its column, and only the registry still knows it — an
    /// unregistered component here is a bug upstream, not a recoverable
    /// state.
    pub fn get_or_insert(
        &mut self,
        registry: &ComponentRegistry,
        components: &[StorageId],
    ) -> TableId {
        let mut key: Vec<StorageId> = components.to_vec();
        key.sort_unstable();
        key.dedup();
        if let Some(id) = self.by_components.get(key.as_slice()) {
            return *id;
        }

        let columns: Vec<(StorageId, Column)> = key
            .iter()
            .map(|id| {
                let column = registry
                    .new_column(*id)
                    .unwrap_or_else(|| panic!("component {id:?} is not registered"));
                (*id, column)
            })
            .collect();

        let table_id = TableId(self.tables.len() as u32);
        self.tables.push(Table::new(columns));
        self.by_components.insert(key.into_boxed_slice(), table_id);
        table_id
    }

    /// The table `id` names.
    #[inline]
    pub fn get(&self, id: TableId) -> Option<&Table> {
        self.tables.get(id.index())
    }

    /// The table `id` names, mutably.
    #[inline]
    pub fn get_mut(&mut self, id: TableId) -> Option<&mut Table> {
        self.tables.get_mut(id.index())
    }
}

#[cfg(test)]
mod tests;
