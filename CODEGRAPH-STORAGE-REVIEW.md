# CodeGraph: revisión del almacenamiento

Referencia inspeccionada: distribución oficial `@colbymchenry/codegraph` 1.6.2,
su biblioteca Windows y el repositorio público
<https://github.com/colbymchenry/codegraph> (MIT).
Se leyó la implementación, no solo el README o propuestas en issues.

## Hallazgos verificados

- `src/db/queries.ts`, `QueryBuilder.runBatched`: INSERT de varias filas,
  descomposición ordenada en tamaños 128/32/8/1 y sentencias reutilizadas por
  tipo/tamaño. Distribución compilada: `dist/db/queries.js`, líneas 194–220.
- `insertNodes`, `storeFileBundle`, `insertUnresolvedRefsBatch`: usan ese
  mecanismo para nodos, aristas y referencias. `storeFileBundle` agrupa las
  tablas y el registro del archivo en una transacción por archivo; no es una
  transacción global por todo el repositorio.
- `src/extraction/store-worker.ts`: en el camino de índice nuevo, un escritor
  dedicado mantiene su conexión y recibe archivos en orden; el hilo principal
  no accede a la base mientras ese escritor está activo.
- `src/db/index.ts`, `configureConnection`: WAL, synchronous=NORMAL,
  cache de páginas de 64 MiB, tablas temporales en memoria y mmap.
  El escritor tiene además un fast-init opcional MEMORY/OFF. Estas opciones
  tienen compromisos de durabilidad; no se trasladan automáticamente.

## Adaptación nativa acotada

`src/adapters/index_batches.rs` adapta la descomposición 128/32/8/1 para las
inserciones de nodos, aristas (incluidas conservadas) y referencias extraídas.
Las referencias recuperan sus IDs con RETURNING id; el resolver consume el
conjunto de IDs, sin asumir el orden de las filas devueltas. Mantiene el
orden de entrada y las reglas actuales de INSERT/INSERT OR IGNORE, los triggers
FTS, foreign keys, cancelación y rollback de la transacción por archivo.
El aviso MIT del código original se conserva en
`vendor/codegraph-native/LICENSE`.

`src/adapters/index_writer.rs` mantiene una conexión durante cada operación de
sync/reindex. Conserva los handles de protección de base y directorios hasta
cerrar SQLite, y vuelve a comprobar metadatos y hardlinks por archivo. Los
sidecars permanecen protegidos durante las transacciones: tras commit/rollback
definitivo se liberan antes de cerrar para permitir la limpieza WAL/SHM en
Windows sin reintentos de unlink. Si aún hay una transacción activa, se conservan
todos los handles durante el cierre y rollback final.
Cada archivo sigue teniendo su propia transacción, validación de esquema,
comprobación de fuentes/configuración y rollback. El progress handler se rearma
por archivo después de desactivarlo para commit/rollback. Las respuestas por
archivo incluyen `writerConnectionReused`; un sync sin trabajo no abre escritor.

Se conservan synchronous=FULL y todas las comprobaciones de seguridad. La
resolución sigue siendo por archivo. La recuperación excepcional de referencias
entrantes de símbolos eliminados/renombrados conserva su inserción individual.
No se declara una mejora global de rendimiento antes de medirla.
