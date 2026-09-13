//! Turns a finished `wgpu-profiler` frame into puffin scopes on a thread named `GPU`.

use puffin::{GlobalProfiler, NanoSecond, ScopeDetails, StreamInfo, ThreadInfo};
use wgpu_profiler::GpuTimerQueryResult;

#[cfg(test)]
mod tests;

/// Reports one finished GPU frame into the global profiler's current
/// frame. No-op when nothing in the batch carried a timestamp.
pub fn report(results: &[GpuTimerQueryResult]) {
    let Some(offset_ns) = clock_offset(results) else {
        return;
    };
    let mut profiler = GlobalProfiler::lock();
    let mut stream = StreamInfo::default();
    collect(&mut profiler, &mut stream, results, offset_ns, 0);
    if stream.num_scopes == 0 {
        return;
    }
    profiler.report_user_scopes(
        ThreadInfo {
            // Ordering only. The GPU track has no start of its own to
            // sort by, and pinning one would be a lie about the clock.
            start_time_ns: None,
            name: "GPU".to_owned(),
        },
        &stream.as_stream_into_ref(),
    );
}

/// Nanoseconds to add to every GPU timestamp so the batch ends at
/// puffin's "now". `None` when no scope in the tree has a time.
fn clock_offset(results: &[GpuTimerQueryResult]) -> Option<i64> {
    fn latest_end(results: &[GpuTimerQueryResult]) -> Option<i64> {
        results
            .iter()
            .flat_map(|query| {
                query
                    .time
                    .as_ref()
                    .map(|time| secs_to_ns(time.end))
                    .into_iter()
                    .chain(latest_end(&query.nested_queries))
            })
            .max()
    }
    latest_end(results).map(|end| puffin::now_ns() - end)
}

fn secs_to_ns(secs: f64) -> NanoSecond {
    (secs * 1e9) as NanoSecond
}

fn collect(
    profiler: &mut GlobalProfiler,
    stream: &mut StreamInfo,
    results: &[GpuTimerQueryResult],
    offset_ns: i64,
    depth: usize,
) {
    let details: Vec<_> = results
        .iter()
        .map(|query| ScopeDetails::from_scope_name(query.label.clone()))
        .collect();
    // Deduplicated by name inside puffin, so a scope seen every frame
    // is registered once and keeps its id.
    let ids = profiler.register_user_scopes(&details);
    for (query, id) in results.iter().zip(ids) {
        let Some(time) = &query.time else {
            continue;
        };
        let start = secs_to_ns(time.start) + offset_ns;
        let end = secs_to_ns(time.end) + offset_ns;

        stream.depth = stream.depth.max(depth);
        stream.num_scopes += 1;
        stream.range_ns.0 = stream.range_ns.0.min(start);
        stream.range_ns.1 = stream.range_ns.1.max(end);

        let (offset, _) = stream.stream.begin_scope(|| start, id, "");
        collect(
            profiler,
            stream,
            &query.nested_queries,
            offset_ns,
            depth + 1,
        );
        stream.stream.end_scope(offset, end);
    }
}
