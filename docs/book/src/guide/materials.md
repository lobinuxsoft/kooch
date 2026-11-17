# Materials

A **material** is a `.material` file: the values a surface is shaded with. A `MeshRenderer` points
at one by `Guid`, so one material shades every mesh that names it, and editing it changes all of
them at once.

## Making one

- **Asset Browser → right-click a folder → New Material.** A material on the engine's PBR surface.
- **Right-click a shader → Create Material.** One that shades with that shader instead — see
  [Shaders](shaders.md).

Then pick it in the `MeshRenderer`'s **material** field — a search box over every material in the
project.

## The PBR surface

With no shader chosen, a material has the engine's metal/roughness PBR fields — the glTF
convention, so a material exported from Blender or Substance means the same thing here.

| Field | Default | What it is |
|---|---|---|
| `base_color` | white | The colour, multiplied into `albedo` when there is a map |
| `metallic` | 0 | 0 is a dielectric — plastic, wood, stone. 1 is bare metal. In between is rarely right |
| `roughness` | 0.5 | 0 is a mirror, 1 is chalk |
| `emissive` | 0 | Light the surface gives off, as a multiple of `base_color`. Not a light: it lights nothing else |
| `albedo` | — | The colour map |
| `normal` | — | A tangent-space normal map. None is the mesh's own normal |
| `metal_roughness` | — | Metal in blue, roughness in green (glTF). Multiplied into the two numbers above |
| `uv_scale`, `uv_offset` | 1, 0 | How many times the maps repeat, and where they start |

Textures themselves — import settings, tiling, why a floor goes blurry at a grazing angle — are
[Textures](textures.md).

**`emissive` is relative to the exposure.** The renderer divides it by the camera's exposure, so an
emissive sign reads as glowing in a dark room and in daylight alike. It does not cast light on what
is around it; put a point light beside it for that.

## The light model

Under **Light model** each material has the controls of the BRDF from *The Callisto Protocol*
(SIGGRAPH 2023). **Every default reproduces plain Lambert + GGX**, so a material you never open
there looks like any PBR engine's. They exist for surfaces that the standard model gets wrong:

| Control | What it does | For |
|---|---|---|
| Fresnel (`fresnel`, tint, falloff) | Brightens the diffuse where light grazes | Velvet, dusty cloth, peach fuzz |
| Retroreflection (`retro`, tint, falloff) | Brightens it where the light comes from behind you | Road signs, moon dust, some fabrics |
| Smooth terminator (`terminator`, tint, `terminator_length`) | Softens the line between lit and shadow | Skin, wax — anything where that line is too hard |
| `specular_falloff` | How much the highlight grows at grazing angles: 0.5 is Schlick | Taming a rim that is too bright |

These apply per light whatever the surface shader is, so a custom shader gets them too.

## Shared, so be careful what you edit

A material is an asset: change one in the Inspector and every mesh using it changes. To make one
crate red without the others, right-click the material → **Duplicate** and point that crate at the
copy.
