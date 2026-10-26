//! The host dies with the editor that launched it (#647), however the editor went: the kernel
//! closes the editor's end of our stdin even on `SIGKILL`, and no destructor runs there.

use std::io::Read;

/// Set by the launcher when it holds our stdin open for as long as it lives.
pub const LIFELINE_ENV: &str = "KOOCH_LIFELINE";

/// Exits the process when stdin reaches end-of-file, if the launcher asked for it. A host run by
/// hand from a terminal is left alone.
pub(crate) fn watch() {
    if std::env::var_os(LIFELINE_ENV).is_none() {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("kooch-lifeline".into())
        .spawn(|| {
            let mut sink = [0u8; 64];
            let mut stdin = std::io::stdin();
            while matches!(stdin.read(&mut sink), Ok(n) if n > 0) {}
            tracing::info!("the editor is gone; the remote host exits with it");
            std::process::exit(0);
        });
    if let Err(e) = spawned {
        tracing::warn!("no lifeline to the editor, an orphan will outlive it: {e}");
    }
}
