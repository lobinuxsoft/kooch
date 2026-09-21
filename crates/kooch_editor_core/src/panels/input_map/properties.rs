//! The properties column: what the selected action, binding or composite is, and its processors.

use super::*;

/// Edits whatever is selected. Empty when nothing is.
pub(super) fn draw_properties(
    ui: &mut egui::Ui,
    map: &ActionMap,
    view: &InputMapView<'_>,
    out: &mut Vec<InputMapAction>,
) {
    let Some(selected) = view.selected else {
        ui.weak("Select an action or a binding to edit it.");
        return;
    };

    match selected {
        Selection::Action(index) => {
            let Some(action) = map.actions.get(index) else {
                return;
            };
            ui.label("Action");
            labeled_control(ui, "Name", |ui, room| {
                // Held in egui's memory while typing: an edit per
                // keystroke pushes every spelling through `resolve`.
                let id = ui.make_persistent_id(("input_action_name", index));
                let mut name = ui
                    .data(|d| d.get_temp::<String>(id))
                    .unwrap_or_else(|| action.name.clone());
                let response = ui.add(egui::TextEdit::singleline(&mut name).desired_width(room));
                if response.changed() {
                    ui.data_mut(|d| d.insert_temp(id, name.clone()));
                }
                if response.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let trimmed = name.trim();
                    // Empty resolves to nothing, a duplicate makes
                    // `resolve` a coin toss.
                    let taken = map
                        .actions
                        .iter()
                        .enumerate()
                        .any(|(other, a)| other != index && a.name == trimmed);
                    if !trimmed.is_empty() && !taken && trimmed != action.name {
                        out.push(InputMapAction::RenameAction {
                            action: index,
                            name: trimmed.to_owned(),
                        });
                    }
                    ui.data_mut(|d| d.remove::<String>(id));
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Type");
                for control_type in [
                    ControlType::Button,
                    ControlType::Axis,
                    ControlType::Vector2,
                    ControlType::Vector3,
                ] {
                    if ui
                        .selectable_label(
                            action.control_type == control_type,
                            control_type_label(control_type),
                        )
                        .clicked()
                        && action.control_type != control_type
                    {
                        out.push(InputMapAction::SetControlType {
                            action: index,
                            control_type,
                        });
                    }
                }
            });
            ui.separator();
            // Run once on whichever binding won, so a sensitivity or a
            // normalize is written here instead of on each binding.
            draw_processors(
                ui,
                ProcessorTarget::Action(index),
                &action.processors,
                action.control_type,
                out,
            );
        }
        Selection::Binding(at) => {
            let Some(binding) = map
                .actions
                .get(at.action)
                .and_then(|a| a.bindings.get(at.binding))
            else {
                return;
            };
            match &binding.role {
                Role::CompositeHead(composite) => {
                    ui.label(format!("{} Composite", composite.label()));
                    draw_composite_parameters(ui, at, *composite, out);
                    ui.weak("Its parts are the rows underneath.");
                    ui.separator();
                    // A head carries processors like any other binding — `read_action` applies them
                    // to the composite's assembled value, which is the only place a stick deadzone
                    // belongs: on the vector, not on each axis.
                    draw_processors(
                        ui,
                        ProcessorTarget::Binding(at),
                        &binding.processors,
                        composite.control_type(),
                        out,
                    );
                }
                Role::Whole(path) | Role::Part { path, .. } => {
                    ui.label(match binding.role {
                        Role::Part { name, .. } => format!("Binding — {}", part_label(name)),
                        _ => "Binding".to_owned(),
                    });
                    draw_control_picker(ui, at, *path, out);
                    ui.separator();
                    draw_processors(
                        ui,
                        ProcessorTarget::Binding(at),
                        &binding.processors,
                        map.actions[at.action].control_type,
                        out,
                    );
                }
            }
        }
    }
}

