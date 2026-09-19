use super::*;
use kooch_core::window_mode::{DisplayModes, Resolution};

/// A wide image in a tall panel fills the width and leaves bars above and below.
#[test]
fn a_wide_size_letterboxes() {
    let panel = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(400.0, 400.0));
    let image = fit(panel, [1920, 1080]);
    assert_eq!(image.width(), 400.0);
    assert_eq!(image.height(), 225.0);
    assert_eq!(image.center(), panel.center());
}

/// A tall image in a wide panel fills the height instead.
#[test]
fn a_tall_size_pillarboxes() {
    let panel = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 300.0));
    let image = fit(panel, [1080, 1920]);
    assert_eq!(image.height(), 300.0);
    assert!(image.width() < 800.0);
}

/// Each size is offered once, whatever the refresh rates behind it.
#[test]
fn refresh_rates_collapse() {
    let mode = |width, height, refresh_mhz| Resolution {
        width,
        height,
        refresh_mhz,
    };
    let modes = DisplayModes {
        modes: vec![
            mode(2560, 1440, 144_000),
            mode(2560, 1440, 60_000),
            mode(1920, 1080, 60_000),
        ],
        exclusive: true,
    };
    assert_eq!(sizes(&modes), [[2560, 1440], [1920, 1080]]);
}
