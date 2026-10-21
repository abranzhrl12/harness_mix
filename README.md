# Agent Harness

Servidor MCP en Rust para que tu asistente explore, lea, edite y verifique proyectos. Un ejecutable, sin Node ni npm en ejecución.

## Instalación

Descarga el ejecutable Windows x64 desde [Releases](https://github.com/abranzhrl12/harness_mix/releases) y guárdalo en una carpeta estable. También puedes compilarlo con Rust 1.88 o posterior:

```sh
cargo build --release --locked
```

El binario queda en `target/release/agent-harness.exe` (Windows) o `target/release/agent-harness` (Unix). La distribución precompilada está orientada a Windows; otras plataformas requieren validación.

## Configuración

Añade esta entrada a la configuración MCP de tu cliente; cambia únicamente las dos rutas:

```json
{
  "mcpServers": {
    "harness": {
      "command": "C:/herramientas/agent-harness.exe",
      "args": ["--root", "C:/proyectos/mi-app"]
    }
  }
}
```

Reinicia la conexión MCP. Puedes reutilizar el mismo ejecutable en varios proyectos cambiando `--root`. No necesitas copiar este repositorio al proyecto ni crear archivos de configuración para el uso básico. El formato de registro puede variar según el cliente.

El índice se mantiene automáticamente en segundo plano por defecto en Windows,
con perfil `full` e índice existente. Agrupa eventos y actualiza los archivos
cambiados sin llamadas del LLM. No requiere añadir `env` a la entrada MCP.
`HARNESS_INDEX_WATCH=0` lo desactiva. Detalles: [AUTO-SYNC](docs/AUTO-SYNC.md).

Si el cliente inicia el proceso dentro del proyecto, puedes omitir `args`: harness detecta la raíz desde las variables del entorno o busca `.git`/`.agents` desde el directorio actual. Con `--root` eliges la carpeta explícitamente.

## Índice de código (opcional)

Para usar la navegación del grafo, ejecuta desde la raíz de tu proyecto, con el binario en PATH:

```sh
agent-harness --init-index
agent-harness --reindex
```

Después usa `agent-harness --sync` para actualizarlo. También puedes añadir `--root <proyecto>` antes de la opción. Estos comandos escriben en `.codegraph/`; `--init-index` exige que esa carpeta no exista. No hace falta un índice para leer archivos o buscar texto.

Los comandos de índice muestran progreso en `stderr`: fase, archivo actual,
archivos confirmados/total y porcentaje cuando el total ya se conoce, tiempo
transcurrido y ETA aproximada de la fase tras suficientes muestras. En una
terminal aparece una barra; al redirigir la salida se emiten líneas limitadas
a una por segundo (además de cambios de fase). Durante el descubrimiento aún
no se conoce el total y no se inventa un porcentaje. El JSON final sigue en
`stdout`, incluye `durationMs` y conserva el estado parcial y los archivos
pendientes. La sincronización añade `indexingDurationMs` y tiempos `durationMs`
en los informes de archivos confirmados (máximo 32 informes). Estos contadores
describen el alcance nativo seleccionado; no certifican cobertura global.
El servidor MCP no emite esta barra ni notificaciones de progreso.

## Crear, editar y verificar

`harness_write` crea un archivo con `path` y `content`, sin sobrescribir destinos.
`createParents=true` crea las carpetas necesarias con validación de rutas y enlaces.
Si el archivo ya contiene exactamente los mismos bytes, devuelve `already_present`;
si difiere, no sobrescribe. `allowUnvalidatedText=true` permite formatos sin parser,
indicando `not_checked`, sin saltarse errores de sintaxis de formatos soportados.
Para `.vue`, valida scripts inline JS/TS y marca templates/estilos como no comprobados.
Para `.cshtml`, realiza una comprobación básica de Razor: bloques `@section`,
`@media` sin escapar (usar `@@media`) y etiquetas HTML desbalanceadas. Los resultados
indican sus límites: no sustituyen la compilación Razor/C# ni comprueban tipos.
`harness_doctor` con `action="syntax"` también aplica estas comprobaciones.
Al crear o editar archivos `.cs`, y al revisarlos con ese diagnóstico, busca los
`.csproj` del ancestro más cercano y avisa si el archivo no está incluido.
Reconoce inclusiones explícitas y los valores predeterminados del SDK .NET;
condiciones, imports personalizados y propiedades heredadas se marcan como
desconocidos cuando requieren evaluar MSBuild.
Harness permite editar y crear `.agents/knowledge/**/*.md`; las demás rutas de
`.agents` siguen protegidas.
La misma respuesta incluye hash, postcheck y resumen; no ejecuta compilaciones.
En fallos posteriores a crear carpetas, informa cuáles quedaron sin borrarlas a ciegas.
`harness_bootstrap` con `compact=true` reduce el catálogo inicial sin ocultar reglas
del proyecto ni avisos de seguridad; el catálogo completo queda recuperable en detalle.
`harness_bootstrap` no usa `action`; para consultar el entorno utiliza
`harness_doctor` con `action="env"`. Los campos desconocidos siguen rechazándose
para no ocultar errores tipográficos.

En `harness_search`, omitir `kind` selecciona búsqueda de texto; `query` sigue
siendo necesaria. `scope` y `path` pueden coexistir si identifican el mismo scope;
si difieren se rechazan. `globs` también filtra archivos en búsquedas de texto,
sin ampliar permisos ni desactivar las exclusiones de seguridad.
`harness_batch` valida los hijos antes de iniciarlos y, ante argumentos inválidos,
devuelve diagnósticos por hijo con índice, herramienta y campo. No inicia trabajos
válidos parcialmente ni convierte un fallo de validación en éxito.

`harness_replace` acepta un `hash` SHA-256 opcional: sin él exige coincidencias
únicas de `oldText` y comprueba cambios concurrentes antes de publicar.
`preview=true` prepara y valida el candidato y devuelve un diff sin publicarlo.
Los errores `STALE` y `MATCH` incluyen `currentHash` de los bytes inspeccionados
y sugerencias acotadas con contexto y líneas; `truncated` indica límites del
diagnóstico. El hash no reserva el archivo: revisa el contexto antes de reintentar.
Estas sugerencias no aplican cambios ni amplían las reglas de coincidencia existentes.

`harness_symbols` con `action="outline"` admite `filterKind="method"` y
`view="compact"`, juntos o por separado. La vista compacta devuelve nombre,
firma, atributos declarados, contenedor, línea e `symbolId`, sin cuerpos ni docs.
Los atributos se extraen de C#; no se calculan rutas MVC efectivas. El filtro se
aplica antes del presupuesto: `filtered` cuenta exclusiones y `omitted` cuenta
elementos elegibles que no caben y se pueden recuperar en detalle.

`harness_write` y `harness_replace` admiten `bom: true` para UTF-8 con BOM y `bom: false`
para omitirlo. Por defecto conservan el BOM existente y los `.cshtml` en Windows
se guardan con BOM. Los hashes y las comprobaciones posteriores incluyen sus bytes.
La navegación local de Razor reconoce `@model`, `@section` y `@helper`.
La indexación de `.cshtml` incorpora esas declaraciones y extrae con el kernel C#
los campos, propiedades, métodos y referencias de `@functions`/`@code` en una sola
proyección por vista, conservando líneas y columnas UTF-16. Publica relaciones
`contains`, resuelve llamadas entre miembros de la misma vista y referencias
`@model` a tipos C# públicos no ambiguos. La búsqueda admite `razor_model`,
`razor_section` y `razor_helper`, también al reparar archivos desactualizados.
No compila Razor ni resuelve expresiones del markup, tipos genéricos de `@model`
o dependencias dinámicas entre vistas; conserva referencias sin resolver.
Los índices existentes necesitan sincronizar o reindexar las vistas para incluirlas.

Para proyectos Razor MVC legacy, las vistas enlazan además sus destinos:
`Url.Action`, `Html.ActionLink`, `Html.Action` y `Html.RenderAction` resuelven
la acción del controlador; `Html.Partial` y `Html.RenderPartial` resuelven el
archivo de vista parcial; `Layout = "..."` resuelve el layout. Estas llamadas
también se detectan dentro de `@{ ... }`, `@(...)`, `@code` y `@functions`.
Los nombres literales se resuelven por convención `Views/{controlador}` y
`Views/Shared`, y una ambigüedad nunca se resuelve por conjetura. El
subconjunto y sus límites están en [razor-legacy-mvc.md](docs/razor-legacy-mvc.md).

Las consultas SQL Server incluyen el número del error y, para errores habituales
de columna, tabla o procedimiento, un mensaje con el identificador sanitizado.
Los mensajes arbitrarios del motor, datos de conversión y credenciales no se exponen.
En SQL Server, `describe` resuelve nombres sin esquema entre objetos visibles del
tipo solicitado. Una coincidencia devuelve `resolvedName` delimitado, `schema`
e `objectId`; varias devuelven `SQL_OBJECT_AMBIGUOUS` con hasta cinco candidatos.
Un nombre con esquema nunca busca alternativas fuera de ese esquema. Admite
identificadores entre corchetes y escapes `]]`, pero no nombres de tres o cuatro
partes. Sin coincidencias informa objeto no encontrado o no visible, sin adivinar
permisos. Las consultas con error 208 reciben una sugerencia de calificar el nombre;
no se reescribe ni reintenta el SQL automáticamente.
Para rutinas, `describe` obtiene `definition` mediante el `objectId` resuelto y
expone parámetros visibles de `sys.parameters`, incluidos parámetros de salida
y el retorno cuando existe una fila con `parameterId=0`. `parametersComplete`
indica si el listado quedó acotado. Una definición nula no demuestra por sí sola
falta de permisos; se informa la limitación sin ejecutar la rutina ni ampliar
`VIEW DEFINITION`. Los metadatos no sustituyen el análisis de valores por defecto
de procedimientos T-SQL ni garantizan un contrato de resultados.

Para Rust, `harness_verify` con `{"language":"rust","package":"domain"}`
ejecuta `cargo check -p domain`. `scope` permite elegir una carpeta de crate o
un `Cargo.toml`; `action="build"` y `action="test"` siguen disponibles.
El proceso tiene un plazo de 60 s y hasta 5 s adicionales para limpieza.

`harness_logs` incluye contexto acotado en memoria: rutas, comando de verificación,
últimas líneas de stdout/stderr en fallos/timeouts y tiempos de búsqueda por carpeta.
Redacta líneas potencialmente sensibles de forma heurística; no detecta todos los
secretos. El historial persistido conserva solo metadatos. Usa `session` en las
llamadas que lo admiten; `sessionId` de bootstrap es un alias de compatibilidad.

## Mover archivos

`harness_move` recibe `from` y `to` relativos al proyecto. Mueve archivos sin
sobrescribir destinos ni modificar imports y actualiza el alcance nativo del
índice existente. [Uso y límites](docs/move-file.md).

## Tareas en segundo plano

Pide al asistente que use `harness_jobs` para lanzar compilaciones o pruebas en segundo plano. Incluye dos ejecuciones simultáneas, avisos MCP, historial persistente y cancelación, sin plazo de ejecución predeterminado. El cliente decide si permite seguir conversando y reaccionar a los avisos. [Uso y límites](docs/JOBS.md).

## Límites y desarrollo

Versión experimental `0.7.1-rc.1`. Puede trabajar con carpetas de distintos proyectos, pero el soporte de símbolos, grafo y verificadores depende del lenguaje; no ofrece paridad completa con CodeGraph Node. Los verificadores requieren las herramientas del lenguaje instaladas.

```sh
cargo test --locked --workspace -- --test-threads=1
agent-harness --help
```

[Referencia completa](docs/REFERENCE.md) · [Contexto, selectores CSS y edición directa](docs/SEARCH-CSS-EDITING.md) · [Aliases, búsquedas filtradas y diff del workspace](docs/TOOL-COMPATIBILITY.md) · [Estado y límites](docs/MIGRATION-STATUS.md) · [Salida a producción](docs/PRODUCTION.md) · [Distribución Windows](release/README.md)
