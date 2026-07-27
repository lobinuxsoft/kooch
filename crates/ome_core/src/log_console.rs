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
}

impl LogEntry {
    /// Whether this line came from a spawned project rather than the
    /// editor.
    ///
    /// Matched on the prefix the editor already forwards child output with,
    /// so a project's log is distinguishable in the panel without a second
    /// capture path.
    pub fn is_from_project(&self) -> bool {
        self.message.starts_with("[game]")
    }
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

    fn push(&self, level: Level, target: String, message: String) {
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
    pub fn snapshot(&self) -> Vec<LogEntry> {
        self.inner
            .lock()
            .map(|inner| inner.entries.iter().cloned().collect())
            .unwrap_or_default()
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

/// A `tracing` layer that records into a [`LogBuffer`].
pub struct LogBufferLayer {
    buffer: LogBuffer,
}

impl<S: Subscriber> Layer<S> for LogBufferLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        self.buffer.push(
            *event.metadata().level(),
            event.metadata().target().to_owned(),
            visitor.finish(),
        );
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
}

impl MessageVisitor {
    fn finish(self) -> String {
        match self.fields.is_empty() {
            true => self.message,
            false => format!("{} {}", self.message, self.fields.join(" ")),
        }
    }
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        match field.name() {
            "message" => self.message = format!("{value:?}"),
            name => self.fields.push(format!("{name}={value:?}")),
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "message" => self.message = value.to_owned(),
            name => self.fields.push(format!("{name}={value}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(buffer: &LogBuffer, message: &str) {
        buffer.push(Level::INFO, "test".to_owned(), message.to_owned());
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

    /// The prefix the editor already forwards child output with is what
    /// tells a project's lines from the editor's.
    #[test]
    fn project_output_is_distinguishable() {
        let buffer = LogBuffer::new();
        entry(&buffer, "[game] the project said this");
        entry(&buffer, "the editor said this");

        let snapshot = buffer.snapshot();
        assert!(snapshot[0].is_from_project());
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
