# Índice automático en segundo plano

Windows x64. Activado por defecto en el perfil MCP `full`, con índice nativo
compatible ya existente. No necesita variables de entorno ni llamadas de sincronización del LLM.

Para un proyecto nuevo, crear primero el índice:

```powershell
.\agent-harness.exe --root E:\proyecto --init-index
```

La entrada MCP habitual basta; reiniciar la conexión después de actualizar el ejecutable:

```json
{
  "command": "C:/herramientas/agent-harness.exe",
  "args": ["--root", "E:/proyecto"]
}
```

`HARNESS_INDEX_WATCH=0` lo desactiva. Sin variable, los perfiles restringidos no
inician el watcher. `HARNESS_INDEX_WATCH=1` exige iniciarlo: un índice ausente,
un fallo de watcher o un perfil restringido impiden arrancar en ese modo explícito.
En el modo automático por defecto, un índice ausente o un fallo de arranque del
watcher se informa en doctor y el servidor sigue disponible; corregir la causa
y reiniciar la conexión para activarlo. No se crea el índice implícitamente.
Un valor distinto de 0 o 1 es inválido.
El watcher nunca crea ni migra un índice y no edita archivos fuente. Vive mientras
está abierto el servidor MCP; no instala un servicio Windows.

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
- Se filtran eventos de directorios generados, `.codegraph`, `.agents` y rutas
  restringidas. La suscripción nativa es recursiva; eventos masivos en subárboles
  filtrados aún pueden producir un aviso de desbordamiento del sistema operativo.

Al arrancar, cada cinco minutos, al cambiar configuraciones JSON/ignores o
directorios, y tras eventos perdidos, se reconcilia el inventario acotado.
Esa reconciliación lee/hashea fuentes para detectar cambios no notificados, pero
solo extrae/publica las modificadas o pendientes. No es un rebuild. El publicador
ya no ejecuta la vista previa de extracción del plan antes de volver a extraer.
Los eventos de metadatos Git filtrados se recuperan en la revisión periódica.

Errores: reintentos con espera creciente de 5 a 60 s; un nuevo guardado puede
adelantar la recuperación. Un lote incremental fallido reintenta sus rutas sin
forzar por sí solo un escaneo global. Los commits confirmados se conservan y
contabilizan aunque otra ruta falle. Cada operación tiene un máximo de 60 s y
cancelación cooperativa al cerrar. Un índice incompatible o relaciones legacy no
soportadas se reportan como error; no se borran para forzar éxito.

## Estado y límites

`harness_doctor` incluye `checks[id=index-auto-sync].details`: `enabled`, `state`
(`starting`, `syncing`, `idle`, `retrying`), `batches`, `reconciliations`,
`changedFilesCommitted`, `lastBatchPaths`, `lastDurationMs`, `lastError` y límites.
Los errores publicados son códigos, no rutas ni mensajes crudos del sistema.

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
