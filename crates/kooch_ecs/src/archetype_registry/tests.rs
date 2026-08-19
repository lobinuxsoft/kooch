use std::any::TypeId;
use std::collections::BTreeSet;

use super::ArchetypeRegistry;
use crate::archetype::{Archetype, ArchetypeId};
use crate::entity::Entity;

struct Position;
struct Velocity;
struct Health;

fn entity(index: u32) -> Entity {
    Entity::new(index, 0)
}

#[test]
fn new_has_empty_archetype() {
    let reg = ArchetypeRegistry::new();
    assert_eq!(reg.archetype_count(), 1);
    assert!(reg.get(ArchetypeId::EMPTY).is_some());
}

#[test]
fn get_or_create_idempotent() {
    let mut reg = ArchetypeRegistry::new();

    let mut components = BTreeSet::new();
    components.insert(TypeId::of::<Position>());

    let id1 = reg.get_or_create(components.clone());
    let id2 = reg.get_or_create(components);

    assert_eq!(id1, id2);
    assert_eq!(reg.archetype_count(), 2); // EMPTY + Position
}

#[test]
fn register_entity_into_archetype() {
    let mut reg = ArchetypeRegistry::new();
    let e1 = entity(0);

    reg.register_entity(e1, ArchetypeId::EMPTY);

    assert_eq!(reg.entity_archetype(e1), Some(ArchetypeId::EMPTY));
    assert_eq!(reg.get(ArchetypeId::EMPTY).unwrap().len(), 1);
}

#[test]
fn register_entity_migrates() {
    let mut reg = ArchetypeRegistry::new();
    let e1 = entity(0);

    reg.register_entity(e1, ArchetypeId::EMPTY);

    let pos_arch = reg.archetype_after_add::<Position>(ArchetypeId::EMPTY);
    reg.register_entity(e1, pos_arch);

    assert_eq!(reg.entity_archetype(e1), Some(pos_arch));
    assert_eq!(reg.get(ArchetypeId::EMPTY).unwrap().len(), 0);
    assert_eq!(reg.get(pos_arch).unwrap().len(), 1);
}

#[test]
fn register_same_archetype_is_noop() {
    let mut reg = ArchetypeRegistry::new();
    let e1 = entity(0);

    reg.register_entity(e1, ArchetypeId::EMPTY);
    reg.register_entity(e1, ArchetypeId::EMPTY);

    assert_eq!(reg.get(ArchetypeId::EMPTY).unwrap().len(), 1);
}

#[test]
fn unregister_entity() {
    let mut reg = ArchetypeRegistry::new();
    let e1 = entity(0);

    reg.register_entity(e1, ArchetypeId::EMPTY);
    let old = reg.unregister_entity(e1);

    assert_eq!(old, Some(ArchetypeId::EMPTY));
    assert!(reg.entity_archetype(e1).is_none());
    assert_eq!(reg.get(ArchetypeId::EMPTY).unwrap().len(), 0);
}

#[test]
fn unregister_unknown_entity() {
    let mut reg = ArchetypeRegistry::new();
    assert_eq!(reg.unregister_entity(entity(99)), None);
}

#[test]
fn archetype_after_add() {
    let mut reg = ArchetypeRegistry::new();

    let a1 = reg.archetype_after_add::<Position>(ArchetypeId::EMPTY);
    let a2 = reg.archetype_after_add::<Velocity>(a1);

    assert_ne!(a1, ArchetypeId::EMPTY);
    assert_ne!(a2, a1);

    let arch = reg.get(a2).unwrap();
    assert!(arch.has_component::<Position>());
    assert!(arch.has_component::<Velocity>());
    assert!(!arch.has_component::<Health>());
}

#[test]
fn archetype_after_remove() {
    let mut reg = ArchetypeRegistry::new();

    let pos_vel = reg.archetype_after_add::<Position>(ArchetypeId::EMPTY);
    let pos_vel = reg.archetype_after_add::<Velocity>(pos_vel);
    let pos_only = reg.archetype_after_remove::<Velocity>(pos_vel);

    let arch = reg.get(pos_only).unwrap();
    assert!(arch.has_component::<Position>());
    assert!(!arch.has_component::<Velocity>());
}

#[test]
fn remove_all_components_returns_to_empty() {
    let mut reg = ArchetypeRegistry::new();

    let with_pos = reg.archetype_after_add::<Position>(ArchetypeId::EMPTY);
    let back_to_empty = reg.archetype_after_remove::<Position>(with_pos);

    assert_eq!(back_to_empty, ArchetypeId::EMPTY);
}

#[test]
fn transition_cache_works() {
    let mut reg = ArchetypeRegistry::new();

    let a1 = reg.archetype_after_add::<Position>(ArchetypeId::EMPTY);
    let a2 = reg.archetype_after_add::<Position>(ArchetypeId::EMPTY);

    assert_eq!(a1, a2);
    // Should not create extra archetypes.
    assert_eq!(reg.archetype_count(), 2);
}

