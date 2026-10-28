# Contención del índice SQLite entre MCP y CLI

## Implementación

Se añadió `src/adapters/index_store/coordinator.rs`: cada operación pública de publicación adquiere un bloqueo exclusivo del sistema sobre `.codegraph/index-write.lock` antes de leer el estado que usará para planificar o publicar. El archivo queda vacío y puede permanecer después de terminar; su existencia no representa propiedad. La propiedad reside en el handle bloqueado, que Windows libera también al terminar el proceso.

Las operaciones completas, rebuild, lotes incrementales, archivos individuales y las publicaciones derivadas de edición/movimiento comparten este protocolo. El fallback incremental a inventario completo recibe el guard existente por referencia, evitando una adquisición recursiva. El escritor SQLite se destruye antes de liberar el coordinador. La inicialización conserva su reclamación exclusiva del directorio nuevo y no sobrescribe índices existentes.

La espera termina al cancelar, agotar el deadline del llamador o alcanzar 30 segundos. El sondeo usa backoff de 10 a 50 ms; no garantiza orden FIFO. Si el coordinador agota su propio plazo devuelve `SYNC_INDEX_BUSY`; si vence el contexto conserva `TIMEOUT` o `CANCELLED`. CLI recibe progreso `waiting_index_writer` y los errores identifican la etapa `wait_writer_coordinator`. No se atribuye un PID desconocido.

Se mantienen WAL, los 5 segundos de espera de SQLite y las transacciones por archivo. Los clientes externos o versiones anteriores pueden ignorar el nuevo protocolo: SQLite conserva la protección final y un bloqueo externo persistente sigue dando un error acotado. MCP y CLI necesitan ejecutables que incluyan esta implementación.

El archivo de coordinación se valida como archivo regular vacío, sin reparse points ni enlaces duros; sus ancestros y su nombre se mantienen protegidos contra reemplazo durante la operación. La desinstalación reconoce el archivo y rechaza eliminar un índice mientras el coordinador está en uso.

### Verificación

Se añadieron pruebas en `tests/index_writer_coordination.rs`, `src/adapters/index_store/coordinator_tests.rs`, `src/adapters/index_watch/coordination_tests.rs` y `crates/skills/src/installation/tests.rs`. Cubren las entradas públicas, cancelación, plazos, recuperación entre procesos, CLI real, lecturas concurrentes, alias de mayúsculas, identidad del lock, fallback, eventos del watcher, una transacción mayor al timeout SQLite y desinstalación. El test ignorado `cooperative_owner_process` es un auxiliar que los otros tests ejecutan como proceso hijo.

La primera ejecución de regresión tuvo 12 fallos porque las publicaciones ignoraban el lock; el test del fallback detectó además una adquisición recursiva. Resultados ejecutados en Windows:

| Alcance | Resultado |
| --- | --- |
| Biblioteca principal, dentro de la ejecución general | 459 pasaron; 7 ignorados |
| 11 suites de integración de indexación | 108 pasaron; 3 ignorados |
| `cargo test -p harness-skills -p harness-application` | 134 pasaron |
| `external_scopes`, `sessions`, `symbol_graph_search`, `syntax_search` | 19 pasaron |
| Pruebas nuevas de esta corrección (incluidas en los grupos anteriores) | 28 pasaron |

Las suites de indexación se ejecutaron con `cargo test -p agent-harness --test index_diagnostics --test index_incremental --test index_init --test index_scope --test index_store --test index_sync --test index_sync_robustness --test index_writer_coordination --test legacy_windows_index --test framework_index_store --test native_reindex_audit`. La auditoría que requiere un índice real y autorización de reindexación permaneció ignorada, igual que la comparación opcional contra el proveedor y el auxiliar de proceso (este último sí fue invocado por sus tests padres).

La ejecución general `cargo test` **no quedó verde**: se detuvo en `tests/architecture.rs` por dos fallos ajenos a este cambio:

- `mcp_keeps_test_bodies_external_and_diagnostic_policy_out_of_native_adapters`: tests inline en `crates/mcp/src/readable_batch.rs`.
- `portable_layers_cannot_gain_unreviewed_infrastructure_dependencies`: dependencia `aho-corasick` en `harness-domain`.

Los dos archivos señalados no presentan diferencias contra el índice Git. Se conservaron esas reglas de arquitectura. Antes de ejecutar las suites se corrigieron ocho inicializadores de fixtures que omitían el campo ya existente `SearchArgs.only_matching`, usando `None` para preservar su comportamiento; los fixtures afectados pasaron después.

Harness no pudo programar el trabajo en segundo plano por `TASK_STORE_BUSY`; `harness_doctor` confirmó la contención del almacén de tareas. La ejecución general se hizo con `harness_verify` directo; los comandos dirigidos usaron Cargo porque la herramienta de verificación no ofrece filtros de test. La observación automática de frescura global devolvió `SIZE`, así que no se toma como certificación global; los resultados anteriores proceden de las ejecuciones reales y la revisión del diff.

