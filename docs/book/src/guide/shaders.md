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
and `emissive`, plus `alpha` and `alpha_clip` for the shaders that are see-through (below).

`emissive` is in **display units**: `1.0` shows the colour at full brightness whatever the camera's
exposure, and above `1.0` it overdrives. The lights are physical, so an emissive added in their units
would need thousands to be seen at all.

A surface can read the engine's material fields with `materials[input.material_id]`, or declare its
own (below). Sample with `sample_surface`, or `textureSampleGrad` and the analytical derivatives
multiplied by `mip_bias_scale`: a visibility buffer has no screen-space derivatives to give
`textureSample`.

A surface declares no bindings and no entry points of its own. The same function runs in both
shading paths, fragment and compute, and each wraps it in its own frame.

### Kinds

The `// kind:` line picks what the file defines. With no line, it is a `surface`.

| Kind | Defines | What the frame does with it |
|---|---|---|
| `surface` | `fn surface(input: SurfaceInput) -> SurfaceOutput` | Inti lights it: lights, shadows, contact march |
| `unlit` | `fn unlit(input: SurfaceInput) -> UnlitOutput` | Shows `color` as it is, under any light and in shadow |
| `post_process` | `fn post_process(input: SurfaceInput) -> vec4<f32>` | One full-screen draw over the finished frame; reads it with `sample_scene(uv)` |
| `transparent` | `fn surface(input: SurfaceInput) -> SurfaceOutput`, setting `alpha` | Lit like a `surface`, then blended over the opaque scene, far to near |

```wgsl
// kind: unlit

fn unlit(input: SurfaceInput) -> UnlitOutput {
    var out: UnlitOutput;
    out.color = vec3<f32>(1.0, 0.0, 0.0);
    out.alpha = 1.0;
    return out;
}
```

### Transparent

A `transparent` shader is a `surface` that also sets `out.alpha`: 0 lets everything behind through, 1
covers it. 🔴 `var out: SurfaceOutput` starts `alpha` at 0, so a hand-written transparent shader that
forgets it is invisible; the graph's output node has an **alpha** pin that is 1 when unwired.

```wgsl
// kind: transparent

fn surface(input: SurfaceInput) -> SurfaceOutput {
    var out: SurfaceOutput;
    out.base_color = vec3<f32>(0.6, 0.8, 1.0);
    out.normal = normalize(input.world_normal);
    out.roughness = 0.05;
    out.alpha = 0.3;
    return out;
}
```

Transparent objects are drawn after the opaque scene, and put in order **per pixel**: each pixel
keeps its four nearest transparent surfaces exactly, so panes that cross, an object inside another
and the far side of a glass seen through its near side all come out right. Both faces are drawn,
each lit from the side you see. Past four, the rest still blend in, without order — only dense
smoke or particles get there, and there the difference does not show.

- **Their shadow follows the alpha**: a pane at 30% blocks about 30% of the light, and a pattern in
  the alpha shows in the shadow. Untick **cast_shadows** on its renderer to cast none. The alpha is
  read at the surface's uv and the time only — an alpha that depends on where the surface stands or
  where it is seen from casts as if it stood flat at the origin. Up to 32 transparent materials a
  frame shade by alpha; any past that casts solid.
- They write no depth: what is behind glass is still what contact shadows and occlusion see.
- The layers need 64-bit atomics that report what they replaced (Vulkan and DX12 have them; Metal
  does not). Without them, or at resolutions too large for the layers to fit one buffer, objects are
  sorted back to front by their centre instead, with back faces culled — then two crossing panes
  can blend in the wrong order.
- They draw on the compute shading path, which is the default.
- The node panel's preview shows a transparent shader over a checker, so its coverage reads.

### Alpha clip

A `surface` or `unlit` shader that sets `out.alpha_clip` is **masked**: wherever `alpha` falls below
it, the surface is cut away and what is behind shows, solid everywhere else. Leaves, grass, fences
and a noise that dissolves an object are this, not `transparent`: nothing blends, so there is no
order to get wrong, and the cut writes depth like any opaque surface.

