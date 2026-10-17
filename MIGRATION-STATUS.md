# Estado de la migración a Rust

## Punto de control

- Rama: `migration/rust-harness`.
- Núcleo: `.agents/harness/native/core`, versión del paquete `0.7.0`.
- Objetivo: distribuir un ejecutable MCP sin Node ni node_modules en ejecución.
- Estado: migración parcial; Node sigue siendo la conexión activa para funciones pendientes.
- Este documento registra lo implementado y el trabajo restante antes de continuar.
- No existe todavía una medición fiable del porcentaje por esfuerzo; contar herramientas no mide paridad.

## Arquitectura disponible

Monolito modular con tres crates:

1. `harness-domain`: contratos, resultados, edición, navegación y fases de trabajo.
2. `harness-application`: casos de uso, puertos, admisión y coordinación de sesiones.
3. `agent-harness`: adaptadores nativos, archivos, procesos, persistencia y transporte MCP.

Hay pruebas que restringen las dependencias de los crates portables. El dominio no
depende del SDK MCP, Tokio ni APIs del sistema operativo. No hacen falta
microservicios para escalar las capacidades de este ejecutable.

## Funciones implementadas

| Herramienta / bloque | Estado actual | Límites importantes |
| --- | --- | --- |
| MCP stdio y CLI | Implementado | Entrada acotada; ejecutable experimental |
| `harness_bootstrap` | Implementado | Sesión, reglas, capacidades y guía de uso |
| `harness_read` | Implementado | UTF-8, hash SHA-256, hasta 100 líneas por fragmento |
| `harness_detail` | Implementado | Caché acotada, paginado UTF-16, referencias locales al proceso |
| `harness_search` | Parcial | Text/files/file_size y syntax de declaraciones Rust; faltan patrones AST generales |
| `harness_config` | Parcial | Sección XML/JSON redactada; no settings runtime ni valores no sensibles |
| `harness_external_scope` | Parcial | Windows: archivo/carpeta, sesión, TTL, identidad y revocación; sin ejecución |
| `harness_verify` | Parcial | Rust/Go/Python; Python py_compile de archivo o unittest de directorio, sin pytest |
| `harness_replace` | Parcial | Publicación segura en Windows; Rust/JSON/Markdown/texto |
| `harness_outline` | Parcial | Declaraciones Rust; sin relaciones semánticas ni expansión de macros |
| `harness_read_symbol` | Parcial | Rust, ambigüedad y detección de symbolId obsoleto |
| `harness_tasks` | Implementado | Fases, transiciones y timeline acotado en memoria |
| `harness_checkpoint` | Parcial | Objetivo, decisiones, pendientes e histórico; refresh local acotado |
| `harness_git_status` | Implementado | Solo lectura, paginado; requiere Git, sin linked worktrees |
| `harness_git_diff` | Implementado | Archivo concreto, staged/worktree y eliminados; sin filtros externos |
| `harness_doctor` | Parcial | Diagnóstico local; versiones opcionales; no CodeGraph/SQL/conexiones |
| `harness_batch` | Parcial | 1..4 lecturas, 2 hijos concurrentes; sin SQL ni herramientas pendientes |
| `harness_logs` | Parcial | 512 eventos, requestId y snapshots Windows opt-in; sin configuración/trazas completas |
| Métricas | Parcial | Operaciones, resultados, rechazos y tiempos por instancia/herramienta; no histórico por sesión |
| Sesiones en disco | Implementado | Formato v1 compatible con Node; histórico no equivale a vigencia |
| Procesos nativos | Implementado | Windows Job Objects, cancelación y límites; Unix con garantías menores |
| Huella antes/después de Cargo | Parcial | Detecta cambios locales observados; no certifica vigencia completa |

### Seguridad y coordinación existentes

- Validación de rutas; rechazo de traversal, secretos, enlaces y reparse points.
- Lecturas, búsquedas, resultados y procesos con presupuestos acotados.
- Admisión de operaciones y leases que conservan las sesiones mientras trabajan.
- Exclusión de ediciones concurrentes y gate de escritura/verificación por sesión.
- Guardado de snapshots fuera del mutex global del repositorio.
- Reemplazos con hash, validación previa, archivo temporal y verificación posterior.
- Preservación de DACL en Windows y artefactos de recuperación ante publicación incierta.
- Un resultado incierto no se presenta como archivo intacto ni edición confirmada.

### Último bloque implementado

Verificador Python nativo por comandos stdlib cerrados, con -I/-S: compilación
sintáctica de archivo explícito o unittest discovery de directorio explícito.
No instala paquetes ni evalúa fuente en build; test sí ejecuta código del proyecto.
Reutiliza ejecutor, gate, huella local y límites existentes. Doctor y bootstrap
detectan python por la política de PATH absoluto. No hay nuevos crates/herramientas.
Siguen pendientes pytest, entornos/dependencias configurables, lint/type checking
y paridad completa. Una salida cero de unittest no certifica que existan tests.

