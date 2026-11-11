//! Reading and writing one component type wherever its values live (#891).
//!
//! 🔴 A value lives in **one** place: the column of the table serving its entity's archetype, or —
//! before the archetype has caught up with it — the per-type map it was written into. The map is
//! the inbox and the column is the home, because every insert site in this engine writes the value
//! first and changes the archetype after, and at the archetype site only a `TypeId` is in hand.
//!
//! So these views look in the column first and the map second. That is a lookup, not a preference.

use crate::component::cpu_storage::ComponentStorage;
use crate::component::locations::Locations;
use crate::component::storage_id::StorageId;
use crate::component::traits::Component;
use crate::entity::Entity;
use crate::storage::{Table, Tables};

/// Where every entity's values live, if they live in a table.
type At = Locations;

/// A read-only view of every value of one component type.
pub struct ComponentView<'a, T: Component> {
    pub(super) id: StorageId,
    pub(super) map: &'a ComponentStorage<T>,
    pub(super) tables: &'a Tables,
    pub(super) entity_at: &'a At,
    /// How many writes this component has seen. Read by the editor's frame snapshot cache (#1561).
    pub(super) revision: u64,
}

// A view is four shared references: copying it is free, and the sites that hold one while walking
// a closure need it to be.
impl<T: Component> Clone for ComponentView<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Component> Copy for ComponentView<'_, T> {}

impl<'a, T: Component> ComponentView<'a, T> {
    /// The value for `entity`, if it has one.
    pub fn get(&self, entity: Entity) -> Option<&'a T> {
        if let Some((table, row)) = self.entity_at.get(entity)
            && let Some(column) = self.tables.get(table).and_then(|held| held.column(self.id))
        {
            // SAFETY: the column was built for the component `id` names, which is `T`.
            return unsafe { column.get::<T>(row.index()) };
        }
        self.map.get(entity)
    }

    /// Whether `entity` has this component.
    pub fn contains(&self, entity: Entity) -> bool {
        self.get(entity).is_some()
    }

    /// How many entities have it.
    pub fn len(&self) -> usize {
        self.map.len()
            + columns(self.tables, self.id)
                .map(|(_, len)| len)
                .sum::<usize>()
    }

    /// Whether nothing has it.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// How many times this component has been written to.
    #[inline]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Every `(entity, value)` pair, the dense columns first and the inbox last.
    pub fn iter(&self) -> impl Iterator<Item = (&'a Entity, &'a T)> {
        rows(self.tables, self.id).chain(self.map.iter())
    }

    /// The change tick of `entity`'s value (#1870); 0 when it has none.
    pub fn changed_at(&self, entity: Entity) -> u64 {
        if let Some((table, row)) = self.entity_at.get(entity)
            && let Some(column) = self.tables.get(table).and_then(|held| held.column(self.id))
        {
            return column.changed_at(row.index());
        }
        self.map.changed_at(entity)
    }

    /// Every pair written after `since`: one `u64` compare per row of a contiguous array.
    pub fn iter_changed(&self, since: u64) -> impl Iterator<Item = (&'a Entity, &'a T)> {
        let map = self.map;
        ticked_rows(self.tables, self.id)
            .filter(move |(_, _, tick)| *tick > since)
            .map(|(entity, value, _)| (entity, value))
            .chain(
                map.iter()
                    .filter(move |(entity, _)| map.changed_at(**entity) > since),
            )
    }
}

/// A writable view of every value of one component type.
pub struct ComponentViewMut<'a, T: Component> {
    pub(super) id: StorageId,
    pub(super) map: &'a mut ComponentStorage<T>,
    pub(super) tables: &'a mut Tables,
    pub(super) entity_at: &'a At,
    pub(super) revision: &'a mut u64,
}

