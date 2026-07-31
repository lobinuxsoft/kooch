//! A bounded log the editor can show.
//!
//! Everything the engine says goes to stdout, which is invisible unless it
//! was started from a terminal — and a project opened from the launcher was
//! not. So the same events go into a buffer the editor reads, in memory,
//! bounded, with no file involved.
//!
//! # One sink, not two
//!
//! This is a `tracing` layer beside the stdout one rather than a second
//! reporting channel. Anything already written with `tracing::info!` arrives
//! here for free — including the child process output the editor forwards as
//! `[game] ...`, which is how a running project's log reaches the panel
//! without any plumbing of its own.
//!
//! A parallel channel would drift: someone would log to one and not the
//! other, and the panel would disagree with the terminal about what
//! happened.
//!
//! # Why egui does not reach the panel
//!
//! Showing a line changes what the panel shows. For every other emitter
//! that is fine; for the UI library drawing the panel it is a loop. egui
//! complains when a widget keeps its rectangle but changes id — which is
//! exactly what a scrolling list does when a line arrives, since the row
//! at a given height is now a *different* row. The complaint is a log
//! line, it lands in the panel, the panel scrolls, and the next frame
//! complains again. Measured: one core, indefinitely, on an editor
//! nobody was touching (#656, #641).
//!
//! So `egui*` is muted **here only**. It still goes to stdout, where
//! reading it does not change it.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

/// How many lines are kept.
///
/// A session's worth of `info` at a few lines a second, and small enough
/// that the panel stays scrollable. Old lines are dropped rather than
/// growing without bound: a log that eats memory for the length of a
/// session is one that gets switched off.
const CAPACITY: usize = 2000;

/// One line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    /// Monotonic, so a consumer can tell what it has already drawn without
    /// comparing strings.
    pub seq: u64,
    pub level: Level,
    /// The emitting module, for the filter.
    pub target: String,
    pub message: String,
    /// Whether a hosted project said this rather than the editor.
    ///
    /// A field rather than a prefix on the message. It used to be sniffed
    /// from a `[game] ` prefix, which meant a project's line could not be
    /// filtered by its own level — everything forwarded arrived as an
    /// `info` from the forwarding module, whatever the project had
    /// actually logged.
    pub from_project: bool,
}

impl LogEntry {
    /// Whether a hosted project said this.
    pub fn is_from_project(&self) -> bool {
        self.from_project
    }
}

/// Removes ANSI escape sequences from a line.
///
/// The engine no longer colourises into a pipe, but a child process is
/// arbitrary: cargo, a script, anything a project spawns. Whoever renders
/// these has no terminal to interpret them, so an escape that survives is
/// drawn as glyphs — `\x1b[2m` arriving in the editor's Console as boxes is
/// how this was found.
///
/// Handles CSI sequences (`ESC [ ... letter`), which is what colour uses.
/// Anything else is left alone rather than guessed at.
pub fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        // ESC [ params letter — consume through the final byte.
        if chars.next() != Some('[') {
            continue;
        }
        for c in chars.by_ref() {
            if c.is_ascii_alphabetic() {
                break;
            }
        }
    }
    out
}

/// The shared buffer. Cheap to clone; every clone is the same log.
#[derive(Clone, Default)]
pub struct LogBuffer {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    entries: VecDeque<LogEntry>,
    next_seq: u64,
    /// Lines dropped to stay under [`CAPACITY`].
    ///
    /// Counted rather than silently forgotten: a panel that says "showing
    /// the last 2000 of 9400" is honest, and one that just shows 2000 looks
    /// like nothing else happened.
    dropped: u64,
}

