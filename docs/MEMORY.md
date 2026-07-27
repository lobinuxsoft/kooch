# oh_my_engine — Project Memory (agent briefing)

> **READ FIRST** al retomar el engine desde cualquier máquina (Linux/Windows).
> Fuente de verdad = este archivo + issues de GitHub. Consolidado de la memoria del asistente el 2026-07-13.
> Los file:line pueden quedar stale — verificá contra el código antes de afirmar.

---

## Qué es

Game engine experimental en **Rust Edition 2024**, arquitectura híbrida CPU-GPU:
gameplay/lógica en CPU, física/render en GPU. Diseñado **planet-scale + GPU-driven**
desde la foundation — todo se evalúa contra (a) escala planetaria, (b) GPU-driven hot
loop sin CPU readback en frame.

- **Licencia:** All Rights Reserved (repo privado/personal `lobinuxsoft/oh_my_engine`).
- **Branches:** `main` (release-please) ← `development` (integración) ← `feat/*`.
- **Disambiguación:** `oh_my_engine` = motor (underscore). `oh-my-agent` = CLI coding agent
  con LLMs embebidos (guiones). Ambos en `/var/mnt/DATA/Repos/`. Si el user dice solo
  "oh my", preguntar cuál.

## Stack

wgpu **29**, winit, kira (audio), gilrs (gamepad), rhai (scripting), egui/eframe **0.34**,
rapier3d 0.22 (física), meshopt 0.6, metis (vendored, meshlet grouping), glam, bytemuck,
slotmap, gltf, image. PipelineCache vía wgpu unsafe + fallback. **Stay-on-wgpu 24 meses
mínimo** (audit #239).

## Workspace (crates)

`ome_core, ome_ecs, ome_window, ome_input, ome_lighting, ome_render, ome_physics,
ome_gravity, ome_world, ome_audio, ome_scripting, ome_editor_core, ome_editor,
ome_gizmos, ome_gizmos_handles, ome_editor_api`. Facade top-level `oh_my_engine` con
`DefaultPlugins` PluginGroup (estilo Bevy).

`ome_sdf` **ELIMINADO (2026-07)**. Contenía dos cosas sin relación: la librería de
primitivas CSG en WGSL, que alimentaba el raymarcher ya borrado en el pivot, y el
almacenamiento de vóxeles disperso con LOD (#136). Lo primero murió con su renderer;
lo segundo era el sustrato de Phase 2.5 y se mudó a **`ome_world::voxel`**, que es
donde vive el resto del streaming.

`ome_bvh` **ELIMINADO (2026-07)** junto con `ome_world::content` y el
`ProceduralCitySource`. Era la estructura de aceleración del raymarcher — sus flags se
llamaban `IS_RAYMARCH` / `ROLE_RAYMARCH_*` — y su único consumidor era contenido de chunk
que nadie rendeaba desde el pivote: el path meshlet tiene su propio culling con Hi-Z. El
`Aabb` se mudó a `ome_core`, que es donde correspondía. ~14k líneas.

**El mundo contiene escenas, NO al revés (#566, corregido 2026-07-21).** Verificado contra
los engines ECS: Bevy escribe `Scene`/`DynamicScene` DENTRO de un `World`
(`write_to_world_with`); el `World` de Unity DOTS contiene la meta-entidad de escena y sus
section entities; el `UWorld` de Unreal contiene levels → UE5 World Partition = grilla
espacial. El contenedor runtime es el mundo; las escenas son contenido que se carga en él.

**Escena y celda son ORTOGONALES, no anidadas.** Una escena abarca muchas celdas y varias
escenas pueden solaparse en una celda. Celda = "¿qué hay cerca?" (espacial, derivado de la
posición). Escena = "¿qué autoró junto un diseñador?" (lógico, decisión humana). Una entidad
pertenece a UNA escena y a UNA celda, independientemente. **Storage indexado por
`(escena, celda)`** = los archivos per-scene-section de Unity. Anidarlos costaría: escenas
solapadas en el mismo volumen, dos personas editando sin conflicto en git, y descargar una
escena vs descargar una celda como operaciones distintas.

Reglas: ownership **espacial y derivado** del transform, nunca campo guardado (se
desincroniza). **Residencia ≠ existencia**. **Sección siempre residente** estilo section-0
de Unity (carga primera, descarga última) = ahí vive la entidad de focus, que no puede ser
dueña de una celda que se descarga sola. Depende de #139 (bridge de física).

Criterio aplicado (del usuario, 2026-07-21): **se queda lo que sirve al sistema galáctico
y a planetas terraformables; lo demás muere y se rehace bien contra el motor de física
actual.** Nada de "tal vez lo usemos". Si hace falta algo de ahí: cherry-pick del historial.

**No confundir los dos "SDF"**: el descartado es SDF-como-técnica-de-render/authoring
(brushes CSG, raymarching). El vigente es SDF-como-formato-de-dato — cada vóxel guarda
una distancia con signo, y de ahí extrae malla el dual contouring (#393/#397/#398). Para
física tampoco hace falta lo primero: el collider de vóxel de rapier 0.34 consume la
grilla directamente.

---

## ⭐ Entrega de eventos: NUNCA funcionó hasta #630 (locked 2026-07-27)

**Todo tipo de evento que no fuera de una lista hardcodeada se enviaba y jamás se leía.**
`Events<T>` tiene buffer doble y está bien; lo que faltaba era **alguien que llamara a
`update()`**. Había **dos** listas hardcodeadas que las dos se declaraban generales:

- `runner.rs::update_events` rotaba **sólo `AppExit`**.
- `winit_app.rs::update_events` rotaba tres tipos, bajo un comentario que decía
  *"Swaps double buffers for all registered event types"*. Ese es el runner del **editor**.

Así que `WindowResized`, `WindowCloseRequested` y todo lo de física estaban muertos. Vivió
cuatro meses (desde `5de14cb`, 2026-02-03) porque `AppExit` era el único evento que alguien
leía.

**`EventRegistry` era la solución pensada y nunca estuvo viva.** `add_event` insertaba un
`Events<E>` suelto en `Resources` (que ya es un mapa type-erased, así que el registry
duplicaba). Y su `update_all` hacía downcast a `Box<dyn EventsUpdatable>` — un trait
**declarado y jamás implementado**, así que el tipo destino no existía. Su propio test lo
confesaba: *"We can't easily call update_all because of the trait object complexity"*.
**Borrado en #630.**

**La solución: `add_event` registra CÓMO rotar.** Un `fn(&mut Resources)` monomorfizado por
tipo en `EventUpdaters`; los runners llaman a `update_all_events`. Un tipo nuevo se entrega
porque alguien llamó a `add_event`, no porque además editó dos runners.

**Dedup por `TypeId` obligatorio:** `AppExit` se registra DOS veces (`App::new` y
`CorePlugin`). Rotar dos veces en un frame descarta lo escrito entre los dos swaps.

**Lo encontró el smoke, no los tests.** Los unitarios de #561 pasaban porque el harness rota
los buffers a mano. Sólo el `App` real lo mostró: 0 eventos con la física produciéndolos.

---

## Estado (2026-07-26, rama `feat/physics-events`, mergeada)

**#561 eventos, sensores y grupos.** #560/#618/#563/#623 mergeados.

**Antes de esto la simulación corría CIEGA:** `step` recibía `&()` como event handler, así que
nada en el engine podía enterarse de que dos cosas se tocaron. Física sí, gameplay no.

**Tres cosas no obvias que resolví:**

1. **`EventHandler` toma `&self` y exige `Send + Sync`** — rapier lo llama desde dentro del
   step, posiblemente desde threads del solver. El colector necesita mutabilidad interior que
   además sea `Sync`. Rapier usa canales de crossbeam; usé `Mutex<Vec<_>>` y me ahorré la dep.
2. **`InteractionGroups::new` toma TRES argumentos en 0.34** — hay un `test_mode` nuevo. Uso
   `And` (los dos lados tienen que acordar). `Or` deja pasar un reclamo unilateral, que es lo
   que vuelve indebugueable un filtro.
3. **No hay evento de stop.** `Playing` es un flag, así que el plugin recuerda el estado
   anterior y limpia en la transición, con un sistema **sin gate de play** — uno gateado no
   puede ver que el play terminó.

**El colector recolecta DURANTE el step y se drena DESPUÉS.** No es prolijidad: un listener
que despawnea lo que acaba de tocar estaría mutando el set que se está iterando.

**`FieldKind::Bitmask` NO se agregó — es metadata `bits` en `FieldMeta`**, igual que `choices`
convierte un `u32` en dropdown. El almacenamiento sigue siendo `u32`, así que un kind nuevo
sería un caso que todo consumidor tendría que manejar y que se lee/escribe idéntico a `U32`.
**El widget preserva los bits que no nombra** — un mask autorado a mano sobrevive una visita.

**El evento del joint roto de #560 por fin tiene dónde ir:** `JointBroke`.

---

## Estado (2026-07-26, rama `feat/collider-material`, mergeada)

**#623 material de collider.** #560, #618, #563 mergeados; #626 y #627 también.

**Antes de esto no había NADA autorable de material.** Todo collider tomaba la fricción 0.5
y restitución 0.0 de rapier, y todo cuerpo damping 0.0. Un `grep` de `ome_physics` sólo
encontraba `damping` en los joints. Una escena se comportaba de un modo que nadie eligió y
nadie podía cambiar.

**Comportamiento NO OBVIO de rapier que hay que saber antes de autorar:** la regla de
combinación de un par se resuelve con `rule1.max(rule2)` **sobre los discriminantes**. Un
collider en `Average` (0) contra uno en `Max` (3) usa `Max`. La regla no se negocia — gana la
más insistente. Está documentado en `CombineRule` y en el tooltip del Inspector.

**Damping ≠ fricción.** El damping actúa sin contacto (aire); la fricción sólo en contacto.
Y confirma lo de #618: "rota lento" **no podía** ser damping, porque el default de rapier es
0.0 y nada lo estaba amortiguando.

**Hueco cerrado de paso:** el trait tenía `linear_velocity`/`set_linear_velocity` pero **no
angular**. Lo descubrí porque mi propio test de damping pasaba midiendo 0 contra 0 — sin
`set_angular_velocity` no había forma de hacer girar un cuerpo. Agregados los dos.

**El material del hijo es del hijo:** un parche de hielo soldado a un cajón sigue siendo
hielo, así que `Attachment` lleva su propio `SurfaceMaterial` y el digest lo cubre — editar la
fricción de un hijo reconstruye el cuerpo padre.

Baseline medido: mismo empujón de 8 m/s, fricción 0.02 → **20.766 m**; fricción 1.5 →
**3.475 m**. Spin de 10 rad/s con damping 4 → **0.0005 rad/s**.

---

## Smoke headless de física — `cargo run --example physics_smoke`

```
cargo run --example physics_smoke --no-default-features \
    --features physics,physics-debug-render
```

Corre el `App` de verdad — plugins, schedule, timestep fijo — sin ventana ni GPU, y
**imprime lo que el solver terminó teniendo**: masas, centros de masa, el ángulo al que
frenó la puerta, si el joint frágil se soltó, y cuántos segmentos dibuja cada categoría del
overlay. Es lo más cerca de un smoke automatizable que hay hoy.

**⚠️ TRAMPA que ya mordió una vez: contar frames NO mide tiempo de física.** El runner por
defecto gira tan rápido como puede y los pasos fijos se acumulan por tiempo **real**. La
primera versión del ejemplo corrió 240 frames en **10 ms** y reportó — correctamente — que
nada se había movido. Hay que esperar `Time::fixed_count()`, no `frame_count()`.

Baseline verificado 2026-07-26 (240 pasos, 4 s simulados):

| | |
|---|---|
| cubo 1 m y esfera r=2, ambos 3 kg autorados | 3.0000 kg los dos, CoM en el origen |
| compuesto con collider hijo a 4 m | 3.0000 kg, CoM **no** arrastrado |
| puerta con límite 0.4 rad | frena en 0.4000 |
| joint de 0.02 bajo 5 kg | se suelta; la carga cae a y=0.499 |
| overlay | 27 contactos / 24 CoM / 3 anclas / 108 AABB / 132 shapes / 0 apagado |

Las anclas de joint pasan de 6 a 3 segmentos cuando uno se rompe — el overlay es coherente
consigo mismo.

---

## Estado (2026-07-26, rama `feat/physics-debug-render`)

**#563 debug render.** #560 y #618 mergeados (PRs #621, #622).

**Rapier NO dibuja nada.** `DebugRenderPipeline` recorre el mundo y llama a un
`draw_line(object, a, b, color)` tuyo — es el único método obligatorio del trait. Te da la
teselación ya hecha (una esfera llega como segmentos, no como centro y radio), el recorrido
completo, y el color por estado en **HSLA con hue en grados**. Es un productor de líneas.

**Esto NO es un segundo contorno de collider.** `ColliderVisualizer` ya dibujaba colliders
desde los componentes del ECS. Eso es la misma cuenta hecha dos veces: si el cuerpo nunca se
construyó, o se construyó de un spec viejo, el gizmo dibuja la forma que *debería* existir y
no puede decir nada de la que hay. **Cuando los dos difieren, esa diferencia es el bug.** Por
eso `collider_shapes` va apagado por defecto — prenderlo es un acto de comparación.

Lo que sólo el solver sabe y no se derivaba de ningún componente: **contactos**, **centro de
masa**, **anclas de joints**, **AABBs de broad-phase**, y **sleep state** (gratis: rapier
oscurece con `sleep_color_multiplier`).

**`debug-render` es feature de cargo de rapier**, envuelta en una de `ome_physics` que sólo
prende el editor. Un build de juego ni la compila — #558 aplicado donde sale más barato. El
método del trait tiene impl por defecto vacía: un backend que no puede introspeccionarse dice
nada en vez de inventar una aproximación.

**El overlay es whole-world, así que va ANTES del early-return por selección** en
`build_gizmo_batch_system`. Las preguntas que contesta se hacen justamente cuando no hay nada
seleccionado. Y con todo apagado el backend ni se llama: "cuesta cero apagado" tiene que
significar que el recorrido no pasa.

Bajé `subdivisions` de 20 a 12: son ~60 segmentos por esfera generados y subidos por frame.

---

## Estado anterior (2026-07-26, rama `feat/physics-mass-control`, PR #622 mergeado)

**#618 mass control.** #560 ya mergeado (PR #621).

**El bug de unidades que el issue NO había identificado:** `RigidBody.mass` decía
*"Mass in kilograms"* y en realidad era *masa adicional sobre la que el volumen del collider
implica a densidad 1.0*, porque `RigidBodyBuilder::additional_mass` de rapier **suma**.
Medido: esfera r=0.5 con `mass = 1.0` pesaba **1.52 kg**; esfera r=2.0 pesaba **34.5 kg**.
O sea "se ve lento" tenía dos causas — el tensor de inercia creciendo (física correcta, lo
que decía el issue) *y* la masa total creciendo con cada shape.

**Decisión (opción A del issue): los colliders no aportan masa.** `mass` es la masa entera y
única autoridad. Un campo que dice kilogramos tiene que significar kilogramos.

**Cómo se arma sin degenerar la inercia** — el detalle que casi muerde: con colliders a
`density(0.0)`, un `additional_mass` escalar escala una inercia que vale cero → aceleración
angular infinita. La forma correcta es `additional_mass_properties` con un `MassProperties`
sacado de la **forma del collider del padre a densidad 1.0** y después `set_mass(m, true)`.
Verificado: masa exacta, centro de masa en el collider del padre (no arrastrado por un hijo
descentrado a 4 m), inercia `0.3 = ⅖·m·r²` correcta y finita.

**`density` es campo de autoring y NO entra en `BodySpec`.** La simulación nunca lo lee; sólo
lo lee el botón *Calculate mass*. Si entrara en el spec, editarlo retiraría el cuerpo y le
tiraría la velocidad en pleno play por un número que el solver ni mira.

**Las mass properties de rapier quedan STALE hasta el primer step.** El editor autora un
mundo que no simula, así que `add_body` llama explícitamente a
`recompute_mass_properties_from_colliders`. Sin eso, el debug render de #563 dibujaría el
punto equivocado.

**El botón no necesitó protocolo nuevo:** `EditorAction::SetField` ya existía, así que el
undo sale gratis y anda igual contra un proyecto remoto — el volumen se calcula del snapshot
del Inspector. El recorrido de hijos **frena en cualquier descendiente con `RigidBody`**,
misma regla que `compound.rs::attachments_for`; si divergen, el botón reporta la masa de un
cuerpo que el solver nunca arma.

---

## Estado anterior (2026-07-26, rama `feat/physics-joints`, PR #621 mergeado)

**#560 joints implementado.** Un componente `Joint` con discriminante reflejado y campos
por tipo vía `FieldCondition` — el patrón de `Collider`, no un componente por tipo de joint.
Los ocho tipos de rapier, impulse y multibody, motores, límites y breaking.

**Dos cosas que el issue prometía y rapier 0.34 no da tal cual** — verificado contra el
source, no contra la doc:

- **`PinSlotJoint::new` es `#[cfg(feature = "dim2")]`.** No existe en 3D. El equivalente
  espacial es un joint **cilíndrico** (desliza y gira sobre el mismo eje) y se arma con el
  `GenericJointBuilder` del propio rapier bloqueando `LIN_Y|LIN_Z|ANG_Y|ANG_Z`. Sigue siendo
  rapier; no es dinámica propia.
- **Joint breaking no existe en rapier.** Lo único que expone es `ImpulseJoint.impulses`.
  Implementado leyendo ese impulso después del step y removiendo la constraint. Leer la
  salida del solver no es un segundo solver — pero **sólo funciona para impulse joints**: un
  multibody se resuelve en coordenadas reducidas, donde no hay impulso de constraint que
  medir. Warning en el Inspector para esa combinación.

**Convención propia y documentada:** límites y motor actúan sobre **un solo eje**, el eje
libre primario del joint (angular para revolute/spherical/generic, lineal para
prismatic/pin-slot; ninguno para fixed/rope/spring). Exponer los seis DoF sería seis veces
la superficie de Inspector para un caso que casi no aparece. Un cono de swing de ragdoll es
lo que esto NO cubre.

**El umbral de breaking se mide sobre el impulso LINEAL solamente.** Rapier reporta seis
componentes — tres de fuerza, tres de torque — y una norma sobre los seis suma newton·s con
newton·m·s: un número contra el que nadie puede escribir un umbral.

---

## Estado anterior (development HEAD `b2e4ddc`, 2026-07-26)

**Sesión 2026-07-26 — cerrado:** #605 (el ECS propio se queda), #607 (identidad de
entidades), #609 (multi-escena), #612 (física en emparentados: compound + gizmo + warnings).
**1035 tests verdes, cero PRs abiertos.**

**Hallazgos del smoke que quedaron como issues** — leer antes de retomar física o editor:

- **#619 — no se puede crear una escena nueva desde el editor.** No hay botón. El template
  (Camera + Sky) existe pero sólo lo usa `project.rs` al abrir un proyecto. Bloquea probar
  multi-escena sin tener archivos ya en disco.
- **#618 — el centro de masa de un cuerpo compuesto sorprende.** Con hijos aportando
  colliders, el centro de masa cae entre las formas y el cuerpo rota lento. **No es un bug:**
  es física correcta — más formas lejos del centro = tensor de inercia más grande = menos
  aceleración angular por el mismo torque. Lo que falta es **control**, y Rapier lo da
  (`ColliderBuilder::density(0)` para forma sin masa, `additional_mass_properties` para
  centro explícito). La decisión pendiente es el **default**.
- **#560 joints — DESBLOQUEADO y ahora urgente.** #607 quitó el bloqueo (dos `EntityRef` por
  joint ya se serializan). Y el warning de #617 le dice al autor "usá un joint" para un
  joint que todavía no existe. Rapier 0.34 trae: fixed, revolute, prismatic, spherical,
  rope, spring, pin_slot, generic + impulse/multibody + motores + límites + breaking.

**Principio que el user enunció y conviene recordar:** *todo lo que Rapier ofrece debería
terminar teniendo un componente.* Joints (#560) es el hueco más grande; scene queries
(#562), eventos y sensores (#561) son los otros ya fileados.

---

## Estado anterior (development HEAD `26ceeb0`, 2026-07-25)

- **#605 CERRADO: `ome_ecs` se queda.** `bevy_ecs` evaluado con mediciones y descartado;
  ver la decisión sticky abajo. El ECS se mejora en el lugar, ordenado por dolor
  demostrado.
- **#609 en curso (`feat/multi-scene`): varias escenas abiertas a la vez.** El world es el
  contenedor, las escenas son contenido. Ver la decisión sticky abajo.
- **#607 MERGEADO (PR #608): identidad de entidades.** Un componente ya puede
  apuntar a otra entidad y sobrevivir un guardado. `Parent` dejó de ser un caso especial y
  `parent_index` quedó como lectura legacy. Desbloqueó #560 y las referencias cross-scene.
- **Épico de física CERRADO en lo esencial.** #139 mergeado (#570): `PhysicsPlugin` con
  componentes `RigidBody`/`Collider`, sync de vida, step en el timestep fijo, writeback a
  `Transform`, y Stop reconstruyendo el mundo físico desde el ECS restaurado. Smokeado por
  el user: un cubo cae, Stop lo restaura.
- **14 PRs mergeados el 2026-07-25.** Física, primitivas de malla + exportador GLB, gizmos
  de collider y de luces, panel de gizmos, campos condicionales del Inspector, y **cinco
  bugs de remote mode / jerarquía** que el user encontró usando el editor.
- **⏳ PR #604 ABIERTO** — borrado del path GPU del ECS. Smokeado indirecto (nada lo usaba).
- **⭐ PRÓXIMA SESIÓN = #605**, decidido por el user: evaluar adoptar `bevy_ecs`. Ver la
  sección nueva de decisiones. **No arrancar #137 antes de eso.**

### ⭐ Lo que se hizo el 2026-07-25 (14 PRs)

| PR | Qué |
|---|---|
| #570 | `PhysicsPlugin` (#139) + orden del frame + borrado de 4 componentes legacy + editor autoriza física + `Transform.scale` en el collider |
| #572 | el refresh del mirror pisaba el drag del gizmo (#571) |
| #575 | 6 primitivas como assets `.glb` + exportador GLB + simplificación con meshopt (#573) |
| #577 | `SpawnMesh` sin ruta en remote mode (#576) |
| #579 | gizmos wireframe de colliders (#574) |
| #583 | `Collider.center` — offset local (#580) |
| #584 | panel 👁 Gizmos por grupos + colliders siempre visibles (#581) |
| #586 | campos del Inspector según el shape — `FieldCondition` (#585) |
| #588 | gizmos de TODOS los seleccionados, no sólo `Transform` (#587) |
| #590 | el `Spawn` remoto tiraba su lista de componentes — la luz nacía inerte (#589) |
| #594 | gizmos de point/spot/directional + segmentos de círculo derivados del radio (#593) |
| #596 | auditoría de `EditorAction` vs `classify`: `Reparent` y `Duplicate` sin ruta (#595) |
| #598 | el menú add-component leía el registry del editor, no el schema del proyecto (#597) |
| #600 | el mirror nunca borraba un parent que el proyecto dejó de reportar (#599) |
| #602 | la escena referenciaba el padre **por nombre** → jerarquía mal al cargar (#601) |

### ⚠️ El patrón que dominó la sesión: remote mode se armó por fases

Cinco bugs de la misma forma —#571, #576, #589, #595, #599— todos **silenciosos** y todos
encontrados por el user clickeando, no leyendo código. Causa: el épico C se armó en fases
(#548→#557) y cada acción que nadie ejercitó en su fase quedó sin ruta y sin test.

La auditoría de #596 recorrió los 38 variantes de `EditorAction`. **Diez de los doce que
mutan el ECS estaban bien ruteados**; los dos que no eran `Reparent` (mutaba el mirror y se
revertía al refresh) y `Duplicate` (no-op total). Ya no debería quedar nada de esa forma.

### Épico de física — backlog auditado contra el source de rapier 0.34 (2026-07-22)

Cobertura verificada módulo por módulo. **#139 CERRADO el 2026-07-25 (#570)** — era el
bloqueante, ya no lo es. El siguiente de la lista es **#137** (todas las shapes detrás de la
abstracción), que ahora tiene sus tres dependencias adentro: primitivas para derivar hulls
(#575), wireframes para ver el resultado (#579), y `FieldCondition` para que trece shapes
sean legibles en el Inspector (#586). **Pero primero va #605.**

| Capacidad de rapier 0.34 | Issue |
|---|---|
| `dynamics` — rigid bodies, step, integration params | **#139** ⭐ BLOQUEANTE |
| `geometry` — colliders, shapes (incl. **voxels**), material, grupos | #137 |
| `dynamics/joint` — 7 impulse + multibody + motores + límites + ruptura | #560 |
| `pipeline` — event handler, hooks, sensores | #561 |
| `pipeline/query_pipeline` — shape casts, proyección, intersecciones | #562 |
| `pipeline/debug_render_pipeline` | #563 |
| `control/character_controller` | #94 (body pide reescritura: especifica SDF) |
| `control/ray_cast_vehicle_controller` | #105 |
| `control/pid_controller` | #567 |
| `island_manager` — regiones activas por distancia | #311 |
| `counters` — timings por etapa | #569 |
| `enhanced-determinism` vs SIMD — **decisión pendiente** | #568 |
| Softbody — **rapier NO lo tiene** | #107 (corregida) |

**#568 es lo más importante que nadie decidió**: `enhanced-determinism` y SIMD son
mutuamente excluyentes (`compile_error!` en rapier). Hoy hay `simd-stable`, o sea el
determinismo cross-machine está APAGADO sin que nadie lo eligiera. Sin él quedan fuera
netcode lockstep, replays como log de inputs y revalidación server-side. Cambiarlo es un
flag; lo caro es el netcode que se construya asumiendo lo contrario.

**Gotcha de diseño para #139 (corregido antes de implementar):** el mapeo entidad↔cuerpo
NO va como `HashMap<Entity, BodyHandle>` — viola las reglas DOD del proyecto. Va como
componente POD `PhysicsBody(u32)` en el archetype + `Vec<Entity>` indexado por body index
para el inverso. El sync pasa a ser un recorrido lineal sobre `(PhysicsBody, Transform)`.
Con Rapier la capa de sync es **obligatoria** (Rapier es dueño de sus `RigidBodySet` /
`ColliderSet`; no puede leer arrays ajenos) — el modelo ECS-nativo tipo Avian no está
disponible porque Avian está soldado a `bevy_ecs`.

### Épico C — Editor remoto (BRP-style) — CERRADO (fases 1-5 mergeadas)

> **Nota 2026-07-25:** cerrado no significa sin deuda. Cinco bugs silenciosos de esta
> arquitectura salieron el 2026-07-25 (#571, #576, #589, #595, #599) y la auditoría de #596
> recorrió los 38 variantes de `EditorAction`. El histórico de fases queda abajo como
> contexto de *por qué* el código está armado así.

**Problema raíz:** el editor standalone (`ome_editor`, launcher hub) NO puede cargar escenas
con componentes del proyecto (`unknown component type: test3::...::MoveComponent`). No es bug:
es un editor genérico compilado antes que el proyecto exista; Rust resuelve tipos en
compile-time, no hay ABI dinámica estable. **Solución (elegida por el user, "no alambre"):
protocolo remoto tipo Bevy Remote Protocol.** El binario del PROYECTO es dueño del ECS y
responde por HTTP; el editor pasa a cliente delgado.

**Arquitectura clave (locked):** UNA fuente de datos (ECS local, real en modo local o
ESPEJO en modo remoto vía `RemoteMirror`) + UN seam de bifurcación (`apply_actions` →
`remote_edit::dispatch` cuando `RemoteState::is_connected()`). Paneles/DTOs/viewport
IDÉNTICOS en ambos modos. Modelo de viewport = **C** (el hub rendea el espejo con su
viewport, que ya rendea meshes bien). Estrategia de entrada = **A→B**: hoy "Open Remote"
separado; cuando el remoto madure, "Open Project" pasa a remoto (borrar un botón, no
reescribir).

**Fases (todas MERGED salvo la última):**
- ✅ **1 — componentes dinámicos** (#548). `ome_ecs::dynamic_components`: componente sin tipo
  local se PARKEA (no aborta la carga, no vacía el mundo). Round-trip intacto. Mató pérdida
  de datos real. `SceneError::UnknownComponent` eliminado.
- ✅ **2 — identidad portable** (#549). `ome_ecs::component::{ComponentId, ComponentNames}`:
  interner nombre↔u32 process-local. `EditorAction`/DTOs llevan `ComponentId`; `TypeId` se
  queda en reflexión local. Seam `dispatch::action_to_command` resuelve ComponentId→TypeId.
- ✅ **3 — crate `ome_remote`** (#550). Server `tiny_http` SÍNCRONO (thread dedicado + puente
  mpsc al main, NO async, NO tokio) + `RemoteClient` blocking (std::net). `protocol` (serde,
  componentes por nombre, Entity=(index,generation), reusa ReflectValue). Métodos:
  ping/list_entities/get_schema/set_field/add/remove/spawn/despawn/save/load. DEFAULT_PORT 15703.
- ✅ **4a — modo `--remote`** (#551). Facade: dep opcional tras feature `remote` + prelude.
  Scaffold main.rs 3er modo `cargo run -- --remote` = DefaultPlugins + RemotePlugin +
  run_systems:false. Verificado vs TEST3 real por HTTP.
- ✅ **4b.1 — `RemoteSession`** (#552). `ome_editor_core::remote_session`: lanza el proyecto
  (patrón PlayState + env OME_ENGINE_ROOT/OME_PROJECT_ROOT), handshake poll_ready, cachea
  snapshot+schema.
- ✅ **4b.2 — `RemoteMirror`** (#553). Reconstruye el snapshot en el ECS local del editor,
  keyed por `EntityId` (NO por nombre; 5 "Mesh" duplicados). Engine components → reflected
  reales (rendean); proyecto → parkeados (DynamicComponents). Marker `MirrorEntity`.
- ✅ **4b.3a — dual-sink dispatch** (#554). `RemoteState` (session+mirror). `apply_actions`
  chequea is_connected → rutea EditorAction al RemoteClient (`remote_edit::dispatch`; traduce
  Entity local→remoto vía `mirror.remote_of`, ComponentId→nombre). UN solo if. Test e2e:
  SetField en el editor llega al server.

**⏳ LO QUE FALTA:**
- **4b.3b — ACTIVACIÓN (próxima sesión, la que hace TODO visible):** insertar `RemoteState`
  en `EditorPlugin`; system que poll_ready + refresh (cada N frames) + `mirror.apply` sobre el
  ECS del editor; registrar `MirrorEntity` como ephemeral; entry point UI "Open Remote" en
  `launch_screen.rs` (junto a "Open Project"). El viewport rendea el mirror automáticamente
  (ya rendea el ECS local). **Necesita smoke visual del user.**
- **Mostrar DynamicComponents parkeados en el Inspector** (deuda fase 1): `queries.rs` hoy solo
  lee componentes registrados del archetype → los componentes del proyecto son invisibles en
  el Inspector remoto. Necesario para authoring completo.
- **Fase 5 — ciclo de vida:** Play/Stop remoto, reconexión tras recompilar, "Rebuild & Relaunch".
- **Convergencia A→B:** cuando el remoto madure, "Open Project" → remoto por defecto.

**Deuda no-épico acumulada (arreglable suelta):** `Duplicate` de entidad no copia componentes
parkeados; drag `.glb`/`.gltf` al slot de mesh (pedido); `Delete` de asset sin confirmación
(#439); thumbnails del asset browser.

### Render path vigente: mesh GPU-driven Nanite-style (NO SDF)

El epic SDF #370 (GPU-driven SDF rendering Lumen-class) quedó **pausado**. El pivot a
**mesh GPU-driven Nanite-style** es el hot path. Phase 1 (#117) cerró completa: meshlet
pipeline end-to-end con cull + visibility buffer + deferred + Hi-Z 2-pass, atomic R64
visibility, cluster LOD DAG (METIS).

### Histórico — #440 two-pass material shading (PR #545, MERGED 2026-07)

Two-pass material shading estilo Bevy/Nanite en el path R64, **todo fragment** (se eliminó
el compute deferred):

- **Schema `Material`**: `albedo`/`normal`/`metal_roughness: Option<Guid>` + builders + RON round-trip.
- **`MaterialParams` 48 B**: `texture_indices: [u32;4]` (`NO_TEXTURE = u32::MAX`). WGSL structs stride-sync.
- **`MaterialTexturePool`** (`material/texture_pool.rs`): registry GUID→`GpuTexture` + bind group
  per-material con fallbacks 1×1 branch-free (white albedo / flat normal `[128,128,255]` / white metal_rough → sample no-op).
- **`MaterialPipeline`**: resuelve + sube imágenes por GUID, trackea triple `[albedo,normal,mr]` por slot.
  `shading_slots()` = `0..next_slot` (slot 0 fallback TAMBIÉN shadea).
- **Shaders** (`shaders/`): `resolve_material_depth.wgsl` (pass 1 → Depth16Unorm),
  `visibility_buffer_resolve.wgsl` (port The-Forge/Bevy: baricéntricas perspective-correct +
  ddx/ddy analíticas), `material_pbr_default.wgsl` (normal-debug × albedo + normal mapping
  tangent-space, `textureSampleGrad`). WGSL sin `#include` → se concatenan en Rust (`compose_material_shader`).
- **Render path** (`vbuf64_stage/`): `MaterialTwoPass` (resolve + N passes per-material,
  depth-test `Equal`, vs emite `slot/65535` como depth, `@invariant`). `DebugResolve`
  (`debug_resolve.rs`): fragment fullscreen para debug modes colorize (1,2,3,4,7). Modos
  normal-look (0,5,6,8,9,10) → two-pass.

### ⭐ Próximo paso — #605: evaluar adoptar `bevy_ecs`

**Decidido por el user el 2026-07-25. Es lo primero de la próxima sesión, antes de #137.**

El motivo declarado por el que `ome_ecs` era propio eran los storages GPU, que `bevy_ecs`
no tiene. **#603 los borró — nunca los usó nadie.** Así que `ome_ecs` es ahora un ECS
archetype CPU-only: la misma categoría que `bevy_ecs`, con mucha menos madurez detrás. La
justificación hay que re-derivarla, no repetirla.

Medido: `ome_ecs` son **9.467 líneas y 186 tests**, con **21 manifests dependiendo** y **43
símbolos** usados desde afuera del crate.

El dato que reencuadra la pregunta: **`ome_ecs` no es sólo un ECS.** Es ECS core +
reflexión (`bevy_reflect`) + formato de escena (`bevy_scene`) + componentes de engine
(`bevy_transform`, componentes de `bevy_render`) + soporte del editor remoto
(`dynamic_components`, `ephemeral`). Así que "adoptar `bevy_ecs`" es en realidad adoptar
media fundación de Bevy — y ahí la pregunta honesta pasa a ser *"¿por qué no Bevy?"*, que
este proyecto ya contestó: el renderer es meshlet GPU-driven propio.

Lo que Bevy tiene y nosotros **no** (cero ocurrencias, verificado): sparse-set storage,
change detection (`Changed<T>`/`Added<T>`), `SystemParam`, `par_iter`, y grafo de schedule
(`SystemSet`, `run_if`). Ese último grupo pesa: **varios bugs de esta sesión fueron
problemas de orden** —el orden del frame en #570, la carrera mirror-vs-drag en #572— y un
schedule de verdad los expresa declarativamente en vez de por orden de inserción.

Salida plausible: quedarse con `ome_ecs` y portar mecanismos sueltos (schedule graph,
change detection) a medida que duelan. Esta sesión demostró que robarle diseños a Bevy de
a uno funciona: el fix de #602 (padre por índice, no por nombre) salió de leer cómo Bevy
remapea referencias a entidades con `EntityMapper`.

## Decisiones arquitecturales sticky (NO reabrir sin OK explícito)

### Física: cómo se conecta al ECS (locked 2026-07-25, #570)

- **`PhysicsBody(u32)` va SIN reflejar.** Es lo que hace funcionar el Stop: `WorldSnapshot`
  lo borra, y el sync siguiente reconstruye el mundo físico desde el ECS restaurado. Una
  sola fuente de verdad, en vez de serializar rapier con serde.
- **El mapeo NO es `HashMap<Entity, BodyHandle>`.** Slot `u32` POD + arrays paralelos.
- **`PhysicsComponentsPlugin`** = tipos reflejados sin solver, para el editor. El editor
  autora, el proyecto simula. Meterle el `PhysicsPlugin` completo al editor levantaría un
  segundo mundo Rapier de entidades espejo.
- **Escala del collider**: caja exacta por eje; esfera toma el eje mayor; cápsula radio por
  horizontales y `half_height` por vertical. Envolver > intersectar.
- **`Collider.center` NO se pre-rota** — rapier compone la pose del cuerpo encima de
  `position_wrt_parent`; rotarlo aplica la rotación dos veces.
- **Spot `inner_angle`/`outer_angle` = SEMI-ángulos.** Nadie los consume todavía; el gizmo
  fijó la convención (#594). Si el shader los lee como ángulo completo, cambiar
  `cone_radius`.

### Identidad de entidades en la persistencia (locked 2026-07-25, #602)

**Una escena NUNCA referencia una entidad por nombre.** Los nombres no son únicos —
`TEST3` tiene cinco entidades llamadas "Mesh" — y un `HashMap<String, Entity>` las colapsa:
gana la última y la jerarquía se reconstruye mal, en silencio, en cada carga.

Tampoco por `Entity`: `sync_scene_to_ecs` hace `despawn_all` + respawn, así que los handles
vuelven con otro índice y otra generación. `WorldSnapshot` existe precisamente porque el
formato de escena no puede preservar identidades. Y dos escenas distintas arrancan las dos
en índice 0 generación 0, así que guardar handles haría colisiones invisibles.

**Se referencia por `PersistentId`** — el camino genérico que la deuda de esta sección
pedía (#607). Un asset ya se direccionaba por `Guid` en vez de por handle; una entidad
hace lo mismo un nivel más abajo.

- **`EntityGuid(NonZeroU64)`** — identidad estable. `NonZeroU64` para que
  `Option<EntityGuid>` siga pesando 8 bytes.
- **`PersistentId` es opt-in.** Sólo lo llevan las entidades que algo referencia. Quién es
  referenciado no se sabe hasta escribir las referencias, así que **guardar asigna
  identidad** — por eso `SceneDocument::from_ecs` toma `&mut Resources`. La alternativa era
  un id por entidad en una galaxia entera.
- **`EntityRef` tiene dos estados explícitos**: `Live(Entity)` en memoria,
  `Persistent { scene, id }` en disco. `Entity` sigue sin implementar `Serialize`, y
  serializar un `Live` es **error** — si el save path no resolvió, la escena falla al
  guardar en vez de cargar después apuntando a entidades arbitrarias.
- **Los ids son locales a la escena, remapeados por instancia.** Es lo que hacen Unity
  (`SceneLoadFlags.NewInstance` + `PostLoadCommandBuffer`) y Unreal (Level Instances), y es
  lo que permite instanciar la misma escena dos veces sin que las dos copias reclamen la
  misma identidad. Una referencia interna **no escribe su `scene`**: una escena que
  nombrara su propio `Guid` en cada link no se podría copiar sin reescribirlos todos.
- **Contador, no random.** Re-guardar produce un diff limpio, y el watermark impide
  reemitir ids que otra escena ya referencia.
- **El pase de remapeo es genérico por `FieldKind::EntityRef`**, no por componente. Un
  componente que el engine nunca vio resuelve igual que `Parent`.
- **Una referencia sin target se guarda y carga como vacía, no falla.** Bajo celdas de
  mundo (#566) ése es el estado normal de una referencia a una celda no residente; fallar
  ahí haría que un borde de streaming parezca una escena corrupta.

**`Parent` dejó de ser especial** — es un componente común con un campo `Entity`.
`parent_index` y `parent` ya no se escriben; las escenas que los tengan siguen cargando.
El sort de `from_ecs` dejó de ser load-bearing (ordenaba la lista a la que `parent_index`
apuntaba); ahora sólo mantiene los archivos diffeables.

**Desbloqueado:** #560 (joints, dos referencias por joint) y las referencias cross-scene,
que `parent_index` no podía expresar ni en principio.

### Multi-escena: el world es el contenedor (locked 2026-07-25, #609)

`SceneManager` era `current: Option<PathBuf>` y cargar reemplazaba el mundo — "el mundo
entero en una sección", en los términos de #566. Ahora es un **registro de escenas
abiertas** con una activa.

- **La escena tiene `Guid`**, que es lo que `EntityRef::Persistent { scene, .. }` venía
  direccionando desde #607. **NO un path**: renombrar el archivo rompería todas las
  referencias que entran.
- **`SceneMember { scene: Guid }`** = hogar de autoría. **Derivado en el load, NUNCA
  serializado** — toda entidad de un archivo pertenece a esa escena, escribirlo guardaría
  el mismo hecho dos veces. Mismo criterio que `Children`/`GlobalTransform`.
- **SIEMPRE hay una escena**, aunque sea "Untitled" sin path (como cualquier editor).
  Arrancar vacío significaría que las entidades creadas antes del primer guardado no
  pertenecen a nada → se guardaría un archivo vacío y marcar dirty no tendría dónde.
- **Las entidades sin `SceneMember` las adopta la escena activa al guardar.** Es cómo lo
  spawneado en el editor consigue hogar sin que el autor piense en eso. Las ephemeral NO
  se adoptan (la cámara del editor no es contenido).
- **`dirty` es POR ESCENA.** Con dos abiertas, guardar una no puede declarar segura la otra.
- **La tabla de remapeo se keyea por `(escena, id)`, NUNCA sólo por id.** Los ids son
  locales a la escena, así que dos escenas numerando ambas una entidad 1 es lo normal;
  keyear por id le daría a cada referencia la escena que cargó última — el mismo fallo que
  hacía inservible resolver padres por nombre. **Hay test.**
- **Abrir el mismo archivo dos veces se RECHAZA.** Dos copias compartirían todos los ids de
  entidad, así que una referencia no podría decir a cuál apunta. Instanciar una escena N
  veces necesita remapeo por instancia — trabajo aparte.
- Un archivo sin `id` recibe uno al cargar y **se marca dirty**, para que persista. Si no,
  tendría id distinto cada sesión y ninguna referencia entrante resolvería nunca.

**Transform de escena: NO es criterio de streaming.** Unreal particiona el espacio en grid
(World Partition) y aparte tiene Level Instances con transform — que en modo *Embedded*
(default) **existen sólo en el editor** y en runtime se disuelven en la grid. Unity:
secciones + `SceneLoadFlags.NewInstance` con `PostLoadOffset`. O sea el transform sirve para
**composición** (instanciar un módulo N veces), no para decidir residencia. Una ciudad de
40km no tiene "una posición": si el criterio fuera origen+radio, cargás la ciudad entera o
nada. La residencia se deriva de dónde quedaron las entidades, como fijó #566.

### Cero readback: la excepción por eventos discretos (locked 2026-07-26, #614)

La regla sigue siendo **cero readback POR FRAME** en el hot loop. La excepción acordada:
**un evento discreto puede sincronizar.**

Discreto = el jugador excava, cae un meteoro, explota algo. Ocurre cuando ocurre, no todos
los frames. Es exactamente lo que hacen los engines de voxel destructible: el costo se paga
en el evento, no en el presupuesto de frame.

**Lo que la excepción NO habilita:** que el streaming del terreno, su selección de LOD, o
el refit de su estructura de aceleración metan readback por frame. Eso sigue prohibido. La
excepción se escribió así a propósito — "el terreno queda exento" sería más fácil de
recordar y sería la puerta por la que dentro de un año entra un hitch que nadie sabe
explicar.

Test mental antes de agregar un readback: **¿esto corre porque pasó algo, o corre siempre?**
Si es lo segundo, no entra en la excepción.

### Rapier define el techo de la física (locked 2026-07-26, #612)

**Implementar SOLO lo que Rapier ofrece. Lo que no permite → WARNING, no construirlo por
afuera.** Mismo razonamiento que "Bevy define el techo de wgpu", pero para física.

Motivo: física escrita por afuera del solver pelea contra el solver — dos autoridades sobre
la misma pose. El caso que originó la regla (un `RigidBody` dinámico emparentado a otro
transform) es exactamente eso, y **Godot lleva años sin resolverlo** (issues #22904 y
#120067, abiertas). Unity y Unreal lo *evitan* en vez de soportarlo: compound colliders (un
cuerpo, varios shapes) y welding.

Un warning honesto vale más que una feature que miente. Godot warnea en el nodo cuando la
configuración física no cierra; copiar ese patrón en el editor.

**Shear resuelto por la misma regla (2026-07-26, #612):** un padre con escala no uniforme
compuesto con un hijo rotado produce *shear*, y las formas de Rapier se construyen desde
dimensiones — no hay forma sheareada. Entonces: **aproximación documentada + warning en el
Inspector**, no una geometría propia que lo represente. El collider queda como best-fit y el
autor se entera dónde está mirando. `GlobalTransform::has_shear` ya lo detectaba para #214;
ahora también lo mira el Inspector para colliders.

**Excepciones acordadas** — lo específico del proyecto que Rapier no cubre:
- **Planetas terraformables** (terreno editable en runtime).
- **Double contouring / marching cubes** para la malla de colisión planetaria.

Las dos son sobre **generar la geometría** que después se le entrega a Rapier como
collider, NO sobre reemplazar el solver. Esa es la línea: **geometría propia sí, dinámica
propia no.**

No contradice "la física detrás de la abstracción": el trait sigue siendo el contrato, pero
su superficie la define lo que Rapier puede hacer, no lo que nos gustaría.

### Joints: cómo se conectan al ECS (locked 2026-07-26, #560)

**El joint nombra a los DOS cuerpos; no vive sobre uno de ellos.** La alternativa
(`connectedBody` de Unity) le cuesta a un cuerpo poder estar en dos joints a la vez, porque
un componente aparece una vez por entidad — y una pelvis de ragdoll está en cuatro. Con dos
`Entity` el joint va en la entidad que el autor quiera, incluso una vacía.

**Los joints NO se direccionan con un componente de slot como los cuerpos.** `PhysicsBody`
existe porque las dos direcciones del mapeo se recorren cada frame: sync pregunta "esta
entidad tiene cuerpo", writeback pregunta "de quién es este cuerpo". **Un joint no tiene
writeback** — nada lo lee de vuelta al ECS. Queda sólo entidad → joint, una vez por frame,
sobre un conjunto mucho más chico. Un map es la forma honesta de eso.

**El joint sabe que tiene que reconstruirse porque recuerda sus dos `BodyHandle`.** No hay
un segundo ciclo de vida que mantener sincronizado: un handle de cuerpo cambia cuando el
cuerpo se reconstruye — edición en el Inspector, cambio de escala, y sobre todo **stop**,
que tira todos los `PhysicsBody` y rearma el mundo desde el ECS restaurado. Así que "se
movieron los handles de mis cuerpos" ya significa todo lo que "terminó la sesión de play"
tendría que significar.

**Un joint roto NO vuelve solo.** El componente sigue autorado, así que olvidar el slot haría
que el próximo sync lo reconstruya y se rompa otra vez, para siempre. El slot queda con
`joint: None` hasta que los handles de los cuerpos se muevan.

### Prefabs = escenas instanciadas (decidido 2026-07-26, #611)

**Abrir e instanciar son operaciones DISTINTAS y sólo una tiene restricción.** #609 rechaza
abrir el mismo archivo dos veces *para editar* (dos copias editables no pueden guardarse
las dos al mismo archivo). **Instanciar nunca tuvo esa restricción** — los ids se hicieron
locales a la escena en #607 justamente para poder remapearlos por instancia.

| | abrir (editar) | instanciar |
|---|---|---|
| ids del archivo | 1:1 con entidades | **remapeados** por instancia |
| se guarda de vuelta | sí | no — la instancia vive en la escena contenedora |
| dos copias a la vez | ambiguo al guardar | válido |

**Una escena ES el prefab. NO inventar formato nuevo.** Dato: el prefab de Unity *es* un
archivo de escena serializado — mismo YAML, la diferencia es semántica. Godot lo hace
explícito con `PackedScene`. Y los dos guardan la instancia como **referencia al origen +
lista de diferencias, NUNCA copia del contenido** (Godot: `instance=ExtResource(...)` +
overrides; Unity: `PrefabInstance` con `m_SourcePrefab` + bloque `m_Modification`). Copiar
el contenido rompería el vínculo que hace que editar el origen actualice las instancias.

**Orden decidido por el user: A ahora, B después.**

- **Fase A (#611)** — instanciación en runtime: `instantiate(scene) -> Entity` con remapeo
  de ids, asset de escena cacheado. Es lo que un juego necesita (spawnear una bala, un
  árbol) y es la fundación de B, así que nada se tira.
- **Fase B** — prefabs completos: `SceneInstance { source, overrides }`, propagación del
  origen a las instancias, subárbol colapsado en el editor, anidados.

**Dos cosas a resolver en la Fase A porque tocan tipos YA MERGEADOS:**

1. **Raíz única.** Instanciar algo como unidad con transform exige una raíz; Godot lo
   obliga. Nuestro `.ome_scene` es lista plana con parents opcionales → N raíces sueltas no
   tienen "un" transform. O se exige raíz única, o se envuelve la instancia al instanciar.
2. **Identidad de la instancia.** `EntityRef::Persistent { scene, id }` dice "la entidad 1
   de la escena X". Con X instanciada tres veces eso es **ambiguo**. La respuesta de Unity:
   las referencias externas apuntan a la *instancia*, que por eso tiene su propio id.

**La decisión que la Fase B espera** (NO adivinarla ahora): si los overrides se guardan por
campo (Unity y Godot) o si una instancia modificada se vuelve su propia escena. Define el
formato de archivo, y se toma con la instanciación ya andando.

### El ECS ya NO tiene storages GPU (2026-07-25, #603)

Existían, nunca los usó nadie, y dos sistemas corrían cada frame sin consumidor. Los datos
llegan al GPU por los buffers de instancia del pipeline de meshlets, armados desde una
query CPU. **Un solo camino, no dos.** Esto invalidó la justificación histórica de por qué
`ome_ecs` es propio — la re-derivación está abajo.

### `ome_ecs` se queda y se mejora (locked 2026-07-25, #605 cerrado)

`bevy_ecs` **no se adopta**. Evaluado con mediciones, no con opiniones; no apareció ningún
blocker técnico, así que la decisión es sobre qué compra la migración frente a qué cuesta.

Lo que la evaluación descartó como riesgo: `bevy_ecs` es standalone de verdad (65 crates
con `reflect+serialize+multi_threaded`, cero `bevy_app`/`bevy_render`); el pipeline
GPU-driven ni se entera (todo `ome_render` toca el ECS por `Query` en **4 lugares**); y
`bevy_reflect` expresa `choices`/`shown_when` con custom attributes.

Lo que lo decidió:

- **Los 42 sitios que acceden al storage directo (`get_cpu`) contra 3 usos de `Query`** son
  un problema en *todos* los caminos. No son motivo para migrar: son trabajo que hay que
  hacer igual, y `ome_ecs::Query` ya expresa tuplas, `Option<Q>`, `With`/`Without` y
  `&mut T`. **Encapsular el ECS detrás de `Query` es el prerequisito de cualquier cambio
  de backend** — hoy la superficie de contacto son 80 archivos.
- **`EntityAllocator::revive`** — identidad literal a través del Play/Stop — es algo que
  `bevy_ecs` rechaza **por diseño** (`spawn_at` no acepta una generación consumida; eso
  *es* su garantía de seguridad de handles), y hay **177 sitios fuera de `ome_ecs`** que
  guardan un `Entity` en un campo. Para un engine con editor, nuestro diseño resuelve mejor
  un problema que Bevy no tiene.
- **51 de los 80 archivos afectados son `ome_editor_core`**, que es justo donde Bevy no
  ofrece diseño para copiar: no tiene editor (`bevy_editor_prototypes` archivado, el
  trabajo se movió al repo principal alrededor de BSN).

**Bevy sigue siendo la referencia a la que robarle diseños de a uno** — la práctica que
produjo el port de SPD, `aabb_in_frustum` y el remapeo de entidades. No se adopta como
dependencia.

Mejoras ordenadas por dolor demostrado, no por paridad de features: identidad de entidades
(#607, hecho) → grafo de scheduling en `ome_core` (los bugs de orden #570/#572 viven en
`app.rs`, no en el ECS) → convertir los 42 storage-sites a `Query` → change detection
(recién tiene sentido después de eso: `Changed<T>` sólo existe dentro de una query).
Sparse sets y `par_iter` cuando duelan; medido, hoy no duelen.

### Física = Rapier (revalidado 2026-07-21 con investigación, NO reabrir)

Jolt gana en performance CPU (~2× en escenas grandes) y en completitud (softbody,
vehículos, ragdolls). Pierde igual por tres razones: en Rust son bindings de terceros que
se autodescriben "early work in progress / watch for exposed nails", exige toolchain C++
(choca con #558, build shippeable), y **no tiene ni plan de GPU**. Rapier tiene wgrapier
(física en WGSL, broadphase BVH + solvers andando, demo de 93k cuerpos / 120k joints) y
prioridad Dimforge 2026 = GPU vía rust-gpu compartiendo tipos con rapier. Para un engine
GPU-driven sobre wgpu eso decide solo. Bonus: rapier 0.32+ trae **collider de vóxel
disperso**, único entre engines rigid-body generales — clave con el pipeline voxel/DC.

**El backend va SIEMPRE detrás del trait `PhysicsBackend`.** `rapier3d::` solo puede
importarse dentro de `ome_physics::rapier_backend`; en cualquier otro archivo es un bug de
arquitectura. Los componentes ECS describen intención (forma, masa, tipo de cuerpo), no
handles. Motivo: la migración a wgrapier tiene que ser reemplazar un crate, no reescribir
el engine.

### El mundo contiene escenas, NO al revés (#566)

Verificado contra engines ECS reales: Bevy escribe `Scene`/`DynamicScene` DENTRO de un
`World`; el `World` de Unity DOTS contiene la meta-entidad de escena y sus section
entities; el `UWorld` de Unreal contiene levels → UE5 World Partition = grilla espacial.

**Escena y celda son ORTOGONALES, no anidadas.** Una escena abarca muchas celdas; varias
escenas se solapan en una celda. Celda = "¿qué hay cerca?" (espacial, DERIVADO del
transform, nunca campo guardado). Escena = "¿qué autoró junto un diseñador?" (lógico).
Storage por `(escena, celda)` = archivos per-scene-section de Unity. Anidarlos costaría
escenas solapadas en el mismo volumen, edición paralela sin conflicto en git, y la
distinción entre descargar una escena y descargar una celda.

`SceneDocument` NO se reemplaza: pasa a ser el payload por `(escena, celda)`. El
`.ome_scene` actual es "el mundo entero en una sección" — la sección 0 de Unity antes de
partir nada. **Sección siempre residente** (carga primera, descarga última) = ahí vive la
entidad de focus, que no puede ser dueña de una celda que se descarga sola.

### SDF: murió la técnica, vive el dato (#565)

`ome_sdf` y `ome_bvh` ELIMINADOS, ~14k líneas. Criterio del usuario: *se queda lo que sirve
al sistema galáctico y a planetas terraformables; lo demás muere y se rehace bien contra el
motor de física actual.* Nada de "tal vez lo usemos" — si hace falta, cherry-pick del
historial.

Fuera: brushes CSG en WGSL, los 7 componentes `Sdf*` de ome_ecs, `ome_bvh` entero (flags
`IS_RAYMARCH`/`ROLE_RAYMARCH_*`; su único consumidor era contenido que nadie rendeaba desde
el pivote), `ChunkContent`, `ProceduralCitySource` (el editor la registraba y generaba
primitivas para un pool que nadie leía — "nunca funcionó" = desconectada, no rota),
`VolumePrimitive`. Adentro: `ome_world::voxel` (sparse storage con LOD, mudado desde
ome_sdf) y todo el streaming por distancia. `Aabb` se mudó a `ome_core`.

**No confundir los dos SDF**: murió SDF-como-técnica-de-render/authoring. Vive
SDF-como-formato-de-dato (un vóxel guarda una distancia con signo) — de ahí extrae malla
dual contouring, y el collider de vóxel de rapier lo consume directo. Para física no hace
falta lo primero.


- **Meshlet shading = two-pass all-fragment.** NO binding_array bindless, NO compute deferred.
  Material variants futuras = shaders dedicados vía `compose_material_shader`.
- **Meshlet grouping (LOD chain DAG) = graph-based METIS k-way**, NUNCA spatial (Morton/Voronoi/
  k-means/hilbert/octree). Probado empíricamente en #470: spatial schemes ignoran topología →
  coverage holes en LODs altos. Edge weights = shared-vertex count (crítico). Crate `metis`
  (LIHPC) `default-features=false, features=["vendored"]` compila estático (Bazzite atomic OK).
  **Cell-boundary vertex-lock sigue necesario** incluso con METIS (minimiza pero no elimina
  shared edges; el simplify destruiría border vertices sin lock explícito).
- **Capability detection runtime + cargo features SOLO por build target, NUNCA por vendor.**
  Un binario corre en cualquier vendor; paths se eligen al startup según `wgpu::Features`.
  Baseline: RDNA 2 / Turing / Adreno X1 (2020+).
- **Bevy define el techo de wgpu.** Ante "¿podemos hacer X de UE5/Nanite?": primero ¿lo hace
  Bevy? Si no, asumir limitación wgpu hasta probar lo contrario. Reference canon:
  `reference_bevy_meshlet_shader.md` (asistente).
- **GPU-driven ≠ DOD-shaped servido a fragment naïve.** El espíritu es: hot loop entera en
  compute, persistent buffers para visibility+work, indirect dispatch, cero readback.
- **prev_lod_indices Vec<u32>** (#535 H3), **parent.lod_error monotone-clamped** (#535 H1) —
  tests invariante en lod_chain.

### Editor remoto (locked 2026-07-21, épico C)

- **El proyecto es dueño del ECS; el editor standalone es cliente.** No cargar tipos del
  proyecto en el editor (no hay ABI Rust estable). Protocolo HTTP tipo BRP (`ome_remote`).
- **Transporte = `tiny_http` SÍNCRONO en thread lateral + puente mpsc al main.** NUNCA async/
  tokio en el core del engine. El server no toca el ECS fuera del main thread (`Stage::First`).
- **UNA fuente de datos + UN seam de edición.** ECS local (real o espejo `RemoteMirror`) leído
  por paneles/viewport igual en ambos modos; la ÚNICA bifurcación es `apply_actions` →
  `remote_edit::dispatch` si `RemoteState::is_connected()`. NO duplicar paths por modo.
- **Identidad cross-proceso = nombre cualificado, NUNCA TypeId.** `ComponentId` interned es
  process-local; el cable lleva nombres. `TypeId` solo para reflexión local.
- **Viewport modelo C** (hub rendea el espejo), entrada **A→B** (Open Remote separado hoy →
  Open Project remoto cuando madure). NO modelo B directo (sin fallback), NO embed de ventana
  (roto en Wayland).

### Lighting stack (locked 2026-05-06)

- **Diffuse GI = Surfel radiance cache + voxel/DC coupling (#450).** NO Radiance Cascades
  (#114 closed as not planned; reopen solo si Sannikov publica RC 3D shipping). SSGI cancelado.
- **Specular = SSR Hi-Z + parallax-corrected probes (#478) + RT futuro** detrás de feature flag.
  Probes specular-only, surfels diffuse-only, sin overlap.
- **Direct shadows: CSM (#476) → VSM (#477).** Shadow sampling abstracto detrás de UNA function
  call en deferred — swap por reemplazo de impl, no de call sites.
- **Sky: stellar_delivery port (#248) ahora → Hillaire 2020 upgrade path** (shader-only, no rompe
  `AtmosphereVolume` API).
- **Volumetric fog = Froxel grid 3D** (#32 reescrito), NO per-pixel raymarch.

---

## Gotchas activos / lecciones

### Herramientas: trampas que costaron tiempo real el 2026-07-25

- **`rustfmt <mod.rs>` formatea TODOS los módulos que ese archivo declara.** Formatear
  `mesh/mod.rs` arrastró `gltf_loader/*`; `ome_gizmos/src/lib.rs` arrastró `mesh/` y
  `renderer/`; `ome_ecs/src/lib.rs` arrastró 15 módulos. **Tres veces en una sesión.**
  Formatear sólo archivos hoja y verificar con `git status` antes de commitear.
- **`--delete-branch` en un PR que es base de otro CIERRA el de arriba** y GitHub no lo deja
  reabrir si la base ya no existe. Pasó con #582 → hubo que abrir #583. Retargetear a
  `development` ANTES de borrar la rama de abajo.
- **Crear una rama desde la rama anterior en vez de `development`** hace que el PR de arriba
  contenga al de abajo (#586 contenía a #584). Mergear el de abajo primero lo reduce solo.
- **Procesos `test3 --remote` huérfanos retienen el puerto 15703.** Si el editor muere sin
  cierre ordenado (un `kill`, un crash), el proyecto sobrevive y la sesión siguiente se
  conecta **en silencio a un binario viejo** — lo que hace intesteable cualquier cambio de
  protocolo. Síntoma: `unknown variant \`set_parent\``. Verificar el puerto antes de smokear.
  Un cierre normal sí apaga el proyecto bien.
- **`pkill -f <patrón>` se mata a sí mismo** si el patrón aparece en su propia línea de
  comando (exit 144). Usar `pgrep -x <nombre> | while read p; do kill $p; done`.
- **`grep -c` con 0 matches sale con exit 1** y corta cadenas `&&`. Separar la verificación
  del commit.
- **No correr suites de GPU con el editor abierto**: un `cargo test --workspace` murió con
  SIGSEGV en `ome_render --lib` compitiendo por la GPU con un editor vivo. No reprodujo.

- **rustfmt version drift (CRÍTICO):** rustfmt local (1.8.0) reformatea TODO el repo distinto a
  como está commiteado, y NO hay toolchain pin ni CI fmt-check. **NUNCA `git add -A crates/<x>/`
  después de `cargo fmt -p <x>`** → barre 100+ archivos de churn. Formatear solo archivos propios,
  `git add <archivos específicos>`. En #545 hubo que revertir 115 archivos de churn antes de pushear.
- **Repo SIN CI:** `.github/workflows/` vacío. "Verde" = MERGEABLE+CLEAN. Verificación 100% local:
  `cargo test`, pipeline-creation test en device real (`vbuf64_stage_pipeline_creation`), smoke del editor.
- **Color space loader:** `ImageLoader` registrado como `srgb()` global → normal/metal_rough se
  cargan en sRGB (incorrecto). Follow-up: hint de color-space por-asset en `.meta`. Albedo (sRGB) ya OK.
- **material_depth = Depth16Unorm:** `f32(id)/65535` round-trip exacto; per-material pass usa `Equal`.
  El vs del material shader DEBE emitir `screen.material_id/65535` como z (dynamic-offset UBO `screen`,
  un slot por material) + `@invariant`.
- **meshlet geometry BGL ahora incluye FRAGMENT** (gpu_meshlet.rs) para reusar `meshlet_bg` en el
  fragment material pass.
- **wgpu 29 gotchas:** `DepthStencilState.depth_write_enabled: Option<bool>`, `depth_compare:
  Option<CompareFunction>`. `Instance::new(InstanceDescriptor{...})` por VALOR (sin `&`). Color
  texture del render stage necesita `RENDER_ATTACHMENT`. `FLOAT32_FILTERABLE` required para linear
  sampler sobre R32Float. `R16Float` NO expone `STORAGE_BINDING` garantizado → usar R32Float.
- **Reversed-Z:** nuevos passes → clear `0.0`, comparator `Greater`/`GreaterEqual`.
  `ome_render::perspective_rh_reverse_z` helper canon. Frustum: usar `row2` para clip.z>=0,
  NUNCA `row3 + row2` (eso es OpenGL [-1,1]).
- **Mesa radv SIGSEGV** con test threads paralelos en crates que init wgpu → `--test-threads=1`.
  naga parse+validate a nivel lib corre seguro sin GPU.
- **Empty-scene draw call floor = 3** (sky + meshlet pass A + pass B indirect): no es leak, es costo
  fijo GPU-driven indirect.

---

## Backlog visual (post-#440) — orden sugerido

- ⭐ **#440** Texture references — pipeline MERGED (#545), issue OPEN para tooling de verificación.
- **[sugerido]** Material inspector + asset browser (prerequisito para verificar #440 y usable de verdad).
- **#441** PBR real (Cook-Torrance + sun + IBL) — `metal_roughness` ya bindeado/reservado; helpers
  baricéntricos + tangent ya existen.
- **#482** triplanar/world-space projection · **#483** foliage BTDF · **#484** HDR (AgX + auto-exposure
  + LUT) · **#485** Clustered Forward+ light culling.
- **#476** CSM → **#477** VSM · **#450** Surfel GI · **#478** reflection probes · **#480** denoiser
  · **#481** motion vectors + FSR 2.x.
- **#453** skinned mesh GPU · **#452** forward transparent · **#444** mesh shaders · **#443** bindless
  · **#392** render graph propio (cuando el stack tenga 4+ passes).
- **#536** vendor upscaling plugin (DLSS/FSR/XeSS) · **#537** cargo features por build target.
- Cerradas recientes: #543 (mesh-frame bench), #544 (timestamp HUD), #542 (flicker LOD).

## Editor — capacidades shipped

Viewport panel → `egui::Image` (offscreen ViewportTarget) · Inspector gimbal-safe (Euler-cached) +
Local/World rotation toggle · drag-drop Components · Hierarchy propagation · `GlobalTransform::lossy_scale()`
+ warning shear · Editor camera (orbit MMB / pan Shift+MMB / zoom rueda / fly RMB+WASD+QE / focus F) ·
Scene serialization `.ome_scene` (RON) con `EphemeralComponents` filter · SceneManager (path+dirty+load/save)
· default scene auto-create · gizmos (translate/rotate/scale + snap + Local/World) · undo/redo per drag ·
native asset picker · persistent dock layout · Perf HUD (FPS/CPU/GPU/RAM/VRAM/draws) · mdBook docs en `docs/book/`
· **Asset Browser** (árbol del crate + drag-drop import + menú contextual + inline create/rename + drag
asset→slot Inspector) · material editor en Inspector · IDE picker · **codegen de proyecto** (scaffolds
component/system + `registrations.rs` + editor embebido + gating) · **cliente remoto** (`RemoteSession` +
`RemoteMirror` + dual-sink, ver épico C — falta activación 4b.3b).

---

## Workflow rules (NEVER violate sin OK explícito del user)

- **Branch first** (`feat/<slug>` desde `development`, nunca directo). Después de crear PR: **STOP**
  salvo que el user pida mergear.
- SOLO `gh pr merge --merge` (**NUNCA squash** — rompe git graph). Conventional commits EN, **sin AI
  signatures / Co-Authored-By**.
- PRs a `development` NO auto-cierran issues (merge a non-default) → cerrar a mano si corresponde.
- **state-of-art production-ready desde commit 1, NO MVP** (`feedback_correct_implementation_day_one`).
- Cada subtask = 1 commit (`git add` específico, NO `-A` tras fmt).
- El user maneja el fin de sesión; el smoke visual lo maneja el user (el agente arranca la app y diagnostica).

## Docs de referencia in-repo

- **`docs/ROADMAP.md`** — qué sigue y por qué, ordenado por bloqueos. Este archivo manda en
  *decisiones*; el roadmap manda en *orden*.

- `docs/decisions/0001_mesh_format.md` — mesh format ADR (glTF + OBJ).
- `docs/research/stack_decisions_2026-05-02.md` — stack choices + rationale.
- `docs/research/implementation_checklist_2026-05-02.md` — phased roadmap con exit gates.
- `docs/research/editor-three-system-architecture.md`, `sdf-csg-composition.md`, `wgpu-capabilities.md`.
- `docs/book/` — mdBook.
