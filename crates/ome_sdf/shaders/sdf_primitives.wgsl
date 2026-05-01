// sdf_primitives.wgsl
//
// Signed Distance Field primitive library for oh_my_engine.
//
// Convention: all primitives are evaluated in local space and centered at
// the origin. Use `transform_point` to bring a world-space sample into a
// primitive's local space using its translation and rotation.
//
// Distance sign: negative = inside, positive = outside, zero = on surface.
//
// Signatures match the components in `ome_ecs` (SdfSphere, SdfBox, SdfCapsule,
// SdfCylinder, SdfTorus, SdfPlane) so CPU data maps directly to GPU calls.

// =============================================================================
// PRIMITIVES
// =============================================================================

/// Sphere centered at the origin.
fn sdf_sphere(p: vec3<f32>, radius: f32) -> f32 {
    return length(p) - radius;
}

/// Axis-aligned box centered at the origin.
/// `half_extents` are the half-sizes along each axis.
fn sdf_box(p: vec3<f32>, half_extents: vec3<f32>) -> f32 {
    let q = abs(p) - half_extents;
    return length(max(q, vec3<f32>(0.0))) + min(max(q.x, max(q.y, q.z)), 0.0);
}

/// Signed distance from `p` to the axis-aligned box defined by `[lo, hi]`.
/// Negative inside, positive outside, zero on the surface — same convention
/// as every other primitive. Used as the BVH-pruning lower bound during
/// sphere-tracing traversal: `aabb_dist(p)` is a strict lower bound on the
/// distance from `p` to any surface inside the box, so a subtree can be
/// pruned whenever `sdf_aabb(p, node) > current_min_acc`.
fn sdf_aabb(p: vec3<f32>, lo: vec3<f32>, hi: vec3<f32>) -> f32 {
    let centre = 0.5 * (lo + hi);
    let half = 0.5 * (hi - lo);
    return sdf_box(p - centre, half);
}

/// Rounded box. Matches `SdfBox { size, rounding }`.
/// `rounding == 0.0` degenerates to a regular box.
fn sdf_rounded_box(p: vec3<f32>, half_extents: vec3<f32>, rounding: f32) -> f32 {
    let q = abs(p) - half_extents + vec3<f32>(rounding);
    return length(max(q, vec3<f32>(0.0)))
        + min(max(q.x, max(q.y, q.z)), 0.0)
        - rounding;
}

/// Capsule aligned with the local Y axis. Matches `SdfCapsule { radius, half_height }`.
/// The cylindrical segment extends from `-half_height` to `+half_height`,
/// with hemispherical caps of `radius`.
fn sdf_capsule_y(p: vec3<f32>, half_height: f32, radius: f32) -> f32 {
    var q = p;
    q.y = q.y - clamp(q.y, -half_height, half_height);
    return length(q) - radius;
}

/// General capsule between two arbitrary endpoints `a` and `b`.
fn sdf_capsule(p: vec3<f32>, a: vec3<f32>, b: vec3<f32>, radius: f32) -> f32 {
    let ab = b - a;
    let ap = p - a;
    let t = clamp(dot(ap, ab) / dot(ab, ab), 0.0, 1.0);
    let closest = a + t * ab;
    return length(p - closest) - radius;
}

/// Cylinder aligned with the local Y axis, capped at `+/- half_height`.
/// Matches `SdfCylinder { radius, half_height }`.
fn sdf_capped_cylinder(p: vec3<f32>, half_height: f32, radius: f32) -> f32 {
    let d = abs(vec2<f32>(length(p.xz), p.y)) - vec2<f32>(radius, half_height);
    return min(max(d.x, d.y), 0.0) + length(max(d, vec2<f32>(0.0)));
}

/// Torus lying in the local XZ plane. Matches `SdfTorus { major_radius, minor_radius }`.
fn sdf_torus(p: vec3<f32>, major_radius: f32, minor_radius: f32) -> f32 {
    let q = vec2<f32>(length(p.xz) - major_radius, p.y);
    return length(q) - minor_radius;
}

/// Infinite plane with arbitrary `normal` and signed offset along it.
fn sdf_plane(p: vec3<f32>, normal: vec3<f32>, offset: f32) -> f32 {
    return dot(p, normalize(normal)) + offset;
}

