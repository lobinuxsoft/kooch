// material_surface.wgsl — the engine's half of the surface contract: filling `SurfaceInput` from the
// reconstructed vertex. Composed after `kooch_surface.wgsl`.

fn surface_input(surf: VertexOutput, frag_coord: vec2<f32>) -> SurfaceInput {
    var input: SurfaceInput;
    input.world_position = surf.world_position;
    input.world_normal = surf.world_normal;
    input.world_tangent = surf.world_tangent;
    input.uv = surf.uv;
    input.ddx_uv = surf.ddx_uv;
    input.ddy_uv = surf.ddy_uv;
    input.mip_bias_scale = screen.mip_bias_scale;
    input.frag_coord = frag_coord;
    input.camera_position = inti.camera_position;
    input.material_id = screen.material_id;
    return input;
}
