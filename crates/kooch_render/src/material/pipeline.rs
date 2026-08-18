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

use std::collections::{HashMap, HashSet};

use kooch_core::Guid;
use kooch_core::asset_database::AssetDatabase;
use kooch_core::asset_loader::AssetServer;
use kooch_core::assets::Assets;
use kooch_core::resource::Resources;

use super::{Material, MaterialParams, MaterialPool, MaterialTexturePool};
use crate::texture::Image;

/// Textures whose `.meta` changed and have to be uploaded again.
///
/// A resource rather than a method call because the editor is what
/// edits an import and the pool lives inside the render stage, several
/// borrows away. Whoever rewrites a sidecar puts the GUID here; the
/// texture sync drains it on its next pass.
///
/// 🔴 It exists because a mip chain is **levels allocated at texture
/// creation**. There is no API that adds one afterwards, so an import
/// setting that only rewrote the file would show its effect the next
/// time the project was opened — which reads as "the checkbox does
/// nothing".
#[derive(Debug, Default)]
pub struct TextureReimports(pub std::collections::HashSet<Guid>);

impl TextureReimports {
    /// Marks `guid` for re-upload on the next texture sync.
    pub fn queue(&mut self, guid: Guid) {
        self.0.insert(guid);
    }
}

/// Static type name [`AssetEntry`s carry] when their loader is
/// [`MaterialLoader`](super::MaterialLoader). Keeps the picker's
/// `#[reflect(asset = …)]` attribute and the pipeline's filter in
/// lock-step: change one and the other fails the regression test
/// pinned in `mesh_renderer::tests`.
pub const MATERIAL_TYPE_NAME: &str = "kooch_render::material::asset::Material";

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
    /// GPU texture store + per-material bind group factory for the
    /// two-pass material shader. Populated during sync alongside `pool`.
    texture_pool: MaterialTexturePool,
    registry: HashMap<Guid, u32>,
    /// Per-slot texture GUID triple `[albedo, normal, metal_roughness]`,
    /// indexed by material slot (parallel to the GPU pool slots). Slot 0
    /// is the fallback's all-`None`. The render path reads this to build
    /// each material pass's bind group via [`MaterialTexturePool`].
    slot_textures: Vec<[Option<Guid>; 3]>,
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
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self::with_capacity(device, queue, DEFAULT_CAPACITY)
    }

    /// Capacity-explicit constructor. `capacity` must be ≥ 1 — slot 0
    /// is always the fallback.
    pub fn with_capacity(device: &wgpu::Device, queue: &wgpu::Queue, capacity: u32) -> Self {
        assert!(capacity >= 1, "MaterialPipeline capacity must be >= 1");
        let initial = vec![MaterialParams::default(); capacity as usize];
        let pool = MaterialPool::new(device, &initial);
        let texture_pool = MaterialTexturePool::new(device, queue);
        // Slot 0 = fallback material, references no textures.
        let slot_textures = vec![[None; 3]];
        Self {
            pool,
            texture_pool,
            registry: HashMap::new(),
            slot_textures,
            next_slot: 1,
            capacity,
        }
    }

    /// Returns the current slot count (registered materials + fallback).
    pub fn registered_count(&self) -> u32 {
        self.registry.len() as u32
    }

    /// Uploads a texture straight into the pool under `guid`.
    ///
    /// For tests and tools that have the pixels rather than a file on
    /// disk: the normal path is `sync_textures`, which resolves a
    /// material's GUIDs through the `AssetServer` and reads them off the
    /// filesystem. A test that wanted a textured surface had to write a
    /// PNG to a temp directory and stand up an asset database for it,
    /// which is a lot of ceremony for four texels.
    pub fn register_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        guid: Guid,
        image: &Image,
    ) {
        self.texture_pool.register(device, queue, guid, image);
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
                    target: "kooch_render::material::sync",
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
    pub fn register(&mut self, queue: &wgpu::Queue, guid: Guid, material: &Material) -> u32 {
        let params = material.to_params();
        let refs = [material.albedo, material.normal, material.metal_roughness];
        if let Some(&slot) = self.registry.get(&guid) {
            self.pool.write(queue, slot, &params);
            self.slot_textures[slot as usize] = refs;
            tracing::debug!(
                target: "kooch_render::material::sync",
                guid = %guid,
                slot,
                "MaterialPipeline.register: refreshed existing slot",
            );
            return slot;
        }
        if self.next_slot >= self.capacity {
            tracing::warn!(
                target: "kooch_render::material::sync",
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
        debug_assert_eq!(
            self.slot_textures.len(),
            slot as usize,
            "slot_textures must stay parallel to sequential slot allocation",
        );
        self.slot_textures.push(refs);
        tracing::debug!(
            target: "kooch_render::material::sync",
            guid = %guid,
            slot,
            registered = self.registry.len(),
            "MaterialPipeline.register: assigned new slot",
        );
        slot
    }

    /// Read-only handle to the texture pool, for building per-material
    /// bind groups in the two-pass render path.
    pub fn texture_pool(&self) -> &MaterialTexturePool {
        &self.texture_pool
    }

    /// The `[albedo, normal, metal_roughness]` texture GUIDs a slot
    /// references. Out-of-range or fallback slots return all-`None`.
    pub fn slot_texture_refs(&self, slot: u32) -> [Option<Guid>; 3] {
        self.slot_textures
            .get(slot as usize)
            .copied()
            .unwrap_or([None; 3])
    }

    /// Range of shading slots (`0..next_slot`) the two-pass path issues a
    /// per-material fragment pass for. Includes slot 0 (fallback white):
    /// geometry with no picked material resolves to it, so it must shade
    /// too — its branch-free fallback textures reproduce the plain look.
    pub fn shading_slots(&self) -> std::ops::Range<u32> {
        0..self.next_slot
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
        device: &wgpu::Device,
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
            target: "kooch_render::material::sync",
            pending = pending.len(),
            type_name = MATERIAL_TYPE_NAME,
            "sync_from_resources: pending materials from AssetDatabase",
        );
        if pending.is_empty() {
            return;
        }

        let Some(mut server) = resources.remove::<AssetServer>() else {
            tracing::warn!(
                target: "kooch_render::material::sync",
                "AssetServer missing; skipping material sync",
            );
            return;
        };

        // Resolve every GUID through the server first (this populates
        // Assets<Material> if not already loaded), then in a second
        // pass read the assets and register them. Two passes because
        // both the load and the read need `resources`, and we want
        // to put the server back before we borrow `Assets<Material>`.
        let mut handles: Vec<(Guid, kooch_core::assets::Handle<Material>)> =
            Vec::with_capacity(pending.len());
        for guid in &pending {
            match server.load_by_guid::<Material>(*guid, resources) {
                Ok(h) => handles.push((*guid, h)),
                Err(e) => {
                    tracing::warn!(
                        target: "kooch_render::material::sync",
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
                target: "kooch_render::material::sync",
                "Assets<Material> missing after load; aborting material sync",
            );
            return;
        };
        let mut snapshots: Vec<(Guid, Material)> = Vec::with_capacity(handles.len());
        for (guid, handle) in handles {
            match assets.get(handle) {
                Some(m) => snapshots.push((guid, m.clone())),
                None => tracing::debug!(
                    target: "kooch_render::material::sync",
                    guid = %guid,
                    "sync_from_resources: handle resolved but Assets<Material>.get returned None — material dropped from this frame's snapshot",
                ),
            }
        }

        // Upload any newly-referenced texture images before registering
        // the materials, so the render path finds a populated texture
        // pool the moment a slot appears.
        self.sync_textures(device, queue, &snapshots, resources);

        for (guid, mat) in snapshots {
            self.register(queue, guid, &mat);
        }
    }

    /// Loads every not-yet-uploaded texture GUID referenced by
    /// `snapshots` through the [`AssetServer`] and registers the decoded
    /// [`Image`]s in the [`MaterialTexturePool`]. Deduplicates against
    /// both the current snapshot set and textures already resident.
    ///
    /// KNOWN LIMITATION: the `AssetServer`'s `ImageLoader` is registered
    /// sRGB for all images, so normal / metal-roughness maps decode in
    /// the wrong color space. Correct handling needs a per-asset
    /// color-space hint in the `.meta` sidecar — a follow-up; albedo
    /// (the sRGB channel) is already correct.
    fn sync_textures(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        snapshots: &[(Guid, Material)],
        resources: &mut Resources,
    ) {
        // A re-import is a texture the pool must forget before it can
        // ask whether it has it. Drained rather than read, so one edit
        // costs one re-upload.
        if let Some(reimports) = resources.get_mut::<TextureReimports>() {
            let guids: Vec<Guid> = reimports.0.drain().collect();
            for guid in guids {
                self.texture_pool.evict(guid);
            }
        }
        let mut pending: Vec<Guid> = Vec::new();
        let mut seen: HashSet<Guid> = HashSet::new();
        for (_, mat) in snapshots {
            for guid in [mat.albedo, mat.normal, mat.metal_roughness]
                .into_iter()
                .flatten()
            {
                if !self.texture_pool.contains(guid) && seen.insert(guid) {
                    pending.push(guid);
                }
            }
        }
        if pending.is_empty() {
            return;
        }

        let Some(mut server) = resources.remove::<AssetServer>() else {
            tracing::warn!(
                target: "kooch_render::material::sync",
                "AssetServer missing; skipping texture sync",
            );
            return;
        };
        let mut handles: Vec<(Guid, kooch_core::assets::Handle<Image>)> =
            Vec::with_capacity(pending.len());
        for guid in &pending {
            match server.load_by_guid::<Image>(*guid, resources) {
                Ok(h) => handles.push((*guid, h)),
                Err(e) => tracing::warn!(
                    target: "kooch_render::material::sync",
                    guid = %guid,
                    error = %e,
                    "failed to load texture image by GUID",
                ),
            }
        }
        resources.insert(server);

        let Some(images) = resources.get::<Assets<Image>>() else {
            tracing::warn!(
                target: "kooch_render::material::sync",
                "Assets<Image> missing after load; aborting texture sync",
            );
            return;
        };
        for (guid, handle) in handles {
            match images.get(handle) {
                Some(img) => self.texture_pool.register(device, queue, guid, img),
                None => tracing::debug!(
                    target: "kooch_render::material::sync",
                    guid = %guid,
                    "texture handle resolved but Assets<Image>.get returned None",
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests;
