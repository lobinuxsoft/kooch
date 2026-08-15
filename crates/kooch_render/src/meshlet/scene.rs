//! Scene-wide GPU-driven meshlet pipeline state.
//!
//! Phase 1.E.1: introduces the `MeshInstance` POD and the storage buffer
//! the scene-wide cull dispatch enumerates. Single-mesh multi-instance
//! for now; the global mesh pool that lets one cull dispatch enumerate
//! meshlets across *different* meshes lands in 1.E.1b.
//!
//! # Why this exists (post Phase 1.D close audit)
//!
//! Phase 1.D delivered per-meshlet primitives (cull, vbuf, deferred,
//! materials) but every test in `tests/` rasterizes a single
//! `GpuMeshletMesh` per dispatch. That path is correct DOD-shape but
//! NOT GPU-driven in spirit: the CPU still enumerates meshes one at a
//! time. `feedback_gpu_driven_spirit.md` and the project's planet-
//! scale + GPU-driven constraints demand: hot loop on GPU, scene
//! enumeration on GPU, indirect dispatch fed by compute. This module
//! is the foundation.

use bytemuck::{Pod, Zeroable};
use glam::Mat4;

/// Sentinel value for [`MeshInstance::lod_force_level`] meaning
/// "no force — let the normal LOD selector decide". Stored as
/// `i32::MIN` so any sensible level (positive small int) cannot
/// collide with it.
pub const LOD_FORCE_NONE: i32 = i32::MIN;
/// This instance samples shadow maps.
///
/// Clear it and the shading path skips the fetch entirely — not a
/// cheaper fetch, no fetch. That cost is per pixel **and** per casting
/// light, which is why it is worth a bit (#804).
pub const INSTANCE_RECEIVES_SHADOWS: u32 = 1u32 << 0;

/// Per-instance scene record consumed by `cs_cull_scene`.
///
/// Layout (96 B, multiple of 16):
/// - `transform` (mat4, 64 B): world-space transform of this instance.
/// - `mesh_id` (u32): index into the global mesh pool (stub for 1.E.1b
///   — single-mesh path ignores this).
/// - `material_id` (u32): material pool index this instance shades against.
/// - `lod_bias` (f32): per-instance LOD bias for the screen-space-error
///   selector (1.E follow-up).
/// - `lod_force_level` (i32): when ≥ 0, the cull short-circuits the
///   LOD selector and emits only meshlets whose `lod_level` matches
///   this value. [`LOD_FORCE_NONE`] = normal selector. Drives the
///   side-by-side LOD inspector (#467).
/// - `group_base` (u32): per-instance prefix-sum base into the
///   `group_max_err` atomic buffer (#474). The shader resolves a
///   group's slot as `group_base + (m.group_index - mesh_desc.group_base)`,
///   which guarantees that two instances of the same mesh write to
///   disjoint slot ranges and pick LOD independently. `0` is valid
///   when the scene has at most one instance per mesh.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct MeshInstance {
    pub transform: [[f32; 4]; 4],
    pub mesh_id: u32,
    pub material_id: u32,
    pub lod_bias: f32,
    pub lod_force_level: i32,
    pub group_base: u32,
    /// Per-instance bits the shading path reads. See
    /// [`INSTANCE_RECEIVES_SHADOWS`].
    ///
    /// Was `_pad0`: same offset, same size, so the 96-byte stride every
    /// shader mirrors is unchanged. 🔴 Seven WGSL files declare this
    /// struct; one left behind reads the next field at the wrong offset
    /// and **does not fail to compile**.
    pub flags: u32,
    pub _pad1: u32,
    pub _pad2: u32,
}

impl MeshInstance {
    pub fn new(transform: Mat4, mesh_id: u32, material_id: u32) -> Self {
        Self {
            transform: transform.to_cols_array_2d(),
            mesh_id,
            material_id,
            lod_bias: 0.0,
            lod_force_level: LOD_FORCE_NONE,
            group_base: 0,
            // Receiving shadows is the default, so a mesh nobody
            // thought about looks the way it always has.
            flags: INSTANCE_RECEIVES_SHADOWS,
            _pad1: 0,
            _pad2: 0,
        }
    }

    /// Convenience: instance with the LOD selector overridden to
    /// emit only meshlets at `level`. Used by the editor's side-by-
    /// side LOD inspector to render each chain layer in isolation.
    pub fn with_lod_force_level(mut self, level: i32) -> Self {
        self.lod_force_level = level;
        self
    }

    pub fn transform_mat4(&self) -> Mat4 {
        Mat4::from_cols_array_2d(&self.transform)
    }
}

