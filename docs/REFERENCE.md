# Comandos y herramientas

[Inicio](../README.md) · [Instalación](INSTALLATION.md) · [Tiempos por operación](PERFORMANCE.md)

Esta referencia corresponde al código fuente `0.8.9` y al contrato `3.1.0`. El perfil `full` anuncia 22 herramientas Rust. El catálogo MCP real es la autoridad para una versión instalada.

## CLI

~~~powershell
agent-harness [--root "C:/proyectos/mi-app"] [opción]
~~~

Sin opción, inicia un servidor persistente por stdio. Sin `--root`, intenta detectar el proyecto por variables del cliente y el directorio actual; usar una ruta explícita evita ambigüedad.

El lanzador de Antigravity y el cliente `harnessmix` se conservan en
[`archive/antigravity`](../archive/antigravity/), fuera del workspace y de la
compilación de MCP.

| Opción | Función y efectos |
| --- | --- |
| `--help`, `-h` | Mostrar ayuda y terminar |
| `--version` | Mostrar la versión y terminar |
| `--init-config` | Crear configuración opcional sin sobrescribir |
| `index` | Crear un índice ausente y sincronizar las fuentes, o sincronizar uno compatible existente |
| `--index init` | Crear esquema vacío; exige que no exista `.codegraph/` |
| `--index plan` | Inspeccionar el plan de sincronización de un índice existente |
| `--index plan --summary` | Devolver conteos exactos por categoría y hasta cinco rutas de muestra por categoría |
| `--index sync` | Publicar archivos añadidos, modificados y borrados del alcance nativo |
| `--index rebuild` | Reextraer también archivos sin cambios, conservando la base |
| `--sync-file src/main.ts` | Actualizar un archivo relativo al proyecto |
| `--sync-file src/main.ts --force` | Forzar su reextracción |
| `--sync-file src/main.ts --tsconfig tsconfig.json` | Seleccionar un tsconfig explícito para aliases |
| `--persist-logs` | Iniciar MCP con historial de logs persistido |

Aliases conservados: `--init-index` = `--index init`, `--sync-plan` = `--index plan`, `--sync` = `--index sync`, `--reindex` = `--index rebuild`. `--force` y `--tsconfig` se muestran como variantes separadas; no se anuncia su combinación.

`--sync-plan --summary` es alias de `--index plan --summary`. El resumen conserva `added`, `modified`, `retryExtraction`, `removed`, `unchanged` y `unchecked`, con `count`, `samplePaths` y `omittedCount` en cada categoría, además de las banderas de cobertura y seguridad. Las rutas se mantienen completas; el presupuesto serializado limita muestras y respuesta a 12 KiB. `detailsIncluded:false` y `omittedSections` indican los campos omitidos. Sin `--summary`, la respuesta completa permanece igual. El resumen se construye después de calcular el plan y no reduce ese cálculo. `--summary` con `sync`, `rebuild`, `init` o `sync-file` es un error.

Las operaciones de índice muestran progreso en stderr y JSON en stdout. `durationMs` mide el comando; la sincronización añade `indexingDurationMs` y tiempos por archivo en informes acotados. Los límites CLI son 30 s para init/plan/sync-file y 300 s para sync/rebuild; son presupuestos cooperativos, no mediciones ni garantías de duración.

## Catálogo MCP

La búsqueda de texto intenta primero la coincidencia literal. Si completa el
recorrido sin coincidencias, una consulta de 2 o más palabras (hasta 16 términos
distintos, solo letras, números y guiones bajos) activa recuperación por términos
y ranking con la biblioteca `bm25`, sin flags, embeddings ni índice persistente.
`retrieval: {mode:"terms", ranking:"bm25", exactMatches:0}` identifica ese resultado;
`score` ordena las filas por relevancia léxica. La recuperación incluye por defecto
dos líneas de contexto con hash; `contextLines` permite cambiar esa ventana.
No infiere sinónimos o intenciones.
Regex, AST, consultas con puntuación y opciones explícitas `regex`/`caseSensitive`
conservan su comportamiento. Se mantienen scopes, globs y contexto; ambas pasadas
comparten el plazo de 15 segundos. Los límites de archivos/bytes son por pasada y
`scanned`/`bytesRead` suman el trabajo de ambas. Un recorrido incompleto no activa
la segunda pasada. La paginación conserva el orden calculado en el snapshot.

Los ejemplos son argumentos JSON de una llamada a la herramienta de la primera columna. Sustituye rutas, hashes y referencias por valores reales. Añade `"session":"ID"` donde el schema lo admita; jobs no acepta ese campo.

