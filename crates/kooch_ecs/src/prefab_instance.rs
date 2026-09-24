//! The link between an entity in a scene and the prefab it came from.

use kooch_core::Guid;

use crate::component::Component;
use crate::entity::Entity;
use crate::reflect::{
    FieldKind, FieldMeta, InspectorVisibility, Reflect, ReflectError, ReflectValue,
};

/// Separates one override from the next.
const RECORD: char = '\u{1e}';
/// Separates the parts of a single record.
const PART: char = '\u{1f}';

/// Marks an entity as an instance of a prefab, and records what has been
/// changed on it since.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PrefabInstance {
    /// The prefab this was stamped from.
    pub source: Option<Guid>,
    /// Field addresses the user has changed on this instance.
    pub overrides: String,
}

/// The field name that means "the component itself", rather than a field on it.
pub const WHOLE_COMPONENT: &str = "";

/// One field the user changed on an instance, addressed relative to the prefab rather than to the
/// world.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OverrideAddress {
    pub entity: usize,
    /// Full type path, as the document stores it.
    pub component: String,
    pub field: String,
}

/// One override: where it applies, and what it changed the value to.
#[derive(Debug, Clone, PartialEq)]
pub struct Override {
    pub address: OverrideAddress,
    /// `None` means the component was taken off this instance — a
    /// decision about presence, which has no value to carry.
    pub value: Option<ReflectValue>,
}

impl PrefabInstance {
    pub fn new(source: Guid) -> Self {
        Self {
            source: Some(source),
            overrides: String::new(),
        }
    }

    /// The addresses this instance has overridden.
    pub fn overrides(&self) -> Vec<Override> {
        self.overrides
            .split(RECORD)
            .filter(|record| !record.is_empty())
            .filter_map(|record| {
                let mut parts = record.split(PART);
                let entity = parts.next()?.parse().ok()?;
                let component = parts.next()?.to_owned();
                let field = parts.next()?.to_owned();
                // A value that will not parse costs its own record rather
                // than the whole set — the same rule as a malformed
                // address, for the same reason.
                let value = match parts.next() {
                    Some(encoded) if !encoded.is_empty() => Some(ron::from_str(encoded).ok()?),
                    _ => None,
                };
                Some(Override {
                    address: OverrideAddress {
                        entity,
                        component,
                        field,
                    },
                    value,
                })
            })
            .collect()
    }

    /// Just the addresses, for the lookups that do not care what changed.
    pub fn addresses(&self) -> Vec<OverrideAddress> {
        self.overrides().into_iter().map(|o| o.address).collect()
    }

    /// Replaces the set, in a stable order so re-saving a scene does not
    /// produce a different file for the same state.
    pub fn set_overrides(&mut self, overrides: impl IntoIterator<Item = Override>) {
        let mut sorted: Vec<Override> = overrides.into_iter().collect();
        sorted.sort_by(|a, b| {
            a.address
                .entity
                .cmp(&b.address.entity)
                .then_with(|| a.address.component.cmp(&b.address.component))
                .then_with(|| a.address.field.cmp(&b.address.field))
        });
        sorted.dedup_by(|a, b| a.address == b.address);
        self.overrides = sorted
            .iter()
            .map(|o| {
                let encoded = o
                    .value
                    .as_ref()
                    .and_then(|value| ron::to_string(value).ok())
                    .unwrap_or_default();
                format!(
                    "{}{PART}{}{PART}{}{PART}{encoded}",
                    o.address.entity, o.address.component, o.address.field,
                )
            })
            .collect::<Vec<_>>()
            .join(&RECORD.to_string())
    }

    /// Records that `address` now differs from the prefab, and to what.
    pub fn mark(&mut self, address: OverrideAddress, value: Option<ReflectValue>) {
        let mut current = self.overrides();
        match current.iter_mut().find(|o| o.address == address) {
            Some(existing) => existing.value = value,
            None => current.push(Override { address, value }),
        }
        self.set_overrides(current);
    }

    /// Drops one override, so the field follows the prefab again.
    pub fn revert(&mut self, address: &OverrideAddress) {
        let kept: Vec<Override> = self
            .overrides()
            .into_iter()
            .filter(|o| &o.address != address)
            .collect();
        self.set_overrides(kept);
    }

    /// Drops every override on this instance.
    pub fn revert_all(&mut self) {
        self.overrides.clear();
    }

    /// Whether `address` is one the prefab must not overwrite.
    pub fn is_overridden(&self, address: &OverrideAddress) -> bool {
        self.overrides().iter().any(|o| &o.address == address)
    }

