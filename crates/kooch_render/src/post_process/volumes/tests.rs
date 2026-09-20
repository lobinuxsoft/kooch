use super::*;
use kooch_ecs::post_process::PostEffect;

fn effect(material: Guid, weight: f32) -> PostEffect {
    PostEffect {
        material: Some(material),
        enabled: true,
        weight,
    }
}

fn volume(effects: Vec<PostEffect>) -> PostProcessVolume {
    PostProcessVolume {
        effects,
        ..Default::default()
    }
}

fn weight_of(stack: &[(Guid, f32)], material: Guid) -> Option<f32> {
    stack
        .iter()
        .find(|(guid, _)| *guid == material)
        .map(|(_, weight)| *weight)
}

/// 🔴 Half-way in is half the effect. A volume that switched on at full strength on arrival is the
/// pop the blend distance exists to avoid.
#[test]
fn a_half_reached_volume_is_half() {
    let ps1 = Guid::new_v4();
    let volume = volume(vec![effect(ps1, 1.0)]);
    let stack = folded(
        &[],
        &[Reached {
            weight: 0.5,
            volume: volume.clone(),
        }],
    );
    assert_eq!(weight_of(&stack, ps1), Some(0.5));
}

/// The scene's own stack is what a volume overrides, and it survives underneath: an effect no
/// volume mentions is still drawn.
#[test]
fn the_base_survives_underneath() {
    let (grain, ps1) = (Guid::new_v4(), Guid::new_v4());
    let volume = volume(vec![effect(ps1, 1.0)]);
    let stack = folded(
        &[(grain, 0.3)],
        &[Reached {
            weight: 1.0,
            volume: volume.clone(),
        }],
    );
    assert_eq!(weight_of(&stack, grain), Some(0.3));
    assert_eq!(weight_of(&stack, ps1), Some(1.0));
}

/// 🔴 An override is towards the volume's value, not away from it: a volume asking for less of an
/// effect than the scene has takes it down, which a max or an add could never do.
#[test]
fn a_volume_can_turn_one_down() {
    let grain = Guid::new_v4();
    let volume = volume(vec![effect(grain, 0.0)]);
    let stack = folded(
        &[(grain, 1.0)],
        &[Reached {
            weight: 1.0,
            volume: volume.clone(),
        }],
    );
    assert_eq!(weight_of(&stack, grain), None, "it should be off entirely");
}

/// Overlapping volumes: the later one in the list overrides the earlier, by its own weight.
#[test]
fn the_last_volume_overrides() {
    let ps1 = Guid::new_v4();
    let (low, high) = (
        volume(vec![effect(ps1, 1.0)]),
        volume(vec![effect(ps1, 0.2)]),
    );
    let stack = folded(
        &[],
        &[
            Reached {
                weight: 1.0,
                volume: low.clone(),
            },
            Reached {
                weight: 1.0,
                volume: high.clone(),
            },
        ],
    );
    let weight = weight_of(&stack, ps1).expect("the effect is in the stack");
    assert!((weight - 0.2).abs() < 1e-5, "it landed at {weight}");
}

/// An effect with no material is a row an author has not filled in yet, not a crash.
#[test]
fn an_empty_slot_is_skipped() {
    let volume = volume(vec![PostEffect {
        material: None,
        ..effect(Guid::new_v4(), 1.0)
    }]);
    assert!(
        folded(
            &[],
            &[Reached {
                weight: 1.0,
                volume: volume.clone(),
            }]
        )
        .is_empty()
    );
}

/// The scene as the ECS holds it: one volume entity, and whatever the sensors reported.
mod scene {
    use super::*;
    use kooch_core::resource::Resources;
    use kooch_ecs::allocator::EntityAllocator;
    use kooch_ecs::archetype_registry::ArchetypeRegistry;
    use kooch_ecs::component::ComponentRegistry;
    use kooch_ecs::entity::Entity;
    use kooch_ecs::query::AccessTracker;
    use kooch_ecs::sensor_occupancy::SensorOccupancy;

    /// A world holding `volumes`, and the entity each one landed on.
    fn world(volumes: Vec<PostProcessVolume>) -> (Resources, Vec<Entity>) {
        let mut resources = Resources::new();
        let mut allocator = EntityAllocator::new();
        let mut registry = ComponentRegistry::new();
        let mut archetypes = ArchetypeRegistry::new();
        registry.register_cpu_reflected::<PostProcessVolume>();
        let signature = [std::any::TypeId::of::<PostProcessVolume>()]
            .into_iter()
            .collect();
        let archetype = archetypes.get_or_create(signature);
        let entities: Vec<Entity> = volumes
            .into_iter()
            .map(|volume| {
                let entity = allocator.spawn();
                registry
                    .get_cpu_mut::<PostProcessVolume>()
                    .expect("registered")
                    .insert(entity, volume);
                archetypes.register_entity(entity, archetype);
                entity
            })
            .collect();
        resources.insert(allocator);
        resources.insert(registry);
        resources.insert(archetypes);
        resources.insert(AccessTracker::new());
        (resources, entities)
    }

    fn shaped(priority: i32, blend_distance: f32) -> PostProcessVolume {
        PostProcessVolume {
            effects: vec![effect(Guid::new_v4(), 1.0)],
            priority,
            blend_distance,
            ..Default::default()
        }
    }

    /// 🔴 The gate: a volume nobody is inside contributes nothing, and costs nothing. Without the
    /// sensor saying so, every volume in the scene would be measured every frame.
    #[test]
    fn an_empty_volume_is_skipped() {
        let (resources, _) = world(vec![shaped(0, 2.0)]);
        assert!(reached(&resources).is_empty());
    }

    #[test]
    fn an_occupied_volume_weighs_its_depth() {
        let (mut resources, entities) = world(vec![shaped(0, 2.0)]);
        let mut occupancy = SensorOccupancy::default();
        occupancy.enter(entities[0], Entity::new(99, 0), 1.0);
        resources.insert(occupancy);
        let reached = reached(&resources);
        assert_eq!(reached.len(), 1);
        // Half-way in, through the ease: more than nothing, less than all of it.
        assert!((0.0..1.0).contains(&reached[0].weight));
    }

    /// A global volume needs no sensor and no occupancy: it is the scene's own look.
    #[test]
    fn a_global_volume_is_always_on() {
        let global = PostProcessVolume {
            global: true,
            ..shaped(0, 2.0)
        };
        let (resources, _) = world(vec![global]);
        let reached = reached(&resources);
        assert_eq!(reached.len(), 1);
        assert_eq!(reached[0].weight, 1.0);
    }

    /// 🔴 Priority is the order they are applied in, lowest first, so the highest has the last word.
    #[test]
    fn priority_orders_the_fold() {
        let (mut resources, entities) = world(vec![shaped(7, 0.0), shaped(-2, 0.0)]);
        let mut occupancy = SensorOccupancy::default();
        for entity in &entities {
            occupancy.enter(*entity, Entity::new(99, 0), 1.0);
        }
        resources.insert(occupancy);
        let reached = reached(&resources);
        let priorities: Vec<i32> = reached.iter().map(|r| r.volume.priority).collect();
        assert_eq!(priorities, vec![-2, 7]);
    }
}
