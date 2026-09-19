//! Where the sun's clipmap levels sit, and the atlas they draw into.

use super::*;

/// The sun's frame: right, up, and the direction it shines along.
pub(super) fn sun_frame(sun: Vec3) -> (Vec3, Vec3, Vec3) {
    let f = sun.normalize_or(Vec3::NEG_Y);
    let up = if f.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
    let right = f.cross(up).normalize();
    (right, right.cross(f), f)
}

/// The world point a level's cull box is centred on.
pub(super) fn level_origin(base: f32, side: u32, level: u32, eye: Vec3, sun: Vec3) -> Vec3 {
    let (right, up, f) = sun_frame(sun);
    let s = side.max(1) as f32;
    let width = base * (level as f32).exp2() / s;
    let plane = glam::Vec2::new(eye.dot(right), eye.dot(up));
    let low = (plane / width).floor() - glam::Vec2::splat((s * 0.5).floor());
    let centre = (low + glam::Vec2::splat(s * 0.5)) * width;
    // ⚠️ `floor(x + 0.5)` and never `round`, mirroring `sun_drift`:
    // WGSL rounds halves to even and Rust rounds them away from zero.
    let along = (eye.dot(f) / width + 0.5).floor() * width;
    right * centre.x + up * centre.y + f * along
}

/// The clipmap level's orthographic clip-from-world.
pub(super) fn level_clip(
    clipmap: ClipmapConfig,
    side: u32,
    level: u32,
    eye: Vec3,
    sun: Vec3,
) -> Mat4 {
    let (right, up, f) = sun_frame(sun);
    let rotation = Mat4::from_cols(
        glam::Vec4::new(right.x, up.x, f.x, 0.0),
        glam::Vec4::new(right.y, up.y, f.y, 0.0),
        glam::Vec4::new(right.z, up.z, f.z, 0.0),
        glam::Vec4::W,
    );
    let half = clipmap.extent(level) * 0.5;
    // Reversed-Z orthographic: 1 at the near plane, 0 at the far,
    // matching `page_depth.wgsl`'s `1 - (z + span) / (2 * span)`.
    let projection = Mat4::from_cols(
        glam::Vec4::new(1.0 / half, 0.0, 0.0, 0.0),
        glam::Vec4::new(0.0, 1.0 / half, 0.0, 0.0),
        glam::Vec4::new(0.0, 0.0, -1.0 / (2.0 * SUN_SPAN), 0.0),
        glam::Vec4::new(0.0, 0.0, 0.5, 1.0),
    );
    projection
        * rotation
        * Mat4::from_translation(-level_origin(clipmap.base, side, level, eye, sun))
}

/// The atlas: one square layer per camera.
pub(super) fn atlas_texture(
    device: &wgpu::Device,
    config: PageConfig,
    pool: PoolConfig,
) -> wgpu::Texture {
    let side = pool.per_row() * config.page;
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shadow_page_atlas"),
        size: wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: pool.layers(),
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: PAGE_DEPTH_FORMAT,
        // COPY_SRC is for the end-to-end rig (`a_lamp_page_holds_what_its_light_sees`), which reads
        // pages back and checks them against the scene — the class of defect that until then was
        // only ever caught by a person staring at a broken frame.
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// The content stamp of every sun clipmap level, for the cache gate.
pub(super) fn sun_gens(
    clipmap: ClipmapConfig,
    side: f32,
    scene_gen: u32,
    eye: Vec3,
    sun: Vec3,
) -> Vec<u32> {
    // Mirrors `sun_basis` in `page_table.wgsl`, term for term.
    let f = sun.normalize_or(Vec3::NEG_Y);
    // Only the sun's own axis is left: the basis' two in-plane vectors
    // went with the snapped centre this no longer hashes.
    let along = eye.dot(f);
    (0..clipmap.levels as usize)
        .map(|level| {
            let width = clipmap.base * (level as f32).exp2() / side;
            let mut h = FNV_SEED;
            for word in [
                // 🔴 The snapped CENTRE is deliberately absent, and used to be two of these words.
                // It turned a whole level's content over every time the camera crossed one of its
                // pages — for pages whose world footprint had not moved at all.
                (along / width + 0.5).floor().to_bits(),
                f.x.to_bits(),
                f.y.to_bits(),
                f.z.to_bits(),
                scene_gen,
            ] {
                h = fnv(h, word);
            }
            // `| 1`: a stamp of zero means "no content" and must never
            // match a generation.
            h | 1
        })
        .collect()
}
