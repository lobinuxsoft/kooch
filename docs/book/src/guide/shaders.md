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

A surface can read the engine's material fields with `materials[input.material_id]`, or declare its
own (below). Sample with `sample_surface`, or `textureSampleGrad` and the analytical derivatives
multiplied by `mip_bias_scale`: a visibility buffer has no screen-space derivatives to give
`textureSample`.

A surface declares no bindings and no entry points of its own. The same function runs in both
shading paths, fragment and compute, and each wraps it in its own frame.

## Parameters

A shader's parameters are plain WGSL, the closest WGSL gets to an HLSL `cbuffer` and `Texture2D`:
the members of `struct SurfaceParams` are the material's fields, and each `var name: texture_2d<f32>;`
is one of its textures. The engine assigns the bindings — a surface never writes `@group` or
`@binding`. A material on that shader shows exactly these fields in the Inspector; the engine's
built-in ones come back when it returns to `(None)`.

```wgsl
// kind: surface
struct SurfaceParams {
    tint: vec4<f32>,       // @color @default(1, 0.5, 0.2, 1)
    strength: f32,         // @range(0, 4) @default(1)
    uv_scale: vec2<f32>,   // @default(1, 1)
}

var detail: texture_2d<f32>;   // @default(white)

fn surface(input: SurfaceInput) -> SurfaceOutput {
    let p = surface_params(input.material_id);
    let uv = input.uv * p.uv_scale;
    let d = sample_surface(detail, input, uv, p.uv_scale);
    // …
}
```

Members are `f32`, `vec2<f32>`, `vec3<f32>` or `vec4<f32>`. The comment after a declaration is
optional and only changes how the editor shows the field; the shader compiles the same without it:

| Hint | On | Does |
|---|---|---|
| `@color` | `vec4<f32>` | a colour picker instead of four numbers |
| `@range(lo, hi)` | `f32` | a slider |
| `@default(...)` | any member | the starting value; one number fills every component |
| `@default(white \| black \| normal)` | a texture | what it samples while unassigned |

Without hints a member starts at zero and a texture at white. `sample_surface(texture, input, uv,
scale)` samples with the analytical derivatives scaled by `scale` and the mip bias, so pass
whatever tiles `uv`. Budget per material: **16 scalars** and **4 textures**; past it the shader
fails to load and names the line.

Values are stored on the material by name. Switching a material to another shader keeps the values
both declare and drops the rest; undo brings them back. New Shader starts from a PBR surface written
against its own parameters.

## When a save does not compile

The engine checks the shader before building anything from it. A broken save keeps the last
version that compiled on screen and logs the error with the line in your file:

```text
shader does not compile, keeping the last good one — line 2: expected expression, found ';'
```

## What a shader costs

One pipeline per shader, not per material: ten materials on three shaders are three pipelines. On
the compute path each material dispatches only over the screen tiles it covers.
