//! A `Type.field` a binding reads, resolved to its component type once: the per-frame read is a
//! lookup by `TypeId`, not a scan of every registered name.

use std::any::TypeId;

use kooch_ecs::component::ComponentRegistry;

#[derive(Debug, Clone, PartialEq)]
pub struct FieldPath {
    /// `Type` as written: a short or a full name.
    pub component: Box<str>,
    pub field: Box<str>,
    resolved: Option<TypeId>,
    /// The registry's type count when `resolved` was found; a code reload grows it.
    seen: usize,
}

impl FieldPath {
    /// `Type.field`; the type may be a full path, so the field is what follows the last dot.
    pub fn parse(path: &str) -> Option<Self> {
        let (component, field) = path.trim().rsplit_once('.')?;
        Some(Self {
            component: component.into(),
            field: field.into(),
            resolved: None,
            seen: usize::MAX,
        })
    }

    /// The component's type, scanned for only when the registry changed since the last time.
    /// A path that names nothing warns then, not every frame.
    pub fn resolve(&mut self, registry: &ComponentRegistry) -> Option<TypeId> {
        let count = registry.type_count();
        if count == self.seen {
            return self.resolved;
        }
        self.seen = count;
        self.resolved = component_id(registry, &self.component);
        let Some(id) = self.resolved else {
            tracing::warn!(
                "a binding names `{}`, which is no reflected component",
                self.component
            );
            return None;
        };
        let known = registry
            .reflect_field_metas(&id)
            .is_some_and(|metas| metas.iter().any(|meta| meta.name == &*self.field));
        if !known {
            tracing::warn!(
                "a binding names `{}.{}`, which is no field",
                self.component,
                self.field
            );
            self.resolved = None;
        }
        self.resolved
    }
}

/// A reflected component by its name as written: `Hud` or its full path.
fn component_id(registry: &ComponentRegistry, name: &str) -> Option<TypeId> {
    registry
        .reflected_type_names()
        .into_iter()
        .find(|(_, full)| *full == name || full.rsplit("::").next() == Some(name))
        .map(|(id, _)| id)
}
