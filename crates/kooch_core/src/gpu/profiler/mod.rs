//! Per-pass GPU timings (#785), and the bridge that puts them in the
//! same flamegraph as the CPU scopes.
//!
//! # Why this exists
//!
//! `profiling::scope!` measures the CPU. On the OneXFly the frame is
//! GPU-bound at 96 % — the engine's CPU work is ~2 ms of a 72 ms frame —
//! so a CPU scope tree can say the frame is slow but never which pass
//! spends it. [`GpuScopes`] wraps [`wgpu_profiler::GpuProfiler`] to
//! answer that, and reports the results into puffin so the answer shows
//! up beside the CPU rows rather than in a second tool.
//!
//! # The API is `begin` / `end`, not a RAII guard
//!
//! `wgpu_profiler::Scope` borrows the encoder for the scope's whole
//! lifetime, which means the code being measured cannot use it.
//! [`GpuScopes::begin`] returns a query and gives the encoder straight
//! back, so a call site reads:
//!
//! ```ignore
//! let q = scopes.begin("shadows", &mut encoder);
//! self.record_shadows(&mut encoder, ...);
//! scopes.end(&mut encoder, q);
//! ```
//!
//! 🔴 **A scope must open and close on the same encoder.** Not a style
//! rule — the scope pushes a debug group, and wgpu rejects the encoder
//! outright at `finish()`: *"A debug group was not popped before the
//! encoder was finished"*. A profiled build would panic where an
//! unprofiled one runs. Verified, not assumed: it is what the first
//! version of these tests did.
//!
//! ⚠️ Scopes on an *encoder* need
//! [`wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS`]; scopes taken
//! from a pass descriptor do not. The encoder form is the one used
//! here because the passes are begun deep inside the render stage, and
//! `gpu/features.rs` already requests that feature — `MeshletGpuTimers`
//! has been writing encoder timestamps on this hardware for a while.
//!
//! # Per-frame order
//!
//! 1. `begin` / `end` around the work, on any encoder of the frame.
//! 2. [`GpuScopes::resolve`] on the **last** encoder before its submit.
//! 3. [`GpuScopes::end_frame`] after every submit of the frame.
//!
//! Nothing here blocks: results arrive a few frames late through a
//! non-blocking buffer map, and a frame whose results are not ready
//! yet simply reports nothing.

#[cfg(feature = "gpu-profiler")]
mod puffin_bridge;

// Two attributes rather than `cfg(all(test, feature = ...))`: the vendor filter matches the literal
// `#[cfg(test)]` line to know which module declarations point at files a shipped engine does not
// carry, and an `all(...)` reads to it as a module whose file went missing.
#[cfg(test)]
#[cfg(feature = "gpu-profiler")]
mod tests;

/// The frame's GPU timings, or nothing at all when the engine was built without `gpu-profiler`.
#[cfg(feature = "gpu-profiler")]
pub struct GpuScopes {
    profiler: wgpu_profiler::GpuProfiler,
    /// Nanoseconds per timestamp tick. Read once per frame from the
    /// queue rather than cached: some backends converge on the value
    /// while the application runs.
    timestamp_period: f32,
    /// A frame that ends with an open query is a bug in a call site,
    /// and it repeats every frame. Log it once.
    reported_error: bool,
    frame_ms: Option<f32>,
    /// The last finished frame's milliseconds per label, summed over every scope that carries it:
    /// what one shader cost across all the materials using it (#1159).
    totals: std::collections::HashMap<String, f32>,
}

/// Handle for one open GPU scope, closed by [`GpuScopes::end`].
#[cfg(feature = "gpu-profiler")]
pub type GpuQuery = Option<wgpu_profiler::GpuProfilerQuery>;

/// Handle for one open GPU scope. Carries nothing in a build without
/// `gpu-profiler`, so every call site compiles unchanged.
#[cfg(not(feature = "gpu-profiler"))]
pub type GpuQuery = ();

