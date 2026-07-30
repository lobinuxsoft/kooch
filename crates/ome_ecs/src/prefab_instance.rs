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

use ome_core::Guid;

use crate::component::Component;
use crate::entity::Entity;
use crate::reflect::{
    FieldKind, FieldMeta, InspectorVisibility, Reflect, ReflectError, ReflectValue,
};

/// Separates one override address from the next.
const RECORD: char = ';';
/// Separates the parts of a single address.
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
    pub fn overrides(&self) -> Vec<OverrideAddress> {
        self.overrides
            .split(RECORD)
            .filter(|record| !record.is_empty())
            .filter_map(|record| {
                let mut parts = record.split(PART);
                let entity = parts.next()?.parse().ok()?;
                let component = parts.next()?.to_owned();
                let field = parts.next()?.to_owned();
                Some(OverrideAddress {
                    entity,
                    component,
                    field,
                })
            })
            .collect()
    }

    /// Replaces the set, in a stable order so re-saving a scene does not
    /// produce a different file for the same state.
    pub fn set_overrides(&mut self, addresses: impl IntoIterator<Item = OverrideAddress>) {
        let mut sorted: Vec<OverrideAddress> = addresses.into_iter().collect();
        sorted.sort_by(|a, b| {
            a.entity
                .cmp(&b.entity)
                .then_with(|| a.component.cmp(&b.component))
                .then_with(|| a.field.cmp(&b.field))
        });
        sorted.dedup();
        self.overrides = sorted
            .iter()
            .map(|a| format!("{}{PART}{}{PART}{}", a.entity, a.component, a.field))
            .collect::<Vec<_>>()
            .join(&RECORD.to_string());
    }

    /// Records that `address` now differs from the prefab.
    pub fn mark(&mut self, address: OverrideAddress) {
        let mut current = self.overrides();
        if !current.contains(&address) {
            current.push(address);
            self.set_overrides(current);
        }
    }

    /// Drops one override, so the field follows the prefab again.
    pub fn revert(&mut self, address: &OverrideAddress) {
        let kept: Vec<OverrideAddress> = self
            .overrides()
            .into_iter()
            .filter(|a| a != address)
            .collect();
        self.set_overrides(kept);
    }

    /// Drops every override on this instance.
    pub fn revert_all(&mut self) {
        self.overrides.clear();
    }

    /// Whether `address` is one the prefab must not overwrite.
    pub fn is_overridden(&self, address: &OverrideAddress) -> bool {
        self.overrides().contains(address)
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
                type_name: "Option<Guid>",
                kind: FieldKind::AssetRef,
                choices: &[],
                bits: &[],
                shown_when: None,
                asset_type: "ome_ecs::scene::document::SceneDocument",
                requires: "",
            },
            FieldMeta {
                name: "overrides",
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
                asset_type: "ome_ecs::scene::document::SceneDocument".to_owned(),
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
    resources: &mut ome_core::resource::Resources,
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
    resources: &mut ome_core::resource::Resources,
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
mod tests {
    use super::*;

    fn address(entity: usize, component: &str, field: &str) -> OverrideAddress {
        OverrideAddress {
            entity,
            component: component.to_owned(),
            field: field.to_owned(),
        }
    }

    #[test]
    fn an_instance_starts_with_nothing_overridden() {
        let instance = PrefabInstance::new(Guid::new_v4());
        assert!(instance.overrides().is_empty());
    }

    #[test]
    fn marking_then_reading_round_trips() {
        let mut instance = PrefabInstance::new(Guid::new_v4());
        let moved = address(0, "ome_ecs::transform::Transform", "position");
        instance.mark(moved.clone());
        assert_eq!(instance.overrides(), vec![moved.clone()]);
        assert!(instance.is_overridden(&moved));
    }

    /// Dragging a gizmo emits an edit per drag; marking the same field
    /// twice must not grow the set, or a long session's scene file fills
    /// with the same address.
    #[test]
    fn marking_the_same_field_twice_records_it_once() {
        let mut instance = PrefabInstance::default();
        let moved = address(0, "T", "position");
        instance.mark(moved.clone());
        instance.mark(moved);
        assert_eq!(instance.overrides().len(), 1);
    }

    /// The whole point of recording: reverting has something to remove.
    /// A diff would have nothing to revert *to*.
    #[test]
    fn reverting_drops_only_the_field_named() {
        let mut instance = PrefabInstance::default();
        let position = address(0, "T", "position");
        let scale = address(0, "T", "scale");
        instance.mark(position.clone());
        instance.mark(scale.clone());

        instance.revert(&position);
        assert!(!instance.is_overridden(&position));
        assert!(
            instance.is_overridden(&scale),
            "an unrelated field was reverted"
        );
    }

    #[test]
    fn reverting_everything_leaves_nothing() {
        let mut instance = PrefabInstance::default();
        instance.mark(address(0, "T", "position"));
        instance.mark(address(1, "U", "health"));
        instance.revert_all();
        assert!(instance.overrides().is_empty());
    }

    /// Two instances in the same state must produce the same bytes, or
    /// re-saving a scene shows a diff where nothing changed.
    #[test]
    fn the_encoding_does_not_depend_on_the_order_marks_arrived_in() {
        let mut first = PrefabInstance::default();
        first.mark(address(1, "B", "y"));
        first.mark(address(0, "A", "x"));

        let mut second = PrefabInstance::default();
        second.mark(address(0, "A", "x"));
        second.mark(address(1, "B", "y"));

        assert_eq!(first.overrides, second.overrides);
    }

    /// A hand-edited scene should cost the overrides it corrupted, not the
    /// instance's link to its prefab.
    #[test]
    fn a_malformed_record_is_skipped_rather_than_poisoning_the_set() {
        let mut instance = PrefabInstance::new(Guid::new_v4());
        instance.mark(address(0, "T", "position"));
        instance.overrides.push_str(";garbage;also\u{1f}bad");

        assert_eq!(instance.overrides().len(), 1);
        assert!(instance.source.is_some(), "the link survived");
    }

    /// Component type paths contain `::` and names can contain most
    /// things; the separators must not be something a real address holds.
    #[test]
    fn a_realistic_address_survives_the_encoding() {
        let mut instance = PrefabInstance::default();
        let real = address(3, "ome_render::mesh_renderer::MeshRenderer", "cast_shadows");
        instance.mark(real.clone());
        assert_eq!(instance.overrides(), vec![real]);
    }
}
