//! Component trait definitions.

use std::any::Any;

use crate::entity::Entity;
use crate::storage::Column;

/// Marker for CPU-only components.
///
/// Any `Send + Sync + 'static` type can implement this.
pub trait Component: Send + Sync + 'static {}

/// Type-erased interface for component storages.
pub(crate) trait AnyStorage: Send + Sync + 'static {
    /// Removes the component for `entity`, if present.
    fn remove_entity(&mut self, entity: Entity);

    /// Returns `self` as `&dyn Any` for downcasting.
    fn as_any(&self) -> &dyn Any;

    /// Returns `self` as `&mut dyn Any` for downcasting.
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// Returns an immutable raw pointer to the component data for `entity`.
    ///
    /// Returns `None` if the entity does not have this component.
    fn get_ptr(&self, entity: Entity) -> Option<*const u8>;

    /// Returns a mutable raw pointer to the component data for `entity`.
    ///
    /// Returns `None` for read-only storages (e.g. GPU components from CPU side).
    fn get_mut_ptr(&mut self, entity: Entity) -> Option<*mut u8>;

    /// Moves `entity`'s value out of here and appends it to `column`.
    ///
    /// 🔴 This is what makes #891 possible without rewriting every insert site. The sites write the
    /// value and *then* change the archetype, and at the archetype site only a `TypeId` is in hand
    /// — but `Column::push<T>` needs the concrete type. Here the type is known, because the impl is
    /// monomorphised: the funnel can promote a value it cannot name.
    ///
    /// `false` when there was nothing to move, which leaves the column one short — so the caller
    /// has to decide the whole move is off, not place the entity anyway.
    fn take_into(&mut self, entity: Entity, column: &mut Column) -> bool;

    /// Moves the value at `row` of `column` back out of the column and parks it here.
    ///
    /// The reverse of [`Self::take_into`], and the reason a half-finished move can always be undone
    /// rather than left pointing at another entity's row.
    fn put_from(&mut self, entity: Entity, column: &mut Column, row: usize) -> bool;

    /// Whether a value for `entity` is parked here.
    fn holds(&self, entity: Entity) -> bool;
}
