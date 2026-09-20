//! `MaterialPipeline` — CPU-side coordinator that mirrors `Assets<Material>` into the GPU
//! [`MaterialPool`].

use std::collections::{HashMap, HashSet};

use kooch_core::Guid;
use kooch_core::asset_database::AssetDatabase;
use kooch_core::asset_loader::{AssetServer, ReloadedAssets};
use kooch_core::assets::Assets;
use kooch_core::resource::Resources;

use super::{
    MAX_PARAM_TEXTURES, Material, MaterialParams, MaterialPool, MaterialTexturePool, PackedParams,
    ParamValue, Shader, ShaderKind, ShaderParam, TextureRef,
};
use crate::texture::Image;

/// A surface shader's source as the render last saw it.
#[derive(Clone, Debug)]
pub struct SurfaceSource {
    /// [`ReloadedAssets`]' revision the source was copied at.
    pub revision: u64,
    pub source: std::sync::Arc<str>,
    /// The declared parameters, in packing order.
    pub params: std::sync::Arc<[ShaderParam]>,
    /// The WGSL generated from `params`, composed ahead of `source`.
    pub params_wgsl: std::sync::Arc<str>,
    /// Which pass builds it: a post-process never reaches the surface path.
    pub kind: ShaderKind,
    /// Assigns `alpha_clip`: rasterised in the masked bin (#452).
    pub masked: bool,
    /// Masks by uv and textures alone, so the cut can become geometry (#452).
    pub still: bool,
}

/// Textures whose `.meta` changed and have to be uploaded again.
#[derive(Debug, Default)]
pub struct TextureReimports(pub std::collections::HashSet<Guid>);

impl TextureReimports {
    /// Marks `guid` for re-upload on the next texture sync.
    pub fn queue(&mut self, guid: Guid) {
        self.0.insert(guid);
    }
}

/// Static type name [`AssetEntry`s carry] when their loader is
/// [`MaterialLoader`](super::MaterialLoader).
pub const MATERIAL_TYPE_NAME: &str = "kooch_render::material::asset::Material";

/// Default capacity of the GPU pool. The shader hard-codes a runtime-sized `array<MaterialParams>`
/// so this is just the upper bound on registered materials per session — bumping it is a no-op
/// other than a slightly larger storage buffer at startup.
pub const DEFAULT_CAPACITY: u32 = 256;

/// Index of the implicit white-diffuse fallback material. Reserved
/// at construction so resolving a missing GUID always lands on a
/// well-defined slot instead of reading uninitialised memory.
pub const FALLBACK_MATERIAL_ID: u32 = 0;

/// One number for everything a slot draws with: its parameters, its textures and its shader.
fn stamp_of(params: &MaterialParams, packed: &PackedParams, shader: Option<Guid>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytemuck::bytes_of(params).hash(&mut hasher);
    bytemuck::cast_slice::<f32, u8>(&packed.values).hash(&mut hasher);
    packed.textures.hash(&mut hasher);
    shader.hash(&mut hasher);
    hasher.finish()
}

/// Coordinates the GPU material pool with the CPU asset storage.
pub struct MaterialPipeline {
    pool: MaterialPool,
    /// GPU texture store + per-material bind group factory for the
    /// two-pass material shader. Populated during sync alongside `pool`.
    texture_pool: MaterialTexturePool,
    registry: HashMap<Guid, u32>,
    /// Per-slot textures in group-4 order, parallel to the GPU pool slots: the PBR maps, or a custom
    /// shader's declared textures. The render builds each material's bind group from it.
    slot_textures: Vec<[TextureRef; MAX_PARAM_TEXTURES as usize]>,
    /// Per-slot `.shader`, parallel to `slot_textures`. `None` is the default surface.
    slot_shaders: Vec<Option<Guid>>,
    /// Per slot, a hash of what it was last registered with. The geometry trim (#452) cuts against
    /// values, so it has to know when they move.
    slot_stamps: Vec<u64>,
    /// Every shader a registered material names, as last loaded.
    surfaces: HashMap<Guid, SurfaceSource>,
    /// Index of the next free slot to hand out. Starts at 1 because
    /// slot 0 is the white-diffuse fallback.
    next_slot: u32,
    capacity: u32,
}

