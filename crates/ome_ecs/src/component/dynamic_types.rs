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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicType {
    /// Fully qualified type name — the identity.
    pub type_name: String,
    /// Its fields, in declaration order. Empty for a marker component.
    pub fields: Vec<DynamicField>,
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
mod tests {
    use super::*;

    fn health(source: &str) -> DynamicType {
        DynamicType {
            type_name: "my_game::Health".into(),
            fields: vec![
                DynamicField {
                    name: "current".into(),
                    kind: FieldKind::U32,
                },
                DynamicField {
                    name: "max".into(),
                    kind: FieldKind::U32,
                },
            ],
            source: source.into(),
        }
    }

    #[test]
    fn a_registered_type_is_found_by_name() {
        let mut registry = DynamicTypeRegistry::new();
        assert!(registry.register(health("game")).is_ok());

        let found = registry.get("my_game::Health").expect("registered");
        assert_eq!(found.fields.len(), 2);
        assert_eq!(found.fields[0].kind, FieldKind::U32);
        assert!(registry.contains("my_game::Health"));
        assert_eq!(registry.len(), 1);
    }

    /// A reload re-registers the same types. If that failed, reloading
    /// would be impossible.
    #[test]
    fn the_same_source_may_register_twice() {
        let mut registry = DynamicTypeRegistry::new();
        registry.register(health("game")).unwrap();

        assert!(registry.register(health("game")).is_ok());
        assert_eq!(registry.len(), 1, "must not duplicate");
    }

    /// After a reload the new definition wins — the plugin author may
    /// have added a field, and the editor must show it.
    #[test]
    fn a_changed_schema_from_the_same_source_replaces_the_old() {
        let mut registry = DynamicTypeRegistry::new();
        registry.register(health("game")).unwrap();

        let mut grown = health("game");
        grown.fields.push(DynamicField {
            name: "regen".into(),
            kind: FieldKind::F32,
        });
        registry.register(grown).unwrap();

        assert_eq!(registry.get("my_game::Health").unwrap().fields.len(), 3);
    }

    /// Two plugins claiming one name is a real collision, and the
    /// registry cannot pick a winner.
    #[test]
    fn a_different_source_is_refused_and_names_the_owner() {
        let mut registry = DynamicTypeRegistry::new();
        registry.register(health("game")).unwrap();

        let err = registry.register(health("mod")).unwrap_err();
        assert_eq!(err, "game");
        assert_eq!(registry.get("my_game::Health").unwrap().source, "game");
    }

    #[test]
    fn unloading_a_source_drops_only_its_types() {
        let mut registry = DynamicTypeRegistry::new();
        registry.register(health("game")).unwrap();
        registry
            .register(DynamicType {
                type_name: "mod::Extra".into(),
                fields: Vec::new(),
                source: "mod".into(),
            })
            .unwrap();

        assert_eq!(registry.remove_source("game"), 1);
        assert!(!registry.contains("my_game::Health"));
        assert!(registry.contains("mod::Extra"));
    }

    #[test]
    fn a_marker_type_registers() {
        let mut registry = DynamicTypeRegistry::new();
        registry
            .register(DynamicType {
                type_name: "my_game::Player".into(),
                fields: Vec::new(),
                source: "game".into(),
            })
            .unwrap();

        assert!(registry.get("my_game::Player").unwrap().fields.is_empty());
    }
}
