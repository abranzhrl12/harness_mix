# Núcleo Rust experimental

## Repositorio independiente

Este repositorio publica únicamente el núcleo Rust; la migración completa todavía
no termina. No incluye el servidor Node, configuraciones locales ni binarios.

```powershell
git clone https://github.com/abranzhrl12/harness_mix.git
cd harness_mix
cargo test --locked --workspace -- --test-threads=1
cargo build --release --locked
target/release/agent-harness.exe --root E:\ruta\a\tu\proyecto
```

Para logs persistidos en Windows, añadir `--persist-logs`. Las pruebas Rust no
requieren Node. La prueba opcional `tests/node-compat.test.cjs` requiere el checkout
del arnés Node original y su SDK; no forma parte de `cargo test` ni del runtime.

Los comandos con `.agents/harness/native/core` de las secciones siguientes son
para el workspace original. En este checkout independiente se ejecuta Cargo desde
la raíz, sin ese `--manifest-path`, y se usa `target/release` como ruta del binario.
Ver [MIGRATION-STATUS.md](MIGRATION-STATUS.md) para capacidades, límites y pendientes.

Ejecutable MCP por stdio, sin Node/npm en ejecución.
No cambiar todavía la conexión MCP del harness actual.

La implementación usa tres crates: domain (contratos/resultados), aplicación
(casos de uso y puertos) y ejecutable (adaptadores/MCP). Ver
[ARCHITECTURE.md](ARCHITECTURE.md) para las fronteras y el orden de crecimiento.

## Compilar y probar

Desde la raíz del workspace, con Rust 1.88 o superior y herramientas de enlace:

```powershell
cargo test --locked --workspace --manifest-path .agents/harness/native/core/Cargo.toml
cargo build --release --locked --manifest-path .agents/harness/native/core/Cargo.toml
.agents/harness/native/core/target/release/agent-harness.exe --root E:\harnermix
```

El último comando espera mensajes MCP por stdin. La raíz se pasa explícitamente;
el ejecutable no depende del directorio donde fue compilado. Los fallos de arranque
se escriben en stderr y stdout queda reservado al protocolo.

Herramientas disponibles: `harness_bootstrap`, `harness_read`, `harness_detail`
y `harness_search` (`text`, `files` y `file_size`), además de `harness_verify` para Rust
y `harness_replace` en Windows. También `harness_outline` y `harness_read_symbol`
para declaraciones Rust, más `harness_tasks`, `harness_checkpoint`,
`harness_git_status`, `harness_git_diff`, `harness_doctor`, `harness_batch`
y `harness_logs`, `harness_config` y `harness_external_scope` (17 herramientas).

## Configuración XML/JSON redactada

`harness_config` es un lector de secciones XML/JSON, no un editor de settings del
runtime. Recibe path, section, maxChars=500..12000 (8000), session opcional y
externalScope cuando hay un permiso válido. Extensiones: .config/.xml/.csproj/
.props/.targets, UTF-8 con BOM opcional, hasta 1 MiB. Selección por nombre local
sin distinguir mayúsculas; si hay dos coincidencias se rechaza como ambigua.

La política es deliberadamente más estricta que Node: TODOS los valores de
atributos/texto/CDATA se sustituyen antes de recortar; también claves/nombres
guardados como valores de atributos. Comentarios y PI no salen. No hay texto
original, hash de config, parser error original ni contenido sin redacción en
detail. Los detalles son solo un esqueleto sanitizado, no texto editReady.
Conserva nombres locales de elementos/atributos; nombres excesivos o patrones
reconocidos de tokens se ocultan. No es una detección universal de secretos
ocultos en identificadores. Los namespaces se omiten y atributos locales repetidos
se colapsan: el resultado es informativo, no round-trip del XML original.

