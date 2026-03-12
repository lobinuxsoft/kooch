//! Component reflection system.
//!
//! Provides runtime introspection and modification of component fields
//! without knowing the concrete type at compile time. This enables the
//! editor inspector, serialization, and scripting systems to work with
//! components generically.
//!
//! # Architecture
//!
//! - [`Reflect`] — trait implemented by components that expose their fields.
//! - [`FieldMeta`] — static metadata for a single field (name, type, kind).
//! - [`FieldKind`] — discriminant for supported field types.
//! - [`ReflectValue`] — type-erased field value for get/set operations.
//! - [`ReflectAccessor`] — bridge between type-erased [`AnyStorage`] and typed [`Reflect`].
//! - [`TypedReflectAccessor`] — concrete accessor that does the downcast internally.

use std::fmt;

/// Metadata describing a single field of a reflected component.
#[derive(Debug, Clone, Copy)]
pub struct FieldMeta {
    /// Field name (e.g. `"hp"`, `"position"`).
    pub name: &'static str,
    /// Full type name (e.g. `"f32"`, `"glam::Vec3"`).
    pub type_name: &'static str,
    /// Discriminant for the field's value type.
    pub kind: FieldKind,
}

/// Discriminant for supported reflected field types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    F32,
    F64,
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    Bool,
    String,
    Vec2,
    Vec3,
    Vec4,
    Quat,
    Mat4,
    /// Struct that also implements [`Reflect`].
    Nested,
}

/// Type-erased value for getting and setting reflected fields.
#[derive(Debug, Clone, PartialEq)]
pub enum ReflectValue {
    F32(f32),
    F64(f64),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    Bool(bool),
    String(String),
    Vec2(glam::Vec2),
    Vec3(glam::Vec3),
    Vec4(glam::Vec4),
    Quat(glam::Quat),
    Mat4(glam::Mat4),
}

impl ReflectValue {
    /// Returns the [`FieldKind`] that matches this value variant.
    pub fn kind(&self) -> FieldKind {
        match self {
            Self::F32(_) => FieldKind::F32,
            Self::F64(_) => FieldKind::F64,
            Self::U8(_) => FieldKind::U8,
            Self::U16(_) => FieldKind::U16,
            Self::U32(_) => FieldKind::U32,
            Self::U64(_) => FieldKind::U64,
            Self::I8(_) => FieldKind::I8,
            Self::I16(_) => FieldKind::I16,
            Self::I32(_) => FieldKind::I32,
            Self::I64(_) => FieldKind::I64,
            Self::Bool(_) => FieldKind::Bool,
            Self::String(_) => FieldKind::String,
            Self::Vec2(_) => FieldKind::Vec2,
            Self::Vec3(_) => FieldKind::Vec3,
            Self::Vec4(_) => FieldKind::Vec4,
            Self::Quat(_) => FieldKind::Quat,
            Self::Mat4(_) => FieldKind::Mat4,
        }
    }
}

impl fmt::Display for ReflectValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::F32(v) => write!(f, "{v}"),
            Self::F64(v) => write!(f, "{v}"),
            Self::U8(v) => write!(f, "{v}"),
            Self::U16(v) => write!(f, "{v}"),
            Self::U32(v) => write!(f, "{v}"),
            Self::U64(v) => write!(f, "{v}"),
            Self::I8(v) => write!(f, "{v}"),
            Self::I16(v) => write!(f, "{v}"),
            Self::I32(v) => write!(f, "{v}"),
            Self::I64(v) => write!(f, "{v}"),
            Self::Bool(v) => write!(f, "{v}"),
            Self::String(v) => write!(f, "{v}"),
            Self::Vec2(v) => write!(f, "({}, {})", v.x, v.y),
            Self::Vec3(v) => write!(f, "({}, {}, {})", v.x, v.y, v.z),
            Self::Vec4(v) => write!(f, "({}, {}, {}, {})", v.x, v.y, v.z, v.w),
            Self::Quat(v) => write!(f, "({}, {}, {}, {})", v.x, v.y, v.z, v.w),
            Self::Mat4(_) => write!(f, "[Mat4]"),
        }
    }
}

