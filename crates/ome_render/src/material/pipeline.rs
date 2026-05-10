//! `MaterialPipeline` — CPU-side coordinator that mirrors
//! `Assets<Material>` into the GPU [`MaterialPool`].
//!
//! Pattern matches `MeshletPipeline`: a [`Guid`] → slot registry
//! plus a per-frame sync function that resolves new GUIDs through
//! the `AssetServer`, fetches the CPU asset, and writes the packed
//! [`MaterialParams`] into a pre-allocated GPU storage buffer.
//!
//! The pool is created with a generous fixed capacity (`DEFAULT_CAPACITY`)
//! at startup so the deferred shader can index it without rebinding.
//! Slot 0 is reserved as the **default white-diffuse** so any
//! `MeshInstance` that fails to resolve a material still renders
//! with sensible colour — matches the legacy behaviour of the old
//! per-render-call `material_id = 0`.

use std::collections::HashMap;

use ome_core::Guid;
use ome_core::asset_database::AssetDatabase;
use ome_core::asset_loader::AssetServer;
use ome_core::assets::Assets;
use ome_core::resource::Resources;

use super::{Material, MaterialParams, MaterialPool};

/// Static type name [`AssetEntry`s carry] when their loader is
/// [`MaterialLoader`](super::MaterialLoader). Keeps the picker's
/// `#[reflect(asset = …)]` attribute and the pipeline's filter in
/// lock-step: change one and the other fails the regression test
/// pinned in `mesh_renderer::tests`.
pub const MATERIAL_TYPE_NAME: &str = "ome_render::material::asset::Material";

/// Default capacity of the GPU pool. The shader hard-codes a
/// runtime-sized `array<MaterialParams>` so this is just the upper
/// bound on registered materials per session — bumping it is a no-op
/// other than a slightly larger storage buffer at startup.
pub const DEFAULT_CAPACITY: u32 = 256;

/// Index of the implicit white-diffuse fallback material. Reserved
/// at construction so resolving a missing GUID always lands on a
/// well-defined slot instead of reading uninitialised memory.
pub const FALLBACK_MATERIAL_ID: u32 = 0;

/// Coordinates the GPU material pool with the CPU asset storage.
///
/// Owns the [`MaterialPool`] and a `Guid → slot` registry. Insert
/// into `Resources` at startup; the meshlet sync system queries
/// [`Self::lookup`] when assembling per-instance `material_id`s,
/// and the per-frame [`Self::sync_from_resources`] keeps the pool
/// in step with `Assets<Material>` as the user picks new materials.
pub struct MaterialPipeline {
    pool: MaterialPool,
    registry: HashMap<Guid, u32>,
    /// Index of the next free slot to hand out. Starts at 1 because
    /// slot 0 is the white-diffuse fallback.
    next_slot: u32,
    capacity: u32,
}

impl MaterialPipeline {
    /// Builds a fresh pipeline with `DEFAULT_CAPACITY` slots and the
    /// fallback material pre-installed at slot 0. Uploads `capacity`
    /// copies of the white-diffuse default to the GPU so reads from
    /// any unused slot are well-defined.
    pub fn new(device: &wgpu::Device) -> Self {
        Self::with_capacity(device, DEFAULT_CAPACITY)
    }

    /// Capacity-explicit constructor. `capacity` must be ≥ 1 — slot 0
    /// is always the fallback.
    pub fn with_capacity(device: &wgpu::Device, capacity: u32) -> Self {
        assert!(capacity >= 1, "MaterialPipeline capacity must be >= 1");
        let initial = vec![MaterialParams::default(); capacity as usize];
        let pool = MaterialPool::new(device, &initial);
        Self {
            pool,
            registry: HashMap::new(),
            next_slot: 1,
            capacity,
        }
    }

    /// Returns the current slot count (registered materials + fallback).
    pub fn registered_count(&self) -> u32 {
        self.registry.len() as u32
    }

    /// Read-only handle to the underlying GPU pool.
    pub fn pool(&self) -> &MaterialPool {
        &self.pool
    }

    /// Returns the slot previously assigned to `guid`, or `None` if
    /// the material has not been registered yet.
    pub fn lookup(&self, guid: Guid) -> Option<u32> {
        self.registry.get(&guid).copied()
    }

