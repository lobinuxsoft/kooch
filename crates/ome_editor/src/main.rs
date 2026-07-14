//! Launcher hub for Oh My Engine projects.
//!
//! Opens the launch screen where users create, open, or select projects.
//! Each project is a Rust crate that compiles and runs as its own binary
//! with the editor embedded (`cargo run`) or as a game (`cargo run --
//! --game`).
//!
//! The full editor bootstrap lives in
//! [`ome_editor_core::bootstrap`](ome_editor_core::run_editor) so a
//! generated project can reuse the exact same plugin set.
//!
//! Run with: cargo run -p ome_editor

fn main() {
    ome_editor_core::run_editor();
}
