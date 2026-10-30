# Índice automático en segundo plano

Windows x64. Activado por defecto en el perfil MCP `full`. Si no existe
`.codegraph`, crea un índice nativo vacío y lo llena mediante la reconciliación
inicial en segundo plano. No necesita llamadas de sincronización del LLM.

La entrada MCP habitual basta; reiniciar la conexión después de actualizar el ejecutable:

```json
{
  "command": "C:/herramientas/agent-harness.exe",
  "args": ["--root", "E:/proyecto"]
}
```

`HARNESS_INDEX_WATCH=0` lo desactiva. Sin variable, los perfiles restringidos no
inician el watcher. `HARNESS_INDEX_WATCH=1` lo habilita explícitamente; los
fallos de inicio se informan, y un perfil restringido no admite ese modo.
Un valor distinto de 0 o 1 es inválido. La creación solo reclama un directorio
`.codegraph` ausente: no sobrescribe ni migra un índice o directorio existente.
Si otro proceso está inicializando, el supervisor reintenta hasta cinco veces;
un fallo persistente queda en estado `degraded` con su causa en doctor.

Cada instancia activa un supervisor. Si otro proceso posee `autosync.lock`,
el supervisor permanece en espera y reintenta cada dos segundos; cuando obtiene
el bloqueo inicia su watcher y reconcilia el inventario para recuperar cambios
ocurridos durante la transición. Solo el bloqueo del sistema operativo decide
la propiedad. Si un propietario sigue vivo pero está bloqueado, nadie toma el
control hasta que libere el bloqueo; no se terminan procesos automáticamente.

El propietario publica telemetría acotada en `.codegraph/autosync.status`:
PID, inicio, pulso, estado y última sincronización satisfactoria. Las demás
instancias la muestran como `observedOwner`: es información no autenticada y
puede estar desactualizada; no demuestra que el propietario esté sano. Un fallo
de telemetría no detiene el watcher. Consulte el estado y el tiempo de la última
sincronización, y no interprete el pulso como certificación global del índice.
Los procesos antiguos pueden no publicar telemetría.

El watcher no migra índices ni edita archivos fuente. Vive mientras está abierto
el servidor MCP; no instala un servicio Windows. Una extracción parcial de
JavaScript se publica solo si el archivo no tenía grafo confirmado o si ya
constaba como parcial; queda marcada (`SYNC_PARTIAL` y `graphStatus: partial`)
para reintentarse y nunca se presenta como cobertura completa. Si existía un
grafo completo, se conserva la última versión confirmada mientras se corrige la
fuente.

## Trabajo incremental

- Notificaciones nativas del filesystem con `notify` 8.2.0, sin sondeos continuos.
- Deduplicación por ruta: 600 ms de calma, máximo 2 s de agrupación bajo cambios
  continuos, antes de admitir el siguiente lote; un lote en curso puede demorar otros.
- Cola de hasta 1024 rutas. Un desbordamiento pide reconciliación en lugar de
  perder cambios o crecer indefinidamente. Los eventos durante un lote se conservan.
- Un trabajador y una conexión SQLite por lote. Lock exclusivo por proyecto para
  impedir dos watchers; las escrituras explícitas siguen la serialización y las
  comprobaciones transaccionales de SQLite. Las consultas no toman el lock del watcher.
- Eventos de archivos normales: solo rutas afectadas, sin inventario del workspace.
  Se compara el hash dentro de la transacción antes de parsear. Mismo contenido:
  cero extracción y ninguna reconstrucción de símbolos/relaciones.
- Nuevas fuentes respetan `.gitignore` locales, `info/exclude`, `include` y `exclude`.
  Los archivos ya indexados conservan la política de sincronización de archivos
  rastreados. Worktrees con fuentes nuevas usan el descubrimiento existente para
  resolver correctamente los ignores de su gitdir externo.
- Renombrados y borrados usan las protecciones del publicador nativo. Los cambios
  de relaciones reutilizan su resolución/invalidation existente; no añaden paridad
  con el proveedor original.
- Se filtran eventos de directorios generados, `.codegraph`, `.agents` (excepto
  `.agents/harness.config.json`) y rutas
  restringidas. La suscripción nativa es recursiva; eventos masivos en subárboles
  filtrados aún pueden producir un aviso de desbordamiento del sistema operativo.

Al arrancar, cada cinco minutos, al cambiar configuraciones JSON/ignores o
directorios, y tras eventos perdidos, se reconcilia el inventario acotado.
Esa reconciliación lee/hashea fuentes para detectar cambios no notificados, pero
solo extrae/publica las modificadas o pendientes. No es un rebuild. El publicador
ya no ejecuta la vista previa de extracción del plan antes de volver a extraer.
Los eventos de metadatos Git filtrados se recuperan en la revisión periódica.

