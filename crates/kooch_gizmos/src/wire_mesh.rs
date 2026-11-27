//! Wire meshes that live on the GPU (#2030): a mesh is uploaded once under an id, posed per frame,
//! and removed by id. A frame where nothing changed uploads only the camera.

use std::collections::HashMap;
use std::mem::size_of;

use bytemuck::{Pod, Zeroable};
use glam::{Quat, Vec3};
use kooch_core::resource::Resources;
use kooch_render::VIEWPORT_DEPTH_FORMAT;

use crate::renderer::active_camera_view_proj;

const SHADER: &str = include_str!("../shaders/wire_mesh.wgsl");

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Default, PartialEq)]
struct MeshGpu {
    position: [f32; 3],
    /// rgba8; alpha 0 is a freed slot.
    color: u32,
    rotation: [f32; 4],
}

struct Slot {
    mesh: u32,
    corners: std::ops::Range<u32>,
}

/// The CPU mirror of the pool and what the GPU still lacks of it.
#[derive(Default)]
pub struct WireMeshes {
    slots: HashMap<u64, Slot>,
    positions: Vec<f32>,
    /// Three per triangle, indices into `positions / 3`.
    corners: Vec<u32>,
    /// One per triangle: its mesh.
    owners: Vec<u32>,
    meshes: Vec<MeshGpu>,
    /// Mesh slots no triangle points at any more; a removed mesh's slot waits for the compaction,
    /// or its dead triangles would draw with the next mesh's pose.
    free: Vec<u32>,
    /// Corners of removed meshes still in the buffers; past half, the pool is compacted.
    dead: usize,
    /// How much of `positions` and `corners` the GPU already holds.
    sent: (usize, usize),
    meshes_dirty: bool,
}

impl WireMeshes {
    /// Uploads a mesh under `id`, replacing one of the same id.
    pub fn insert(&mut self, id: u64, vertices: &[Vec3], triangles: &[[u32; 3]], color: [u8; 4]) {
        self.remove(id);
        let base = (self.positions.len() / 3) as u32;
        self.positions
            .extend(vertices.iter().flat_map(|v| v.to_array()));
        let mesh = self.free.pop().unwrap_or(self.meshes.len() as u32);
        let start = self.corners.len() as u32;
        let count = vertices.len() as u32;
        for triangle in triangles.iter().filter(|t| t.iter().all(|&i| i < count)) {
            self.corners.extend(triangle.iter().map(|&i| base + i));
            self.owners.push(mesh);
        }
        let entry = MeshGpu {
            position: [0.0; 3],
            color: u32::from_le_bytes(color),
            rotation: Quat::IDENTITY.to_array(),
        };
        match self.meshes.get_mut(mesh as usize) {
            Some(slot) => *slot = entry,
            None => self.meshes.push(entry),
        }
        self.meshes_dirty = true;
        let corners = start..self.corners.len() as u32;
        self.slots.insert(id, Slot { mesh, corners });
    }

    /// Drops a mesh; its triangles stop drawing at once and are reclaimed by the next compaction.
    pub fn remove(&mut self, id: u64) {
        let Some(slot) = self.slots.remove(&id) else {
            return;
        };
        if let Some(mesh) = self.meshes.get_mut(slot.mesh as usize) {
            mesh.color = 0;
        }
        self.dead += slot.corners.len();
        self.meshes_dirty = true;
        if self.dead * 2 > self.corners.len() {
            self.compact();
        }
    }

    /// Places a mesh in the frame's space.
    pub fn pose(&mut self, id: u64, position: Vec3, rotation: Quat) {
        let Some(slot) = self.slots.get(&id) else {
            return;
        };
        let Some(mesh) = self.meshes.get_mut(slot.mesh as usize) else {
            return;
        };
        let (position, rotation) = (position.to_array(), rotation.to_array());
        if mesh.position != position || mesh.rotation != rotation {
            mesh.position = position;
            mesh.rotation = rotation;
            self.meshes_dirty = true;
        }
    }

    /// Whether `id` is held.
    pub fn contains(&self, id: u64) -> bool {
        self.slots.contains_key(&id)
    }

    /// Whether nothing is held.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Ids held now.
    pub fn ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.slots.keys().copied()
    }

    /// Removes every mesh.
    pub fn clear(&mut self) {
        *self = Self::default();
        self.meshes_dirty = true;
    }

    fn compact(&mut self) {
        let mut positions = Vec::new();
        let mut corners = Vec::new();
        let mut owners = Vec::new();
        for slot in self.slots.values_mut() {
            let start = corners.len() as u32;
            let mut remap: HashMap<u32, u32> = HashMap::new();
            let range = slot.corners.start as usize..slot.corners.end as usize;
            for &old in self.corners.get(range).unwrap_or_default() {
                let new = *remap.entry(old).or_insert_with(|| {
                    let at = old as usize * 3;
                    positions
                        .extend_from_slice(self.positions.get(at..at + 3).unwrap_or(&[0.0; 3]));
                    (positions.len() / 3 - 1) as u32
                });
                corners.push(new);
            }
            owners.extend(std::iter::repeat_n(
                slot.mesh,
                (corners.len() - start as usize) / 3,
            ));
            slot.corners = start..corners.len() as u32;
        }
        self.positions = positions;
        self.corners = corners;
        self.owners = owners;
        let live: std::collections::HashSet<u32> = self.slots.values().map(|s| s.mesh).collect();
        self.free = (0..self.meshes.len() as u32)
            .filter(|m| !live.contains(m))
            .collect();
        self.dead = 0;
        self.sent = (0, 0);
    }
}