impl LogBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a line a hosted project logged, with the level and target
    /// it logged it at.
    ///
    /// Not routed through `tracing`: doing that would re-stamp the line
    /// with the forwarder's level and module, which is what made a
    /// project's warnings indistinguishable from its chatter.
    pub fn push_project(
        &self,
        level: Level,
        target: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.record(level, target.into(), message.into(), true);
    }

    fn push(&self, level: Level, target: String, message: String) {
        self.record(level, target, message, false);
    }

    fn record(&self, level: Level, target: String, message: String, from_project: bool) {
        let Ok(mut inner) = self.inner.lock() else {
            // A poisoned lock means a panic is already being reported
            // somewhere; losing a log line is not the thing to escalate.
            return;
        };
        let seq = inner.next_seq;
        inner.next_seq += 1;
        inner.entries.push_back(LogEntry {
            seq,
            level,
            target,
            message,
            from_project,
        });
        while inner.entries.len() > CAPACITY {
            inner.entries.pop_front();
            inner.dropped += 1;
        }
    }

    /// Copies the entries out, oldest first.
    ///
    /// A copy rather than a borrow: the panel draws inside the egui pass
    /// while systems on other threads are still logging, and holding the
    /// lock across a frame would let a log line block a repaint.
    ///
    /// **Every call clones every line.** A viewer redrawing at 60 fps wants
    /// [`entries_after`](Self::entries_after) instead — that is what
    /// [`seq`](LogEntry::seq) is for, and copying two thousand lines to
    /// find out that none of them changed is what it exists to avoid.
    pub fn snapshot(&self) -> Vec<LogEntry> {
        self.inner
            .lock()
            .map(|inner| inner.entries.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// The lines newer than `seq`, oldest first.
    ///
    /// What a viewer holding its own copy needs: on a quiet frame this
    /// clones nothing, and on a busy one it clones what actually arrived.
    pub fn entries_after(&self, seq: u64) -> Vec<LogEntry> {
        self.inner
            .lock()
            .map(|inner| {
                inner
                    .entries
                    .iter()
                    // Newest last, so the tail is a suffix — take it from
                    // the back and stop at the first line already seen.
                    .rev()
                    .take_while(|entry| entry.seq > seq)
                    .cloned()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The sequence numbers still held, as `(oldest, newest)`. `None` when
    /// the log is empty.
    ///
    /// A viewer compares these against its own copy: a newest that moved
    /// means there is something to fetch, and an oldest that moved past
    /// what it holds means lines were dropped — or the log was cleared,
    /// which is the same discovery.
    pub fn seq_range(&self) -> Option<(u64, u64)> {
        self.inner
            .lock()
            .ok()
            .and_then(|inner| Some((inner.entries.front()?.seq, inner.entries.back()?.seq)))
    }

    /// How many lines were dropped to stay bounded.
    pub fn dropped(&self) -> u64 {
        self.inner.lock().map(|inner| inner.dropped).unwrap_or(0)
    }

    /// Number of lines held.
    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .map(|inner| inner.entries.len())
            .unwrap_or(0)
    }

    /// `true` when nothing has been logged.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Forgets everything, including the dropped count.
    ///
    /// What a Clear button does: after it, "showing the last N of N" is
    /// true again.
    pub fn clear(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.entries.clear();
            inner.dropped = 0;
        }
    }

    /// The layer that fills this buffer.
    pub fn layer(&self) -> LogBufferLayer {
        LogBufferLayer {
            buffer: self.clone(),
        }
    }
}

/// Target prefixes that never reach the panel — see the module docs for
/// why the UI library is not allowed to log into the UI.
const MUTED_TARGETS: &[&str] = &["egui"];

/// A `tracing` layer that records into a [`LogBuffer`].
pub struct LogBufferLayer {
    buffer: LogBuffer,
}

impl<S: Subscriber> Layer<S> for LogBufferLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        // A crate logging through the `log` crate arrives via the tracing
        // bridge, whose metadata target is the literal `"log"` for every
        // one of them — the real one travels as a `log.target` field. Use
        // it when it is there, or the panel's own filter sees a single
        // undifferentiated target and every mute would have to be by
        // message text.
        let target = visitor
            .log_target
            .take()
            .unwrap_or_else(|| event.metadata().target().to_owned());

        if MUTED_TARGETS
            .iter()
            .any(|muted| target == *muted || target.starts_with(&format!("{muted}::")))
        {
            return;
        }

        self.buffer
            .push(*event.metadata().level(), target, visitor.finish());
    }
}

/// Renders an event's fields into one line.
///
/// The `message` field is the line; the rest are appended as `key=value`,
/// which is what the stdout formatter does and what makes the two agree.
#[derive(Default)]
struct MessageVisitor {
    message: String,
    fields: Vec<String>,
    /// The emitter's real target, when the event came through the `log`
    /// bridge rather than from `tracing` directly.
    log_target: Option<String>,
}

impl MessageVisitor {
    fn finish(self) -> String {
        match self.fields.is_empty() {
            true => self.message,
            false => format!("{} {}", self.message, self.fields.join(" ")),
        }
    }
}

