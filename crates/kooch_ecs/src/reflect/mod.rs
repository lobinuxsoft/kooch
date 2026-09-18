//! Component reflection system.

mod accessor;
pub mod asset_registry;
mod entity_ref;
mod error;
mod field;
mod nested;
mod trait_def;
mod value;

pub(crate) use accessor::{ReflectAccessor, TypedReflectAccessor};
pub use asset_registry::{ReflectedAssetRegistration, reflected_asset, reflected_asset_types};
pub use entity_ref::EntityRef;
pub use error::ReflectError;
pub use field::{
    FieldChoice, FieldCondition, FieldKind, FieldMeta, FieldRange, InspectorVisibility,
};
pub use nested::{list_from, list_value, struct_value};
pub use trait_def::Reflect;
pub use value::ReflectValue;

#[cfg(test)]
mod tests;
