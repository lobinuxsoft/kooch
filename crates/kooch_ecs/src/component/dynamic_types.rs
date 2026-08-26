//! Component types this binary has no Rust type for.
//!
//! [`ComponentRegistry`](super::ComponentRegistry) is keyed by `TypeId`,
//! which a plugin's types do not have here — they were compiled into a
//! different library. So they are registered by **name**, alongside the
//! field list needed to draw and serialise them.
//!
//! This is the type-level counterpart to
//! [`DynamicComponents`](crate::dynamic_components::DynamicComponents),
//! which already stores *instances* of components this binary cannot
//! name. One says "a `my_game::Health` exists and has these fields", the
//! other says "entity 7 has one, holding these values".
//!
//! Names are the identity, so a plugin rebuilt with the same type keeps
//! its registration: re-registering an identical schema is idempotent,
//! which is what makes reload possible without wiping the world.

use std::collections::HashMap;

use crate::reflect::FieldKind;

/// One field of a dynamically registered component.
///
/// Owned, unlike `FieldMeta`, whose `&'static str` can only come from a
/// type the compiler saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicField {
    /// Field name, as shown in the Inspector and written to a scene.
    pub name: String,
    /// What the field holds.
    pub kind: FieldKind,
}

/// A component type registered by name.
///
/// Not `Eq`: the default values it carries include floats.
#[derive(Debug, Clone, PartialEq)]
pub struct DynamicType {
    /// Fully qualified type name — the identity.
    pub type_name: String,
    /// Its fields, in declaration order. Empty for a marker component.
    pub fields: Vec<DynamicField>,
    /// What a fresh one holds, straight from the type's `Default`.
    ///
    /// The editor cannot call `Default` on a type it never compiled, so
    /// this is the only way it can add the component to a prefab with the
    /// values its author chose. Empty when the plugin predates the field.
    pub defaults: Vec<(String, crate::reflect::ReflectValue)>,
    /// Who registered it, for diagnostics and for unregistering on
    /// unload.
    pub source: String,
}

/// Component types known by name rather than by `TypeId`.
///
/// Lives as a resource. The editor reads it next to the reflected
/// registry so a plugin's component appears in the Add Component menu
/// and the Inspector like any other.
#[derive(Debug, Default, Clone)]
pub struct DynamicTypeRegistry {
    by_name: HashMap<String, DynamicType>,
}

impl DynamicTypeRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a type, or confirms one that is already there.
    ///
    /// Re-registering the **same** schema from the **same** source
    /// succeeds and changes nothing — that is what a plugin reload does,
    /// and failing it would make reloading impossible. A schema that
    /// differs replaces the old one, because after a reload the new
    /// definition is the true one.
    ///
    /// Returns `Err` with the existing owner when a *different* source
    /// claims a name already taken, which is a genuine collision between
    /// two plugins and cannot be resolved here.
    pub fn register(&mut self, ty: DynamicType) -> Result<(), String> {
        if let Some(existing) = self.by_name.get(&ty.type_name)
            && existing.source != ty.source
        {
            return Err(existing.source.clone());
        }
        self.by_name.insert(ty.type_name.clone(), ty);
        Ok(())
    }

    /// Looks a type up by name.
    pub fn get(&self, type_name: &str) -> Option<&DynamicType> {
        self.by_name.get(type_name)
    }

    /// Whether a type is registered under this name.
    pub fn contains(&self, type_name: &str) -> bool {
        self.by_name.contains_key(type_name)
    }

    /// Every registered type, unordered.
    pub fn iter(&self) -> impl Iterator<Item = &DynamicType> {
        self.by_name.values()
    }

    /// How many types are registered.
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    /// Whether nothing is registered.
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    /// Drops every type registered by `source`.
    ///
    /// Called when a plugin is unloaded, so its types stop appearing in
    /// the Add Component menu. Instances already placed on entities are
    /// **not** touched: they live in `DynamicComponents` keyed by name,
    /// and discarding them would lose the user's data on a reload that
    /// is about to re-register the very same type.
    pub fn remove_source(&mut self, source: &str) -> usize {
        let before = self.by_name.len();
        self.by_name.retain(|_, ty| ty.source != source);
        before - self.by_name.len()
    }
}

#[cfg(test)]
mod tests;
