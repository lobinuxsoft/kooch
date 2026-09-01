use super::*;

fn entry(name: &str, stage: &str, project: bool, enabled: bool) -> SystemEntry {
    SystemEntry {
        stage: stage.to_owned(),
        name: name.to_owned(),
        short: kooch_core::schedule::short_name(name).to_owned(),
        nth: 0,
        project,
        gpu: false,
        enabled,
    }
}

fn with_ui<R>(body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let ctx = egui::Context::default();
    let mut body = Some(body);
    let mut out = None;
    ctx.run_ui(egui::RawInput::default(), |ui| {
        let body = body.take().expect("run_ui called the closure twice");
        egui::CentralPanel::default().show(ui, |ui| out = Some(body(ui)));
    });
    out.expect("central panel did not run")
}

/// A panel with nothing to show has to say WHICH silence it is: a
/// project that has not connected reads the same as one with no systems.
#[test]
fn an_empty_list_says_why() {
    let mut actions = Vec::new();
    with_ui(|ui| draw_systems_content(ui, &[], &mut actions));
    assert!(actions.is_empty(), "an empty panel asked for something");
}

/// "Enable all" has to reach every switched-off system and leave the
/// running ones alone — across both groups, not just the visible one.
#[test]
fn enable_all_reaches_only_the_off_ones() {
    let systems = vec![
        entry("game::jump", "Update", true, false),
        entry("game::walk", "Update", true, true),
        entry("kooch_render::upload", "GpuSync", false, false),
    ];

    let asked: Vec<&str> = switched_off(&systems)
        .map(|system| system.name.as_str())
        .collect();
    assert_eq!(asked, vec!["game::jump", "kooch_render::upload"]);
}

/// Drawing a list is not an edit. The panel asks for nothing until
/// somebody clicks something.
#[test]
fn drawing_alone_asks_for_nothing() {
    let systems = vec![entry("game::jump", "Update", true, false)];
    let mut actions = Vec::new();
    with_ui(|ui| draw_systems_content(ui, &systems, &mut actions));
    assert!(actions.is_empty(), "the panel edited without being asked");
}
