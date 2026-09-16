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
`uv`, the analytical `ddx_uv` / `ddy_uv`, `mip_bias_scale`, `frag_coord`, `camera_position`, `time`
(seconds since the engine started, for a surface that moves) and `material_id`.
`SurfaceOutput` is what Inti lights: `base_color`, a world-space `normal`, `metallic`, `roughness`
and `emissive`.

`emissive` is in **display units**: `1.0` shows the colour at full brightness whatever the camera's
exposure, and above `1.0` it overdrives. The lights are physical, so an emissive added in their units
would need thousands to be seen at all.

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

Members are `f32`, `vec2<f32>`, `vec3<f32>` or `vec4<f32>`; a whole number is an `f32` hinted
`@int`. `SURFACE_DEFAULTS` is an ordinary WGSL constant — naga evaluates it, so `vec3(0.25)` or an arithmetic expression works — and without it
every member starts at zero. The comment after a declaration is optional and only changes how the
editor shows the field:

| Hint | On | Does |
|---|---|---|
| `@color` | `vec4<f32>` | a colour picker instead of four numbers |
| `@range(lo, hi)` | `f32` | a slider |
| `@int` | `f32` | whole steps — a stepped slider with `@range`, a stepped field without |
| `@default(white \| black \| normal)` | a texture | what it samples while unassigned — WGSL gives a texture no starting value |

`sample_surface(texture, input, uv, scale)` samples with the analytical derivatives scaled by
`scale` and the mip bias, so pass whatever tiles `uv`. Budget per material: **64 scalars** and
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

🔴 **A project's shaders are authored as nodes.** The graph writes the WGSL, so nobody has to learn a
shading language to make a material. Hand-written `.shader` files keep working — the engine's own
surface is one — and **New Shader (WGSL)** is still there for what a graph cannot reach yet.

**Asset Browser → right-click a folder → New Shader Graph** writes a `.shader` whose graph is already
a material: an albedo texture tinted by a colour, with a roughness of its own. Double-clicking a
`.shader` opens it in the **Shader Graph** panel; one written by hand has no graph to draw, so it
opens in the IDE instead.

**The graph owns the file.** It rides in a block comment at the top — the way Shader Forge carries
`/*SF_DATA;…*/` — and everything below it is generated: parameters, defaults, textures and the
`surface` function. Saving the graph rewrites all of it, so edits made by hand there are lost.

Right-click the background to add a node, drag between pins to wire them, and press **Save** to write
the shader. Every wire carries a `vec4<f32>`: unused components are zero, and each node reads the
components it needs, so any output plugs into any input.

The menu groups the nodes the way the panels do:

| Menu | Nodes |
|---|---|
| **Input** | UV, World Normal, World Position, View Direction, **Time**, Float, Int, Vector 2, Vector 3, Vector 4, Color, Texture |
| **Constants** | Float, Int, Vector 2, Vector 3, Vector 4, Color |
| **Math** | Add, Subtract, Multiply, Divide, One Minus, Abs, Floor, Fract, Sine, Cosine, Min, Max, Clamp, Step, Smoothstep, Power, Saturate, Remap, Mix |
| **Vector** | Dot, Cross, Normalize, Length, Distance, Reflect, Swizzle, Combine, Split |
| **Effects** | Fresnel, Unpack Normal, Panner, Rotator, Tiling, Desaturate, Blend |
| **Shapes** | Circle, Rectangle, Ring, Polygon, Checker |
| **Noise** | Value Noise, Gradient Noise, Simplex Noise (fBm, turbulence, ridged), White Noise, Voronoi |
| **Output** | Surface Output: base colour, normal, metallic, roughness, emissive |

- **Float, Int, Vector 2/3/4 and Color** are the material's parameters — each one member of
  `SurfaceParams`, with its name and starting value, edited in the node the way the material's
  Inspector will show it: a slider when a Float or Int has a range, a colour picker for a Color.
  Graphs from before these existed carry a single `Param` node; they open converted.
- **Constants** are the same types written into the shader instead of the material: no name, nothing
  for the Inspector to show, just a value. An old graph's four-number `Constant` opens as a
  **Vector 4** with all four kept.
- Nodes lay their fields out top to bottom, and a short list of choices — a texture's fallback, a
  blend mode — is a dropdown, so a node stays about as wide as its title.
- **Texture** writes a `var name: texture_2d<f32>;` and samples it at the uv it is given. **Unpack
  Normal** turns that sample into a world-space normal through the mesh's tangent frame.
- **Time** is seconds, its sine, its cosine and a tenth of it, one pin each — the clock **Panner**
  scrolls a coordinate with.
- **Outputs come whole and in channels.** A colour — Texture, Color, Blend, Desaturate — has **RGBA**,
  **RGB**, **R**, **G**, **B** and **A**; a position or direction has **XYZ**, **X**, **Y**, **Z**; UV
  has **UV**, **U**, **V**; a vector has its whole and each component. The first pin is always the whole
  value. **Time** comes apart only — *time*, *sine*, *cosine*, *tenth* — and so do **Voronoi** and
  **Split**, because together those numbers mean nothing. Maths, noises and shapes answer with one
  value; **Split** takes any of them apart.