impl Default for MeshInstance {
    fn default() -> Self {
        Self::new(Mat4::IDENTITY, 0, 0)
    }
}

/// Per-frame scene parameters consumed by `cs_cull_scene` — the
/// per-meshlet `CullParams` already carries the camera state; this
/// adds the instance-count + per-mesh meshlet-count needed for the
/// 1D thread → (instance, meshlet) decoding.
///
/// Layout (16 B):
/// - `instance_count`: number of valid `MeshInstance` slots in the buffer.
/// - `meshlets_per_mesh`: meshlet count of the (single) mesh registered
///   for this scene. 1.E.1b replaces this with a per-mesh-id lookup.
/// - `_pad0`, `_pad1`: keep 16-byte alignment.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
pub struct SceneCullParams {
    pub instance_count: u32,
    pub meshlets_per_mesh: u32,
    pub _pad0: u32,
    pub _pad1: u32,
}

impl SceneCullParams {
    pub fn new(instance_count: u32, meshlets_per_mesh: u32) -> Self {
        Self {
            instance_count,
            meshlets_per_mesh,
            _pad0: 0,
            _pad1: 0,
        }
    }
}

/// Owns the scene-wide instance storage buffer + an upload helper.
///
/// Capacity starts at a construction-time guess and grows to fit via
/// [`MeshletScene::ensure_capacity`]; growth re-creates the buffer, which
/// is cheap next to the per-frame upload. Recreating it is safe because
/// every consumer builds its bind group per frame — nothing caches a
/// reference to the buffer across frames.
pub struct MeshletScene {
    instance_buffer: wgpu::Buffer,
    /// Each instance's transform **from the previous frame** (#481), as a
    /// flat array the motion-vector pass indexes with the same
    /// `inst_id`.
    ///
    /// 🔴 Parallel to the instances rather than a field inside them. The
    /// record is 96 bytes and six shaders mirror its layout; growing it
    /// would mean editing all six to add a matrix that exactly one of
    /// them reads. Separate arrays, indexed by the same id, is also what
    /// the engine's own data-oriented rule asks for.
    previous_transform_buffer: wgpu::Buffer,
    capacity: u32,
    bgl: wgpu::BindGroupLayout,
    /// Last frame's transform for each entity that had one.
    ///
    /// 🔴 Keyed by ENTITY, never by position. The instance vector is
    /// rebuilt from an ECS query every frame, so an entity appearing or
    /// changing archetype renumbers everything after it — index `i`
    /// simply is not the same object two frames running. Keyed by index,
    /// a reorder hands each instance somebody else's previous matrix and
    /// the motion vectors come out wrong with nothing failing.
    ///
    /// A map on the CPU, feeding a flat array on the GPU. The DOD rule
    /// bans hash lookups from hot paths that cross to the GPU; this one
    /// runs once per frame over the instance list to *build* that array,
    /// which is the streaming-and-coordination case it allows.
    previous_transforms: std::collections::HashMap<kooch_ecs::entity::Entity, [[f32; 4]; 4]>,
    /// Scratch for the upload, kept so the per-frame gather does not
    /// allocate.
    previous_scratch: Vec<[[f32; 4]; 4]>,
}

