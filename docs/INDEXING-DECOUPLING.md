# Indexación desacoplada por etapas

## Arquitectura

Los casos de uso viven en `crates/application/src/indexing/` y los puertos
en `crates/application/src/ports/indexing.rs`. No importan SQLite, filesystem,
parsers nativos ni tipos de adaptadores.

Los modelos compartidos `Observation`, `Plan` e `Inventory` viven en
`crates/domain/src/indexing.rs`. Aplicación no añade dependencias nuevas.

| Caso de uso | Responsabilidad | Puerto |
| --- | --- | --- |
| `inventory.rs` | Selección, presupuestos, observaciones y diagnósticos | `InventoryPort` |
| `planning.rs` | Snapshot, inventario, clasificación, exclusiones y preview | `PlanningPort` |
| `workspace.rs` | Rebuild, ondas acotadas, errores y commits parciales | `WorkspacePort` |
| `changed_paths.rs` | Watcher, fallback a inventario y commits parciales | `ChangedPathsPort` |
| `file.rs` | Validación, scope, captura de fuente y apertura transaccional | `FileIndexPort` |
| `transaction.rs` | Validar, publicar, revalidar, integridad, commit/rollback | `FileTransactionPort` |
| `../indexing.rs` | Diez etapas de publicación e invalidación | `PublicationSession` |

Publicación separa hash, extracción, captura, borrado, escritura, restauración,
invalidación, resolución, revalidación de frameworks y metadatos.

`src/adapters/index_store.rs` conserva entradas públicas y composición. Los módulos
`workspace`, `changed_paths`, `file`, `transaction` y `publication/sqlite`
implementan los puertos nativos. `index_inventory.rs` conserva recorrido físico,
seguridad de rutas, metadata y lecturas protegidas. `index_sync.rs` conserva apertura
read-only, validación de esquema y lecturas SQL; planificación usa un único snapshot.

Se desacoplan los ciclos de vida del indexador nativo, no se elimina infraestructura:
parsers, consultas de resolución, persistencia, pins Windows, inicialización del esquema
y monitor de eventos siguen siendo implementaciones concretas. No se afirma que cada
helper interno se haya convertido en un puerto. Los reportes JSON públicos se mantienen
por compatibilidad. Un éxito de publicación aislada no confirma commit ni frescura global.

## Garantías

- Transacciones por archivo, no por workspace.
- Antes del commit se revalidan scope, fuentes, configuración e integridad.
- Error/cancelación provoca rollback, incluso con contexto cancelado.
- Durabilidad incierta nunca se comunica como rollback confirmado.
- Cancelación posterior al commit confirmado no oculta la escritura.
- Errores posteriores conservan la contabilidad de commits anteriores.
- Inventario incompleto/ilegible/limitado no demuestra ausencia.
- Preparación opcional posicional, máximo cuatro extracciones y un writer.
  Un lote de preparación incompleto se descarta sin desplazar otros archivos.
- No se modifica esquema, durabilidad ni archivos fuente del proyecto indexado.

## Pruebas

Sin SQLite ni archivos reales:

```powershell
cargo test -p harness-application --lib --test indexing_inventory --test indexing_workspace --test indexing_transaction --test indexing_publication
```

Cubren inventario, límites, clasificación, coordinación de planificación,
rebuild, lotes, errores locales/sistémicos, watcher/fallback, reportes acotados,
preflight y fallo/cancelación en cada etapa transaccional y de publicación.

Con infraestructura real:

```powershell
cargo test -p agent-harness --lib index_
cargo test -p agent-harness --test architecture
cargo test -p agent-harness --test index_store --test index_incremental --test index_sync_robustness --test reexports --test star_reexports --test tsconfig_paths --test angular_di_resolution --test angular_providers_resolution --test angular_router --test vue_composables --test vue_returned_methods --test framework_index_store
```

Las reglas de arquitectura recorren todos los módulos de aplicación de indexación
y verifican que la fachada delegue los ciclos de vida. Las integraciones verifican
SQL, parsers, rutas protegidas, rollback real, referencias, providers y frameworks.

Verificación seleccionada (2026-10-22): 49 pruebas de aplicación, 125 unitarias
de indexación, 22 de arquitectura y 93 integraciones aprobadas. Total: 289 aprobadas,
0 fallidas y 6 ignoradas (oráculos externos y un probe manual).
No es la suite completa del workspace. Formato y `git diff --check` sin errores.

## Rendimiento

Los puertos permiten sustituir y medir una etapa sin cambiar las demás.
`HARNESS_INDEX_PROFILE=1` conserva etiquetas por etapa.
`HARNESS_INDEX_WORKERS` sigue limitado a 1–4.

No hay aceleración medida atribuible a este refactor.
`HARNESS_INDEX_BATCH_INVALIDATION=1` todavía delega en incremental: no puede omitir
reintentos globales sin una pasada final correcta para creación, borrado, ambigüedad,
imports, frameworks y cancelación.

Los cambios previos del usuario en el `Cargo.toml` raíz se preservan.