    /// Resolves `guid` to its slot, falling back to
    /// [`FALLBACK_MATERIAL_ID`] when the GUID is unknown. Used by
    /// the meshlet scene system when assembling `MeshInstance`s.
    pub fn lookup_or_fallback(&self, guid: Option<Guid>) -> u32 {
        let Some(g) = guid else {
            return FALLBACK_MATERIAL_ID;
        };
        match self.registry.get(&g) {
            Some(&slot) => slot,
            None => {
                tracing::debug!(
                    target: "ome_render::material::sync",
                    guid = %g,
                    registered = self.registry.len(),
                    "lookup_or_fallback miss; using FALLBACK_MATERIAL_ID",
                );
                FALLBACK_MATERIAL_ID
            }
        }
    }

    /// Writes `material`'s packed params into the GPU pool and
    /// records the assigned slot under `guid`. Idempotent: calling
    /// twice with the same GUID returns the existing slot and
    /// **upgrades the GPU contents** so live edits land without a
    /// new slot allocation.
    pub fn register(
        &mut self,
        queue: &wgpu::Queue,
        guid: Guid,
        material: &Material,
    ) -> u32 {
        let params = material.to_params();
        if let Some(&slot) = self.registry.get(&guid) {
            self.pool.write(queue, slot, &params);
            tracing::debug!(
                target: "ome_render::material::sync",
                guid = %guid,
                slot,
                "MaterialPipeline.register: refreshed existing slot",
            );
            return slot;
        }
        if self.next_slot >= self.capacity {
            tracing::warn!(
                target: "ome_render::material::sync",
                guid = %guid,
                capacity = self.capacity,
                "MaterialPipeline pool full; falling back to slot 0",
            );
            return FALLBACK_MATERIAL_ID;
        }
        let slot = self.next_slot;
        self.next_slot += 1;
        self.pool.write(queue, slot, &params);
        self.registry.insert(guid, slot);
        tracing::debug!(
            target: "ome_render::material::sync",
            guid = %guid,
            slot,
            registered = self.registry.len(),
            "MaterialPipeline.register: assigned new slot",
        );
        slot
    }

