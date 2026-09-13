//! Component trait definitions.

use std::any::Any;

use crate::entity::Entity;

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
}
