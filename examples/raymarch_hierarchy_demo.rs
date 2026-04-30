//! Reproduces issue #354: BVH cull view-dependent visibility regression
//! of #352 in scenes with >3 SDF entities. Spawns the same entities as
//! `TestEngine2.0/scenes/HierarchyTest.ome_scene` (10 entities — Floor,
//! SphereRoot, BoxRoot, BoxChildBox, SDF Cylinder, SDF Torus, plus a
//! camera) without needing the full editor / scene-loader pipeline.
//!
//! Run with: cargo run --example raymarch_hierarchy_demo
//!
//! Optional env vars:
//!   - `OME_CAM_ANGLE=<degrees>` — camera yaw around the scene origin
//!     (defaults to 0). Use this to capture deterministic screenshots
//!     from multiple angles for the AC.

use glam::{Quat, Vec3, Vec4};
use ome_core::prelude::*;
use ome_ecs::commands::Commands;
use ome_ecs::sdf_blend::{
    MODE_REPLACE, MODE_SMOOTH_INTERSECTION, MODE_SMOOTH_SUBTRACTION, MODE_SMOOTH_UNION,
};
use ome_ecs::{
    EcsPlugin, PerspectiveCamera, SdfBlend, SdfBox, SdfCylinder, SdfSphere, SdfTorus, Transform,
};
use ome_render::RayMarchPlugin;
use ome_window::WindowPlugin;

fn cam_angle_deg() -> f32 {
    std::env::var("OME_CAM_ANGLE")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(0.0)
}

fn spawn_scene(resources: &mut Resources) {
    let Some(mut commands) = resources.remove::<Commands>() else {
        return;
    };

    // Camera — yaw around scene centre at radius 8m, height 2.83m.
    // Scene centre roughly at (0, 0.5, 0); cam looks back toward it.
    let yaw_deg = cam_angle_deg();
    let yaw = yaw_deg.to_radians();
    let cam_pos = Vec3::new(yaw.sin() * 8.0, 2.83, yaw.cos() * 8.0);
    let look_dir = (Vec3::new(0.0, 0.5, 0.0) - cam_pos).normalize();
    let cam_rot = Quat::from_rotation_arc(Vec3::NEG_Z, look_dir);

    commands
        .spawn(resources)
        .insert(Transform {
            position: cam_pos,
            rotation: cam_rot,
            scale: Vec3::ONE,
        })
        .insert(PerspectiveCamera {
            active: true,
            priority: 0,
            fov: 90.0,
            near: 0.1,
            far: 1000.0,
            clear_color: Vec4::new(0.0, 0.0, 0.0, 1.0),
        });

    // Mirror of HierarchyTest.ome_scene SDF entities.
    // `SphereRoot` — SMOOTH_SUBTRACTION (mode 3) at (2.7, 0.66, 0.73), radius 1.
    commands
        .spawn(resources)
        .insert(Transform {
            position: Vec3::new(2.698_904, 0.663_336_4, 0.730_327_6),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        })
        .insert(SdfSphere {
            visible: true,
            collide: false,
            radius: 1.0,
        })
        .insert(SdfBlend {
            mode: MODE_SMOOTH_SUBTRACTION,
            smoothness: 0.2,
        });

    // `Floor` — large box, no SdfBlend (default REPLACE / ADD), at (-1.23, 0, 0.56).
    commands
        .spawn(resources)
        .insert(Transform {
            position: Vec3::new(-1.229_705_8, 0.0, 0.557_777_4),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        })
        .insert(SdfBox {
            visible: true,
            collide: false,
            size: Vec3::new(20.0, 0.1, 20.0),
            rounding: 0.1,
        })
        .insert(SdfBlend {
            mode: MODE_REPLACE,
            smoothness: 0.0,
        });

    // `BoxRoot` — SMOOTH_UNION at (3.22, 0.94, 0.14), 1×1×1 rounded 0.25.
    commands
        .spawn(resources)
        .insert(Transform {
            position: Vec3::new(3.225_889, 0.937_881_23, 0.141_695_98),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        })
        .insert(SdfBox {
            visible: true,
            collide: false,
            size: Vec3::ONE,
            rounding: 0.25,
        })
        .insert(SdfBlend {
            mode: MODE_SMOOTH_UNION,
            smoothness: 0.1,
        });

    // `BoxChildBox` — SMOOTH_UNION rotated quat (0, -0.383, 0, 0.924).
    commands
        .spawn(resources)
        .insert(Transform {
            position: Vec3::new(3.001_953_6, 2.020_014_8, -0.001_248_836_5),
            rotation: Quat::from_xyzw(0.0, -0.382_683_46, 0.0, 0.923_879_5),
            scale: Vec3::ONE,
        })
        .insert(SdfBox {
            visible: true,
            collide: false,
            size: Vec3::splat(0.4),
            rounding: 0.1,
        })
        .insert(SdfBlend {
            mode: MODE_SMOOTH_UNION,
            smoothness: 0.2,
        });

    // `SDF Cylinder` — SMOOTH_UNION at (-2.24, 0.94, 1.98).
    commands
        .spawn(resources)
        .insert(Transform {
            position: Vec3::new(-2.237_854_5, 0.937_351_35, 1.983_192_4),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        })
        .insert(SdfCylinder {
            visible: true,
            collide: false,
            radius: 1.0,
            half_height: 1.0,
        })
        .insert(SdfBlend {
            mode: MODE_SMOOTH_UNION,
            smoothness: 0.1,
        });

    // `SDF Torus` — SMOOTH_UNION at (-1.86, 0.28, 0).
    commands
        .spawn(resources)
        .insert(Transform {
            position: Vec3::new(-1.862_454_4, 0.284_203_95, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        })
        .insert(SdfTorus {
            visible: true,
            collide: false,
            major_radius: 1.0,
            minor_radius: 0.25,
        })
        .insert(SdfBlend {
            mode: MODE_SMOOTH_UNION,
            smoothness: 0.1,
        });

    // Reference flag so the unused-imports warning stays clean if a
    // mode constant is dropped from the scene later — and a sanity
    // check that the four mode constants are wired through.
    let _modes = [
        MODE_REPLACE,
        MODE_SMOOTH_UNION,
        MODE_SMOOTH_INTERSECTION,
        MODE_SMOOTH_SUBTRACTION,
    ];

    resources.insert(commands);
}

fn main() {
    ome_core::init_tracing();

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugin(WindowPlugin {
        title: format!(
            "Ray March Hierarchy Demo — yaw {:>5.1}°",
            cam_angle_deg()
        ),
        width: 1280,
        height: 720,
    });
    app.add_plugin(EcsPlugin);
    app.add_plugin(RayMarchPlugin::default());
    app.add_system(Stage::Startup, spawn_scene);
    app.run();
}
