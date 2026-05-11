// meshlet_deferred_r64.wgsl — visibility-buffer compute shading for the
// atomic R64 path (#493).
//
// Reads packed `(depth_bits << 32) | (visible_slot << 7 | tri_idx)` from
// the storage R64 vbuf, ignores the depth half (already used at raster
// time to win the atomicMax), and resolves
// `(instance_id, meshlet_idx)` via `visible_meshlets[]`. Sentinel for
// "no fragment" is `packed == 0`, matching reversed-Z far plane + cleared
// vbuf; mirrors Bevy's resolve_render_targets convention.

struct CameraUniforms {
    view_proj: mat4x4<f32>,
}

struct ScreenUniforms {
    size: vec2<u32>,
    material_id: u32,
    debug_mode: u32,
}

struct MaterialParams {
    base_color: vec4<f32>,
    metallic_roughness_emissive_pad: vec4<f32>,
}

struct MeshVertexStored {
    position: array<f32, 3>,
    normal: array<f32, 3>,
    uv: array<f32, 2>,
}

struct MeshletDescriptor {
    vertex_offset: u32,
    triangle_offset: u32,
    vertex_count: u32,
    triangle_count: u32,
    aabb_min: vec3<f32>,
    parent_meshlet_index: u32,
    aabb_max: vec3<f32>,
    lod_error: f32,
    bounds_center: vec3<f32>,
    bounding_radius: f32,
    cone_apex: vec3<f32>,
    cone_cutoff: f32,
    cone_axis: vec3<f32>,
    group_index: u32,
    children_group_index: u32,
    lod_level: u32,
    _pad4: u32,
    _pad5: u32,
}

struct MeshInstance {
    transform: mat4x4<f32>,
    mesh_id: u32,
    material_id: u32,
    lod_bias: f32,
    lod_force_level: i32,
    group_base: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0) var<uniform> camera: CameraUniforms;
@group(0) @binding(1) var<uniform> screen: ScreenUniforms;
@group(0) @binding(2) var vbuf64: texture_storage_2d<r64uint, atomic>;
@group(0) @binding(4) var density_accumulator: texture_storage_2d<r32uint, atomic>;
@group(0) @binding(3) var color_out: texture_storage_2d<rgba8unorm, write>;

@group(2) @binding(0) var<storage, read> materials: array<MaterialParams>;

@group(1) @binding(0) var<storage, read> vertices: array<MeshVertexStored>;
@group(1) @binding(1) var<storage, read> meshlet_vertices: array<u32>;
@group(1) @binding(2) var<storage, read> meshlet_triangles: array<u32>;
@group(1) @binding(3) var<storage, read> descriptors: array<MeshletDescriptor>;

@group(3) @binding(0) var<storage, read> visible_meshlets: array<u32>;
@group(3) @binding(1) var<storage, read> instances: array<MeshInstance>;

fn fetch_local_vertex_index(byte_offset: u32) -> u32 {
    let word_idx = byte_offset / 4u;
    let byte_in_word = byte_offset & 3u;
    let packed = meshlet_triangles[word_idx];
    return (packed >> (byte_in_word * 8u)) & 0xffu;
}

fn corner_normal(desc: MeshletDescriptor, tri_idx: u32, corner: u32) -> vec3<f32> {
    let byte_offset = desc.triangle_offset + tri_idx * 3u + corner;
    let local = fetch_local_vertex_index(byte_offset);
    let global = meshlet_vertices[desc.vertex_offset + local];
    let v = vertices[global];
    return vec3<f32>(v.normal[0], v.normal[1], v.normal[2]);
}

fn hash_to_rgb(x: u32) -> vec3<f32> {
    var h = x;
    h ^= h >> 16u;
    h = h * 0x7feb352du;
    h ^= h >> 15u;
    h = h * 0x846ca68bu;
    h ^= h >> 16u;
    let r = f32(h & 0xffu) / 255.0;
    let g = f32((h >> 8u) & 0xffu) / 255.0;
    let b = f32((h >> 16u) & 0xffu) / 255.0;
    return vec3<f32>(r, g, b) * 0.8 + 0.2;
}

