//! End-to-end: a scope opened on an encoder has to come back out as a
//! puffin scope on the `GPU` thread. Every piece between those two
//! points is a place a capture can go silently empty — the query set,
//! the resolve, the buffer map, the clock shift — and none of them
//! report anything when they fail.

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

/// A declared parent has to survive the trip through the bridge: a flat
/// tree reports the shading pass and the pass containing it as siblings,
/// and their times then read as additive when one is inside the other.
///
/// 🔴 This failed on the first run with `begin` for both scopes — being
/// open is not what makes a scope a parent, `begin_child` is.
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
