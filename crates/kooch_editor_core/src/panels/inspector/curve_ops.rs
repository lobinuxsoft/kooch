//! The author's way into [`spline::ops`](kooch_ecs::spline::ops) (#1447).
//!
//! 🔴 Every button emits the **whole knot list** as one `SetField` on `points` — the same edit the
//! Inspector's list widget and the viewport grips already make. One way a curve gets written means
//! one Ctrl+Z puts it back, and no second source of truth to disagree with the first.

use kooch_ecs::component::ComponentId;
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::{ReflectValue, list_from, list_value};
use kooch_ecs::spline::{self, Knot, arc, make, ops};

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
    /// How thick the curve is at a knot, for the shapes and for `Set radius` (#1542).
    knot: f32,
    made: f32,
    arc: f32,
    sweep: f32,
    foot: f32,
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
            knot: spline::DEFAULT_RADIUS,
            made: 1.0,
            arc: 1.0,
            sweep: 90.0,
            foot: 1.0,
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

            boxed(ui, |ui| {
                ui.label("knots");
                ui.add(crate::numeric::drag(&mut asked.count).range(2..=512));
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

            boxed(ui, |ui| {
                ui.label("cuts");
                ui.add(crate::numeric::drag(&mut asked.cuts).range(1..=16));
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

            boxed(ui, |ui| {
                ui.label("corner");
                ui.add(
                    crate::numeric::drag(&mut asked.radius)
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

            boxed(ui, |ui| {
                ui.label("from");
                ui.add(
                    crate::numeric::drag(&mut asked.from)
                        .speed(0.1)
                        .range(0.0..=length)
                        .suffix(" m"),
                );
                ui.label("to");
                ui.add(
                    crate::numeric::drag(&mut asked.to)
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

            // The one knot field the Inspector cannot reach in bulk: the list edits it one knot at
            // a time, and a swept profile wants the same width along the whole curve.
            let any = !points.is_empty();
            boxed(ui, |ui| {
                ui.label("knot radius");
                ui.add(knot_radius(&mut asked.knot));
                if ui
                    .add_enabled(any, egui::Button::new("Set radius"))
                    .on_hover_text(
                        "Give every knot this radius. It is what a Spline Sweep is thick, and \
                         what the viewport grips scale.",
                    )
                    .on_disabled_hover_text("This curve has no knots yet.")
                    .clicked()
                {
                    let mut knots = points.clone();
                    for knot in &mut knots {
                        knot.radius = asked.knot;
                    }
                    edited(knots, actions);
                }
            });

            boxed(ui, |ui| {
                if ui
                    .add_enabled(usable, egui::Button::new("Reverse"))
                    .on_hover_text("Walk the curve the other way. The roll flips with it.")
                    .on_disabled_hover_text(NEEDS_TWO)
                    .clicked()
                {
                    edited(ops::reverse(&points), actions);
                }
            });

            // 🔴 These REPLACE the curve rather than operating on it: one press throws away
            // whatever was there, and one Ctrl+Z is the only way back. Kept behind a named
            // sub-panel, shut until asked for, so nobody lands on one reaching for Reverse.
            egui::CollapsingHeader::new("Replace with a shape")
                .id_salt(("curve_make", entity))
                .show(ui, |ui| {
                    ui.group(|ui| {
                        ui.set_min_width(ui.available_width());
                        ui.label("Shared by the three");
                        ui.horizontal(|ui| {
                            ui.label("segments");
                            ui.add(crate::numeric::drag(&mut asked.segments).range(3..=256))
                                .on_hover_text("How many knots each shape is built from.");
                        });
                        ui.horizontal(|ui| {
                            ui.label("knot radius");
                            ui.add(knot_radius(&mut asked.knot))
                                .on_hover_text("Every knot the shape comes out with.");
                        });
                    });

                    boxed(ui, |ui| {
                        ui.label("radius");
                        ui.add(shape_radius(&mut asked.made));
                        if ui
                            .button("Circle")
                            .on_hover_text(
                                "A closed ring of that radius on the XZ plane, around the \
                                 entity's origin. Aim it with the transform.",
                            )
                            .clicked()
                        {
                            let knots = make::circle(asked.made, asked.segments);
                            made(sized(knots, asked.knot), true, actions);
                        }
                    });

                    boxed(ui, |ui| {
                        ui.label("radius");
                        ui.add(shape_radius(&mut asked.arc));
                        ui.label("angle");
                        ui.add(
                            crate::numeric::drag(&mut asked.sweep)
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
                            let knots = make::arc(asked.arc, sweep, asked.segments);
                            made(sized(knots, asked.knot), false, actions);
                        }
                    });

                    ui.group(|ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label("foot");
                            ui.add(shape_radius(&mut asked.foot))
                                .on_hover_text("Radius at the bottom, where it starts.");
                            ui.label("top");
                            ui.add(shape_radius(&mut asked.tip))
                                .on_hover_text("Radius at the top. Below the foot it winds in.");
                        });
                        ui.horizontal(|ui| {
                            ui.label("height");
                            ui.add(
                                crate::numeric::drag(&mut asked.height)
                                    .speed(0.1)
                                    .range(-1000.0..=1000.0)
                                    .suffix(" m"),
                            )
                            .on_hover_text("How far it climbs.");
                            ui.label("turns");
                            ui.add(
                                crate::numeric::drag(&mut asked.turns)
                                    .speed(0.1)
                                    .range(-64.0..=64.0),
                            )
                            .on_hover_text("Negative winds the other way.");
                            if ui
                                .button("Spiral")
                                .on_hover_text(
                                    "A helix from the foot radius to the top one. Put a Spline \
                                     Sweep with the Steps profile on it and it is a spiral \
                                     staircase.",
                                )
                                .clicked()
                            {
                                let knots = make::spiral(
                                    asked.foot,
                                    asked.tip,
                                    asked.height,
                                    asked.turns,
                                    asked.segments,
                                );
                                made(sized(knots, asked.knot), false, actions);
                            }
                        });
                    });
                });
        });

    ui.ctx().data_mut(|data| data.insert_temp(id, asked));
}

/// One operation per box: its own inputs, its own button, nothing shared with the box below.
fn boxed(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    ui.group(|ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(add);
    });
}

/// How wide the shape comes out, in metres.
fn shape_radius(value: &mut f32) -> egui::DragValue<'_> {
    crate::numeric::drag(value)
        .speed(0.1)
        .range(0.0..=1000.0)
        .suffix(" m")
}

/// How thick the curve is at a knot — what a sweep is built around.
fn knot_radius(value: &mut f32) -> egui::DragValue<'_> {
    crate::numeric::drag(value)
        .speed(0.05)
        .range(spline::RADIUS_MIN as f64..=spline::RADIUS_MAX as f64)
        .suffix(" m")
}

/// The same radius on every knot a generator just made.
fn sized(mut knots: Vec<Knot>, radius: f32) -> Vec<Knot> {
    for knot in &mut knots {
        knot.radius = radius;
    }
    knots
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
