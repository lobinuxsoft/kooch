use super::*;

/// Runs `body` against a real `Ui`, since this reads egui's layout.
fn with_ui<R>(width: f32, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let ctx = egui::Context::default();
    let mut body = Some(body);
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(width, 600.0),
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

/// The bug this widget exists for: a short name must still give a
/// row that reaches the far edge of the panel, or everything to the
/// right of the text ignores the pointer.
#[test]
fn a_short_name_still_fills_the_panel() {
    let (row, available) = with_ui(400.0, |ui| {
        let available = ui.available_width();
        let resp = SelectableRow::new("a").show(ui);
        (resp.rect.width(), available)
    });
    assert!(
        (row - available).abs() < 1.0,
        "row was {row} wide in {available} of panel"
    );
}

/// …and a name far too long must not push past it either, or the
/// panel scrolls sideways to reveal nothing.
#[test]
fn a_very_long_name_does_not_widen_the_row() {
    let (row, available) = with_ui(400.0, |ui| {
        let available = ui.available_width();
        let resp = SelectableRow::new("name ".repeat(200)).show(ui);
        (resp.rect.width(), available)
    });
    assert!(
        row <= available + 1.0,
        "row was {row} wide in {available} of panel — the text was not truncated"
    );
}

/// Two rows of very different name lengths have to be the same size,
/// or the selection highlight is ragged and a virtualized list
/// cannot predict where row N sits.
#[test]
fn every_row_is_the_same_size_whatever_it_says() {
    let (short, long, expected) = with_ui(400.0, |ui| {
        let expected = row_height(ui);
        let short = SelectableRow::new("x").show(ui).rect;
        let long = SelectableRow::new("a much, much longer label")
            .show(ui)
            .rect;
        (short, long, expected)
    });
    assert!((short.width() - long.width()).abs() < 1.0, "widths differ");
    assert!(
        (short.height() - expected).abs() < 1.0,
        "row height {} is not what row_height promised ({expected})",
        short.height()
    );
}
