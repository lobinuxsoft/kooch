//! "Spawn" dropdown menu in the World panel toolbar — one entry per
//! commonly-spawned entity archetype.
//!
//! Anything with more than one variant gets a submenu: `Cameras`,
//! `3D Object`, `Lights`. The cameras were loose at the top level until
//! a third one arrived and made the inconsistency obvious.

use std::any::TypeId;

use kooch_camera::VirtualCamera;
use kooch_ecs::directional_light::DirectionalLight;
use kooch_ecs::mesh_renderer::MeshRenderer;
use kooch_ecs::orthographic_camera::OrthographicCamera;
use kooch_ecs::perspective_camera::PerspectiveCamera;
use kooch_ecs::point_light::PointLight;
use kooch_ecs::sky_renderer::SkyRenderer;
use kooch_ecs::spot_light::SpotLight;

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

/// The spawn entries, shared by every menu that offers them.
///
/// 🔴 One list, four callers: the panel's empty space, a scene header,
/// a row's "New Child" and its "New in This Scene". There used to be a
/// fifth — a toolbar button — and the reason it is gone is the reason
/// this function exists: a second list of the same commands is a list
/// that drifts, and the one that drifts is always the one fewer people
/// use (#591). Every one of these menus names a place, which a toolbar
/// button standing above the list never could.
pub(super) fn spawn_entries(
    ui: &mut egui::Ui,
    actions: &mut Vec<EditorAction>,
    into: crate::actions::SpawnTarget,
) {
    {
        if ui.button(format!("{} Entity", icons::CUBE)).clicked() {
            actions.push(EditorAction::Spawn {
                into,
                extra: vec![],
                name: None,
            });
            ui.close();
        }
        ui.separator();
        ui.menu_button("Cameras", |ui| {
            if ui.button("Perspective Camera").clicked() {
                actions.push(EditorAction::Spawn {
                    into,
                    extra: vec![TypeId::of::<PerspectiveCamera>()],
                    name: Some("Perspective Camera".to_owned()),
                });
                ui.close();
            }
            if ui.button("Orthographic Camera").clicked() {
                actions.push(EditorAction::Spawn {
                    into,
                    extra: vec![TypeId::of::<OrthographicCamera>()],
                    name: Some("Orthographic Camera".to_owned()),
                });
                ui.close();
            }
            ui.separator();
            // Separated because it is not a camera: it is a framing that
            // drives one. Sitting in the same list unmarked invites
            // spawning it and wondering why nothing renders through it.
            if ui.button("Virtual Camera").clicked() {
                actions.push(EditorAction::Spawn {
                    into,
                    extra: vec![TypeId::of::<VirtualCamera>()],
                    name: Some("Virtual Camera".to_owned()),
                });
                ui.close();
            }
        });
        if ui.button("Mesh Renderer").clicked() {
            actions.push(EditorAction::Spawn {
                into,
                extra: vec![TypeId::of::<MeshRenderer>()],
                name: Some("Mesh".to_owned()),
            });
            ui.close();
        }
        ui.menu_button("3D Object", |ui| {
            // Driven by the same list the baker writes, so a primitive
            // cannot appear in the menu without a file behind it.
            for (name, _) in kooch_render::mesh::Primitive::CANONICAL {
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
        // Its own entry rather than a row under "3D Object": those are
        // baked `.glb` files that cannot be edited, and this is the one
        // shape the editor can still change afterwards (#946).
        if ui.button(format!("{} Block", icons::CUBE)).clicked() {
            actions.push(EditorAction::SpawnBlock { into });
            ui.close();
        }
        if ui.button("Sky").clicked() {
            actions.push(EditorAction::Spawn {
                into,
                extra: vec![TypeId::of::<SkyRenderer>()],
                name: Some("Sky".to_owned()),
            });
            ui.close();
        }
        ui.menu_button("Lights", |ui| {
            if ui.button("Directional Light").clicked() {
                actions.push(EditorAction::Spawn {
                    into,
                    extra: vec![TypeId::of::<DirectionalLight>()],
                    name: Some("Directional Light".to_owned()),
                });
                ui.close();
            }
            if ui.button("Point Light").clicked() {
                actions.push(EditorAction::Spawn {
                    into,
                    extra: vec![TypeId::of::<PointLight>()],
                    name: Some("Point Light".to_owned()),
                });
                ui.close();
            }
            if ui.button("Spot Light").clicked() {
                actions.push(EditorAction::Spawn {
                    into,
                    extra: vec![TypeId::of::<SpotLight>()],
                    name: Some("Spot Light".to_owned()),
                });
                ui.close();
            }
        });
    }
}
