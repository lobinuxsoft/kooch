//! Input Map panel — where bindings are configured.
//!
//! Shows the map and what the **running host** says each action is worth;
//! it never resolves the map itself. Evaluating for display while the
//! host evaluates for play would put one value in two code paths — the
//! shape behind all five prefab bugs in #611. Hence an empty live column
//! when nothing is playing.
//!
//! Rebinding reads the editor's own input backend (#711,
//! `bootstrap.rs`), registered after the egui layer so a key typed into a
//! focused field stops there.

use kooch_input::actions::{
    Action, ActionMap, Binding, BothHeld, Composite, ControlPath, ControlType, DeviceClass,
    PartName, Processor, Role, VectorMode,
};

use crate::icons;
use crate::widgets::SelectableRow;

/// What the panel needs to draw one frame.
pub(crate) struct InputMapView<'a> {
    /// The map being edited, if one is open.
    pub map: Option<&'a ActionMap>,
    /// Per-action live values, in the map's order, as reported by the
    /// host. Empty when nothing is playing.
    pub live: &'a [LiveAction],
    /// The binding waiting for a key, if a rebind is in progress.
    pub awaiting: Option<BindingAddress>,
    /// Whether the open map diverges from its file.
    pub dirty: bool,
    /// What the properties pane at the bottom is editing.
    pub selected: Option<Selection>,
    /// Whether the open document is a single `.inputaction`.
    ///
    /// It is held as a map of one, so everything below draws the same
    /// either way — what changes is that the map-level controls (add an
    /// action, delete one, the priority) describe something the file does
    /// not have, and offering them would let you save a `.inputaction`
    /// with two actions in it.
    pub single_action: bool,
}

/// What the host says an action is worth right now.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LiveAction {
    pub value: glam::Vec3,
    pub pressed: bool,
}

/// What the properties pane is editing. Unity splits this into four
/// views; one enum covers the same ground in a single column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Selection {
    Action(usize),
    Binding(BindingAddress),
}

/// Which binding a click was on. Positional rather than an id: a binding
/// has no identity of its own, and the edit is applied to the same list
/// the click came from, in the same frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct BindingAddress {
    pub action: usize,
    pub binding: usize,
}

/// Whose processor list an edit is aimed at.
///
/// Both lists exist and run at different moments: a binding's shape the
/// **device** (a stick's deadzone), an action's shape the **meaning**,
/// once, on whichever binding won.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ProcessorTarget {
    Binding(BindingAddress),
    Action(usize),
}

/// What the user did, for the caller to apply. Intent rather than
/// mutation: the map may live behind an asset handle, an undo stack or a
/// socket, and a panel that writes through all three knows all three.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum InputMapAction {
    /// Write the map back to its file.
    Save,
    /// Select an action or a binding — what the properties pane edits.
    Select(Selection),
    /// Rename an action — the name is what gameplay resolves, so this
    /// is the most consequential edit here.
    RenameAction {
        action: usize,
        name: String,
    },
    /// Change what an action produces.
    SetControlType {
        action: usize,
        control_type: ControlType,
    },
    /// Start listening for an input to put on this binding.
    BeginRebind(BindingAddress),
    /// Stop listening without changing anything.
    CancelRebind,
    /// Replace what this binding reads.
    Rebind {
        at: BindingAddress,
        path: ControlPath,
    },
    RemoveBinding(BindingAddress),
    AddBinding {
        action: usize,
    },
    /// Add a composite and one unbound part per name it declares.
    AddComposite {
        action: usize,
        composite: Composite,
    },
    /// Change a composite's parameters — its mode, or which side wins.
    SetComposite {
        at: BindingAddress,
        composite: Composite,
    },
    AddProcessor {
        to: ProcessorTarget,
        processor: Processor,
    },
    /// Replace one in place — how its parameters are edited.
    SetProcessor {
        to: ProcessorTarget,
        index: usize,
        processor: Processor,
    },
    RemoveProcessor {
        to: ProcessorTarget,
        index: usize,
    },
    /// Move one up (`-1`) or down (`+1`).
    ///
    /// Order is meaning, not presentation: a deadzone after a scale cuts
    /// a different amount than one before it.
    MoveProcessor {
        to: ProcessorTarget,
        index: usize,
        delta: i32,
    },
    AddAction,
    RemoveAction {
        action: usize,
    },
}

