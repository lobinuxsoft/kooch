# ADR 0002 — La cámara no tiene plano far

**Status:** Accepted
**Date:** 2026-08-08
**Issue:** #735 (contact shadows), PR #752
**Afecta:** #732 (upscaling temporal), #248/#250 (atmósfera), #254 (post), y todo lo que lea depth
**Relacionado:** #488 (reversed-Z), #476 (CSM)

---

## Contexto

`perspective_rh_reverse_z(fovy, aspect, near, far)` construía la proyección de la cámara con
un plano far **finito**. Funcionaba, y siguió funcionando hasta que #735 tuvo que portar el
ray march de Bevy sobre el depth buffer.

Bevy construye `Mat4::perspective_infinite_reverse_rh(fov, aspect, near)`
(`bevy_camera/src/projection.rs:339`) — **sin far**. Sobre esa base montan todo su stack de
profundidad:

| Helper de Bevy | Qué asume |
|---|---|
| `perspective_camera_near()` (`view_transformations.wgsl:166`) | es literalmente `clip_from_view[3][2]` |
| `depth_ndc_to_view_z()` (`:172`) | es `-near / ndc_z` |
| `bevy_pbr::raymarch` | `ray_depth = 1.0 / ray_point_cs.z` |

**Las tres identidades sólo valen con far infinito.** Con far finito la relación es
`linear_z = A / (B − ndc_z)` con `r = far/(near−far)`, `A = r·near`, `B = 1+r`.

El primer intento de #735 portó el march conservando el far finito y llevando esos dos
coeficientes en el uniform. Compilaba, corría, y tenía una mentira adentro: **el parámetro
`thickness`, documentado en metros, significaba una distancia distinta en cada escena**
según dónde estuviera el far. Es la clase de error que se lee como "hay que tunearlo" y se
tunea para siempre.

Y no era un problema de contact shadows: SSR, SSAO, fog, el upscaling temporal (#732) y la
atmósfera (#248) leen depth exactamente igual. Cada uno se habría encontrado lo mismo, y
cada uno lo habría resuelto por su cuenta o se habría olvidado.

## Decisión

**La cámara renderiza con `perspective_infinite_rh_reverse_z(fovy, aspect, near)`.**

Consecuencia que compra todo lo demás: **`ndc.z` es exactamente `near / distancia`**. Un
shader recupera metros con una división y ningún uniform extra.

`perspective_rh_reverse_z` (finita) **sobrevive** para el único trabajo que necesita un
frustum acotado: fitear cascadas de sombra a una rebanada de la vista. Una rebanada de un
frustum ilimitado es ilimitada. Bevy hace lo mismo: sus cascadas fitean contra distancias de
split explícitas, no contra la proyección de la cámara.

`PerspectiveCamera.far` **se conserva** como campo. Ya no clipea nada de lo que la cámara
dibuja; lo leen el fit de cascadas (acotado además por `shadow_distance`) y el gizmo de
frustum del editor.

## Consecuencias

### Lo que NO hubo que tocar, verificado

- **El frustum cull.** Sin far, la fila `ndc.z >= 0` degenera a una normal de longitud cero.
  `extract_frustum_planes` ya devolvía `[0,0,0,0]` en vez de dividir por ella, y el shader ya
  recorría **5 planos**. Pinneado por
  `the_vanished_far_plane_culls_nothing_instead_of_producing_nan`.
- **Hi-Z**, el depth clear a `0.0` y el comparador `Greater`: reversed-Z no cambió, sólo se
  fue el far.
- **Las cascadas** (#476), por `projection_to`.

### Lo que sí, y era un bug esperando

- **Picking.** `viewport_cursor_to_ray` desproyectaba en `ndc.z = 0`, que ahora es el
  infinito y desproyecta a `w = 0`: **todo click habría devuelto `None`**. Usa el plano near,
  que está sobre el mismo rayo por el ojo y es finito con cualquier proyección.

### Precisión

Reversed-Z gasta el exponente del float donde está el ojo. Un far finito además gasta parte
del rango describiendo la distancia entre `far` y el infinito, que no dibuja nada. Sacarlo
es estrictamente mejor, y para escala planetaria elimina un número que alguien tenía que
elegir bien.

## Alternativas consideradas

**Conservar el far y llevar los dos coeficientes de linearización a cada consumidor.** Es lo
que hizo el primer intento. Funciona y se paga en cada técnica futura, con la garantía de que
alguna se lo va a olvidar — y el síntoma es un parámetro en metros que no mide metros.

**Portar los shaders de Bevy adaptando sus fórmulas de profundidad.** Se descartó por la
regla del proyecto (2026-08-07): la parte gráfica se porta **literal**, porque una reescritura
equivalente-en-papel ya costó un `select` invertido que sólo apareció construyendo una vista
de debug. Adaptar sus fórmulas es reescribir la parte más sutil de su código.
