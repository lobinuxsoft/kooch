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
