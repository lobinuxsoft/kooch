use super::*;
use crate::panels::input_map::{BindingAddress, InputMapAction as Edit, Selection};
use crate::state::OpenInputMap;
use kooch_input::actions::{Action, ActionMap, ControlType};

fn resources() -> Resources {
    let mut r = Resources::new();
    r.insert(OpenInputMap {
        path: "unused.inputmap".into(),
        kind: crate::state::OpenInputKind::Map,
        map: ActionMap::new("gameplay")
            .add(Action::new("jump", ControlType::Button))
            .add(Action::new("move", ControlType::Vector2)),
        focus_requested: false,
        selected: None,
        dirty: false,
    });
    r
}

/// The name is what gameplay resolves, so renaming is the edit that
/// matters most — and the one that used to be impossible.
#[test]
fn an_action_can_be_renamed() {
    let mut r = resources();
    edit_input_map(
        &mut r,
        &Edit::RenameAction {
            action: 0,
            name: "leap".into(),
        },
    );
    let open = r.get::<OpenInputMap>().unwrap();
    assert_eq!(open.map.resolve("leap").map(|id| id.index()), Some(0));
    assert!(open.map.resolve("jump").is_none());
    assert!(open.dirty);
}

/// 🔴 Adding a composite creates its parts too.
///
/// A bare head is a composite that reads as nothing with no clue
/// which parts are missing — the state the panel could not even
/// produce before, since there was no way to add one at all.
#[test]
fn a_composite_arrives_with_one_part_per_name() {
    use kooch_input::actions::{Composite, PartName, Role, VectorMode};

    for composite in Composite::ALL.iter().copied() {
        let mut r = resources();
        edit_input_map(
            &mut r,
            &Edit::AddComposite {
                action: 1,
                composite,
            },
        );
        let open = r.get::<OpenInputMap>().unwrap();
        let bindings = &open.map.actions[1].bindings;

        assert!(
            matches!(bindings.first().map(|b| &b.role), Some(Role::CompositeHead(c)) if *c == composite)
        );
        let parts: Vec<PartName> = bindings
            .iter()
            .filter_map(|b| match b.role {
                Role::Part { name, .. } => Some(name),
                _ => None,
            })
            .collect();
        assert_eq!(
            parts,
            PartName::of(composite).to_vec(),
            "{composite:?} did not get the parts it declares"
        );
        assert!(open.dirty);
    }

    // And the head is selected, so its Mode is on screen — the
    // setting that decides whether a diagonal outruns a straight line.
    let mut r = resources();
    edit_input_map(
        &mut r,
        &Edit::AddComposite {
            action: 1,
            composite: Composite::Vector2 {
                mode: VectorMode::DigitalNormalized,
            },
        },
    );
    let open = r.get::<OpenInputMap>().unwrap();
    assert_eq!(
        open.selected,
        Some(Selection::Binding(BindingAddress {
            action: 1,
            binding: 0
        }))
    );
}

/// Processors are added, edited, reordered and removed — the whole
/// loop, since a list you can only add to is not editable.
#[test]
fn a_bindings_processors_can_be_edited() {
    use kooch_input::actions::Processor;

    let mut r = resources();
    edit_input_map(&mut r, &Edit::AddBinding { action: 0 });
    let at = BindingAddress {
        action: 0,
        binding: 0,
    };
    let scale = Processor::Scale { factor: 1.0 };
    let clamp = Processor::Clamp {
        min: -1.0,
        max: 1.0,
    };

    edit_input_map(
        &mut r,
        &Edit::AddProcessor {
            to: b(at),
            processor: scale,
        },
    );
    edit_input_map(
        &mut r,
        &Edit::AddProcessor {
            to: b(at),
            processor: clamp,
        },
    );
    assert_eq!(processors(&r, at), vec![scale, clamp], "added out of order");

    edit_input_map(
        &mut r,
        &Edit::SetProcessor {
            to: b(at),
            index: 0,
            processor: Processor::Scale { factor: 2.5 },
        },
    );
    assert_eq!(processors(&r, at)[0], Processor::Scale { factor: 2.5 });

    edit_input_map(
        &mut r,
        &Edit::MoveProcessor {
            to: b(at),
            index: 0,
            delta: 1,
        },
    );
    assert_eq!(processors(&r, at)[0], clamp, "the move did not reorder");

    edit_input_map(
        &mut r,
        &Edit::RemoveProcessor {
            to: b(at),
            index: 0,
        },
    );
    assert_eq!(processors(&r, at).len(), 1);
}

