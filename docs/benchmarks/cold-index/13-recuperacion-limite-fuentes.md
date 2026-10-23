# Presupuesto de fuentes y continuación ante fallos (2026-10-23)

## Reproducción aislada

El `SYNC_LIMIT` de `useCreateComposer.ts` se reprodujo en una copia privada de
Noctalia. Se copiaron sus fuentes y se hizo un backup SQLite consistente abriendo
el original en modo read-only. La copia tenía 225 archivos, 3709 nodos, 3699
aristas y 4352 referencias pendientes. No se reconstruyó el índice original.

El ejecutable anterior (`release-max`, SHA-256
`59cac28a6ff688c2933adfe9c54d581445f9ca4dc84f36d62fd6b6b85eb29d38`)
falló en `resolve_references`: 65 observaciones nuevas, 882 hits y 257030 bytes
verificados. El tope de 64 archivos se comprobaba antes de consultar si el
siguiente candidato estaba indexado. La transacción no se confirmó.

Dos regresiones fallaron antes de corregirlo: admitir 128 fuentes pequeñas y
descartar candidatos no indexados con el presupuesto lleno.

## Corrección

- Se sustituye el tope fijo de 64 fuentes por un presupuesto retenido de 64 MiB:
  capacidad del buffer, longitud de la clave y estimación conservadora de
  metadatos. Se mantiene el máximo de entradas de `MAX_ROWS` (20000).
- Las fuentes vacías también consumen presupuesto. La lectura temporal conserva
  el límite de archivo de `read_source`; no se retiene un buffer que no cabe.
- Primero se consulta la pertenencia al índice: un candidato no indexado no
  consume reservas ni provoca una lectura de fuente. No hay caché negativa.
- Se mantienen hash inicial, comparación exacta de todos los bytes antes del
  commit, cancelación y plazos, incluidos los hits de caché.
- Los errores distinguen presupuesto de entradas y de memoria retenida.

El archivo de la copia se publicó con la corrección: `committed=true`, 65 fuentes
y 260888 bytes revalidados. Duró 600 ms con el binario de desarrollo optimizado.
**No es un A/B de rendimiento**: son perfiles distintos y el caso anterior falló.

## Fallos parciales

Workspace y lotes de eventos continúan tras errores aislados con rollback
confirmado, incluidos límites locales. Los puertos informan si falló una
precondición global; la aplicación no interpreta diagnósticos internos de SQLite.

Esquema/índice inválido, bloqueo del índice, integridad, rutas inseguras,
cancelación, pérdida de scope y commit/rollback incierto siguen deteniendo el
proceso. No se amplían plazos ni se eliminan comprobaciones de seguridad.

Los reportes conservan hasta 32 detalles, `failedFiles` exacto y
`failureReportsOmitted`, además de archivos no intentados y parada anticipada.
Se cuentan los commits incluso si ya no caben en la lista de detalles. Con
fallos el resultado sigue siendo parcial: `selectedScopeApplied=false` y salida
de error del CLI. Un grafo parcial no se anuncia como globalmente fresco.

## Verificación

- Aplicación: 74 pruebas aprobadas; continuación, cancelación, scope, rollback y
  reportes acotados sin perder commits.
- Unidad nativa: 109 aprobadas y 2 ignoradas (oráculo externo/probe manual), sin
  fallos. Incluye 32 pruebas de imports, presupuestos y revalidación.
- Script reproducible: `scripts/repro-source-budget.cjs`; artefactos privados en
  `target/repro-noctalia-source-budget-20261023/`.
- La copia completó 416 archivos pendientes sin fallos. El ejecutable máximo
  también publicó una actualización forzada con 156 fuentes revalidadas y pasó
  la comprobación posterior de todo el alcance seleccionado.
- Build `release-max` corregido e instalado: SHA-256
  `bea366cda34f7a3332680db9063c7f0e8b7e00cfa7d064229e7c392d54a9b7ea`.
- Noctalia original: se reanudó desde 225 archivos, sin borrar el índice. La
  pasada confirmó 416 archivos adicionales en 135151 ms, con cero fallos,
  cero archivos restantes y `selectedScopeApplied=true`. La base terminó con
  641 archivos. No es una indexación en frío desde cero ni un A/B de velocidad.
- Durante esa pasada se mantuvo el lease exclusivo del watcher automático;
  existía un backup SQLite previo en `target/noctalia-before-source-budget-fix.db`.
  El informe completo está en `target/noctalia-live-source-budget-fix.json`.

No se garantiza que cualquier proyecto pueda indexarse sin alcanzar un límite
de seguridad; se preserva el trabajo confirmado y se informa lo que falte.