/// Draws the panel. Returns what the user asked for.
pub(crate) fn draw_input_map_content(
    ui: &mut egui::Ui,
    view: InputMapView<'_>,
) -> Vec<InputMapAction> {
    let mut actions = Vec::new();

    let Some(map) = view.map else {
        ui.weak("No input map open.");
        ui.label("Create one in the asset browser: New → Input Map.");
        return actions;
    };

    // Wrapped so `Save` stays reachable in a narrow tab: edits live in
    // memory until it is pressed.
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("{} {}", icons::GAME_CONTROLLER, map.name));
        if !view.single_action {
            ui.weak(format!("priority {}", map.priority));
            if ui.button(format!("{} Action", icons::PLUS)).clicked() {
                actions.push(InputMapAction::AddAction);
            }
        }
        // Same contract a prefab has; the marker makes it visible.
        if ui
            .add_enabled(view.dirty, egui::Button::new("Save"))
            .on_hover_text("Write these bindings back to the file")
            .clicked()
        {
            actions.push(InputMapAction::Save);
        }
        if view.dirty {
            ui.weak("• unsaved");
        }
    });
    if view.live.is_empty() {
        ui.weak("Values appear while the game is playing.");
    }
    ui.separator();

    // Unity's third column. A column rather than a strip along the
    // bottom: the strip's fixed height came off the tree whether or not
    // anything was selected, and the tree is the part that grows.
    //
    // Before the central panel, because egui allocates edges first —
    // reversed, the side panel gets only what the tree declined.
    let (default_width, width_range) = properties_column(ui.available_width());
    // `Panel::right` rather than `SidePanel`: egui 0.35 folded the four
    // side/top/bottom builders into one `Panel`, so the old name does not
    // resolve and `default_width`/`width_range` are now size-agnostic.
    egui::Panel::right("input_map_properties")
        .resizable(true)
        .default_size(default_width)
        .size_range(width_range)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .id_salt("input_map_properties")
                .show(ui, |ui| {
                    draw_properties(ui, map, &view, &mut actions);
                });
        });

    egui::CentralPanel::default().show(ui, |ui| {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .id_salt("input_map_actions")
            .show(ui, |ui| {
                for (index, action) in map.actions.iter().enumerate() {
                    draw_action(ui, index, action, &view, &mut actions);
                }
            });
    });

    actions
}

/// Starting width of the properties column and the range a drag may take
/// it to, for a tab of `tab_width`.
///
/// Fractions, never pixels: a 190px floor on a 260px tab left the tree
/// 70px and pushed the properties off the right edge. A fixed minimum
/// does not stop the window shrinking under it, it just stops fitting.
/// The ceiling is half the tab, so the properties never outgrow the tree.
fn properties_column(tab_width: f32) -> (f32, std::ops::RangeInclusive<f32>) {
    (tab_width * 0.34, (tab_width * 0.25)..=(tab_width * 0.5))
}

/// A labelled control: side by side when there is room, stacked when
/// there is not. `add` receives the width to take.
///
/// Squeezing a field into what is left of a narrow line is what clipped
/// `Binding — left` to `g — left`. The threshold comes off the font, so
/// it follows editor zoom.
fn labeled_control<R>(
    ui: &mut egui::Ui,
    label: &str,
    add: impl FnOnce(&mut egui::Ui, f32) -> R,
) -> R {
    let side_by_side = ui.text_style_height(&egui::TextStyle::Body) * 12.0;
    if ui.available_width() >= side_by_side {
        ui.horizontal(|ui| {
            ui.label(label);
            let room = (ui.available_width() - ui.spacing().item_spacing.x).max(1.0);
            add(ui, room)
        })
        .inner
    } else {
        ui.label(label);
        let room = ui.available_width().max(1.0);
        add(ui, room)
    }
}