| Herramienta | Acción o ejemplo mínimo | Resultado |
| --- | --- | --- |
| `harness_bootstrap` | `{"compact":true}` | Raíz, sesión, catálogo y límites; reutilizar la sesión |
| `harness_search` | `{"kind":"text","query":"answer","scope":"src"}` | Páginas con `limit`; continúa con el `nextCall`/`cursor` del snapshot |
| `harness_read` | `{"path":"src/main.ts","start":1,"end":40}` | Texto UTF-8 y hash; `lines` puede sustituir a `end` |
| `harness_symbols` | `{"action":"outline","path":"src/main.ts"}` | Estructura local sin requerir índice |
| `harness_explore` | `{"query":"answer","filePath":"src/main.ts","symbol":"answer","view":"impact"}` | Relaciones indexadas y límites de cobertura |
| `harness_detail` | `{"ref":"REF","offset":0,"maxChars":4000}` | Página de contenido omitido, no una nueva búsqueda |
| `harness_data_query` | `{"resultRef":"REF","query":"rows[].name"}` | Proyección JMESPath; alternativas excluyentes: `path` o `data` |
| `harness_git` | `{"action":"status"}` | Estado Git; `{"action":"diff"}` consulta diferencias |
| `harness_config` | `{"path":"Web.config","section":"appSettings"}` | Sección XML/JSON con redacción heurística |
| `harness_doctor` | `{"action":"env","versions":false}` | Entorno y capacidades; otras acciones abajo |
| `harness_ignore` | `{"action":"list"}` | Consultar política; también `init`, `add`, `remove` |
| `harness_external_scope` | `{"action":"list","session":"ID"}` | Scopes externos de la sesión; grant/revoke explícitos |
| `harness_session` | `{"action":"status","session":"ID"}` | Estado persistente; también tasks/checkpoint/handover |
| `harness_sql` | `{"action":"status","engine":"postgresql","connect":false}` | Estado del motor; [configuración SQL](SQL-QUICKSTART.md) |
| `harness_xlsx` | `{"path":"datos.xlsx"}` | Inspección; con `sheet` lee rango o página |
| `harness_svg` | `{"path":"icon.svg","mode":"inspect"}` | Inspección o `preview` de SVG estático |
| `harness_verify` | `{"language":"typescript","scope":"src/main.ts","action":"build"}` | Verificación acotada por lenguaje |
| `harness_jobs` | `{"action":"list"}` | Trabajos, historial, observación y cancelación |
| `harness_batch` | `{"jobs":[{"tool":"harness_read","arguments":{"path":"src/main.ts","start":1,"end":5}}],"maxConcurrent":2}` | Lecturas independientes; hasta 4 hijos |
| `harness_write` | `{"path":"src/new.ts","content":"export const n = 1;\n"}` | Crear sin sobrescribir, con validación disponible |
| `harness_replace` | `{"path":"src/main.ts","hash":"SHA256","oldText":"42","newText":"43","preview":true}` | Preview, o publicar omitiendo `preview` |
| `harness_move` | `{"from":"src/old.ts","to":"src/new.ts"}` | Mover sin sobrescribir ni actualizar imports |
| `harness_delete` | `{"path":"src/obsolete.ts"}` | Eliminar permanentemente un archivo autorizado; `expectedHash` opcional |
| `harness_proposal` | `{"action":"prepare","path":"src/main.ts","content":"export const n = 2;\n"}` | Guardar original, candidato y diff sin modificar el archivo; [detalles](proposals.md) |

Los nombres antiguos de outline/read_symbol, SQL por acción, Git por acción, logs, tareas y checkpoint no son herramientas separadas anunciadas por Rust. Por ejemplo, el preview es `harness_replace` con `preview:true`; observar trabajos es `harness_jobs` con `action:"observe"`.

## Símbolos y grafo

- HTML: parser Rust `tl`, elementos/atributos/padres y lectura exacta mediante
  `symbol:"html:#guardar"`; [selectores y límites](html-parser.md).
- `harness_symbols action="read"`: `{"path":"src/main.ts","symbol":"answer","view":"full"}`. Usa `signature` si no necesitas el cuerpo.
- `outline` admite `query`, `filterKind:"method"` y `view:"compact"`. La vista compacta omite cuerpos y documentación.
- `action:"extract_preview"` prepara una extracción Vue; no publica archivos.
- `harness_explore` admite `view:"summary"|"full"|"impact"`, selectores exactos y límites de profundidad/presupuesto.
- `syncIndex:true` es una mutación explícita del índice existente. No es necesario para consultas normales y puede dejar commits por archivo.
- `requireMatchingFiles:true` pide comprobaciones de fuentes. Una búsqueda vacía y acotada no demuestra ausencia de referencias.
- `detail.ref`, `nextCall`, frescura, ambigüedad y campos de omisión forman parte del resultado; recupera lo que falte antes de editar.

## Edición

HTML (`.html` y `.htm`, sin distinción de mayúsculas) se puede crear y editar con
`harness_write` y `harness_replace` sin `allowUnvalidatedText`. Las propuestas
también admiten HTML directamente. Se procesa como texto UTF-8 en Rust, sin
iniciar Node ni un parser externo por cada edición. Se conservan las protecciones
de rutas, hashes y publicación atómica. La respuesta indica
`syntax.status="not_checked"`, `language="html"` y `validation="none"`:
esto no certifica HTML ni expresiones Angular. Para verificar un proyecto Angular,
usa `harness_verify` con `language:"angular"` y el scope del proyecto; esa
comprobación explícita utiliza sus dependencias locales.

