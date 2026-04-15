//! Minimal ray-marching demo: spawns a camera + a few SDF spheres and
//! renders them with the sphere-tracing fragment shader.
//!
//! Run with: cargo run --example raymarch_demo

use glam::{Quat, Vec3, Vec4};
use ome_core::prelude::*;
use ome_ecs::commands::Commands;
use ome_ecs::{EcsPlugin, PerspectiveCamera, SdfSphere, Transform};
use ome_render::RayMarchPlugin;
use ome_window::WindowPlugin;

fn spawn_scene(resources: &mut Resources) {
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

fn main() {
    ome_core::init_tracing();

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugin(WindowPlugin {
        title: "Ray March Demo".into(),
        width: 1280,
        height: 720,
    });
    app.add_plugin(EcsPlugin);
    app.add_plugin(RayMarchPlugin::default());
    app.add_system(Stage::Startup, spawn_scene);
    app.run();
}
