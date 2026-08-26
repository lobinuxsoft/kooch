//! The ECS walk, against a real component + archetype registry.
//!
//! The interesting failures here are not arithmetic — they are a light
//! that exists and does not reach the buffer. Every assertion below is
//! some version of "the thing on screen and the thing in the Inspector
//! are the same thing".

use std::any::TypeId;

use glam::{Mat4, Quat, Vec3};

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

use kooch_lighting::{LIGHT_KIND_DIRECTIONAL, LIGHT_KIND_POINT, LIGHT_KIND_SPOT, LightFrame};

/// The walk, as every test here reaches it now.
///
/// It used to be `extract_lights`, next to a second walk in the shadow
/// stage that read the same archetypes again. There is one walk.
fn extract_lights(resources: &Resources) -> kooch_lighting::ExtractedLights {
    let frame = LightFrame::extract(resources);
    frame.lights().clone()
}

fn world() -> Resources {
    let mut r = Resources::new();
    r.insert(EntityAllocator::new());
    r.insert(ComponentRegistry::new());
    r.insert(ArchetypeRegistry::new());
    r.insert(AccessTracker::new());
    r.insert(Commands::new());

    let registry = r.get_mut::<ComponentRegistry>().unwrap();
    registry.register_cpu_reflected::<GlobalTransform>();
    registry.register_cpu_reflected::<DirectionalLight>();
    registry.register_cpu_reflected::<PointLight>();
    registry.register_cpu_reflected::<SpotLight>();
    r
}

fn spawn(resources: &mut Resources) -> Entity {
    let mut commands = resources.remove::<Commands>().unwrap();
    let entity = commands.spawn(resources).id();
    commands.apply(resources);
    resources.insert(commands);
    entity
}

fn insert<T: Component>(resources: &mut Resources, entity: Entity, value: T) {
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<T>()
    {
        storage.insert(entity, value);
    }
    let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() else {
        return;
    };
    let current = match archetypes.entity_archetype(entity) {
        Some(current) => current,
        None => {
            let empty = archetypes.get_or_create(Default::default());
            archetypes.register_entity(entity, empty);
            empty
        }
    };
    let next = archetypes.archetype_after_add_dynamic(current, TypeId::of::<T>());
    archetypes.register_entity(entity, next);
}

fn light_at<T: Component>(resources: &mut Resources, matrix: Mat4, light: T) -> Entity {
    let entity = spawn(resources);
    insert(resources, entity, GlobalTransform { matrix });
    insert(resources, entity, light);
    entity
}

#[test]
fn extracts_every_kind() {
    let mut r = world();
    light_at(&mut r, Mat4::IDENTITY, DirectionalLight::default());
    light_at(
        &mut r,
        Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0)),
        PointLight::default(),
    );
    light_at(
        &mut r,
        Mat4::from_translation(Vec3::new(-4.0, 0.0, 0.0)),
        SpotLight::default(),
    );

    let lights = extract_lights(&r).lights;
    assert_eq!(lights.len(), 3);
    assert!(lights.iter().any(|l| l.kind == LIGHT_KIND_DIRECTIONAL));
    assert!(lights.iter().any(|l| l.kind == LIGHT_KIND_POINT));
    assert!(lights.iter().any(|l| l.kind == LIGHT_KIND_SPOT));
}

#[test]
fn a_point_light_carries_its_world_position() {
    let mut r = world();
    light_at(
        &mut r,
        Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0)),
        PointLight::default(),
    );
    let lights = extract_lights(&r).lights;
    assert_eq!(lights[0].position, [1.0, 2.0, 3.0]);
}

