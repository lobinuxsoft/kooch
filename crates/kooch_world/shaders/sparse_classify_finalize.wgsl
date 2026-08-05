// Derive `[ceil_div(needs_count, FINALIZE_WORKGROUP_SIZE), 1, 1]`
// indirect dispatch args from a producer pass that wrote `needs_count`.
// Single-thread compute — runs once per chunk classification, not in
// any per-frame loop, so the 1-thread workgroup is acceptable.
//
// Reused by classify (override = 64u, matching `@workgroup_size(64)`
// of classify_main's consumer) and populate (override = 1u, since
// populate_main does 1 workgroup per marked cell). Pipeline-overridable
// `override` rather than `const` so a single shader file feeds both
// passes — the alternative (a second copy of this file with a different
// constant) would drift the moment one consumer's workgroup size moves.

struct NeedsCount {
    value: u32,
}

struct DispatchIndirectArgs {
    x: u32,
    y: u32,
    z: u32,
}

override FINALIZE_WORKGROUP_SIZE: u32 = 64u;

@group(0) @binding(0) var<storage, read> finalize_needs_count: NeedsCount;
@group(0) @binding(1) var<storage, read_write> finalize_indirect_args: DispatchIndirectArgs;

@compute @workgroup_size(1)
fn finalize_main() {
    let n = finalize_needs_count.value;
    finalize_indirect_args.x = (n + FINALIZE_WORKGROUP_SIZE - 1u) / FINALIZE_WORKGROUP_SIZE;
    finalize_indirect_args.y = 1u;
    finalize_indirect_args.z = 1u;
}
