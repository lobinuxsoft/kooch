//! Selecting an entity must write nothing: every widget drawn with no input has to report no edit,
//! or the scene turns dirty on a click in the World panel.

use std::collections::HashMap;

use kooch_ecs::component::ComponentId;
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::{Reflect, ReflectValue};

use super::{RotationContext, RotationDisplayMode};

/// Draws `value`'s fields for a few frames with no input, returning every edit reported.
fn idle<T: Reflect>(value: &T) -> Vec<(String, ReflectValue)> {
    let _guard = crate::panels::id_stability_probe::PROBE_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let metas = value.reflect_fields();
    let fields: Vec<(String, ReflectValue)> = metas
        .iter()
        .filter_map(|meta| Some((meta.name.to_owned(), value.reflect_get(meta.name)?)))
        .collect();
    let ctx = egui::Context::default();
    let mut cache = HashMap::new();
    let mut edits = Vec::new();
    for _ in 0..3 {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(600.0, 1200.0),
            )),
            ..Default::default()
        };
        ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                edits.extend(super::single::draw_reflected_fields(
                    ui,
                    Entity::new(1, 0),
                    Some(std::any::TypeId::of::<T>()),
                    ComponentId(7),
                    &fields,
                    Some(metas),
                    &mut cache,
                    RotationContext {
                        mode: RotationDisplayMode::Local,
                        self_global: None,
                        parent_global: None,
                    },
                    &[],
                    &[],
                    &[],
                ));
            });
        });
    }
    edits
}

#[test]
fn idle_components_write_nothing() {
    let transform = kooch_ecs::transform::Transform {
        position: glam::Vec3::new(0.0, 8.0, 18.0),
        rotation: glam::Quat::from_euler(glam::EulerRot::XYZ, -0.0, 0.0, -0.0),
        ..Default::default()
    };
    let cases: Vec<(&str, Vec<(String, ReflectValue)>)> = vec![
        ("Transform", idle(&transform)),
        (
            "VirtualCamera",
            idle(&kooch_camera::VirtualCamera::default()),
        ),
        (
            "CameraCollision",
            idle(&kooch_camera::CameraCollision::default()),
        ),
        (
            "CameraFraming",
            idle(&kooch_camera::CameraFraming::default()),
        ),
        (
            "PerspectiveCamera",
            idle(&kooch_ecs::PerspectiveCamera::default()),
        ),
        (
            "Collider",
            idle(&kooch_physics::components::Collider::default()),
        ),
        (
            "PhysicsBody",
            idle(&kooch_physics::components::PhysicsBody::default()),
        ),
        ("Sprint", idle(&kooch_character::Sprint::default())),
    ];
    let dirty: Vec<_> = cases
        .into_iter()
        .filter(|(_, edits)| !edits.is_empty())
        .collect();
    assert!(
        dirty.is_empty(),
        "drawn with no input, these wrote: {dirty:?}"
    );
}
