//! Capturing one entity as a prefab, and stamping it back out (#611).

use crate::commands::Commands;
use crate::component::ComponentRegistry;
use crate::hierarchy::{Children, Parent};
use crate::persistent_id::{EntityGuid, PersistentIdAllocator};
use crate::reflect::{EntityRef, ReflectValue};
use crate::scene::{ComponentDescription, EntityDescription, SceneDocument, SceneError};
use kooch_core::Guid;

use super::{Health, setup_resources};

/// `Parent` and `Children` are both written by hand: a test that relied on
/// the hierarchy sync running would be asserting two things at once.
fn parent_child(
    resources: &mut kooch_core::resource::Resources,
    parent: crate::entity::Entity,
    child: crate::entity::Entity,
) {
    {
        let registry = resources.get_mut::<ComponentRegistry>().unwrap();
        registry.register_cpu_reflected::<Parent>();
        registry.register_cpu_reflected::<Children>();
        if let Some(storage) = registry.get_cpu_mut::<Parent>() {
            storage.insert(child, Parent { entity: parent });
        }
        if let Some(storage) = registry.get_cpu_mut::<Children>() {
            storage.insert(
                parent,
                Children {
                    entities: vec![child],
                },
            );
        }
    }
    super::add_to_archetype(resources, child, std::any::TypeId::of::<Parent>());
    super::add_to_archetype(resources, parent, std::any::TypeId::of::<Children>());
}

fn named(
    resources: &mut kooch_core::resource::Resources,
    entity: crate::entity::Entity,
    name: &str,
) {
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    registry.register_cpu_reflected::<crate::name::Name>();
    registry.insert_default_reflected(&std::any::TypeId::of::<crate::name::Name>(), entity);
    if let Some(storage) = registry.get_cpu_mut::<crate::name::Name>()
        && let Some(value) = storage.get_mut(entity)
    {
        value.value = name.to_owned();
    }
    super::add_to_archetype(
        resources,
        entity,
        std::any::TypeId::of::<crate::name::Name>(),
    );
}

/// A root with one child, plus an unrelated entity that must not be dragged in.
fn world_with_a_subtree() -> (
    kooch_core::resource::Resources,
    crate::entity::Entity,
    crate::entity::Entity,
) {
    let mut resources = setup_resources();
    let (root, child, outsider) = {
        let mut commands = resources.remove::<Commands>().unwrap();
        let root = commands
            .spawn(&mut resources)
            .insert_reflected(Health { hp: 9, max_hp: 9 })
            .id();
        let child = commands
            .spawn(&mut resources)
            .insert_reflected(Health { hp: 1, max_hp: 1 })
            .id();
        let outsider = commands
            .spawn(&mut resources)
            .insert_reflected(Health { hp: 5, max_hp: 5 })
            .id();
        commands.apply(&mut resources);
        resources.insert(commands);
        (root, child, outsider)
    };
    named(&mut resources, root, "Ball");
    named(&mut resources, child, "Trail");
    named(&mut resources, outsider, "Ground");
    parent_child(&mut resources, root, child);
    (resources, root, outsider)
}

#[path = "prefab/subtrees.rs"]
mod subtrees;
use super::add_to_archetype;
use subtrees::world_with_a_deep_subtree;
#[path = "prefab/roots.rs"]
mod roots;
use roots::{described, document, identified, parented_to};
#[path = "prefab/instances.rs"]
mod instances;
#[path = "prefab/references.rs"]
mod references;