// 5-stop perceptual gradient for the density / overdraw heatmaps:
//   t = 0.00 → black     (background / nothing rasterized here)
//   t = 0.25 → blue      (sparse — single fragment)
//   t = 0.50 → green     (moderate — small handful of clusters)
//   t = 0.75 → yellow    (busy — many overlapping clusters)
//   t = 1.00 → red       (saturated — sub-pixel triangle territory)
//
// Polynomial form ties each channel to where it should peak so the
// transition is smooth and bandlimited. Good enough for diagnostic
// overlays — fancier perceptual LUTs (turbo, viridis) are not worth
// the extra ALU on a debug-only path.
fn density_heatmap(t: f32) -> vec3<f32> {
    let r = clamp(2.0 * t - 1.0, 0.0, 1.0);
    let g = clamp(1.0 - 2.0 * abs(t - 0.5), 0.0, 1.0);
    let b = clamp(1.0 - 2.0 * t, 0.0, 1.0);
    return vec3<f32>(r, g, b);
}

@compute @workgroup_size(8, 8, 1)
fn cs_shade_scene_r64(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= screen.size.x || gid.y >= screen.size.y) {
        return;
    }
    let pixel = vec2<u32>(gid.x, gid.y);
    let packed = textureLoad(vbuf64, pixel).x;

    var color = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    if (packed != 0lu) {
        // Discard the depth half — it served its purpose winning the
        // atomicMax at raster time. The id half carries
        // `(visible_slot << 7) | tri_idx`. Unlike the R32 path the
        // visible_slot is NOT offset by +1 (we use depth==0 as the
        // background sentinel, mirroring Bevy).
        let packed_ids = u32(packed);
        let visible_slot = packed_ids >> 7u;
        let tri_idx = packed_ids & 0x7Fu;

        let packed_visible = visible_meshlets[visible_slot];
        let inst_id = packed_visible >> 16u;
        let meshlet_id = packed_visible & 0xffffu;

        var rgb: vec3<f32>;
        if (screen.debug_mode == 1u) {
            rgb = hash_to_rgb(meshlet_id);
        } else if (screen.debug_mode == 2u) {
            rgb = hash_to_rgb(inst_id);
        } else if (screen.debug_mode == 3u) {
            // TriangleDensity — colour-by-contribution heatmap.
            // The accumulator was zeroed at frame start and the
            // vbuf64 fragment did one atomicAdd per contributing
            // cluster fragment, so `count` is the per-pixel
            // contribution count. Saturate at MAX_DENSITY so the
            // gradient covers a usable range — anything brighter
            // than the warm end is sub-pixel triangle territory and
            // a calibration signal for the LOD `target_error_pixels`
            // knob.
            let count = textureLoad(density_accumulator, pixel).x;
            const MAX_DENSITY: f32 = 32.0;
            let t = clamp(f32(count) / MAX_DENSITY, 0.0, 1.0);
            rgb = density_heatmap(t);
        } else if (screen.debug_mode == 7u) {
            // CullPassthrough — flat green for every vbuf-covered
            // pixel. See meshlet_deferred.wgsl for the rationale.
            rgb = vec3<f32>(0.0, 1.0, 0.0);
        } else {
            let inst = instances[inst_id];
            let desc = descriptors[meshlet_id];

            let n0 = corner_normal(desc, tri_idx, 0u);
            let n1 = corner_normal(desc, tri_idx, 1u);
            let n2 = corner_normal(desc, tri_idx, 2u);
            let avg = (n0 + n1 + n2) / 3.0;
            let world_n = (inst.transform * vec4<f32>(avg, 0.0)).xyz;
            let n = normalize(world_n);

            let normal_debug = n * 0.5 + 0.5;
            let m = materials[inst.material_id];
            rgb = normal_debug * m.base_color.rgb;
        }

        color = vec4<f32>(rgb, 1.0);
    }

    textureStore(color_out, vec2<i32>(i32(pixel.x), i32(pixel.y)), color);
}