```wgsl
var albedo: texture_2d<f32>;   // @default(white)

fn surface(input: SurfaceInput) -> SurfaceOutput {
    var out: SurfaceOutput;
    let leaf = sample_surface(albedo, input, input.uv, vec2<f32>(1.0));
    out.base_color = leaf.rgb;
    out.normal = normalize(input.world_normal);
    out.roughness = 0.8;
    out.alpha = leaf.a;
    out.alpha_clip = 0.5;
    return out;
}
```

- `alpha_clip` starts at 0, which cuts nothing: a shader that never sets it is opaque and costs what
  it always did. In the graph, the output node's **alpha clip** pin makes it masked only once it is
  wired.
- The cut is exact per pixel, at the texture's own resolution, and follows `time`, position and
  view: each masked material rasterises in a pass of its own that runs its `surface` for every
  fragment. Only masked objects pay for that; up to 32 masked materials a frame, and any past that
  draws solid.
- **A still cut becomes geometry.** When the shader's code never mentions `input.time`,
  `input.world_position`, `input.camera_position` or `input.frag_coord`, the engine bakes the cut
  once and rebuilds the mesh along its contour: the object turns plain opaque, discards nothing,
  keeps its meshlet LODs and casts a solid shadow of the real shape. It happens by itself, a few
  frames after the material stops changing, and only for a mesh whose uv stays inside its square —
  a tiled uv keeps the per-pixel cut. Nothing to author: the log line under
  `kooch_render::meshlet::trim` says how many triangles it kept.
- **Their shadow is cut too**, at the surface's uv and the time only, like a transparent one's.
- Back faces are culled as on any surface: a leaf card seen from behind is not drawn.
- The node panel's preview shows the cut.

A `transparent` shader can clip too: what falls below `alpha_clip` is gone — it takes none of the
pixel's four layers and is neither lit nor blended — and the rest blends by its `alpha` as before.
Its shadow drops the cut part and dithers the rest. The transparent output node has the same
**alpha clip** pin.

### Post-process

A `post_process` shader is drawn over what the camera rendered. `input.uv` is the screen in 0..1,
the world fields are zero, `sample_scene(uv)` is the frame, and `scene_size()` is its size in
pixels.

```wgsl
// kind: post_process

fn post_process(input: SurfaceInput) -> vec4<f32> {
    let scene = sample_scene(input.uv);
    let edge = 1.0 - length(input.uv - vec2<f32>(0.5)) * 1.4;
    return vec4<f32>(scene.rgb * clamp(edge, 0.0, 1.0), scene.a);
}
```

To see it, put a **Post Process** component on an entity and add an effect to its **effects**
list. The list is a stack: effects run top to bottom, each reading what the one above it produced,
and the arrows reorder them. Each row has:

- **material**: a material using a `post_process` shader.
- **enabled**: switches that effect alone off.
- **weight**: how much of the effect mixes over what it read, from 0 to 1. The engine blends it,
  as `mix(input, effect, weight)`, so a shader needs nothing to support it.

An effect that is off, at weight 0, or without a material is skipped and costs nothing. Every other
effect is one full-screen pass, so on a handheld a long stack is budget spent. A scene saved with the
older `materials` list loads with each material as an effect, on, at full weight.
It runs in the View panel, the Game panel and the game window alike, over the scene and **under**
the gizmos. Without the component, or with it off, the frame costs what it cost before.

In the graph, the **Scene Color** node reads the frame (unwired, at this pixel). It reads black in
any other kind — only a post-process frame has a scene. The output node has two pins, **color**
and **alpha**. The node panel previews this kind over a built-in test image, so an effect reads
before it is assigned to anything. The test image has a hue sweep across, a brightness ramp down,
and a checker in one corner: banding and dither show on the gradients, pixelation and blur on the
checker. The **UV** node is the screen uv here.

