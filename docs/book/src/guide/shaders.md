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
`uv`, the analytical `ddx_uv` / `ddy_uv`, `mip_bias_scale`, `frag_coord`, `camera_position` and
`material_id`.
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
the members of `struct SurfaceParams` are the material's fields, each `var name: texture_2d<f32>;`
is one of its textures, and `const SURFACE_DEFAULTS` is what a new material starts with. The engine
assigns the bindings — a surface never writes `@group` or `@binding`. A material on that shader
shows exactly these fields in the Inspector; the engine's built-in ones come back when it returns
to `(None)`.

```wgsl
// kind: surface
struct SurfaceParams {
    tint: vec4<f32>,       // @color
    strength: f32,         // @range(0, 4)
    uv_scale: vec2<f32>,
}

const SURFACE_DEFAULTS = SurfaceParams(vec4(1.0, 0.5, 0.2, 1.0), 1.0, vec2(1.0));

var detail: texture_2d<f32>;   // @default(white)

fn surface(input: SurfaceInput) -> SurfaceOutput {
    let p = surface_params(input.material_id);
    let uv = input.uv * p.uv_scale;
    let d = sample_surface(detail, input, uv, p.uv_scale);
    // …
}
```

Members are `f32`, `vec2<f32>`, `vec3<f32>` or `vec4<f32>`. `SURFACE_DEFAULTS` is an ordinary WGSL
constant — naga evaluates it, so `vec3(0.25)` or an arithmetic expression works — and without it
every member starts at zero. The comment after a declaration is optional and only changes how the
editor shows the field:

| Hint | On | Does |
|---|---|---|
| `@color` | `vec4<f32>` | a colour picker instead of four numbers |
| `@range(lo, hi)` | `f32` | a slider |
| `@default(white \| black \| normal)` | a texture | what it samples while unassigned — WGSL gives a texture no starting value |

`sample_surface(texture, input, uv, scale)` samples with the analytical derivatives scaled by
`scale` and the mip bias, so pass whatever tiles `uv`. Budget per material: **16 scalars** and
**4 textures**; past it the shader fails to load and names the line.

Values are stored on the material by name. Switching a material to another shader keeps the values
both declare and drops the rest; undo brings them back. New Shader starts from a PBR surface written
against its own parameters.

### Editing `.shader` files in an IDE

No IDE can check a `.shader` completely: it reads one file and cannot see what the engine composes
around a surface, so `SurfaceInput`, `surface_params` and everything built on them look undefined.
The fix in every IDE is the same — read `.shader` as WGSL, and turn off the diagnostics that hinge
on the missing part. The editor's Console is the authority on whether a shader compiles.

#### VS Code

Install [wgsl-analyzer](https://marketplace.visualstudio.com/items?itemName=wgsl-analyzer.wgsl-analyzer).
Opening a project adds these to `.vscode/settings.json`, creating it if absent. Keys you already set
keep their values; a file with comments is left untouched and the Console says so, and then these go
in by hand:

```json
{
  "files.associations": { "*.shader": "wgsl" },
  "wgsl-analyzer.diagnostics.typeErrors": false,
  "wgsl-analyzer.diagnostics.nagaParsingErrors": false,
  "wgsl-analyzer.inlayHints.typeHints": false
}
```

#### Other IDEs

Not configured by the editor yet. Associate the extension with WGSL, then pass the same three
wgsl-analyzer settings through the IDE's language-server configuration:

- **Zed** — `.zed/settings.json`: `"file_types": { "WGSL": ["shader"] }`.
- **Helix** — `.helix/languages.toml`: a `[[language]]` entry with `name = "wgsl"` and
  `file-types = ["wgsl", "shader"]`. Helix replaces the list rather than extending it, so the
  built-in `wgsl` has to be repeated.

## The shader graph

A shader can be authored as nodes instead of code. **Asset Browser → right-click a folder → New
Shader Graph** writes a `.shader` with an empty graph; double-clicking any `.shader` opens it in the
**Shader Graph** panel.

🔴 **The graph owns the file.** It rides in a block comment at the top — the way Shader Forge carries
`/*SF_DATA;…*/` — and everything below it is generated: parameters, defaults, textures and the
`surface` function. Saving the graph rewrites all of it, so edits made by hand there are lost. A
shader written by hand carries no graph, and the panel says so rather than opening an empty one.

Right-click the background to add a node, drag between pins to wire them, and press **Save** to write
the shader. Every wire carries a `vec4<f32>`: unused components are zero, and each node reads the
components it needs, so any output plugs into any input.

| Node | Gives |
|---|---|
| UV, World Normal, World Position, View Direction | the shaded point, from `SurfaceInput` |
| Param | one member of `SurfaceParams`: its name, width, colour hint and starting value |
| Texture | a `var name: texture_2d<f32>;` sampled at the uv it is given |
| Constant | a value written into the shader rather than the material |
| Add, Multiply, Mix, Dot, Power, Saturate | the arithmetic |
| Surface Output | base colour, normal, metallic, roughness, emissive |

An unconnected output keeps a usable default: the geometric normal, roughness `0.5`, and zero for the
rest — so half a graph still renders.

## When a save does not compile

The engine checks the shader before building anything from it. A broken save keeps the last
version that compiled on screen and logs the error with the line in your file:

```text
shader does not compile, keeping the last good one — line 2: expected expression, found ';'
```

## What a shader costs

One pipeline per shader, not per material: ten materials on three shaders are three pipelines. On
the compute path each material dispatches only over the screen tiles it covers.
