//! The author's way into [`spline::ops`](kooch_ecs::spline::ops) (#1447).
//!
//! 🔴 Every button emits the **whole knot list** as one `SetField` on `points` — the same edit the
//! Inspector's list widget and the viewport grips already make. One way a curve gets written means
//! one Ctrl+Z puts it back, and no second source of truth to disagree with the first.

use kooch_ecs::component::ComponentId;
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::{ReflectValue, list_from, list_value};
use kooch_ecs::spline::{Knot, arc, make, ops};

use crate::actions::EditorAction;
use crate::state::ComponentDisplayInfo;

/// What each operation was last asked for. Kept in egui's own store rather than threaded through
/// the panel: it is a widget's position, not editor state, and it must not reach a saved scene.
#[derive(Clone, Copy)]
struct Asked {
    count: usize,
    cuts: usize,
    radius: f32,
    from: f32,
    to: f32,
    /// What the primitives below are built from (#1528).
    segments: usize,
    made: f32,
    sweep: f32,
    tip: f32,
    height: f32,
    turns: f32,
}

impl Default for Asked {
    fn default() -> Self {
        Self {
            count: 16,
            cuts: 1,
            radius: 0.5,
            from: 0.0,
            to: 0.0,
            segments: make::DEFAULT_SEGMENTS,
            made: 1.0,
            sweep: 90.0,
            tip: 1.0,
            height: 3.0,
            turns: 2.0,
        }
    }
}