/// The binding's processors, in the order they run.
pub(super) fn draw_processors(
    ui: &mut egui::Ui,
    to: ProcessorTarget,
    processors: &[Processor],
    control_type: ControlType,
    out: &mut Vec<InputMapAction>,
) {
    ui.horizontal_wrapped(|ui| {
        ui.label("Processors");
        ui.menu_button(icons::PLUS, |ui| {
            // Filtered like the composite menu: a 2D processor on a
            // button is skipped by `apply`, so offering one is offering
            // a row that shapes nothing.
            for processor in Processor::ALL.iter().copied() {
                if !processor.applies_to(control_type) {
                    continue;
                }
                if ui.button(processor.label()).clicked() {
                    out.push(InputMapAction::AddProcessor { to, processor });
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text("Shape the value between the control and the action");
    });

    if processors.is_empty() {
        ui.weak("None — the control's value passes through.");
        return;
    }

    let last = processors.len() - 1;
    for (index, processor) in processors.iter().enumerate() {
        ui.push_id(("processor", to, index), |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(processor.label());
                // Disabled at the ends rather than hidden, so the row
                // does not change width as it moves.
                if ui
                    .add_enabled(index > 0, egui::Button::new("↑").small())
                    .clicked()
                {
                    out.push(InputMapAction::MoveProcessor {
                        to,
                        index,
                        delta: -1,
                    });
                }
                if ui
                    .add_enabled(index < last, egui::Button::new("↓").small())
                    .clicked()
                {
                    out.push(InputMapAction::MoveProcessor {
                        to,
                        index,
                        delta: 1,
                    });
                }
                if ui.small_button(icons::TRASH).clicked() {
                    out.push(InputMapAction::RemoveProcessor { to, index });
                }
            });
            if let Some(edited) = draw_processor_parameters(ui, *processor) {
                out.push(InputMapAction::SetProcessor {
                    to,
                    index,
                    processor: edited,
                });
            }
        });
    }
}

/// The knobs one processor has. `None` when nothing was changed.
pub(super) fn draw_processor_parameters(
    ui: &mut egui::Ui,
    processor: Processor,
) -> Option<Processor> {
    let mut edited = processor;
    let changed = match &mut edited {
        Processor::AxisDeadzone { min, max } | Processor::StickDeadzone { min, max } => {
            drag(ui, "min", min, 0.0..=1.0) | drag(ui, "max", max, 0.0..=1.0)
        }
        Processor::Clamp { min, max } => {
            drag(ui, "min", min, -10.0..=10.0) | drag(ui, "max", max, -10.0..=10.0)
        }
        Processor::Normalize { min, max, zero } => {
            drag(ui, "min", min, -10.0..=10.0)
                | drag(ui, "max", max, -10.0..=10.0)
                | drag(ui, "zero", zero, -10.0..=10.0)
        }
        Processor::Scale { factor } => drag(ui, "factor", factor, -10.0..=10.0),
        Processor::ScaleVector2 { x, y } => {
            drag(ui, "x", x, -10.0..=10.0) | drag(ui, "y", y, -10.0..=10.0)
        }
        Processor::InvertVector2 { x, y } => {
            let mut changed = false;
            ui.horizontal_wrapped(|ui| {
                changed |= ui.checkbox(x, "x").changed();
                changed |= ui.checkbox(y, "y").changed();
            });
            changed
        }
        Processor::Invert | Processor::NormalizeVector2 => false,
    };
    changed.then_some(edited)
}

/// A labelled number. Dragged rather than typed, since every one of these
/// is a feel setting found by moving it and watching.
pub(super) fn drag(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
) -> bool {
    labeled_control(ui, label, |ui, room| {
        ui.add_sized(
            [room, ui.spacing().interact_size.y],
            egui::DragValue::new(value).speed(0.01).range(range),
        )
        .changed()
    })
}

/// Edits whatever knobs a composite has. Modifier composites have none.
pub(super) fn draw_composite_parameters(
    ui: &mut egui::Ui,
    at: BindingAddress,
    composite: Composite,
    out: &mut Vec<InputMapAction>,
) {
    match composite {
        Composite::Vector2 { mode } | Composite::Vector3 { mode } => {
            // The one setting people get wrong: buttons need capping or a
            // diagonal outruns a straight line, and a stick must not be
            // capped or it loses how far it is pushed.
            ui.horizontal_wrapped(|ui| {
                ui.label("Mode");
                for option in [
                    VectorMode::DigitalNormalized,
                    VectorMode::Digital,
                    VectorMode::Analog,
                ] {
                    if ui
                        .selectable_label(mode == option, vector_mode_label(option))
                        .on_hover_text(vector_mode_hint(option))
                        .clicked()
                        && mode != option
                    {
                        out.push(InputMapAction::SetComposite {
                            at,
                            composite: match composite {
                                Composite::Vector3 { .. } => Composite::Vector3 { mode: option },
                                _ => Composite::Vector2 { mode: option },
                            },
                        });
                    }
                }
            });
        }
        Composite::Axis1D { both_held } => {
            ui.horizontal_wrapped(|ui| {
                ui.label("Both held");
                for option in [BothHeld::Neither, BothHeld::Positive, BothHeld::Negative] {
                    if ui
                        .selectable_label(both_held == option, both_held_label(option))
                        .clicked()
                        && both_held != option
                    {
                        out.push(InputMapAction::SetComposite {
                            at,
                            composite: Composite::Axis1D { both_held: option },
                        });
                    }
                }
            });
        }
        Composite::OneModifier | Composite::TwoModifiers => {
            ui.weak("Fires only while its modifiers are held.");
        }
    }
}

pub(super) fn vector_mode_label(mode: VectorMode) -> &'static str {
    match mode {
        VectorMode::DigitalNormalized => "digital normalized",
        VectorMode::Digital => "digital",
        VectorMode::Analog => "analog",
    }
}