/// Axis-aligned plane perpendicular to the local Y axis at y = 0.
/// Matches `SdfPlane` (no parameters — orientation comes from Transform).
fn sdf_plane_y(p: vec3<f32>) -> f32 {
    return p.y;
}

// =============================================================================
// CSG OPERATIONS
// =============================================================================

/// Boolean union (OR): the combined shape.
fn sdf_union(d1: f32, d2: f32) -> f32 {
    return min(d1, d2);
}

/// Boolean intersection (AND): only where both overlap.
fn sdf_intersection(d1: f32, d2: f32) -> f32 {
    return max(d1, d2);
}

/// Boolean subtraction (d1 - d2): carves d2 out of d1.
fn sdf_subtraction(d1: f32, d2: f32) -> f32 {
    return max(d1, -d2);
}

// =============================================================================
// SMOOTH CSG (polynomial smooth-min, k controls blend radius)
// =============================================================================

/// Smooth union. `k` is the blend radius; larger = softer transition.
fn sdf_smooth_union(d1: f32, d2: f32, k: f32) -> f32 {
    let h = clamp(0.5 + 0.5 * (d2 - d1) / k, 0.0, 1.0);
    return mix(d2, d1, h) - k * h * (1.0 - h);
}

/// Smooth intersection.
fn sdf_smooth_intersection(d1: f32, d2: f32, k: f32) -> f32 {
    let h = clamp(0.5 - 0.5 * (d2 - d1) / k, 0.0, 1.0);
    return mix(d2, d1, h) + k * h * (1.0 - h);
}

/// Smooth subtraction.
fn sdf_smooth_subtraction(d1: f32, d2: f32, k: f32) -> f32 {
    let h = clamp(0.5 - 0.5 * (d2 + d1) / k, 0.0, 1.0);
    return mix(d1, -d2, h) + k * h * (1.0 - h);
}

// =============================================================================
// TRANSFORMATIONS
// =============================================================================

/// Rotates `v` by quaternion `q` (x, y, z, w).
fn quat_rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}

/// Transforms a world-space point into a primitive's local space given
/// the primitive's world translation and rotation. Apply this before
/// calling any `sdf_*` evaluator so the primitive stays axis-aligned
/// in its own frame.
fn transform_point(p: vec3<f32>, position: vec3<f32>, rotation: vec4<f32>) -> vec3<f32> {
    let translated = p - position;
    // Rotate by the inverse rotation (conjugate for unit quaternions).
    let inv = vec4<f32>(-rotation.xyz, rotation.w);
    return quat_rotate(inv, translated);
}

/// Scales a sample point for a primitive with uniform scale `s`.
/// NOTE: when sampling a scaled primitive, the returned distance must
/// be multiplied by `s` to remain a valid SDF (Lipschitz bound).
fn scale_point(p: vec3<f32>, s: f32) -> vec3<f32> {
    return p / s;
}

// =============================================================================
// NORMAL CALCULATION HELPERS
// =============================================================================

/// Returns the central-difference epsilon to use when computing an SDF
/// surface normal at a hit point.
///
/// Returns 10% of the local hit threshold — sub-precision relative to
/// the hit-detection itself — so the central-difference samples are
/// guaranteed never to bridge `min(a, b)` gradient discontinuities at
/// CSG seams. Without this, normals near smooth-blend zones average
/// across incompatible primitives and produce dark patches on lit
/// surfaces (#225 H#1).
///
/// `dist` is the ray distance traveled to the hit point (the depth
/// proxy that the hit threshold uses). `surface_threshold` and
/// `epsilon_factor` come from the caller's `RayMarchParams`-equivalent
/// uniform — pass them in so this helper stays decoupled from any
/// renderer-specific binding layout.
///
/// **Use this in any future shader that computes SDF normals via
/// central differences** (compute pathtracer, debug visualization,
/// etc.). Do NOT hardcode an epsilon — the project learned the hard
/// way that a constant `0.001` causes the artifact above.
fn sdf_normal_eps(dist: f32, surface_threshold: f32, epsilon_factor: f32) -> f32 {
    return (surface_threshold + epsilon_factor * dist) * 0.1;
}
