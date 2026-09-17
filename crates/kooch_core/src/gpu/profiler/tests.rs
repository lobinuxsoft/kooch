//! End-to-end: a scope opened on an encoder has to come back out as a puffin scope on the `GPU`
//! thread.

use std::sync::{Arc, Mutex};

use super::*;

/// `puffin` state is global, and so is the frame these tests close.
static PUFFIN: Mutex<()> = Mutex::new(());

/// A device with the timestamp features, or `None` on an adapter that
/// cannot measure — CI runners without a GPU included.
fn timestamp_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::METAL,
        flags: wgpu::InstanceFlags::default(),
        backend_options: wgpu::BackendOptions::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        display: None,
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .ok()?;
    // The encoder form of a scope needs INSIDE_ENCODERS specifically.
    // Asking for a feature the adapter lacks fails device creation, so
    // check before requesting rather than after.
    let wanted = wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
    if !adapter.features().contains(wanted) {
        return None;
    }
    // Inside passes too, where the adapter has it: the compute shading path times each material there.
    let wanted = wanted | (adapter.features() & wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES);
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("gpu_scopes_test_device"),
        required_features: wanted,
        ..Default::default()
    }))
    .ok()
}

/// Some GPU work with a duration that is not zero, so a timestamp pair
/// has something to bracket.
fn busy_copy(device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder) {
    let bytes = 4 << 20;
    let src = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("src"),
        size: bytes,
        usage: wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let dst = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("dst"),
        size: bytes,
        usage: wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_buffer_to_buffer(&src, 0, &dst, 0, bytes);
}

#[test]
fn a_scope_reaches_puffin() {
    let _guard = PUFFIN.lock().unwrap_or_else(|e| e.into_inner());
    let Some((device, queue)) = timestamp_device() else {
        eprintln!("skipped: no adapter with timestamp queries");
        return;
    };
    let mut scopes = GpuScopes::new(&device, &queue).expect("profiler settings are valid");

    let frames: Arc<Mutex<Vec<Arc<puffin::FrameData>>>> = Arc::default();
    let sink = {
        let frames = Arc::clone(&frames);
        puffin::GlobalProfiler::lock().add_sink(Box::new(move |frame| {
            frames.lock().unwrap().push(frame);
        }))
    };
    puffin::set_scopes_on(true);

    // Several frames: the results of one come back a submit or two
    // later, so a single frame proves nothing either way.
    for _ in 0..8 {
        let mut encoder = device.create_command_encoder(&Default::default());
        let query = scopes.begin("test pass", &mut encoder);
        busy_copy(&device, &mut encoder);
        scopes.end(&mut encoder, query);
        scopes.resolve(&mut encoder);
        queue.submit(Some(encoder.finish()));
        // Only a test may do this. It is what lets the buffer mapping
        // complete inside the loop instead of frames later.
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });
        scopes.end_frame(&queue);
        puffin::GlobalProfiler::lock().new_frame();
    }

    puffin::GlobalProfiler::lock().remove_sink(sink);
    let frames = frames.lock().unwrap();
    let gpu_scopes: usize = frames
        .iter()
        .filter_map(|frame| frame.unpacked().ok())
        .flat_map(|frame| {
            frame
                .thread_streams
                .iter()
                .filter(|(info, _)| info.name == "GPU")
                .map(|(_, stream)| stream.num_scopes)
                .collect::<Vec<_>>()
        })
        .sum();
    assert!(
        gpu_scopes > 0,
        "no GPU scope reached puffin across {} frames",
        frames.len()
    );
}

/// A declared parent has to survive the trip through the bridge: a flat tree reports the shading
/// pass and the pass containing it as siblings, and their times then read as additive when one is
/// inside the other.
#[test]
fn nesting_survives_the_bridge() {
    let _guard = PUFFIN.lock().unwrap_or_else(|e| e.into_inner());
    let Some((device, queue)) = timestamp_device() else {
        eprintln!("skipped: no adapter with timestamp queries");
        return;
    };
    let mut scopes = GpuScopes::new(&device, &queue).expect("profiler settings are valid");

    let frames: Arc<Mutex<Vec<Arc<puffin::FrameData>>>> = Arc::default();
    let sink = {
        let frames = Arc::clone(&frames);
        puffin::GlobalProfiler::lock().add_sink(Box::new(move |frame| {
            frames.lock().unwrap().push(frame);
        }))
    };
    puffin::set_scopes_on(true);

    for _ in 0..8 {
        let mut encoder = device.create_command_encoder(&Default::default());
        let outer = scopes.begin("outer", &mut encoder);
        let inner = scopes.begin_child("inner", &mut encoder, &outer);
        busy_copy(&device, &mut encoder);
        scopes.end(&mut encoder, inner);
        scopes.end(&mut encoder, outer);
        scopes.resolve(&mut encoder);
        queue.submit(Some(encoder.finish()));
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });
        scopes.end_frame(&queue);
        puffin::GlobalProfiler::lock().new_frame();
    }

    puffin::GlobalProfiler::lock().remove_sink(sink);
    let deepest = frames
        .lock()
        .unwrap()
        .iter()
        .filter_map(|frame| frame.unpacked().ok())
        .flat_map(|frame| {
            frame
                .thread_streams
                .iter()
                .filter(|(info, _)| info.name == "GPU")
                .map(|(_, stream)| stream.depth)
                .collect::<Vec<_>>()
        })
        .max()
        .unwrap_or(0);
    assert!(deepest >= 1, "the inner scope came back as a sibling");
}