#[test]
fn iter_matching_basic() {
    let mut reg = ArchetypeRegistry::new();

    // Create archetypes: (Pos), (Pos, Vel), (Pos, Vel, Health)
    let pos = reg.archetype_after_add::<Position>(ArchetypeId::EMPTY);
    let pos_vel = reg.archetype_after_add::<Velocity>(pos);
    let pos_vel_hp = reg.archetype_after_add::<Health>(pos_vel);

    // Add entities.
    reg.register_entity(entity(0), pos);
    reg.register_entity(entity(1), pos_vel);
    reg.register_entity(entity(2), pos_vel);
    reg.register_entity(entity(3), pos_vel_hp);

    // Query for (Position, Velocity) should match pos_vel and pos_vel_hp.
    let required = [TypeId::of::<Position>(), TypeId::of::<Velocity>()];
    let matched: Vec<&Archetype> = reg.iter_matching(&required).collect();

    assert_eq!(matched.len(), 2);
    let total_entities: usize = matched.iter().map(|a| a.len()).sum();
    assert_eq!(total_entities, 3); // e1, e2, e3
}

#[test]
fn iter_matching_empty_required() {
    let mut reg = ArchetypeRegistry::new();

    let pos = reg.archetype_after_add::<Position>(ArchetypeId::EMPTY);
    reg.register_entity(entity(0), pos);
    reg.register_entity(entity(1), ArchetypeId::EMPTY);

    // Empty required matches ALL archetypes.
    let matched: Vec<&Archetype> = reg.iter_matching(&[]).collect();
    assert_eq!(matched.len(), 2);
}

#[test]
fn full_migration_workflow() {
    let mut reg = ArchetypeRegistry::new();
    let e = entity(0);

    // Entity starts in EMPTY archetype.
    reg.register_entity(e, ArchetypeId::EMPTY);

    // Add Position.
    let current = reg.entity_archetype(e).unwrap();
    let new_arch = reg.archetype_after_add::<Position>(current);
    reg.register_entity(e, new_arch);

    // Add Velocity.
    let current = reg.entity_archetype(e).unwrap();
    let new_arch = reg.archetype_after_add::<Velocity>(current);
    reg.register_entity(e, new_arch);

    // Verify entity has both components in its archetype.
    let arch_id = reg.entity_archetype(e).unwrap();
    let arch = reg.get(arch_id).unwrap();
    assert!(arch.has_component::<Position>());
    assert!(arch.has_component::<Velocity>());
    assert_eq!(arch.entities(), &[e]);

    // Remove Position.
    let new_arch = reg.archetype_after_remove::<Position>(arch_id);
    reg.register_entity(e, new_arch);

    let arch_id = reg.entity_archetype(e).unwrap();
    let arch = reg.get(arch_id).unwrap();
    assert!(!arch.has_component::<Position>());
    assert!(arch.has_component::<Velocity>());
}

#[test]
fn dynamic_transition_methods() {
    let mut reg = ArchetypeRegistry::new();

    let type_id = TypeId::of::<Position>();
    let a1 = reg.archetype_after_add_dynamic(ArchetypeId::EMPTY, type_id);
    let a2 = reg.archetype_after_add::<Position>(ArchetypeId::EMPTY);

    assert_eq!(a1, a2);
}

#[test]
fn multiple_entities_per_archetype() {
    let mut reg = ArchetypeRegistry::new();
    let arch = reg.archetype_after_add::<Position>(ArchetypeId::EMPTY);

    for i in 0..100 {
        reg.register_entity(entity(i), arch);
    }

    assert_eq!(reg.get(arch).unwrap().len(), 100);
}

#[test]
fn gc_removes_empty_archetypes() {
    let mut reg = ArchetypeRegistry::new();
    let e = entity(0);

    // Create chain: EMPTY → +Pos → +Vel → -Pos (leaves (Pos) and (Pos,Vel) empty).
    reg.register_entity(e, ArchetypeId::EMPTY);
    let pos = reg.archetype_after_add::<Position>(ArchetypeId::EMPTY);
    reg.register_entity(e, pos);
    let pos_vel = reg.archetype_after_add::<Velocity>(pos);
    reg.register_entity(e, pos_vel);
    let vel_only = reg.archetype_after_remove::<Position>(pos_vel);
    reg.register_entity(e, vel_only);

    // 4 archetypes: EMPTY, (Pos), (Pos,Vel), (Vel).
    assert_eq!(reg.archetype_count(), 4);

    // GC should remove (Pos) and (Pos,Vel) — both empty.
    let removed = reg.gc_empty_archetypes();
    assert_eq!(removed, 2);
    assert_eq!(reg.archetype_count(), 2); // EMPTY + (Vel)

    // EMPTY is preserved even though it has 0 entities.
    assert!(reg.get(ArchetypeId::EMPTY).is_some());
    // (Vel) still has the entity.
    assert_eq!(reg.get(vel_only).unwrap().len(), 1);
}

