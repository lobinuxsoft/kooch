# Lighting

How to light a scene. How the renderer does it — the shading model, shadow pages, clustering — is
[Lighting — Inti](../architecture/lighting.md); this page is what an author touches.

Lighting needs the `lighting` feature, which a project created by the editor already has.

## The lights

Add them from the World panel's spawn menu → **Lights**, or as components on any entity. The
entity's transform places and aims them; the gizmo shows which way a light points.

| Component | Is | `intensity` in |
|---|---|---|
| `DirectionalLight` | A sun: one direction, everywhere, no position | **lux** |
| `PointLight` | A bulb: every direction, out to `range` | **lumens** |
| `SpotLight` | A cone: `inner_angle` fully lit, fading to zero at `outer_angle` (half-angles, degrees) | **lumens** |

Hover `intensity` in the Inspector and the tooltip says which unit it is. They differ by orders of
magnitude, and they used to look the same.

`radius` on a point or spot light is the size of the bulb. It widens and softens the highlight on
shiny surfaces; it does not soften the shadow.

### Why the defaults are big

| Light | Default |
|---|---|
| `DirectionalLight` | 10 000 lux — daylight, not direct sun (100 000) |
| `PointLight`, `SpotLight` | 32 000 lm — about forty real household bulbs |

The sun's is a real number. The bulbs' is not: Kóoch computes **direct light only**, and most of
what lights a real room is light bouncing off the walls. A real 800 lm bulb renders as almost
nothing, so the default stands in for the bounces. It is named for that —
`lumens::ROOM_LIGHT_NO_GI` — and goes back to a real bulb when global illumination lands.

`kooch::kooch_ecs::light_consts` has the named values, so you pick a situation rather than a
magnitude: `lux::AMBIENT_DAYLIGHT`, `lux::DIRECT_SUNLIGHT`, `lumens::LED_BULB_9W`,
`lumens::ROOM_LIGHT_NO_GI`, `lumens::FLOODLIGHT`.

## Exposure and ambient: the project's render settings

Two things light a scene that are not lights, and both are fields of the project's
`.rendersettings` asset, edited in the Inspector:

- **Exposure** — `aperture_f_stops`, `shutter_speed_s`, `sensitivity_iso`, a real camera's three
  numbers. A scene that is all white is over-exposed, not over-lit: close the aperture (a higher
  f-number) before touching the lights. f/16 at 1/125 s and ISO 100 is a sunny day; f/2.8 is the
  default, between a sunny exterior and a room.
- **Ambient** — `ambient_sky_color`, `ambient_ground_color`, `ambient_intensity`. A fill from
  above and below, so a surface facing away from every light is dark rather than black.

The render settings ship with the game and belong in version control. They are the author's, not
the player's.

## Shadows

Every light casts shadows by default (`cast_shadows`). The sun's shadow is split into cascades
around the camera; spot and point lights render their own maps.

- **`contact_shadows`** adds a short screen-space march for the last few centimetres — where a
  foot meets the floor. On by default for the sun, off for the others.
- **How many cast at once is bounded.** With virtual shadow pages — the default — the bound is the
  page pool's budget. With them off it is four spot-light maps and 32 point-light cubes, ranked by
  distance to the camera. Either way a light past the budget keeps lighting and stops casting, and
  the editor says which of the two a shadowless light is.
- The quality settings — resolution, softness, the virtual shadow pages — are in the render
  settings and explained in the architecture page.

## Layers

`layers` says which surfaces a light lights; `shadow_layers` which surfaces cast into its shadow.
Both are bit masks, all bits by default. Untick a layer and that light ignores it: a light that
only lights the player, or a scenery mesh that casts no shadow from one lamp.

## The sky

A `SkyRenderer` draws the backdrop; the highest `priority` among the active ones wins. Its
`material` takes one whose shader is `kind: sky` — see [Shaders](shaders.md#sky). With none, or
one that cannot draw, the engine's own sky is shown.

The sky does **not** light the scene yet: ambient is the two authored colours above, not the sky
sampled. Making the atmosphere light what it surrounds is
[#250](https://github.com/lobinuxsoft/kooch/issues/250) and
[#248](https://github.com/lobinuxsoft/kooch/issues/248).

## Seeing what the lights do

The View's debug menu has views for what the lighting computed — shadow cascades, the virtual
shadow pages, how many lights each pixel pays for. The architecture page shows what each one
means.