Errores transitorios: hasta cinco intentos con esperas de 5, 10, 20 y 40 s;
un nuevo evento o la reconciliación periódica permite una nueva serie de intentos.
Las limitaciones de cobertura (`SYNC_INCOMPLETE`), configuración, esquema,
lenguaje o relaciones no soportadas no provocan reintentos rápidos: el watcher
queda `degraded` y espera cambios o la reconciliación periódica. Así, por ejemplo,
un archivo demasiado grande no dispara un inventario completo cada minuto.
Un lote incremental fallido reintenta sus rutas sin
forzar por sí solo un escaneo global. Los commits confirmados se conservan y
contabilizan aunque otra ruta falle. Cada operación tiene un máximo de 60 s y
cancelación cooperativa al cerrar. Un índice incompatible o relaciones legacy no
soportadas se reportan como error; no se borran para forzar éxito.

## Estado y límites

### Excluir bibliotecas del análisis semántico

Integra esta sección en `.agents/harness.config.json`, conservando otras opciones:

```json
{
  "indexing": {
    "semanticExclude": ["Content/plugins/**", "Scripts/vendor/**"]
  }
}
```

Son globs relativos a la raíz del proyecto, con `/`, hasta 128 patrones de 1000
caracteres. No se admiten rutas absolutas, `..`, negaciones ni barras invertidas.
En Windows no distinguen mayúsculas. Una lista vacía habilita el análisis habitual.
No es una exclusión del inventario: se conservan la lectura, búsqueda y extracción
básica de símbolos. Las rutas coincidentes no se usan para resolver relaciones
semánticas JS/TS, tampoco como destino de imports o reexports; esas referencias
quedan pendientes. Así un fallo del analizador semántico en una biblioteca excluida
no bloquea la publicación de los archivos de la aplicación ni dispara reintentos.
Los errores de extracción básica siguen reportándose.

El watcher detecta cambios de esta configuración y solicita una reconciliación.
La política forma parte del estado de resolución; se invalidan las relaciones
derivadas de imports para evitar conservar conclusiones de la política anterior.
Si cambia durante una publicación, la transacción se revierte. Una configuración
inválida se informa como `SYNC_CONFIG` y no se ignora silenciosamente.

`harness_doctor` incluye `checks[id=index-auto-sync].details`: `enabled`, `state`
(`starting`, `syncing`, `idle`, `retrying`, `degraded`), `batches`, `reconciliations`,
`changedFilesCommitted`, `lastBatchPaths`, `lastDurationMs`, `lastError` y límites.
`retryScheduled`, `retryDelayMs` y `consecutiveFailures` muestran la política de
reintentos. `lastResult` conserva un resumen acotado del último resultado:
contadores de cobertura, commits y fallo con código, ruta relativa cuando existe
y mensaje controlado por Harness; no incluye fuentes ni errores crudos del sistema.
`lastReconciliation` conserva ese diagnóstico del último inventario completo,
incluso después de un lote incremental exitoso. Una cobertura incompleta mantiene
el estado `degraded` hasta una reconciliación completa exitosa.

Las entradas restringidas por política se cuentan como excluidas, no como trabajo
fallido: por sí solas no bloquean `syncIndex:true` ni provocan reintentos del watcher.
Explore devuelve `SYNC_RESTRICTED_SKIPPED` como advertencia y conserva las
comprobaciones de `requireMatchingFiles`. No se leen ni se eliminan esas entradas.
`restrictedIndexedReasons` agrupa cantidades por regla: `config_filename`,
`npmrc_filename`, `appsettings_filename`, `private_key_extension`,
`sensitive_path_prefix` y `version_control_directory`. No contiene rutas ni hashes.
Esto no certifica la vigencia global del índice ni permite ignorar errores reales
de extracción, archivos sin comprobar o un inventario interrumpido.

`idle` indica el último lote terminado, no una instantánea atómica ni garantía
global de frescura. Las comprobaciones de hashes de explore siguen activas. El
watcher de índice no vuelve actuales las verificaciones históricas de tests/build.
No se afirma optimalidad universal: el coste de resolver relaciones depende del
grafo; los límites de alcance y tamaño del indexador siguen aplicándose.

Prueba MCP de desarrollo sobre un ejecutable compilado:

```powershell
node scripts/smoke-mcp.cjs target/debug/agent-harness.exe --watch-index
```

Comprueba que un cambio de fuente aparece en explore sin `syncIndex` ni `--sync`.
Node solo ejecuta la sonda; no es una dependencia del watcher.

Implementación de notificaciones: [documentación de notify](https://docs.rs/notify/8.2.0/notify/).
