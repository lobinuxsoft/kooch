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

mod accessor;
mod error;
mod field;
mod trait_def;
mod value;

pub use field::{FieldChoice, FieldKind, FieldMeta, InspectorVisibility};
pub use value::ReflectValue;
pub use error::ReflectError;
pub use trait_def::Reflect;
pub(crate) use accessor::{ReflectAccessor, TypedReflectAccessor};

#[cfg(test)]
mod tests;
