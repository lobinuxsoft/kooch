//! Reads a `.puffin` capture and prints where the time went (#785).
//!
//! The panel answers this interactively; this answers it in a terminal,
//! which is what a capture from the handheld will need — and what makes
//! "the profiler slows the editor down" a measurement rather than an
//! impression.
//!
//! cargo run -p kooch_editor_core --features profiling --example read_capture -- <file.puffin>

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("pass a .puffin path");
    let mut file = std::io::BufReader::new(std::fs::File::open(&path)?);
    let view = puffin::FrameView::read(&mut file)?;

    let frames: Vec<_> = view.all_uniq().cloned().collect();
    println!("{} frames", frames.len());

    let mut durations: Vec<f64> = Vec::new();
    // Total nanoseconds per scope name, and how many times it ran.
    let mut totals: std::collections::HashMap<String, (i64, usize)> =
        std::collections::HashMap::new();

    for frame in &frames {
        let unpacked = frame.unpacked()?;
        durations.push(unpacked.duration_ns() as f64 / 1e6);
        for (_, stream) in unpacked.thread_streams.iter() {
            let reader = puffin::Reader::from_start(&stream.stream);
            for scope in reader {
                let Ok(scope) = scope else { break };
                let name = view
                    .scope_collection()
                    .fetch_by_id(&scope.id)
                    .map(|d| d.name().to_string())
                    .unwrap_or_else(|| format!("scope#{:?}", scope.id));
                let entry = totals.entry(name).or_insert((0, 0));
                entry.0 += scope.record.duration_ns;
                entry.1 += 1;
            }
        }
    }

    durations.sort_by(|a, b| a.total_cmp(b));
    let median = durations.get(durations.len() / 2).copied().unwrap_or(0.0);
    let p99 = durations
        .get(durations.len() * 99 / 100)
        .copied()
        .unwrap_or(0.0);
    println!(
        "frame ms: median {median:.2}  p99 {p99:.2}  max {:.2}",
        durations.last().copied().unwrap_or(0.0)
    );

    let mut ranked: Vec<_> = totals.into_iter().collect();
    ranked.sort_by_key(|(_, (ns, _))| -*ns);
    let frame_count = frames.len().max(1) as f64;
    println!("\n{:<44} {:>9} {:>9}", "scope", "ms/frame", "calls/f");
    for (name, (ns, calls)) in ranked.into_iter().take(25) {
        println!(
            "{:<44} {:>9.3} {:>9.1}",
            name,
            ns as f64 / 1e6 / frame_count,
            calls as f64 / frame_count,
        );
    }
    Ok(())
}
