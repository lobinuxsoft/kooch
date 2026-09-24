use super::filter::WorldFilter;
use super::*;
use crate::state::EntityDisplayInfo;
use crate::state::ReflectedFields;

fn entity_info(index: u32, scene: Option<kooch_core::Guid>) -> EntityDisplayInfo {
    EntityDisplayInfo {
        is_prefab_instance: false,
        entity: Entity::new(index, 0),
        components: Vec::new(),
        parent: None,
        children: Vec::new(),
        depth: 0,
        global_rotation: None,
        scene,
        parent_global_rotation: None,
    }
}

fn scene_info(id: kooch_core::Guid, active: bool) -> SceneDisplayInfo {
    SceneDisplayInfo {
        id,
        name: "Scene".to_owned(),
        path: None,
        dirty: false,
        active,
    }
}

/// Runs `body` against a real `Ui`, since everything here reads or
/// writes egui's own layout and persisted state.
fn with_ui<R>(body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let ctx = egui::Context::default();
    let mut body = Some(body);
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(400.0, 600.0),
        )),
        ..Default::default()
    };
    let mut out = None;
    ctx.run_ui(input, |ui| {
        let body = body.take().expect("run_ui called the closure twice");
        egui::CentralPanel::default().show(ui, |ui| out = Some(body(ui)));
    });
    out.expect("central panel did not run")
}

mod filter;
mod hierarchy;
mod scrolling;
