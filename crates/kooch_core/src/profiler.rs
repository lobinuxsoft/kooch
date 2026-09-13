//! Where a profiler is expected to be, so both ends agree (#785).

/// Port the profiler serves on, and the one the panel offers to connect to.
pub const DEFAULT_PORT: u16 = 8585;

/// Environment variable that overrides the address, e.g. `KOOCH_PROFILER_ADDR=0.0.0.0:9000`.
pub const ADDR_VAR: &str = "KOOCH_PROFILER_ADDR";

/// What the game listens on unless told otherwise.
pub fn default_bind_addr() -> String {
    std::env::var(ADDR_VAR).unwrap_or_else(|_| format!("0.0.0.0:{DEFAULT_PORT}"))
}

/// What the panel offers to connect to before anyone types an address.
pub fn default_connect_addr() -> String {
    format!("127.0.0.1:{DEFAULT_PORT}")
}
