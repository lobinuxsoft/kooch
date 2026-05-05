// meshlet_deferred.wgsl — visibility-buffer compute shading.
//
// Two entry points:
//
// (cs_shade) — single-mesh path (Phase 1.D):
//   Reads packed `((meshlet_id+1) << 7) | tri_idx` from the visibility
//   buffer. Uses the per-call `model: ModelUniforms` for world-space
//   normal transform. `screen.material_id` is per-render-call.
//
// (cs_shade_scene) — scene-wide path (Phase 1.E):
//   Reads packed `((visible_slot+1) << 7) | tri_idx` from the
//   visibility buffer. Resolves (instance_id, meshlet_idx) via
//   `visible_meshlets[visible_slot]`, then fetches per-instance
//   transform + material_id from `instances[]`. Background pixels
//   keep the clear color.
//
// Both paths average the triangle's three vertex normals and modulate
// the resulting normal-debug shading by the material's base colour.
// Bary-correct interpolation lands when textures need real UV interp.

struct CameraUniforms {
    view_proj: mat4x4<f32>,
}

struct ModelUniforms {
    model: mat4x4<f32>,
}

struct ScreenUniforms {
    size: vec2<u32>,
    material_id: u32,
    // Debug visualization mode (#451). Stable u32 discriminants from
    // MeshletDebugMode in Rust; 0 = Off (production path).
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
    _pad0: u32,
    aabb_max: vec3<f32>,
    _pad1: u32,
    bounds_center: vec3<f32>,
    bounding_radius: f32,
    cone_apex: vec3<f32>,
    cone_cutoff: f32,
    cone_axis: vec3<f32>,
    _pad2: u32,
}

@group(0) @binding(0) var<uniform> camera: CameraUniforms;
@group(0) @binding(1) var<uniform> model: ModelUniforms;
@group(0) @binding(2) var<uniform> screen: ScreenUniforms;
@group(0) @binding(3) var vis_buffer: texture_2d<u32>;
@group(0) @binding(4) var color_out: texture_storage_2d<rgba8unorm, write>;

@group(2) @binding(0) var<storage, read> materials: array<MaterialParams>;

@group(1) @binding(0) var<storage, read> vertices: array<MeshVertexStored>;
@group(1) @binding(1) var<storage, read> meshlet_vertices: array<u32>;
@group(1) @binding(2) var<storage, read> meshlet_triangles: array<u32>;
@group(1) @binding(3) var<storage, read> descriptors: array<MeshletDescriptor>;

// Scene-path bindings (group 3) — only bound for cs_shade_scene.
struct MeshInstance {
    transform: mat4x4<f32>,
    mesh_id: u32,
    material_id: u32,
    lod_bias: f32,
    _pad: u32,
}
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

// PCG-style hash → vec3 rgb in [0.2, 1.0]. The 0.2 floor keeps any
// id from collapsing to black (which the alpha=0 background uses).
// Same constants as Bevy's meshlet_visualizer; adequate for visual
// distinguishability at cluster / instance scale.
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

@compute @workgroup_size(8, 8, 1)
fn cs_shade(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= screen.size.x || gid.y >= screen.size.y) {
        return;
    }
    let pixel = vec2<u32>(gid.x, gid.y);
    let packed = textureLoad(vis_buffer, pixel, 0).r;

    // Alpha = 0 is the background sentinel: the blit composes this
    // texture over whatever was rendered before (sky / clear), so
    // pixels untouched by any meshlet must not overwrite it.
    var color = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    if (packed != 0u) {
        let meshlet_id = (packed >> 7u) - 1u;
        let tri_idx = packed & 0x7Fu;
        let desc = descriptors[meshlet_id];

        // Average the triangle's three vertex normals — visually
        // identical to flat-shaded forward output. Bary-correct
        // interpolation lands when materials need real UV interp
        // (texture-mapped PBR follow-up).
        let n0 = corner_normal(desc, tri_idx, 0u);
        let n1 = corner_normal(desc, tri_idx, 1u);
        let n2 = corner_normal(desc, tri_idx, 2u);
        let avg = (n0 + n1 + n2) / 3.0;
        let world_n = (model.model * vec4<f32>(avg, 0.0)).xyz;
        let n = normalize(world_n);

        // PR-7: modulate normal-debug shading by the material's base
        // colour. Materials pool is indexed via `screen.material_id`
        // (per-render-call assignment); per-meshlet material ids land
        // with bindless textures in a follow-up.
        let normal_debug = n * 0.5 + 0.5;
        let m = materials[screen.material_id];
        // Force alpha = 1 for any pixel covered by a meshlet — the
        // alpha = 0 sentinel is reserved for the background-pass-through
        // path the blit composes. Material's own alpha will land back
        // here once translucency / cutout is wired up.
        color = vec4<f32>(normal_debug * m.base_color.rgb, 1.0);
    }

    textureStore(color_out, vec2<i32>(i32(pixel.x), i32(pixel.y)), color);
}

@compute @workgroup_size(8, 8, 1)
fn cs_shade_scene(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= screen.size.x || gid.y >= screen.size.y) {
        return;
    }
    let pixel = vec2<u32>(gid.x, gid.y);
    let packed = textureLoad(vis_buffer, pixel, 0).r;

    // Alpha = 0 is the background sentinel: the blit composes this
    // texture over whatever was rendered before (sky / clear), so
    // pixels untouched by any meshlet must not overwrite it.
    var color = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    if (packed != 0u) {
        // Decode visible-slot index (vbuf scene path packs +1 to keep
        // 0 = background). One indirection through visible_meshlets[]
        // recovers (instance_id, meshlet_idx).
        let visible_slot = (packed >> 7u) - 1u;
        let tri_idx = packed & 0x7Fu;
        let packed_visible = visible_meshlets[visible_slot];
        let inst_id = packed_visible >> 16u;
        let meshlet_id = packed_visible & 0xffffu;

        // Debug-mode short-circuit: skip the per-triangle normal fetch
        // when the mode does not need it. Cluster/instance id paint
        // the entire meshlet a flat colour so the cluster boundaries
        // are the dominant visual feature.
        var rgb: vec3<f32>;
        if (screen.debug_mode == 1u) {
            // MeshletIds — use the global meshlet descriptor index so
            // two instances of the same mesh share the cluster colour
            // (the cluster identity is what's being visualized).
            rgb = hash_to_rgb(meshlet_id);
        } else if (screen.debug_mode == 2u) {
            // InstanceIds — colour by entity coverage.
            rgb = hash_to_rgb(inst_id);
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

        // Force alpha = 1 for any pixel covered by a meshlet — the
        // alpha = 0 sentinel is reserved for the background-pass-through
        // path the blit composes.
        color = vec4<f32>(rgb, 1.0);
    }

    textureStore(color_out, vec2<i32>(i32(pixel.x), i32(pixel.y)), color);
}
