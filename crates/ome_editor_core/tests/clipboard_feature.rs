//! The Copy buttons depend on a Cargo feature, and nothing else notices.
//!
//! `egui-winit`'s `clipboard` feature gates the whole body of
//! `Clipboard::set_text`. Turned off, it compiles to an empty function:
//! `handle_platform_output` hands it the text, it returns, and the system
//! clipboard is never touched. No error, no log line, no failing test —
//! the Console's Copy button simply does nothing, which is how it shipped.
//!
//! Nothing in the codebase can catch that. There is no symbol to call and
//! no result to check; the code is absent. So this reads the manifest
//! instead, which is where the mistake would be made.

use std::path::Path;

/// The workspace manifest, as text.
fn workspace_manifest() -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<name> sits two levels below the workspace root");
    std::fs::read_to_string(root.join("Cargo.toml")).expect("the workspace manifest is readable")
}

/// The `egui-winit` dependency declaration, from `[workspace.dependencies]`
/// to the end of its entry.
fn egui_winit_entry(manifest: &str) -> &str {
    let start = manifest
        .find("egui-winit")
        .expect("egui-winit is a workspace dependency");
    let rest = &manifest[start..];
    // Entries are separated by the next dependency line at column zero.
    match rest[1..].find("\negui-wgpu") {
        Some(end) => &rest[..end + 1],
        None => rest,
    }
}

/// Without this feature, copying is a no-op that reports nothing.
#[test]
fn egui_winit_keeps_its_clipboard_feature() {
    let manifest = workspace_manifest();
    let entry = egui_winit_entry(&manifest);

    assert!(
        entry.contains("\"clipboard\"") || !entry.contains("default-features = false"),
        "egui-winit has default features off and does not ask for `clipboard`, \
         so Copy will silently do nothing. Declaration was:\n{entry}",
    );
}

/// `smithay-clipboard` is the half that talks to a Wayland compositor —
/// `arboard` alone only reaches X11, so on a Wayland session copying would
/// still fail, just for a different reason.
#[test]
fn the_wayland_clipboard_backend_is_linked() {
    let output = std::process::Command::new(env!("CARGO"))
        .args(["tree", "-p", "ome_editor_core", "-i", "smithay-clipboard"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output();

    // A sandbox with no registry cannot answer, and that is not this test
    // failing. But "the package is not there" *is* — and cargo reports
    // both by failing, so the two have to be told apart. Treating the
    // first as the second is how a guard turns into decoration.
    let Ok(output) = output else {
        eprintln!("skipped: cargo tree could not be run");
        return;
    };
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("did not match any packages"),
        "smithay-clipboard is not in the dependency tree, so copying will \
         not reach a Wayland compositor",
    );
    if !output.status.success() {
        eprintln!("skipped: cargo tree failed: {}", stderr.trim());
        return;
    }

    assert!(
        String::from_utf8_lossy(&output.stdout).contains("smithay-clipboard"),
        "cargo tree reported success without naming smithay-clipboard",
    );
}
