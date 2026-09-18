//! Sending the editor's input to the project it is driving.

use kooch_core::resource::Resources;
use kooch_input::{InputBackend, InputSnapshot};
use kooch_remote::protocol::Method;

use crate::remote_session::RemoteState;

/// Reads the editor's input and posts it to the host, while playing.
pub(crate) fn send_input_to_host(resources: &mut Resources) {
    if !should_send(resources) {
        return;
    }

    let Some(backend) = resources.get::<Box<dyn InputBackend>>() else {
        return;
    };
    let snapshot = InputSnapshot::from_backend(backend.as_ref());

    let Some(state) = resources.get_mut::<RemoteState>() else {
        return;
    };
    // Nothing held and nothing moving, and the host already knows: a snapshot per frame of an idle
    // keyboard is a round trip that changes nothing. The *first* idle one still goes, because it is
    // what releases whatever was held when the player let go.
    if snapshot.is_idle() && state.last_input_was_idle {
        return;
    }
    state.last_input_was_idle = snapshot.is_idle();

    let Some(session) = state.session.as_ref().filter(|_| state.is_connected()) else {
        return;
    };
    let payload = match kooch_remote::serde_json::to_value(&snapshot) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::warn!(%error, "could not encode the input snapshot");
            return;
        }
    };
    // 🔴 Sent, not asked. The reply was already being thrown away with `let _ =`, and waiting for it
    // cost 5.9 ms a frame — the editor slept until the host reached its next `Stage::First` to
    // receive an acknowledgement nobody read (#1013).
    let _ = session.client().notify(Method::Extension {
        name: "input.state".to_owned(),
        payload,
    });
}

/// Whether this frame's input belongs to the game.
fn should_send(resources: &Resources) -> bool {
    resources
        .get::<crate::input_focus::InputFocus>()
        .is_some_and(|focus| focus.belongs_to(crate::input_focus::InputOwner::Game))
}

#[cfg(test)]
mod tests;
