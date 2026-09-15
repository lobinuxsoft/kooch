# Shaders

A material's look comes from a **surface shader**: a `.shader` file with one WGSL function. Every
material without one uses the engine's PBR surface, so nothing changes until you pick a shader.

## Making one

- **Asset Browser → right-click a folder under `assets/` → New Shader.** The file starts as a copy
  of the engine's PBR surface, ready to edit.
- **Right-click the `.shader` → Create Material.** A material beside it, already shading with it.
- **Any material → Inspector → Shader.** Switch a material to another shader, or back to `(None)`,
  the engine's PBR surface.

Edit the file in your IDE (**Open in IDE**) and save: the editor picks the change up within a
second, locally and in the running project.

## The contract

```wgsl
// kind: surface

fn surface(input: SurfaceInput) -> SurfaceOutput {
    var out: SurfaceOutput;
    out.base_color = vec3<f32>(1.0, 0.0, 0.0);
    out.normal = normalize(input.world_normal);
    out.metallic = 0.0;
    out.roughness = 0.5;
    out.emissive = vec3<f32>(0.0);
    return out;
}
```

`SurfaceInput` carries the reconstructed point: `world_position`, `world_normal`, `world_tangent`,
`uv`, the analytical `ddx_uv` / `ddy_uv`, `mip_bias_scale`, `frag_coord` and `material_id`.
`SurfaceOutput` is what Inti lights: `base_color`, a world-space `normal`, `metallic`, `roughness`
and `emissive`.

A surface can read the material's parameters with `materials[input.material_id]` and sample its
three maps — `albedo_tex`, `normal_tex`, `metal_rough_tex` — through `material_sampler`. Sample
with `textureSampleGrad` and the analytical derivatives multiplied by `mip_bias_scale`: a
visibility buffer has no screen-space derivatives to give `textureSample`.

A surface declares no bindings and no entry points of its own. The same function runs in both
shading paths, fragment and compute, and each wraps it in its own frame.

## When a save does not compile

The engine checks the shader before building anything from it. A broken save keeps the last
version that compiled on screen and logs the error with the line in your file:

```text
shader does not compile, keeping the last good one — line 2: expected expression, found ';'
```

## What a shader costs

One pipeline per shader, not per material: ten materials on three shaders are three pipelines. On
the compute path each material dispatches only over the screen tiles it covers.
