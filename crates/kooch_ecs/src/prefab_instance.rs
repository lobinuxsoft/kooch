//! The link between an entity in a scene and the prefab it came from.
//!
//! # What the link is for
//!
//! With a dozen instances of one prefab placed by hand, changing the prefab
//! and re-placing all twelve is work with no result — so nobody does it.
//! They edit the twelve by hand instead, and one of them ends up different.
//! The link is what lets a change to the prefab reach them.
//!
//! # Why it is an editor concept
//!
//! Only the editor propagates. A running game spawning bullets from a
//! prefab wants entities, not a relationship to maintain, so
//! [`spawn_prefab`](crate::scene::spawn_prefab) attaches nothing and the
//! runtime pays nothing. Unity draws the same line: in a build,
//! `Instantiate` gives a plain object and the prefab connection is an
//! authoring concept.
//!
//! The component is still ordinary scene data — it is written to the scene
//! file, because the link has to survive closing the editor.
//!
//! # Why an override carries its value
//!
//! Because nothing else does. A scene stores an instance as a reference to
//! its prefab plus this list — the instance's entities are not written —
//! so an override that recorded only *which* field changed would be a
//! change that vanished on save.
//!
//! A record with no value is the other kind of decision: the user took
//! that component off this instance, and there is no value to keep.
//!
//! # Why the overrides are a string
//!
//! An override is "the user changed this field on *this* instance, so
//! leave it alone when the prefab changes". That is a *set of field
//! addresses*, and a scene file stores components as
//! `Vec<(String, ReflectValue)>` — which has scalars, vectors, asset
//! references and strings, but no list.
//!
//! The alternatives were a new `ReflectValue` variant, which drags every
//! reflection consumer and the whole inspector along for one component, or
//! moving overrides out of the component and into the scene document,
//! which puts one instance's business in the file's structure. Encoding
//! the set into one field keeps the change to this component, and the
//! format already round-trips strings exactly.
//!
//! Only the *addresses* are stored. The values are already in the
//! instance's own components, because a scene writes its entities out in
//! full — see #611 for why that is deliberate.

use kooch_core::Guid;

use crate::component::Component;
use crate::entity::Entity;
use crate::reflect::{
    FieldKind, FieldMeta, InspectorVisibility, Reflect, ReflectError, ReflectValue,
};

/// Separates one override from the next.
///
/// ASCII's own record separator rather than a punctuation character: a
/// record now ends in a serialised value, and a value can contain any
/// punctuation you care to name. Control characters are the one thing RON
/// will not emit raw — it escapes them inside strings — which is what
/// makes this safe where `;` was not.
const RECORD: char = '\u{1e}';
/// Separates the parts of a single record.
const PART: char = '\u{1f}';

/// Marks an entity as an instance of a prefab, and records what has been
/// changed on it since.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PrefabInstance {
    /// The prefab this was stamped from.
    ///
    /// A [`Guid`] rather than a path, so moving or renaming the prefab
    /// does not break the instances of it.
    pub source: Option<Guid>,
    /// Field addresses the user has changed on this instance.
    ///
    /// Opaque on purpose — see the module docs. Read it through
    /// [`Self::overrides`] and write it through [`Self::set_overrides`]
    /// rather than by hand.
    pub overrides: String,
}

/// The field name that means "the component itself", rather than a field
/// on it.
///
/// A user who adds or removes a component on an instance has made a
/// decision about its *presence*, and propagation has to respect it in
/// both directions: it must not delete a component they added, and must
/// not restore one they removed. Encoded as an address with no field so it
/// travels with the rest of the set instead of needing a second one.
pub const WHOLE_COMPONENT: &str = "";

/// One field the user changed on an instance, addressed relative to the
/// prefab rather than to the world.
///
/// `entity` is the index into the prefab document, not an [`Entity`]: the
/// address has to survive a session, and a handle does not.
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
    ///
    /// Malformed records are skipped rather than failing the whole set: a
    /// hand-edited scene file should cost the overrides it corrupted, not
    /// the instance's link to its prefab.
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
    ///
    /// Re-marking replaces the value: the user changing the same field
    /// twice is one override, not two, and the second value is the one
    /// that survives.
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
                doc: "The prefab this instance came from.\n\nSaving the prefab propagates its \
changes here, except where an override says otherwise.",
                type_name: "Option<Guid>",
                kind: FieldKind::AssetRef,
                choices: &[],
                bits: &[],
                shown_when: None,
                asset_type: "kooch_ecs::scene::document::SceneDocument",
                requires: "",
            },
            FieldMeta {
                name: "overrides",
                doc: "Fields this instance changed away from its prefab, as RON.\n\nWritten by \
the editor when you edit an instance. An override survives the prefab \
being saved — that is what makes it an override.",
                type_name: "String",
                kind: FieldKind::String,
                choices: &[],
                bits: &[],
                shown_when: None,
                asset_type: "",
                requires: "",
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
///
/// # Why every member is tagged and not just the root
///
/// An override names *which entity of the prefab* it belongs to, and
/// propagation has to go the other way — given entity `i` of the prefab,
/// which live entity holds it. A link on the root alone cannot answer
/// either question for a prefab with children.
///
/// The alternatives were addressing by name path, which this codebase has
/// already learned does not work — a scene with five meshes called `Mesh`
/// is ordinary, and resolving parents by name was a bug it fixed — or by
/// child index, which is unique until someone adds or reorders a child in
/// the instance, at which point every override after it silently addresses
/// a different entity.
#[derive(Debug, Clone, Default, crate::Reflect)]
#[reflect(inspector = "read_only")]
pub struct PrefabMember {
    /// The instance this belongs to — the entity carrying
    /// [`PrefabInstance`].
    ///
    /// Saved and resolved by the same generic entity-reference pass that
    /// handles `Parent`, so it survives a scene round trip.
    pub root: Entity,
    /// Index into the prefab document's `entities`.
    ///
    /// `u32` rather than `usize`: a prefab is one authored object, and this
    /// sits on every entity of every instance in a scene.
    pub index: u32,
}

impl Component for PrefabMember {}

/// Marks `root` as an instance of the prefab `source`.
///
/// Called by the **editor's** instancing, not by
/// [`spawn_prefab`](crate::scene::spawn_prefab): a game spawning bullets
/// wants entities, not a relationship to maintain. Both go through the
/// same spawn; only one attaches the link.
/// `members[i]` must be the entity spawned for the prefab's entity `i` —
/// which is what
/// [`instantiate_members`](crate::scene::instantiate_members) hands back.
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
///
/// The archetype half is not optional: a component the archetype does not
/// know about is invisible to every query, so an instance the propagation
/// pass cannot find is an instance it walks straight past.
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
