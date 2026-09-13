//! on both GPU paths.

#![cfg(feature = "gpu-profiler")]

mod common;

use std::sync::{Arc, Mutex};

use common::build_cube_mesh;
use glam::{Mat4, Vec3};
use kooch_core::Guid;
use kooch_core::gpu::GpuScopes;
use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::registry::ComponentRegistry;
use kooch_ecs::hierarchy::global_transform::GlobalTransform;
use kooch_ecs::mesh_renderer::MeshRenderer;
use kooch_ecs::query::AccessTracker;
use kooch_render::material::{Material, MaterialPipeline};
use kooch_render::meshlet::{
    MeshletDebugCaps, MeshletRenderStage, MeshletRenderStageConfig, build_default_meshlets,
};
use kooch_render::vbuf64::Vbuf64Support;

/// puffin's frame boundary is global, and these tests close it.
static PUFFIN: Mutex<()> = Mutex::new(());

/// A device with timestamp queries, and with the R64 visibility-buffer
/// features when `atomic_vbuf` asks for them. `None` when the adapter
/// cannot supply what was asked — the path simply is not testable here.
fn device_for(atomic_vbuf: bool) -> Option<(wgpu::Device, wgpu::Queue)> {
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

    // Scopes on an encoder need INSIDE_ENCODERS specifically; asking for
    // a feature the adapter lacks fails device creation outright.
    let mut needed =
        wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
    if atomic_vbuf {
        // 🔴 `all_required_features`, not the bundle spelled out again. This site listed
        // `vbuf64_features` alone, which is every flag the R64 path needs except `SHADER_F16` — and
        // the R64 path builds `fsr3_accumulate`, whose first line is `enable f16`.
        needed |= kooch_core::gpu::all_required_features() | wgpu::Features::TEXTURE_ATOMIC;
    }
    if !adapter.features().contains(needed) {
        return None;
    }

    // Mirrors the production `GpuContext` limits: the cull pipeline
    // binds 5 groups and the Hi-Z SPD build wants 12 storage textures.
    let mut limits = wgpu::Limits::default();
    limits.max_storage_textures_per_shader_stage =
        16.min(adapter.limits().max_storage_textures_per_shader_stage);
    limits.max_bind_groups = 6.min(adapter.limits().max_bind_groups);
    limits.max_storage_buffers_per_shader_stage =
        16.min(adapter.limits().max_storage_buffers_per_shader_stage);

    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("gpu_scopes_test_device"),
        required_features: needed,
        required_limits: limits,
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::default(),
    }))
    .ok()
}

/// One cube in front of a camera, and a `GpuScopes` in `Resources` — which is the only thing that
/// makes the stage record anything. The path is decided by what the device supports, exactly as in
/// production.
fn scene(device: &wgpu::Device, queue: &wgpu::Queue) -> (Resources, MeshletRenderStage) {
    let mut resources = Resources::new();
    resources.insert(EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    resources.insert(ArchetypeRegistry::new());
    resources.insert(AccessTracker::new());

    let mut materials = MaterialPipeline::with_capacity(device, queue, 4);
    materials.register(queue, Guid::new_v4(), &Material::default());
    resources.insert(materials);
    resources.insert(GpuScopes::new(device, queue).expect("profiler settings are valid"));

    let mut stage = MeshletRenderStage::new(
        device,
        MeshletRenderStageConfig {
            size: (256, 256),
            instance_capacity: 8,
            meshlet_capacity: 512,
            vbuf64: Vbuf64Support::detect(device),
            debug_caps: MeshletDebugCaps::detect(device),
            ..Default::default()
        },
    );
    let mesh = build_default_meshlets(&build_cube_mesh()).expect("cube meshlets");
    let guid = Guid::new_v4();
    stage.ensure_gpu_mesh(device, guid, &mesh);

    let mut commands = Commands::new();
    commands
        .spawn(&mut resources)
        .insert(MeshRenderer {
            mesh: Some(guid),
            visible: true,
            ..Default::default()
        })
        .insert(GlobalTransform {
            matrix: Mat4::IDENTITY,
        });
    commands.apply(&mut resources);

    (resources, stage)
}

/// Renders `frames` frames and returns every scope name that reached
/// puffin, asserting along the way that at least one carried a real
/// timestamp.
fn recorded_labels(device: &wgpu::Device, queue: &wgpu::Queue, frames: usize) -> Vec<String> {
    labels_with_shading(device, queue, frames, false)
}

/// [`recorded_labels`], with the virtual shadow pages turned on.
fn labels_with_pages(device: &wgpu::Device, queue: &wgpu::Queue, frames: usize) -> Vec<String> {
    labels_from(device, queue, frames, false, true)
}

/// [`recorded_labels`], with the shading path chosen (#824).
fn labels_with_shading(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frames: usize,
    compute_shading: bool,
) -> Vec<String> {
    labels_from(device, queue, frames, compute_shading, false)
}

fn labels_from(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frames: usize,
    compute_shading: bool,
    virtual_pages: bool,
) -> Vec<String> {
    let (mut resources, mut stage) = scene(device, queue);
    if virtual_pages {
        resources.insert(kooch_render::shadow::ShadowSettings {
            virtual_pages: true,
            ..Default::default()
        });
    }
    stage.set_compute_shading(compute_shading);
    let captured: Arc<Mutex<Vec<Arc<puffin::FrameData>>>> = Arc::default();
    let sink = {
        let captured = Arc::clone(&captured);
        puffin::GlobalProfiler::lock().add_sink(Box::new(move |frame| {
            captured.lock().unwrap().push(frame);
        }))
    };
    puffin::set_scopes_on(true);

    let camera = kooch_render::ViewCamera::looking_at(Vec3::new(0.0, 0.0, 4.0), Vec3::ZERO);
    for _ in 0..frames {
        stage.render_with_assets_primary(device, queue, &resources, &camera, 1.0);

        // What `RenderPlugin::render_passes` does after the frame's
        // last submit. Without it the timestamps sit in their query
        // sets and nothing is ever reported.
        let mut scopes = resources
            .remove::<GpuScopes>()
            .expect("inserted by the scene");
        let mut encoder = device.create_command_encoder(&Default::default());
        scopes.resolve(&mut encoder);
        queue.submit(Some(encoder.finish()));
        // Only a test may block like this. It is what lets the buffer
        // mapping complete inside the loop rather than frames later.
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });
        scopes.end_frame(queue);
        resources.insert(scopes);
        puffin::GlobalProfiler::lock().new_frame();
    }
    puffin::GlobalProfiler::lock().remove_sink(sink);

    // Rebuilt from the frames the sink saw, exactly as the panel does:
    // the names travel with the frames, and a name that never made it
    // into one is a name the panel would draw as `scope#ScopeId(n)`.
    let mut view = puffin::FrameView::default();
    let mut timed_scopes = 0usize;
    for frame in captured.lock().unwrap().iter() {
        // `.ok()` rather than a `let else`: whether this can fail
        // depends on puffin's `packing` feature, and the pattern is
        // irrefutable without it.
        let Some(unpacked) = frame.unpacked().ok() else {
            continue;
        };
        timed_scopes += unpacked
            .thread_streams
            .iter()
            .filter(|(thread, _)| thread.name == "GPU")
            .map(|(_, stream)| stream.num_scopes)
            .sum::<usize>();
        view.add_frame(Arc::clone(frame));
    }
    assert!(
        timed_scopes > 0,
        "no timestamped GPU scope reached puffin — any name below was \
         registered without a measurement behind it"
    );
    view.scope_collection()
        .scopes_by_name()
        .keys()
        .map(|name| name.to_string())
        .collect()
}

