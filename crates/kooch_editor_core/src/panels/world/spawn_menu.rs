//! "Spawn" dropdown menu in the World panel toolbar — one entry per commonly-spawned entity
//! archetype.

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
fn display_name(stem: &str) -> String {
    let mut chars = stem.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
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

/// One entry per block shape, each holding the parameters it will spawn with (#1106). Kept in egui
/// memory, so a staircase tuned once stays tuned for the session.
fn block_menu(
    ui: &mut egui::Ui,
    actions: &mut Vec<EditorAction>,
    into: crate::actions::SpawnTarget,
) {
    ui.menu_button(format!("{} Block", icons::CUBE), |ui| {
        let id = ui.make_persistent_id("block_shapes");
        let mut shapes: Vec<kooch_blockmesh::Shape> = ui
            .data_mut(|data| data.get_temp(id))
            .unwrap_or_else(|| kooch_blockmesh::Shape::DEFAULTS.to_vec());
        for shape in &mut shapes {
            ui.menu_button(shape.label(), |ui| {
                shape_fields(ui, shape);
                ui.separator();
                if ui.button("Spawn").clicked() {
                    actions.push(EditorAction::SpawnBlock {
                        into,
                        shape: *shape,
                    });
                    ui.close();
                }
            });
        }
        ui.data_mut(|data| data.insert_temp(id, shapes));
    });
}

/// The parameters of one shape, as drag fields.
fn shape_fields(ui: &mut egui::Ui, shape: &mut kooch_blockmesh::Shape) {
    use kooch_blockmesh::Shape;
    let count = |ui: &mut egui::Ui, label: &str, value: &mut u32, min: u32| {
        ui.horizontal(|ui| {
            ui.label(label);
            ui.add(egui::DragValue::new(value).range(min..=128));
        });
    };
    let length = |ui: &mut egui::Ui, label: &str, value: &mut f32| {
        ui.horizontal(|ui| {
            ui.label(label);
            ui.add(
                egui::DragValue::new(value)
                    .speed(0.05)
                    .range(0.01..=1000.0)
                    .suffix(" m"),
            );
        });
    };
    match shape {
        Shape::Cube { size } => {
            length(ui, "X", &mut size.x);
            length(ui, "Y", &mut size.y);
            length(ui, "Z", &mut size.z);
        }
        Shape::Stairs {
            steps,
            width,
            rise,
            run,
        } => {
            count(ui, "Steps", steps, 1);
            length(ui, "Width", width);
            length(ui, "Rise", rise);
            length(ui, "Run", run);
        }
        Shape::Ramp { width, rise, run } => {
            length(ui, "Width", width);
            length(ui, "Rise", rise);
            length(ui, "Run", run);
        }
        Shape::Arch {
            segments,
            radius,
            thickness,
            depth,
        } => {
            count(ui, "Segments", segments, 1);
            length(ui, "Radius", radius);
            length(ui, "Thickness", thickness);
            length(ui, "Depth", depth);
        }
        Shape::Cylinder {
            sides,
            radius,
            height,
        }
        | Shape::Cone {
            sides,
            radius,
            height,
        } => {
            count(ui, "Sides", sides, 3);
            length(ui, "Radius", radius);
            length(ui, "Height", height);
        }
        Shape::Plane {
            subdivisions,
            size,
            thickness,
        } => {
            count(ui, "Subdivisions", subdivisions, 1);
            length(ui, "Size", size);
            length(ui, "Thickness", thickness);
        }
    }
}
