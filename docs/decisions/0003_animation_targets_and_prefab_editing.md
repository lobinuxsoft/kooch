# ADR 0003 — Cómo una animación nombra lo que anima, y dónde se edita un prefab

**Status:** Accepted
**Date:** 2026-09-28
**Issue:** #717 (el grafo), #715 (animación de escena), #716 (identidad del target)
**Bloquea:** #1354 (abrir un prefab en el viewport)
**Bloqueado por:** #712 (una referencia dentro de una instancia de prefab se pierde al cargar)

---

## Contexto

El Timeline necesita nombrar lo que anima: una entidad, un componente, un campo.
Y necesita hacerlo **desde dentro de un prefab**, porque un prefab es un archivo
con animaciones propias que hay que poder editar sin meterlo en una escena.

Los handles no sirven: `Entity` es índice + generación, se reparte de nuevo en
cada instanciación, y #1300 registra que los índices además suben en cada
recarga de escena. Los ids persistentes tampoco alcanzan por sí solos: #712
midió que una referencia a una entidad dentro de una instancia de prefab se
resuelve **57 microsegundos antes** de que los prefabs se expandan, y se
descarta en silencio.

Lo que el motor ya tiene, y conviene decirlo porque es fácil creer lo
contrario: **referencias a entidades completas**. `EntityRef::{ Live(Entity),
Persistent { scene: Option<Guid>, id: EntityGuid } }`, `FieldKind::EntityRef`
en la reflexión, conversión live↔persistente al guardar y cargar, y
`scene/entity_refs.rs` asignando `PersistentId` a toda entidad que alguien
referencie. Que `VirtualCamera.group` sea un número **no** es una prohibición
general: es una elección local, tomada porque las referencias hacia prefabs
están rotas (#712).

## Cómo lo resuelven los demás

| Motor | Direccionamiento | Qué rompe |
|---|---|---|
| **Unity** | string de path relativo desde el root del Animator + tipo de componente + `propertyName` | renombrar un hijo rompe la curva, en silencio |
| **Godot** | `NodePath` relativo al `root_node` del `AnimationPlayer` | el path guardado en un `PackedScene` se lleva el nombre de la instancia, así que **sólo la primera instancia anima** (godot#26144) |
| **UE5** | `FGuid` por binding (`FMovieSceneBinding`): *possessable* por soft object path, *spawnable* propiedad de la secuencia | los bindings **no se resuelven en runtime**: mover la secuencia, recolocar el actor o animar algo spawneado rompe callado. UE 5.5 agregó *Dynamic Binding* para taparlo |

Los tres tienen el mismo bug de familia con distinta cara. No es un problema
resuelto en ninguna parte: es un problema donde hay que **elegir a conciencia
qué rompe**.

## Decisión

**Dos direccionamientos, elegidos por el alcance del track, no uno solo.**
Elegir uno es el error de diseño a evitar.

### 1. Dentro del root de la animación → path de nombres hasheado (#716)

Un track que apunta a algo bajo el root —un hueso, una entidad interna de un
prefab— se direcciona por el **hash de la lista de nombres entre el root y el
target**, como el `AnimationTargetId` de Bevy.

Es estable entre recargas, entre instanciaciones y entre órdenes de carga
**precisamente porque no deriva de nada asignado en runtime**. Y regala
retargeting: un clip que anima `Hips` corre en cualquier rig con un hueso
llamado `Hips`, sin código.

Rompe al renombrar, igual que Unity. Se acepta: es el único de los tres modos
de rotura que el autor **ve** mientras lo causa.

### 2. Cruzando ese límite → `EntityRef` (y por eso #712 es prerrequisito)

Un track que nombra *esa* puerta, en *esta* escena, fuera del subárbol del
root, usa `EntityRef::Persistent`, que es el mecanismo que ya existe. **No se
construye Timeline encima de #712 sin arreglarlo**: hoy ese track carga vacío
y no avisa.

### 3. El código del proyecto entra al grafo por punteros a función, no por trait objects

El `ScriptPlayable<T>` / `PlayableBehaviour` de Unity es la razón por la que su
API es flexible y no configurable, y la capacidad hay que copiarla. La **forma**
no: un nodo con métodos virtuales es `Box<dyn Node>`, que las reglas DOD
prohíben y que además mete una vtable en medio del paso topológico.

El rig ya lo resolvió: `CameraRig` es una lista registrada de
`RigFn = fn(&mut RigStep)`. Igual acá — un `NodeKind { output, prepare, process }`
registrado, con el `u32 type_tag` del nodo funcionando como índice a la tabla
de kinds. Detalle en #717.

### 4. Un prefab se edita como una escena, en el viewport (#1354)

Un `.prefab` ya deserializa a `SceneDocument`, el mismo tipo que un `.scene`.
Entonces el modelo es el de **Godot** —una escena *es* el prefab, se abre en su
propia pestaña— y no el de Unity, que necesita un Prefab Mode aparte
justamente porque sus prefabs no son escenas. Kóoch no tiene esa deuda: abrir
un prefab está más cerca de una pestaña que de un modo.

## Consecuencias

- Un clip es reusable entre instancias **por defecto**, que es la propiedad que
  godot#26144 pierde.
- El autor puede romper un clip renombrando una entidad. Hay que avisarlo al
  cargar, como avisan los warnings de autoría de cámara (#1342/#1352): un track
  cuyo id no resuelve tiene que decirlo, no quedarse callado.
- Hay dos formas de nombrar un target y el editor tiene que dejar claro cuál
  está usando cada track, o el autor no va a entender por qué uno sobrevive a
  copiar el prefab y el otro no.
- #712 pasa a `priority:high`: dejó de ser un bug de cámara y es un
  prerrequisito del Timeline.

## Alternativas descartadas

- **Sólo `EntityGuid`, para todo.** Ata el clip a una instancia y mata el
  retargeting. Es la rotura de UE5 y de godot#26144 a la vez.
- **Sólo paths de nombres.** No puede nombrar nada fuera del subárbol del root,
  que es justo lo que una cutscene necesita.
- **`usize` índice en la lista de entidades del prefab**, reusando el
  `OverrideAddress { entity: usize, component, field }` que `PrefabInstance` ya
  usa. Tentador por consistencia, y descartado: rompe al reordenar o insertar
  entidades en el prefab, que es una operación normal de autoría y **no deja
  rastro** de haber roto nada.
- **Prefab Mode al estilo Unity**, un modo de edición aparte. Es el precio que
  paga Unity por que sus prefabs no sean escenas; acá sería pagarlo sin deberlo.