impl<'a, T: Component> ComponentViewMut<'a, T> {
    /// The value for `entity`, mutably, outliving the view.
    ///
    /// For `get_cpu_mut::<T>().and_then(|view| view.into_mut(entity))`, where the view is a
    /// temporary and [`Self::get_mut`]'s borrow would die with it.
    pub fn into_mut(self, entity: Entity) -> Option<&'a mut T> {
        *self.revision += 1;
        if let Some((table, row)) = self.entity_at.get(entity)
            && let Some(column) = self
                .tables
                .get_mut(table)
                .and_then(|held| held.column_mut(self.id))
        {
            // SAFETY: the column holds `T` and the view owned the exclusive borrow.
            return unsafe { column.get_mut::<T>(row.index()) };
        }
        self.map.get_mut(entity)
    }
}

impl<T: Component> ComponentViewMut<'_, T> {
    /// The value for `entity`, if it has one.
    pub fn get(&self, entity: Entity) -> Option<&T> {
        if let Some(column) = self.column_of(entity) {
            let (_, row) = self.entity_at.get(entity)?;
            // SAFETY: the column holds `T`, and the row is this registry's own.
            return unsafe { column.get::<T>(row.index()) };
        }
        self.map.get(entity)
    }

    /// The value for `entity`, mutably.
    ///
    /// Counts as a write: whether the caller changes anything is not knowable here, and a missed
    /// bump is a panel that stops updating. A miss hands out nothing, so it is not one.
    pub fn get_mut(&mut self, entity: Entity) -> Option<&mut T> {
        if let Some((table, row)) = self.entity_at.get(entity)
            && let Some(column) = self
                .tables
                .get_mut(table)
                .and_then(|held| held.column_mut(self.id))
        {
            *self.revision += 1;
            // SAFETY: the column holds `T` and `&mut self` is exclusive.
            return unsafe { column.get_mut::<T>(row.index()) };
        }
        let value = self.map.get_mut(entity);
        if value.is_some() {
            *self.revision += 1;
        }
        value
    }

    /// Stores `value` for `entity`, returning what it replaced.
    ///
    /// Lands in the column when the entity already has a row for this component, and in the inbox
    /// otherwise — which is the case every time a component is **gained**, because the archetype
    /// change that would build the column has not happened yet.
    pub fn insert(&mut self, entity: Entity, value: T) -> Option<T> {
        *self.revision += 1;
        if let Some((table, row)) = self.entity_at.get(entity)
            && let Some(column) = self
                .tables
                .get_mut(table)
                .and_then(|held| held.column_mut(self.id))
        {
            // SAFETY: the column holds `T` and `&mut self` is exclusive.
            return unsafe { column.replace::<T>(row.index(), value) };
        }
        self.map.insert(entity, value)
    }

    /// Takes `entity`'s value out of the **inbox**, and only there.
    ///
    /// 🔴 Named for what it can do. A value already in a column cannot be taken out on its own: a
    /// column loses a row only when the whole table does, and the entity's other components live
    /// in that row. Taking a component off an entity for good is
    /// [`move_entity::take`](crate::move_entity::take), which drops the value **and** the archetype
    /// that claimed it. A caller that unparks and never changes the archetype has removed nothing.
    pub fn unpark(&mut self, entity: Entity) -> Option<T> {
        *self.revision += 1;
        self.map.remove(entity)
    }

    /// Whether `entity` has this component.
    pub fn contains(&self, entity: Entity) -> bool {
        self.get(entity).is_some()
    }

    /// How many entities have it.
    pub fn len(&self) -> usize {
        self.map.len()
            + columns(self.tables, self.id)
                .map(|(_, len)| len)
                .sum::<usize>()
    }

    /// Whether nothing has it.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// How many times this component has been written to.
    #[inline]
    pub fn revision(&self) -> u64 {
        *self.revision
    }

    /// Every `(entity, value)` pair.
    pub fn iter(&self) -> impl Iterator<Item = (&Entity, &T)> {
        rows(self.tables, self.id).chain(self.map.iter())
    }

    /// Every `(entity, value)` pair, mutably. A row counts as written only when its [`Mut`] is
    /// written through, so a loop that reads everything and changes a few marks only the few.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&Entity, Mut<'_, T>)> {
        *self.revision += 1;
        let id = self.id;
        let dense = self.tables.iter_mut().filter_map(move |table| {
            let entities = std::ptr::from_ref(table.entities());
            let column = table.column_mut(id)?;
            // SAFETY: `entities` and the column are distinct fields of the
            // table, and the column holds `T` with one item per row.
            let (values, ticks) = unsafe { column.as_mut_slice_ticked::<T>() };
            Some(
                unsafe { &*entities }
                    .iter()
                    .zip(values.iter_mut().zip(ticks))
                    .map(|(entity, (value, tick))| (entity, Mut::new(value, Some(tick)))),
            )
        });
        dense.flatten().chain(
            self.map
                .iter_mut()
                .map(|(entity, value)| (entity, Mut::new(value, None))),
        )
    }

    /// The column holding `entity`'s value, if it is in one.
    fn column_of(&self, entity: Entity) -> Option<&crate::storage::Column> {
        let (table, _) = self.entity_at.get(entity)?;
        self.tables.get(table)?.column(self.id)
    }
}

