//! Running a block's painted faces down a curve (#1526).
//!
//! 🔴 The button pushes the *choice*, not the geometry: this panel draws from a snapshot and has
//! no world to extrude in. `EditorAction::ExtrudeAlong` is built where the mesh can be, once.

use kooch_ecs::entity::Entity;

use crate::actions::EditorAction;
use crate::panels::world::entity_row::display_name_for;
use crate::state::EntityDisplayInfo;

/// What the extrude was last asked for. Lives in egui's own store: it is a widget's position, not
/// editor state, and it must never reach a saved scene.
#[derive(Clone, Copy)]
struct Asked {
    spline: Option<Entity>,
    samples: u32,
    taper: bool,
}

impl Default for Asked {
    fn default() -> Self {
        Self {
            spline: None,
            samples: 16,
            taper: false,
        }
    }
}

/// Draws the operations for a `Block` component, pushing one edit per click.
pub(super) fn draw(
    ui: &mut egui::Ui,
    entity: Entity,
    entities: &[EntityDisplayInfo],
    picked_faces: Option<(Entity, usize)>,
    actions: &mut Vec<EditorAction>,
) {
    let id = egui::Id::new(("block_ops", entity));
    let mut asked = ui
        .ctx()
        .data_mut(|data| data.get_temp::<Asked>(id))
        .unwrap_or_default();

    let faces = match picked_faces {
        Some((owner, count)) if owner == entity => count,
        _ => 0,
    };
    // A curve the author has not chosen, or one that has gone away with the entity holding it.
    let curve = asked
        .spline
        .filter(|spline| entities.iter().any(|info| info.entity == *spline));

    egui::CollapsingHeader::new("Face operations")
        .id_salt(id)
        .show(ui, |ui| {
            ui.group(|ui| {
                ui.set_min_width(ui.available_width());
                ui.label("Extrude along a curve");
                ui.horizontal(|ui| {
                    ui.label("curve");
                    pick_curve(ui, &mut asked.spline, entities, entity);
                });
                ui.horizontal(|ui| {
                    ui.label("rings");
                    ui.add(crate::numeric::drag(&mut asked.samples).range(1..=256))
                        .on_hover_text(
                            "How many sections it lays down, spaced evenly in metres along the \
                             curve rather than in parameter.",
                        );
                    ui.checkbox(&mut asked.taper, "taper").on_hover_text(
                        "Scale each section by the knot radius there, so a path can narrow what \
                         it carries.",
                    );
                });
                let ready = faces > 0 && curve.is_some();
                ui.horizontal(|ui| {
                    ui.label(match faces {
                        0 => "no faces painted".to_owned(),
                        1 => "1 face".to_owned(),
                        many => format!("{many} faces"),
                    });
                    if ui
                        .add_enabled(ready, egui::Button::new("Extrude"))
                        .on_hover_text(
                            "Run the painted faces down the curve. The faces keep the way they \
                             were modelled; the curve only carries them from there.",
                        )
                        .on_disabled_hover_text(why_not(faces, curve.is_some()))
                        .clicked()
                        && let Some(spline) = curve
                    {
                        actions.push(EditorAction::ExtrudeAlong {
                            entity,
                            spline,
                            samples: asked.samples,
                            taper: asked.taper,
                        });
                    }
                });
            });
        });

    ui.ctx().data_mut(|data| data.insert_temp(id, asked));
}

/// Which of the two halves is missing, said where the author is already pointing.
fn why_not(faces: usize, curve: bool) -> &'static str {
    match (faces > 0, curve) {
        (false, true) => "Switch to Face mode and paint the faces to run down the curve.",
        (true, false) => "Pick the entity whose Spline is the path.",
        _ => "Paint faces on this block, then pick the curve to run them down.",
    }
}

/// The entities carrying a `Spline`, this block aside.
fn pick_curve(
    ui: &mut egui::Ui,
    chosen: &mut Option<Entity>,
    entities: &[EntityDisplayInfo],
    entity: Entity,
) {
    let label = chosen
        .and_then(|spline| entities.iter().find(|info| info.entity == spline))
        .and_then(display_name_for)
        .unwrap_or_else(|| "(none)".to_owned());
    egui::ComboBox::from_id_salt(("extrude_curve", entity))
        .selected_text(label)
        .show_ui(ui, |ui| {
            for info in entities.iter().filter(|info| {
                info.entity != entity && info.components.iter().any(|c| c.short_name == "Spline")
            }) {
                let picked = *chosen == Some(info.entity);
                let name = display_name_for(info).unwrap_or_else(|| "(unnamed)".to_owned());
                if ui.selectable_label(picked, name).clicked() {
                    *chosen = Some(info.entity);
                }
            }
        });
}
