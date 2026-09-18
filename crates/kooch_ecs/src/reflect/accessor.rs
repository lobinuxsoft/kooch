// ---------------------------------------------------------------------------
// ReflectAccessor — type-erased bridge
// ---------------------------------------------------------------------------

use crate::component::cpu_storage::ComponentStorage;
use crate::component::traits::{AnyStorage, Component};
use crate::entity::Entity;

use super::error::ReflectError;
use super::field::{FieldMeta, InspectorVisibility};
use super::trait_def::Reflect;
use super::value::ReflectValue;

/// Type-erased adapter connecting [`AnyStorage`] with [`Reflect`].
pub(crate) trait ReflectAccessor: Send + Sync {
    /// Returns field metadata for the component type.
    fn fields(&self) -> &'static [FieldMeta];

    /// Reads all field values of the component **at** `value`.
    unsafe fn read_fields(&self, value: *const u8) -> Vec<(String, ReflectValue)>;

    /// Sets a single field on the component **at** `value`.
    unsafe fn write_field(
        &self,
        value: *mut u8,
        field: &str,
        new: ReflectValue,
    ) -> Result<(), ReflectError>;

    /// Creates a boxed default instance (for spawning).
    #[cfg(test)]
    fn default_value(&self) -> Box<dyn std::any::Any + Send + Sync>;

    /// The field values a freshly-constructed component would have.
    fn default_fields(&self) -> Vec<(String, ReflectValue)>;

    /// Inserts a default instance into the storage for the given entity.
    ///
    /// Returns `true` if inserted successfully.
    fn insert_default_into(&self, storage: &mut dyn AnyStorage, entity: Entity) -> bool;

    /// Returns the inspector visibility for this component type.
    fn inspector_visibility(&self) -> InspectorVisibility;

    /// Returns the editor category (if any) for grouping in the menu.
    fn category(&self) -> Option<&'static str>;
}

/// Concrete [`ReflectAccessor`] for a component type `T: Reflect`.
pub(crate) struct TypedReflectAccessor<T: Reflect> {
    inserter: Box<dyn Fn(&mut dyn AnyStorage, Entity) -> bool + Send + Sync>,
    _marker: std::marker::PhantomData<T>,
}

impl<T: Component + Reflect> TypedReflectAccessor<T> {
    /// Creates an accessor for a component.
    pub(crate) fn new_cpu() -> Self {
        Self {
            inserter: Box::new(|storage, entity| {
                if let Some(cpu) = storage.as_any_mut().downcast_mut::<ComponentStorage<T>>() {
                    cpu.insert(entity, T::reflect_default());
                    true
                } else {
                    false
                }
            }),
            _marker: std::marker::PhantomData,
        }
    }
}

impl<T: Reflect> ReflectAccessor for TypedReflectAccessor<T> {
    fn fields(&self) -> &'static [FieldMeta] {
        T::reflect_default().reflect_fields()
    }

    fn default_fields(&self) -> Vec<(String, ReflectValue)> {
        let component = T::reflect_default();
        component
            .reflect_fields()
            .iter()
            .filter_map(|meta| {
                component
                    .reflect_get(meta.name)
                    .map(|value| (meta.name.to_owned(), value))
            })
            .collect()
    }

    unsafe fn read_fields(&self, value: *const u8) -> Vec<(String, ReflectValue)> {
        // SAFETY: the caller guarantees `value` points to a live `T`.
        let component = unsafe { &*value.cast::<T>() };
        component
            .reflect_fields()
            .iter()
            .filter_map(|meta| {
                component
                    .reflect_get(meta.name)
                    .map(|v| (meta.name.to_owned(), v))
            })
            .collect()
    }

    unsafe fn write_field(
        &self,
        value: *mut u8,
        field: &str,
        new: ReflectValue,
    ) -> Result<(), ReflectError> {
        // SAFETY: the caller guarantees `value` points to a live `T` and holds exclusive access.
        let component = unsafe { &mut *value.cast::<T>() };
        component.reflect_set(field, new)
    }

    #[cfg(test)]
    fn default_value(&self) -> Box<dyn std::any::Any + Send + Sync> {
        Box::new(T::reflect_default())
    }

    fn insert_default_into(&self, storage: &mut dyn AnyStorage, entity: Entity) -> bool {
        (self.inserter)(storage, entity)
    }

    fn inspector_visibility(&self) -> InspectorVisibility {
        T::inspector_visibility()
    }

    fn category(&self) -> Option<&'static str> {
        T::category()
    }
}
