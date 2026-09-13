//! [`RemotePlugin`] — starts the server and drains it each tick.

use kooch_core::app::App;
use kooch_core::frame_pacing::{FramePace, FrameRequest, FrameWaker};
use kooch_core::plugin::Plugin;
use kooch_core::resource::Resources;
use kooch_core::run_state::Playing;
use kooch_core::stage::Stage;
use kooch_core::time::Time;

use crate::handlers::handle;
use crate::server::RemoteServer;

/// Adds the remote editor server to a running project: a local socket on a dedicated thread and a
/// [`Stage::First`] system answering queued requests. A bind failure leaves it inert rather than
/// aborting.
pub struct RemotePlugin {
    /// Socket name to bind, or `None` to read it from the environment.
    name: Option<String>,
}

impl RemotePlugin {
    /// The plugin on the socket name the launcher passed down ([`NAME_ENV`](crate::NAME_ENV)), or
    /// [`DEFAULT_NAME`](crate::DEFAULT_NAME) when run by hand.
    pub fn new() -> Self {
        Self { name: None }
    }
}

impl Default for RemotePlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for RemotePlugin {
    fn build(&self, app: &mut App) {
        // Present since `App::new`; cloned rather than borrowed because
        // the listener thread outlives this call.
        let waker = app
            .resources
            .get::<FrameWaker>()
            .cloned()
            .unwrap_or_default();
        let started = match &self.name {
            Some(name) => RemoteServer::start_waking(name, waker),
            None => RemoteServer::start_from_env(waker),
        };
        match started {
            Ok(server) => {
                app.insert_resource(server);
                // #656 — under an editor nothing simulates between edits, so the baseline sleeps;
                // the socket wakes it and Play overrides it.
                app.insert_resource(FrameRequest::new(FramePace::Wait));
                // First stage: apply remote edits before this frame's
                // systems observe the world, so a client edit lands the
                // same tick it arrives.
                app.add_system(Stage::First, serve_pending_system);
                // Last, so it sees the flag the frame ended with rather
                // than the one it started with — pressing Play must not
                // cost a frame of latency at the far end of a socket.
                app.add_system(Stage::Last, pace_system);
            }
            Err(e) => {
                tracing::warn!("remote editor server disabled: {e}");
            }
        }
    }
}

/// Paces the loop while playing to the fixed timestep: a host draws nothing, so faster frames find
/// no step owed (#656). `After` is a ceiling; the socket still wakes it.
fn pace_system(resources: &mut Resources) {
    if !Playing::is_playing(resources) {
        return;
    }
    report_frame_cost(resources);

    let pace = match resources.get::<Time>() {
        Some(time) => FramePace::After(time.until_next_fixed_step()),
        // No clock to pace against; the pre-#656 behaviour.
        None => FramePace::Continuous,
    };
    FrameRequest::raise(resources, pace);
}

/// Environment variable enabling the frame-cost report — off, since the host's log lands in the
/// editor's Console every frame (#656).
const PROFILE_ENV: &str = "KOOCH_REMOTE_PROFILE";

/// Rolling frame-cost totals for the report below.
#[derive(Default)]
struct FrameCostProbe {
    frames: u32,
    /// The running total at the start of the window, so the report can
    /// subtract rather than count steps itself.
    fixed_steps: u64,
    work: std::time::Duration,
    serving: std::time::Duration,
    window_start: Option<std::time::Instant>,
}

/// Every two seconds, where the host's frame goes: `serving` dominating means send less, `work`
/// means the simulation (#645 measures the editor's half). Runs in `Stage::Last`.
fn report_frame_cost(resources: &mut Resources) {
    if std::env::var_os(PROFILE_ENV).is_none() {
        return;
    }

    let Some((work, fixed_steps)) = resources
        .get::<Time>()
        .map(|time| (time.frame_start().elapsed(), time.fixed_count()))
    else {
        return;
    };
    let served = resources
        .get::<ServeCost>()
        .map(|cost| cost.0)
        .unwrap_or_default();

    let probe = match resources.get_mut::<FrameCostProbe>() {
        Some(probe) => probe,
        None => {
            resources.insert(FrameCostProbe::default());
            // `fixed_steps` is a running total, so the first window has
            // no baseline to subtract and is skipped rather than
            // reported as "every step since startup happened just now".
            if let Some(probe) = resources.get_mut::<FrameCostProbe>() {
                probe.fixed_steps = fixed_steps;
            }
            return;
        }
    };

    probe.frames += 1;
    probe.work += work;
    probe.serving += served;
    let now = std::time::Instant::now();
    let started = *probe.window_start.get_or_insert(now);
    let window = now.duration_since(started);
    if window < std::time::Duration::from_secs(2) {
        return;
    }

    let frames = probe.frames.max(1);
    let steps = fixed_steps.saturating_sub(probe.fixed_steps);
    let work_ms = probe.work.as_secs_f32() * 1000.0 / frames as f32;
    let serving_ms = probe.serving.as_secs_f32() * 1000.0 / frames as f32;
    tracing::info!(
        fps = frames as f32 / window.as_secs_f32(),
        steps_per_s = steps as f32 / window.as_secs_f32(),
        work_ms,
        serving_ms,
        simulating_ms = work_ms - serving_ms,
        "host frame cost",
    );

    *probe = FrameCostProbe {
        fixed_steps,
        window_start: Some(now),
        ..Default::default()
    };
}

/// What answering the editor cost this frame.
#[derive(Default)]
struct ServeCost(std::time::Duration);

/// Executes every queued request and replies to each waiting listener.
fn serve_pending_system(resources: &mut Resources) {
    let Some(pending) = resources
        .get::<RemoteServer>()
        .map(|server| server.take_pending())
    else {
        return;
    };
    // Recorded even when empty: a frame that served nothing must not
    // carry the previous frame's cost into the average.
    if pending.is_empty() {
        record_serve_cost(resources, std::time::Duration::ZERO);
        return;
    }

    let started = std::time::Instant::now();
    for item in pending {
        let response = handle(&item.request, resources);
        // A failed send means the listener stopped waiting (client hung
        // up); drop the response and move on.
        let _ = item.reply.send(response);
    }
    record_serve_cost(resources, started.elapsed());
}

fn record_serve_cost(resources: &mut Resources, cost: std::time::Duration) {
    match resources.get_mut::<ServeCost>() {
        Some(slot) => slot.0 = cost,
        None => {
            resources.insert(ServeCost(cost));
        }
    }
}