/// The scope correction that #441 was rewritten around: the light's
/// direction is its transform's, not a field and not the sky's sun.
/// Rotating the entity has to move the light.
#[test]
fn a_directional_lights_direction_follows_its_transform() {
    let mut r = world();
    light_at(
        &mut r,
        Mat4::from_quat(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        DirectionalLight::default(),
    );
    let dir = Vec3::from(extract_lights(&r).lights[0].direction);
    assert!(
        dir.abs_diff_eq(Vec3::NEG_Y, 1e-5),
        "expected -Y, got {dir:?}"
    );
}

#[test]
fn inactive_lights_do_not_reach_the_gpu() {
    let mut r = world();
    light_at(
        &mut r,
        Mat4::IDENTITY,
        DirectionalLight {
            active: false,
            ..Default::default()
        },
    );
    assert!(extract_lights(&r).lights.is_empty());
}

/// A light with no transform has no direction and no position. Placing
/// it at the origin pointing down would be an invention, and an
/// invention that renders is one nobody goes looking for.
#[test]
fn a_light_without_a_transform_is_skipped_not_invented() {
    let mut r = world();
    let entity = spawn(&mut r);
    insert(&mut r, entity, PointLight::default());
    assert!(extract_lights(&r).lights.is_empty());
}

#[test]
fn an_empty_world_extracts_nothing_rather_than_panicking() {
    assert!(extract_lights(&world()).lights.is_empty());
}

/// The single-light debug view (#743) addresses a light by its slot in
/// the buffer, and the only thing that decides that slot is the order
/// this walk happens to run in. If a slot ever stops naming the entity
/// it was resolved from, the view isolates a different light than the
/// one selected and looks like a shading bug.
#[test]
fn a_slot_names_the_entity_it_came_from() {
    let mut r = world();
    let sun = light_at(&mut r, Mat4::IDENTITY, DirectionalLight::default());
    let lamp = light_at(&mut r, Mat4::IDENTITY, PointLight::default());
    let torch = light_at(&mut r, Mat4::IDENTITY, SpotLight::default());

    let extracted = extract_lights(&r);
    for (entity, kind) in [
        (sun, LIGHT_KIND_DIRECTIONAL),
        (lamp, LIGHT_KIND_POINT),
        (torch, LIGHT_KIND_SPOT),
    ] {
        let slot = extracted.slot_of(entity).expect("every light has a slot");
        assert_eq!(
            extracted.lights[slot as usize].kind, kind,
            "slot {slot} holds a different light than the entity it resolved from",
        );
    }
}

/// Selecting a crate or the camera is the common case, not an error.
#[test]
fn an_entity_that_is_not_a_light_has_no_slot() {
    let mut r = world();
    light_at(&mut r, Mat4::IDENTITY, DirectionalLight::default());
    let not_a_light = spawn(&mut r);

    assert!(extract_lights(&r).slot_of(not_a_light).is_none());
}

/// An inactive light is not in the buffer, so its slot cannot exist —
/// and the view has to say "nothing selected" rather than point at
/// whichever light shifted down into its index.
#[test]
fn an_inactive_light_has_no_slot() {
    let mut r = world();
    let off = light_at(
        &mut r,
        Mat4::IDENTITY,
        DirectionalLight {
            active: false,
            ..Default::default()
        },
    );
    light_at(&mut r, Mat4::IDENTITY, PointLight::default());

    let extracted = extract_lights(&r);
    assert_eq!(extracted.lights.len(), 1);
    assert!(extracted.slot_of(off).is_none());
}

/// The smoke test that found this: two lights in a scene were switched
/// off, the single-light view rendered magenta for both, and magenta was
/// also what selecting a crate produced. Same pixel, two different
/// fixes — tick a checkbox, or select something else.
///
/// A view built to stop two causes from looking alike does not get to
/// introduce a third pair, so an inactive light says it is inactive.
#[test]
fn an_inactive_light_says_so_instead_of_saying_nothing() {
    let mut r = world();
    let off = light_at(
        &mut r,
        Mat4::IDENTITY,
        PointLight {
            active: false,
            ..Default::default()
        },
    );
    let not_a_light = spawn(&mut r);

    let note = kooch_lighting::shadow_note(&r, off).expect("an inactive light still reports");
    assert!(
        note.contains("inactive"),
        "an inactive light reported {note:?}, which does not name the reason it is invisible",
    );
    assert!(
        kooch_lighting::shadow_note(&r, not_a_light).is_none(),
        "only a non-light reports nothing — that is what the panel turns into \
         `Select a light in the World panel`",
    );
}

/// Every kind, because the checkbox is on all three and the smoke found
/// it on two of them.
#[test]
fn every_light_kind_reports_when_switched_off() {
    let mut r = world();
    let sun = light_at(
        &mut r,
        Mat4::IDENTITY,
        DirectionalLight {
            active: false,
            ..Default::default()
        },
    );
    let lamp = light_at(
        &mut r,
        Mat4::IDENTITY,
        PointLight {
            active: false,
            ..Default::default()
        },
    );
    let torch = light_at(
        &mut r,
        Mat4::IDENTITY,
        SpotLight {
            active: false,
            ..Default::default()
        },
    );

    for entity in [sun, lamp, torch] {
        let note = kooch_lighting::shadow_note(&r, entity).expect("reports while off");
        assert!(note.contains("inactive"), "got {note:?}");
    }
}

/// 🔴 `SpotLight::outer_angle` is in DEGREES and is a half-angle. The
/// shadow frustum is built in radians, and 45 taken for radians is a
/// 2578° cone: it clamps to the widest frustum allowed and produces a
/// map covering a hemisphere for a light that lights a doorway. Nothing
/// about that looks like a unit bug on screen.
#[test]
fn a_spots_shadow_angle_is_converted_to_radians() {
    let mut r = world();
    light_at(
        &mut r,
        Mat4::IDENTITY,
        SpotLight {
            active: true,
            cast_shadows: true,
            outer_angle: 45.0,
            ..Default::default()
        },
    );

    let spots = LightFrame::extract(&r).spot_shadows()
        [..4.min(LightFrame::extract(&r).spot_shadows().len())]
        .to_vec();
    assert_eq!(spots.len(), 1);
    assert!(
        (spots[0].outer_angle - std::f32::consts::FRAC_PI_4).abs() < 1e-5,
        "45 degrees should reach the frustum as {} radians, got {}",
        std::f32::consts::FRAC_PI_4,
        spots[0].outer_angle,
    );
}

/// A spot that does not cast has no source, so it takes no layer from
/// one that does.
#[test]
fn only_casting_spots_get_a_shadow_source() {
    let mut r = world();
    light_at(
        &mut r,
        Mat4::IDENTITY,
        SpotLight {
            active: true,
            cast_shadows: false,
            ..Default::default()
        },
    );
    assert!(
        LightFrame::extract(&r).spot_shadows()
            [..4.min(LightFrame::extract(&r).spot_shadows().len())]
            .to_vec()
            .is_empty()
    );
}

/// 🔴 The froxel grid does not cluster directional lights — they reach
/// every cell — so the shading loop walks the first `directional_count`
/// entries linearly and takes the rest from its cell.
///
/// That is only correct while they are a **prefix**. If a point light
/// ever lands before a directional one, the linear walk lights the wrong
/// light and the sun goes missing from the clustered half: a scene lit
/// by lamps with no daylight, and nothing anywhere reporting it.
#[test]
fn directional_lights_are_a_prefix() {
    // Spawned deliberately out of order: a point, then a directional,
    // then a spot. The walk is what has to put them right, not the
    // order somebody happened to author them in.
    let mut resources = world();
    light_at(
        &mut resources,
        Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0)),
        PointLight::default(),
    );
    light_at(&mut resources, Mat4::IDENTITY, DirectionalLight::default());
    light_at(
        &mut resources,
        Mat4::from_translation(Vec3::new(-4.0, 0.0, 0.0)),
        SpotLight::default(),
    );

    let extracted = extract_lights(&resources);
    assert_eq!(extracted.directional_count, 1);
    for (i, light) in extracted.lights.iter().enumerate() {
        let directional = light.kind == LIGHT_KIND_DIRECTIONAL;
        assert_eq!(
            directional,
            (i as u32) < extracted.directional_count,
            "light {i} of kind {} breaks the prefix",
            light.kind,
        );
    }
}