/// Errors that can occur during reflection operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReflectError {
    /// The requested field name does not exist on this component.
    FieldNotFound(String),
    /// The provided value type does not match the field's expected type.
    TypeMismatch {
        field: String,
        expected: FieldKind,
        got: FieldKind,
    },
    /// The component does not exist for the given entity.
    ComponentNotFound,
    /// The storage does not support mutable access (e.g. GPU from CPU).
    ReadOnly,
}

impl fmt::Display for ReflectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FieldNotFound(name) => write!(f, "field not found: {name}"),
            Self::TypeMismatch {
                field,
                expected,
                got,
            } => write!(f, "type mismatch on field '{field}': expected {expected:?}, got {got:?}"),
            Self::ComponentNotFound => write!(f, "component not found for entity"),
            Self::ReadOnly => write!(f, "storage is read-only from CPU"),
        }
    }
}

impl std::error::Error for ReflectError {}

/// Runtime introspection for component types.
///
/// Implement this trait to expose a component's fields to the editor
/// inspector, serialization, and scripting systems.
///
/// # Example
///
/// ```ignore
/// struct Health {
///     hp: u32,
///     max_hp: u32,
/// }
///
/// impl Reflect for Health {
///     fn reflect_fields(&self) -> &'static [FieldMeta] {
///         &[
///             FieldMeta { name: "hp", type_name: "u32", kind: FieldKind::U32 },
///             FieldMeta { name: "max_hp", type_name: "u32", kind: FieldKind::U32 },
///         ]
///     }
///
///     fn reflect_get(&self, field: &str) -> Option<ReflectValue> {
///         match field {
///             "hp" => Some(ReflectValue::U32(self.hp)),
///             "max_hp" => Some(ReflectValue::U32(self.max_hp)),
///             _ => None,
///         }
///     }
///
///     fn reflect_set(&mut self, field: &str, value: ReflectValue) -> Result<(), ReflectError> {
///         match field {
///             "hp" => match value {
///                 ReflectValue::U32(v) => { self.hp = v; Ok(()) }
///                 other => Err(ReflectError::TypeMismatch {
///                     field: "hp".into(), expected: FieldKind::U32, got: other.kind(),
///                 }),
///             },
///             "max_hp" => match value {
///                 ReflectValue::U32(v) => { self.max_hp = v; Ok(()) }
///                 other => Err(ReflectError::TypeMismatch {
///                     field: "max_hp".into(), expected: FieldKind::U32, got: other.kind(),
///                 }),
///             },
///             _ => Err(ReflectError::FieldNotFound(field.into())),
///         }
///     }
///
///     fn reflect_default() -> Self {
///         Health { hp: 100, max_hp: 100 }
///     }
/// }
/// ```
pub trait Reflect: Send + Sync + 'static {
    /// Returns static metadata for all fields of this type.
    fn reflect_fields(&self) -> &'static [FieldMeta];

    /// Gets the value of a field by name.
    fn reflect_get(&self, field: &str) -> Option<ReflectValue>;

    /// Sets the value of a field by name.
    fn reflect_set(&mut self, field: &str, value: ReflectValue) -> Result<(), ReflectError>;

    /// Creates an instance with default values (for editor "Add Component").
    fn reflect_default() -> Self
    where
        Self: Sized;
}

// ---------------------------------------------------------------------------
// ReflectAccessor — type-erased bridge
// ---------------------------------------------------------------------------

use crate::component::cpu_storage::ComponentStorage;
use crate::component::gpu_storage::GpuComponentStorage;
use crate::component::traits::{AnyStorage, Component, GpuComponent};
use crate::entity::Entity;

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

    /// Reads all field values for an entity's component.
    fn get_fields(
        &self,
        storage: &dyn AnyStorage,
        entity: Entity,
    ) -> Option<Vec<(String, ReflectValue)>>;

    /// Sets a single field on an entity's component.
    fn set_field(
        &self,
        storage: &mut dyn AnyStorage,
        entity: Entity,
        field: &str,
        value: ReflectValue,
    ) -> Result<(), ReflectError>;

    /// Creates a boxed default instance (for spawning).
    fn default_value(&self) -> Box<dyn std::any::Any + Send + Sync>;

    /// Inserts a default instance into the storage for the given entity.
    ///
    /// Returns `true` if inserted successfully.
    fn insert_default_into(&self, storage: &mut dyn AnyStorage, entity: Entity) -> bool;
}

