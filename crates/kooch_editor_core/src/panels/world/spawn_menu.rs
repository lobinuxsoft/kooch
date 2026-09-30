//! "Spawn" dropdown menu in the World panel toolbar — one entry per commonly-spawned entity
//! archetype.

use std::any::TypeId;

use kooch_camera::orbit::CameraOrbit;
use kooch_camera::{OrbitalFollow, PanTilt, RotationComposer, ThirdPersonFollow, VirtualCamera};
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
fn display_name(stem: &str) -> String {
    let mut chars = stem.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// The components an Orbital Follow rig is, in the order the menu spawns them.
///
/// 🔴 Named rather than written inline so a test can assert the set is one the rig does not report
/// on: since #1397 the components **are** the rig, and a menu that spawns an incomplete one is the
/// new way to get this wrong (#1399).
pub(crate) fn orbital_rig() -> Vec<TypeId> {
    vec![
        TypeId::of::<VirtualCamera>(),
        TypeId::of::<OrbitalFollow>(),
        TypeId::of::<RotationComposer>(),
        TypeId::of::<CameraOrbit>(),
    ]
}

/// The same for a Third Person Follow rig.
pub(crate) fn shoulder_rig() -> Vec<TypeId> {
    vec![
        TypeId::of::<VirtualCamera>(),
        TypeId::of::<ThirdPersonFollow>(),
        TypeId::of::<PanTilt>(),
        TypeId::of::<CameraOrbit>(),
    ]
}

/// The spawn entries, shared by every menu that offers them.
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
            // 🔴 The whole rig, because since #1397 the components **are** the rig and a bare vcam
            // is one the engine reports as incomplete the moment it spawns. Assembling four
            // components from memory is the way this is got wrong now (#1399).
            if ui.button("Orbital Follow Camera").clicked() {
                actions.push(EditorAction::Spawn {
                    into,
                    extra: orbital_rig(),
                    name: Some("Orbital Follow Camera".to_owned()),
                });
                ui.close();
            }
            // `PanTilt`, not a composer: a composer turns back to re-frame the target and cancels
            // the shoulder's effect on screen — measured at −0.117 against 0.000 (#1379).
            if ui.button("Third Person Follow Camera").clicked() {
                actions.push(EditorAction::Spawn {
                    into,
                    extra: shoulder_rig(),
                    name: Some("Third Person Follow Camera".to_owned()),
                });
                ui.close();
            }
            // Kept: a rig assembled by hand is a legitimate thing to want, and a menu that removed
            // the option would be deciding for the author.
            if ui.button("Virtual Camera (bare)").clicked() {
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
        block_menu(ui, actions, into);
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

/// One entry per block shape, spawned with its defaults. The parameters live on the block's
/// `BlockShape`, so they are tuned in the Inspector and seen in the scene as they change (#1106).
fn block_menu(
    ui: &mut egui::Ui,
    actions: &mut Vec<EditorAction>,
    into: crate::actions::SpawnTarget,
) {
    ui.menu_button(format!("{} Block", icons::CUBE), |ui| {
        for shape in kooch_blockmesh::Shape::DEFAULTS {
            if ui.button(shape.label()).clicked() {
                actions.push(EditorAction::SpawnBlock { into, shape });
                ui.close();
            }
        }
    });
}

#[cfg(test)]
mod tests;
