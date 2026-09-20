//! Typing a layer name (#1218). The table is rebuilt from the file every frame, so what is being
//! typed has to live somewhere else until the field is let go — this drives the widget the way a
//! keyboard does and reads the action that comes out.

use kooch_core::Guid;
use kooch_core::layers::LayerNames;

use crate::actions::EditorAction;

/// Draws the layer table for a few frames, feeding `events` on the frame after the first, and hands
/// back every action it pushed.
fn typing(events: Vec<Vec<egui::Event>>) -> Vec<EditorAction> {
    // egui's font atlas is global, and two contexts building one at once deadlock it: the same
    // lock the id-stability probe takes for the same reason.
    let _guard = crate::panels::id_stability_probe::PROBE_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let ctx = egui::Context::default();
    let guid = Guid::new_v4();
    let names = LayerNames::default();
    let mut actions = Vec::new();
    for frame in 0..events.len() {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 1200.0),
            )),
            events: events[frame].clone(),
            ..Default::default()
        };
        ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                super::asset_view::draw_layers(ui, guid, &names, &mut actions);
            });
        });
    }
    actions
}

/// Tab reaches the first field of the table: the rows are focusable, which every step below needs.
fn tab() -> Vec<egui::Event> {
    vec![egui::Event::Key {
        key: egui::Key::Tab,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]
}

fn enter() -> Vec<egui::Event> {
    vec![egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]
}

/// 🔴 The bug this file exists for: the row is rebuilt from the file each frame, so a buffer rebuilt
/// with it loses the keystroke and the name can never be changed. What is typed has to survive the
/// frame and reach the action.
#[test]
fn a_typed_name_reaches_the_action() {
    let typed = typing(vec![
        Vec::new(),
        tab(),
        vec![egui::Event::Text("Water".to_owned())],
        enter(),
        Vec::new(),
    ]);
    let named: Vec<&EditorAction> = typed
        .iter()
        .filter(|action| matches!(action, EditorAction::RenameLayer { .. }))
        .collect();
    assert_eq!(
        named.len(),
        1,
        "one commit for one gesture, got {}",
        typed.len()
    );
    let EditorAction::RenameLayer { index, name, .. } = named[0] else {
        unreachable!()
    };
    assert_eq!(*index, 0, "the first row is the one that was typed into");
    assert!(
        name.contains("Water"),
        "the typed text never reached the action: {name:?}",
    );
}

/// 🔴 A write per keystroke would round-trip the file to the running project on every letter. The
/// name is committed once, when the field is let go.
#[test]
fn typing_alone_writes_nothing() {
    let typed = typing(vec![
        Vec::new(),
        tab(),
        vec![egui::Event::Text("Wa".to_owned())],
        vec![egui::Event::Text("ter".to_owned())],
    ]);
    assert!(
        !typed
            .iter()
            .any(|action| matches!(action, EditorAction::RenameLayer { .. })),
        "a keystroke wrote the file",
    );
}