/// Every table holding a column for `id`, with how many rows it has.
fn columns(tables: &Tables, id: StorageId) -> impl Iterator<Item = (&Table, usize)> {
    tables
        .iter()
        .filter_map(move |table| table.column(id).map(|column| (table, column.len())))
}

/// Every `(entity, value)` pair held in a column, table by table.
fn rows<'a, T: Component>(
    tables: &'a Tables,
    id: StorageId,
) -> impl Iterator<Item = (&'a Entity, &'a T)> {
    tables
        .iter()
        .filter_map(move |table| {
            let column = table.column(id)?;
            // SAFETY: the column was built for the component `id` names, and
            // holds one item per row of this table.
            Some(
                table
                    .entities()
                    .iter()
                    .zip(unsafe { column.as_slice::<T>() }),
            )
        })
        .flatten()
}

/// Every `(entity, value, change tick)` held in a column, table by table.
fn ticked_rows<'a, T: Component>(
    tables: &'a Tables,
    id: StorageId,
) -> impl Iterator<Item = (&'a Entity, &'a T, u64)> {
    tables
        .iter()
        .filter_map(move |table| {
            let column = table.column(id)?;
            // SAFETY: as `rows`.
            let values = unsafe { column.as_slice::<T>() };
            Some(
                table
                    .entities()
                    .iter()
                    .zip(values)
                    .enumerate()
                    .map(move |(row, (entity, value))| (entity, value, column.changed_at(row))),
            )
        })
        .flatten()
}

/// A value handed out by [`ComponentViewMut::iter_mut`]: reading is free, writing stamps the row's
/// change tick (#1870), the way Bevy's `Mut` does.
pub struct Mut<'a, T> {
    value: &'a mut T,
    tick: Option<&'a std::sync::atomic::AtomicU64>,
}

impl<'a, T> Mut<'a, T> {
    fn new(value: &'a mut T, tick: Option<&'a std::sync::atomic::AtomicU64>) -> Self {
        Self { value, tick }
    }

    /// The value as a plain `&mut`, counted as a write.
    pub fn into_inner(self) -> &'a mut T {
        stamp(self.tick);
        self.value
    }

    /// Writes `value` only when it differs, so recomputing an unchanged value marks nothing.
    pub fn set_if_neq(&mut self, value: T)
    where
        T: PartialEq,
    {
        if *self.value != value {
            stamp(self.tick);
            *self.value = value;
        }
    }
}

impl<T> std::ops::Deref for Mut<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.value
    }
}

impl<T> std::ops::DerefMut for Mut<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        stamp(self.tick);
        self.value
    }
}

fn stamp(tick: Option<&std::sync::atomic::AtomicU64>) {
    if let Some(tick) = tick {
        tick.store(
            kooch_core::change_tick::now(),
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}
