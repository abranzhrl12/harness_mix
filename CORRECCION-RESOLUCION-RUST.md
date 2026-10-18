# Corrección acotada de resolución Rust

La resolución de `calls` y `function_ref` Rust deja de aceptar una coincidencia
global por nombre corto. Reutiliza `syn` y el lector de fuentes indexadas con
comprobación de hash y revalidación antes del commit.

## Alcance

- Las llamadas de método no se convierten en llamadas a funciones homónimas.
- Parámetros, patrones, variables y clausuras impiden una asociación por nombre.
- Se permiten llamadas directas a funciones libres declaradas en el mismo archivo
  cuando no hay imports, macros o bindings que hagan incierto el nombre.
- Se comprueba el subconjunto `super::modulo::funcion`: módulo padre declarado,
  cadena de `mod.rs` hasta un `lib.rs`/`main.rs` indexado, destino único indexado,
  declaración de función y visibilidad compatible entre hermanos.
- Las relaciones de módulos se invalidan y prueban de nuevo ante cambios Rust;
  un cambio de `#[path]` o un segundo archivo de módulo no conserva un vínculo
  anteriormente válido.

## Límites deliberados

No es resolución de tipos ni equivalencia completa con rustc o CodeGraph Node.
Los métodos, imports Rust, reexports, módulos inline, rutas generales `crate::`,
atributos condicionales salvo el módulo `cfg(windows)` en Windows, referencias a
funciones sin llamada y casos fuera del presupuesto quedan pendientes.
La detección de shadowing sobrebloquea entre scopes: prefiere perder una relación
antes que inventarla. Los parsers y caches conservan límites explícitos.

Los índices anteriores pueden contener relaciones falsas creadas por el resolver
anterior. Para eliminarlas todas se necesita `--reindex` con el binario nuevo;
un sync sin cambios no reconstruye relaciones históricas. El índice antiguo de
la raíz `E:\harnermix` se conserva sin cambios: el rebuild nativo se detuvo en
`SYNC_UNSUPPORTED_RELATION`, con cero archivos publicados. Se creó y reindexó un
índice nativo independiente en `.agents/harness/native/core/.codegraph`.

## Validación

Pruebas dirigidas: `cargo test --locked --lib rust_free_calls -- --nocapture`.
Incluyen métodos y clausuras, parámetros/patrones/imports, BOM/CRLF/UTF-16 y
publicación SQLite con recuperación e invalidación de la llamada entre hermanos.
Resultado: cinco pruebas aprobadas, cero fallos, ejecución de 1,34 s. Incluye
una prueba con copias de `lib.rs`, `adapters/mod.rs`, `explore.rs` e
`index_store.rs` reales: se demuestra `synchronized -> sync_workspace_all`.
La compilación inicial de dependencias con el perfil optimizado existente tardó
10 min 35 s; no se ejecutó la suite completa.

## Reindexado real y auditoría posterior

- Binario: `target/debug/agent-harness.exe`, perfil dev optimizado existente;
  no es una medida release ni comparable con el benchmark anterior de 71 archivos.
- `--reindex` del repositorio core: 116/116 archivos publicados, cero pendientes,
  cero fallos, 19 188 ms. Perfil nativo acotado, no cobertura global certificada.
- Plan posterior: 116 archivos sin cambios; cero añadidos, modificados, borrados,
  pendientes de extracción o archivos sin comprobar.
- Test incremental de desplazamiento/renombrado/eliminación: aprobado, 1,22 s.
- Auditoría opt-in `tests/native_reindex_audit.rs`: aprobada, 0,26 s. Lee el índice
  core mediante la API nativa `Request::Explore`, sin SQL directo ni clientes MCP.
- Confirmado en el índice real: una llamada `synchronized -> sync_workspace_all`;
  cero pares falsos `process.rs -> rust_symbols::collect`,
  `index_store.rs -> Application::execute` y `index_store.rs -> Operation::run`.
- Para `run`, el límite de selección de homónimos marca el grafo global truncado.
  La prueba exige que el destino exacto esté seleccionado, que todos los nodos
  seleccionados hayan sido expandidos y que no exista ninguna arista/vecino.
  No utiliza el recorte por sí solo como prueba de ausencia.

Repetir únicamente la auditoría: establecer `HARNESS_AUDIT_EXISTING_INDEX=1` y
ejecutar `cargo test --locked -p agent-harness --test native_reindex_audit -- --ignored --nocapture`.
El archivo nuevo de auditoría se incorpora después con `--sync-file`; no cambia
las relaciones de producción comprobadas. Aún no se afirma paridad completa con
CodeGraph Node ni se ha cambiado el backend MCP activo.

Cierre tras incorporar el test: 117 archivos sin cambios, cero cambios de
inventario o extracciones pendientes; auditoría repetida y aprobada en 0,36 s.
