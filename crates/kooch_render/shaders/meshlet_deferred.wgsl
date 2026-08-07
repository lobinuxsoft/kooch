// meshlet_deferred.wgsl — visibility-buffer compute shading, the
// fallback path for adapters without 64-bit texture atomics (Metal /
// MSL has no `atomic_uint64`, and pre-baseline adapters lack the
// feature).
//
// CONCATENATED in Rust after `surface_reconstruct.wgsl` (geometry
// bindings on groups 1 and 3, barycentric reconstruction) and
// `inti_pbr.wgsl` (the shading model, group 4). What this file owns is
// the R32Uint visibility-buffer read and the two entry points.
//
// Two entry points:
//
// (cs_shade) — single-mesh path (Phase 1.D):
//   Reads packed `((meshlet_id+1) << 7) | tri_idx` from the visibility
//   buffer. Uses the per-call `model: ModelUniforms` for the world-space
//   normal transform. `screen.material_id` is per-render-call. Kept on
//   the averaged-normal reconstruction because it has no instance
//   buffer to resolve against; it survives for the single-mesh tests
//   and is not a production path.
//
// (cs_shade_scene) — scene-wide path (Phase 1.E):
//   Reads packed `((visible_slot+1) << 7) | tri_idx`, then reconstructs
//   the full surface through the same barycentric path the R64 route
//   uses and shades it with Inti.
//
// # The +1
//
// This path clears the visibility buffer to zero and treats zero as
// background, so every written id is biased by one. The R64 path uses
// the depth bits for that instead, which is why the two decodes differ
// by exactly this and nothing else.

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
    // x metallic, y roughness, z emissive, w pad.
    metallic_roughness_emissive_pad: vec4<f32>,
    // albedo, normal, metal_roughness pool indices + pad.
    // 0xffffffff = no map (fall back to scalars). Only the two-pass
    // fragment path samples them; this path uses the scalars.
    texture_indices: vec4<u32>,
}

@group(0) @binding(0) var<uniform> camera: CameraUniforms;
@group(0) @binding(1) var<uniform> model: ModelUniforms;
@group(0) @binding(2) var<uniform> screen: ScreenUniforms;
@group(0) @binding(3) var vis_buffer: texture_2d<u32>;
@group(0) @binding(4) var color_out: texture_storage_2d<rgba8unorm, write>;

@group(2) @binding(0) var<storage, read> materials: array<MaterialParams>;

// `MeshletDebugMode::Normals`. Pinned to the Rust enum by a test in
// `debug.rs`; WGSL cannot import a Rust constant.
const DEBUG_MODE_NORMALS: u32 = 11u;
// `MeshletDebugMode::ShadowCascades`, pinned by the same test.
const DEBUG_MODE_SHADOW_CASCADES: u32 = 12u;
// `MeshletDebugMode::ContactShadows`, pinned by the same test.
const DEBUG_MODE_CONTACT_SHADOWS: u32 = 13u;

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

// Single-mesh path's own corner fetch: it addresses descriptors by
// meshlet id directly and has no instance transform, so it cannot go
// through `resolve_surface`.
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

    // Alpha = 0 is the background sentinel: the blit composes this
    // texture over whatever was rendered before (sky / clear), so
    // pixels untouched by any meshlet must not overwrite it.
    var color = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    if (packed != 0u) {
        let meshlet_id = (packed >> 7u) - 1u;
        let tri_idx = packed & 0x7Fu;
        let desc = descriptors[meshlet_id];

        let n0 = corner_normal(desc, tri_idx, 0u);
        let n1 = corner_normal(desc, tri_idx, 1u);
        let n2 = corner_normal(desc, tri_idx, 2u);
        let avg = (n0 + n1 + n2) / 3.0;
        let world_n = (model.model * vec4<f32>(avg, 0.0)).xyz;
        let n = normalize(world_n);

        let m = materials[screen.material_id];
        // Force alpha = 1 for any pixel covered by a meshlet — the
        // alpha = 0 sentinel is reserved for the background-pass-through
        // path the blit composes.
        color = vec4<f32>((n * 0.5 + 0.5) * m.base_color.rgb, 1.0);
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

    var color = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    if (packed != 0u) {
        let visible_slot = (packed >> 7u) - 1u;
        let tri_idx = packed & 0x7Fu;

        // Debug-mode short-circuit: the id-colorize modes paint the
        // whole meshlet flat, so they skip the surface reconstruction
        // entirely rather than compute attributes nothing reads.
        var rgb: vec3<f32>;
        if (screen.debug_mode == 1u || screen.debug_mode == 2u) {
            let packed_visible = visible_meshlets[visible_slot];
            let inst_id = packed_visible >> 16u;
            let meshlet_id = packed_visible & 0xffffu;
            // MeshletIds uses the global descriptor index so two
            // instances of the same mesh share the cluster colour (the
            // cluster identity is what is being visualized);
            // InstanceIds colours by entity coverage.
            rgb = select(hash_to_rgb(inst_id), hash_to_rgb(meshlet_id), screen.debug_mode == 1u);
        } else if (screen.debug_mode == 7u) {
            // CullPassthrough — any pixel reaching this branch is one
            // the meshlet pipeline classified as visible (passed cull,
            // landed in the vbuf, won its atomicMax). Flat green.
            rgb = vec3<f32>(0.0, 1.0, 0.0);
        } else {
            // The same reconstruction the R64 path runs. Before #441
            // this branch averaged the triangle's three vertex normals
            // and had no world position at all — which was invisible
            // while shading was a function of the normal alone, and
            // would have lit every triangle's centroid the moment a
            // point light needed a distance.
            let surf = resolve_surface(visible_slot, tri_idx, vec2<f32>(pixel) + vec2<f32>(0.5));
            let n = normalize(surf.world_normal);

            if (screen.debug_mode == DEBUG_MODE_NORMALS) {
                rgb = n * 0.5 + 0.5;
            } else if (screen.debug_mode == DEBUG_MODE_SHADOW_CASCADES) {
                let vd = dot(
                    surf.world_position - inti.camera_position, inti.camera_forward);
                rgb = inti_shadow_debug(surf.world_position, vd);
            } else if (screen.debug_mode == DEBUG_MODE_CONTACT_SHADOWS) {
                rgb = inti_contact_shadow_debug_view(
                    surf.world_position, n, vec2<f32>(pixel) + vec2<f32>(0.5));
            } else {
                let m = materials[surf.material_id];
                // No texture sampling on this path: a compute shader
                // has no implicit derivatives, and the analytical ones
                // exist to feed `textureSampleGrad`, which is a
                // fragment-stage call. Scalars only — the R64 fragment
                // path is where maps land.
                var radiance = inti_shade(
                    surf.world_position,
                    n,
                    m.base_color.rgb,
                    m.metallic_roughness_emissive_pad.x,
                    m.metallic_roughness_emissive_pad.y,
                    // Pixel centre — the contact-shadow jitter wants the
                    // same coordinate the fragment path passes, so the
                    // two paths dither identically.
                    vec2<f32>(pixel) + vec2<f32>(0.5),
                );
                radiance += m.base_color.rgb * m.metallic_roughness_emissive_pad.z;
                rgb = inti_tonemap(radiance);
            }
        }

        color = vec4<f32>(rgb, 1.0);
    }

    textureStore(color_out, vec2<i32>(i32(pixel.x), i32(pixel.y)), color);
}
