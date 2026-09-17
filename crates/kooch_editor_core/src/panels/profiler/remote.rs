//! The profiler reading a **game**, over the network (#785).

use egui::Ui;

use kooch_core::profiler::default_connect_addr;

use crate::project::EditorConfig;

/// Everything the remote view needs to survive between frames.
static REMOTE: std::sync::OnceLock<std::sync::Mutex<Remote>> = std::sync::OnceLock::new();

struct Remote {
    /// Address being edited, which is not necessarily the one connected.
    addr: String,
    /// 🔴 Dropping this closes the connection — its thread stops on an
    /// `alive` flag the `Drop` impl clears. Keeping it in a static is
    /// what makes Connect mean "stay connected".
    client: Option<puffin_http::Client>,
    /// Separate from the local panel's, so switching sources does not
    /// reset the zoom, the pause, or the sort column of either.
    ui: puffin_egui::ProfilerUi,
}

impl Default for Remote {
    fn default() -> Self {
        Self {
            addr: EditorConfig::load()
                .profiler_addr
                .unwrap_or_else(default_connect_addr),
            client: None,
            ui: puffin_egui::ProfilerUi::default(),
        }
    }
}

/// Draws the connection controls and, once connected, the game's frames.
pub(super) fn draw(ui: &mut Ui, catalog: &[crate::panels::inspector::AssetCatalogEntry]) {
    let mut remote = REMOTE
        .get_or_init(Default::default)
        .lock()
        .expect("the remote profiler mutex is never held across a panic");
    let Remote {
        addr,
        client,
        ui: profiler_ui,
    } = &mut *remote;

    ui.horizontal(|ui| {
        ui.label("Game address:");
        // Enter connects, because typing an address and then hunting for
        // a button is the wrong shape for something retyped every session
        // until it is remembered.
        let entered = ui
            .add(
                egui::TextEdit::singleline(addr)
                    .desired_width(180.0)
                    .hint_text("192.168.0.36:8585"),
            )
            .lost_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter));

        let connected = client.is_some();
        let button = if connected { "Disconnect" } else { "Connect" };
        if ui.button(button).clicked() || (entered && !connected) {
            if connected {
                // Explicit, rather than waiting for the static to be
                // replaced: the `Drop` is what stops the thread.
                *client = None;
            } else {
                *client = Some(puffin_http::Client::new(addr.clone()));
                remember(addr);
            }
        }

        match client.as_ref() {
            // 🔴 "Connected" here is not "the button was pressed". `puffin_http::Client` retries
            // forever in its own thread and reconnects after a drop, so a game that has not been
            // started yet is a perfectly normal state — and so is one that just exited.
            Some(client) if client.connected() => {
                ui.colored_label(egui::Color32::from_rgb(0x4c, 0xaf, 0x50), "● connected");
            }
            Some(_) => {
                ui.spinner();
                ui.label("waiting for the game…");
            }
            None => {
                ui.label("not connected");
            }
        }
    });

    let Some(connection) = client.as_ref() else {
        ui.add_space(8.0);
        ui.label(
            "Build the game with the profiling feature and run it; it listens on 0.0.0.0:8585 \
             and logs the address at startup.",
        );
        ui.add_space(4.0);
        ui.code("cargo build --release --features profiling");
        ui.add_space(8.0);
        ui.label(
            "⚠️ A capture taken with the handheld plugged in and unthrottled says nothing \
             about the 10 W target.",
        );
        return;
    };

    let mut reconnect = false;
    {
        let mut frames = connection.frame_view();
        super::keep_all_frames(&mut frames);
        ui.separator();
        let costs = super::shaders::shader_costs_in(&frames);
        super::shaders::draw(ui, &super::shaders::named_shader_costs(catalog, &costs));
        ui.horizontal(|ui| {
            let stats = frames.stats();
            ui.label(format!(
                "{} frames, {:.1} MiB",
                frames.all_uniq().count(),
                stats.bytes_of_ram_used() as f64 / (1024.0 * 1024.0),
            ));
            if ui
                .button(format!("{} Clear history", crate::icons::TRASH))
                .on_hover_text("Reconnects, which is what drops the frames received so far")
                .clicked()
            {
                // 🔴 Clearing by hand would cost the scope NAMES.
                reconnect = true;
            }
            if ui
                .button("Save capture")
                .on_hover_text("Writes a .puffin file the standalone viewer can open")
                .clicked()
            {
                match super::local::save_view(&frames) {
                    Ok(path) => tracing::info!(
                        target: "kooch_editor::profiler",
                        path = %path.display(),
                        "wrote a profiler capture of the game",
                    ),
                    Err(error) => tracing::error!(
                        target: "kooch_editor::profiler",
                        %error,
                        "could not write the profiler capture",
                    ),
                }
            }
        });
        ui.separator();

        // 🟢 The flamegraph is drawn while frames keep arriving, and here that is correct — the
        // opposite of the local view, which hides it while recording because drawing it cost 10.97
        // ms of a 15.98 ms frame.
        profiler_ui.ui(ui, &mut puffin_egui::MaybeMutRef::MutRef(&mut frames));
    }

    if reconnect {
        *client = Some(puffin_http::Client::new(addr.clone()));
    }
}

/// Stores the address for the next session.
fn remember(addr: &str) {
    let mut config = EditorConfig::load();
    config.profiler_addr = Some(addr.to_owned());
    if let Err(error) = config.save() {
        tracing::warn!(
            target: "kooch_editor::profiler",
            %error,
            "could not remember the profiler address",
        );
    }
}
