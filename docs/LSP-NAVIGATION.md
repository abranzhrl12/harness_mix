# Navegación LSP

La herramienta Rust `harness_lsp` complementa el grafo con definiciones y referencias del servidor de lenguaje instalado. Está disponible en el perfil `full`, porque arranca procesos del operador. El índice Harness existente se conserva.

```json
{"action":"definition","path":"src/app.ts","line":12,"character":8,"session":"<data.session>"}
{"action":"references","path":"src/app.ts","line":12,"character":8,"includeDeclaration":true,"session":"<data.session>"}
```

Las posiciones de entrada son base 0 y columnas UTF-16. La respuesta declara `positionEncoding` para sus rangos y convierte la posición de entrada al encoding negociado. Las rutas de workspace admiten `alias:/src/app.ts`. Los resultados están limitados a 100 ubicaciones dentro del proyecto; `truncated` y `omittedOutsideWorkspace` informan omisiones. Una respuesta vacía conserva `absenceProven:false`: el handshake LSP no certifica que terminó el análisis.

## Servidores

Por defecto se buscan `rust-analyzer`, `gopls` y `typescript-language-server --stdio` en el entorno del proceso. TypeScript/JavaScript necesitan Node, typescript-language-server y una instalación TypeScript que incluya tsserver. El fixture real usa TypeScript 5.9.3.

El operador puede definir `HARNESS_LSP_SERVERS` como JSON antes de iniciar Harness. Este mapa reemplaza los valores predeterminados; una clave omitida deshabilita ese lenguaje. Se admiten `rust`, `go`, `typescript` y `javascript`.

```json
{
  "rust": {"command":"rust-analyzer","args":[]},
  "go": {"command":"gopls","args":[]},
  "typescript": {
    "command":"C:/Program Files/nodejs/node.exe",
    "args":["C:/tools/lsp/node_modules/typescript-language-server/lib/cli.mjs","--stdio"]
  },
  "javascript": {
    "command":"C:/Program Files/nodejs/node.exe",
    "args":["C:/tools/lsp/node_modules/typescript-language-server/lib/cli.mjs","--stdio"]
  }
}
```

Los comandos provienen del entorno del operador, fuera de los argumentos de la herramienta y de la configuración del repositorio. Se leen una vez por workspace. Cambiar ese entorno requiere reiniciar/reconectar el servidor MCP.

Cada workspace mantiene un worker y servidores bajo demanda, con cola de cuatro consultas. Cada documento admite hasta 2 MiB y solo queda abierto el archivo consultado; los anteriores se cierran para que los cambios guardados en disco no queden ocultos por una copia antigua. Las consultas repetidas sincronizan el documento completo. Los errores descartan la sesión afectada para que una consulta posterior arranque un servidor nuevo.

## Versión y verificación

Se fija `mcpls-core = 0.8.0`. Las pruebas reales con 0.3.8 revelaron desconexiones intermitentes: su recepción del protocolo podía cancelarse al enviar comandos. La versión 0.8 separa el lector y documenta la corrección del framing. Véase la [fuente publicada](https://docs.rs/crate/mcpls-core/0.8.0/source/src/lsp/transport.rs).

Las pruebas TS/JS de `crates/lsp/src/tests.rs` comprueban definición y referencias entre archivos, columnas con emoji, cambios del documento y desaparición de referencias tras guardar otro archivo. Para ejecutarlas se requiere `HARNESS_TEST_TYPESCRIPT_LSP` con la ruta del CLI oficial:

```powershell
$env:HARNESS_TEST_TYPESCRIPT_LSP = 'C:/tools/lsp/node_modules/typescript-language-server/lib/cli.mjs'
cargo test --locked -p harness-lsp -- --include-ignored
```

La incorporación está en el código fuente. Para verla en un cliente se necesita ejecutar un binario reconstruido y reconectar MCP; esta tarea no reemplaza el ejecutable instalado ni modifica la configuración del usuario.