/// 🔴 A composite head carries processors, and the panel has to
/// offer them.
///
/// `read_action` applies the head's processors to the composite's
/// assembled value — which is where a stick deadzone belongs, on the
/// vector rather than per axis. The shipped starter map already has
/// one there, so it was visible, evaluated, and uneditable.
#[test]
fn a_composite_head_takes_processors_too() {
    use kooch_input::actions::{Composite, Processor, Role, VectorMode};

    let mut r = resources();
    edit_input_map(
        &mut r,
        &Edit::AddComposite {
            action: 1,
            composite: Composite::Vector2 {
                mode: VectorMode::Analog,
            },
        },
    );
    let head = BindingAddress {
        action: 1,
        binding: 0,
    };
    let deadzone = Processor::StickDeadzone { min: 0.2, max: 0.9 };
    edit_input_map(
        &mut r,
        &Edit::AddProcessor {
            to: b(head),
            processor: deadzone,
        },
    );

    let bindings = &r.get::<OpenInputMap>().unwrap().map.actions[1].bindings;
    assert!(matches!(bindings[0].role, Role::CompositeHead(_)));
    assert_eq!(bindings[0].processors, vec![deadzone]);
    assert!(
        bindings[1].processors.is_empty(),
        "the processor landed on a part instead of the head"
    );
}

/// 🔴 And the head's menu is filtered by what the **composite**
/// produces, not by the action's type — a 2D composite assembles a
/// vector even when it sits under an action typed as a button.
#[test]
fn a_stick_deadzone_is_offered_on_a_2d_composite() {
    use kooch_input::actions::{Composite, ControlType, Processor, VectorMode};

    let composite = Composite::Vector2 {
        mode: VectorMode::Analog,
    };
    let deadzone = Processor::StickDeadzone { min: 0.1, max: 0.9 };
    assert!(
        deadzone.applies_to(composite.control_type()),
        "the one processor a stick composite exists for is not offered on it"
    );
    assert!(
        !deadzone.applies_to(ControlType::Button),
        "filtering by the action's type would hide it under a button action"
    );
}

/// A move off either end is refused rather than clamped: a no-move
/// that still marked the file unsaved would be a lie.
#[test]
fn a_processor_cannot_be_moved_off_the_list() {
    use kooch_input::actions::Processor;

    let mut r = resources();
    edit_input_map(&mut r, &Edit::AddBinding { action: 0 });
    let at = BindingAddress {
        action: 0,
        binding: 0,
    };
    edit_input_map(
        &mut r,
        &Edit::AddProcessor {
            to: b(at),
            processor: Processor::Invert,
        },
    );

    for delta in [-1, 1] {
        r.get_mut::<OpenInputMap>().unwrap().dirty = false;
        edit_input_map(
            &mut r,
            &Edit::MoveProcessor {
                to: b(at),
                index: 0,
                delta,
            },
        );
        assert!(
            !r.get::<OpenInputMap>().unwrap().dirty,
            "moving by {delta} off the end claimed a change"
        );
    }
}

/// 🔴 Order is meaning. The menu offers a list, and a list that
/// evaluated as a set would make reordering a cosmetic no-op — so
/// this asserts against the evaluator, not against the Vec.
#[test]
fn processor_order_changes_the_value() {
    use kooch_input::actions::{Action, Binding, ControlPath, ControlType, Processor, evaluate};
    use kooch_input::ids::KeyCode;
    use kooch_input::mock_backend::MockInputBackend;

    // A key reads 1.0. Scaled to 4, then clamped to 2 → 2.
    // Clamped to 2 first, then scaled → 4.
    let scale = Processor::Scale { factor: 4.0 };
    let clamp = Processor::Clamp {
        min: -2.0,
        max: 2.0,
    };
    let value = |order: [Processor; 2]| {
        let mut binding = Binding::to(ControlPath::Key(KeyCode::Space));
        binding.processors = order.to_vec();
        let action = Action::new("a", ControlType::Axis).bind(binding);
        let mut backend = MockInputBackend::new();
        backend.press_key(KeyCode::Space);
        evaluate(&action, &backend).axis()
    };

    assert_eq!(value([scale, clamp]), 2.0, "scale then clamp");
    assert_eq!(value([clamp, scale]), 4.0, "clamp then scale");
}

