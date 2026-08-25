//! `place` against a real world.

use super::*;
use crate::archetype_registry::ArchetypeRegistry;
use crate::commands::Commands;
use crate::component::ComponentRegistry;
use crate::entity::Entity;
use crate::hierarchy::{Children, Parent};
use crate::query::AccessTracker;
use crate::scene_member::SceneMember;
use kooch_core::Guid;
use kooch_core::resource::Resources;

fn world(count: usize) -> (Resources, Vec<Entity>, Guid) {
    let mut r = Resources::new();
    r.insert(crate::allocator::EntityAllocator::new());
    r.insert(ComponentRegistry::new());
    r.insert(ArchetypeRegistry::new());
    r.insert(AccessTracker::new());
    r.insert(Commands::new());
    {
        let reg = r.get_mut::<ComponentRegistry>().unwrap();
        reg.register_cpu_reflected::<Order>();
        reg.register_cpu_reflected::<Parent>();
        reg.register_cpu_reflected::<Children>();
        reg.register_cpu_reflected::<crate::transform::Transform>();
        reg.register_cpu::<SceneMember>();
    }

    let scene = Guid::new_v4();
    let mut made = Vec::new();
    for _ in 0..count {
        let mut commands = r.remove::<Commands>().unwrap();
        let e = commands.spawn(&mut r).id();
        commands.apply(&mut r);
        r.insert(commands);
        if let Some(archetypes) = r.get_mut::<ArchetypeRegistry>() {
            archetypes.register_entity(e, crate::archetype::ArchetypeId::EMPTY);
        }
        if let Some(reg) = r.get_mut::<ComponentRegistry>()
            && let Some(s) = reg.get_cpu_mut::<SceneMember>()
        {
            s.insert(e, SceneMember::new(scene));
        }
        if let Some(archetypes) = r.get_mut::<ArchetypeRegistry>()
            && let Some(current) = archetypes.entity_archetype(e)
        {
            let next = archetypes
                .archetype_after_add_dynamic(current, std::any::TypeId::of::<SceneMember>());
            archetypes.register_entity(e, next);
        }
        made.push(e);
    }
    (r, made, scene)
}

fn shown(resources: &Resources, entities: &[Entity]) -> Vec<Entity> {
    let orders = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Order>());
    let mut sorted = entities.to_vec();
    sorted.sort_by_key(|e| {
        let v = orders.and_then(|s| s.get(*e)).map(|o| o.value);
        (v.is_none(), v.unwrap_or(0), e.index())
    });
    sorted
}

/// The first move in an unordered group numbers the whole group, because
/// there is nothing to sit between until there is.
#[test]
fn the_first_move_numbers_the_group() {
    let (mut r, e, _) = world(4);
    // Move the last one to the front.
    assert!(place(&mut r, e[3], None, Some(e[0])));
    assert_eq!(shown(&r, &e), vec![e[3], e[0], e[1], e[2]]);
}

/// Once numbered, moving one writes one value — which is what keeps a
/// scene diff from showing every sibling as changed.
#[test]
fn a_later_move_writes_one_value() {
    let (mut r, e, _) = world(4);
    place(&mut r, e[3], None, Some(e[0]));
    let before: Vec<Option<u32>> = e
        .iter()
        .map(|&x| crate::order::place::order_of(&r, x))
        .collect();

    // e[2] between e[0] and e[1].
    assert!(place(&mut r, e[2], None, Some(e[1])));
    let after: Vec<Option<u32>> = e
        .iter()
        .map(|&x| crate::order::place::order_of(&r, x))
        .collect();

    let changed = before.iter().zip(&after).filter(|(a, b)| a != b).count();
    assert_eq!(changed, 1, "moving one sibling rewrote {changed} of them");
    assert_eq!(shown(&r, &e), vec![e[3], e[0], e[2], e[1]]);
}

/// `before: None` puts it last.
#[test]
fn moving_to_the_end_puts_it_last() {
    let (mut r, e, _) = world(3);
    assert!(place(&mut r, e[0], None, None));
    assert_eq!(shown(&r, &e), vec![e[1], e[2], e[0]]);
}

/// Ordering a child group leaves the roots alone, and the child follows
/// its new parent.
#[test]
fn a_child_is_ordered_among_its_new_siblings() {
    let (mut r, e, _) = world(4);
    // e[1] and e[2] become children of e[0].
    assert!(place(&mut r, e[1], Some(e[0]), None));
    assert!(place(&mut r, e[2], Some(e[0]), None));
    assert_eq!(shown(&r, &[e[1], e[2]]), vec![e[1], e[2]]);

    // Now put e[2] first among them.
    assert!(place(&mut r, e[2], Some(e[0]), Some(e[1])));
    assert_eq!(shown(&r, &[e[1], e[2]]), vec![e[2], e[1]]);

    let parent_of = |r: &Resources, x: Entity| {
        r.get::<ComponentRegistry>()
            .and_then(|reg| reg.get_cpu::<Parent>())
            .and_then(|s| s.get(x))
            .map(|p| p.entity)
    };
    assert_eq!(parent_of(&r, e[2]), Some(e[0]));
}

/// 🔴 Moving an entity into its own subtree is refused. Allowing it
/// detaches that subtree from the world: the cycle has no root, so
/// nothing walking down from a scene ever reaches it again.
#[test]
fn an_entity_cannot_move_into_itself() {
    let (mut r, e, _) = world(3);
    place(&mut r, e[1], Some(e[0]), None);
    place(&mut r, e[2], Some(e[1]), None);

    assert!(!place(&mut r, e[0], Some(e[2]), None), "grandchild");
    assert!(!place(&mut r, e[0], Some(e[0]), None), "itself");
}