impl MessageVisitor {
    /// Files a non-message field, keeping the bridge's bookkeeping out of
    /// the line.
    ///
    /// `log.target` becomes the entry's target; `log.module_path`,
    /// `log.file` and `log.line` are dropped. They say the same thing as
    /// the target and made every bridged line three times its length —
    /// which, in a panel, is three times the scrolling.
    fn field(&mut self, name: &str, value: String) {
        match name {
            // Recorded as a string by the bridge, but a `Debug` capture
            // would arrive quoted and no prefix would ever match.
            "log.target" => self.log_target = Some(value.trim_matches('"').to_owned()),
            "log.module_path" | "log.file" | "log.line" => {}
            _ => self.fields.push(format!("{name}={value}")),
        }
    }
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        match field.name() {
            "message" => self.message = format!("{value:?}"),
            name => self.field(name, format!("{value:?}")),
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "message" => self.message = value.to_owned(),
            name => self.field(name, value.to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(buffer: &LogBuffer, message: &str) {
        buffer.push(Level::INFO, "test".to_owned(), message.to_owned());
    }

    /// Runs `emit` with only this buffer's layer installed, and returns
    /// what reached the buffer.
    ///
    /// A local subscriber rather than the global one: these run in
    /// parallel with every other test in the crate, and a global default
    /// can only be set once per process.
    fn through_the_layer(emit: impl FnOnce()) -> Vec<LogEntry> {
        use tracing_subscriber::layer::SubscriberExt as _;

        let buffer = LogBuffer::new();
        let subscriber = tracing_subscriber::registry().with(buffer.layer());
        tracing::subscriber::with_default(subscriber, emit);
        buffer.snapshot()
    }

    /// egui's own complaints must not land in the panel egui is drawing.
    ///
    /// The complaint is about a widget whose rect stayed and whose id
    /// changed — which is what a scrolling list does when a line arrives.
    /// Showing it adds a line, which scrolls the list, which produces the
    /// next complaint: one core, forever (#656).
    #[test]
    fn egui_never_reaches_the_panel() {
        let entries = through_the_layer(|| {
            tracing::warn!(target: "egui", "changed id between passes");
            tracing::warn!(target: "egui::context", "changed id between passes");
            tracing::info!(target: "kooch_render", "uploaded meshlet asset");
        });

        let targets: Vec<_> = entries.iter().map(|e| e.target.as_str()).collect();
        assert_eq!(
            targets,
            vec!["kooch_render"],
            "an egui line reached the buffer",
        );
    }

    /// A near-miss must still get through: muting is by target, not by
    /// "contains egui somewhere".
    #[test]
    fn a_crate_merely_named_after_egui_still_logs() {
        let entries = through_the_layer(|| {
            tracing::warn!(target: "eguide", "not egui");
        });
        assert_eq!(entries.len(), 1, "an unrelated crate was muted");
    }

    /// Everything arriving through the `log` bridge shares the metadata
    /// target `"log"`. Without reading `log.target` the panel's filter
    /// sees one undifferentiated emitter — and the mute above would never
    /// match, because egui logs through exactly that bridge.
    #[test]
    fn the_bridge_target_is_the_one_recorded() {
        let entries = through_the_layer(|| {
            tracing::warn!(log.target = "wgpu_core", message = "surface lost");
        });

        assert_eq!(entries[0].target, "wgpu_core");
        assert_eq!(
            entries[0].message, "surface lost",
            "the bridge's bookkeeping leaked into the line",
        );
    }

    /// Same bridge, muted crate: the mute has to survive the indirection
    /// or it does nothing at all for the case it exists for.
    #[test]
    fn egui_through_the_bridge_is_muted_too() {
        let entries = through_the_layer(|| {
            tracing::warn!(
                log.target = "egui::context",
                log.file = "context.rs",
                log.line = 4254,
                message = "Widget rect changed id between passes",
            );
        });
        assert!(entries.is_empty(), "egui got in through the log bridge");
    }

    #[test]
    fn entries_come_back_oldest_first() {
        let buffer = LogBuffer::new();
        entry(&buffer, "first");
        entry(&buffer, "second");

        let messages: Vec<_> = buffer.snapshot().into_iter().map(|e| e.message).collect();
        assert_eq!(messages, vec!["first", "second"]);
    }

    /// The sequence is what lets a consumer tell new lines from redrawn
    /// ones without comparing text.
    #[test]
    fn sequence_numbers_are_monotonic_and_survive_dropping() {
        let buffer = LogBuffer::new();
        for i in 0..CAPACITY + 10 {
            entry(&buffer, &format!("line {i}"));
        }

        let snapshot = buffer.snapshot();
        assert_eq!(snapshot.len(), CAPACITY, "the buffer grew past its cap");
        assert_eq!(snapshot[0].seq, 10, "the oldest kept line is not the 11th");
        assert!(
            snapshot.windows(2).all(|w| w[1].seq == w[0].seq + 1),
            "sequence numbers are not contiguous",
        );
    }

    /// A panel showing 2000 lines out of 9400 has to be able to say so.
    /// Silently showing 2000 looks like nothing else happened.
    #[test]
    fn dropped_lines_are_counted() {
        let buffer = LogBuffer::new();
        for i in 0..CAPACITY + 5 {
            entry(&buffer, &format!("line {i}"));
        }
        assert_eq!(buffer.dropped(), 5);
    }

    #[test]
    fn clearing_resets_the_dropped_count_too() {
        let buffer = LogBuffer::new();
        for i in 0..CAPACITY + 5 {
            entry(&buffer, &format!("line {i}"));
        }
        buffer.clear();

        assert!(buffer.is_empty());
        assert_eq!(buffer.dropped(), 0, "cleared but still claiming losses");
    }

    /// Every clone is the same log — the layer holds one and the editor
    /// another, and they have to agree.
    #[test]
    fn clones_share_one_buffer() {
        let buffer = LogBuffer::new();
        let other = buffer.clone();
        entry(&buffer, "written through the first handle");

        assert_eq!(other.len(), 1);
    }

    /// A colourised child line has to arrive readable: the renderer has no
    /// terminal, so a surviving escape is drawn as glyphs.
    #[test]
    fn ansi_escapes_are_stripped() {
        let coloured = "\u{1b}[2m2026-07-27\u{1b}[0m \u{1b}[32m INFO\u{1b}[0m a sensor was entered";
        assert_eq!(
            strip_ansi(coloured),
            "2026-07-27  INFO a sensor was entered"
        );
    }

    /// A line with nothing to strip must come back untouched, including
    /// text that merely looks like a sequence.
    #[test]
    fn plain_text_survives_stripping() {
        assert_eq!(
            strip_ansi("a sensor was entered a=8"),
            "a sensor was entered a=8"
        );
        assert_eq!(
            strip_ansi("half_extents [2.0, 1.0]"),
            "half_extents [2.0, 1.0]"
        );
    }

    /// A truncated escape at the end of a line must not eat the line or
    /// loop — child output arrives in chunks and can be cut anywhere.
    #[test]
    fn a_truncated_escape_terminates() {
        assert_eq!(strip_ansi("text \u{1b}["), "text ");
        assert_eq!(strip_ansi("text \u{1b}"), "text ");
    }

    /// A project's line keeps its own level and target, so filtering by
    /// severity works on it. Sniffing a prefix could not do that: every
    /// forwarded line arrived as an `info` from the forwarding module.
    #[test]
    fn a_projects_line_keeps_its_own_level_and_target() {
        let buffer = LogBuffer::new();
        buffer.push_project(Level::WARN, "kooch_physics", "a joint is waiting");
        entry(&buffer, "the editor said this");

        let snapshot = buffer.snapshot();
        assert!(snapshot[0].is_from_project());
        assert_eq!(snapshot[0].level, Level::WARN);
        assert_eq!(snapshot[0].target, "kooch_physics");
        assert!(!snapshot[1].is_from_project());
    }

    /// Fields have to survive: half the engine's useful lines are
    /// `warn!(entity = 3, "...")` and a panel showing only the message
    /// drops the part that identifies what went wrong.
    #[test]
    fn structured_fields_reach_the_line() {
        let buffer = LogBuffer::new();
        let layer = buffer.layer();
        let subscriber = tracing_subscriber::registry().with(layer);

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(entity = 7, "a joint broke under load");
        });

        let line = &buffer.snapshot()[0].message;
        assert!(line.contains("a joint broke under load"), "{line}");
        assert!(line.contains("entity=7"), "the field was dropped: {line}");
    }

    /// And the level, or a filter has nothing to filter on.
    #[test]
    fn the_level_is_recorded() {
        let buffer = LogBuffer::new();
        let subscriber = tracing_subscriber::registry().with(buffer.layer());

        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!("something worth noticing");
        });

        assert_eq!(buffer.snapshot()[0].level, Level::WARN);
    }
}

#[cfg(test)]
use tracing_subscriber::prelude::*;