    /// What the user set this field to, if they set it.
    pub fn value_of(&self, address: &OverrideAddress) -> Option<ReflectValue> {
        self.overrides()
            .into_iter()
            .find(|o| &o.address == address)
            .and_then(|o| o.value)
    }

    /// Whether the user decided whether this component is on this
    /// instance — by adding it, or by taking it off.
    pub fn owns_component(&self, entity: usize, component: &str) -> bool {
        self.is_overridden(&OverrideAddress {
            entity,
            component: component.to_owned(),
            field: WHOLE_COMPONENT.to_owned(),
        })
    }
}

impl Component for PrefabInstance {}

impl Reflect for PrefabInstance {
    fn reflect_fields(&self) -> &'static [FieldMeta] {
        static FIELDS: &[FieldMeta] = &[
            FieldMeta {
                name: "source",
                group: "",
                doc: "The prefab this instance came from.\n\nSaving the prefab propagates its \
changes here, except where an override says otherwise.",
                type_name: "Option<Guid>",
                kind: FieldKind::AssetRef,
                choices: &[],
                bits: &[],
                range: None,
                shown_when: None,
                asset_type: "kooch_ecs::scene::document::SceneDocument",
                requires: "",
                fields: &[],
                layers: false,
                layer: false,
                hidden: false,
            },
            FieldMeta {
                name: "overrides",
                group: "",
                doc: "Fields this instance changed away from its prefab, as RON.\n\nWritten by \
the editor when you edit an instance. An override survives the prefab \
being saved — that is what makes it an override.",
                type_name: "String",
                kind: FieldKind::String,
                choices: &[],
                bits: &[],
                range: None,
                shown_when: None,
                asset_type: "",
                requires: "",
                fields: &[],
                layers: false,
                layer: false,
                hidden: false,
            },
        ];
        FIELDS
    }

    fn reflect_get(&self, field: &str) -> Option<ReflectValue> {
        match field {
            "source" => Some(ReflectValue::AssetRef {
                guid: self.source,
                asset_type: "kooch_ecs::scene::document::SceneDocument".to_owned(),
            }),
            "overrides" => Some(ReflectValue::String(self.overrides.clone())),
            _ => None,
        }
    }

    fn reflect_set(&mut self, field: &str, value: ReflectValue) -> Result<(), ReflectError> {
        match (field, value) {
            ("source", ReflectValue::AssetRef { guid, .. }) => {
                self.source = guid;
                Ok(())
            }
            ("overrides", ReflectValue::String(value)) => {
                self.overrides = value;
                Ok(())
            }
            (field, _) => Err(ReflectError::FieldNotFound(field.to_owned())),
        }
    }

    fn reflect_default() -> Self {
        Self::default()
    }

    /// The link is shown, not edited. Retargeting an instance at another
    /// prefab by typing a guid, or hand-editing the override set, are both
    /// ways to break the connection with no way to see that you did.
    fn inspector_visibility() -> InspectorVisibility {
        InspectorVisibility::ReadOnly
    }
}

/// Marks one entity of an instance as the prefab entity it was built from.
#[derive(Debug, Clone, Default, crate::Reflect)]
#[reflect(inspector = "read_only")]
pub struct PrefabMember {
    /// The instance this belongs to — the entity carrying [`PrefabInstance`].
    pub root: Entity,
    /// Index into the prefab document's `entities`.
    pub index: u32,
}

impl Component for PrefabMember {}

/// Marks `root` as an instance of the prefab `source`.
pub fn attach(
    resources: &mut kooch_core::resource::Resources,
    root: crate::entity::Entity,
    members: &[crate::entity::Entity],
    source: Guid,
) {
    insert_reflected(resources, root, PrefabInstance::new(source));
    for (index, entity) in members.iter().enumerate() {
        insert_reflected(
            resources,
            *entity,
            PrefabMember {
                root,
                index: index as u32,
            },
        );
    }
}

/// Inserts a reflected component and tells the archetype about it.
fn insert_reflected<T: Component + crate::reflect::Reflect + Clone>(
    resources: &mut kooch_core::resource::Resources,
    entity: crate::entity::Entity,
    value: T,
) {
    use crate::archetype_registry::ArchetypeRegistry;
    use crate::component::ComponentRegistry;

    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.register_cpu_reflected::<T>();
        if let Some(storage) = registry.get_cpu_mut::<T>() {
            storage.insert(entity, value);
        }
    }
    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>()
        && let Some(current) = archetypes.entity_archetype(entity)
    {
        let next = archetypes.archetype_after_add_dynamic(current, std::any::TypeId::of::<T>());
        archetypes.register_entity(entity, next);
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod value_tests;

#[cfg(test)]
mod record_meaning_tests;