/// A 2D processor on a button is skipped by `apply`, so the menu
/// must not offer one — the same trap the composite menu avoids.
#[test]
fn the_processor_menu_only_offers_what_applies() {
    use kooch_input::actions::{ControlType as CT, Processor};

    let offered = |t: CT| Processor::ALL.iter().filter(|p| p.applies_to(t)).count();
    assert!(offered(CT::Button) > 0);
    assert!(
        offered(CT::Vector2) > offered(CT::Button),
        "a 2D action must be offered more than a button, not the same list"
    );
    assert!(
        !Processor::StickDeadzone { min: 0.1, max: 0.9 }.applies_to(CT::Button),
        "a stick deadzone on a button shapes nothing"
    );
}

/// Shorthand for "the processors of this binding".
fn b(at: BindingAddress) -> crate::panels::input_map::ProcessorTarget {
    crate::panels::input_map::ProcessorTarget::Binding(at)
}

fn processors(r: &Resources, at: BindingAddress) -> Vec<kooch_input::actions::Processor> {
    r.get::<OpenInputMap>().unwrap().map.actions[at.action].bindings[at.binding]
        .processors
        .clone()
}

/// 🔴 Deleting a composite deletes its parts.
///
/// Reported from the panel: the head went and the parts stayed. They
/// are invisible once orphaned — `groups` skips a part with no head
/// above it — so they survived in the file and were read by nothing.
#[test]
fn removing_a_composite_takes_its_parts_with_it() {
    use kooch_input::actions::{Composite, Role, VectorMode};

    let mut r = resources();
    // A plain binding after the composite, to prove the delete stops
    // at the parts rather than eating whatever follows.
    edit_input_map(
        &mut r,
        &Edit::AddComposite {
            action: 1,
            composite: Composite::Vector2 {
                mode: VectorMode::DigitalNormalized,
            },
        },
    );
    edit_input_map(&mut r, &Edit::AddBinding { action: 1 });
    assert_eq!(
        r.get::<OpenInputMap>().unwrap().map.actions[1]
            .bindings
            .len(),
        6
    );

    edit_input_map(
        &mut r,
        &Edit::RemoveBinding(BindingAddress {
            action: 1,
            binding: 0,
        }),
    );

    let bindings = &r.get::<OpenInputMap>().unwrap().map.actions[1].bindings;
    assert_eq!(
        bindings.len(),
        1,
        "the composite left {} rows behind: {bindings:#?}",
        bindings.len() - 1
    );
    assert!(
        matches!(bindings[0].role, Role::Whole(_)),
        "the delete ate the plain binding that followed"
    );
}

/// A single part can still be removed on its own — a composite with a
/// direction missing is a valid thing to author on the way to another.
#[test]
fn removing_one_part_leaves_the_composite() {
    use kooch_input::actions::{Composite, Role, VectorMode};

    let mut r = resources();
    edit_input_map(
        &mut r,
        &Edit::AddComposite {
            action: 1,
            composite: Composite::Vector2 {
                mode: VectorMode::Analog,
            },
        },
    );
    edit_input_map(
        &mut r,
        &Edit::RemoveBinding(BindingAddress {
            action: 1,
            binding: 2,
        }),
    );

    let bindings = &r.get::<OpenInputMap>().unwrap().map.actions[1].bindings;
    assert_eq!(bindings.len(), 4, "removing a part took more than itself");
    assert!(matches!(bindings[0].role, Role::CompositeHead(_)));
}

/// 🔴 A `.inputaction` opens, edits and saves as itself.
///
/// It is held as a map of one so the panel needs no second code
/// path — Unity does the same internally for singleton actions — but
/// the file has to keep its shape. Saving a map where an action
/// belongs would produce a file nothing can load.
#[test]
fn a_standalone_action_round_trips_through_the_panel() {
    use kooch_input::actions::{Composite, VectorMode};

    let dir = std::env::temp_dir().join("kooch_single_action_test");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("Jump.inputaction");
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("inputaction.meta"));

    let action = kooch_input::actions::Action::new("jump", ControlType::Button);
    kooch_input::actions::save_action(&action, &path).expect("write");

    let mut r = Resources::new();
    open_input_map(&mut r, &path);

    {
        let open = r.get::<OpenInputMap>().expect("the action did not open");
        assert_eq!(
            open.kind,
            crate::state::OpenInputKind::SingleAction,
            "a .inputaction opened as a map"
        );
        assert_eq!(open.map.actions.len(), 1, "wrapped in more than one");
    }

    // Edited through the same panel actions a map uses.
    edit_input_map(
        &mut r,
        &Edit::AddComposite {
            action: 0,
            composite: Composite::Vector2 {
                mode: VectorMode::Analog,
            },
        },
    );
    handle_asset_op(&EditorAction::SaveInputMap, &mut r);

    // And it is still one action on disk, not a map.
    let text = std::fs::read_to_string(&path).expect("read back");
    let reloaded: kooch_input::actions::Action =
        ron::from_str(&text).expect("a .inputaction must still parse as an action");
    assert_eq!(reloaded.name, "jump");
    assert!(
        !reloaded.bindings.is_empty(),
        "the composite added in the panel did not reach the file"
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("inputaction.meta"));
}