### Bloque anterior: verificación Go

Verificador Go nativo con argumentos cerrados, módulo/ruta local validada y
política de entorno del hijo sin GOFLAGS/hook de caché, descargas, cgo o go.work.
Comparte ejecutor, límites, gate de sesión, resultados y observación local con
Rust. Build/test solo del paquete raíz; faltan subpaquetes, workspaces, cgo y
paridad multilenguaje completa. Doctor y bootstrap detectan Go sin exigirlo para
las demás herramientas. No se añaden dependencias ni herramientas MCP.

### Bloque anterior: búsqueda syntax

Búsqueda syntax de declaraciones Rust por selector kind[:nombre exacto], usando
el colector AST existente sin dependencias nuevas ni Node. Respeta ignores y
permisos externos, admite batch y devuelve ubicación/hash/symbolId/firma, sin
cuerpos. Errores del parser quedan sanitizados y hacen explícita la cobertura
incompleta. No incluye expresiones, patrones ast-grep, items locales, expansión
de macros ni relaciones semánticas. Se mantienen 17 herramientas.

### Bloque anterior: configuración JSON

Configuración JSON redactada en el mismo adaptador nativo, sin dependencias nuevas.
Sección de primer nivel exacta; nombres de propiedades y escalares ocultos antes
de preview/detail. Presupuestos 1 MiB/10000 nodos/profundidad 64 y salida acotada.
Compatible con batch, scopes Windows y logs persistidos. Las claves duplicadas
usan el último valor; no sigue referencias ni ofrece texto editable. Se mantienen
17 herramientas y sigue pendiente la configuración dinámica del runtime.

### Bloque anterior: XML y alcances externos

Configuración XML por parser nativo y alcances externos Windows: 17 herramientas.
Config oculta todos los valores antes del recorte y no devuelve fuente en errores
ni detail; rechaza DTD/entidades, múltiples coincidencias y límites excedidos.
Scopes requieren consentimiento explícito declarado, sesión propietaria y ruta
concreta; caducan, se revocan y comprueban identidad sin conservar pins ociosos.
Pueden usarse en read/search/navegación/config, también dentro de batch compatible,
pero nunca conceden edición, procesos o SQL. Archivos únicos no autorizan vecinos.
Son coberturas parciales: valores no sensibles también redactados; faltan scopes
Unix y configuración dinámica del harness. JSON se añadió después.

### Bloque anterior: file_size

file_size nativo dentro de search, también usable por batch, sin Node/ripgrep ni
CodeGraph. Cuenta LF/CRLF, vacío y última línea; filtra >minLines y devuelve solo
metadata ordenada. Reutiliza validación de rutas/lecturas estables, presupuestos,
ignores y resultados parciales. Límites explícitos más estrictos que Node:
2 MiB/archivo, 64 MiB de reservas, 10000 archivos, 4 MiB de metadata y 5 segundos.
Se mantiene el total de 15 herramientas, ampliando una existente.

### Bloque anterior: persistencia de logs

Persistencia y rotación Windows con --persist-logs (apagada por defecto). Archivo
nativo separado de Node: current/previous y staging fijo, 256 KiB por snapshot,
512 eventos. Directorios fijados, hardlinks/reparse points rechazados, escritor
exclusivo y formato estricto. Recupera staging válido y marca eventos restaurados
como históricos. Un fallo de archivo no modifica el resultado de la herramienta;
se publica loggingWarning y el estado de persistencia. No se restauran métricas.
Se probaron reinicio del ejecutable sin Node, rotación, locks, archivos ajenos,
hardlinks, recuperación y rechazo de staging obsoleto. No hay garantías de energía
ni de I/O con plazo duro. La conexión activa sigue sin cambios.

### Bloque anterior: correlación

Correlación requestId UUID v4 generada por la aplicación antes de admisión.
El envelope y su evento comparten ID, también para rechazo/cancelación/abandono;
el transporte conserva el identificador cuando responde antes que el worker.
Batch conserva IDs propios de hijos, también en salida recortada. Logs acepta
filtro UUID canónico. No es el ID JSON-RPC ni una garantía de entrega; faltan
trazas completas de transporte previas a prepare.

### Bloque anterior: logs volátiles

Logs de metadatos nativos con cursor, niveles, herramienta y lentitud: 15
herramientas publicadas. Eventos de resultado/rechazo/abandono enlazados al guard
de medición. Buffer acotado de 512 entradas, códigos permitidos, sin contenidos,
argumentos ni rutas. Doctor/bootstrap muestran su estado. No se leen logs Node;
el modo por defecto no escribe en disco y las trazas completas siguen sin soporte.