No se reemplazaron binarios instalados ni se ejecutó mantenimiento sobre el índice del usuario.

### Activación pendiente

Compilar la versión de distribución con `cargo build --release --locked` y actualizar el ejecutable mediante el procedimiento habitual de instalación. Reiniciar las conexiones MCP para que servidor y CLI usen la versión nueva; comprobar ruta, versión y `harness_bootstrap`. La existencia de `index-write.lock` después del reinicio es normal: no hay que borrarlo para liberar el bloqueo. La compilación release y la sustitución del binario instalado no forman parte de los checks ejecutados arriba.

## Investigación y propuesta original

## Problema y alcance

En Windows se ha observado `SYNC_INDEX: busy` al ejecutar mantenimiento del índice mientras MCP atiende el mismo workspace. El error comunicado no incluye la etapa ni el código extendido SQLite; por ello **la causa concreta aún no está demostrada**. Tener el servidor abierto no implica por sí mismo que mantenga una transacción de escritura. Tampoco se ha comprobado que el ejecutable instalado corresponda a este árbol de código.

Este documento propone cómo reproducir, diagnosticar y corregir la contención sin matar procesos ni eliminar archivos SQLite. No se ha ejecutado mantenimiento ni modificado el índice real durante esta investigación.

## Evidencia del código actual

- `src/cli.rs`: `--index` despacha `sync`, `rebuild` y operaciones por archivo al mismo almacén. `src/adapters/index_store.rs` aclara que rebuild vuelve a extraer archivos sin cambios, conserva la base existente y confirma por archivo; no es una transacción global.
- `src/adapters/index_init.rs`: un índice nuevo termina en WAL. `src/adapters/index_writer.rs`: el escritor exige un índice ya en WAL; no migra automáticamente otro modo. WAL permite lectores concurrentes, pero no varios escritores simultáneos.
- `src/adapters/mod.rs`, `index_writer.rs` e `index_sync.rs`: el timeout SQLite es de 5.000 ms en las conexiones principales. `index_lock.rs` ya espera con backoff ante conflictos Windows de apertura (códigos 32/33). Son mecanismos diferentes y no garantizan que una escritura termine dentro del plazo.
- `src/adapters/index_store/file.rs` inicia transacciones `IMMEDIATE`; `transaction.rs` confirma o revierte cada archivo y trata un fallo de commit como estado incierto. `tests/index_init.rs` verifica que un escritor bloqueante produce `SYNC_INDEX_BUSY` tras una espera acotada.
- `src/transport/runtime/single.rs` puede iniciar el watcher en Windows cuando está habilitado y existe el índice; `index_watch/worker.rs` ejecuta reconciliación o sincronización incremental. El watcher reencola fallos reintentables con backoff limitado (`index_watch/report.rs`). Esto corrige la afirmación previa de que toda sincronización automática era opt-in: depende de la configuración y del estado del índice.
- `index_store/sync_diagnostic.rs` captura etapa y códigos SQLite primario y extendido; `competingProcessId` es null. El indicador `competingWriterDetected` se basa en BUSY/LOCKED: no identifica ni demuestra cuál proceso es propietario.
- Hay entradas por sincronización de rutas modificadas, sincronización de archivo y movimiento, además de CLI y watcher. `changed_paths.rs` puede recurrir a una sincronización completa: cualquier coordinación debe evitar adquirir dos veces el mismo bloqueo.

Las búsquedas generales tuvieron archivos omitidos o errores de lectura; las búsquedas acotadas en `src` y los archivos citados sí se completaron. Hay que revisar todos los puntos de escritura antes de implementar el protocolo.

## Reproducción y diagnóstico previos

1. Confirmar versión y ruta de los ejecutables MCP y CLI, configuración del watcher y ruta canónica del workspace. Registrar el estado del watcher mediante los diagnósticos disponibles.
2. Reproducir en un workspace temporal representativo, con un índice inicializado y código de prueba. Lanzar MCP y CLI como procesos separados; sincronizar deliberadamente el inicio de dos operaciones de escritura mediante barreras, no sleeps como única garantía. Conservar stdout/stderr e informe completo, tiempos, etapa, códigos SQLite primario/extendido y estado de commit.
3. Distinguir bloqueo SQLite de conflicto de apertura Windows, esquema incompatible, plazo agotado y estado de commit incierto. Comprobar también el journal mode real. Si no se reproduce, instrumentar la etapa y duración de espera sin afirmar una causa.
4. Repetir con un escritor externo que no use Harness: permitirá comprobar qué parte resuelve la coordinación cooperativa y qué parte sigue correspondiendo a SQLite.

## Solución propuesta