/// A composite's parameters are editable, and only on its head.
#[test]
fn a_composites_mode_can_be_changed() {
    use kooch_input::actions::{Composite, Role, VectorMode};

    let mut r = resources();
    let digital = Composite::Vector2 {
        mode: VectorMode::DigitalNormalized,
    };
    let analog = Composite::Vector2 {
        mode: VectorMode::Analog,
    };
    edit_input_map(
        &mut r,
        &Edit::AddComposite {
            action: 1,
            composite: digital,
        },
    );

    let head = BindingAddress {
        action: 1,
        binding: 0,
    };
    edit_input_map(
        &mut r,
        &Edit::SetComposite {
            at: head,
            composite: analog,
        },
    );
    let open = r.get::<OpenInputMap>().unwrap();
    assert!(
        matches!(&open.map.actions[1].bindings[0].role, Role::CompositeHead(c) if *c == analog)
    );

    // Aimed at a part it is a no-op, not a part turned into a head.
    let part = BindingAddress {
        action: 1,
        binding: 1,
    };
    let before = r.get::<OpenInputMap>().unwrap().map.clone();
    edit_input_map(
        &mut r,
        &Edit::SetComposite {
            at: part,
            composite: digital,
        },
    );
    assert_eq!(r.get::<OpenInputMap>().unwrap().map, before);
}

/// 🔴 Only composites that fit the action are offered. A 2D composite
/// under a Button action reads as nothing, with no error to say why.
#[test]
fn a_composite_declares_the_type_it_produces() {
    use kooch_input::actions::{Composite, ControlType as CT};

    let fits = |t: CT| {
        Composite::ALL
            .iter()
            .filter(|c| c.control_type() == t)
            .count()
    };
    assert!(fits(CT::Vector2) >= 1, "nothing to offer a 2D action");
    assert!(fits(CT::Vector3) >= 1, "nothing to offer a 3D action");
    assert!(fits(CT::Axis) >= 1, "nothing to offer an axis action");
    assert!(fits(CT::Button) >= 1, "nothing to offer a button action");
}

/// Selecting is not an edit: it must not mark the file unsaved.
#[test]
fn selecting_does_not_dirty_the_document() {
    let mut r = resources();
    edit_input_map(&mut r, &Edit::Select(Selection::Action(1)));
    let open = r.get::<OpenInputMap>().unwrap();
    assert_eq!(open.selected, Some(Selection::Action(1)));
    assert!(!open.dirty, "selecting a row claimed an unsaved change");
}

/// A new action arrives selected, so the properties pane is already
/// showing the name field rather than leaving a row to hunt for.
#[test]
fn a_new_action_is_selected_on_arrival() {
    let mut r = resources();
    edit_input_map(&mut r, &Edit::AddAction);
    let open = r.get::<OpenInputMap>().unwrap();
    assert_eq!(open.selected, Some(Selection::Action(2)));
}

/// Setting the same value is not a change — otherwise clicking the
/// type an action already has would mark the file unsaved.
#[test]
fn a_no_op_edit_leaves_the_document_clean() {
    let mut r = resources();
    edit_input_map(
        &mut r,
        &Edit::SetControlType {
            action: 0,
            control_type: ControlType::Button,
        },
    );
    assert!(!r.get::<OpenInputMap>().unwrap().dirty);

    edit_input_map(
        &mut r,
        &Edit::SetControlType {
            action: 0,
            control_type: ControlType::Axis,
        },
    );
    assert!(r.get::<OpenInputMap>().unwrap().dirty);
}
