# Desacoplamiento incremental de la indexación

## Límites actuales

- `crates/application/src/ports/indexing.rs`: `PublicationSession`, unidad de trabajo
  que expone operaciones de publicación y artefactos mediante tipos asociados;
  y contrato de invalidación. No importa SQLite ni tipos del adaptador.
- `crates/application/src/indexing.rs`: caso de uso `publish`, orden de etapas,
  cancelación entre etapas y política de motores por lenguaje/configuración.
- `src/adapters/index_store/publication/invalidation.rs`: implementaciones SQLite,
  consultas de pendientes, snapshots de declaraciones y conexión con el retry memo.
- `publication.rs`: fachada y composición de módulos.
- `publication/sqlite.rs`: implementa `PublicationSession`, recibe la invalidación
  por DI y encapsula transacción, snapshots, parser y caches concretos.
- `index_store.rs`: selecciona la implementación y conserva commit/rollback.

La orquestación de publicación ya vive en aplicación: comprobación de hash,
extracción, captura del grafo anterior, borrado, escritura, restauración de enlaces,
invalidación, resolución, revalidación y metadatos. Cada operación se puede sustituir
en una sesión de prueba sin SQLite ni parsers. Los artefactos se pasan explícitamente
entre etapas; aplicación no interpreta IDs internos ni snapshots del driver.

No es una migración de TODO el indexador: inventario, scheduler de preparación,
apertura de conexión y commit/rollback siguen en `index_store`. El commit se mantiene
fuera del caso de uso porque todavía requiere revalidar fuentes/configuración y
comprobar integridad. Un éxito de `publish` no certifica commit ni frescura global.
Los modelos de la especialización SQLite no son modelos de dominio.

## Batch y corrección

`HARNESS_INDEX_BATCH_INVALIDATION=1` selecciona la estrategia batch, que por ahora
delega en incremental. Omitir invalidación global sin una pasada final deja enlaces
obsoletos o pendientes sin resolver. No hay una nueva mejora de rendimiento atribuible
a este refactor. Una implementación diferida futura deberá cubrir creación, borrado,
ambigüedad, imports y frameworks, cancelación y rebuild sobre índices existentes.

## Verificación

- La política por lenguaje se prueba sin filesystem ni SQLite.
- `cargo test -p harness-application --test indexing_publication`: orden, paso de
  artefactos, fallo/cancelación en cada etapa, timeout inicial y ruta sin cambios.
- Cada etapa emite `PublicationStage`; el adaptador mantiene las etiquetas del
  profiler existente (`HARNESS_INDEX_PROFILE=1`) para comparar optimizaciones.
- Una implementación inyectada que falla comprueba el rollback de archivos, nodos,
  enlaces y referencias pendientes.
- Ambas estrategias comprueban resolución de llamadas a declaraciones publicadas
  después del consumidor y retirada de enlaces cuando desaparece el destino.

No se modifica el esquema de la base de datos ni se cambia la durabilidad.
Los perfiles de compilación presentes en `Cargo.toml` son cambios previos del usuario.

Verificación del refactor (2026-10-22): 125 unitarias de indexación aprobadas
(2 ignoradas), 93 integraciones aprobadas (4 ignoradas por herramientas de referencia
externas), 21 pruebas de arquitectura aprobadas y 14 pruebas de aplicación aprobadas
(9 unitarias + 5 del pipeline). Total: 253 aprobadas y 6 ignoradas; no es la suite
completa del workspace. `git diff --check` sin errores de whitespace.