#[cfg(feature = "gpu-profiler")]
impl GpuScopes {
    /// Builds the profiler against `device`. Returns `None` when `wgpu-profiler` rejects the
    /// settings, which is the only failure it has: a device without timestamp support is not an
    /// error, it yields a profiler whose scopes measure nothing.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        let settings = wgpu_profiler::GpuProfilerSettings {
            enable_timer_queries: true,
            // Cheap, and it names the same regions inside RenderDoc as
            // the ones the flamegraph shows.
            enable_debug_groups: true,
            // Deep enough that a frame's results are ready before the
            // ring wraps, shallow enough to keep the lag readable.
            max_num_pending_frames: 3,
        };
        match wgpu_profiler::GpuProfiler::new(device, settings) {
            Ok(profiler) => Some(Self {
                profiler,
                timestamp_period: queue.get_timestamp_period(),
                reported_error: false,
                frame_ms: None,
                totals: std::collections::HashMap::new(),
            }),
            Err(err) => {
                tracing::warn!(?err, "GPU scopes disabled: profiler creation failed");
                None
            }
        }
    }

    /// What the GPU spent this frame, across EVERY scope — shadows included.
    pub fn frame_ms(&self) -> Option<f32> {
        self.frame_ms
    }

    /// Opens a top-level scope on `encoder` and hands the encoder back.
    #[must_use]
    pub fn begin(&self, label: impl Into<String>, encoder: &mut wgpu::CommandEncoder) -> GpuQuery {
        Some(self.profiler.begin_query(label, encoder))
    }

    /// Opens a scope nested inside `parent`.
    #[must_use]
    pub fn begin_child(
        &self,
        label: impl Into<String>,
        encoder: &mut wgpu::CommandEncoder,
        parent: &GpuQuery,
    ) -> GpuQuery {
        Some(
            self.profiler
                .begin_query(label, encoder)
                .with_parent(parent.as_ref()),
        )
    }

    /// Closes a scope opened by [`Self::begin`], on the same encoder.
    pub fn end(&self, encoder: &mut wgpu::CommandEncoder, query: GpuQuery) {
        if let Some(query) = query {
            self.profiler.end_query(encoder, query);
        }
    }

    /// Opens a scope inside a render or compute pass, nested in `parent`. Measures only with
    /// `TIMESTAMP_QUERY_INSIDE_PASSES`; without it the scope still names the region in a capture.
    #[must_use]
    pub fn begin_in<R: wgpu_profiler::ProfilerCommandRecorder>(
        &self,
        label: impl Into<String>,
        pass: &mut R,
        parent: Option<&GpuQuery>,
    ) -> GpuQuery {
        let parent = parent.and_then(Option::as_ref);
        Some(self.profiler.begin_query(label, pass).with_parent(parent))
    }

    /// Closes a scope opened by [`Self::begin_in`], on the same pass.
    pub fn end_in<R: wgpu_profiler::ProfilerCommandRecorder>(&self, pass: &mut R, query: GpuQuery) {
        if let Some(query) = query {
            self.profiler.end_query(pass, query);
        }
    }

    /// Milliseconds the scopes labelled `label` took in the last finished frame, summed.
    pub fn scope_ms(&self, label: &str) -> Option<f32> {
        self.totals.get(label).copied()
    }

    /// Every label of the last finished frame with its summed milliseconds.
    pub fn totals(&self) -> impl Iterator<Item = (&str, f32)> {
        self.totals.iter().map(|(label, ms)| (label.as_str(), *ms))
    }

    /// Copies this frame's timestamps out of their query sets. Must be
    /// recorded on the last encoder of the frame, before its submit,
    /// and after every scope is closed.
    pub fn resolve(&mut self, encoder: &mut wgpu::CommandEncoder) {
        self.profiler.resolve_queries(encoder);
    }

    /// Closes the frame and reports whatever older frame has finished to puffin. Call after the
    /// frame's last submit.
    pub fn end_frame(&mut self, queue: &wgpu::Queue) {
        self.timestamp_period = queue.get_timestamp_period();
        if let Err(err) = self.profiler.end_frame() {
            if !self.reported_error {
                self.reported_error = true;
                tracing::warn!(
                    ?err,
                    "GPU scope frame not closed — a begin() without its end(); \
                     GPU timings will be missing until it is fixed"
                );
            }
            return;
        }
        if let Some(results) = self.profiler.process_finished_frame(self.timestamp_period) {
            self.frame_ms = Some(gpu_span_ms(&results));
            self.totals.clear();
            sum_by_label(&results, &mut self.totals);
            puffin_bridge::report(&results);
        }
    }
}

/// The frame's GPU span, in milliseconds, from a finished batch.
#[cfg(feature = "gpu-profiler")]
pub(crate) fn gpu_span_ms(results: &[wgpu_profiler::GpuTimerQueryResult]) -> f32 {
    results
        .iter()
        .filter_map(|scope| scope.time.as_ref())
        .map(|span| (span.end - span.start) as f32 * 1000.0)
        .sum()
}

/// Adds each scope's milliseconds under its label, children included.
#[cfg(feature = "gpu-profiler")]
pub(crate) fn sum_by_label(
    results: &[wgpu_profiler::GpuTimerQueryResult],
    totals: &mut std::collections::HashMap<String, f32>,
) {
    for scope in results {
        if let Some(span) = &scope.time {
            let ms = (span.end - span.start) as f32 * 1000.0;
            *totals.entry(scope.label.clone()).or_default() += ms;
        }
        sum_by_label(&scope.nested_queries, totals);
    }
}

/// GPU scopes compiled out. Every method is present and does nothing,
/// so the render code has one shape rather than a `cfg` at each site.
#[cfg(not(feature = "gpu-profiler"))]
pub struct GpuScopes;

#[cfg(not(feature = "gpu-profiler"))]
impl GpuScopes {
    pub fn new(_device: &wgpu::Device, _queue: &wgpu::Queue) -> Option<Self> {
        None
    }

    #[must_use]
    pub fn begin(
        &self,
        _label: impl Into<String>,
        _encoder: &mut wgpu::CommandEncoder,
    ) -> GpuQuery {
    }

    #[must_use]
    pub fn begin_child(
        &self,
        _label: impl Into<String>,
        _encoder: &mut wgpu::CommandEncoder,
        _parent: &GpuQuery,
    ) -> GpuQuery {
    }

    pub fn end(&self, _encoder: &mut wgpu::CommandEncoder, _query: GpuQuery) {}

    #[must_use]
    pub fn begin_in<R>(
        &self,
        _label: impl Into<String>,
        _pass: &mut R,
        _parent: Option<&GpuQuery>,
    ) -> GpuQuery {
    }

    pub fn end_in<R>(&self, _pass: &mut R, _query: GpuQuery) {}

    pub fn scope_ms(&self, _label: &str) -> Option<f32> {
        None
    }

    pub fn totals(&self) -> impl Iterator<Item = (&str, f32)> {
        std::iter::empty()
    }

    pub fn resolve(&mut self, _encoder: &mut wgpu::CommandEncoder) {}

    pub fn end_frame(&mut self, _queue: &wgpu::Queue) {}

    pub fn frame_ms(&self) -> Option<f32> {
        None
    }
}