/// Concrete [`ReflectAccessor`] for a component type `T: Reflect`.
///
/// Handles the unsafe downcast from `*const u8` / `*mut u8` to `&T` / `&mut T`
/// internally, keeping the public API safe.
///
/// Stores a closure for inserting defaults, since the insert strategy
/// differs between CPU and GPU storages.
pub(crate) struct TypedReflectAccessor<T: Reflect> {
    inserter: Box<dyn Fn(&mut dyn AnyStorage, Entity) -> bool + Send + Sync>,
    _marker: std::marker::PhantomData<T>,
}

impl<T: Component + Reflect> TypedReflectAccessor<T> {
    /// Creates an accessor for a CPU component.
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

impl<T: GpuComponent + Reflect> TypedReflectAccessor<T> {
    /// Creates an accessor for a GPU component.
    pub(crate) fn new_gpu() -> Self {
        Self {
            inserter: Box::new(|storage, entity| {
                if let Some(gpu) = storage.as_any_mut().downcast_mut::<GpuComponentStorage<T>>() {
                    gpu.insert(entity, T::reflect_default());
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

    fn get_fields(
        &self,
        storage: &dyn AnyStorage,
        entity: Entity,
    ) -> Option<Vec<(String, ReflectValue)>> {
        let ptr = storage.get_ptr(entity)?;
        // SAFETY: The pointer comes from a storage that was registered with
        // TypeId::of::<T>(), so the data behind it is a valid `T`.
        let component = unsafe { &*(ptr as *const T) };
        let fields = component.reflect_fields();
        let values = fields
            .iter()
            .filter_map(|meta| {
                component
                    .reflect_get(meta.name)
                    .map(|v| (meta.name.to_owned(), v))
            })
            .collect();
        Some(values)
    }

    fn set_field(
        &self,
        storage: &mut dyn AnyStorage,
        entity: Entity,
        field: &str,
        value: ReflectValue,
    ) -> Result<(), ReflectError> {
        if !storage.is_mutable() {
            return Err(ReflectError::ReadOnly);
        }
        let ptr = storage
            .get_mut_ptr(entity)
            .ok_or(ReflectError::ComponentNotFound)?;
        // SAFETY: Same TypeId guarantee as get_fields, and is_mutable() check
        // ensures we're allowed to write.
        let component = unsafe { &mut *(ptr as *mut T) };
        component.reflect_set(field, value)
    }

    fn default_value(&self) -> Box<dyn std::any::Any + Send + Sync> {
        Box::new(T::reflect_default())
    }

    fn insert_default_into(&self, storage: &mut dyn AnyStorage, entity: Entity) -> bool {
        (self.inserter)(storage, entity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Test component with manual Reflect impl -----------------------------

    #[derive(Debug, Clone, PartialEq)]
    struct Health {
        hp: u32,
        max_hp: u32,
    }

    impl crate::component::traits::Component for Health {}

    impl Reflect for Health {
        fn reflect_fields(&self) -> &'static [FieldMeta] {
            static FIELDS: &[FieldMeta] = &[
                FieldMeta {
                    name: "hp",
                    type_name: "u32",
                    kind: FieldKind::U32,
                },
                FieldMeta {
                    name: "max_hp",
                    type_name: "u32",
                    kind: FieldKind::U32,
                },
            ];
            FIELDS
        }

        fn reflect_get(&self, field: &str) -> Option<ReflectValue> {
            match field {
                "hp" => Some(ReflectValue::U32(self.hp)),
                "max_hp" => Some(ReflectValue::U32(self.max_hp)),
                _ => None,
            }
        }

        fn reflect_set(&mut self, field: &str, value: ReflectValue) -> Result<(), ReflectError> {
            match field {
                "hp" => match value {
                    ReflectValue::U32(v) => {
                        self.hp = v;
                        Ok(())
                    }
                    other => Err(ReflectError::TypeMismatch {
                        field: "hp".into(),
                        expected: FieldKind::U32,
                        got: other.kind(),
                    }),
                },
                "max_hp" => match value {
                    ReflectValue::U32(v) => {
                        self.max_hp = v;
                        Ok(())
                    }
                    other => Err(ReflectError::TypeMismatch {
                        field: "max_hp".into(),
                        expected: FieldKind::U32,
                        got: other.kind(),
                    }),
                },
                _ => Err(ReflectError::FieldNotFound(field.into())),
            }
        }

        fn reflect_default() -> Self {
            Health {
                hp: 100,
                max_hp: 100,
            }
        }
    }

    // -- GPU component with Reflect ------------------------------------------

    #[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
    #[repr(C)]
    struct Position {
        x: f32,
        y: f32,
        z: f32,
        _pad: f32,
    }

    impl crate::component::traits::GpuComponent for Position {}

    impl Reflect for Position {
        fn reflect_fields(&self) -> &'static [FieldMeta] {
            static FIELDS: &[FieldMeta] = &[
                FieldMeta {
                    name: "x",
                    type_name: "f32",
                    kind: FieldKind::F32,
                },
                FieldMeta {
                    name: "y",
                    type_name: "f32",
                    kind: FieldKind::F32,
                },
                FieldMeta {
                    name: "z",
                    type_name: "f32",
                    kind: FieldKind::F32,
                },
            ];
            FIELDS
        }

        fn reflect_get(&self, field: &str) -> Option<ReflectValue> {
            match field {
                "x" => Some(ReflectValue::F32(self.x)),
                "y" => Some(ReflectValue::F32(self.y)),
                "z" => Some(ReflectValue::F32(self.z)),
                _ => None,
            }
        }

        fn reflect_set(&mut self, field: &str, value: ReflectValue) -> Result<(), ReflectError> {
            match field {
                "x" => match value {
                    ReflectValue::F32(v) => {
                        self.x = v;
                        Ok(())
                    }
                    other => Err(ReflectError::TypeMismatch {
                        field: "x".into(),
                        expected: FieldKind::F32,
                        got: other.kind(),
                    }),
                },
                "y" => match value {
                    ReflectValue::F32(v) => {
                        self.y = v;
                        Ok(())
                    }
                    other => Err(ReflectError::TypeMismatch {
                        field: "y".into(),
                        expected: FieldKind::F32,
                        got: other.kind(),
                    }),
                },
                "z" => match value {
                    ReflectValue::F32(v) => {
                        self.z = v;
                        Ok(())
                    }
                    other => Err(ReflectError::TypeMismatch {
                        field: "z".into(),
                        expected: FieldKind::F32,
                        got: other.kind(),
                    }),
                },
                _ => Err(ReflectError::FieldNotFound(field.into())),
            }
        }

        fn reflect_default() -> Self {
            Position {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                _pad: 0.0,
            }
        }
    }

    // -- Reflect trait tests --------------------------------------------------

    #[test]
    fn reflect_fields_returns_metadata() {
        let h = Health { hp: 50, max_hp: 100 };
        let fields = h.reflect_fields();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "hp");
        assert_eq!(fields[0].kind, FieldKind::U32);
        assert_eq!(fields[1].name, "max_hp");
    }

    #[test]
    fn reflect_get_returns_values() {
        let h = Health { hp: 42, max_hp: 100 };
        assert_eq!(h.reflect_get("hp"), Some(ReflectValue::U32(42)));
        assert_eq!(h.reflect_get("max_hp"), Some(ReflectValue::U32(100)));
        assert_eq!(h.reflect_get("nonexistent"), None);
    }

    #[test]
    fn reflect_set_modifies_values() {
        let mut h = Health { hp: 50, max_hp: 100 };
        h.reflect_set("hp", ReflectValue::U32(75)).unwrap();
        assert_eq!(h.hp, 75);
    }

    #[test]
    fn reflect_set_type_mismatch() {
        let mut h = Health { hp: 50, max_hp: 100 };
        let err = h.reflect_set("hp", ReflectValue::F32(1.0)).unwrap_err();
        assert_eq!(
            err,
            ReflectError::TypeMismatch {
                field: "hp".into(),
                expected: FieldKind::U32,
                got: FieldKind::F32,
            }
        );
    }

    #[test]
    fn reflect_set_field_not_found() {
        let mut h = Health { hp: 50, max_hp: 100 };
        let err = h.reflect_set("nope", ReflectValue::U32(1)).unwrap_err();
        assert_eq!(err, ReflectError::FieldNotFound("nope".into()));
    }

    #[test]
    fn reflect_default_creates_instance() {
        let h = Health::reflect_default();
        assert_eq!(h.hp, 100);
        assert_eq!(h.max_hp, 100);
    }

    #[test]
    fn reflect_value_kind() {
        assert_eq!(ReflectValue::F32(1.0).kind(), FieldKind::F32);
        assert_eq!(ReflectValue::U32(1).kind(), FieldKind::U32);
        assert_eq!(ReflectValue::Bool(true).kind(), FieldKind::Bool);
        assert_eq!(
            ReflectValue::Vec3(glam::Vec3::ZERO).kind(),
            FieldKind::Vec3
        );
    }

    #[test]
    fn reflect_value_display() {
        assert_eq!(format!("{}", ReflectValue::U32(42)), "42");
        assert_eq!(format!("{}", ReflectValue::F32(3.14)), "3.14");
        assert_eq!(format!("{}", ReflectValue::Bool(true)), "true");
        assert_eq!(
            format!("{}", ReflectValue::Vec3(glam::Vec3::new(1.0, 2.0, 3.0))),
            "(1, 2, 3)"
        );
    }

    // -- TypedReflectAccessor tests ------------------------------------------

    #[test]
    fn accessor_fields_returns_metadata() {
        let accessor = TypedReflectAccessor::<Health>::new_cpu();
        let fields = accessor.fields();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "hp");
    }

    #[test]
    fn accessor_get_fields_from_cpu_storage() {
        use crate::component::cpu_storage::ComponentStorage;
        use crate::entity::Entity;

        let mut storage = ComponentStorage::<Health>::new();
        let e = Entity::new(0, 0);
        storage.insert(e, Health { hp: 42, max_hp: 100 });

        let accessor = TypedReflectAccessor::<Health>::new_cpu();
        let fields = accessor.get_fields(&storage, e).unwrap();

        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0], ("hp".to_owned(), ReflectValue::U32(42)));
        assert_eq!(fields[1], ("max_hp".to_owned(), ReflectValue::U32(100)));
    }

    #[test]
    fn accessor_get_fields_missing_entity() {
        use crate::component::cpu_storage::ComponentStorage;
        use crate::entity::Entity;

        let storage = ComponentStorage::<Health>::new();
        let e = Entity::new(99, 0);

        let accessor = TypedReflectAccessor::<Health>::new_cpu();
        assert!(accessor.get_fields(&storage, e).is_none());
    }

    #[test]
    fn accessor_set_field_on_cpu_storage() {
        use crate::component::cpu_storage::ComponentStorage;
        use crate::entity::Entity;

        let mut storage = ComponentStorage::<Health>::new();
        let e = Entity::new(0, 0);
        storage.insert(e, Health { hp: 50, max_hp: 100 });

        let accessor = TypedReflectAccessor::<Health>::new_cpu();
        accessor
            .set_field(&mut storage, e, "hp", ReflectValue::U32(75))
            .unwrap();

        assert_eq!(storage.get(e).unwrap().hp, 75);
    }

    #[test]
    fn accessor_set_field_readonly_storage() {
        use crate::component::gpu_storage::GpuComponentStorage;
        use crate::entity::Entity;

        let mut storage = GpuComponentStorage::<Position>::new("test");
        let e = Entity::new(0, 0);
        storage.insert(e, Position::reflect_default());

        let accessor = TypedReflectAccessor::<Position>::new_gpu();
        let err = accessor
            .set_field(&mut storage, e, "x", ReflectValue::F32(5.0))
            .unwrap_err();
        assert_eq!(err, ReflectError::ReadOnly);
    }

    #[test]
    fn accessor_default_value() {
        let accessor = TypedReflectAccessor::<Health>::new_cpu();
        let boxed = accessor.default_value();
        let health = boxed.downcast::<Health>().unwrap();
        assert_eq!(health.hp, 100);
        assert_eq!(health.max_hp, 100);
    }

    // -- Registry integration tests ------------------------------------------

    #[test]
    fn registry_register_cpu_reflected() {
        use crate::component::registry::ComponentRegistry;

        let mut registry = ComponentRegistry::new();
        registry.register_cpu_reflected::<Health>();

        assert!(registry.has_reflector(&std::any::TypeId::of::<Health>()));
        let metas = registry
            .reflect_field_metas(&std::any::TypeId::of::<Health>())
            .unwrap();
        assert_eq!(metas.len(), 2);
        assert_eq!(metas[0].name, "hp");
    }

    #[test]
    fn registry_reflect_get_fields() {
        use crate::component::registry::ComponentRegistry;
        use crate::entity::Entity;

        let mut registry = ComponentRegistry::new();
        registry.register_cpu_reflected::<Health>();
        let e = Entity::new(0, 0);
        registry
            .get_cpu_mut::<Health>()
            .unwrap()
            .insert(e, Health { hp: 42, max_hp: 100 });

        let fields = registry
            .reflect_get_fields(&std::any::TypeId::of::<Health>(), e)
            .unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0], ("hp".to_owned(), ReflectValue::U32(42)));
    }

