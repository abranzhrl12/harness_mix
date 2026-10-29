# Integración LSP multilenguaje desde Rust

Investigación iniciada: 2026-10-28; ampliada con la prueba mcpls-core. Resultado: **serena-rs y mcpls-core permiten consultas reales desde Rust contra rust-analyzer y gopls** en los fixtures probados. No se integró una nueva capacidad productiva. Se amplió a una hora el plazo por herramienta de harness_mix en Codex; requiere reconexión.

## Actualización de integración

Se incorporó el adaptador Rust `harness_lsp` con **mcpls-core 0.8.0**, disponible en crates.io. La conclusión anterior de que los tags posteriores no estaban publicados era incorrecta. Las pruebas reales TS/JS expusieron desconexiones intermitentes en 0.3.8; el lector de protocolo podía cancelarse dentro del select del cliente. 0.8 separa ese lector y contiene regresiones del framing. Se descartó 0.3.9 porque conserva el diseño anterior.

La [guía de navegación LSP](../LSP-NAVIGATION.md) documenta la integración y `HARNESS_LSP_SERVERS`. La primera suite real adaptada a 0.8 pasó 5 pruebas (0 fallos, 0 ignoradas), incluidas definiciones/referencias TS/JS entre archivos, emoji y frescura tras guardar el llamador. Rust y Go conservan evidencia histórica con 0.3.8; esa evidencia no certifica 0.8.