impl MaterialPipeline {
    /// Builds a fresh pipeline with `DEFAULT_CAPACITY` slots and the fallback material
    /// pre-installed at slot 0. Uploads `capacity` copies of the white-diffuse default to the GPU
    /// so reads from any unused slot are well-defined.
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
        let slot_textures = vec![PackedParams::default_surface(&Material::default()).textures];
        Self {
            pool,
            texture_pool,
            registry: HashMap::new(),
            slot_textures,
            slot_shaders: vec![None],
            slot_stamps: vec![0],
            surfaces: HashMap::new(),
            next_slot: 1,
            capacity,
        }
    }

    /// Returns the current slot count (registered materials + fallback).
    pub fn registered_count(&self) -> u32 {
        self.registry.len() as u32
    }

    /// Sets how many samples the material sampler takes along the long axis of a footprint, and
    /// reports whether it changed.
    pub fn set_anisotropy(&mut self, device: &wgpu::Device, samples: u16) -> bool {
        self.texture_pool.set_anisotropy(device, samples)
    }

    /// Uploads a texture straight into the pool under `guid`.
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

    /// Writes `material`'s packed params into the GPU pool and records the assigned slot under
    /// `guid`. Idempotent: calling twice with the same GUID returns the existing slot and
    /// **upgrades the GPU contents** so live edits land without a new slot allocation.
    pub fn register(&mut self, queue: &wgpu::Queue, guid: Guid, material: &Material) -> u32 {
        let params = material.to_params();
        let packed = self.pack(material);
        let refs = packed.textures;
        let stamp = stamp_of(&params, &packed, material.shader);
        if let Some(&slot) = self.registry.get(&guid) {
            self.pool.write(queue, slot, &params);
            self.pool.write_values(queue, slot, &packed.values);
            self.slot_textures[slot as usize] = refs;
            self.slot_shaders[slot as usize] = material.shader;
            self.slot_stamps[slot as usize] = stamp;
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
        self.pool.write_values(queue, slot, &packed.values);
        self.registry.insert(guid, slot);
        debug_assert_eq!(
            self.slot_textures.len(),
            slot as usize,
            "slot_textures must stay parallel to sequential slot allocation",
        );
        self.slot_textures.push(refs);
        self.slot_shaders.push(material.shader);
        self.slot_stamps.push(stamp);
        tracing::debug!(
            target: "kooch_render::material::sync",
            guid = %guid,
            slot,
            registered = self.registry.len(),
            "MaterialPipeline.register: assigned new slot",
        );
        slot
    }

    /// What `material` uploads: its shader's parameters when that shader is loaded, the PBR maps
    /// otherwise — the same surface the render picks for it.
    fn pack(&self, material: &Material) -> PackedParams {
        match material.shader.and_then(|guid| self.surfaces.get(&guid)) {
            Some(surface) => PackedParams::for_shader(material, &surface.params),
            None => PackedParams::default_surface(material),
        }
    }

    /// Read-only handle to the texture pool, for building per-material
    /// bind groups in the two-pass render path.
    pub fn texture_pool(&self) -> &MaterialTexturePool {
        &self.texture_pool
    }

    /// The textures a slot binds, in group-4 order. Out-of-range slots bind the fallback's.
    pub fn slot_texture_refs(&self, slot: u32) -> [TextureRef; MAX_PARAM_TEXTURES as usize] {
        self.slot_textures
            .get(slot as usize)
            .copied()
            .unwrap_or(self.slot_textures[0])
    }

    /// A hash of the values a slot was registered with: the geometry trim (#452) cuts against them
    /// and rebuilds when they move.
    pub fn slot_stamp(&self, slot: u32) -> u64 {
        self.slot_stamps.get(slot as usize).copied().unwrap_or(0)
    }

    /// The custom surface a slot shades with, or `None` for the default one — including a slot
    /// whose shader has not loaded.
    pub fn slot_surface(&self, slot: u32) -> Option<(Guid, &SurfaceSource)> {
        let guid = (*self.slot_shaders.get(slot as usize)?)?;
        self.surfaces.get(&guid).map(|surface| (guid, surface))
    }

    /// Range of shading slots (`0..next_slot`) the two-pass path issues a per-material fragment
    /// pass for. Includes slot 0 (fallback white): geometry with no picked material resolves to it,
    /// so it must shade too — its branch-free fallback textures reproduce the plain look.
    pub fn shading_slots(&self) -> std::ops::Range<u32> {
        0..self.next_slot
    }

    /// Per-frame sync. Walks every [`Material`] entry the [`AssetDatabase`] knows about, resolves
    /// each GUID through the [`AssetServer`], and registers it. Idempotent — already- registered
    /// GUIDs are re-uploaded so live RON edits land without restarting the editor.
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

        // Resolve every GUID through the server first (this populates Assets<Material> if not
        // already loaded), then in a second pass read the assets and register them.
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
        self.sync_surfaces(&snapshots, resources);

        for (guid, mat) in snapshots {
            self.register(queue, guid, &mat);
        }
    }

    /// Makes `shader` the surface of every material naming `guid`, without an asset behind it — a
    /// shader generated at runtime, or a test's. Register the materials after it: their parameters
    /// are packed against it.
    pub fn add_shader(&mut self, guid: Guid, shader: &Shader) {
        self.surfaces.insert(
            guid,
            SurfaceSource {
                revision: 0,
                source: shader.source.as_str().into(),
                params: shader.params.clone().into(),
                params_wgsl: shader.params_wgsl().into(),
                kind: shader.kind,
                masked: shader.masked(),
                still: shader.masks_still(),
            },
        );
    }

    /// Loads every shader the materials name and copies a source out whenever its revision moved,
    /// which is what tells the render to rebuild that shader's pipelines.
    fn sync_surfaces(&mut self, snapshots: &[(Guid, Material)], resources: &mut Resources) {
        let named: HashSet<Guid> = snapshots.iter().filter_map(|(_, m)| m.shader).collect();
        self.surfaces.retain(|guid, _| named.contains(guid));
        if named.is_empty() {
            return;
        }
        let Some(mut server) = resources.remove::<AssetServer>() else {
            return;
        };
        let handles: Vec<_> = named
            .into_iter()
            .filter_map(
                |guid| match server.load_by_guid::<Shader>(guid, resources) {
                    Ok(handle) => Some((guid, handle)),
                    Err(e) => {
                        tracing::warn!(
                            target: "kooch_render::material::sync",
                            guid = %guid,
                            error = %e,
                            "failed to load shader; its materials use the default surface",
                        );
                        None
                    }
                },
            )
            .collect();
        resources.insert(server);

        let (Some(shaders), reloaded) = (
            resources.get::<Assets<Shader>>(),
            resources.get::<ReloadedAssets>(),
        ) else {
            return;
        };
        for (guid, handle) in handles {
            let revision = reloaded.map_or(0, |r| r.revision(guid));
            if self
                .surfaces
                .get(&guid)
                .is_some_and(|s| s.revision == revision)
            {
                continue;
            }
            let Some(shader) = shaders.get(handle) else {
                continue;
            };
            self.surfaces.insert(
                guid,
                SurfaceSource {
                    revision,
                    source: shader.source.as_str().into(),
                    params: shader.params.clone().into(),
                    params_wgsl: shader.params_wgsl().into(),
                    kind: shader.kind,
                    masked: shader.masked(),
                    still: shader.masks_still(),
                },
            );
        }
    }

    /// Loads every not-yet-uploaded texture GUID referenced by `snapshots` through the
    /// [`AssetServer`] and registers the decoded [`Image`]s in the [`MaterialTexturePool`].
    /// Deduplicates against both the current snapshot set and textures already resident.
    fn sync_textures(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        snapshots: &[(Guid, Material)],
        resources: &mut Resources,
    ) {
        // The sampler follows the project's setting. Here rather than in the render, which only has
        // `&Resources` — and cheap: it rebuilds one sampler when the number changes and returns
        // immediately when it has not.
        if let Some(shading) = resources.get::<crate::quality::ShadingSettings>().copied()
            && self.texture_pool.set_anisotropy(device, shading.anisotropy)
        {
            tracing::info!(
                target: "kooch_render::material",
                samples = shading.anisotropy,
                "material sampler anisotropy changed",
            );
        }

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
            let values = mat.values.values().filter_map(|value| match value {
                ParamValue::Texture(guid) => *guid,
                ParamValue::Number(_) => None,
            });
            for guid in [mat.albedo, mat.normal, mat.metal_roughness]
                .into_iter()
                .flatten()
                .chain(values)
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
