//! Launcher hub for Kóoch projects.
//!
//! Opens the launch screen where users create, open, or select projects.
//! Each project is a Rust crate that compiles and runs as its own binary
//! with the editor embedded (`cargo run`) or as a game (`cargo run --
//! --game`).
//!
//! The full editor bootstrap lives in
//! [`kooch_editor_core::bootstrap`](kooch_editor_core::run_editor) so a
//! generated project can reuse the exact same plugin set.
//!
//! Run with: cargo run -p kooch_editor

fn main() {
    kooch_editor_core::run_editor();
}

#[cfg(test)]
mod profiler_required_tests;