    #[test]
    fn registry_reflect_set_field() {
        use crate::component::registry::ComponentRegistry;
        use crate::entity::Entity;

        let mut registry = ComponentRegistry::new();
        registry.register_cpu_reflected::<Health>();
        let e = Entity::new(0, 0);
        registry
            .get_cpu_mut::<Health>()
            .unwrap()
            .insert(e, Health { hp: 50, max_hp: 100 });

        registry
            .reflect_set_field(
                &std::any::TypeId::of::<Health>(),
                e,
                "hp",
                ReflectValue::U32(75),
            )
            .unwrap();

        let health = registry.get_cpu::<Health>().unwrap().get(e).unwrap();
        assert_eq!(health.hp, 75);
    }

    #[test]
    fn registry_reflected_type_ids() {
        use crate::component::registry::ComponentRegistry;

        let mut registry = ComponentRegistry::new();
        registry.register_cpu_reflected::<Health>();
        registry.register_gpu::<Position>("pos"); // Not reflected

        let ids = registry.reflected_type_ids();
        assert_eq!(ids.len(), 1);
        assert!(ids.contains(&std::any::TypeId::of::<Health>()));
    }

    #[test]
    fn registry_non_reflected_has_no_reflector() {
        use crate::component::registry::ComponentRegistry;

        let mut registry = ComponentRegistry::new();
        registry.register_cpu::<Health>(); // Without reflection

        assert!(!registry.has_reflector(&std::any::TypeId::of::<Health>()));
        assert!(registry
            .reflect_get_fields(&std::any::TypeId::of::<Health>(), Entity::new(0, 0))
            .is_none());
    }