Lee la implementación completa y usa su texto exacto y hash. `chunks` permite reemplazos únicos no solapados sobre el mismo original. Ante `STALE`, relee; no fuerces una escritura para evitar el rechazo.

`harness_write` admite `createParents:true` y `allowUnvalidatedText:true` para formatos sin parser; este último no omite errores de formatos soportados. `expectedHash` permite reemplazar un archivo existente solo si su hash coincide. Sin él, contenido idéntico es un no-op y contenido diferente no se sobrescribe. `bom:true|false` controla UTF-8 BOM; por defecto conserva el existente y usa BOM para Razor en Windows.

Validación de sintaxis no equivale a typecheck, compilación ni corrección. Se rechazan rutas protegidas, enlaces y escapes del proyecto. `.agents/knowledge/**/*.md` es la excepción Markdown admitida dentro de `.agents`. Un fallo puede informar carpetas creadas que quedaron conservadas.

## Diagnóstico, trabajos y sesiones

| Acción | Ejemplo |
| --- | --- |
| Sintaxis | `harness_doctor {"action":"syntax","files":["src/main.ts"]}` |
| Logs | `harness_doctor {"action":"logs","limit":10}` |
| Proceso | `harness_doctor {"action":"process","pid":1234}` |
| HTTP | `harness_doctor {"action":"health","healthUrl":"http://127.0.0.1:3000/health"}` |
| Lanzar trabajo | `harness_jobs {"action":"start","verify":{"language":"rust","action":"test"}}` |
| Consultar/cancelar | `harness_jobs {"action":"status","taskId":"ID"}`, o `action:"cancel"` |
| Eventos | `harness_jobs {"action":"events","after":0}` |
| Observar salida | `harness_jobs {"action":"observe","taskId":"ID","filter":"error","lines":20}` |
| Checkpoint | `harness_session {"action":"checkpoint","session":"ID","goal":"Objetivo"}` |
| Handover | `harness_session {"action":"handover","session":"ID","kind":"summary"}` |

Jobs admite dos ejecuciones simultáneas, cola acotada, reintentos limitados, cancelación e historial persistido. No tiene deadline de ejecución por defecto; `timeoutSeconds` lo establece. Las notificaciones dependen del cliente. Un checkpoint es estado/historial, no un snapshot restaurable ni undo/redo.

Los verificadores soportan Rust, Go, Python, JavaScript, TypeScript/TSX, Angular y .NET dentro de sus planificadores. Rust puede seleccionar `package`; .NET exige `scope` explícito de proyecto/solución y SDK o MSBuild disponibles. El build TS/TSX comprueba sintaxis nativa: no ejecuta ese archivo ni comprueba tipos; sus tests no están implementados. Otras verificaciones pueden ejecutar código del proyecto y no son un sandbox.

### Compilador oficial de Angular

`harness_verify {"language":"angular","action":"build","scope":"frontend:/","project":"web","configuration":"development"}` invoca las APIs de `@angular/compiler-cli` y TypeScript instalados en ese proyecto mediante Node, sin shell, npm, npx, descargas ni instalaciones automáticas. En modo de una raíz usa `scope:"."`. `project` es el nombre de la aplicación o biblioteca en `angular.json`, no el alias del workspace Harness; se puede omitir cuando hay un solo target build. `configuration` selecciona opciones de build (incluido su `tsConfig`), y por defecto respeta `defaultConfiguration`. También se admite `scope:"tsconfig.app.json"` directamente, omitiendo `project` y `configuration`.

El compilador oficial comprueba TypeScript y las plantillas inline o externas con `noEmit:true`; no genera bundles ni sustituye `ng build` para assets, estilos, `fileReplacements` o empaquetado. La configuración de build selecciona el tsconfig; las transformaciones del builder no se ejecutan. Respeta la configuración Angular efectiva del tsconfig, incluido `extends`, y devuelve `strictTemplates` y un aviso si no está explícitamente habilitado. No modifica los tsconfig ni escribe archivos auxiliares de compilación. Los diagnósticos incluyen código TS/NG, severidad y archivo/línea/columna cuando el compilador los proporciona. Dependencias ausentes, selección ambigua o configuración inválida se informan como errores, nunca como una compilación aprobada. `verificationMode:"angular_official_compiler"` identifica esta verificación y `compilerVersion` informa la versión utilizada.

Para trabajos largos: `harness_jobs {"action":"start","verify":{"language":"angular","action":"build","scope":"frontend:/","project":"web"}}`. La llamada directa tiene un presupuesto de 60 segundos; jobs conserva su timeout configurable, cancelación, salida acotada e historial. Las pruebas Angular (`action:"test"`) todavía no están implementadas.

Prueba real optativa de la integración (no instala paquetes): define `HARNESS_ANGULAR_FIXTURE` con la ruta de una carpeta que tenga `@angular/core`, `@angular/compiler`, `@angular/compiler-cli` y TypeScript compatibles instalados, y ejecuta `cargo test --locked --lib angular_official_compiler_detects_real_template_errors_without_emitting -- --ignored --nocapture`.

Los tiempos representativos, la diferencia entre arranque y llamada persistente y los casos aún no medidos están en [PERFORMANCE.md](PERFORMANCE.md).
