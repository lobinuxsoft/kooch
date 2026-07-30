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

/// Title-cases a primitive's asset stem for display: `cube` → `Cube`.
///
/// The stem is the filename, so it has to stay lowercase; the menu entry
/// is what a person reads.
fn display_name(stem: &str) -> String {
    let mut chars = stem.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Renders the "+ Spawn" menu button. Pushes one or more
/// `EditorAction::Spawn` entries to `actions` when the user picks an item.
pub(super) fn draw_spawn_menu(ui: &mut egui::Ui, actions: &mut Vec<EditorAction>) {
    ui.menu_button(format!("{} Spawn", icons::PLUS), |ui| {
        spawn_entries(ui, actions);
    });
}

/// The entries themselves, without the button that opens them.
///
/// Split out so the right-click menu on the World panel's empty space
/// offers exactly what the toolbar does. Two lists would drift, and the
/// one that drifts is always the one fewer people use (#591).
pub(super) fn spawn_entries(ui: &mut egui::Ui, actions: &mut Vec<EditorAction>) {
    {
        if ui.button(format!("{} Entity", icons::CUBE)).clicked() {
            actions.push(EditorAction::Spawn {
                extra: vec![],
                name: None,
            });
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
            // Driven by the same list the baker writes, so a primitive
            // cannot appear in the menu without a file behind it.
            for (name, _) in ome_render::mesh::Primitive::CANONICAL {
                if ui.button(display_name(name)).clicked() {
                    actions.push(EditorAction::SpawnMesh {
                        path: std::path::PathBuf::from(format!("meshes/primitives/{name}.glb")),
                        name: display_name(name),
                    });
                    ui.close();
                }
            }
            ui.separator();
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
    }
}
