# ADR 0004 — Sombras a escala planetaria: dos bandas, no seis

**Status:** Proposed
**Date:** 2026-11-26
**Issue:** #782
**Relacionado:** #477 (VSM), #2009 (ancla del sol), #2022 (span por nivel), #2008 (bias de Unreal),
#2017 (borrar el sistema viejo), #1999 (LOD por distancia), #248 (atmósfera)

---

## Contexto

#782 listaba seis bandas candidatas (CSM, contacto, distance field, horizon maps, analítica,
eclipses) porque en agosto las sombras terminaban a 100 m. Eso cambió:

- **VSM es el único sistema** (#2017 borra el viejo). El clipmap del sol tiene 17 niveles, de
  1.28 m a **84 km** de extent: más que los 40 km de Unreal.
- **#2022** escala el rango de profundidad con el nivel (`max(2 km, extent × 100)`, el
  `ZRangeScale` de Unreal) y **#2008** porta el bias de Unreal. Los dos fallos que hacían que la
  sombra lejana se viera mal ya no existen.
- **El terreno planetario elige su LOD por distancia, nunca por frustum** (#1999). Los bloques de
  atrás de la cámara existen, con un nivel tosco. Un shadow map necesita justamente eso: los
  oclusores que están fuera de cuadro.

Unreal, leído en su fuente (`VirtualShadowMapProjectionDirectional.ush`, rama `release`), no
tiene respuesta pasado su último nivel: `TraceDirectional` devuelve *"Fully lit outside clipmap
range"*. Las Distance Field Shadows cubren eso solo para geometría no-Nanite (UE-364657).

## Decisión

**Dos bandas.** Cada una cubre una clase distinta de oclusor, así que se combinan con `min` y no
hay costura entre ellas.

### Banda 1 — Relieve: el clipmap VSM, extendido hasta cubrir el cuerpo más grande

El número de niveles deja de ser fijo (`22 − 6 + 1`) y pasa a salir de la escena: alcanza con que
el nivel más tosco cubra el diámetro del planeta más grande.

| Nivel | Extent | Texel | Qué cubre |
|---|---|---|---|
| 16 (hoy el último) | 84 km | 5.1 m | la escala de Unreal ×2 |
| 20 | 1342 km | 82 m | un continente |
| 24 | 21 475 km | 1.3 km | la Tierra entera (12 742 km) |

Por qué alcanza con eso:

- **El costo lo pone la pantalla, no el rango.** Cada píxel marca una página en *su* nivel, así
  que un nivel sin píxeles no marca nada. Desde órbita, la Tierra ocupa la pantalla con texels de
  ~1 km: la misma cantidad de páginas que mirar el suelo de cerca.
- **La memoria es la tabla de páginas.** Son 128² entradas más por nivel; ocho niveles más son
  ~131 k entradas, y el pool de páginas físicas no cambia.
- **La precisión ya está resuelta.** El ancla del sol es absoluta en f64 (#2009), y el span por
  nivel deja la profundidad en ~0.2 texel en cualquier nivel (#2022). En el nivel 24, el paso de
  profundidad en f32 es de ~256 m contra un texel de 1.3 km.
- **Los oclusores ya existen,** porque el LOD es por distancia (#1999). Una montaña a 300 km
  detrás de la cámara proyecta con su bloque tosco, a la resolución del texel que la recibe.

Así se cubre la autosombra del terreno a cualquier distancia, las crestas al atardecer y la
sombra de una nave sobre el suelo. Es el pedido de la #782, *"cuando existan naves que salgan del
planeta"*, con el sistema que ya existe.

### Banda 2 — Cuerpos: oclusión analítica por esferas

Para cada cuerpo con `Planet`, una prueba rayo-esfera hacia el sol contra una esfera de radio
`radius − depth`, debajo del relieve más bajo para no sombrear dos veces lo que ya resuelve la
banda 1. La penumbra sale del solapamiento entre el disco del sol (`sun_softness`) y el disco del
cuerpo vistos desde el punto.

Resuelve lo que un shadow map no puede o pagaría caro:

- **El terminador con exactitud:** un valle orientado al sol, pasado el terminador, queda en la
  sombra del planeta y no solo de su `N·L`.
- **Eclipses:** un planeta sobre su luna, una luna sobre su planeta, umbra y penumbra.
- **Una nave en órbita del lado nocturno,** que queda fuera de cualquier página del clipmap.

Cuesta una prueba rayo-esfera por cuerpo y por píxel; con N ≤ 8 cuerpos cargados no se mide
contra 13.9 ms. Referencia para el modelo de penumbra: Schneegans et al., *real-time eclipse
shadows*, CGF 2025 (MIT, CosmoScout VR). ⚠️ Pendiente de leer: el PDF supera el límite de
descarga de la herramienta.

### Combinación

`visibilidad = min(banda_1, banda_2)`. No hace falta transición: la banda 2 nunca ve el relieve
y la banda 1 nunca ve la esfera interior del cuerpo.

## Descartado, con el número

| Técnica | Por qué no |
|---|---|
| **Horizon maps** (Max 1988; Fourier-compressed, DLR / HPG 2025) | Precómputo offline: **150 horas de cómputo** para un set de Marte, ~3 min por parche. El terreno de Kóoch es procedural y va a ser editable: cada edición invalida el horizonte de todo lo que la ve. Su ventaja (no tener los oclusores en memoria) es un problema que el LOD por distancia ya no tiene |
| **Ray march sobre heightmap** | El terreno es un SDF con cuevas (`depth`), no un heightfield |
| **Ray march sobre el SDF del generador** | El ruido no es Lipschitz-acotado (`× 1500` de una fbm), así que el paso seguro es diminuto. Cientos de evaluaciones del generador por píxel |
| **Distance Field Shadows de Unreal** | Es la banda que ellos usan pasado su clipmap. La nuestra llega a cubrir el planeta, así que no queda hueco que llenar |
| **Más cascadas** | Es el sistema que se borra en #2017 |

## Fuera de alcance

- **Galáctico.** Una estrella a esa distancia es una luz direccional que nada ocluye, salvo los
  cuerpos, y eso lo cubre la banda 2.
- **Sombras de objetos sub-píxel a distancia,** como una nave a 1000 km: el texel del nivel que
  la recibe es más grande que la nave, y lo correcto es que no proyecte.

## Consecuencias

- **Dos soles** (`Casters.sun: bool`, ver #782) es un problema ortogonal a las bandas: cada sol
  necesita su clipmap. Va en su propia issue.
- Implementación, una issue por paso:
  1. Niveles del clipmap derivados del planeta más grande (banda 1).
  2. Oclusión analítica de cuerpos con penumbra (banda 2).
  3. `Casters.sun` como contador, con un clipmap por sol.
- **Riesgo a medir en el paso 1:** que el raster de los niveles toscos se trague el presupuesto
  cuando muchos bloques entran a la vez en una página (desde órbita, todo el hemisferio). La
  medición va en el body de esa issue antes de decidir un tope.
