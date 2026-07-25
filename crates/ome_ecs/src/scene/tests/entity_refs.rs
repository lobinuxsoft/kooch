//! Entity references surviving a save/load round trip.
//!
//! This is the behaviour `parent_index` provided for `Parent` alone and
//! that nothing else could have — see #607.

use super::setup_resources;
use crate::commands::Commands;
use crate::component::{Component, ComponentRegistry};
use crate::entity::Entity;
use crate::persistent_id::PersistentId;
use crate::reflect::ReflectValue;
use crate::scene::{SceneDocument, sync_scene_to_ecs};
use ome_core::resource::Resources;

/// The shape #560's joints need: a component naming two other entities.
#[derive(Debug, Default, Clone, PartialEq, ome_ecs_macros::Reflect)]
pub(super) struct Link {
    pub target: Entity,
    pub label: String,
}

impl Component for Link {}

/// Spawns `count` entities, then points the first one at the last.
fn world_with_a_link(count: usize) -> (Resources, Entity, Entity) {
    let mut resources = setup_resources();
    let mut spawned = Vec::new();

    {
        let mut commands = resources.remove::<Commands>().unwrap();
        for _ in 0..count {
            spawned.push(commands.spawn(&mut resources).id());
        }
        commands.apply(&mut resources);
        resources.insert(commands);
    }

    let source = spawned[0];
    let target = *spawned.last().unwrap();

    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.register_cpu_reflected::<Link>();
        if let Some(storage) = registry.get_cpu_mut::<Link>() {
            storage.insert(
                source,
                Link {
                    target,
                    label: "anchor".into(),
                },
            );
        }
    }
    // The archetype has to know, or the save walk will not see it.
    if let Some(archetypes) = resources.get_mut::<crate::archetype_registry::ArchetypeRegistry>()
        && let Some(current) = archetypes.entity_archetype(source)
    {
        let next = archetypes.archetype_after_add_dynamic(current, std::any::TypeId::of::<Link>());
        archetypes.register_entity(source, next);
    }

    (resources, source, target)
}

fn link_of(resources: &Resources, entity: Entity) -> Link {
    resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Link>())
        .and_then(|s| s.get(entity))
        .cloned()
        .expect("the link survived")
}

/// The whole point: a reference still points at the same entity after a
/// round trip, even though every handle was reassigned.
#[test]
fn a_reference_survives_a_save_and_load() {
    let (mut resources, source, target) = world_with_a_link(3);
    let document = SceneDocument::from_ecs(&mut resources);

    let mut reloaded = setup_resources();
    reloaded
        .get_mut::<ComponentRegistry>()
        .unwrap()
        .register_cpu_reflected::<Link>();
    sync_scene_to_ecs(&document, &mut reloaded).expect("loads");

    // Handles are not preserved — that is the reason ids exist.
    let entities: Vec<Entity> = reloaded
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Link>())
        .map(|s| s.iter().map(|(&e, _)| e).collect())
        .unwrap_or_default();
    assert_eq!(entities.len(), 1, "exactly one entity carries a Link");

    let link = link_of(&reloaded, entities[0]);
    assert_eq!(link.label, "anchor");
    assert!(
        link.target.is_valid(),
        "the reference resolved to a live entity, got {:?}",
        link.target,
    );

    // And it points at the entity holding the same identity it did before.
    let target_id = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<PersistentId>())
        .and_then(|s| s.get(target))
        .expect("saving gave the target an id")
        .id;
    let reloaded_id = reloaded
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<PersistentId>())
        .and_then(|s| s.get(link.target))
        .expect("the target kept its id")
        .id;
    assert_eq!(reloaded_id, target_id, "it points at the same identity");
    assert_ne!(
        link.target, entities[0],
        "the reference points at another entity, not at its own holder",
    );
    // `source` is deliberately not compared against the reloaded handle:
    // a fresh world allocates from zero, so the indices coincide by
    // accident and an assertion either way would prove nothing.
    let _ = source;
}

/// Ids are handed out only to entities something points at. A world where
/// nothing references anything should gain no identity at all.
#[test]
fn saving_assigns_ids_only_to_referenced_entities() {
    let (mut resources, _, target) = world_with_a_link(4);
    let _ = SceneDocument::from_ecs(&mut resources);

    let with_ids: Vec<Entity> = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<PersistentId>())
        .map(|s| s.iter().map(|(&e, _)| e).collect())
        .unwrap_or_default();

    assert_eq!(
        with_ids,
        vec![target],
        "only the referenced entity got an id",
    );
}

/// Re-saving must not renumber anything: a second scene pointing here
/// would have its references silently redirected.
#[test]
fn re_saving_keeps_the_ids_it_already_assigned() {
    let (mut resources, _, target) = world_with_a_link(3);

    let _ = SceneDocument::from_ecs(&mut resources);
    let first = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<PersistentId>())
        .and_then(|s| s.get(target))
        .expect("id assigned")
        .id;

    let _ = SceneDocument::from_ecs(&mut resources);
    let second = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<PersistentId>())
        .and_then(|s| s.get(target))
        .expect("id still there")
        .id;

    assert_eq!(first, second, "ids must be stable across saves");
}

/// A file must never contain a runtime handle. `EntityRef` refuses to
/// serialise one, so this asserts the save path converted rather than
/// relying on the file merely looking right.
#[test]
fn a_saved_document_serialises_without_live_handles() {
    let (mut resources, _, _) = world_with_a_link(2);
    let document = SceneDocument::from_ecs(&mut resources);

    let encoded = ron::to_string(&document);
    assert!(
        encoded.is_ok(),
        "a saved document must serialise; got {:?}",
        encoded.err(),
    );

    // And the reference is present rather than quietly dropped.
    let link = document
        .entities
        .iter()
        .flat_map(|e| &e.components)
        .find(|c| c.type_name.ends_with("Link"))
        .expect("the Link component was saved");
    let (_, value) = link
        .fields
        .iter()
        .find(|(name, _)| name == "target")
        .expect("the target field was saved");
    assert!(
        matches!(value, ReflectValue::EntityRef(Some(r)) if r.is_unresolved()),
        "a saved reference must be persistent, got {value:?}",
    );
}