/// Draws [`WireMeshes`] in one call, edges only, on top of the scene.
pub struct WireMeshRenderer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    camera: wgpu::Buffer,
    positions: wgpu::Buffer,
    corners: wgpu::Buffer,
    owners: wgpu::Buffer,
    meshes: wgpu::Buffer,
    bind_group: Option<wgpu::BindGroup>,
}

impl WireMeshRenderer {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("wire_mesh_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("wire_mesh_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage(1),
                storage(2),
                storage(3),
                storage(4),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("wire_mesh_pipeline_layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("wire_mesh_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            // On top, like the physics lines: a collider lies on the surface it was built from.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: VIEWPORT_DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: pipeline_cache,
        });
        let buffer = |label, size, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let storage = wgpu::BufferUsages::STORAGE;
        Self {
            pipeline,
            layout,
            camera: buffer("wire_mesh_camera", 64, wgpu::BufferUsages::UNIFORM),
            positions: buffer("wire_mesh_positions", 4096, storage),
            corners: buffer("wire_mesh_corners", 4096, storage),
            owners: buffer("wire_mesh_owners", 4096, storage),
            meshes: buffer("wire_mesh_meshes", 4096, storage),
            bind_group: None,
        }
    }

    /// Sends what changed in `pool` and draws it into `view`.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        resources: &Resources,
        pool: &mut WireMeshes,
        viewport_size: (u32, u32),
    ) {
        if viewport_size.0 == 0 || viewport_size.1 == 0 {
            return;
        }
        let aspect = viewport_size.0 as f32 / viewport_size.1 as f32;
        let Some(view_proj) = active_camera_view_proj(resources, aspect) else {
            return;
        };
        self.render_as(device, queue, encoder, view, depth, pool, view_proj);
    }

    /// The same, seen from `view_proj` rather than the active camera: a view with its own camera.
    #[allow(clippy::too_many_arguments)]
    pub fn render_as(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        pool: &mut WireMeshes,
        view_proj: glam::Mat4,
    ) {
        if pool.slots.is_empty() {
            return;
        }
        queue.write_buffer(
            &self.camera,
            0,
            bytemuck::bytes_of(&view_proj.to_cols_array()),
        );
        self.upload(device, queue, pool);
        let Some(bind_group) = &self.bind_group else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("wire_mesh_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..pool.corners.len() as u32, 0..1);
    }

    /// Appends what the GPU lacks; a buffer that outgrew its size is rebuilt whole.
    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, pool: &mut WireMeshes) {
        let mut rebind = self.bind_group.is_none();
        rebind |= fit(
            device,
            &mut self.positions,
            bytes(&pool.positions),
            &mut pool.sent.0,
        );
        let before = pool.sent.1;
        rebind |= fit(
            device,
            &mut self.corners,
            bytes(&pool.corners),
            &mut pool.sent.1,
        );
        let mut owners_sent = before / 3;
        rebind |= fit(
            device,
            &mut self.owners,
            bytes(&pool.owners),
            &mut owners_sent,
        );
        let (pool_positions, pool_corners) = (pool.positions.len(), pool.corners.len());
        write_tail(
            queue,
            &self.positions,
            &pool.positions,
            pool.sent.0,
            pool_positions,
        );
        write_tail(
            queue,
            &self.corners,
            &pool.corners,
            pool.sent.1,
            pool_corners,
        );
        write_tail(
            queue,
            &self.owners,
            &pool.owners,
            owners_sent,
            pool.owners.len(),
        );
        pool.sent = (pool_positions, pool_corners);
        if fit(device, &mut self.meshes, bytes(&pool.meshes), &mut 0) {
            rebind = true;
            pool.meshes_dirty = true;
        }
        if pool.meshes_dirty {
            queue.write_buffer(&self.meshes, 0, bytemuck::cast_slice(&pool.meshes));
            pool.meshes_dirty = false;
        }
        if rebind {
            self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("wire_mesh_bg"),
                layout: &self.layout,
                entries: &[
                    entry(0, &self.camera),
                    entry(1, &self.positions),
                    entry(2, &self.corners),
                    entry(3, &self.owners),
                    entry(4, &self.meshes),
                ],
            }));
        }
    }
}

fn bytes<T>(items: &[T]) -> u64 {
    (items.len() * size_of::<T>()) as u64
}

/// Grows `buffer` to the next power of two past `needed`, resetting `sent` so all of it goes again.
fn fit(device: &wgpu::Device, buffer: &mut wgpu::Buffer, needed: u64, sent: &mut usize) -> bool {
    if needed <= buffer.size() {
        return false;
    }
    *buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("wire_mesh_grown"),
        size: needed.next_power_of_two(),
        usage: buffer.usage(),
        mapped_at_creation: false,
    });
    *sent = 0;
    true
}

fn write_tail<T: Pod>(
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    items: &[T],
    from: usize,
    to: usize,
) {
    if let Some(tail) = items.get(from..to).filter(|tail| !tail.is_empty()) {
        let offset = (from * size_of::<T>()) as u64;
        queue.write_buffer(buffer, offset, bytemuck::cast_slice(tail));
    }
}

fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}