- **Swizzle** reorders components (`xyzw` passes through, `xxxx` splashes the first, `yx` swaps), and
  **Combine** builds a vector from four numbers.
- **Noise**: Value, Gradient (Perlin) and Simplex are one node with a **basis** dropdown and a
  **fractal** one — **fbm** (smooth), **turbulence** (billowy) or **ridged** (sharp crests). Every
  input can stay unconnected:
  - **octaves** (1) — layers of detail, up to 8; **roughness** (0.5) — how much each layer keeps of
    the last; **lacunarity** (2) — how much finer each layer is.
  - **distortion** — warps the coordinate by the noise itself, for marble and smoke.
  - **phase** — wire **Time** in and the noise changes where it stands instead of sliding away.
  - Outputs: **value** in 0..1, and **color**, three unrelated samples for tinting.

  Distortion, phase and color each cost extra samples, paid only while they are wired.
- **White Noise** is one random value per cell.
- **Voronoi** has a **metric** dropdown (euclidean round cells, manhattan diamonds, chebyshev
  squares) and the inputs **randomness** (0 a regular grid, 1 fully random), **phase** (wire **Time**
  in and the points circle), and **smoothness** (blends cells into each other). Outputs: **F1** the
  distance to the nearest point, **F2** to the second, **border** the true distance to the cell edge
  (a wider search, run only while wired), **cell** a random value per cell, and **position** the
  nearest point.
- **Shapes** read the uv square with its middle at `0.5`, and answer with a mask in every component.
- A few nodes lean on a small WGSL function (`graph_noise`, `graph_rotate`, …). It is written into the
  file **only when a node asks for it**, so a generated shader carries nothing it does not use.

An unconnected output keeps a usable default: the geometric normal, roughness `0.5`, and zero for the
rest — so half a graph still renders. Unconnected *inputs* read as zero, and every node is written so
that zero is never a NaN: a divide by zero is zero, `normalize` of nothing points up, and two equal
`smoothstep` edges are pushed apart.

### The preview

The column on the right shows the shader **on a shape**, turning, updated as the graph changes.

- The shape is any of the engine's primitives — cube, sphere, capsule, cylinder, cone, quad — picked
  from the dropdown above it. A gradient reads one way on a sphere and another on a quad, and a
  shader is worth judging on the geometry it will actually run on.
- It runs the **same `surface` function the scene runs**, from the same generated WGSL. What differs
  is the frame around it: one primitive rasterised and one key light, with none of Inti, the shadow
  pages or the visibility buffer behind it — which is why a preview costs a thumbnail, not a second
  viewport.
- Parameters show the **starting values the shader declares** (`SURFACE_DEFAULTS`), not a material's:
  what is being previewed is the shader, before anything has been assigned to it.
- A **Texture** node's `preview` field picks an image to see it with, without a material. It is saved
  with the graph, like where the nodes sit, and reaches neither the WGSL nor any material — those
  still start from the node's fallback (`white`, `black` or `normal`).
- Drag the column's edge to resize it; the image is re-rendered at the new size once you let go.
- **Unpack Normal** works here because the preview builds a tangent frame per primitive; the engine's
  meshes carry none.
- While the graph does not compile, the column says so and why, instead of going on showing the last
  version that did.

### Moving around

- **The view follows its panel.** Moving the window, or the panel inside the dock, leaves the graph
  where it was *relative to the panel* — not where it was on screen.
- A graph **opens framed**: its window takes most of the screen, and the view is zoomed and centred on
  every node. **Fit** frames it again at any time, and **Arrange** does so once it has laid the graph out.
- **Minimap**, toggled in the toolbar: a box per node and a rectangle around what you are looking at.
  Click anywhere on it to send the view there.
- **Arrange** lays the graph out in columns counted **back from the Surface Output**: a node sits one
  column left of the furthest thing it feeds, so a parameter wired straight into the output stays
  beside it. Each column follows the pins its nodes feed — what goes into base colour above what
  goes into roughness — which is what keeps the wires from crossing. It is the shape of Godot's
  `arrange_nodes` without its inner-shift pass, which earns its keep on graphs far larger than a shader's. It is the shape of Godot's `arrange_nodes`
  without its inner-shift pass, which earns its keep on graphs far larger than a shader's.
- Dragging a node marks the file unsaved: where the nodes sit is part of what the `.shader` carries,
  so **Save** is what keeps a layout.

### What the graph cannot do

**One graph is one pass.** The visibility buffer holds one surface per pixel and each material's pass
runs against it, so there is no second pass of the same surface to add an outline or a shell with.
Effects that genuinely need another pass belong to a shader **kind** rather than to the graph:
transparency and refraction to the forward pass (#452), glow and screen distortion to post-process,
grass and fur to compute-generated geometry. A rim light is the exception that fits today — that is
what **Fresnel** is for.

## When a save does not compile

The engine checks the shader before building anything from it. A broken save keeps the last
version that compiled on screen and logs the error with the line in your file:

```text
shader does not compile, keeping the last good one — line 2: expected expression, found ';'
```

## What a shader costs

One pipeline per shader, not per material: ten materials on three shaders are three pipelines. On
the compute path each material dispatches only over the screen tiles it covers.
