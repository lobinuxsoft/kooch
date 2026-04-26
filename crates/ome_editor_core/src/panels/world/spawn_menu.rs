//! "Spawn" dropdown menu in the World panel toolbar — one entry per
//! commonly-spawned entity archetype, plus an SDF / Lights submenu.

use std::any::TypeId;

use ome_ecs::directional_light::DirectionalLight;
use ome_ecs::mesh_renderer::MeshRenderer;
use ome_ecs::orthographic_camera::OrthographicCamera;
use ome_ecs::perspective_camera::PerspectiveCamera;
use ome_ecs::point_light::PointLight;
use ome_ecs::sdf_box::SdfBox;
use ome_ecs::sdf_capsule::SdfCapsule;
use ome_ecs::sdf_cylinder::SdfCylinder;
use ome_ecs::sdf_plane::SdfPlane;
use ome_ecs::sdf_sphere::SdfSphere;
use ome_ecs::sdf_torus::SdfTorus;
use ome_ecs::sky_renderer::SkyRenderer;
use ome_ecs::spot_light::SpotLight;

use crate::actions::EditorAction;
use crate::icons;

/// Renders the "+ Spawn" menu button. Pushes one or more
/// `EditorAction::Spawn` entries to `actions` when the user picks an item.
pub(super) fn draw_spawn_menu(ui: &mut egui::Ui, actions: &mut Vec<EditorAction>) {
    ui.menu_button(format!("{} Spawn", icons::PLUS), |ui| {
        if ui.button(format!("{} Entity", icons::CUBE)).clicked() {
            actions.push(EditorAction::Spawn { extra: vec![], name: None });
            ui.close();
        }
        ui.separator();
        if ui.button("Perspective Camera").clicked() {
            actions.push(EditorAction::Spawn {
                extra: vec![TypeId::of::<PerspectiveCamera>()],
                name: Some("Perspective Camera".to_owned()),
            });
            ui.close();
        }
        if ui.button("Orthographic Camera").clicked() {
            actions.push(EditorAction::Spawn {
                extra: vec![TypeId::of::<OrthographicCamera>()],
                name: Some("Orthographic Camera".to_owned()),
            });
            ui.close();
        }
        ui.separator();
        ui.menu_button("SDF Shapes", |ui| {
            if ui.button("Sphere").clicked() {
                actions.push(EditorAction::Spawn {
                    extra: vec![TypeId::of::<SdfSphere>()],
                    name: Some("SDF Sphere".to_owned()),
                });
                ui.close();
            }
            if ui.button("Box").clicked() {
                actions.push(EditorAction::Spawn {
                    extra: vec![TypeId::of::<SdfBox>()],
                    name: Some("SDF Box".to_owned()),
                });
                ui.close();
            }
            if ui.button("Capsule").clicked() {
                actions.push(EditorAction::Spawn {
                    extra: vec![TypeId::of::<SdfCapsule>()],
                    name: Some("SDF Capsule".to_owned()),
                });
                ui.close();
            }
            if ui.button("Cylinder").clicked() {
                actions.push(EditorAction::Spawn {
                    extra: vec![TypeId::of::<SdfCylinder>()],
                    name: Some("SDF Cylinder".to_owned()),
                });
                ui.close();
            }
            if ui.button("Torus").clicked() {
                actions.push(EditorAction::Spawn {
                    extra: vec![TypeId::of::<SdfTorus>()],
                    name: Some("SDF Torus".to_owned()),
                });
                ui.close();
            }
            if ui.button("Plane").clicked() {
                actions.push(EditorAction::Spawn {
                    extra: vec![TypeId::of::<SdfPlane>()],
                    name: Some("SDF Plane".to_owned()),
                });
                ui.close();
            }
        });
        if ui.button("Mesh Renderer").clicked() {
            actions.push(EditorAction::Spawn {
                extra: vec![TypeId::of::<MeshRenderer>()],
                name: Some("Mesh".to_owned()),
            });
            ui.close();
        }
        if ui.button("Sky").clicked() {
            actions.push(EditorAction::Spawn {
                extra: vec![TypeId::of::<SkyRenderer>()],
                name: Some("Sky".to_owned()),
            });
            ui.close();
        }
        ui.menu_button("Lights", |ui| {
            if ui.button("Directional Light").clicked() {
                actions.push(EditorAction::Spawn {
                    extra: vec![TypeId::of::<DirectionalLight>()],
                    name: Some("Directional Light".to_owned()),
                });
                ui.close();
            }
            if ui.button("Point Light").clicked() {
                actions.push(EditorAction::Spawn {
                    extra: vec![TypeId::of::<PointLight>()],
                    name: Some("Point Light".to_owned()),
                });
                ui.close();
            }
            if ui.button("Spot Light").clicked() {
                actions.push(EditorAction::Spawn {
                    extra: vec![TypeId::of::<SpotLight>()],
                    name: Some("Spot Light".to_owned()),
                });
                ui.close();
            }
        });
    });
}