pub(super) fn vector_mode_hint(mode: VectorMode) -> &'static str {
    match mode {
        VectorMode::DigitalNormalized => "Buttons, capped at length 1. What WASD needs",
        VectorMode::Digital => "Buttons, uncapped — a diagonal is longer",
        VectorMode::Analog => "Sticks, passed through as pushed",
    }
}

pub(super) fn both_held_label(both: BothHeld) -> &'static str {
    match both {
        BothHeld::Neither => "cancel",
        BothHeld::Positive => "positive",
        BothHeld::Negative => "negative",
    }
}

/// Picks the control a binding reads.
pub(super) fn draw_control_picker(
    ui: &mut egui::Ui,
    at: BindingAddress,
    current: ControlPath,
    out: &mut Vec<InputMapAction>,
) {
    // Wrapped: three device names plus a label do not fit a narrow
    // column, and `horizontal` runs them off the edge.
    ui.horizontal_wrapped(|ui| {
        ui.label("Device");
        for device in [
            DeviceClass::Keyboard,
            DeviceClass::Mouse,
            DeviceClass::Gamepad,
        ] {
            if ui
                .selectable_label(current.device() == device, format!("{device:?}"))
                .clicked()
                && current.device() != device
            {
                // First control of that device: unbound reads nothing
                // and looks broken.
                if let Some(path) = first_control(device) {
                    out.push(InputMapAction::Rebind { at, path });
                }
            }
        }
    });

    labeled_control(ui, "Control", |ui, room| {
        egui::ComboBox::from_id_salt(("control_picker", at.action, at.binding))
            .width(room)
            .selected_text(control_label(current))
            .show_ui(ui, |ui| {
                for path in controls_of(current.device()) {
                    if ui
                        .selectable_label(path == current, control_label(path))
                        .clicked()
                        && path != current
                    {
                        out.push(InputMapAction::Rebind { at, path });
                    }
                }
            });
    });
}

/// Every control a device class offers, for the picker.
pub(super) fn controls_of(device: DeviceClass) -> Vec<ControlPath> {
    match device {
        DeviceClass::Keyboard => kooch_input::ids::KeyCode::ALL
            .iter()
            .map(|k| ControlPath::Key(*k))
            .collect(),
        // Motion after the buttons, both in one list: a look binds motion, a fire binds a button,
        // and the author picks from what the mouse has rather than from a kind of control (#1266).
        DeviceClass::Mouse => kooch_input::ids::MouseButton::ALL
            .iter()
            .map(|b| ControlPath::Mouse(*b))
            .chain(
                kooch_input::ids::MouseAxis::ALL
                    .iter()
                    .map(|a| ControlPath::MouseMotion(*a)),
            )
            .collect(),
        // Buttons and axes both, since a binding on a pad can be either
        // and forcing that choice into a third dropdown would be a
        // distinction the author does not think in.
        DeviceClass::Gamepad => kooch_input::ids::GamepadButton::ALL
            .iter()
            .map(|b| ControlPath::Button(*b))
            .chain(
                kooch_input::ids::GamepadAxis::ALL
                    .iter()
                    .map(|a| ControlPath::Axis(*a)),
            )
            .collect(),
    }
}

pub(super) fn first_control(device: DeviceClass) -> Option<ControlPath> {
    controls_of(device).into_iter().next()
}

/// A control without its device prefix, for a list already grouped by one.
pub(super) fn control_label(path: ControlPath) -> String {
    match path {
        ControlPath::Key(key) => format!("{key:?}"),
        ControlPath::Mouse(button) => format!("{button:?}"),
        ControlPath::Button(button) => format!("{button:?}"),
        ControlPath::Axis(axis) => format!("{axis:?} (axis)"),
        ControlPath::MouseMotion(axis) => format!("Motion {axis:?} (axis)"),
    }
}
