# `harness-mcp`

Crate portable que adapta `harness-application` y `harness-domain` al protocolo MCP. Contiene el servidor, el perfil de herramientas, el límite de entrada, la presentación de respuestas y el ciclo de ejecución/cancelación de operaciones. La composición nativa del proceso permanece en `src/transport/runtime.rs`.

## Frontera

Este crate depende de `application`, `domain`, `rmcp`, Tokio y `serde_json`. No construye adaptadores nativos ni lee configuración del proceso. La capa de composición nativa resuelve la configuración del host y entrega opciones explícitas al servidor.

`McpServer::new(application, jobs)` recibe la aplicación y el servicio opcional de trabajos. Sus constructores encadenables `with_eager_tools(bool)` y `with_context_presentation(bool)` controlan, respectivamente, los metadatos de carga anticipada y el formato de texto contextual. Los wrappers nativos conservan la compatibilidad con `HARNESS_MCP_EAGER_TOOLS` y `HARNESS_MCP_PRESENTATION` y pasan el resultado a esos builders; el crate no consulta esas variables.

La API también publica `BoundedLines` y `MAX_MESSAGE_BYTES` para que el runtime limite cada línea JSON-RPC antes de que el SDK la acumule en memoria.

## Módulos

| Módulo | Responsabilidad |
| --- | --- |
| `server` | Rutas MCP, capacidades, llamadas a herramientas y `ServerHandler` |
| `execution` | Admisión, cancelación, límites de tiempo y propiedad de workers |
| `input` | Lectura asíncrona con máximo de bytes por mensaje |
| `presentation` | Conversión y limpieza de sobres de dominio a respuestas MCP |
| `profile` | Filtrado de herramientas y reducción del esquema batch |
| `jobs` | Traducción de servicio de trabajos y eventos a resultados/notificaciones MCP |

## Verificación

Las pruebas de integración en `tests/architecture.rs` comprueban la API pública del servidor desde fuera del crate. Las pruebas de cancelación viven como pruebas unitarias porque cubren detalles privados del dispatcher y de `execute`.