1. **Coordinador de escritura por índice**, compartido por todas las entradas que mutan el grafo: sincronización completa, rebuild, lotes del watcher, archivo individual y efectos de edición/movimiento. Adquirir antes de planificar o leer el estado que determina una publicación, mantener durante la operación lógica y liberar con RAII. Dejar lecturas normales concurrentes en WAL. Inventariar inicialización y cualquier otro escritor antes de decidir cómo participan.
2. **Bloqueo entre procesos respaldado por el sistema operativo**, con identidad estable del índice/workspace y protección frente a alias de ruta, enlaces y reemplazo del archivo de bloqueo. Usar un mecanismo que se libere al cerrar el handle o terminar el proceso; no utilizar la mera existencia de un archivo o un PID como bloqueo. Evaluar explícitamente los límites de permisos y del sistema de archivos, y fallar de forma clara donde no se pueda garantizar la coordinación. SQLite sigue siendo la última defensa frente a clientes externos o versiones antiguas que no respeten el protocolo.
3. **Un solo propietario por operación lógica**, con funciones internas que reciben un guard ya adquirido. En particular, el fallback de `sync_changed_paths` a `sync_workspace_all` debe reutilizar el guard, no volver a adquirirlo y causar un deadlock. Definir orden de adquisición antes de abrir transacciones o tomar otros locks y comprobar que no bloquea el hilo de ejecución asíncrono indebidamente.
4. **Espera acotada y cancelable** para el coordinador, respetando el deadline de `OperationContext`, con intervalos de backoff y jitter; proponer un valor configurable con un límite superior razonable y documentar su valor por defecto tras medir. Mantener el `busy_timeout` SQLite como protección para procesos externos; no sumar inadvertidamente múltiples timeouts por archivo. No reintentar ciegamente `SQLITE_LOCKED`, errores permanentes ni `SYNC_COMMIT_UNCERTAIN`.
5. **Watcher cooperativo**: mientras espera mantenimiento, conservar eventos y reencolarlos; al terminar, reconciliar para cubrir cambios ocurridos durante el rebuild. Evitar inanición entre el watcher y CLI y respetar cancelación. Seguir informando cobertura parcial y archivos confirmados si una operación termina a medias.
6. **Diagnósticos claros**: indicar etapa (`coordinator`, apertura o transacción), tiempo esperado, operación y estado de commit; incluir PID/inicio/operación del dueño solo si metadatos asociados al guard se pueden validar, y señalar que son orientativos. Una operación externa puede ser dueña del lock SQLite sin ser identificable. No afirmar éxito o frescura global a partir de un rebuild parcial.

No enviar señales ni finalizar automáticamente el PID: puede ser un servidor en uso o un proceso no relacionado, y no soluciona correctamente commits inciertos. Si hay bloqueo externo persistente, devolver un error accionable para que el usuario identifique y cierre el cliente de forma controlada. No borrar la base ni sus archivos `-wal`/`-shm`.

## Pruebas de aceptación

- Dos procesos Harness para el mismo índice: el segundo espera; al terminar el primero, continúa y el índice queda íntegro. Probar CLI vs watcher y dos CLI, y confirmar que lecturas siguen funcionando en WAL.
- Bloqueo sostenido más allá del plazo y cancelación: salida acotada, diagnóstico preciso, sin afirmar rollback de archivos ya confirmados y sin perder eventos pendientes.
- Terminar deliberadamente el propietario en un workspace temporal: el guard del sistema se libera; el siguiente proceso avanza y SQLite recupera la base. Verificar integridad y reportar el estado parcial, sin eliminar sidecars.
- Cliente SQLite externo con `BEGIN IMMEDIATE`: comprobar timeout y diagnóstico reales; el coordinador cooperativo no debe atribuir falsamente un PID ni garantizar exclusividad frente a terceros.
- Fallback incremental a full, escrituras individuales, múltiples workspaces, alias de ruta y varias instancias MCP: sin deadlock, sin exclusión entre índices distintos y sin escrituras simultáneas de participantes para un mismo índice.
- Ediciones recibidas durante rebuild: quedan pendientes y se reconcilian; interrupción después de un commit conserva informe de progreso y una posterior sincronización segura.
- Índice no WAL, acceso denegado, corrupción, `BUSY_SNAPSHOT`/`LOCKED` si se reproducen y fallo de commit: mensajes diferenciados; no reintento inseguro ni reparación destructiva automática.

## Entrega y límites

Implementar primero la reproducción y pruebas que fallen, después el coordinador y los diagnósticos. Ejecutar las pruebas de integración multiproceso en Windows y la suite de indexación pertinente. Validar la versión realmente instalada por separado antes de desplegar. Esta propuesta reduce la contención entre participantes compatibles; no hace al índice «indestructible» ni reemplaza la validación de esquema, integridad y cobertura.