    // -- Commands integration tests ------------------------------------------

    #[test]
    fn commands_insert_reflected() {
        use crate::allocator::EntityAllocator;
        use crate::archetype_registry::ArchetypeRegistry;
        use crate::commands::Commands;
        use crate::component::registry::ComponentRegistry;
        use crate::query::{AccessTracker, Query};
        use ome_core::resource::Resources;

        let mut resources = Resources::new();
        resources.insert(EntityAllocator::new());
        resources.insert(ComponentRegistry::new());
        resources.insert(ArchetypeRegistry::new());
        resources.insert(AccessTracker::new());

        let mut commands = Commands::new();
        let entity = commands
            .spawn(&mut resources)
            .insert_reflected(Health { hp: 42, max_hp: 100 })
            .id();
        commands.apply(&mut resources);

        // Verify component works via query.
        let query = Query::<&Health>::new(&resources);
        assert_eq!(query.get(entity).unwrap().hp, 42);

        // Verify reflector was registered.
        let registry = resources.get::<ComponentRegistry>().unwrap();
        assert!(registry.has_reflector(&std::any::TypeId::of::<Health>()));

        // Verify reflected get works.
        let fields = registry
            .reflect_get_fields(&std::any::TypeId::of::<Health>(), entity)
            .unwrap();
        assert_eq!(fields[0], ("hp".to_owned(), ReflectValue::U32(42)));
    }
}
