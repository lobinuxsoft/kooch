//! Component types this binary has no Rust type for.

use std::collections::HashMap;

use crate::reflect::FieldKind;

/// One field of a dynamically registered component.
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
    pub defaults: Vec<(String, crate::reflect::ReflectValue)>,
    /// Who registered it, for diagnostics and for unregistering on
    /// unload.
    pub source: String,
}

/// Component types known by name rather than by `TypeId`.
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
    pub fn remove_source(&mut self, source: &str) -> usize {
        let before = self.by_name.len();
        self.by_name.retain(|_, ty| ty.source != source);
        before - self.by_name.len()
    }
}

#[cfg(test)]
mod tests;
