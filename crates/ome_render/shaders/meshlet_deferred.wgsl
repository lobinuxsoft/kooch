// meshlet_deferred.wgsl — visibility-buffer compute shading.
//
// One thread per pixel of the output target. Reads the packed
// (meshlet_id+1, triangle_id) from the visibility buffer; if 0 the
// pixel is background and gets the clear color. Otherwise the
// shader re-derives the triangle's three vertices, averages their
// world-space normals, and emits the same normal-debug RGB the
// forward path would.
//
// Bary-correct interpolation lands in PR-7 with materials. PR-6
// stays minimal: the asserts target shading parity (within a tight
// tolerance) between forward-rasterized and deferred-shaded cubes.

struct CameraUniforms {
    view_proj: mat4x4<f32>,
}

struct ModelUniforms {
    model: mat4x4<f32>,
}

struct ScreenUniforms {
    size: vec2<u32>,
    material_id: u32,
    _pad: u32,
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

@compute @workgroup_size(8, 8, 1)
fn cs_shade(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= screen.size.x || gid.y >= screen.size.y) {
        return;
    }
    let pixel = vec2<u32>(gid.x, gid.y);
    let packed = textureLoad(vis_buffer, pixel, 0).r;

    var color = vec4<f32>(0.0, 0.0, 0.0, 1.0);
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
        color = vec4<f32>(normal_debug * m.base_color.rgb, m.base_color.a);
    }

    textureStore(color_out, vec2<i32>(i32(pixel.x), i32(pixel.y)), color);
}