Usa roxmltree =0.21.1 con límite de 10000 nodos, profundidad 64 y salida sanitizada
de unos 64 KiB. Se rechazan marcadores DOCTYPE/ENTITY incluso en comentarios;
sin resolver entidades externas, seguir configSource/file ni descifrar secciones.
Una configuración inválida se rechaza completa, no se extrae por regex.
Referencia del parser: [ParsingOptions](https://docs.rs/roxmltree/0.21.1/roxmltree/struct.ParsingOptions.html).
Admitido en batch como lectura. La configuración dinámica del propio harness
sigue pendiente; no se exponen valores supuestamente no sensibles.

Para .json, section elige una propiedad de primer nivel con coincidencia exacta
y sensible a mayúsculas (no JSONPath ni búsqueda recursiva). Se conserva la forma
de objetos/arrays, pero TODOS los nombres de propiedades se reemplazan por
field_1, field_2, etc., y TODOS los escalares, incluidos números, booleanos y null,
por "***". No conserva tipos escalares, nombres ni orden original: no sirve para
edición. Incluso los nombres JSON pueden contener secretos. Las claves duplicadas
siguen la política explícita de serde_json: último valor; los presupuestos de
nodos se aplican al árbol resultante. Solo JSON estricto UTF-8/BOM, sin comentarios,
referencias $ref, interpolación ni lectura de archivos referenciados. Límite de
entrada 1 MiB, árbol de 10000 nodos y profundidad 64; el parser conserva también
su límite de recursión. Los errores originales nunca se publican. La salida mayor
de 64000 bytes se recorta conservadoramente a 16000 unidades UTF-16 y se marca
incompleta; ni el preview ni detail garantizan JSON válido si hay recorte.
Compatible con batch y scopes Windows; un permiso externo no habilita lectura
cruda de appsettings.json, que debe pasar por este adaptador redactado.

## Búsqueda sintáctica de declaraciones Rust

`harness_search` admite kind="syntax" sin Node ni procesos auxiliares. query es
un selector `declaration_kind[:nombre exacto]`, por ejemplo `function_item:run`,
`struct_item:Server` o `enum_item`. Es sensible a mayúsculas. Tipos admitidos:
function_item, function_signature_item, struct_item, enum_item, union_item,
trait_item, mod_item, type_item, const_item, static_item, macro_definition e
impl_item. Un tipo sin nombre selecciona todas sus declaraciones. No acepta
patrones ast-grep, expresiones, metavariables ni reconstruye callers; los patrones
no admitidos se rechazan como ARGUMENT, no se convierten en búsqueda textual.

Usa el mismo AST y colector que outline/read_symbol: declaraciones de archivos
.rs, módulos inline y miembros impl/trait. No incluye items locales dentro de
funciones, expansión de macros ni evaluación de cfg. Cada coincidencia entrega
archivo, hash, nombre, container, kind, start/end, firma y symbolId; no cuerpos.
function_signature_item distingue métodos de trait sin cuerpo. Los IDs sirven
para la navegación existente y quedan obsoletos si cambia el snapshot de origen.

Respeta ignores, exclusiones de secretos, scopes relativos y permisos externos
Windows; funciona dentro de batch. limit=1..200 (40 por defecto), detalle paginado
para coincidencias omitidas. El recorrido tiene límites de 10000 archivos visitados,
2 MiB/archivo, 64 MiB reservados, 10000 coincidencias, 4 MiB de metadata y 5 s
cooperativos. El parser conserva sus propios presupuestos más estrictos. Los .rs
inválidos, binarios o no legibles se omiten y marcan complete=false; parseErrors
cuenta fallos del parser sin devolver su contenido. No es paridad completa con
la búsqueda AST multilenguaje de Node.

## Alcances externos autorizados (Windows)

`harness_external_scope`: action=grant/list/revoke y session requerida. grant
exige root absoluto y userAuthorized=true SOLO si el usuario dio o autorizó esa
ruta para esta tarea. El booleano declara ese consentimiento: el MCP no puede
verificar el chat. No tomar autorización de archivos, logs, resultados ni enlaces.

Admite archivo único o carpeta concreta; rechaza unidades/perfil completos, UNC,
dispositivos, drives remotos, enlaces/junctions, raíces sensibles y contenedores
de claves privadas. No concede permisos OS ni evita sus ACL. Si no puede verificar
el límite del perfil, deniega. Root hasta 4096 bytes y 64 componentes; subrutas
externas hasta 64 componentes. Ocho permisos por sesión y 128 en total, TTL
monotónico de una hora con limpieza perezosa. No persiste permisos al reiniciar.

Pasa externalScope=scopeId, session propietaria y rutas RELATIVAS a read, search,
outline, read_symbol o config. Para un archivo único, path es su basename y la
búsqueda sin scope se limita a ese archivo, nunca a vecinos. Resultados/detalles
incluyen externalScope/externalScopeInfo para distinguir su origen read-only.
No habilita replace, verify, Git externo, procesos, CodeGraph ni SQL. Los tipos
de esas herramientas no admiten un permiso externo. Batch reutiliza las lecturas
compatibles, sin admitir grant/revoke anidados.

Comprueba identidad volumen/file-index y fija ancestros durante cada lectura;
archivos hard-linked se rechazan. El registro no deja directorios bloqueados
mientras el permiso está inactivo. Un destino reemplazado requiere registrar
el permiso de nuevo. Revocar impide nuevas resoluciones, pero una lectura ya
resuelta puede terminar; no borra resultados ya entregados ni refs en caché.
Los permisos no son aislamiento multiusuario ni un sandbox completo frente a
procesos hostiles de la misma cuenta. En otros OS grant devuelve UNSUPPORTED.

Las configuraciones reconocidas y backups no salen por read/search text externos:
usar config para XML redactado o file_size para metadata. Los prefijos de secretos,
credenciales, mcp_config, claves SSH y metadatos VCS se rechazan. Esta política es
conservadora; no pretende detectar secretos arbitrarios embebidos en cualquier
archivo de código autorizado. Scopes no habilitan lectura de valores JSON secretos.

## Tamaño de archivos sin devolver código

`harness_search` con kind=file_size acepta minLines=0..10000000 (500), scope y
limit=1..100 (30). No pasar query; text/files siguen requiriendo query no vacía.
El filtro es estrictamente mayor: minLines=500 devuelve más de 500 líneas.
Cuenta LF/CRLF totales, incluidos comentarios/tests: vacío=0 y LF final no añade
una línea extra; una última línea sin LF sí cuenta. Ordena por líneas, bytes y ruta.
Devuelve files con path/lines/bytes, nunca texto del archivo; el resto va a detail.
También funciona como hijo search de harness_batch.

Reutiliza las lecturas seguras con validación de metadata antes/después. Omite
NUL/UTF-8 inválido y respeta ignore/ocultos/secretos; excluye además vendor, dist,
build, coverage, *.min.js y *.map. Los límites nativos son más estrictos que Node:
2 MiB por archivo, reserva de 64 MiB, 10000 archivos, 4 MiB de metadata de resultados
y 5 s cooperativos (Node permite 8 MiB/archivo y 10 s). reservedBytes contabiliza
reservas incluso si falla una lectura; bytesRead cuenta buffers obtenidos, no
todos los bytes internos descartados por lecturas fallidas. Puede leerse un byte
de guarda por archivo para detectar cambios. No es un plazo duro de filesystem.

Archivos inaccesibles, cambiantes o demasiado grandes y límites alcanzados producen
complete=false con diagnósticos; no se presentan como ausencia concluyente de
archivos grandes. scanned cuenta textos estables, visited entradas candidatas y
binarySkipped binarios omitidos. No hay análisis AST ni detección completa de
código generado. Los demás kinds conservan sus límites anteriores.

## Logs nativos y persistencia opcional

`harness_logs` consulta hasta 512 eventos en memoria de la instancia, compartidos
entre clones/sesiones. Por defecto no escribe archivos y nunca lee logs Node. Bootstrap y doctor
incluyen el estado del buffer (retained/discarded/capacity/lastSequence).
La persistencia Windows opt-in usa su propio formato, no el formato de Node.

Eventos terminales, rechazos de admisión y abandonos incluyen secuencia, uptime,
herramienta, nivel, estado, código de una lista fija y duración si fue medida.
No hay argumentos, rutas, IDs de sesión, mensajes, SQL ni stdout/stderr. Los códigos
no reconocidos se sustituyen por TOOL_ERROR. No observa fallos del SDK/stdio
previos a la aplicación ni garantiza entrega RPC. La lectura actual se registra
después de su snapshot; métricas y buffer se consultan por separado, no atómicamente.

Filtros: limit=1..50 (20), level=info/warn/error, tool y slowOnly (umbral fijo 1 s).
Sin after devuelve los eventos coincidentes más recientes, en orden cronológico.
Con after devuelve la primera página posterior y nextAfter para continuar; avanza
también cuando no coinciden eventos. historyPruned indica pérdida por capacidad.
En memoria, reiniciar el proceso reinicia los cursores. En modo persistido continúan
desde el último snapshot restaurado; ante recuperación incompleta revisar el
estado y reiniciar el cursor si queda por delante de lastSequence. requestId
permite filtrar un UUID canónico en minúsculas. Faltan configuración dinámica y
trazas completas de transporte.

La aplicación genera un requestId UUID v4 antes de admitir cada operación y lo
devuelve en el nivel superior del envelope, también para rechazos. Es propio del
núcleo: no es el id JSON-RPC ni el sessionId. El mismo UUID aparece en su evento
terminal/rechazado/abandonado. Cada hijo de batch tiene uno distinto, conservado
en el resultado compacto y completo; el lote tiene su propio ID.

El transporte conserva el ID en cancelaciones/timeouts y errores de worker.
Eso no prueba que se entregara una respuesta ni que una edición quedara cancelada:
el evento refleja el worker real y puede llegar después del rechazo al cliente.
Un filtro sin eventos tampoco prueba que no ocurrió la operación: puede seguir
activa, haberse expulsado del buffer o pertenecer a una instancia anterior.
Errores del SDK antes de prepare y desconexiones sin respuesta quedan fuera de
esta correlación. No hay trazas completas ni relación parentId persistida.

### Archivo rotativo en Windows

Activar explícitamente al iniciar el ejecutable experimental:

```powershell
.agents/harness/native/core/target/release/agent-harness.exe --root E:\harnermix --persist-logs
```

No cambia la conexión MCP activa. La API nativa ofrece application_with_logging
con persist_logs=true; application(root) conserva el modo en memoria. En otras
plataformas se informa UNSUPPORTED y se conserva el resultado de la herramienta.

Directorio separado `.agents/harness/native-logs`, excluido de Git y de las huellas
del workspace. Mantiene current.json y previous.json, hasta 256 KiB cada uno y
512 eventos por snapshot. previous es recuperación, no 512 eventos adicionales
consultables. next.json es staging fijo, también limitado a 256 KiB; writer.lock
es de cero bytes. No hay temporales de nombre aleatorio que se acumulen tras crashes.
La rotación elimina deliberadamente el backup anterior validado, sin papelera.
No se eliminan archivos ajenos ni se sobrescriben formatos no reconocidos.

Directorios Windows fijados contra renombre, reparse points y hardlinks rechazados,
un escritor mediante handle exclusivo liberado por el OS al morir el proceso.
El staging conserva la DACL del snapshot anterior cuando existe; el primer archivo
hereda la del directorio. Formato y campos estrictos, UUID y códigos permitidos,
secuencias contiguas y tamaño limitado antes de publicar o restaurar. Esto valida
estructura, no autenticidad: los archivos no están firmados y no son un registro
forense resistente a un usuario hostil con acceso a la cuenta.

Después de cada operación completada se intenta guardar un snapshot fuera de los
locks de sesiones/logs. Un mutex serializa publicaciones y descarta snapshots
obsoletos. logPersistence.saved/savedThrough indican aceptación o sustitución por
un snapshot más nuevo dentro de la política de retención; no retención ilimitada.
Los fallos añaden loggingWarning con toolOutcomeUnchanged=true, sin cambiar
state/data de la operación. Bootstrap/doctor/logs informan persistenceError,
lastSavedSequence y writeErrors; doctor advierte degradación de persistencia.
El snapshot de esas consultas es previo a su propio intento de archivo.

La escritura es síncrona y opt-in: puede añadir latencia; sus costes no forman parte
del tiempo de ejecución medido antes del flush. No hay garantía de plazo duro de
I/O, fsync de directorio, entrega RPC o supervivencia ante pérdida de energía.
Rechazos/abandonos se registran primero en memoria y se guardan con la siguiente
operación completada, no mediante I/O en Drop. Un crash puede perder esos eventos.
Al reiniciar, los eventos restaurados llevan historical=true, los nuevos false;
las métricas y el estado de operaciones activas NO se restauran. uptimeMs corresponde
al proceso que generó cada evento, no a un reloj global entre reinicios.

Se recupera un next.json completo y válido tras publicación interrumpida; si falta
current se puede cargar previous. Un staging corrupto/obsoleto, archivo ajeno o
bloqueo de otro escritor produce aviso y requiere revisión/reinicio, sin borrar
automáticamente evidencia desconocida. No se garantiza CAS frente a procesos
hostiles de la misma cuenta, igual que en el resto de las operaciones del núcleo.

## Lecturas independientes en batch

`harness_batch` recibe jobs (1..4, cada uno con tool y arguments) y session opcional.
Admite read, search, outline, read_symbol, config, detail, git_status y git_diff con sus
nombres harness_ completos. Una session en el lote reemplaza la de cada hijo.
No admite external_scope, verify, edición, checkpoint/tareas, diagnóstico, SQL ni batch anidado.
Se decodifican todos los argumentos antes de iniciar cualquier hijo; comprobaciones
de rutas/rangos siguen ejecutándose por hijo. No resuelve dependencias ni ofrece
un snapshot consistente del workspace: atomic=false.

Conserva el orden y el estado/error de cada índice, aunque otro hijo falle. Hay
dos coordinadores como máximo y dos hijos concurrentes por lote. El coordinador
no consume uno de los cuatro cupos de trabajo normales; cada hijo conserva su
admisión, lease, métricas, límite de búsquedas y el plazo/cancelación del lote.
Puede haber seis operaciones activas en métricas al contar coordinadores e hijos;
los trabajos normales siguen limitados a cuatro. La saturación rechaza, no encola.

Si la respuesta agregada supera 20000 unidades UTF-16, devuelve estados compactos
y guarda los resultados completos en el detail del lote. Los refs propios de los
hijos siguen siendo locales, acotados y sujetos a la política de caché existente.
Las métricas cuentan tanto lote como hijos: no sumar ambas como si fueran RPC
independientes entregadas al cliente. No se agregan herramientas Node pendientes.

## Métricas de ejecución

Doctor devuelve `metrics`; checkpoint devuelve `stats` con la misma cobertura
tanto en el resumen como en detail. Son contadores de la instancia Application,
compartidos entre clones y sesiones: `sessionSpecific=false`, `persistent=false`.
Se reinician al crear la instancia; no se restauran del histórico de sesión.

Incluyen intentos, admitidas, rechazadas, completadas, abandonadas, activas,
pico de activas y resultados ok/error/cancelled/timeout, totales y por herramienta.
Los rechazos distinguen saturación, cancelación y timeout previos a admisión.
Las duraciones total/máxima usan microsegundos de reloj monotónico desde la
reserva hasta el resultado terminal o abandono; incluyen espera del worker,
no son tiempo de CPU ni una medición separada de cola. Las sumas de operaciones
concurrentes pueden superar el uptime. No se publican percentiles estimados.

La consulta actual figura como activa y no completada en su propio snapshot.
Una operación reservada y descartada, o que termina por unwind sin envelope,
cuenta como abandoned: eso no demuestra que no haya ejecutado código. El guard
retiene la medición hasta el resultado real del worker, incluso si el cliente
ya recibió una cancelación. No cuenta errores de protocolo/deserialización
rechazados por el SDK antes de entrar a la aplicación ni entrega/facturación RPC.
Con --persist-logs la fase de archivo ocurre después: los permisos pueden seguir
ocupados mientras el contador ya muestra la herramienta como completada.
El estado medido es el del envelope: un doctor completado que diagnostica un
problema local no equivale a un fallo de ejecución del propio doctor.

Sin rutas, argumentos, IDs de sesión, mensajes ni contenidos en los contadores;
el número de claves por herramienta queda limitado por el catálogo compilado.
Aún faltan métricas Node de bytes, lecturas/repeticiones y desglose por sesión.

## Diagnóstico nativo

`harness_doctor` es de solo lectura: comprueba el runtime, localización de
Cargo/Rustc/Git, la existencia de la raíz y las rutas de sesión/índice. El diagnóstico no crea
directorios, prueba escritura, ejecuta builds/tests, sincroniza índices ni repara
conexiones. Con --persist-logs, la instrumentación común puede guardar el evento
después del diagnóstico; sin esa opción no hay escritura de logs.
La presencia de .codegraph se informa como warning: no demuestra
integridad ni vigencia. CodeGraph y SQL quedan skipped; connection=true registra
la solicitud pero no inventa una conexión comprobada.

versions=false por defecto (diferencia deliberada con Node): no ejecuta procesos.
versions=true permite exclusivamente --version de Cargo/Rustc/Git, con 2 segundos
y 4 KiB combinados por proceso. Solo devuelve componentes numéricos de versión;
ni stdout/stderr bruto, rutas de ejecutables ni valores de configuración. Los
shims instalados de toolchain pueden consultar su gestor o selección de proyecto;
el diagnóstico no es un sandbox de red. Un timeout/fallo de sonda es warning y
versionChecked=false, no una versión válida. Cancelación/plazo global detienen
la operación. Herramientas opcionales ausentes son warnings, no fallos del MCP.
health resume únicamente los checks locales realizados, no las conexiones skipped.

La detección nativa comparte la resolución del ejecutor: entradas absolutas de
PATH y ejecutable directo, sin .cmd/.bat ni búsqueda en el directorio actual.
La detección del bootstrap aplica esa misma regla a Cargo/Rustc/Git.

## Git de solo lectura

Requiere Git instalado en una entrada absoluta de PATH, no Node. La raíz debe
contener un directorio .git real; linked worktrees con .git archivo no se soportan
todavía. Comprueba que core.worktree no redirija la consulta fuera del workspace.
No hace fetch, stage, commit ni push. No es un sandbox para repositorios hostiles.

`harness_git_status`: includeUntracked (true por defecto), offset=0..1000000 y
limit=1..100 (30 por defecto). Porcelain v2 con registros NUL, estado de rama,
HEAD/unborn/detached, upstream y ahead/behind locales, entradas, contadores y
nextOffset. Omite submódulos y agrupa directorios untracked. Hasta 6000 unidades
UTF-16 en entradas por página; páginas omitidas quedan en detail. Una entrada
individual que no cabe produce GIT_SIZE, sin cursor estancado.

`harness_git_diff`: archivo relativo concreto (también eliminado), staged=false
(index:worktree) o true (HEAD:index), maxLines=1..500 (120), maxChars=500..12000
(6000). Devuelve preview, hunks, indicador binario y patch completo en detail
cuando hay recorte. No muestra contenido untracked, metadatos .git, rutas
restringidas ni configuraciones de conexión reconocidas. No es un diff semántico.

Usa procesos nativos acotados, sin shell: 15 segundos y 2 MiB combinados por
subproceso, con plazo global de operación de 30 segundos. No devuelve stdout
parcial como resultado completo ni expone stderr/configuración. Elimina GIT_*
heredados, impide prompts/lazy fetch mediante entorno, evita locks opcionales,
desactiva fsmonitor/untrackedCache, diff externo y textconv. Enumera solo nombres
de filtros y anula clean/process/required; nombres no seguros o más de 12 filtros
rechazan la consulta antes de status/diff. Las rutas se pasan literalmente tras
--. Se mantienen las limitaciones TOCTOU de lectura documentadas en el núcleo.

## Observación del workspace durante Cargo

`harness_verify` añade `workspaceObservation`: unchanged, changed o unknown,
comparando huellas SHA-256 de rutas y contenido antes/después de Cargo. Incluye
archivos ocultos e ignorados; no lee secretos. Excluye directorios target,
node_modules, .git/.hg/.svn y el estado/logs propios del arnés. Presupuesto por
observación: 10000 entradas, 64 MiB, 2 MiB por archivo y 5 segundos cooperativos.
Enlaces, rutas restringidas, errores o límites producen unknown sin ocultar el
resultado de Cargo. La creación inicial de Cargo.lock puede producir changed.

Esto NO certifica vigencia: no observa dependencias externas, entorno, toolchain,
cambios revertidos entre lecturas ni cambios posteriores. `verificationFreshness`
sigue not_tracked y los checks del checkpoint mantienen current=null. Aún falta
completar el seguimiento de todos los inputs de verificación.
La observación se conserva en checks observados de la sesión viva; la persistencia
v1 compatible con Node la elimina, igual que otros datos de versión.

## Flujo de sesión (0.7.0)

`harness_tasks` requiere session y acepta action=status (defecto), update o
timeline. Fases: analysis/implementation/verification/review. Estados: pending,
in_progress, blocked, completed, cancelled. Solo una fase puede estar activa;
bloquear o reabrir una fase terminada requiere nota. La nota admite hasta 300
unidades UTF-16. Una actualización idéntica no crea evento ni vuelve a guardar.
Los estados son agent-declared, nunca prueba de que una compilación pasó.

Timeline conserva 128 eventos, con after y limit=1..20 (defecto 10), nextAfter y
historyPruned. Distingue phase_updated (agent-declared) de edit_applied y
verification_finished (harness-observed). No se persiste: al reiniciar o restaurar
una sesión expulsada de caché se reinician secuencia/eventos; reiniciar el cursor.

`harness_checkpoint` requiere session y permite goal (1000 unidades UTF-16),
decisions y pending (20 elementos de 500 unidades cada lista). Actualiza solo
campos presentes; cadenas/listas vacías los borran. Devuelve resumen compacto y
contenido completo en detail. Objetivo, decisiones y fases se guardan en el mismo
formato v1 que Node. Persistencia fallida devuelve persisted=false y warning con
memoryUpdated=true; la sesión sigue disponible en memoria. Una mutación iniciada
espera su resultado aun si llega cancelación, sin fingir que no cambió el estado.

Las ediciones confirmadas y verificaciones que devuelven resultado se registran
cuando llevan session. Hasta 50 archivos editados y 20 checks por sesión; tras
restaurar son historical, no nuevos hechos observados. La persistencia elimina
versiones/flags de vigencia. Los checks observados llevan current=null: no hay
certificación de vigencia total del código todavía. Las métricas de ejecución
son de la instancia, no de la sesión. refreshFreshness=true
compara la huella local actual con los checks de la sesión viva: localFreshness
indica matches_local_workspace, local_workspace_changed, no_stable_baseline o
unknown. Solo compara baselines estables con idéntica cobertura. La comparación
se deriva para esa respuesta y su detail; no se guarda como vigencia permanente.
freshness.localObservation incluye hora, hash/código de error y cobertura.
available sigue false para vigencia total y current nunca se vuelve true.
Los checks restaurados no se revalidan. El escaneo usa el mismo presupuesto que
las huellas de Cargo, fuera del mutex global y dentro del gate de sesión; un
escritor concurrente recibe SESSION_BUSY. Una lectura sin refresh no reutiliza
evaluaciones antiguas. Los límites o archivos restringidos devuelven unknown,
sin impedir actualizaciones declaradas válidas. El checkpoint
no compacta el historial del cliente. Los fallos previos a un resultado de tool
y las publicaciones inciertas no se convierten en hechos confirmados.

Un gate por sesión serializa mutaciones, verificaciones y su guardado; un segundo
escritor recibe SESSION_BUSY, no se encola. Lecturas/estado de la sesión siguen
disponibles mientras se guarda. Se conserva el lease hasta terminar y se guarda
un snapshot propio fuera del mutex global del repositorio, evitando publicaciones
fuera de orden entre clones. La creación/restauración inicial sigue usando el I/O
del adaptador anterior. No ejecutar servidores escritores concurrentes sobre la
misma sesión/árbol. Un fallo al registrar hechos o guardarlos se informa mediante
sessionWarning sin ocultar el resultado real de una edición o compilación.

## Navegación Rust (0.6.0)

`harness_outline` recibe `path`, `query`, `maxChars` (1000–16000; defecto 8000)
y `session` opcional. Devuelve firmas plegadas, contenedores, líneas, documentación,
hash SHA-256 y symbolId. query prioriza coincidencias léxicas; no filtra el resto.
El presupuesto cuenta la suma de items JSON en unidades UTF-16, no tokens ni el
envelope completo. Las omisiones se conservan en detalle cuando el almacén lo admite.

`harness_read_symbol` recibe `path`, `symbol`, `container`, `symbolId`, `view` y
`session`. Rechaza nombres ambiguos; un ID anterior a un cambio de archivo produce
STALE_SYMBOL. view=signature no devuelve cuerpo/anclas y marca editReady=false.
full devuelve un fragmento exacto del archivo, sin reimprimir AST; cuerpos de más
de 100 líneas o 16000 bytes pasan íntegros a harness_detail con text=null.

Soporta funciones, structs, enums, unions, traits, impls, módulos, tipos, constantes,
estáticos y macro_rules con nombre; recorre módulos inline y miembros impl/trait.
No recorre declaraciones locales a bloques, imports, variantes/campos ni extern
blocks; no expande macros, carga módulos externos ni evalúa cfg. No es CodeGraph
ni información semántica de callers. Falla para otros lenguajes y Rust inválido.
Documentación procede de atributos doc; los spans pueden incluir atributos/docs,
por lo que no se promete igualdad de límites/kinds con el parser Node.

El ID usa el convenio hash:inicio:fin en offsets UTF-16 y SHA-256 recortado a 24
hexadecimales. Los spans internos son bytes UTF-8, con ajuste explícito para BOM y
shebang. Solo permanecen datos propios después de cada parse: el AST no sale del
thread y se limpia su arena de spans para no acumular fuentes entre peticiones.
El parser compartido con edición conserva el límite de 256 KiB/anidamiento 64,
y navegación admite hasta 1000 símbolos, nombres de 1024 bytes y contenedor de
4096 bytes. La cancelación se comprueba entre lecturas y declaraciones; no
interrumpe una llamada síncrona al parser que ya empezó. Sin caché de símbolos aún.

## Edición experimental (0.5.0, Windows)

`harness_replace` recibe `path`, `hash`, `oldText`, `newText` y `session` opcional.
Solo reemplaza archivos existentes. El hash SHA-256 procede de una lectura vigente;
el texto original debe aparecer una sola vez, incluidas coincidencias solapadas.
Admite variación LF/CRLF y normaliza el reemplazo al estilo de la región editada.
Conserva BOM y bytes fuera de la región. Presupuesto conjunto oldText/newText:
64000 bytes; archivo original/candidato: máximo 2 MiB. Rechaza NUL y texto no UTF-8.

Por ahora valida Rust con syn y JSON con serde_json antes de escribir; Markdown y
TXT indican `syntax.status: "not_applicable"`, no una validación de su contenido.
Los demás formatos se rechazan con PARSER_UNAVAILABLE, sin escribir. Rust tiene
un límite de 256 KiB y un filtro conservador de anidamiento de delimitadores (64),
que también puede rechazar comentarios o literales muy anidados. No comprueba tipos,
expansión de macros ni semántica. Exige candidato válido completo; a diferencia de
Node, no permite conservar errores sintácticos preexistentes.

Rechaza secretos, aliases Windows, symlinks/junctions, hardlinks, archivos readonly,
metadata de Git y toda la carpeta `.agents`, además de `AGENTS.md`. Fija los
directorios ancestros con handles que impiden renombrarlos durante la operación.
Serializa escrituras entre clones de la aplicación (EDIT_BUSY), prepara un temporal
en el mismo directorio, copia/protege la DACL original antes de escribir contenido,
sincroniza y vuelve a comprobar ruta/hash/cancelación antes de publicar.

La publicación usa ReplaceFileW con backup y sin ignorar errores de ACL/atributos.
Después relee/verifica el hash. Un fallo incierto conserva los artefactos disponibles
`.harness-edit-*`/`.harness-recovery-*` y devuelve EDIT_OUTCOME_UNKNOWN con rutas de
recuperación; no se debe reintentar sin inspeccionar el destino. Un fallo al eliminar
el backup tras éxito se informa como recoveryBackup. No hay garantía de durabilidad
ante corte de energía ni CAS contra editores/procesos externos que escriban justo
después de la última comprobación. No ejecutar varios servidores escritores sobre
el mismo árbol; esto no es un sandbox frente a procesos hostiles del mismo usuario.

La cancelación anterior a publicación deja el archivo intacto. Una vez iniciada la
publicación, el transporte espera su resultado y no oculta una edición completada
tras una respuesta de cancelación. Esto puede superar el plazo nominal de 30 s si
el sistema de archivos se bloquea. Tras desconexión o crash, releer antes de reintentar.
Registra hechos históricos de edición mediante el flujo de sesión; no implementa edición Unix;
en plataformas no Windows falla antes de preparar temporales.

## Verificación Rust (0.4.0)

`harness_verify` acepta `language: "rust"`, `scope` relativo a un directorio de
crate o a su `Cargo.toml`, y `action: "build" | "test"` (build por defecto).
Invoca Cargo con `--manifest-path`, no desde una raíz sin manifiesto. No hay
shell, comandos libres ni fallback a Node. Requiere Cargo/Rust instalados en
entradas absolutas de PATH; distribuir el harness no incluye compiladores.

Ejecuta código del proyecto (build scripts/tests), con acceso y entorno del usuario;
no constituye un sandbox. Cargo también puede descargar dependencias. El resultado
indica `passed`, código de salida y salida acotada, con detalle paginable cuando se
recorta. Un código no cero produce envelope `state: "error"`. La vigencia de la
verificación no se persiste todavía: `verificationFreshness: "not_tracked"`.

Límite combinado stdout/stderr de 2 MiB y plazo de ejecución de 30 s. Dos lectores
concurrentes usan una cola de ocho bloques de 8 KiB. La limpieza tiene un plazo
adicional de 5 s; un fallo de limpieza se informa, no se convierte en éxito.
Windows crea el hijo suspendido, lo asigna a un Job Object con kill-on-close y
solo entonces lo reanuda. Al cancelar, vencer el plazo o exceder salida termina
el job y consulta ActiveProcesses hasta cero antes de confirmar su cierre.
También termina descendientes al salir el proceso principal. Existe una ventana
de fallo abrupto del harness entre crear el hijo suspendido y asignarlo al job.
Unix usa un grupo de procesos; no impide escapar con setsid y no confirma cierre
de todo el árbol. Las pruebas de procesos de esta etapa se ejecutaron en Windows.

No tiene aún paridad con Node: no migra configuración de verificadores personalizados,
lenguajes distintos de Rust/Go/Python/JavaScript ni diagnósticos Rust estructurados. Node usaba Cargo check con
scope de paquete; esta versión usa build/test con scope de ruta explícito.

## Verificación JavaScript nativa (parcial)

`harness_verify` acepta language="javascript" o "js", scope explícito de un archivo
.js/.cjs/.mjs local, action="build"|"test". build significa comprobación sintáctica
con `node --check`, sin ejecutar el archivo; no transpila, empaqueta ni resuelve
imports para verificar sus dependencias. test ejecuta únicamente la ruta indicada
con `node --test --test-reporter=tap --test-concurrency=1`. Requiere Node compatible
con estas opciones (probado con Node 22); no ejecuta npm, scripts de package.json,
Jest/Vitest ni instala paquetes. JSX/TypeScript y directorios se rechazan.

El harness sigue siendo un ejecutable Rust sin node_modules. Node es un runtime
externo opcional para proyectos JavaScript, como Python/Go/Cargo para sus lenguajes:
si falta, solo esta verificación devuelve PROCESS_EXECUTABLE. Las demás herramientas
nativas siguen disponibles sin Node. Bootstrap/doctor lo descubren con PATH absoluto.

El hijo recibe NODE_OPTIONS y NODE_PATH vacíos, sin NODE_V8_COVERAGE heredado.
No se alteran variables globales. Las rutas se validan dentro del workspace y se
pasan como ./ruta, evitando el problema del prefijo Windows verbatim en el runner;
se rechazan metacaracteres glob para no ampliar el archivo solicitado. No acepta
flags del usuario ni scopes externos. Los imports del proyecto sí pueden ejecutar
código y acceder a red/archivos: no es sandbox ni auditoría de dependencias.

Comparte el plazo de 30 s, salida 2 MiB, cancelación, Job Objects, gate y huella
local parcial. verificationMode distingue node_syntax_check y node_test_file.
passed solo refleja exit code cero; testCountVerified=false, sin certificar cantidad
ni cobertura de tests. La salida puede contener fuente o secretos del proyecto.
Prueba real opt-in: `cargo test --locked --lib real_javascript_check_tests_and_failures -- --ignored --nocapture`.
Se probaron CommonJS/ESM, check sin ejecutar, tests aprobados/fallidos y sintaxis
inválida con Node instalado, sin paquetes adicionales.
Referencia: [CLI de Node 22](https://nodejs.org/download/release/v22.16.0/docs/api/cli.html).

## Verificación Python nativa (parcial)

`harness_verify` acepta language="python" con scope explícito relativo al workspace:
build requiere un archivo .py y ejecuta `python -I -S -m py_compile <archivo>`.
Solo comprueba sintaxis, sin importar/ejecutar ese archivo; no es type checking ni
lint. Escribe bytecode normal en __pycache__ y puede cambiar la huella local.
test requiere un directorio y ejecuta `python -I -S -B -m unittest discover -s
<directorio> -t <directorio> -p test*.py`. El patrón es fijo, sin flags del usuario.
Discovery puede importar paquetes, ejecutar load_tests y recorrer subpaquetes:
el alcance autorizado es el directorio completo, no un archivo individual.

Requiere Python 3 instalado como `python`/python.exe en una entrada absoluta de
PATH; no usa py.exe, python3 ni instala un intérprete o paquetes. -I evita el
directorio actual y user site-packages en el arranque e ignora variables PYTHON*;
-S omite site y sus hooks. No se cargan automáticamente dependencias de terceros,
pytest ni configuración de proyectos. unittest añade después su directorio de
descubrimiento al import path. -B evita escribir bytecode en imports de tests,
pero no evita escrituras explícitas de los tests. No es un sandbox ni bloqueo de red.

Comparte límites de 30 s/2 MiB, cancelación, gate de sesión y resultados del
ejecutor Rust. verificationMode distingue py_compile_syntax_only y
unittest_discovery. passed indica código de salida cero, no cantidad de tests:
unittest puede devolver éxito con cero tests; testCountVerified=false lo explicita.
verificationFreshness permanece not_tracked; no ejecuta permisos externos.
La salida puede contener fuente/secretos del proyecto; no equivale a config redactada.

Prueba real opt-in (no obliga a instalar Python para correr la suite estándar):
`cargo test --locked --lib real_python_compilation_unittest_and_failures -- --ignored --nocapture`.
Se comprobaron compilación sin ejecutar fuente, rechazo de sintaxis inválida,
tests aprobados/fallidos y que un py_compile.py local no sustituye la stdlib.
Referencias: [opciones del intérprete](https://docs.python.org/3/using/cmdline.html),
[py_compile](https://docs.python.org/3/library/py_compile.html) y
[unittest](https://docs.python.org/3/library/unittest.html).

## Verificación Go nativa (parcial)

`harness_verify` también acepta language="go", action="build"|"test" y scope
relativo a un directorio de módulo con go.mod propio, o al archivo go.mod. Verifica
solo el paquete raíz (`.`), no todos los subpaquetes ni un workspace go.work.
Requiere un Go instalado compatible con `-C`; el ejecutable no incluye compiladores.
Invoca `go -C <directorio validado> build/test -mod=readonly -buildvcs=false .`,
y añade `-count=1 -timeout=20s` para tests. No acepta flags ni comandos libres.

El proceso hijo usa GOENV=off, GOFLAGS vacío, GOCACHEPROG vacío, GOWORK=off,
GOTOOLCHAIN=local, GO111MODULE=on, GOPROXY/GOSUMDB=off, GONOPROXY=none,
GOPRIVATE vacío, GOVCS=*:off y CGO_ENABLED=0; elimina GOOS/GOARCH/GOEXPERIMENT
heredados. No cambia variables globales del harness. Esta política deshabilita
descargas automáticas del comando Go y hooks heredados reconocidos; no es un
sandbox ni bloquea la red o el sistema de archivos para código ejecutado en tests.
Los reemplazos locales de go.mod y cachés externas no se auditan como inputs.
Puede escribir caché, go.sum, temporales o binarios normales de build. Los permisos
externos de lectura no habilitan este ejecutor. No hace go mod tidy ni go generate.
Referencias: [comando Go](https://pkg.go.dev/cmd/go) y
[selección de toolchain](https://go.dev/doc/toolchain).

Conserva límites de 30 s/2 MiB, cancelación, Job Objects y huella local parcial del
ejecutor existente. Una caché fría puede agotar el plazo antes de ejecutar tests:
se informa timedOut/passed=false, no éxito. verificationFreshness sigue not_tracked.
La salida del compilador/tests es contenido del proyecto y puede contener secretos;
no equivale a la política de redacción fuerte de harness_config.

La prueba real Go es opt-in para no exigir Go a cada consumidor de la suite:
`cargo test --locked --lib real_go_build_test_and_failure -- --ignored --nocapture`.
En Windows se comprobaron build, test y fallo de compilación con Go instalado; una
primera compilación de tests agotó 30 s y la repetición con caché preparada pasó.

Bootstrap usa `session` e `includeRules`, devuelve reglas, lenguajes de la raíz y
disponibilidad de herramientas en PATH (sin ejecutar comandos). Reutiliza hashes
para omitir reglas/capacidades sin cambios. Las reglas largas incluyen una
referencia recuperable en `harness_detail`, sin perder el contenido completo.
La detección PATH no reproduce todavía los localizadores especiales de .NET.

Conserva hasta 16 sesiones en memoria y restaura sesiones de
`.agents/harness/.state/<uuid>.json`, formato v1 compartido con Node. Guarda nuevas
sesiones mediante archivo temporal y rename, con límite de 256 archivos y 256 KiB
por sesión. Si no puede guardar, bootstrap devuelve `persistenceWarning` y conserva
la sesión en memoria. Solo restaura campos permitidos: objetivo, decisiones, fases,
ediciones y verificaciones históricas; descarta flags de vigencia y objetos extras.
Las sesiones con operaciones activas tienen leases en memoria y no se descartan
para admitir otras sesiones; si todas están ocupadas se devuelve `SESSION_LIMIT`.
Las tareas/checkpoints y hechos históricos se actualizan con el flujo descrito arriba;
Las métricas son volátiles de la instancia y el refresh solo de cobertura local.
Evicta sesiones antiguas de la caché, sin
borrar sus archivos. No ejecutar servidores concurrentes que escriban la misma sesión.

Read devuelve rango, texto UTF-8 y hash SHA-256 del archivo entero;
incluye anchors con hasta dos líneas antes/después, limitadas a 1000 unidades
UTF-16 por lado sin cortar caracteres. Conserva límites de 100 líneas, 16000 bytes de fragmento y
2 MiB por archivo. Rechaza rutas absolutas, traversal, secretos y enlaces/reparse points.
El rechazo de `..` es más estricto que el servidor Node, incluso para rutas internas.
No está endurecido contra un proceso hostil que cambie directorios ancestros
durante la lectura; requiere una revisión específica antes de usarlo en producción.

No hay fallback automático a Node. Las herramientas no migradas no se anuncian.
Detail conserva hasta 64 respuestas y 8 MiB por proceso, con vencimiento de 10
minutos comprobado al consultar/guardar. Los offsets y tamaños son unidades UTF-16,
como en JavaScript; `nextOffset` evita cortar pares sustitutos. Las referencias
no sobreviven a un reinicio. Si un detalle excede el presupuesto se indica
`omitted.detailUnavailable`, sin inventar una referencia.

Search usa una biblioteca Rust, sin invocar rg ni Node. Busca texto literal o
subcadenas de rutas; respeta `.gitignore`/`.ignore` y no usa ignores globales del
usuario. Omite ocultos salvo la raíz explícita, enlaces, secretos, `.git`,
`node_modules` y `target`. Estos últimos se excluyen aun sin gitignore.
Límites: 1–200 resultados compactos (40 por defecto), 500 unidades UTF-16 por línea,
10000 archivos/resultados almacenados, 64 MiB de contenido y presupuesto de 5 s
comprobado entre entradas. La lectura por archivo conserva el máximo de 2 MiB.
`complete`, `skipped`, `limited`, los diagnósticos y `detail.complete` señalan
resultados parciales. `omitted.matches` cuenta resultados observados no mostrados,
no estima coincidencias fuera del presupuesto. Texto no UTF-8 o archivos demasiado
grandes hacen la búsqueda incompleta; los binarios con NUL se omiten normalmente.
Sin búsqueda AST, file_size ni scopes externos todavía.

## Ejecución y cancelación (0.3.0)

La aplicación admite hasta cuatro operaciones activas, de las cuales como máximo
dos pueden ser búsquedas. Reserva el cupo antes de enviar trabajo al pool bloqueante
de Tokio. No hay cola propia: al saturarse devuelve `QUEUE_FULL`. Las solicitudes
rechazadas no esperan el mutex del almacén de resultados. Estos límites pertenecen
a la aplicación y se comparten entre clones; el núcleo no depende de Tokio.

El transporte MCP enlaza su señal de cancelación con un contexto portable y aplica
un plazo de 30 s. Abortar el handler también señala al trabajador. Read y rules
comprueban el contexto entre lecturas de 32 KiB; search lo comprueba entre entradas,
lecturas y líneas. Una búsqueda puede devolver resultados parciales con estado
`cancelled` o `timeout` y `detail.complete=false`. La respuesta inmediata de
cancelación del transporte no incluye necesariamente esos detalles.

El cupo y el lease se conservan hasta que el trabajador termina físicamente,
aunque el cliente ya haya recibido cancelación/timeout. Esto evita que cancelar
repetidamente acumule trabajo sin límite. La cancelación es cooperativa: no
interrumpe una llamada al sistema de archivos que ya esté bloqueada ni la
enumeración interna de un directorio. El cierre del runtime puede esperar a esos
trabajadores. La ejecución de Cargo usa el adaptador de procesos descrito arriba.

Los mensajes de entrada por stdio tienen un máximo de 128 KiB por línea JSON-RPC.
Si se supera, se cierra esa conexión; la restricción no recorta las respuestas.

Los tests verifican handshake, descubrimiento, búsqueda, detalles y restauración
tras reinicio por procesos reales con PATH vacío, además de controles de lectura,
persistencia y junctions en Windows. Hay además una prueba diferencial que usa el
SDK MCP de Node y las clases existentes `Sessions`, `SessionDisk` y `ResultStore`:
comprueba sesiones en ambas direcciones, hashes y páginas UTF-16 idénticas.
Esta prueba requiere Node y las dependencias actuales solo durante desarrollo:

```powershell
node --test .agents/harness/native/core/tests/node-compat.test.cjs
```

Por defecto usa el binario debug creado por `cargo test`. Para verificar el release,
establecer `HARNESS_RUST_BINARY` con la ruta absoluta de ese ejecutable.
La cobertura aún no equivale a paridad completa de todas las herramientas Node.

## Plan de continuación

1. Completar paridad de métricas por sesión, logs y vigencia de todos los inputs.
2. Completar búsquedas AST/scopes, paridad de límites, batch SQL y configuración.
3. Extender navegación, edición y verificadores a otros lenguajes/plataformas.
4. Prototipar AST/gramáticas y SVG con bibliotecas nativas, verificando compatibilidad y licencias.
5. Reutilizar los motores XLSX y SQL existentes mientras se migran sus adaptadores.
6. Auditar CodeGraph, navegador, launcher y hooks para identificar dependencias Node residuales.
7. Validar todos los contratos en Windows/Linux y empaquetar solo artefactos de ejecución.

Criterio de reemplazo: capacidades acordadas verificadas, pruebas de seguridad y
compatibilidad aprobadas y ejecución en una máquina sin Node/npm. No equivale a
un único archivo físico: los motores auxiliares pueden distribuirse junto al núcleo.
Medir MB, archivos, arranque y latencia antes de afirmar mejoras de rendimiento.