`color` is in the same display units as `emissive`: `1.0` is full brightness at any exposure. `alpha`
is carried into `SurfaceOutput.alpha`; opaque passes ignore it. An unlit shader assumes no sun,
which is what a planet's distant impostor or an atmosphere card needs.

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
| **UV** | Panner, Rotator, Tiling, Polar Coordinates, Twirl, Radial Shear, Spherize |
| **Effects** | Fresnel, Unpack Normal, Desaturate, Blend |
| **Shapes** | Circle, Rectangle, Ring, Polygon, Checker |
| **Noise** | Value Noise, Gradient Noise, Simplex Noise (fBm, turbulence, ridged), White Noise, Voronoi |
| **Output** | Output, with a **kind**: `surface` takes base colour, normal, metallic, roughness, emissive; `transparent` the same plus alpha; `unlit` takes color and alpha |

- **Float, Int, Vector 2/3/4 and Color** are the material's parameters — each one member of
  `SurfaceParams`, with its name and starting value, edited in the node the way the material's
  Inspector will show it: a slider when a Float or Int has a range, a colour picker for a Color.
  Graphs from before these existed carry a single `Param` node; they open converted.
- **Constants** are the same types written into the shader instead of the material: no name, nothing
  for the Inspector to show, just a value. An old graph's four-number `Constant` opens as a
  **Vector 4** with all four kept.
- **Pins say what they take.** Each name ends in how many components it reads or gives — `speed (2)`,
  `radius (1)`, `color (4)`; a pin with no number works component by component. Pins and wires are
  coloured the way Unity Shader Graph colours them: **(1)** cyan, **(2)** green, **(3)** yellow,
  **(4)** pink, any grey. A wire always connects. A single number fills every component, as in Unity and
  Unreal: `colour × Float` scales all three channels, and a number wired into a `(2)` pin sets both.
- **Rest the pointer** on a pin for what it is for and what it does unwired, on a node for what the
  node does, or on a node in the Add menu before placing it. Node headers are tinted by category.
- Nodes lay their fields out top to bottom, under their pins, and a short list of choices — a texture's fallback, a
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
  - **tiling** — how many cells fit across the uv square, per axis, so the noise **repeats with no
    seam**. It takes over from **scale** on the axis it is given (`0` leaves that axis open) and is
    rounded to whole cells, because half a cell cannot come back to itself. A **simplex** basis
    refuses it: its lattice is skewed, so a period there repeats in skewed space and not in uv —
    use value or gradient for anything that has to tile.
  - Outputs: **value** in 0..1, and **color**, three unrelated samples for tinting.

  Distortion, phase and color each cost extra samples, paid only while they are wired.
- **White Noise** is one random value per cell.
- **Voronoi** has a **metric** dropdown (euclidean round cells, manhattan diamonds, chebyshev
  squares) and the inputs **randomness** (0 a regular grid, 1 fully random), **phase** (wire **Time**
  in and the points circle), and **smoothness** (0..1, rounds F1, F2 and the border alike). Outputs: **F1** the
  distance to the nearest point, **F2** to the second, **border** the true distance to the cell edge
  (a wider search, run only while wired), **cell** a random value per cell, and **position** the
  nearest point. **White Noise** and **Voronoi** take the same **tiling** input as the fractal
  noises.

  **Around a turn:** `PolarCoordinates` gives the angle on **y**, in 0..1 for a full turn, so a
  noise read through it tiles with `tiling = (0, n)` — the radius stays open and the angle comes
  back to itself after `n` cells. Without it the turn shows a cut where the angle wraps.

  🔴 The tiled axis has to **arrive spanning 0..1**: the period counts cells over that range, so
  anything upstream that scales it — polar's own *length scale*, a **Tiling** node, a **Multiply** —
  leaves the period closing somewhere other than the edge, and the seam comes back. Set that scale
  to 1 and ask for the density with **tiling** instead. And the value the frame reads is the
  **material's**, not the node's: a parameter node holds the default a new material starts from,
  while one that already exists keeps what it was registered with until the Inspector changes it.
- **UV** nodes take a coordinate and give one back, to feed a Texture, a noise or a shape. The four
  distortions work around a **centre** and, left unwired, read the mesh's uv around its middle (0.5):
  - **Polar Coordinates** gives **radius** (distance from the centre × 2 × *radial scale*) and
    **angle** (0..1 around it × *length scale*): a gradient becomes a radial sweep, a checker a dartboard.
  - **Twirl** swirls more the further out (*strength* 10 by default).
  - **Radial Shear** and **Spherize** shear into a spiral or bulge like a lens (*strength* 10).
  - **offset** moves the result.
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
- **Arrange** lays the graph out in columns counted **back from the Output**: a node sits one
  column left of the furthest thing it feeds, so a parameter wired straight into the output stays
  beside it. Each column follows the pins its nodes feed — what goes into base colour above what
  goes into roughness — which is what keeps the wires from crossing. It is the shape of Godot's
  `arrange_nodes` without its inner-shift pass, which earns its keep on graphs far larger than a shader's.
