# CHECK — estado y correcciones del harness

Fecha: 2026-10-18. Windows x86-64, binario de desarrollo.
Base auditada: `eabb7f9`; las correcciones siguientes están en el working tree.

## Resultado actual

**386 pruebas aprobadas, 0 fallidas y 18 opcionales pendientes.**

| Ejecución final | Aprobadas | Fallidas | Omitidas |
| --- | ---: | ---: | ---: |
| Workspace completo, código de salida 0 | 381 | 0 | 23 |
| Cuatro verificadores reales opcionales, ejecutados aparte | 4 | 0 | 0 |
| Aceptación con copia temporal de 76 fuentes, ejecutada aparte | 1 | 0 | 0 |

Los cinco opcionales ejecutados aparte están entre los 23 omitidos de la primera
fila, por eso quedan 18 sin ejecutar. Las sondas MCP adicionales también pasan.
La auditoría inicial tenía 370 aprobadas y 3 fallidas; las tres fallidas ahora
pasan y hay 13 casos nuevos en la suite. Esto no certifica servicios externos
sin configurar, todas las entradas posibles ni todas las plataformas.

## Correcciones de la auditoría

| Hallazgo | Corrección implementada | Prueba de regresión |
| --- | --- | --- |
| H1: `runId` y listas devolvían respuestas fijas | `process_status`, `jobs` y doctor comparten el mismo gestor; `runId = taskId`, historial recuperado incluido. Sin gestor adjunto se devuelve unsupported, no una lista ficticia | Módulo `tasks`, `tests/process_status.rs`, `tests/tasks_stdio.rs` |
| H2: doctor confundía entorno con funcionamiento completo | Compara las rutas MCP reales con el perfil; consulta el almacén existente; `connection: true` añade consulta del índice y ping SQL cuando están disponibles. Declara `allToolsVerified: false` y límites | `tests/doctor.rs`, `tests/stdio.rs`, `tests/tasks_stdio.rs` |
| H3: selector inválido del fixture era válido según el contrato actual | Fixture usa `fresh()`, que sí es inválido, y conserva la aserción de cero escrituras. Una prueba adicional cubre varios identificadores válidos | `tests/index_scope.rs` |
| H4: acceso denegado a PID parecía proceso inexistente | Se diferencia `unknown/access_denied` y fallo de consulta de ausencia confirmada | Prueba de clasificación Windows en módulo `process_status` y tests PID existentes |
| H5: dos fixtures asumían llamadas entre archivos sin prueba de módulo | Usan padre explícito y `super::a::target()`, dentro del subconjunto soportado. Se comprueba la arista ANTES de borrar sus metadatos; se mantiene la protección legacy | `tests/index_store.rs` |
| H6: TypeScript válido fallaba con Node 22.16 | Build de TS/TSX usa Oxc nativo, sin Node, ejecución ni comprobación de tipos. Los tests TS/TSX se rechazan explícitamente | Módulos `verify`/`tasks`, prueba MCP sin Node en `tests/stdio.rs` |
| H7: HTTPS podía dar positivo usando HTTP plano | HTTPS se rechaza ANTES de conectar. HTTP valida URLs, controles, autoridad y puerto; se corrige IPv6 | Módulo `process_status` y listener sin conexiones en `tests/process_status.rs` |

No se eliminan las aserciones de seguridad para lograr verde. El resolver Rust
no se amplió para adivinar llamadas por nombre. HTTPS no se anuncia como una
conexión TLS implementada, y el build TypeScript no se presenta como typecheck.

## Checklist

