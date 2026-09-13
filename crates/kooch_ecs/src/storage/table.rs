//! A table: the columns of one component set, sharing a row index.

use crate::component::StorageId;
use crate::entity::Entity;
use crate::storage::column::Column;

/// Which row of a [`Table`] an entity's components live in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TableRow(pub u32);

impl TableRow {
    /// The row as an index.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// One [`Column`] per component of a fixed component set, plus the entity that owns each row.
pub struct Table {
    /// Dense, in the order the columns were given.
    columns: Vec<Column>,
    /// Which component each dense column holds. Parallel to `columns`.
    ids: Vec<StorageId>,
    /// `StorageId::index()` → position in `columns`. Sparse, and only as
    /// long as the largest id this table holds — not as long as every id
    /// in the world.
    sparse: Vec<Option<u32>>,
    /// Row → the entity that owns it.
    entities: Vec<Entity>,
}

impl Table {
    /// A table for the given components, each with its own empty column.
    pub fn new(columns: impl IntoIterator<Item = (StorageId, Column)>) -> Self {
        let mut table = Self {
            columns: Vec::new(),
            ids: Vec::new(),
            sparse: Vec::new(),
            entities: Vec::new(),
        };
        for (id, column) in columns {
            let slot = id.index();
            if slot >= table.sparse.len() {
                table.sparse.resize(slot + 1, None);
            }
            assert!(
                table.sparse[slot].is_none(),
                "component {id:?} was given two columns"
            );
            table.sparse[slot] = Some(table.columns.len() as u32);
            table.columns.push(column);
            table.ids.push(id);
        }
        table
    }

    /// How many rows the table holds.
    #[inline]
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// Whether the table holds no rows.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// The entity owning each row, in row order.
    #[inline]
    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    #[cfg(test)]
    /// The components this table holds a column for.
    #[inline]
    pub fn component_ids(&self) -> &[StorageId] {
        &self.ids
    }

    /// The column holding `id`, if this table has one.
    pub fn column(&self, id: StorageId) -> Option<&Column> {
        let slot = *self.sparse.get(id.index())?;
        self.columns.get(slot? as usize)
    }

    /// The column holding `id`, mutably.
    pub fn column_mut(&mut self, id: StorageId) -> Option<&mut Column> {
        let slot = (*self.sparse.get(id.index())?)? as usize;
        self.columns.get_mut(slot)
    }

    /// Whether every column holds exactly one item per row.
    pub fn rows_agree(&self) -> bool {
        self.columns
            .iter()
            .all(|column| column.len() == self.entities.len())
    }

    /// Claims the next row for `entity` and returns it.
    pub fn push_entity(&mut self, entity: Entity) -> TableRow {
        let row = TableRow(self.entities.len() as u32);
        self.entities.push(entity);
        row
    }

    /// Removes `row` from every column and from the entity list, pulling the last row into the
    /// hole.
    pub fn swap_remove(&mut self, row: TableRow) -> Option<Entity> {
        assert!(
            row.index() < self.entities.len(),
            "row {} is past the end ({})",
            row.0,
            self.entities.len()
        );
        debug_assert!(self.rows_agree(), "a column drifted out of step");

        for column in &mut self.columns {
            column.swap_remove(row.index());
        }
        self.entities.swap_remove(row.index());

        // `swap_remove` on a Vec leaves the moved element at `row` — unless
        // the removed one was last, in which case nothing moved.
        self.entities.get(row.index()).copied()
    }

    /// Moves `row` into `dst`, carrying every component both tables hold and **dropping** the ones
    /// only this table has.
    pub fn move_row_to(&mut self, row: TableRow, dst: &mut Table) -> (TableRow, Option<Entity>) {
        assert!(
            row.index() < self.entities.len(),
            "row {} is past the end ({})",
            row.0,
            self.entities.len()
        );
        debug_assert!(self.rows_agree(), "a column drifted out of step");

        let entity = self.entities[row.index()];
        let landed = dst.push_entity(entity);

        for index in 0..self.columns.len() {
            let id = self.ids[index];
            match dst.column_mut(id) {
                // SAFETY: both columns were built for the component `id`
                // names, so they hold the same item type.
                Some(target) => unsafe {
                    self.columns[index].move_row_to(row.index(), target);
                },
                // Only this table has it: the entity is losing it, so the
                // value is destroyed rather than carried.
                None => self.columns[index].swap_remove(row.index()),
            }
        }

        self.entities.swap_remove(row.index());
        (landed, self.entities.get(row.index()).copied())
    }
}

#[cfg(test)]
mod tests;
