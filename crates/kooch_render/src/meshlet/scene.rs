//! Scene-wide GPU-driven meshlet pipeline state.

use bytemuck::{Pod, Zeroable};
use glam::Mat4;

/// Sentinel value for [`MeshInstance::lod_force_level`] meaning "no force — let the normal LOD
/// selector decide". Stored as `i32::MIN` so any sensible level (positive small int) cannot collide
/// with it.
pub const LOD_FORCE_NONE: i32 = i32::MIN;
/// This instance samples shadow maps.
pub const INSTANCE_RECEIVES_SHADOWS: u32 = 1u32 << 0;
/// Drawn by the forward pass, blended and sorted, and by no cull (#452). Such instances sit after
/// every opaque one; see [`opaque_count`].
pub const INSTANCE_TRANSPARENT: u32 = 1u32 << 1;

/// How many instances lead the list before the first transparent one: what every cull is given.
pub fn opaque_count(instances: &[MeshInstance]) -> usize {
    instances
        .iter()
        .position(|i| i.flags & INSTANCE_TRANSPARENT != 0)
        .unwrap_or(instances.len())
}

/// Per-instance scene record consumed by `cs_cull_scene`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct MeshInstance {
    pub transform: [[f32; 4]; 4],
    pub mesh_id: u32,
    pub material_id: u32,
    pub lod_bias: f32,
    pub lod_force_level: i32,
    pub group_base: u32,
    /// Per-instance bits the shading path reads. See `INSTANCE_RECEIVES_SHADOWS`.
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

    #[cfg(test)]
    pub fn transform_mat4(&self) -> Mat4 {
        Mat4::from_cols_array_2d(&self.transform)
    }
}

impl Default for MeshInstance {
    fn default() -> Self {
        Self::new(Mat4::IDENTITY, 0, 0)
    }
}

/// Per-frame scene parameters consumed by `cs_cull_scene` — the per-meshlet `CullParams` already
/// carries the camera state; this adds the instance-count + per-mesh meshlet-count needed for the
/// 1D thread → (instance, meshlet) decoding.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
pub struct SceneCullParams {
    pub instance_count: u32,
    pub meshlets_per_mesh: u32,
    /// LOD groups the scene actually has — `instance_group_capacity`'s O(1) prefix sum, not
    /// `instance_count * meshlets_per_mesh`.
    pub group_capacity: u32,
    /// Chunk slots the two-level cull's list holds (#1002).
    pub chunk_capacity: u32,
}

impl SceneCullParams {
    pub fn new(instance_count: u32, meshlets_per_mesh: u32) -> Self {
        Self {
            instance_count,
            meshlets_per_mesh,
            group_capacity: 0,
            chunk_capacity: 0,
        }
    }

    /// The scene's real LOD-group count, for whoever sizes an arena
    /// indexed by group.
    pub fn with_groups(mut self, groups: u32) -> Self {
        self.group_capacity = groups;
        self
    }

    /// The chunk list's capacity, which the two-level cull clamps to.
    pub fn with_chunks(mut self, chunks: u32) -> Self {
        self.chunk_capacity = chunks;
        self
    }
}

/// Owns the scene-wide instance storage buffer + an upload helper.
pub struct MeshletScene {
    instance_buffer: wgpu::Buffer,
    /// Each instance's transform **from the previous frame** (#481), as a flat array the
    /// motion-vector pass indexes with the same `inst_id`.
    previous_transform_buffer: wgpu::Buffer,
    capacity: u32,
    bgl: wgpu::BindGroupLayout,
    /// Last frame's transform for each entity that had one.
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

    /// Uploads the instances **and** each one's transform from the previous frame, then remembers
    /// this frame's for the next one (#481).
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

        // Rebuilt rather than updated: an entity that stopped rendering has to leave, or the map
        // grows for the lifetime of the process and a despawned object's matrix comes back if its
        // entity id is reused.
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

/// Decodes the packed `(instance_id, meshlet_id)` value the scene cull shader writes into
/// `visible_meshlets`. CPU mirror of the WGSL extract logic so tests can verify expected pairs
/// without reimplementing the bit math.
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
