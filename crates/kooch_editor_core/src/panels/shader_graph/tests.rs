use egui::{Pos2, Rect, Vec2};

use super::*;

fn panel() -> Rect {
    Rect::from_min_size(Pos2::new(300.0, 120.0), Vec2::new(900.0, 600.0))
}

/// 🔴 Opening a graph shows all of it, centred: a graph that opens off to one side, or only partly on
/// screen, is a graph the user has to go looking for.
#[test]
fn a_graph_opens_framed() {
    let bounds = Rect::from_min_size(Pos2::new(-400.0, -50.0), Vec2::new(2400.0, 900.0));

    let view = framed(bounds, panel());

    let shown = Rect::from_min_max(view * bounds.min, view * bounds.max);
    assert!(
        (shown.center() - panel().center()).length() < 0.01,
        "not centred: {shown:?}"
    );
    assert!(
        panel().expand(0.01).contains_rect(shown),
        "part of the graph is off screen: {shown:?}",
    );
}

/// A small graph is not blown up past actual size to fill the panel.
#[test]
fn a_small_graph_stays_actual_size() {
    let bounds = Rect::from_min_size(Pos2::ZERO, Vec2::new(200.0, 100.0));
    assert_eq!(framed(bounds, panel()).scaling, 1.0);
}
