//! The tables of a world, and the component set each one serves.

use std::collections::HashMap;

use crate::component::{ComponentRegistry, StorageId};
use crate::entity::Entity;
use crate::storage::column::Column;
use crate::storage::table::{Table, TableRow};

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

    /// The table serving `components`, creating it if this is the first time that set is asked for.
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

    /// The table serving `components`, **without creating one**.
    pub fn find(&self, components: &[StorageId]) -> Option<TableId> {
        let mut key: Vec<StorageId> = components.to_vec();
        key.sort_unstable();
        key.dedup();
        self.by_components.get(key.as_slice()).copied()
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

    /// Moves a row from one table to another, returning where it landed and the entity dragged into
    /// the hole it left.
    pub fn move_row(
        &mut self,
        from: TableId,
        row: TableRow,
        to: TableId,
    ) -> (TableRow, Option<Entity>) {
        assert_ne!(from, to, "a row cannot move to the table it is already in");
        assert!(from.index() < self.tables.len(), "unknown table {from:?}");
        assert!(to.index() < self.tables.len(), "unknown table {to:?}");

        // `split_at_mut` is what makes two `&mut` out of one `Vec`: the
        // pivot sits between the two indices, so each half holds exactly
        // one of them.
        let (source, target) = if from.index() < to.index() {
            let (left, right) = self.tables.split_at_mut(to.index());
            (&mut left[from.index()], &mut right[0])
        } else {
            let (left, right) = self.tables.split_at_mut(from.index());
            (&mut right[0], &mut left[to.index()])
        };

        source.move_row_to(row, target)
    }
}

#[cfg(test)]
mod tests;
