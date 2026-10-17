# Núcleo Rust experimental

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
y `harness_search` (`text` y `files`), además de `harness_verify` para Rust
y `harness_replace` en Windows. También `harness_outline` y `harness_read_symbol`
para declaraciones Rust, más `harness_tasks` y `harness_checkpoint`.

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
conectar estas huellas al seguimiento de versiones y refreshFreshness.
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
certificación de vigencia del código ni métricas todavía. refreshFreshness=true
falla con UNSUPPORTED antes de actualizar los campos declarados. El checkpoint
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
los demás lenguajes ni diagnósticos Rust estructurados. Node usaba Cargo check con
scope de paquete; esta versión usa build/test con scope de ruta explícito.

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
no implementa aún métricas ni refresco de vigencia. Evicta sesiones antiguas de la caché, sin
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

1. Completar tareas/checkpoint, métricas, anchors y contratos compartidos con Node.
2. Completar búsquedas (file_size/syntax/scopes) y migrar procesos: presupuestos, admisión por capacidad, UTF-8 y finalización del árbol.
3. Migrar edición con hash, escritura atómica y validación sintáctica antes de guardar.
4. Prototipar AST/gramáticas y SVG con bibliotecas nativas, verificando compatibilidad y licencias.
5. Reutilizar los motores XLSX y SQL existentes mientras se migran sus adaptadores.
6. Auditar CodeGraph, navegador, launcher y hooks para identificar dependencias Node residuales.
7. Validar todos los contratos en Windows/Linux y empaquetar solo artefactos de ejecución.

Criterio de reemplazo: capacidades acordadas verificadas, pruebas de seguridad y
compatibilidad aprobadas y ejecución en una máquina sin Node/npm. No equivale a
un único archivo físico: los motores auxiliares pueden distribuirse junto al núcleo.
Medir MB, archivos, arranque y latencia antes de afirmar mejoras de rendimiento.
