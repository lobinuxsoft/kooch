//! A log the game leaves on disk, for when nobody is watching it run.

use std::fs::File;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// What the file is called, beside the executable.
const LOG_NAME: &str = "kooch.log";

/// The previous run's log, kept under this name.
const PREVIOUS_NAME: &str = "kooch.log.prev";

/// A file several writers share, since the tracing layer and the panic
/// hook both write to it.
#[derive(Clone)]
pub struct SharedLog(Arc<Mutex<File>>);

impl Write for SharedLog {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.0.lock() {
            Ok(mut file) => file.write(buf),
            // A poisoned lock means a writer panicked mid-line. Losing
            // the line is better than panicking inside the logger, which
            // would be a panic while reporting a panic.
            Err(_) => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.0.lock() {
            Ok(mut file) => file.flush(),
            Err(_) => Ok(()),
        }
    }
}

/// Opens this run's log, rotating the last one out of the way.
pub fn open_log() -> Option<(SharedLog, PathBuf)> {
    for dir in candidate_dirs() {
        let path = dir.join(LOG_NAME);
        // Best effort: a missing previous log is the normal first run.
        let _ = std::fs::rename(&path, dir.join(PREVIOUS_NAME));
        if let Ok(file) = File::create(&path) {
            return Some((SharedLog(Arc::new(Mutex::new(file))), path));
        }
    }
    None
}

/// Where to try putting the log, best first.
fn candidate_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        dirs.push(dir.to_path_buf());
    }
    // An installed game may sit somewhere its user cannot write.
    dirs.push(std::env::temp_dir());
    dirs
}

/// Sends panics to the log as well as to stderr.
pub fn log_panics(log: SharedLog) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mut log = log.clone();
        // Written as one `write_all` rather than several `writeln!`s:
        // two threads panicking at once would otherwise interleave into
        // a line that describes neither.
        let report = format!(
            "\n=== panic ===\n{}\nlocation: {}\n{}=== end panic ===\n",
            info.payload_as_str().unwrap_or("<no message>"),
            info.location()
                .map(|l| l.to_string())
                .unwrap_or_else(|| "<unknown>".to_owned()),
            backtrace(),
        );
        let _ = log.write_all(report.as_bytes());
        // Flushed here, not left to the layer: the process is about to
        // stop, and an unflushed buffer is a log file that ends just
        // before the interesting part.
        let _ = log.flush();
        previous(info);
    }));
}

/// The backtrace, when the environment asked for one.
fn backtrace() -> String {
    match std::env::var("RUST_BACKTRACE").is_ok_and(|v| v != "0") {
        true => format!("{}\n", std::backtrace::Backtrace::force_capture()),
        false => String::new(),
    }
}

#[cfg(test)]
mod log_file_tests;
