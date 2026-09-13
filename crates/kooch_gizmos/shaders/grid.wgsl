// Infinite ground grid, per pixel: no geometry, so it fades where it should and never ends.
// Adapted from Godot's editor grid (MIT): a level from `log(distance)` whose fraction crossfades,
// plus angle and camera-distance fades.

struct GridUniforms {
    inverse_view_proj: mat4x4<f32>,
    view_proj: mat4x4<f32>,
    camera_position: vec3<f32>,
    // The finer of the two levels being crossfaded, in world units.
    small_step: f32,
    cell_color: vec3<f32>,
    // How many small cells make one large one.
    steps: f32,
    counting_color: vec3<f32>,
    // 0 at the start of a level, 1 as the next one takes over.
    blend: f32,
    axis_x_color: vec3<f32>,
    // Height of the plane. Zero for the ground.
    plane_y: f32,
    axis_z_color: vec3<f32>,
    // Where the fade reaches nothing, from the camera's own position.
    fade_distance: f32,
    // x: 1 when the world axes should be drawn, 0 for a guide.
    // A vec4, not a scalar plus padding: WGSL aligns a trailing vec3 to 16, Rust's [f32; 3] to 4.
    flags: vec4<f32>,
}

@group(0) @binding(0) var<uniform> grid: GridUniforms;

struct Fragment {
    @builtin(position) clip: vec4<f32>,
    @location(0) near: vec3<f32>,
    @location(1) far: vec3<f32>,
}

fn unproject(x: f32, y: f32, z: f32) -> vec3<f32> {
    let world = grid.inverse_view_proj * vec4<f32>(x, y, z, 1.0);
    return world.xyz / world.w;
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Fragment {
    // One oversized triangle rather than two: no seam down the diagonal.
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let corner = corners[index];

    var out: Fragment;
    out.clip = vec4<f32>(corner, 1.0, 1.0);
    // Reversed-Z: 1.0 is the near plane, which is why these look
    // swapped against every tutorial.
    out.near = unproject(corner.x, corner.y, 1.0);
    out.far = unproject(corner.x, corner.y, 0.0);
    return out;
}

/// How much of a line covers this pixel, for a coordinate in cells. `fwidth` keeps lines a constant
/// screen width at any distance.
fn coverage(cell: vec2<f32>) -> f32 {
    let derivative = fwidth(cell);
    let to_line = abs(fract(cell - 0.5) - 0.5) / derivative;
    return 1.0 - min(min(to_line.x, to_line.y), 1.0);
}

struct Shaded {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

@fragment
fn fs_main(in: Fragment) -> Shaded {
    let direction = in.far - in.near;
    let t = (grid.plane_y - in.near.y) / direction.y;
    // Behind the eye, or past the far plane: nothing to draw.
    if (t <= 0.0 || t >= 1.0) {
        discard;
    }
    let world = in.near + direction * t;

    // 🔴 The crossfade: `fine` fades out by `blend` while `coarse` takes over, landing exactly on
    // the next level's fine lines.
    let fine = coverage(world.xz / grid.small_step) * (1.0 - grid.blend);
    let coarse = coverage(world.xz / (grid.small_step * grid.steps));
    var line = max(fine, coarse);
    if (line <= 0.0) {
        discard;
    }

    // From the camera's own place on the plane. A grid that faded from
    // a fixed origin would be gone the moment you walked away from it.
    let eye_on_plane = vec2<f32>(grid.camera_position.x, grid.camera_position.z);
    let reach = 1.0 - min(distance(world.xz, eye_on_plane) / grid.fade_distance, 1.0);
    let distance_fade = smoothstep(0.02, 0.3, reach);

    // Looking ALONG the plane turns a grid into a solid sheet at the
    // horizon. The view's angle to the normal is what removes it.
    let to_eye = normalize(grid.camera_position - world);
    let angle_fade = smoothstep(0.05, 0.2, abs(to_eye.y));

    var color = mix(grid.cell_color, grid.counting_color, coarse);

    // The axes over the cells crossing them, and only where a world
    // grid asked for them.
    if (grid.flags.x > 0.5) {
        let axis = abs(world.xz) / fwidth(world.xz);
        if (axis.y < 1.0) {
            color = grid.axis_x_color;
            line = max(line, 1.0 - axis.y);
        }
        if (axis.x < 1.0) {
            color = grid.axis_z_color;
            line = max(line, 1.0 - axis.x);
        }
    }

    let alpha = line * distance_fade * angle_fade;
    if (alpha <= 0.0) {
        discard;
    }

    var out: Shaded;
    out.color = vec4<f32>(color, alpha);
    // Written so the grid sits IN the scene rather than over it: a block
    // in front of it occludes it, which is the whole complaint about
    // gizmos drawing with `CompareFunction::Always`.
    let clip = grid.view_proj * vec4<f32>(world, 1.0);
    out.depth = clip.z / clip.w;
    return out;
}