- [x] Corregir las implementaciones y declaraciones afectadas por H1–H7.
- [x] Conservar las comprobaciones de validación previa, commits parciales y protección de borrado.
- [x] Añadir pruebas de sintaxis TS y TSX válida/inválida sin ejecutar el archivo.
- [x] Añadir prueba MCP de seguimiento compartido antes y después de reiniciar.
- [x] Añadir pruebas de doctor sin creación de archivos y de consulta real del índice.
- [x] Publicar las capacidades y límites en el contrato y en `docs/JOBS.md`.
- [x] Completar la ejecución final de toda la suite y anotar recuentos: 381/0/23.
- [x] Repetir los cuatro verificadores reales opcionales: 4/4.
- [x] Repetir aceptación sobre copia temporal del código real: aprobada.
- [x] Repetir sonda MCP de H1/H2/H6/H7: todas sus aserciones aprobadas.
- [ ] Verificar SQL Server con credenciales y servidor reales.
- [ ] Certificar integración con la interfaz real del usuario.
- [ ] Certificar paridad con el proveedor CodeGraph original.
- [ ] Certificar MSRV 1.88, release, otras plataformas y objetivos de rendimiento.

## Matriz de las 30 herramientas

OK significa los casos automatizados y el alcance indicado; no garantiza todas
las entradas posibles, plataformas o clientes. La ejecución final terminó sin fallos.

| Herramienta | Cobertura y evidencia |
| --- | --- |
| `harness_batch` | Orden, fallos parciales, límites y rechazos: `tests/batch.rs` |
| `harness_bootstrap` | Descubrimiento, sesión y contratos: stdio/llm_contract |
| `harness_checkpoint` | Persistencia y vigencia histórica: workflow |
| `harness_config` | XML/JSON, redacción, límites: config/json_config |
| `harness_data_query` | Proyecciones, Unicode, referencias y redacción: data_query/llm_contract |
| `harness_detail` | Paginación UTF-16, límites y TTL: dominio results/stdio |
| `harness_diagnostics` | Sintaxis Rust/JS/TS/JSON: diagnostics |
| `harness_doctor` | Entorno, registro MCP, almacén e índice; NO certifica todas las funciones |
| `harness_explore` | Fixtures, sincronización y aceptación real; alcance nativo acotado, sin paridad total Node |
| `harness_external_scope` | Autorización, identidad, revocación: external_scopes |
| `harness_git_diff` | Worktrees, staging, binarios, filtros: git |
| `harness_git_status` | Repositorios y metadatos de worktrees: git |
| `harness_handover` | Vista previa y exportación: handover |
| `harness_jobs` | Concurrencia, cancelación, reintentos, persistencia, reinicio y TS nativo: tasks/tasks_stdio |
| `harness_logs` | Historial y límites: logs/disk_logs/metrics |
| `harness_outline` | Rust y JS/TS: symbols/ts_symbols; no todos los lenguajes |
| `harness_process_status` | PID/TCP/HTTP y registro compartido; HTTPS rechazado explícitamente |
| `harness_read` | Lectura real por MCP, hashes, límites y rutas: project/stdio |
| `harness_read_symbol` | Cuerpos, IDs, límites: symbols/ts_symbols |
| `harness_replace` | Publicación, hashes, permisos y concurrencia: edit; publicación específica de Windows |
| `harness_search` | Texto, archivos, selectores y tamaño: search/syntax_search/file_size/stdio |
| `harness_sql_describe` | Conectada al adaptador; NO VERIFICADA contra un servidor real |
| `harness_sql_query` | Conectada al adaptador; NO VERIFICADA contra un servidor real |
| `harness_sql_search` | Conectada al adaptador; NO VERIFICADA contra un servidor real |
| `harness_sql_status` | Ausencia de configuración comprobada; ping real pendiente de conexión |
| `harness_svg` | Inspección, preview y rechazos: svg |
| `harness_tasks` | Tablero de sesión y persistencia: workflow; distinto de jobs |
| `harness_verify` | Rust/.NET/Go/JS/Python y sintaxis nativa TS/TSX; sin typecheck ni runner de tests TS |
| `harness_xlsx_inspect` | Libro de fixture: xlsx |
| `harness_xlsx_read` | Lectura y batch: xlsx |

## Reproducción