/// Draws the operations for a `Spline` component, pushing one edit per click.
pub(super) fn draw(
    ui: &mut egui::Ui,
    entity: Entity,
    component: ComponentId,
    comp: &ComponentDisplayInfo,
    actions: &mut Vec<EditorAction>,
) {
    let Some((points, closed)) = curve(comp) else {
        return;
    };
    let length = arc::length(&points, closed);
    let id = egui::Id::new(("curve_ops", entity));
    let mut asked = ui
        .ctx()
        .data_mut(|data| data.get_temp::<Asked>(id))
        .unwrap_or_default();
    // A range nobody has set yet means the whole curve, so Trim starts as a no-op rather than
    // deleting everything the first time it is pressed.
    if !(asked.to > 0.0) {
        asked.to = length;
    }

    let edited = |knots: Vec<Knot>, actions: &mut Vec<EditorAction>| {
        actions.push(EditorAction::SetField {
            entity,
            component,
            field: "points".to_owned(),
            value: list_value(&knots),
        });
    };

    // 🔴 `closed` travels with the knots, not after them. A ring written onto an open curve draws
    // with a gap and a line written onto a closed one joins its two ends — and the author would be
    // looking at a shape nobody asked for with nothing to say why.
    let made = |knots: Vec<Knot>, ring: bool, actions: &mut Vec<EditorAction>| {
        if ring != closed {
            actions.push(EditorAction::SetField {
                entity,
                component,
                field: "closed".to_owned(),
                value: ReflectValue::Bool(ring),
            });
        }
        actions.push(EditorAction::SetField {
            entity,
            component,
            field: "points".to_owned(),
            value: list_value(&knots),
        });
    };

    const NEEDS_TWO: &str = "A curve needs at least two knots before it can be operated on.";

    egui::CollapsingHeader::new("Curve operations")
        .id_salt(id)
        .show(ui, |ui| {
            // Fewer than two knots is a curve being drawn, not one that can be operated on.
            let usable = points.len() >= 2;
            ui.label(format!("{length:.2} m along {} knots", points.len()));

            ui.horizontal(|ui| {
                ui.label("knots");
                ui.add(egui::DragValue::new(&mut asked.count).range(2..=512));
                if ui
                    .add_enabled(usable, egui::Button::new("Resample"))
                    .on_hover_text(
                        "Replace the knots with evenly spaced ones, measured in metres along \
                         the curve rather than in parameter. Authored tangents are lost.",
                    )
                    .on_disabled_hover_text(NEEDS_TWO)
                    .clicked()
                {
                    edited(ops::resample(&points, closed, asked.count), actions);
                }
            });

            ui.horizontal(|ui| {
                ui.label("cuts");
                ui.add(egui::DragValue::new(&mut asked.cuts).range(1..=16));
                if ui
                    .add_enabled(usable, egui::Button::new("Subdivide"))
                    .on_hover_text(
                        "Add this many knots inside every segment, keeping the spacing the \
                         author chose. Resample is the one that evens it out.",
                    )
                    .on_disabled_hover_text(NEEDS_TWO)
                    .clicked()
                {
                    edited(ops::subdivide(&points, closed, asked.cuts), actions);
                }
            });

            ui.horizontal(|ui| {
                ui.label("radius");
                ui.add(
                    egui::DragValue::new(&mut asked.radius)
                        .speed(0.05)
                        .range(0.0..=100.0)
                        .suffix(" m"),
                );
                if ui
                    .add_enabled(usable, egui::Button::new("Fillet"))
                    .on_hover_text(
                        "Round every corner by pulling it back along both neighbours. Clamped \
                         to half of each segment, so two corners cannot cross.",
                    )
                    .on_disabled_hover_text(NEEDS_TWO)
                    .clicked()
                {
                    edited(ops::fillet(&points, closed, asked.radius), actions);
                }
            });

            ui.horizontal(|ui| {
                ui.label("from");
                ui.add(
                    egui::DragValue::new(&mut asked.from)
                        .speed(0.1)
                        .range(0.0..=length)
                        .suffix(" m"),
                );
                ui.label("to");
                ui.add(
                    egui::DragValue::new(&mut asked.to)
                        .speed(0.1)
                        .range(0.0..=length)
                        .suffix(" m"),
                );
                if ui
                    .add_enabled(usable, egui::Button::new("Trim"))
                    .on_hover_text(
                        "Keep only the curve between these two distances, in metres. The \
                         result is open: a ring cut anywhere is a line.",
                    )
                    .on_disabled_hover_text(NEEDS_TWO)
                    .clicked()
                {
                    let trimmed = ops::trim(&points, closed, asked.from, asked.to);
                    // 🔴 Trimming a ring opens it, and leaving `closed` set would join the two
                    // fresh ends back together — exactly the part just removed.
                    if closed {
                        actions.push(EditorAction::SetField {
                            entity,
                            component,
                            field: "closed".to_owned(),
                            value: ReflectValue::Bool(false),
                        });
                    }
                    edited(trimmed, actions);
                }
            });

            if ui
                .add_enabled(usable, egui::Button::new("Reverse"))
                .on_hover_text("Walk the curve the other way. The roll flips with it.")
                .on_disabled_hover_text(NEEDS_TWO)
                .clicked()
            {
                edited(ops::reverse(&points), actions);
            }

            // 🔴 These REPLACE the curve rather than operating on it: one press throws away
            // whatever was there, and one Ctrl+Z is the only way back. Kept behind a named
            // sub-panel, shut until asked for, so nobody lands on one reaching for Reverse.
            egui::CollapsingHeader::new("Replace with a shape")
                .id_salt(("curve_make", entity))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("segments");
                        ui.add(egui::DragValue::new(&mut asked.segments).range(3..=256))
                            .on_hover_text("How many knots each shape below is built from.");
                    });

                    ui.horizontal(|ui| {
                        ui.label("radius");
                        ui.add(
                            egui::DragValue::new(&mut asked.made)
                                .speed(0.1)
                                .range(0.0..=1000.0)
                                .suffix(" m"),
                        )
                        .on_hover_text("Shared by the three shapes. A spiral starts here.");
                    });

                    if ui
                        .button("Circle")
                        .on_hover_text(
                            "A closed ring of that radius on the XZ plane, around the entity's \
                             origin. Aim it with the transform.",
                        )
                        .clicked()
                    {
                        made(make::circle(asked.made, asked.segments), true, actions);
                    }

                    ui.horizontal(|ui| {
                        ui.label("angle");
                        ui.add(
                            egui::DragValue::new(&mut asked.sweep)
                                .speed(1.0)
                                .range(-720.0..=720.0)
                                .suffix("°"),
                        );
                        if ui
                            .button("Arc")
                            .on_hover_text(
                                "An open arc of that radius, turning by this angle and \
                                 starting along +X.",
                            )
                            .clicked()
                        {
                            let sweep = asked.sweep.to_radians();
                            made(make::arc(asked.made, sweep, asked.segments), false, actions);
                        }
                    });

                    ui.horizontal(|ui| {
                        ui.label("top");
                        ui.add(
                            egui::DragValue::new(&mut asked.tip)
                                .speed(0.1)
                                .range(0.0..=1000.0)
                                .suffix(" m"),
                        )
                        .on_hover_text(
                            "Radius at the top. Below the radius above it winds inwards.",
                        );
                        ui.label("height");
                        ui.add(
                            egui::DragValue::new(&mut asked.height)
                                .speed(0.1)
                                .range(-1000.0..=1000.0)
                                .suffix(" m"),
                        )
                        .on_hover_text("How far it climbs.");
                    });

                    ui.horizontal(|ui| {
                        ui.label("turns");
                        ui.add(
                            egui::DragValue::new(&mut asked.turns)
                                .speed(0.1)
                                .range(-64.0..=64.0),
                        )
                        .on_hover_text("Negative winds the other way.");
                        if ui
                            .button("Spiral")
                            .on_hover_text(
                                "A helix from the radius above to the top one. Put a Spline \
                                 Sweep with the Steps profile on it and it is a spiral staircase.",
                            )
                            .clicked()
                        {
                            made(
                                make::spiral(
                                    asked.made,
                                    asked.tip,
                                    asked.height,
                                    asked.turns,
                                    asked.segments,
                                ),
                                false,
                                actions,
                            );
                        }
                    });
                });
        });

    ui.ctx().data_mut(|data| data.insert_temp(id, asked));
}

/// The knots and the closed flag this component is showing.
fn curve(comp: &ComponentDisplayInfo) -> Option<(Vec<Knot>, bool)> {
    let fields = comp.fields.values()?;
    let points = fields.iter().find_map(|(field, value)| {
        (field == "points").then(|| list_from::<Knot>(value.clone(), None, "points").ok())
    })??;
    let closed = fields
        .iter()
        .any(|(field, value)| field == "closed" && matches!(value, ReflectValue::Bool(true)));
    Some((points, closed))
}
