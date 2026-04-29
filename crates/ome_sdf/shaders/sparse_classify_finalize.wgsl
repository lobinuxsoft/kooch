// Derive `[ceil_div(needs_count, 64), 1, 1]` indirect dispatch args
// from the classify pass output. Single-thread compute pass — runs
// once per chunk classification, not in any per-frame loop, so the
// 1-thread workgroup is acceptable.
//
// Kept in its own module (rather than as a second entry point in
// `sparse_classify.wgsl`) because the bindings differ from the
// classify entry — and classify itself is concatenated with sampler
// shader source, while finalize never is.

struct NeedsCount {
    value: u32,
}

struct DispatchIndirectArgs {
    x: u32,
    y: u32,
    z: u32,
}

const FINALIZE_WORKGROUP_SIZE: u32 = 64u;

@group(0) @binding(0) var<storage, read> finalize_needs_count: NeedsCount;
@group(0) @binding(1) var<storage, read_write> finalize_indirect_args: DispatchIndirectArgs;

@compute @workgroup_size(1)
fn finalize_main() {
    let n = finalize_needs_count.value;
    finalize_indirect_args.x = (n + FINALIZE_WORKGROUP_SIZE - 1u) / FINALIZE_WORKGROUP_SIZE;
    finalize_indirect_args.y = 1u;
    finalize_indirect_args.z = 1u;
}
