//! The host's side of the editor's input: snapshots in, rumble and cursor requests out.

/// Receives the input the editor captured: a windowless host gets no key events, so Play plus a key
/// did nothing (#710). `RemoteInputBackend` feeds the same `InputBackend` a game reads; here for
/// the same reason as `PhysicsRemotePlugin`.
pub struct InputRemotePlugin;

impl kooch_core::plugin::Plugin for InputRemotePlugin {
    fn build(&self, app: &mut kooch_core::app::App) {
        let backend: Box<dyn kooch_input::InputBackend> =
            Box::new(kooch_input::RemoteInputBackend::new());
        app.insert_resource(backend);
        // The host has no `InputPlugin`, so this is its frame boundary: without it no edge ever
        // expired and the event queue grew for the whole Play session (#766).
        app.add_event::<kooch_input::InputEvent>();
        app.add_system(kooch_core::stage::Stage::Input, pump_remote_input);
        // After gameplay, so a rumble asked this frame leaves with the next pull.
        app.add_system(kooch_core::stage::Stage::PostUpdate, forward_rumble);
        app.add_system(kooch_core::stage::Stage::PostUpdate, forward_cursor);
        app.add_system(
            kooch_core::stage::Stage::Startup,
            |resources: &mut kooch_core::resource::Resources| {
                if !resources.contains::<kooch_remote::extensions::RemoteExtensions>() {
                    resources.insert(kooch_remote::extensions::RemoteExtensions::default());
                }
                let Some(extensions) =
                    resources.get_mut::<kooch_remote::extensions::RemoteExtensions>()
                else {
                    return;
                };
                extensions.register(
                    "input.state",
                    Box::new(|resources, payload| {
                        let snapshot: kooch_input::InputSnapshot =
                            kooch_remote::serde_json::from_value(payload.clone())
                                .map_err(|e| format!("malformed input snapshot: {e}"))?;
                        let backend = resources
                            .get_mut::<Box<dyn kooch_input::InputBackend>>()
                            .ok_or_else(|| "this host has no input backend".to_owned())?;
                        // Through the trait, not a downcast: a real-device backend ignores
                        // snapshots by default.
                        backend.apply_snapshot(&snapshot);
                        Ok(kooch_remote::serde_json::Value::Null)
                    }),
                );
            },
        );
    }

    fn name(&self) -> &str {
        "InputRemotePlugin"
    }
}

/// Hands the editor the rumbles the game asked for: the editor's process holds the pads (#57).
fn forward_rumble(resources: &mut kooch_core::resource::Resources) {
    let Some(backend) = resources.get_mut::<Box<dyn kooch_input::InputBackend>>() else {
        return;
    };
    let rumbles = backend.take_rumbles();
    if rumbles.is_empty() {
        return;
    }
    let outbox = resources.get_or_default::<kooch_remote::HostOutbox>();
    for (pad, rumble) in rumbles {
        let signal = kooch_input::RumbleSignal {
            pad: pad.index(),
            rumble,
        };
        match kooch_remote::serde_json::to_value(signal) {
            Ok(payload) => outbox.push(kooch_input::RUMBLE_SIGNAL, payload),
            Err(error) => tracing::warn!(%error, "could not encode a rumble"),
        }
    }
}

/// The cursor mode last handed to the editor.
#[derive(Default)]
struct SentCursor(Option<kooch_input::CursorMode>);

/// Hands the editor the game's [`kooch_input::CursorMode`] when it changes, so a click on the
/// game's UI keeps the cursor free in the editor as it does in a built game.
fn forward_cursor(resources: &mut kooch_core::resource::Resources) {
    let Some(wanted) = resources.get::<kooch_input::CursorMode>().copied() else {
        return;
    };
    let sent = resources.get_or_default::<SentCursor>();
    if sent.0 == Some(wanted) {
        return;
    }
    sent.0 = Some(wanted);
    match kooch_remote::serde_json::to_value(wanted) {
        Ok(payload) => resources
            .get_or_default::<kooch_remote::HostOutbox>()
            .push(kooch_input::CURSOR_SIGNAL, payload),
        Err(error) => tracing::warn!(%error, "could not encode the cursor mode"),
    }
}

/// The host's half of `InputPlugin`'s frame: publish the edges the snapshots brought, hand the
/// events to the game.
fn pump_remote_input(resources: &mut kooch_core::resource::Resources) {
    let Some(backend) = resources.get_mut::<Box<dyn kooch_input::InputBackend>>() else {
        return;
    };
    backend.begin_frame();
    let events = backend.poll();
    if let Some(buffer) = resources.get_mut::<kooch_core::event::Events<kooch_input::InputEvent>>()
    {
        for event in events {
            buffer.send(event);
        }
    }
}
