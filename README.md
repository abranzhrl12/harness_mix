# Agent Harness

Para orientarte en este repositorio, consulta la sección relevante del [mapa del proyecto](PROJECT_MAP.md). Usa la [skill project-map](docs/project-map/skill/SKILL.md) cuando necesites ampliar o actualizar el mapa; no hace falta cargarla completa para cada tarea.

Servidor MCP nativo en Rust para investigar, leer, editar y verificar proyectos. La distribución preparada es Windows x64. El ejecutable directo no necesita Node; la instalación npm utiliza un lanzador Node.

## Instalación rápida

~~~powershell
npm install -g @abranzhrl1212/agent-harness
agent-harness --version
~~~

Configura el cliente MCP para iniciar el ejecutable con `--root` apuntando a tu proyecto. Reinicia la conexión y comienza con `harness_bootstrap`; reutiliza `data.session` en las llamadas posteriores.

[Instalación y configuración completas](docs/INSTALLATION.md), incluidas compilación desde código fuente y configuración sin npm.

Para frontend, backend y servicios en una sola conexión, ejecuta `agent-harness workspace init` en su carpeta común y registra cada uno con `agent-harness workspace add <alias> <ruta>`. El arranque sin argumentos descubre `harness-workspace.json` desde la carpeta actual. Cada proyecto conserva su propio índice. Consulta la [guía de workspaces](docs/WORKSPACE.md) para configurar el directorio de arranque del cliente o pasar un manifiesto explícito.

## Qué ofrece

- Búsqueda de texto/archivos, navegación de símbolos y relaciones de un índice existente.
- Creación, reemplazo con hash, preview y movimiento de archivos.
- Verificación por lenguaje y trabajos en segundo plano.
- Git de solo lectura, sesiones, configuración redactada y diagnóstico.
- Catálogos SQL Server/PostgreSQL; lectura de XLSX, SVG y proyecciones JSON.

Go dispone de navegación de símbolos, validación sintáctica de ediciones y un grafo de declaraciones e imports, además de compilación y tests. Consulta la [cobertura y los límites de Go](docs/go-native-support.md); el grafo todavía no resuelve llamadas ni paquetes importados.

El perfil `full` anuncia **22 herramientas MCP**, algunas con varias acciones. Los nombres y parámetros vigentes están en la [referencia de comandos](docs/REFERENCE.md). Consulta también el catálogo real del cliente después de actualizar: el contrato puede variar entre versiones.

## Índice opcional

Leer archivos y buscar texto no requieren índice. Para crear uno en un proyecto nuevo:

~~~powershell
agent-harness --root "C:/proyectos/mi-app" --index init
agent-harness --root "C:/proyectos/mi-app" --index rebuild
~~~

Estas operaciones escriben en `.codegraph/`. `init` exige que esa carpeta no exista; `rebuild` utiliza un índice compatible y puede dejar commits por archivo si se interrumpe. No borra las fuentes ni certifica cobertura global.

`sync` y `rebuild` preparan las fuentes en paralelo (hasta cuatro trabajadores según los procesadores disponibles) y publican en SQLite con un único escritor. Para comparar con la ejecución secuencial o limitar el consumo:

~~~powershell
$env:HARNESS_INDEX_WORKERS = "4" # De 1 a 4; 1 desactiva la preparación paralela
agent-harness --root "C:/proyectos/mi-app" --index rebuild
~~~

El informe incluye `extractionWorkers`, `databaseWriters`, `indexingDurationMs` y `failures`. La lista `failures` conserva todos los fallos de publicación aunque `files` solo muestre 32 resultados. Cada fallo aporta archivo, código, mensaje y diagnóstico de etapa; los errores del extractor conservan la causa original y los de sintaxis Rust incluyen ubicaciones cuando están disponibles. Los errores SQLite de inicialización, planificación y publicación conservan su causa y códigos. Para medir la mejora, compara el mismo conjunto de fuentes e índice con 1 y 4 trabajadores, sin incluir compilación; el beneficio depende del peso de la extracción frente a las escrituras y la resolución del grafo.

`inventoryDiagnostics` explica los archivos que no pudieron leerse o comprobarse; `uncheckedFilePaths` enumera el trabajo pendiente y `skippedFileReports` conserva las limitaciones del proveedor por archivo.

En Windows, con perfil `full` e índice existente compatible, el mantenimiento automático está habilitado por defecto. `HARNESS_INDEX_WATCH=0` lo desactiva.

## Documentación

| Guía | Contenido |
| --- | --- |
| [Instalación](docs/INSTALLATION.md) | npm, ejecutable, Rust y conexión MCP |
| [Comandos](docs/REFERENCE.md) | Opciones CLI, 22 herramientas, ejemplos y límites |
| [Workspace de varios proyectos](docs/WORKSPACE.md) | Manifiesto y trabajo entre varios grafos |
| [Rendimiento](docs/PERFORMANCE.md) | Tiempos por opción, muestras y reproducción |
| [Bases de datos](docs/SQL-QUICKSTART.md) | Conexión, TLS, permisos y caché |
| [Arquitectura y desarrollo](docs/ARCHITECTURE.md) | Módulos, parsers, pruebas y distribución |
| [Contrato generado](docs/TOOL-CONTRACT.md) | Disponibilidad Rust/Node; incluye herramientas exclusivas de Node |

Los planes y reportes antiguos fueron consolidados; se pueden recuperar en el historial de Git. Las mediciones son observaciones de sus fixtures, no promesas de velocidad para cualquier proyecto.