impl MeshletScene {
    /// Allocates an instance buffer sized for `capacity` slots.
    pub fn new(device: &wgpu::Device, capacity: u32) -> Self {
        assert!(capacity > 0, "MeshletScene capacity must be non-zero");
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_scene_instances"),
            size: capacity as u64 * std::mem::size_of::<MeshInstance>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let previous_transform_buffer = create_previous_buffer(device, capacity);
        let bgl = Self::bind_group_layout(device);
        Self {
            instance_buffer,
            previous_transform_buffer,
            capacity,
            bgl,
            previous_transforms: std::collections::HashMap::new(),
            previous_scratch: Vec::new(),
        }
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Grows the instance buffer to hold at least `required` slots.
    ///
    /// Construction-time capacity is a starting guess, not a contract: a
    /// scene is authored, not declared, and the renderer finds out how
    /// many instances it has when it walks the ECS. Until this existed
    /// the 257th mesh instance aborted the process — in the editor *and*
    /// in a shipped game, since both build the stage with the same
    /// default of 256.
    ///
    /// Geometric growth, matching
    /// [`MeshletCull::ensure_capacity`](crate::meshlet::MeshletCull::ensure_capacity):
    /// next power of two, and never less than double, so a scene that
    /// grows an instance at a time does not reallocate every frame.
    ///
    /// The old buffer is dropped rather than retired into a frame slot.
    /// This is called from `render()` **before** any command encoder for
    /// the frame binds it, so nothing in flight can be referencing it.
    pub fn ensure_capacity(&mut self, device: &wgpu::Device, required: u32) {
        if required <= self.capacity {
            return;
        }
        let new_capacity = required
            .checked_next_power_of_two()
            .unwrap_or(required)
            .max(self.capacity.saturating_mul(2));
        self.instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_scene_instances"),
            size: new_capacity as u64 * std::mem::size_of::<MeshInstance>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.previous_transform_buffer = create_previous_buffer(device, new_capacity);
        tracing::debug!(
            target: "kooch_render::meshlet::scene",
            from = self.capacity,
            to = new_capacity,
            required,
            "grew the instance buffer",
        );
        self.capacity = new_capacity;
    }

    pub fn previous_transform_buffer(&self) -> &wgpu::Buffer {
        &self.previous_transform_buffer
    }

    pub fn instance_buffer(&self) -> &wgpu::Buffer {
        &self.instance_buffer
    }

    /// Bind group layout for `cs_cull_scene` group(2): instance buffer +
    /// `SceneCullParams` UBO.
    pub fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("meshlet_scene_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: std::num::NonZeroU64::new(std::mem::size_of::<
                            SceneCullParams,
                        >()
                            as u64),
                    },
                    count: None,
                },
            ],
        })
    }

    /// Cached layout for the dispatcher's pipeline-layout building.
    pub fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.bgl
    }

    /// Uploads `instances[..]` into the GPU buffer (offset 0). Caller
    /// is responsible for keeping `instances.len() <= capacity`.
    pub fn upload_instances(&self, queue: &wgpu::Queue, instances: &[MeshInstance]) {
        self.upload_instance_data(queue, instances);
    }

    /// Uploads the instances **and** each one's transform from the
    /// previous frame, then remembers this frame's for the next one
    /// (#481).
    ///
    /// An entity seen for the first time gets its current transform as
    /// its previous one, which is a motion vector of zero. That is the
    /// right answer: an object that did not exist last frame has no
    /// history for a temporal pass to reproject, and claiming it moved
    /// from wherever the slot's last occupant was would smear it across
    /// the screen on its first frame.
    pub fn upload_instances_with_history(
        &mut self,
        queue: &wgpu::Queue,
        instances: &[MeshInstance],
        entities: &[kooch_ecs::entity::Entity],
    ) {
        debug_assert_eq!(instances.len(), entities.len());
        self.previous_scratch.clear();
        self.previous_scratch
            .extend(instances.iter().zip(entities).map(|(instance, entity)| {
                self.previous_transforms
                    .get(entity)
                    .copied()
                    .unwrap_or(instance.transform)
            }));
        if !self.previous_scratch.is_empty() {
            queue.write_buffer(
                &self.previous_transform_buffer,
                0,
                bytemuck::cast_slice(&self.previous_scratch),
            );
        }
        self.upload_instance_data(queue, instances);

        // Rebuilt rather than updated: an entity that stopped rendering
        // has to leave, or the map grows for the lifetime of the process
        // and a despawned object's matrix comes back if its entity id is
        // reused.
        self.previous_transforms.clear();
        self.previous_transforms.extend(
            entities
                .iter()
                .zip(instances)
                .map(|(entity, instance)| (*entity, instance.transform)),
        );
    }

    fn upload_instance_data(&self, queue: &wgpu::Queue, instances: &[MeshInstance]) {
        assert!(
            instances.len() as u32 <= self.capacity,
            "instance count {} exceeds scene capacity {}",
            instances.len(),
            self.capacity,
        );
        if instances.is_empty() {
            return;
        }
        queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(instances));
    }
}

/// Decodes the packed `(instance_id, meshlet_id)` value the scene cull
/// shader writes into `visible_meshlets`. CPU mirror of the WGSL
/// extract logic so tests can verify expected pairs without
/// reimplementing the bit math.
pub fn decode_scene_visible_id(packed: u32) -> (u32, u32) {
    // bit 16..32 = instance_id, bit 0..16 = meshlet_id
    (packed >> 16, packed & 0xFFFF)
}

/// Inverse of [`decode_scene_visible_id`]. Both must be < 0x1_0000.
pub fn encode_scene_visible_id(instance_id: u32, meshlet_id: u32) -> u32 {
    debug_assert!(instance_id < (1 << 16));
    debug_assert!(meshlet_id < (1 << 16));
    (instance_id << 16) | meshlet_id
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod capacity_tests;

fn create_previous_buffer(device: &wgpu::Device, capacity: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("meshlet_scene_previous_transforms"),
        size: capacity as u64 * std::mem::size_of::<[[f32; 4]; 4]>() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
