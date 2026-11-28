//! The buttons that bake a terrain field (#709) out of a mesh or an image.

use kooch_core::Guid;

use crate::actions::EditorAction;

const RESOLUTION_ID: &str = "sdf_field_resolution";
const DEFAULT_RESOLUTION: u32 = 64;

/// Resolution, then the button that bakes the mesh into a distance field.
pub(super) fn draw_sdf_bake(ui: &mut egui::Ui, guid: Guid, actions: &mut Vec<EditorAction>) {
    let id = egui::Id::new(RESOLUTION_ID).with(guid);
    let mut resolution = ui.data_mut(|d| *d.get_temp_mut_or(id, DEFAULT_RESOLUTION));
    ui.horizontal(|ui| {
        ui.add(
            egui::DragValue::new(&mut resolution)
                .range(8..=256)
                .prefix("resolution: "),
        )
        .on_hover_text(
            "Texels along the mesh's longest side. 64 is 512 KiB for a cube-shaped mesh; 128 is 4 MiB",
        );
        if ui
            .button("Create SDF field")
            .on_hover_text(
                "Signed distances around the mesh, written to assets/terrain. A Field stamp \
                 stretches it over its box: the mesh stamped into the ground, overhangs included",
            )
            .clicked()
        {
            actions.push(EditorAction::BakeSdfField {
                source: guid,
                resolution,
            });
        }
    });
    ui.data_mut(|d| d.insert_temp(id, resolution));
}

/// The button that turns the image's brightness into a heightmap field.
pub(super) fn draw_height_bake(ui: &mut egui::Ui, guid: Guid, actions: &mut Vec<EditorAction>) {
    if ui
        .button("Create height field")
        .on_hover_text(
            "The image's brightness as heights, black the lowest and white the highest, written to \
             assets/terrain. 16-bit images keep their precision",
        )
        .clicked()
    {
        actions.push(EditorAction::BakeHeightField { source: guid });
    }
}