### Bloque anterior: batch

Batch integra las lecturas migradas (14 herramientas publicadas). Tipos de todos
los argumentos validados antes de ejecutar, resultados ordenados por índice,
errores parciales y detalle agregado acotado. Máximo dos coordinadores fuera de
los cuatro cupos normales; hijos sujetos a admisión, cancelación, plazo, leases y
métricas comunes. No mutaciones, recursión, verify ni SQL. No es transacción.

### Bloque anterior: métricas

Métricas acotadas de ejecución en doctor.metrics y checkpoint.stats (también en
detail). Compartidas entre clones, no persistidas ni específicas de una sesión.
Contabilizan admisión/rechazo, resultados terminales, abandono, actividad/picos y
duración total/máxima. Un guard RAII evita dobles conteos y fugas al descartar
operaciones. No retienen argumentos ni rutas. No equivalen a RPC entregadas,
facturación o estadísticas de lectura de Node; esas coberturas siguen pendientes.

### Bloque anterior: diagnóstico

Diagnóstico nativo de runtime, ejecutables y rutas, sin cambios en disco. Hay 13
herramientas publicadas. Por defecto solo localiza; versions=true ejecuta --version
con 2 segundos/4 KiB por herramienta y publica solo versión numérica sanitizada.
La presencia del índice no valida su contenido. CodeGraph/SQL siguen skipped.
La detección de herramientas nativas comparte la política real del ejecutor.

### Bloque anterior: Git

Git nativo añade dos herramientas (12 publicadas). Status usa porcelain v2/NUL,
paginación y contadores. Diff admite archivos eliminados y conserva recortes en
detail. Ambos usan el ejecutor nativo, no Node ni shell; anulan filtros,
fsmonitor, diff externo y textconv. Comprueban que Git no cambie de workspace,
rechazan rutas sensibles y no hacen mutaciones. Falta linked worktrees (.git
archivo). Las pruebas usan repositorios temporales, índices sin cambios, nombres
Unicode, binarios, límites de salida y configuraciones que detectarían ejecución
accidental de filtros. El Git del sistema sigue siendo requisito externo.

### Bloque anterior: huellas y refresh local

`workspaceObservation` compara rutas/contenido SHA-256 antes y después de Cargo:
`unchanged`, `changed` o `unknown`. Incluye ocultos/ignorados, salvo salidas y
directorios excluidos documentados en README. Límites: 10000 entradas, 64 MiB,
2 MiB por archivo y 5 segundos cooperativos por observación.

No rastrea dependencias externas, entorno ni toolchain. Tampoco detecta todos los
cambios intermedios revertidos. `verificationFreshness` sigue `not_tracked` y los
checks mantienen `current=null`. Las observaciones se conservan en la sesión viva,
no en el formato persistido v1. `refreshFreshness=true` compara esas huellas con
una observación nueva: matches_local_workspace, local_workspace_changed,
no_stable_baseline o unknown. La evaluación solo existe en la respuesta y su
detail; no se reutiliza como vigencia permanente. No revalida checks restaurados
ni convierte current en true. Se serializa con verificaciones/escrituras de la
misma sesión, sin retener el mutex global durante el escaneo.

## Verificación realizada antes de este documento

- 130 pruebas Rust cubren configuración, alcances, búsqueda y planes de verificación Rust/Go/Python.
- Prueba Python opt-in pasó con intérprete real: compilación, unittest y fallos esperados.
- Prueba real Go opt-in: build/test/fallo de compilación pasaron en Windows con caché preparada.
- La primera compilación de tests Go agotó 30 s; no se amplió el límite ni se marcó éxito.
- Clippy de todo el workspace y todos los targets: sin warnings.
- Formato Cargo comprobado.
- Compilación release completada con las últimas modificaciones.
- Dos pruebas de procesos fallaron al solaparse con la compilación optimizada;
  pasaron sin esa carga. Pendiente sustituir esperas temporales frágiles por
  sincronización determinista; no ocultar esta limitación.
- Las dos pruebas de compatibilidad con Node pasaron en la etapa 0.7.0 anterior;
  no se volvieron a ejecutar en el último bloque de huellas.
- El diagnóstico del arnés reportó un error en la comprobación del índice;
  no debe interpretarse como índice validado ni conexión restablecida.

## Pendiente de migrar o completar

### 1. Vigencia y observabilidad

