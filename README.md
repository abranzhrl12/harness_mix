# Agent Harness

Servidor MCP nativo en Rust para investigar, leer, editar y verificar proyectos. La distribución preparada es Windows x64. El ejecutable directo no necesita Node; la instalación npm utiliza un lanzador Node.

## Instalación rápida

~~~powershell
npm install -g @abranzhrl1212/agent-harness
agent-harness --version
~~~

Configura el cliente MCP para iniciar el ejecutable con `--root` apuntando a tu proyecto. Reinicia la conexión y comienza con `harness_bootstrap`; reutiliza `data.session` en las llamadas posteriores.

[Instalación y configuración completas](docs/INSTALLATION.md), incluidas compilación desde código fuente y configuración sin npm.

## Qué ofrece

- Búsqueda de texto/archivos, navegación de símbolos y relaciones de un índice existente.
- Creación, reemplazo con hash, preview y movimiento de archivos.
- Verificación por lenguaje y trabajos en segundo plano.
- Git de solo lectura, sesiones, configuración redactada y diagnóstico.
- Catálogos SQL Server/PostgreSQL; lectura de XLSX, SVG y proyecciones JSON.

El perfil `full` anuncia **22 herramientas MCP**, algunas con varias acciones. Los nombres y parámetros vigentes están en la [referencia de comandos](docs/REFERENCE.md). Consulta también el catálogo real del cliente después de actualizar: el contrato puede variar entre versiones.

## Índice opcional

Leer archivos y buscar texto no requieren índice. Para crear uno en un proyecto nuevo:

~~~powershell
agent-harness --root "C:/proyectos/mi-app" --index init
agent-harness --root "C:/proyectos/mi-app" --index rebuild
~~~

Estas operaciones escriben en `.codegraph/`. `init` exige que esa carpeta no exista; `rebuild` utiliza un índice compatible y puede dejar commits por archivo si se interrumpe. No borra las fuentes ni certifica cobertura global.

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
