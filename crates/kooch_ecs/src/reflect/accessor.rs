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
///
/// Stored in [`ComponentRegistry`] by `TypeId`, allowing the editor and
/// other runtime systems to inspect/modify components without knowing `T`.
///
/// This trait is `pub(crate)` — external code uses the
/// [`ComponentRegistry`] reflection API instead of calling accessors directly.
pub(crate) trait ReflectAccessor: Send + Sync {
    /// Returns field metadata for the component type.
    fn fields(&self) -> &'static [FieldMeta];

    /// Reads all field values of the component **at** `value`.
    ///
    /// 🔴 A raw address rather than a storage, so reflection stops caring
    /// *where* the component lives. It used to take `&dyn AnyStorage` and
    /// look the entity up itself — which meant the inspector, scene
    /// saving, undo and the remote mirror could only ever see components
    /// held in the per-type map. A value that moved to a table column
    /// would have gone missing from all four, silently. See #891.
    ///
    /// # Safety
    ///
    /// `value` must point to a live component of this accessor's type.
    unsafe fn read_fields(&self, value: *const u8) -> Vec<(String, ReflectValue)>;

    /// Sets a single field on the component **at** `value`.
    ///
    /// # Safety
    ///
    /// `value` must point to a live component of this accessor's type, and
    /// the caller must hold exclusive access to it.
    unsafe fn write_field(
        &self,
        value: *mut u8,
        field: &str,
        new: ReflectValue,
    ) -> Result<(), ReflectError>;

    /// Creates a boxed default instance (for spawning).
    fn default_value(&self) -> Box<dyn std::any::Any + Send + Sync>;

    /// The field values a freshly-constructed component would have.
    ///
    /// Read off the type's own `reflect_default` rather than synthesised
    /// from field *kinds*: a component whose default sets `visible: true`
    /// has to arrive that way, and a zero-per-kind table would disagree
    /// with what actually spawning it gives. Reading it needs `T`, which
    /// only exists inside this impl — [`Self::default_value`] hands back a
    /// `Box<dyn Any>` that a caller cannot look into.
    ///
    /// Used to add a component to a prefab, where there is no entity to
    /// insert one on and then read back.
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
///
/// Handles the unsafe downcast from `*const u8` / `*mut u8` to `&T` / `&mut T`
/// internally, keeping the public API safe.
///
/// Stores a closure for inserting defaults so the concrete component type
/// is captured once, at registration, rather than at every insert.
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
        // SAFETY: the caller guarantees `value` points to a live `T` and
        // holds exclusive access. Every storage is writable now that the
        // GPU-backed, read-only kind is gone (#603); `ReflectError::ReadOnly`
        // survives for types that refuse a write in their own `reflect_set`,
        // such as `Parent`.
        let component = unsafe { &mut *value.cast::<T>() };
        component.reflect_set(field, new)
    }

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
