use super::*;

/// The events that cost the most and change the least.
#[test]
fn redundant_events_do_not_ask_for_a_frame() {
    assert!(
        !wants_a_frame(&WindowEvent::Moved(winit::dpi::PhysicalPosition::new(3, 4))),
        "moving the window changes nothing inside it",
    );
    assert!(
        !wants_a_frame(&WindowEvent::AxisMotion {
            device_id: winit::event::DeviceId::dummy(),
            axis: 0,
            value: 1.0,
        }),
        "AxisMotion duplicates CursorMoved, at twice the rate",
    );
}

/// And the ones that must never be dropped: while the loop idles,
/// this is the only thing that produces a frame at all.
#[test]
fn input_always_asks_for_a_frame() {
    assert!(wants_a_frame(&WindowEvent::CursorMoved {
        device_id: winit::event::DeviceId::dummy(),
        position: winit::dpi::PhysicalPosition::new(1.0, 2.0),
    }));
    assert!(wants_a_frame(&WindowEvent::Focused(true)));
    assert!(wants_a_frame(&WindowEvent::CursorLeft {
        device_id: winit::event::DeviceId::dummy(),
    }));
}
