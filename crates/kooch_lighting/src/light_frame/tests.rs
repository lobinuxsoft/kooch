use glam::{Mat4, Vec3};
use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::{Component, ComponentRegistry};
use kooch_ecs::directional_light::DirectionalLight;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::point_light::PointLight;
use kooch_ecs::query::AccessTracker;
use kooch_ecs::spot_light::SpotLight;

use super::LightFrame;

fn world() -> Resources {
    let mut resources = Resources::new();
    resources.insert(ComponentRegistry::new());
    resources.insert(ArchetypeRegistry::new());
    resources.insert(AccessTracker::new());
    resources.insert(EntityAllocator::new());
    resources
}

fn light_at<T: Component>(resources: &mut Resources, at: Vec3, light: T) -> Entity {
    let mut commands = Commands::new();
    let entity = commands
        .spawn(resources)
        .insert(light)
        .insert(GlobalTransform {
            matrix: Mat4::from_translation(at),
        })
        .id();
    commands.apply(resources);
    entity
}

fn sun(cast_shadows: bool) -> DirectionalLight {
    DirectionalLight {
        cast_shadows,
        ..Default::default()
    }
}

fn lamp(cast_shadows: bool) -> PointLight {
    PointLight {
        cast_shadows,
        ..Default::default()
    }
}

fn torch(cast_shadows: bool) -> SpotLight {
    SpotLight {
        cast_shadows,
        ..Default::default()
    }
}

/// 🎯 The point of the whole change: one walk answers both questions, and
/// they cannot disagree because there is nothing to disagree with.
#[test]
fn one_walk_answers_both_questions() {
    let mut r = world();
    light_at(&mut r, Vec3::ZERO, sun(true));
    light_at(&mut r, Vec3::X, lamp(true));
    light_at(&mut r, Vec3::Y, torch(true));

    let frame = LightFrame::extract(&r);

    assert_eq!(frame.lights().lights.len(), 3, "all three reach the GPU");
    assert!(frame.sun().is_some());
    assert_eq!(frame.point_shadows().len(), 1);
    assert_eq!(frame.spot_shadows().len(), 1);
}

/// 🔴 The invariant a second walk could break silently: a shadow source's
/// slot has to index the very light it came from. Two walks agreed only
/// by luck of ordering; one walk records the slot as it pushes.
#[test]
fn a_slot_points_at_its_own_light() {
    let mut r = world();
    light_at(&mut r, Vec3::ZERO, sun(false));
    let lamp_entity = light_at(&mut r, Vec3::X, lamp(true));
    let torch_entity = light_at(&mut r, Vec3::Y, torch(true));

    let frame = LightFrame::extract(&r);
    let entities = &frame.lights().entities;

    let point = &frame.point_shadows()[0];
    assert_eq!(point.entity, lamp_entity);
    assert_eq!(entities[point.buffer_slot as usize], lamp_entity);

    let spot = &frame.spot_shadows()[0];
    assert_eq!(spot.entity, torch_entity);
    assert_eq!(entities[spot.buffer_slot as usize], torch_entity);
}

/// A light that does not cast still lights the scene. It belongs in the
/// buffer and in neither shadow list.
#[test]
fn a_non_caster_lights_without_casting() {
    let mut r = world();
    light_at(&mut r, Vec3::X, lamp(false));
    light_at(&mut r, Vec3::Y, torch(false));

    let frame = LightFrame::extract(&r);

    assert_eq!(frame.lights().lights.len(), 2);
    assert!(frame.point_shadows().is_empty());
    assert!(frame.spot_shadows().is_empty());
    assert!(frame.sun().is_none());
}

/// The directional lights are a PREFIX of the buffer, not a subset — the
/// shading loop walks `0..directional_count` and is only right while they
/// come first.
#[test]
fn the_directionals_are_a_prefix() {
    let mut r = world();
    light_at(&mut r, Vec3::X, lamp(false));
    light_at(&mut r, Vec3::ZERO, sun(false));
    light_at(&mut r, Vec3::Y, torch(false));

    let frame = LightFrame::extract(&r);

    assert_eq!(frame.lights().directional_count, 1);
    assert_eq!(
        frame.lights().lights[0].kind,
        crate::LIGHT_KIND_DIRECTIONAL,
        "the sun sorted to the front regardless of spawn order"
    );
}

/// 🔴 The budget is decided in ONE place. A second walk that truncated
/// afterwards would hand a spot a slot the shadow pass never filled.
#[test]
fn the_spot_budget_is_decided_once() {
    let mut r = world();
    for i in 0..(crate::MAX_SPOT_SHADOWS + 3) {
        light_at(&mut r, Vec3::X * i as f32, torch(true));
    }

    let frame = LightFrame::extract(&r);

    assert_eq!(frame.spot_shadows().len(), crate::MAX_SPOT_SHADOWS);
    // And every spot that got a source also got a slot in its GpuLight.
    for (slot, source) in frame.spot_shadows().iter().enumerate() {
        let light = &frame.lights().lights[source.buffer_slot as usize];
        assert_eq!(
            light.shadow_slot, slot as u32,
            "the source and the light agree on the slot"
        );
    }
}