```powershell
cargo test --locked --workspace --no-fail-fast --jobs 4 -- --test-threads=1
cargo test --locked -p agent-harness --lib 'adapters::verify::tests::real_' -- --ignored --nocapture --test-threads=1
cargo test --locked --test real_core_validation real_core_copy_lifecycle_without_node -- --ignored --nocapture --test-threads=1
```

Evidencia local, excluida de Git por estar bajo `target/`:

- `target/audit/workspace-tests-complete.log`: auditoría inicial; 365/3/23.
- `target/audit/runtime-tests.log`: cuatro verificadores originales, aprobados.
- `target/audit/real-core-acceptance.log`: aceptación original de 76 fuentes.
- `target/audit/final-fixed-workspace-tests.log`: suite final corregida.
- `target/audit/fixed-runtime-tests.log`: los cuatro verificadores repetidos tras las correcciones.
- `target/audit/fixed-real-core-acceptance.log`: aceptación repetida tras las correcciones.
- `target/audit/fixed-mcp-probe.json`: sonda final con aserciones de H1/H2/H6/H7 y registro 30/30.
- `target/audit/mcp-probe.json` y `probe.cjs`: sonda JSON-RPC sin SDK externo.

## Límites y pendientes externos

En el host hay Cargo/Rust 1.99.0, Git 2.46.0.windows.1, Node 22.16.0,
Python 3.12.4, Go 1.27.0 y SDK .NET 9.0.300.
SQL no tiene configuración en este checkout ni cadena de conexión disponible
para la auditoría. No se inventaron credenciales ni se consultaron otros proyectos.

Permanecen fuera de verificación completa los siguientes 18 casos opcionales
después de ejecutar los cuatro runtimes y la aceptación temporal:

| Motivo | Casos |
| --- | --- |
| Proveedor CodeGraph/Node, Rolldown y/o release ausentes (8) | relative_resolution_matches_codegraph_in_supported_priority_cases; extraction_matches_the_installed_codegraph_provider; import_mapping_and_unambiguous_paths_match_provider_subset; named_reexport_targets_match_full_provider_indexing; star_binding_matrix_matches_native_esm_and_rolldown_generated_bundles; alias_targets_match_installed_provider_for_supported_cases; real_core_graph_parity_review; real_core_release_vs_provider_cli |
| Binario histórico ausente (1) | frozen_native_position_ab |
| Índice/proyecto externo no proporcionado (7) | compare_context_on_existing_index; real_codegraph_index_can_be_read_without_running_node; real_project_summary_omissions_are_not_graph_loss; existing_native_reindex_has_proven_call_and_no_confirmed_false_pairs; real_project_command_timings; real_project_context_presentation; real_project_payload_and_log_audit |
| Diagnósticos de fase/rendimiento (2) | rust_resolution_phase_probe; real_core_extraction_cost (requiere release) |

`tests/node-compat.test.cjs` necesita el checkout Node original y su SDK.
Los proyectos independientes `experiments/` no son parte de la suite del runtime.
Browser, regression y web_check son del backend Node, no de este ejecutable.
No se certifican Linux/macOS, MSRV ni todos los proveedores/clientes.

## Rendimiento

La auditoría inicial midió 30 llamadas calientes por operación: p95 de 1,051 ms
para leer, 2,568 ms para buscar texto y 0,376 ms para consultar un trabajo,
sobre fixtures pequeños. La aceptación inicial de 76 fuentes tardó 43,18 s.
Estas cifras corresponden a la versión anterior a las correcciones; no se usan
para afirmar rendimiento de la versión corregida.

Después de corregir, la sonda pequeña midió p95 de 1,900 ms para leer,
5,712 ms para buscar texto y 1,014 ms para consultar un trabajo. La aceptación
de 76 fuentes tardó 30,21 s (sync inicial 10.025 ms, reindex 18.709 ms).
Son ejecuciones en condiciones distintas, no una comparación A/B ni evidencia
de mejora o regresión de rendimiento.

No existe todavía un presupuesto de CPU/RAM/latencia ni una comparación A/B
que permita declarar todo el harness óptimo.
