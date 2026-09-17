//! #1159 — the Shaders table under "A running game" reads what the game sent, not this editor.

use std::sync::{Arc, Mutex};

/// A frame the way a client receives one: a shader's scopes, twice, beside a pass that is not one.
#[test]
fn received_shader_scopes_sum() {
    let view = Arc::new(Mutex::new(puffin::FrameView::default()));
    let sink_view = Arc::clone(&view);
    let sink = puffin::GlobalProfiler::lock().add_sink(Box::new(move |frame| {
        sink_view.lock().unwrap().add_frame(frame)
    }));
    puffin::set_scopes_on(true);
    puffin::GlobalProfiler::lock().emit_scope_snapshot();

    for _ in 0..2 {
        {
            puffin::profile_scope!("shade");
            for _ in 0..2 {
                puffin::profile_scope!("shader probe");
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        puffin::GlobalProfiler::lock().new_frame();
    }
    puffin::set_scopes_on(false);
    puffin::GlobalProfiler::lock().remove_sink(sink);

    let costs = kooch_editor_core::shader_costs_in(&view.lock().unwrap());

    assert_eq!(costs.len(), 1, "only shader scopes count: {costs:?}");
    let (label, ms) = &costs[0];
    assert_eq!(label, "shader probe");
    assert!(*ms >= 4.0, "both scopes of the frame add up: {ms} ms");
}
