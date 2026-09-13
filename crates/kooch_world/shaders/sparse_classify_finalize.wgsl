// Derives `[ceil_div(needs_count, FINALIZE_WORKGROUP_SIZE), 1, 1]` indirect args. An `override`,
// not a `const`, so one file feeds classify (64) and populate (1) without a copy.

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
