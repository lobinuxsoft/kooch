//! Reads a `.puffin` capture and prints where the time went (#785).
//!
//! The panel answers this interactively; this answers it in a terminal,
//! which is what a capture from the handheld will need — and what makes
//! "the profiler slows the editor down" a measurement rather than an
//! impression.
//!
//! Two views, for the two questions:
//!
//! - **The tree** — who is inside whom, averaged over every frame in the
//!   capture. This is the one to read first: a flat number cannot say
//!   whether 40 ms of sky is inside the frame or beside it.
//! - **The flat ranking** — every scope by total cost, wherever it ran.
//!   Useful for a scope that appears in several places at once.
//!
//! ⚠️ The panel's Table view is the flat one, and it opens sorted by
//! call count. That is why a capture of a 70 ms frame looks like it is
//! made of `BindGroup::drop`: 56 calls of 0.1 µs sort above one pass of
//! 40 ms. Sort by total self time, or read the Flamegraph — the panel's
//! own tree.
//!
//! `--split` answers the question the averages cannot: **what is
//! different about the slow frames?** A capture where the camera moves
//! is two populations, and a mean over both describes neither. It also
//! prints the frame against the GPU work that produced it — the ratio
//! between them is what separates "the GPU is the wall" from "something
//! is making us wait for it twice" (#814).
//!
//! cargo run -p kooch_editor_core --features profiling --example read_capture -- <file.puffin> [--slowest] [--split]

use std::collections::HashMap;

/// One node of the merged call tree: a scope, wherever it appeared at
/// this position under this parent, summed across the capture.
#[derive(Default)]
struct Node {
    /// Nanoseconds spent inside this scope, children included.
    total_ns: i64,
    /// How many times it was entered across the whole capture.
    calls: usize,
    children: HashMap<String, Node>,
}