- [ ] Seguimiento de versiones del workspace y cobertura explícita por verificación.
- [ ] Invalidación de checks por cambios posteriores y tratamiento del estado restaurado.
- [x] Refresh de cobertura local, serializado con verificaciones de la sesión.
- [ ] Completar vigencia total (inputs externos, entorno y toolchain).
- [x] Métricas reales de ejecución por instancia y herramienta, sin valores simulados.
- [ ] Paridad de estadísticas por sesión, bytes y lecturas/repeticiones de Node.
- [x] Diagnóstico nativo local (runtime, herramientas y rutas).
- [ ] Completar diagnóstico de índice/conexiones CodeGraph, SQL y logs/métricas.
- [x] Logs acotados en memoria y consulta nativa con filtros/cursor.
- [x] Correlación por requestId de operaciones preparadas, resultados y eventos.
- [x] Logging persistido/rotado Windows opt-in, con recuperación y validación estricta.
- [ ] Persistencia Unix, configuración dinámica y trazas completas de transporte.
- [ ] Estabilizar pruebas temporizadas de procesos bajo carga.

### 2. Herramientas de repositorio

- [x] `harness_git_status` y `harness_git_diff` nativos de solo lectura.
- [ ] Soporte Git para linked worktrees con .git archivo.
- [x] `harness_batch` para lecturas nativas con admisión y cancelación compartidas.
- [ ] Extender batch a SQL y otras lecturas cuando sus adaptadores estén migrados.
- [x] Lectura XML nativa con redacción de todos los valores y límites explícitos.
- [x] Configuración JSON redactada, incluyendo nombres de propiedades.
- [ ] Configuración runtime y paridad de valores no sensibles con política segura.
- [x] Alcances externos Windows autorizados, acotados por sesión/TTL/identidad.
- [ ] Alcances externos Unix y cobertura de herramientas todavía no migradas.
- [x] Búsqueda de tamaño (`file_size`) nativa con límites explícitos.
- [x] Búsqueda syntax de declaraciones Rust con selectores kind[:nombre].
- [ ] Patrones AST generales/multilenguaje y equivalencia completa de límites/cobertura.
- [ ] Navegación, validación y edición para los demás lenguajes requeridos.
- [x] Verificación Go parcial del paquete raíz de módulo, sin descargas/cgo/workspaces.
- [x] Verificación Python parcial: py_compile de archivo y unittest de directorio.
- [ ] Verificación de .NET, JavaScript y paridad completa de Python/Go/otros adaptadores.

### 3. Relaciones e integraciones

- [ ] Integración CodeGraph para relaciones y flujos mediante `harness_explore`.
- [ ] SQL Server: configuración segura, estado, catálogo, descripción y consultas auditadas de solo lectura.
- [ ] Integrar o sustituir las utilidades XLSX existentes desde el núcleo Rust.
- [ ] Inventariar y migrar utilidades SVG, navegador/regresión web y otras herramientas auxiliares.
- [ ] Auditar el catálogo publicado de Node frente al catálogo Rust antes de declarar paridad.

Que una utilidad auxiliar ya tenga un ejecutable Rust no significa que su
herramienta MCP esté integrada en este núcleo.

### 4. Distribución y retirada de Node

- [ ] Pruebas de compatibilidad por herramienta y errores, no solo camino exitoso.
- [ ] Pruebas de instalación y ejecución en una máquina sin Node/npm.
- [ ] Definir plataformas soportadas y garantías de procesos/edición por plataforma.
- [ ] Empaquetado reproducible, documentación de instalación y actualización.
- [ ] Revisar licencias, dependencias y tamaño final del artefacto distribuible.
- [ ] Cambiar la conexión MCP solo después de cumplir los criterios de aceptación.
- [ ] Mantener un procedimiento de reversión comprobado.
- [ ] Retirar Node y node_modules únicamente cuando no quede ninguna dependencia funcional.

El ejecutable MCP puede ser autónomo sin Node, pero verificar proyectos sigue
requiriendo sus toolchains (por ejemplo, Cargo/Rust) y puede ejecutar código del
proyecto. No es un sandbox para código hostil.

## Orden recomendado para continuar

1. Completar vigencia del código y estabilizar pruebas de procesos.
2. Añadir métricas, doctor y logs.
3. Migrar Git, batch, configuración y alcances externos.
4. Completar búsqueda, parsers y verificadores por lenguaje.
5. Integrar CodeGraph, SQL y utilidades restantes según uso real.
6. Ejecutar paridad integral, empaquetar y probar sin Node.
7. Cambiar la conexión y retirar dependencias antiguas con reversión disponible.

## Criterio de finalización

La migración termina cuando todas las capacidades requeridas tienen reemplazo
probado y documentado, el cliente usa el ejecutable Rust, y las pruebas de
distribución confirman que Node/node_modules ya no son necesarios. Un build
exitoso o tener diez herramientas nativas no demuestra por sí solo esa paridad.