    /// Per-frame sync. Walks every [`Material`] entry the
    /// [`AssetDatabase`] knows about, resolves each GUID through
    /// the [`AssetServer`], and registers it. Idempotent — already-
    /// registered GUIDs are re-uploaded so live RON edits land
    /// without restarting the editor.
    ///
    /// Take/put pattern on `AssetServer` mirrors what
    /// `MeshletRenderStage::sync_assets_to_gpu` does — necessary
    /// because `load_by_guid` needs `&mut Resources` while we
    /// already hold `&mut AssetServer`.
    pub fn sync_from_resources(
        &mut self,
        queue: &wgpu::Queue,
        resources: &mut Resources,
    ) {
        let pending: Vec<Guid> = match resources.get::<AssetDatabase>() {
            Some(db) => db
                .entries_of_type(MATERIAL_TYPE_NAME)
                .map(|(guid, _)| guid)
                .collect(),
            None => return,
        };
        tracing::debug!(
            target: "ome_render::material::sync",
            pending = pending.len(),
            type_name = MATERIAL_TYPE_NAME,
            "sync_from_resources: pending materials from AssetDatabase",
        );
        if pending.is_empty() {
            return;
        }

        let Some(mut server) = resources.remove::<AssetServer>() else {
            tracing::warn!(
                target: "ome_render::material::sync",
                "AssetServer missing; skipping material sync",
            );
            return;
        };

        // Resolve every GUID through the server first (this populates
        // Assets<Material> if not already loaded), then in a second
        // pass read the assets and register them. Two passes because
        // both the load and the read need `resources`, and we want
        // to put the server back before we borrow `Assets<Material>`.
        let mut handles: Vec<(Guid, ome_core::assets::Handle<Material>)> =
            Vec::with_capacity(pending.len());
        for guid in &pending {
            match server.load_by_guid::<Material>(*guid, resources) {
                Ok(h) => handles.push((*guid, h)),
                Err(e) => {
                    tracing::warn!(
                        target: "ome_render::material::sync",
                        guid = %guid,
                        error = %e,
                        "failed to load material asset by GUID",
                    );
                }
            }
        }
        resources.insert(server);

        let Some(assets) = resources.get::<Assets<Material>>() else {
            tracing::warn!(
                target: "ome_render::material::sync",
                "Assets<Material> missing after load; aborting material sync",
            );
            return;
        };
        let mut snapshots: Vec<(Guid, Material)> = Vec::with_capacity(handles.len());
        for (guid, handle) in handles {
            match assets.get(handle) {
                Some(m) => snapshots.push((guid, m.clone())),
                None => tracing::debug!(
                    target: "ome_render::material::sync",
                    guid = %guid,
                    "sync_from_resources: handle resolved but Assets<Material>.get returned None — material dropped from this frame's snapshot",
                ),
            }
        }
        for (guid, mat) in snapshots {
            self.register(queue, guid, &mat);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::OnceLock;

    // Shared device per test binary — see issue #334. Mesa radv races
    // when many threads invoke `request_adapter` concurrently.
    static SHARED_DEVICE: OnceLock<Option<(wgpu::Device, wgpu::Queue)>> = OnceLock::new();

    fn try_acquire_device() -> Option<(wgpu::Device, wgpu::Queue)> {
        SHARED_DEVICE
            .get_or_init(|| {
                let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                    backends: wgpu::Backends::VULKAN
                        | wgpu::Backends::DX12
                        | wgpu::Backends::METAL,
                    flags: wgpu::InstanceFlags::default(),
                    backend_options: wgpu::BackendOptions::default(),
                    memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
                    display: None,
                });
                let adapter = pollster::block_on(instance.request_adapter(
                    &wgpu::RequestAdapterOptions {
                        power_preference: wgpu::PowerPreference::HighPerformance,
                        compatible_surface: None,
                        force_fallback_adapter: false,
                    },
                ))
                .ok()?;
                pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                    label: Some("material_pipeline_test_device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::default(),
                    trace: wgpu::Trace::Off,
                    experimental_features: wgpu::ExperimentalFeatures::default(),
                }))
                .ok()
            })
            .clone()
    }

    #[test]
    fn lookup_or_fallback_returns_zero_for_unknown_guid() {
        let Some((device, _)) = try_acquire_device() else { return };
        let pipeline = MaterialPipeline::new(&device);
        assert_eq!(pipeline.lookup_or_fallback(None), FALLBACK_MATERIAL_ID);
        assert_eq!(
            pipeline.lookup_or_fallback(Some(Guid::new_v4())),
            FALLBACK_MATERIAL_ID,
        );
    }

    #[test]
    fn register_assigns_distinct_slots_starting_at_one() {
        let Some((device, queue)) = try_acquire_device() else { return };
        let mut pipeline = MaterialPipeline::new(&device);
        let g1 = Guid::new_v4();
        let g2 = Guid::new_v4();
        let s1 = pipeline.register(&queue, g1, &Material::default());
        let s2 = pipeline.register(&queue, g2, &Material::default());
        assert_eq!(s1, 1);
        assert_eq!(s2, 2);
        assert_eq!(pipeline.lookup(g1), Some(1));
        assert_eq!(pipeline.lookup(g2), Some(2));
        assert_eq!(pipeline.registered_count(), 2);
    }

    #[test]
    fn register_is_idempotent_on_same_guid() {
        let Some((device, queue)) = try_acquire_device() else { return };
        let mut pipeline = MaterialPipeline::new(&device);
        let g = Guid::new_v4();
        let s0 = pipeline.register(&queue, g, &Material::default());
        let s1 = pipeline.register(&queue, g, &Material::new([1.0, 0.0, 0.0, 1.0], 0.0, 0.5, 0.0));
        assert_eq!(s0, s1, "same GUID must reuse the same slot");
        assert_eq!(pipeline.registered_count(), 1);
    }

    #[test]
    fn register_falls_back_when_capacity_exhausted() {
        let Some((device, queue)) = try_acquire_device() else { return };
        // capacity = 2 → slot 0 fallback + slot 1 = the only spawnable slot.
        let mut pipeline = MaterialPipeline::with_capacity(&device, 2);
        let g1 = Guid::new_v4();
        let g2 = Guid::new_v4();
        let s1 = pipeline.register(&queue, g1, &Material::default());
        let s2 = pipeline.register(&queue, g2, &Material::default());
        assert_eq!(s1, 1, "first registration takes slot 1");
        assert_eq!(
            s2, FALLBACK_MATERIAL_ID,
            "overflowing registration falls back to slot 0",
        );
    }
}
