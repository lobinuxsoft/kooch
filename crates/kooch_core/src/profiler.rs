//! Where a profiler is expected to be, so both ends agree (#785).
//!
//! The game opens the socket (`kooch::profiler::ProfilingPlugin`) and the
//! editor's Profiler panel connects to it. Neither crate can see the
//! other — the facade depends on the editor, not the reverse — so the two
//! numbers they have to agree on live here, in the crate underneath both.

/// Port the profiler serves on, and the one the panel offers to connect
/// to.
///
/// 8585 is puffin's own default, so `puffin_viewer --url <host>:8585`
/// works against a Kóoch game with nothing else to remember.
pub const DEFAULT_PORT: u16 = 8585;

/// Environment variable that overrides the address, e.g.
/// `KOOCH_PROFILER_ADDR=0.0.0.0:9000`.
///
/// Read by the game when it binds. It exists so a build already copied
/// onto a device can be moved off a taken port without compiling a new
/// one — recompiling for the handheld is a cross-compile, not an edit.
pub const ADDR_VAR: &str = "KOOCH_PROFILER_ADDR";

/// What the game listens on unless told otherwise.
///
/// 🔴 `0.0.0.0`, not `127.0.0.1`. Bound to loopback the game is reachable
/// only from the machine running it, which is the handheld — the one
/// machine that will not be running the viewer. The symptom is a
/// connection that times out with nothing logged at either end.
pub fn default_bind_addr() -> String {
    std::env::var(ADDR_VAR).unwrap_or_else(|_| format!("0.0.0.0:{DEFAULT_PORT}"))
}

/// What the panel offers to connect to before anyone types an address.
///
/// Loopback, because a game launched from the editor's Play button runs
/// on this machine. The handheld's address is typed once and remembered.
pub fn default_connect_addr() -> String {
    format!("127.0.0.1:{DEFAULT_PORT}")
}