/// 🔴 A nested scope is inside its parent's range, so adding both counts the same nanoseconds twice.
#[cfg(feature = "gpu-profiler")]
#[test]
fn nesting_is_not_counted_twice() {
    use super::gpu_span_ms;

    let leaf = |label: &str, from: f64, to: f64| wgpu_profiler::GpuTimerQueryResult {
        label: label.to_owned(),
        time: Some(from..to),
        nested_queries: Vec::new(),
        pid: 0,
        tid: std::thread::current().id(),
    };

    let mut parent = leaf("shadow pages", 0.0, 0.009);
    parent.nested_queries = vec![leaf("page raster", 0.001, 0.008)];
    let results = vec![parent, leaf("main view", 0.010, 0.0105)];

    let ms = gpu_span_ms(&results);
    assert!(
        (ms - 9.5).abs() < 0.01,
        "expected the two top-level spans (9 + 0.5), got {ms}",
    );
}

/// A scope the driver never resolved is skipped, not counted as zero —
/// a zero would drag the average down and read as the GPU speeding up.
#[cfg(feature = "gpu-profiler")]
#[test]
fn an_unresolved_scope_is_skipped() {
    use super::gpu_span_ms;

    let unresolved = wgpu_profiler::GpuTimerQueryResult {
        label: "never resolved".to_owned(),
        time: None,
        nested_queries: Vec::new(),
        pid: 0,
        tid: std::thread::current().id(),
    };
    assert_eq!(gpu_span_ms(&[unresolved]), 0.0);
}

/// One shader's cost is every scope carrying its label, however deep: two materials using the same
/// shader are two scopes nested in the shading pass, and the graph shows their sum (#1159).
#[test]
fn same_labels_sum_across_depths() {
    let scope = |label: &str, start: f64, end: f64, nested| wgpu_profiler::GpuTimerQueryResult {
        label: label.to_owned(),
        pid: 0,
        tid: std::thread::current().id(),
        time: Some(start..end),
        nested_queries: nested,
    };
    let results = vec![scope(
        "shade",
        0.0,
        0.010,
        vec![
            scope("shader a", 0.0, 0.002, vec![]),
            scope("shader b", 0.002, 0.005, vec![]),
            scope("shader a", 0.005, 0.006, vec![]),
        ],
    )];

    let mut totals = std::collections::HashMap::new();
    sum_by_label(&results, &mut totals);

    let a = totals["shader a"];
    assert!((a - 3.0).abs() < 1e-3, "shader a: {a} ms");
    assert!((totals["shader b"] - 3.0).abs() < 1e-3);
    assert!((totals["shade"] - 10.0).abs() < 1e-3);
}

/// A scope opened inside a compute pass (one per material on the compute shading path) comes back
/// with a time under its label. Skipped where the adapter cannot write timestamps inside passes.
#[test]
fn a_pass_scope_is_timed() {
    let _guard = PUFFIN.lock().unwrap_or_else(|e| e.into_inner());
    let Some((device, queue)) = timestamp_device() else {
        eprintln!("skipped: no adapter with timestamp queries");
        return;
    };
    if !device
        .features()
        .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES)
    {
        eprintln!("skipped: no timestamps inside passes");
        return;
    }
    let mut scopes = GpuScopes::new(&device, &queue).expect("profiler settings are valid");
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("busy"),
        source: wgpu::ShaderSource::Wgsl(
            "@compute @workgroup_size(8) fn main() { var x = 0u; for (var i = 0u; i < 64u; i++) { x += i; } }"
                .into(),
        ),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("busy"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });

    for _ in 0..8 {
        let mut encoder = device.create_command_encoder(&Default::default());
        let parent = scopes.begin("shade", &mut encoder);
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            for _ in 0..2 {
                let query = scopes.begin_in("shader a", &mut pass, Some(&parent));
                pass.dispatch_workgroups(64, 64, 1);
                scopes.end_in(&mut pass, query);
            }
        }
        scopes.end(&mut encoder, parent);
        scopes.resolve(&mut encoder);
        queue.submit(Some(encoder.finish()));
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });
        scopes.end_frame(&queue);
    }

    assert!(
        scopes.scope_ms("shader a").is_some(),
        "no time for the in-pass scope: {:?}",
        scopes.totals().collect::<Vec<_>>()
    );
}
