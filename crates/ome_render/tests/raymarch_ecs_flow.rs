//! End-to-end ECS flow regression for the raymarch SDF render path
//! (issue #351). Verifies that the deferred-spawn → transform-propagation
//! → render-query chain converges to a state where the raymarch scene
//! collector sees every visible `SdfSphere` after a deterministic
//! number of frame stages.
//!
//! The historical bug rendered only the sky gradient because the
//! `Query<(Entity, &SdfSphere, &GlobalTransform, ..)>` used by the
//! collector matched zero archetypes — `GlobalTransform` had not been
//! populated yet. This test re-runs the same `App` lifecycle the
//! `raymarch_demo` example uses (sans windowing / GPU) and asserts on
//! the per-frame visibility timeline.
//!
//! It does NOT exercise the GPU BVH (`bvh.current_n()`) — that path is
//! covered by the GPU-gated tests under `bvh/gpu_tests/`. This test
//! is CPU-only so it runs in CI without a graphics adapter.

use glam::{Quat, Vec3, Vec4};
use ome_core::prelude::*;
use ome_ecs::commands::Commands;
use ome_ecs::hierarchy::GlobalTransform;
use ome_ecs::query::Query;
use ome_ecs::{EcsPlugin, PerspectiveCamera, SdfSphere, Transform};

fn spawn_three_spheres(resources: &mut Resources) {
    let Some(mut commands) = resources.remove::<Commands>() else {
        return;
    };

    commands
        .spawn(resources)
        .insert(Transform {
            position: Vec3::new(0.0, 1.0, 5.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        })
        .insert(PerspectiveCamera {
            active: true,
            priority: 0,
            fov: 60.0,
            near: 0.1,
            far: 200.0,
            clear_color: Vec4::new(0.1, 0.2, 0.4, 1.0),
        });

    for (x, r) in [(-2.0_f32, 0.8_f32), (0.0, 1.0), (2.0, 0.6)] {
        commands
            .spawn(resources)
            .insert(Transform {
                position: Vec3::new(x, 0.0, 0.0),
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            })
            .insert(SdfSphere {
                visible: true,
                collide: false,
                radius: r,
            });
    }

    resources.insert(commands);
}

fn count_visible_spheres(app: &App) -> usize {
    let q = Query::<(&SdfSphere, &GlobalTransform)>::new(&app.resources);
    q.iter().count()
}

/// The raymarch collector query is `(Entity, &SdfSphere,
/// &GlobalTransform, Option<&SdfBlend>)`. Mirrors the same matching
/// rule (`SdfSphere AND GlobalTransform`) without depending on
/// `ome_render` internals.
#[test]
fn three_spheres_visible_to_render_query_after_deferred_spawn() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugin(EcsPlugin);
    app.add_system(Stage::Startup, spawn_three_spheres);
    app.finish_plugins();

    // Startup runs the EcsPlugin's `register_builtin_components` and
    // then `spawn_three_spheres`, which buffers four entity spawns
    // (1 camera + 3 spheres) into `Commands`.
    app.schedule.run_startup(&mut app.resources);

    // Frame 1: PostUpdate's `transform_propagation_system` runs BEFORE
    // GpuSync's `commands_apply_system`, so no `Transform` is in
    // storage yet — no `GlobalTransform` is produced. The render
    // query matches nothing.
    app.schedule.run_frame_stages(&mut app.resources);
    assert_eq!(
        count_visible_spheres(&app),
        0,
        "frame 1: spawns are still buffered when transform_propagation runs",
    );

    // Frame 2: GpuSync's `commands_apply_system` ran in frame 1, so
    // `Transform` exists in storage. PostUpdate's
    // `transform_propagation_system` now produces `GlobalTransform`
    // for every entity with a `Transform`, and the in-system
    // archetype sync at `transform_propagation.rs:116-129` migrates
    // those entities into archetypes that include `GlobalTransform`
    // — the render query starts matching all 3 spheres.
    app.schedule.run_frame_stages(&mut app.resources);
    assert_eq!(
        count_visible_spheres(&app),
        3,
        "frame 2: all 3 spheres visible to the render query",
    );

    // Steady state: subsequent frames keep matching all 3 spheres.
    // Guards against a regression where `register_entity` cycles the
    // entity in/out of the `+GlobalTransform` archetype each frame.
    for frame in 3..=8 {
        app.schedule.run_frame_stages(&mut app.resources);
        assert_eq!(
            count_visible_spheres(&app),
            3,
            "frame {frame}: steady-state match must remain stable",
        );
    }
}