Fuentes de la corrección: [registro oficial](https://crates.io/api/v1/crates/mcpls-core), [lector 0.8](https://docs.rs/crate/mcpls-core/0.8.0/source/src/lsp/transport.rs) y [cliente 0.8](https://docs.rs/crate/mcpls-core/0.8.0/source/src/lsp/client.rs).

## Opciones comparadas (investigación inicial)

| Opción | Qué aporta | Licencia observada | Evaluación |
|---|---|---|---|
| Serena original | Servidor MCP externo con varios servidores LSP simultáneos | No se auditó su licencia en esta prueba | Amplia cobertura documentada; no se confirmó un SDK Rust oficial. |
| serena-rs 0.1.7 | Crate Rust con cliente LSP público y servidor MCP | MIT | Reutilización directa demostrada. Es reciente; el repositorio enlazado redirigió a login, aunque el código publicado está en docs.rs. |
| mcpls-core 0.3.8 | Puente MCP/LSP, arranque y gestión de procesos LSP, capacidades, estado y shutdown | MIT OR Apache-2.0 | Definiciones y referencias entre archivos validadas en Rust y Go: 2 pruebas correctas. El estado interno permanece Initializing tras handshake en 0.3.8; no sirve como indicador de carga completa. |
| async-lsp 0.2.4 | Framework Rust que admite cliente y servidor | MIT OR Apache-2.0 | Alternativa de menor nivel; Harness conservaría toda la gestión de proyectos y políticas. |
| lsp-types | Tipos del protocolo | Revisar versión elegida al adoptar | No es un cliente ni un gestor de servidores. |
| lsp-server 0.10 | Armazón síncrono orientado a servidores | MIT OR Apache-2.0 | No resuelve por sí solo el cliente multilenguaje. |
| helix-lsp | Cliente real del editor Helix | MPL-2.0 en master consultado | Arrastra dependencias internas de Helix; menos apropiado como biblioteca pequeña para Harness. |

Ambas bibliotecas pasaron pruebas positivas mínimas. mcpls-core ofrece una interfaz embebible con capacidades y procesos, pero su estado de cliente en 0.3.8 no debe usarse como garantía de disponibilidad semántica. No hay evidencia comparativa suficiente para declararla superior a serena-rs en precisión, madurez o rendimiento. Octocode y otros motores con grafo/MCP no se ejecutaron.

## Prueba realizada

Crate independiente: [experiments/lsp-library-probe](../../experiments/lsp-library-probe/Cargo.toml). No se modificaron Cargo.toml ni Cargo.lock del workspace principal.

Se fijó `serena-rs = "=0.1.7"` y se usó la misma interfaz `LspClient` contra servidores ya instalados:

- `rust-analyzer 1.99.0 (b940084d 2026-09-28)`.
- `gopls v0.21.1`.

Cada prueba crea un proyecto temporal mínimo, abre un documento y consulta definición y referencias de una llamada. Comprueba el archivo físico y las posiciones esperadas, no solo que exista una respuesta. La comparación de archivos usa canonicalización; no convierte indiscriminadamente rutas a minúsculas.

La prueba es de lectura: no renombra símbolos ni aplica cambios. No crea un índice Harness; cada servidor analiza únicamente su fixture temporal. Hay plazos de inicialización/navegación y cierre; la captura de panic permite intentar el cierre antes de propagar el fallo.

### Validado

```text
cargo test --manifest-path experiments/lsp-library-probe/Cargo.toml
2 passed; 0 failed; 0 ignored
```

Ejecución mediante Harness: `harness_verify`, language=rust, action=test, scope=experiments/lsp-library-probe, timeoutSeconds=900. Última ejecución: exit 0, 7.75 s de pruebas y 13.012 s totales, incluida recompilación. La compilación inicial descargó dependencias y tardó 3m28s; no comparar esos tiempos como rendimiento de consulta.

Evidencia completa: [verification.json](../../experiments/lsp-library-probe/verification.json). Harness informó `verificationFreshness=not_tracked` y observación del workspace desconocida (`beforeError/afterError=PATH`); por eso esta evidencia se limita al comando ejecutado, sus resultados y las pruebas contenidas en el experimento.

### Hallazgos durante la prueba

1. La primera consulta Rust devolvió una lista vacía durante carga. Posteriormente definition respondió pero references todavía devolvió None. La prueba positiva sondea ambas consultas con plazo hasta obtener datos del fixture conocido. **Esto no es un detector productivo de readiness ni permite concluir ausencia de referencias.**
2. rust-analyzer devolvió la letra de unidad Windows con distinta capitalización. La comparación textual de URI fallaba aunque se trataba del mismo archivo; se corrigió la comprobación del experimento a identidad física.
3. La ejecución final produjo dos warnings de notify: `Input watch path is neither a file nor a directory`. No impidieron los asserts; su origen no se investigó.
4. El cliente de serena-rs publica `supports_type_hierarchy` devolviendo true sin comprobar capacidades y `did_change` usa sincronización completa. No adoptar esos contratos como garantía universal.
5. El experimento independiente resolvió rmcp 3.8.0 transitivamente. No demuestra compatibilidad de integración con la versión fijada por Harness.

Se corrigió también un fallo intermedio de compilación del experimento: el helper de cleanup requería una dependencia explícita. El estado final compila y pasa las dos pruebas.

## Prueba mcpls-core 0.3.8 (validada)

Crate independiente: [experiments/mcpls-core-probe](../../experiments/mcpls-core-probe/Cargo.toml), con versión exacta y Cargo.lock separado. La prueba abre un documento llamador y valida definiciones y referencias entre archivos en un paquete Rust válido y un módulo Go. Comprueba archivo físico y rango de definición; normaliza respuestas Location/LocationLink y drena las notificaciones. El cierre tiene plazo y se intenta también después de panic.

Resultado final mediante `harness_verify`, scope=experiments/mcpls-core-probe, timeoutSeconds=3600: **2 passed, 0 failed, 0 ignored**, exit 0. Tiempo de pruebas: 7.13 s; total con compilación: 11.535 s. Evidencia completa: [verification-success.json](../../experiments/mcpls-core-probe/verification-success.json). El resultado inicial fallido y recuperado del log se conserva en [verification.json](../../experiments/mcpls-core-probe/verification.json).

Se corrigieron errores del fixture: manifiesto Cargo sin paquete, llamada Rust sin módulo/import válido, raíz Go demasiado amplia y posiciones incorrectas. Además, el código publicado de mcpls-core 0.3.8 completa initialize/initialized pero deja el estado del cliente en Initializing: la comprobación por estado falló en ambos servidores incluso después de 180 s. La prueba final no oculta este comportamiento: registra el estado y valida respuestas semánticas positivas con plazo. Ese sondeo no demuestra ausencia de referencias ni sustituye un indicador productivo de disponibilidad del índice.

Esta caracterización cubre un fixture por lenguaje. No demuestra soporte universal, cancelación ni integración con Harness. Cargo resolvió rmcp 2.2.0; la compatibilidad con la versión del producto sigue pendiente. No se comparó rendimiento con serena-rs ni con el grafo.

### Tiempo de conexión de desarrollo

Por solicitud del usuario se configuró `mcp_servers.harness_mix.tool_timeout_sec = 3600` en la configuración local de Codex, preservando las demás entradas y creando una copia de respaldo. El cambio requiere reconectar MCP; no se afirma que el límite de la conexión ya abierta haya cambiado. No se encontró una opción documentada para desactivar totalmente el plazo. Cargo se ejecutó con timeoutSeconds=3600 en esta prueba.

## Recomendación para Harness

Evaluar **mcpls-core detrás de un adaptador de consultas propio**, manteniendo async-lsp como opción si el puente completo introduce demasiado acoplamiento. Conservar el grafo actual para navegación general; consultar LSP para resolución semántica precisa según capacidades.

El siguiente paso productivo sería una interfaz pequeña de definición, referencias y diagnósticos. Debe distinguir encontrado, vacío con cobertura conocida, inicializando, no soportado y fallo. Evitar importar todo el catálogo MCP del proveedor y duplicar herramientas existentes.

**Pendiente:** corregir o adaptar el indicador de estado de mcpls-core, pruebas de macros y referencias entre paquetes, Unicode/posiciones, sincronización tras editar, cancelación real, políticas de alcance/sesión, comparación con nuestro grafo y medición de contexto/latencia. No se ejecutó la suite raíz porque no se cambió código productivo.

## Fuentes primarias

- [mcpls-core 0.3.8: ciclo de inicialización](https://docs.rs/crate/mcpls-core/0.3.8/source/src/lsp/lifecycle.rs) y [estado del cliente](https://docs.rs/crate/mcpls-core/0.3.8/source/src/lsp/client.rs).
- [Codex: tool_timeout_sec](https://learn.chatgpt.com/docs/config-file/config-reference).

- [Serena: soporte multilenguaje](https://oraios.github.io/serena/01-about/020_programming-languages.html).
- [Serena: clientes MCP y transportes](https://oraios.github.io/serena/02-usage/030_clients.html).
- [serena-rs: paquete, historial y licencia](https://docs.rs/crate/serena-rs/latest).
- [serena-rs: interfaz LspClient](https://docs.rs/serena-rs/latest/serena_rs/lsp/struct.LspClient.html) y [fuente del cliente](https://docs.rs/serena-rs/latest/src/serena_rs/lsp/client.rs.html).
- [mcpls-core 0.3.8: crate y arquitectura](https://docs.rs/mcpls-core/0.3.8/mcpls_core/) y [API LspServer](https://docs.rs/mcpls-core/0.3.8/mcpls_core/lsp/struct.LspServer.html), [API ServerState](https://docs.rs/mcpls-core/0.3.8/mcpls_core/lsp/enum.ServerState.html). La afirmación inicial sobre versiones posteriores se corrigió arriba: 0.8.0 sí está publicado y es la versión integrada.
- [mcpls: historial de capacidades/readiness](https://github.com/bug-ops/mcpls/blob/main/CHANGELOG.md) y [manifiesto](https://github.com/bug-ops/mcpls/blob/main/Cargo.toml).
- [async-lsp: roles cliente/servidor y licencia](https://docs.rs/crate/async-lsp/0.2.4).
- [lsp-types: alcance de la biblioteca](https://docs.rs/lsp-types/latest/lsp_types/).
- [lsp-server: alcance](https://docs.rs/crate/lsp-server/0.10.0) y [manifiesto/licencia](https://raw.githubusercontent.com/rust-lang/rust-analyzer/master/lib/lsp-server/Cargo.toml).
- [Helix: dependencias del cliente](https://github.com/helix-editor/helix/blob/master/helix-lsp/Cargo.toml) y [licencia](https://raw.githubusercontent.com/helix-editor/helix/master/LICENSE).
