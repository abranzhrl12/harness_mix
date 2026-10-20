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
`harness_replace` acepta un `hash` SHA-256 opcional: sin él exige coincidencias
únicas de `oldText` y comprueba cambios concurrentes antes de publicar.
Ambas herramientas admiten `bom: true` para UTF-8 con BOM y `bom: false`
para omitirlo. Por defecto conservan el BOM existente y los `.cshtml` en Windows
se guardan con BOM. Los hashes y las comprobaciones posteriores incluyen sus bytes.
La navegación local de Razor reconoce `@model`, `@section` y `@helper`;
las relaciones del grafo siguen dependiendo de la cobertura del índice.

Las consultas SQL Server incluyen el número del error y, para errores habituales
de columna, tabla o procedimiento, un mensaje con el identificador sanitizado.
Los mensajes arbitrarios del motor, datos de conversión y credenciales no se exponen.

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