- Dragging a node marks the file unsaved: where the nodes sit is part of what the `.shader` carries,
  so **Save** is what keeps a layout.

### Editing

The **?** button in the toolbar lists every binding.

- **Undo and redo:** Ctrl+Z, and Ctrl+Y or Ctrl+Shift+Z. The graph is a document of its own, so the
  Edit menu names the step it would undo: *Move nodes*, *Connect*, *Edit value*. A drag or a number
  being dragged is one step until the mouse is released. Undo puts the graph back in memory; the file
  is still yours to **Save**.
- **Selecting:**
  - Click a node to select it alone. Shift + click adds it to the selection, and Ctrl + click
    takes it out.
  - **Shift + drag** on the background box-selects, and Ctrl + Shift + drag deselects.
  - Ctrl+A selects every node. Escape, or a click on the background, clears the selection.
  - Dragging any selected node moves them all.
- **Clipboard:**
  - Ctrl+C, Ctrl+X and Ctrl+V copy, cut and paste the selected nodes **with the wires between
    them**.
  - A paste lands at the pointer, and Ctrl+D duplicates beside the original. Either way, what was
    pasted comes out selected.
  - The output node is never copied, since a graph has one.
  - A pasted parameter whose name is already declared is renamed (`levels` becomes `levels_2`),
    because two parameters of one name would be one uniform.
- **Delete** or **Backspace** removes the selection. **F** frames it.
- **Groups:** Ctrl+G groups the selection.
  - A group's frame fits its nodes every frame, so it follows them as they move.
  - Dragging the title moves the group's nodes, and only those. A frame carried over another node
    never takes it in.
  - Membership is explicit. Drop a dragged node inside a group and it joins; right-click a node and
    choose **Remove from group** to take it out. A node is in one group at most.
  - Double-click the title to rename the group in place. Right-click the title to rename it,
    recolour it or **Ungroup** it. Ungrouping keeps the nodes.
- **Notes:** right-click the background, then **Add note**. Drag a note to move it, and right-click
  it to edit its text or delete it.

Groups and notes are saved in a block of their own at the end of the `.shader`, and codegen never
reads them. A graph without any writes exactly the file it wrote before.

The graph widget is an in-tree fork of egui-snarl (`crates/egui_snarl`). The published crate
selects with Shift only, cannot have its selection set, and does not expose node sizes.
`crates/egui_snarl/KOOCH.md` lists every change made to it, and
`.github/scripts/update_egui_snarl.py` moves the fork onto a newer upstream release.

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

**Measured, not guessed.** Each material's shading is its own GPU scope, labelled by its shader, so
the cost of a shader is every material using it added up:

- The **Shader Graph header** reads `24 nodes · 0.42 ms GPU` for the saved file.
- The **Profiler** opens with **Shaders (GPU)**, every shader on screen by name, most expensive first,
  each as a share of the handheld's 13.9 ms frame.
- The flamegraph shows them nested under the shading pass (`shade: fragment` or the compute path's).
- **On the handheld**: with **A running game** selected, the table reads the frames the game sends over
  the network, under the same names. A game built with the profiling preset carries the shader scopes;
  run it on the OneXFly, connect, and the table is the handheld's, not the desktop's.

What the number means:
- **It is the last finished frame**, a few frames behind, because timestamps come back without
  blocking.
- **In the editor it adds up both views**, Edit and Game, since each draws the scene.
- **`—` means nothing on screen uses the shader**, or the GPU cannot write timestamps inside a pass
  (the compute path needs `TIMESTAMP_QUERY_INSIDE_PASSES`).
- **A game build carries none of this** unless it was built with the profiling preset: the scopes
  compile out.