/// Edits whatever is selected. Empty when nothing is.
fn draw_properties(
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
                    // A head carries processors like any other binding —
                    // `read_action` applies them to the composite's
                    // assembled value, which is the only place a stick
                    // deadzone belongs: on the vector, not on each axis.
                    // Filtered by what the composite produces rather than
                    // by the action's type, since that is what arrives.
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
///
/// Order is meaning: a deadzone before a scale cuts the raw value, after
/// it cuts the scaled one. So they are a list you can reorder, not a set.
fn draw_processors(
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
fn draw_processor_parameters(ui: &mut egui::Ui, processor: Processor) -> Option<Processor> {
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
fn drag(
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
fn draw_composite_parameters(
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

fn vector_mode_label(mode: VectorMode) -> &'static str {
    match mode {
        VectorMode::DigitalNormalized => "digital normalized",
        VectorMode::Digital => "digital",
        VectorMode::Analog => "analog",
    }
}

fn vector_mode_hint(mode: VectorMode) -> &'static str {
    match mode {
        VectorMode::DigitalNormalized => "Buttons, capped at length 1. What WASD needs",
        VectorMode::Digital => "Buttons, uncapped — a diagonal is longer",
        VectorMode::Analog => "Sticks, passed through as pushed",
    }
}

fn both_held_label(both: BothHeld) -> &'static str {
    match both {
        BothHeld::Neither => "cancel",
        BothHeld::Positive => "positive",
        BothHeld::Negative => "negative",
    }
}

/// Picks the control a binding reads.
///
/// Two dropdowns rather than Unity's control-path tree: ours is a closed
/// enum, so the device narrows the list and `ALL` makes it exhaustive by
/// construction. Theirs has to parse `<Gamepad>/buttonSouth` out of a
/// string and offer wildcards on top.
fn draw_control_picker(
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
fn controls_of(device: DeviceClass) -> Vec<ControlPath> {
    match device {
        DeviceClass::Keyboard => kooch_input::ids::KeyCode::ALL
            .iter()
            .map(|k| ControlPath::Key(*k))
            .collect(),
        DeviceClass::Mouse => kooch_input::ids::MouseButton::ALL
            .iter()
            .map(|b| ControlPath::Mouse(*b))
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

fn first_control(device: DeviceClass) -> Option<ControlPath> {
    controls_of(device).into_iter().next()
}

/// A control without its device prefix, for a list already grouped by one.
fn control_label(path: ControlPath) -> String {
    match path {
        ControlPath::Key(key) => format!("{key:?}"),
        ControlPath::Mouse(button) => format!("{button:?}"),
        ControlPath::Button(button) => format!("{button:?}"),
        ControlPath::Axis(axis) => format!("{axis:?} (axis)"),
    }
}

fn draw_action(
    ui: &mut egui::Ui,
    index: usize,
    action: &Action,
    view: &InputMapView<'_>,
    out: &mut Vec<InputMapAction>,
) {
    let live = view.live.get(index).copied();
    let id = ui.make_persistent_id(("input_action", index));
    let state =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, index == 0);

    state
        .show_header(ui, |ui| {
            ui.horizontal(|ui| {
                let header = SelectableRow::new(format!(
                    "{} {}",
                    control_type_icon(action.control_type),
                    action.name
                ))
                .selected(view.selected == Some(Selection::Action(index)))
                .show(ui);
                if header.clicked() {
                    out.push(InputMapAction::Select(Selection::Action(index)));
                }
                ui.weak(control_type_label(action.control_type));

                // The live half. Only meaningful while something plays,
                // and deliberately absent rather than zeroed otherwise —
                // a zero would read as "not firing", which is a different
                // statement from "nobody is asking".
                if let Some(live) = live {
                    ui.separator();
                    if live.pressed {
                        ui.colored_label(egui::Color32::from_rgb(120, 220, 120), "▶");
                    } else {
                        ui.weak("·");
                    }
                    ui.weak(format_value(action.control_type, live.value));
                }
            });
        })
        .body(|ui| {
            for (binding_index, binding) in action.bindings.iter().enumerate() {
                let at = BindingAddress {
                    action: index,
                    binding: binding_index,
                };
                draw_binding(
                    ui,
                    at,
                    binding,
                    view.awaiting == Some(at),
                    view.selected == Some(Selection::Binding(at)),
                    out,
                );
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .small_button(format!("{} Binding", icons::PLUS))
                    .clicked()
                {
                    out.push(InputMapAction::AddBinding { action: index });
                }
                draw_add_composite_menu(ui, index, action.control_type, out);
                // A single-action file has nothing left once its action
                // is gone, so removing it is not offered.
                if !view.single_action
                    && ui
                        .small_button(format!("{} Action", icons::TRASH))
                        .clicked()
                {
                    out.push(InputMapAction::RemoveAction { action: index });
                }
            });
        });
}

/// The "+ Composite" menu, listing what fits `control_type`.
///
/// Filtered the way Unity filters its own, and for the same reason: a 2D
/// composite under a Button action is a binding that reads as nothing,
/// with no error to say why. Offering it is offering a trap.
///
/// Everything is offered when nothing fits, rather than an empty menu —
/// a menu that opens onto nothing reads as broken, and the type is one
/// click away in the properties pane.
fn draw_add_composite_menu(
    ui: &mut egui::Ui,
    action: usize,
    control_type: ControlType,
    out: &mut Vec<InputMapAction>,
) {
    ui.menu_button(format!("{} Composite", icons::PLUS), |ui| {
        let fits: Vec<Composite> = Composite::ALL
            .iter()
            .copied()
            .filter(|c| c.control_type() == control_type)
            .collect();
        let offered = if fits.is_empty() {
            Composite::ALL.to_vec()
        } else {
            fits
        };
        for composite in offered {
            if ui.button(composite.label()).clicked() {
                out.push(InputMapAction::AddComposite { action, composite });
                ui.close();
            }
        }
    })
    .response
    .on_hover_text("Several controls read as one value — WASD, or Ctrl+S");
}

fn draw_binding(
    ui: &mut egui::Ui,
    at: BindingAddress,
    binding: &Binding,
    awaiting: bool,
    selected: bool,
    out: &mut Vec<InputMapAction>,
) {
    let label = match (&binding.role, awaiting) {
        (_, true) => "  press any key…  (Esc to cancel)".to_owned(),
        (Role::Whole(path), _) => format!("  {}", describe(*path)),
        (Role::CompositeHead(composite), _) => {
            format!("  {} {composite:?}", icons::TREE_STRUCTURE)
        }
        (Role::Part { name, path }, _) => {
            format!("      {} — {}", part_label(*name), describe(*path))
        }
    };

    ui.horizontal(|ui| {
        let response = SelectableRow::new(label)
            .selected(selected || awaiting)
            .show(ui);
        // A click selects; the pane edits. Listening immediately would
        // make it impossible to look at a binding without arming it.
        if response.clicked() {
            out.push(InputMapAction::Select(Selection::Binding(at)));
        }
        response.context_menu(|ui| {
            if ui.button(format!("{} Remove", icons::TRASH)).clicked() {
                out.push(InputMapAction::RemoveBinding(at));
                ui.close();
            }
        });
    });

    if !binding.processors.is_empty() {
        // The one row drawn without `SelectableRow`, so the one that
        // did not inherit its truncation.
        ui.horizontal_wrapped(|ui| {
            ui.add_space(24.0);
            for processor in &binding.processors {
                ui.add(
                    egui::Label::new(egui::RichText::new(format!("{processor:?}")).weak())
                        .truncate(),
                );
            }
        });
    }
}

/// A control, as an author would name it.
fn describe(path: ControlPath) -> String {
    let device = match path.device() {
        DeviceClass::Keyboard => "Key",
        DeviceClass::Mouse => "Mouse",
        DeviceClass::Gamepad => "Pad",
    };
    let control = match path {
        ControlPath::Key(key) => format!("{key:?}"),
        ControlPath::Mouse(button) => format!("{button:?}"),
        ControlPath::Button(button) => format!("{button:?}"),
        ControlPath::Axis(axis) => format!("{axis:?}"),
    };
    format!("{device} / {control}")
}

fn part_label(name: PartName) -> &'static str {
    match name {
        PartName::Positive => "positive",
        PartName::Negative => "negative",
        PartName::Up => "up",
        PartName::Down => "down",
        PartName::Left => "left",
        PartName::Right => "right",
        PartName::Forward => "forward",
        PartName::Backward => "backward",
        PartName::Modifier => "modifier",
        PartName::Modifier2 => "modifier 2",
        PartName::Value => "value",
    }
}

fn control_type_label(control_type: ControlType) -> &'static str {
    match control_type {
        ControlType::Button => "button",
        ControlType::Axis => "axis",
        ControlType::Vector2 => "vector2",
        ControlType::Vector3 => "vector3",
    }
}

fn control_type_icon(control_type: ControlType) -> &'static str {
    match control_type {
        ControlType::Button => icons::CUBE,
        ControlType::Axis => icons::SLIDERS,
        ControlType::Vector2 | ControlType::Vector3 => icons::ARROWS_CLOCKWISE,
    }
}

/// Formats a live value the way the action's type reads it.
fn format_value(control_type: ControlType, value: glam::Vec3) -> String {
    match control_type {
        ControlType::Button => format!("{:.0}", value.x),
        ControlType::Axis => format!("{:+.2}", value.x),
        ControlType::Vector2 => format!("{:+.2}, {:+.2}", value.x, value.y),
        ControlType::Vector3 => {
            format!("{:+.2}, {:+.2}, {:+.2}", value.x, value.y, value.z)
        }
    }
}

#[cfg(test)]
mod tests;
