//! "Spawn" dropdown menu in the World panel toolbar — one entry per
//! commonly-spawned entity archetype, plus an SDF / Lights submenu.

use std::any::TypeId;

use ome_ecs::directional_light::DirectionalLight;
use ome_ecs::mesh_renderer::MeshRenderer;
use ome_ecs::orthographic_camera::OrthographicCamera;
use ome_ecs::perspective_camera::PerspectiveCamera;
use ome_ecs::point_light::PointLight;
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
        if ui.button("Mesh Renderer").clicked() {
            actions.push(EditorAction::Spawn {
                extra: vec![TypeId::of::<MeshRenderer>()],
                name: Some("Mesh".to_owned()),
            });
            ui.close();
        }
        ui.menu_button("3D Object", |ui| {
            if ui.button("Suzanne (demo)").clicked() {
                actions.push(EditorAction::SpawnMesh {
                    path: std::path::PathBuf::from("meshes/suzanne.glb"),
                    name: "Suzanne".to_owned(),
                });
                ui.close();
            }
        });
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