#[test]
fn an_empty_world_extracts_nothing() {
    let frame = LightFrame::extract(&world());

    assert!(frame.lights().lights.is_empty());
    assert!(frame.sun().is_none());
    assert!(frame.point_shadows().is_empty());
    assert!(frame.spot_shadows().is_empty());
}

/// Ranking is the only part that depends on where anyone stands, so it
/// stays out of the walk — and the nearer lamp wins.
#[test]
fn ranking_puts_the_nearer_lamp_first() {
    let mut r = world();
    let far = light_at(&mut r, Vec3::X * 100.0, lamp(true));
    let near = light_at(&mut r, Vec3::X * 2.0, lamp(true));

    let frame = LightFrame::extract(&r);
    let ranked = frame.ranked_points(Vec3::ZERO, 2);

    assert_eq!(ranked[0].entity, near);
    assert_eq!(ranked[1].entity, far);
    assert_eq!(
        frame.ranked_points(Vec3::ZERO, 1).len(),
        1,
        "the cut applies"
    );
}

/// 🔴 Despawn is **deferred**: `EntityAllocator::despawn` queues into
/// `pending_despawn`, so between it and the next sync the archetype still
/// lists an entity that is gone. The walk has to ask.
#[test]
fn a_despawned_light_is_skipped() {
    let mut r = world();
    let doomed = light_at(&mut r, Vec3::X, lamp(true));
    light_at(&mut r, Vec3::Y, lamp(true));

    assert_eq!(LightFrame::extract(&r).lights().lights.len(), 2);

    // Straight at the allocator, which is what makes the removal
    // deferred — the archetype has not been told yet.
    r.get_mut::<EntityAllocator>().unwrap().despawn(doomed);

    let frame = LightFrame::extract(&r);
    assert_eq!(frame.lights().lights.len(), 1, "the dead one is not lit");
    assert_eq!(frame.point_shadows().len(), 1, "nor does it cast");
    assert!(!frame.lights().entities.contains(&doomed));
}

/// 🔴 The walk is shared by every view of a frame, and each view makes its
/// own cube selection — so `assign_point_slots` runs more than once over
/// the same lights. A lamp the previous view picked must not still be
/// holding that view's slot, or it samples a cube drawn for somebody else.
#[test]
fn a_second_selection_replaces_the_first() {
    use kooch_ecs::entity::Entity as E;

    let mut r = world();
    let a = light_at(&mut r, Vec3::X, lamp(true));
    let b = light_at(&mut r, Vec3::Y, lamp(true));
    let c = light_at(&mut r, Vec3::Z, lamp(true));

    let mut frame = LightFrame::extract(&r);
    let slot_of = |frame: &LightFrame, entity: E| {
        let index = frame.lights().slot_of(entity).unwrap() as usize;
        frame.lights().lights[index].shadow_slot
    };

    crate::extract::assign_point_slots(frame.lights_mut(), &[a, b]);
    assert_eq!(slot_of(&frame, a), 0);
    assert_eq!(slot_of(&frame, b), 1);
    assert_eq!(slot_of(&frame, c), crate::NO_SHADOW_SLOT);

    // A second view picks a different lamp.
    crate::extract::assign_point_slots(frame.lights_mut(), &[c]);
    assert_eq!(slot_of(&frame, c), 0);
    assert_eq!(
        slot_of(&frame, a),
        crate::NO_SHADOW_SLOT,
        "the first view's pick let go of its slot"
    );
    assert_eq!(slot_of(&frame, b), crate::NO_SHADOW_SLOT);
}

/// And it must not touch the spots. Their slots are handed out during the
/// walk and belong to nobody else.
#[test]
fn a_selection_leaves_the_spots_alone() {
    let mut r = world();
    let torch_entity = light_at(&mut r, Vec3::Y, torch(true));
    let lamp_entity = light_at(&mut r, Vec3::X, lamp(true));

    let mut frame = LightFrame::extract(&r);
    let spot_slot = |frame: &LightFrame| {
        let index = frame.lights().slot_of(torch_entity).unwrap() as usize;
        frame.lights().lights[index].shadow_slot
    };
    assert_eq!(spot_slot(&frame), 0, "handed out during the walk");

    crate::extract::assign_point_slots(frame.lights_mut(), &[lamp_entity]);

    assert_eq!(spot_slot(&frame), 0, "still the spot's own slot");
}
