// transparent_args.wgsl — the tail's draws, emptied when no pixel overflowed its four layers
// (#452): then the tail costs one dispatch and no raster at all.

struct DrawArgs {
    vertex_count: u32,
    instance_count: u32,
    first_vertex: u32,
    first_instance: u32,
}

@group(0) @binding(0) var<storage, read> overflow: array<u32>;
@group(0) @binding(1) var<storage, read_write> args: array<DrawArgs>;

@compute @workgroup_size(64)
fn cs_args(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&args) && overflow[0] == 0u) {
        args[id.x].instance_count = 0u;
    }
}
