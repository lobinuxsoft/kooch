//! Lays out a distributable editor: the binary, and the engine source
//! beside it (#754 phase 2).
//!
//! ```sh
//! cargo build --release -p kooch_editor
//! cargo run --release --features editor --example package_editor -- dist/
//! ```
//!
//! Produces:
//!
//! ```text
//! dist/
//!   kooch_editor            the binary
//!   engine/                 what a project gets vendored into it
//!   assets/                 what the editor itself renders with
//! ```
//!
//! # Why the source is beside the binary
//!
//! A project compiles the engine — gameplay is native Rust and links it
//! as an `rlib` — so an editor that ships only a binary produces
//! projects that cannot build. `engine_vendor::vendor_source` looks for
//! exactly this `engine/` next to the executable.
//!
//! Embedding the source *inside* the binary would make the install one
//! file, and is the obvious follow-up. It is not this, because it makes
//! every incremental build of the editor re-pack the engine tree.
//!
//! # Not a cross-compiler
//!
//! This packages for the platform it runs on. Building an editor for
//! Windows means running this on Windows — the same answer Bevy's
//! release workflow gives, where a matrix of native runners replaces
//! cross-compilation. `metis` is vendored C, which is the part that
//! makes cross-compiling more than a target flag.

use std::path::{Path, PathBuf};

fn main() {
    let Some(dest) = std::env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage: package_editor <output-dir>");
        std::process::exit(2);
    };
    let engine_root = repo_root();

    let binary = engine_root
        .join("target/release")
        .join(editor_file_name())
        .exists()
        .then(|| engine_root.join("target/release").join(editor_file_name()))
        .or_else(|| {
            let debug = engine_root.join("target/debug").join(editor_file_name());
            debug.exists().then_some(debug)
        });
    let Some(binary) = binary else {
        eprintln!(
            "no editor binary in {}/target/{{release,debug}} — build it first:\n  \
             cargo build --release -p kooch_editor",
            engine_root.display(),
        );
        std::process::exit(1);
    };

    std::fs::create_dir_all(&dest).expect("create output dir");
    std::fs::copy(&binary, dest.join(editor_file_name())).expect("copy editor binary");

    // The engine source, in the layout `vendor_source` expects. Reuses
    // the same copy the editor performs, so what ships is exactly what a
    // project would have received from a development editor — including
    // the skipped `target/` directories.
    kooch::kooch_editor_core::engine_vendor::vendor_engine(&dest, &engine_root)
        .expect("copy engine source");

    // The editor's own assets, found at run time by walking up from the
    // executable looking for `assets/` (`bootstrap::engine_root`).
    copy_dir(&engine_root.join("assets"), &dest.join("assets"));

    println!("packaged editor into {}", dest.display());
    println!("  engine source: {}", dest.join("engine").display());
}

fn editor_file_name() -> &'static str {
    if cfg!(windows) {
        "kooch_editor.exe"
    } else {
        "kooch_editor"
    }
}

/// The engine repo, from this example's own manifest directory.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn copy_dir(from: &Path, to: &Path) {
    if !from.is_dir() {
        return;
    }
    std::fs::create_dir_all(to).expect("create dir");
    for entry in std::fs::read_dir(from).expect("read dir") {
        let entry = entry.expect("dir entry");
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        if src.is_dir() {
            copy_dir(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).expect("copy file");
        }
    }
}