impl Node {
    /// Nanoseconds this scope spent that no child accounts for.
    fn self_ns(&self) -> i64 {
        self.total_ns - self.children.values().map(|c| c.total_ns).sum::<i64>()
    }
}

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("pass a .puffin path");
    let mut file = std::io::BufReader::new(std::fs::File::open(&path)?);
    let view = puffin::FrameView::read(&mut file)?;

    let frames: Vec<_> = view.all_uniq().cloned().collect();
    println!("{} frames", frames.len());

    // The slowest frame, on its own. An average hides a stall by
    // definition: one frame of 700 ms across a thousand moves the mean
    // by 0.7 ms and the median by nothing at all.
    if std::env::args().any(|a| a == "--slowest")
        && let Some(worst) = frames.iter().max_by_key(|f| f.duration_ns())
    {
        let unpacked = worst.unpacked()?;
        println!(
            "\n── slowest frame: {:.2} ms ──",
            unpacked.duration_ns() as f64 / 1e6
        );
        let mut root = Node::default();
        let mut ignored = HashMap::new();
        for (thread, stream_info) in unpacked.thread_streams.iter() {
            let stream = &stream_info.stream;
            let node = root.children.entry(thread.name.clone()).or_default();
            if let Ok(scopes) = puffin::Reader::from_start(stream).read_top_scopes() {
                absorb(&view, stream, &scopes, node, &mut ignored);
            }
        }
        print_children(&root, 1.0, 0);
    }

    let mut durations: Vec<f64> = Vec::new();
    // Total nanoseconds per scope name, and how many times it ran.
    let mut totals: HashMap<String, (i64, usize)> = HashMap::new();
    // The same, kept per thread and per position in the call tree.
    let mut roots: HashMap<String, Node> = HashMap::new();

    for frame in &frames {
        let unpacked = frame.unpacked()?;
        durations.push(unpacked.duration_ns() as f64 / 1e6);
        for (thread, stream_info) in unpacked.thread_streams.iter() {
            let stream = &stream_info.stream;
            // A thread of its own in the tree: the `GPU` rows are not
            // nested under the CPU scope that recorded them, and
            // printing them as if they were would be a lie about which
            // one contains which.
            let root = roots.entry(thread.name.clone()).or_default();
            let Ok(scopes) = puffin::Reader::from_start(stream).read_top_scopes() else {
                continue;
            };
            absorb(&view, stream, &scopes, root, &mut totals);
            root.calls = root.calls.max(1);
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

    let frame_count = frames.len().max(1) as f64;

    println!("\n── tree (ms/frame, self time in brackets) ──");
    let mut threads: Vec<_> = roots.into_iter().collect();
    // Heaviest thread first: on a GPU-bound frame that is the one to
    // read, and it is not always the main thread.
    threads.sort_by_key(|(_, node)| -node.children.values().map(|c| c.total_ns).sum::<i64>());
    for (thread, node) in threads {
        println!("\n[{thread}]");
        print_children(&node, frame_count, 1);
    }

    if std::env::args().any(|a| a == "--split") {
        split(&view, &frames)?;
    }

    let mut ranked: Vec<_> = totals.into_iter().collect();
    ranked.sort_by_key(|(_, (ns, _))| -*ns);
    println!("\n── flat, by total cost ──");
    println!("{:<44} {:>9} {:>9}", "scope", "ms/frame", "calls/f");
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

/// One frame reduced to what a comparison needs.
struct Sample {
    ms: f64,
    /// Everything the `GPU` thread reported for this frame.
    gpu_ms: f64,
    /// Self time per scope *path*, so a `blit` on the GPU thread is
    /// never added to a `blit` on the CPU.
    self_ms: HashMap<String, f64>,
}

/// Compares the fastest quarter of frames against the slowest.
///
/// # Why a split and not an average
///
/// A capture taken while the camera moves holds two populations, and a
/// mean over both describes neither. The interesting number is not what
/// a frame costs — it is what grows when the frame gets slow, which is a
/// subtraction the eye cannot do across two trees.
fn split(
    view: &puffin::FrameView,
    frames: &[std::sync::Arc<puffin::FrameData>],
) -> anyhow::Result<()> {
    let mut samples: Vec<Sample> = Vec::with_capacity(frames.len());
    for frame in frames {
        let unpacked = frame.unpacked()?;
        let mut self_ms = HashMap::new();
        let mut gpu_ms = 0.0;
        for (thread, stream_info) in unpacked.thread_streams.iter() {
            let stream = &stream_info.stream;
            let mut node = Node::default();
            let mut ignored = HashMap::new();
            let Ok(scopes) = puffin::Reader::from_start(stream).read_top_scopes() else {
                continue;
            };
            absorb(view, stream, &scopes, &mut node, &mut ignored);
            if thread.name == "GPU" {
                gpu_ms = node.children.values().map(|c| c.total_ns).sum::<i64>() as f64 / 1e6;
            }
            flatten(&node, &thread.name, &mut self_ms);
        }
        samples.push(Sample {
            ms: unpacked.duration_ns() as f64 / 1e6,
            gpu_ms,
            self_ms,
        });
    }
    samples.sort_by(|a, b| a.ms.total_cmp(&b.ms));

    let quarter = (samples.len() / 4).max(1);
    let fast = &samples[..quarter];
    let slow = &samples[samples.len() - quarter..];
    let mean = |group: &[Sample], path: &str| -> f64 {
        group
            .iter()
            .filter_map(|s| s.self_ms.get(path))
            .sum::<f64>()
            / group.len() as f64
    };

    let paths: std::collections::BTreeSet<&String> =
        samples.iter().flat_map(|s| s.self_ms.keys()).collect();
    let mut rows: Vec<(f64, String)> = Vec::new();
    for path in paths {
        let (f, s) = (mean(fast, path), mean(slow, path));
        if (s - f).abs() < 0.05 {
            continue;
        }
        rows.push((s - f, format!("{f:>9.3} {s:>9.3} {:>+9.3}   {path}", s - f)));
    }
    rows.sort_by(|a, b| b.0.total_cmp(&a.0));

    let group_mean =
        |group: &[Sample]| group.iter().map(|s| s.ms).sum::<f64>() / group.len() as f64;
    println!(
        "\n── fastest {quarter} frames ({:.2} ms) vs slowest {quarter} ({:.2} ms), by self time ──",
        group_mean(fast),
        group_mean(slow),
    );
    println!("{:>9} {:>9} {:>9}   scope path", "fast", "slow", "delta");
    for (_, row) in rows.iter().take(20) {
        println!("{row}");
    }

    // 🔴 The ratio is the finding, not the frame time. A frame that
    // costs what its GPU work costs is GPU-bound and honest; one that
    // costs twice it is waiting for the GPU twice, which is a swapchain
    // problem wearing a shading problem's clothes (#814).
    println!("\n── frame against the GPU work that produced it ──");
    println!(
        "{:>9} {:>9} {:>10}   decile",
        "frame ms", "GPU ms", "frame/GPU"
    );
    for decile in 0..10 {
        let lo = samples.len() * decile / 10;
        let hi = (samples.len() * (decile + 1) / 10).max(lo + 1);
        let group = &samples[lo..hi];
        let frame = group_mean(group);
        let gpu = group.iter().map(|s| s.gpu_ms).sum::<f64>() / group.len() as f64;
        println!(
            "{frame:>9.2} {gpu:>9.2} {:>10.2}   p{}",
            frame / gpu.max(0.001),
            decile * 10
        );
    }
    Ok(())
}

/// Flattens a tree into self time keyed by full path.
fn flatten(node: &Node, path: &str, out: &mut HashMap<String, f64>) {
    for (name, child) in &node.children {
        let path = format!("{path} > {name}");
        *out.entry(path.clone()).or_default() += child.self_ns() as f64 / 1e6;
        flatten(child, &path, out);
    }
}

/// Walks one scope level, merging it into `parent` and into the flat
/// totals, then recurses into each scope's children.
fn absorb(
    view: &puffin::FrameView,
    stream: &puffin::Stream,
    scopes: &[puffin::Scope<'_>],
    parent: &mut Node,
    totals: &mut HashMap<String, (i64, usize)>,
) {
    for scope in scopes {
        let name = view
            .scope_collection()
            .fetch_by_id(&scope.id)
            .map(|d| d.name().to_string())
            .unwrap_or_else(|| format!("scope#{:?}", scope.id));

        let flat = totals.entry(name.clone()).or_insert((0, 0));
        flat.0 += scope.record.duration_ns;
        flat.1 += 1;

        let node = parent.children.entry(name).or_default();
        node.total_ns += scope.record.duration_ns;
        node.calls += 1;

        let Ok(reader) = puffin::Reader::with_offset(stream, scope.child_begin_position) else {
            continue;
        };
        // The reader stops at this scope's own SCOPE_END, so iterating
        // it yields exactly the direct children.
        let children: Vec<_> = reader.flatten().collect();
        absorb(view, stream, &children, node, totals);
    }
}

/// Prints a node's children, heaviest first.
///
/// Anything under 1 % of a frame is folded into one line: a tree with
/// four hundred rows of 0.001 ms is the same problem as the flat table
/// sorted by call count.
fn print_children(node: &Node, frames: f64, depth: usize) {
    let mut children: Vec<_> = node.children.values().zip(node.children.keys()).collect();
    children.sort_by_key(|(child, _)| -child.total_ns);

    let frame_ns_1pct = children
        .iter()
        .map(|(c, _)| c.total_ns)
        .max()
        .unwrap_or(0)
        .max(1)
        / 100;

    let mut folded = (0i64, 0usize);
    for (child, name) in &children {
        if child.total_ns < frame_ns_1pct && depth > 1 {
            folded.0 += child.total_ns;
            folded.1 += 1;
            continue;
        }
        let indent = "  ".repeat(depth);
        println!(
            "{indent}{:<40} {:>8.3} [{:>7.3}] ×{:.1}",
            name,
            child.total_ns as f64 / 1e6 / frames,
            child.self_ns() as f64 / 1e6 / frames,
            child.calls as f64 / frames,
        );
        print_children(child, frames, depth + 1);
    }
    if folded.1 > 0 {
        let indent = "  ".repeat(depth);
        println!(
            "{indent}{:<40} {:>8.3}",
            format!("… {} smaller scopes", folded.1),
            folded.0 as f64 / 1e6 / frames,
        );
    }
}