/// 🔴 The names are the deliverable, not the count. #769 is blocked on knowing *which* pass owns the
/// 96 %, and a `GPU` row of anonymous boxes answers that no better than no row at all.
#[test]
fn the_atomic_path_names_its_passes() {
    let _guard = PUFFIN.lock().unwrap_or_else(|e| e.into_inner());
    let Some((device, queue)) = device_for(true) else {
        eprintln!("no adapter with timestamps + int64 atomics; skipping");
        return;
    };
    let labels = recorded_labels(&device, &queue, 8);
    assert!(
        labels.iter().any(|l| l == "cull"),
        "no cull scope among {labels:?}"
    );
    // 🔴 TWO scopes, not the fused one. `raster + shade` was split when the marking moved between
    // the halves: `Vbuf64Stage::render` became `render_geometry` / `render_shading` so the page
    // marking could read THIS frame's depth, which is the order Unreal use.
    assert!(
        labels.iter().any(|l| l == "raster"),
        "no raster scope among {labels:?}"
    );
    assert!(
        labels.iter().any(|l| l == "shade"),
        "no shading scope among {labels:?}"
    );
    // together. The raster is the same on both shading paths, so a capture that only names the pair
    // dilutes whatever the shading gained: a fifth off the shading reads as a tenth off the fused
    // number.
    assert!(
        labels.iter().any(|l| l == "shade: fragment"),
        "the shading pass is not named separately among {labels:?}"
    );
}

/// 🔴 The label a capture is read by (#824).
#[test]
fn the_compute_path_names_itself() {
    let _guard = PUFFIN.lock().unwrap_or_else(|e| e.into_inner());
    let Some((device, queue)) = device_for(true) else {
        eprintln!("no adapter with timestamps + int64 atomics; skipping");
        return;
    };
    let labels = labels_with_shading(&device, &queue, 8, true);
    assert!(
        labels.iter().any(|l| l == "shade: compute"),
        "no compute shading scope among {labels:?}"
    );
    // ⚠️ Nothing more than that, and the reason is the harness rather than the renderer:
    // `scope_delta` is a DELTA.
}

/// The fallback path an adapter without int64 atomics takes. Left
/// uninstrumented it would produce a capture with a `GPU` row and
/// nothing in it, which reads as "the GPU did nothing".
#[test]
fn the_hi_z_path_names_its_passes() {
    let _guard = PUFFIN.lock().unwrap_or_else(|e| e.into_inner());
    let Some((device, queue)) = device_for(false) else {
        eprintln!("no adapter with timestamp queries; skipping");
        return;
    };
    let labels = recorded_labels(&device, &queue, 8);
    assert!(
        labels.iter().any(|l| l == "cull + raster A"),
        "no pass-A scope among {labels:?}"
    );
    assert!(
        labels.iter().any(|l| l == "hi-z build"),
        "no Hi-Z build scope among {labels:?}"
    );
    assert!(
        labels.iter().any(|l| l == "shade"),
        "no shading scope among {labels:?}"
    );
}

/// 🔴 The track that cost the OneXFly 34 ms nobody could see.
#[test]
fn the_pages_name_their_passes() {
    let _guard = PUFFIN.lock().unwrap_or_else(|e| e.into_inner());
    let Some((device, queue)) = device_for(true) else {
        eprintln!("no adapter with timestamps + int64 atomics; skipping");
        return;
    };
    let labels = labels_with_pages(&device, &queue, 8);
    for wanted in [
        "shadow pages",
        "page mark",
        "page raster",
        "page cull",
        "page expand",
        "page depth",
    ] {
        assert!(
            labels.iter().any(|l| l == wanted),
            "no {wanted:?} scope among {labels:?}"
        );
    }
}