#[test]
fn gc_preserves_empty_archetype() {
    let mut reg = ArchetypeRegistry::new();
    // EMPTY has 0 entities but should never be removed.
    let removed = reg.gc_empty_archetypes();
    assert_eq!(removed, 0);
    assert_eq!(reg.archetype_count(), 1);
}

#[test]
fn gc_invalidates_transition_cache() {
    let mut reg = ArchetypeRegistry::new();

    // Build transitions: EMPTY +Pos → (Pos), (Pos) +Vel → (Pos,Vel).
    let pos = reg.archetype_after_add::<Position>(ArchetypeId::EMPTY);
    let _pos_vel = reg.archetype_after_add::<Velocity>(pos);

    // No entities — GC removes both.
    reg.gc_empty_archetypes();
    assert_eq!(reg.archetype_count(), 1);

    // Transitions are rebuilt on demand — should still work.
    let pos2 = reg.archetype_after_add::<Position>(ArchetypeId::EMPTY);
    assert!(reg.get(pos2).is_some());
}

// -- The table behind an archetype (#891, stage 5b) -------------------------

mod tables {
    use std::collections::BTreeSet;

    use crate::archetype::ArchetypeId;
    use crate::archetype_registry::ArchetypeRegistry;
    use crate::component::ComponentRegistry;
    use crate::component::traits::Component;

    struct Health(u32);
    impl Component for Health {}

    struct Speed(f32);
    impl Component for Speed {}

    fn components() -> ComponentRegistry {
        let mut registry = ComponentRegistry::new();
        registry.register_cpu::<Health>();
        registry.register_cpu::<Speed>();
        registry
    }

    fn set(types: &[std::any::TypeId]) -> BTreeSet<std::any::TypeId> {
        types.iter().copied().collect()
    }

    #[test]
    fn an_archetype_gets_a_table_for_its_set() {
        let components = components();
        let mut archetypes = ArchetypeRegistry::new();
        let id = archetypes.get_or_create(set(&[
            std::any::TypeId::of::<Health>(),
            std::any::TypeId::of::<Speed>(),
        ]));

        let table = archetypes.table_of(id, &components).unwrap();
        let table = archetypes.tables().get(table).unwrap();

        assert_eq!(table.component_ids().len(), 2);
        assert!(table.is_empty());
        assert!(table.rows_agree(), "an empty table is a valid one");
    }

    /// The answer cannot change — an archetype's component set is fixed —
    /// so asking twice must not build a second table nobody would notice.
    #[test]
    fn the_answer_is_stable() {
        let components = components();
        let mut archetypes = ArchetypeRegistry::new();
        let id = archetypes.get_or_create(set(&[std::any::TypeId::of::<Health>()]));

        let first = archetypes.table_of(id, &components).unwrap();
        let again = archetypes.table_of(id, &components).unwrap();

        assert_eq!(first, again);
        assert_eq!(archetypes.tables().len(), 1);
    }

    #[test]
    fn different_sets_get_different_tables() {
        let components = components();
        let mut archetypes = ArchetypeRegistry::new();
        let one = archetypes.get_or_create(set(&[std::any::TypeId::of::<Health>()]));
        let two = archetypes.get_or_create(set(&[std::any::TypeId::of::<Speed>()]));

        let a = archetypes.table_of(one, &components).unwrap();
        let b = archetypes.table_of(two, &components).unwrap();

        assert_ne!(a, b);
        assert_eq!(archetypes.tables().len(), 2);
    }

    /// An entity with no components lives in the empty archetype, and that
    /// has to have a table like any other — a column-less one.
    #[test]
    fn the_empty_archetype_has_a_table() {
        let components = components();
        let mut archetypes = ArchetypeRegistry::new();

        let table = archetypes
            .table_of(ArchetypeId::EMPTY, &components)
            .unwrap();

        assert!(
            archetypes
                .tables()
                .get(table)
                .unwrap()
                .component_ids()
                .is_empty()
        );
    }

    #[test]
    fn an_unknown_archetype_has_no_table() {
        let components = components();
        let mut archetypes = ArchetypeRegistry::new();
        let known = archetypes.get_or_create(set(&[std::any::TypeId::of::<Health>()]));

        // A different set that was never created.
        let unknown =
            crate::archetype::ArchetypeId::from_components(&set(
                &[std::any::TypeId::of::<Speed>()],
            ));
        assert_ne!(known, unknown);

        assert_eq!(archetypes.table_of(unknown, &components), None);
    }

    /// 🔴 Nothing is built until something asks. A table per archetype
    /// created eagerly would need the component registry in all 38 places
    /// that transition an archetype, none of which cares about storage.
    #[test]
    fn no_table_exists_until_asked_for() {
        let components = components();
        let mut archetypes = ArchetypeRegistry::new();
        archetypes.get_or_create(set(&[std::any::TypeId::of::<Health>()]));

        assert!(archetypes.tables().is_empty());

        archetypes
            .table_of(ArchetypeId::EMPTY, &components)
            .unwrap();
        assert_eq!(archetypes.tables().len(), 1);
    }
}
