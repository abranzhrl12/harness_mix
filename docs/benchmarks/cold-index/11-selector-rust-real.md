# Paso 11: selector de pendientes especializado para Rust

## Alcance

Se implementa y verifica una variante únicamente para publicaciones Rust sin
cambio de configuración. JavaScript, TypeScript, Vue, HTML y el resto conservan
la consulta actual. Rust con cambio de configuración también conserva la
consulta general para no perder reintentos de framework.

Los tests sintéticos del paso 10 justifican probar esta separación, pero sus
porcentajes no se presentan como mejoras del indexado global.

## Diseño y protocolo

El prefijo SQL común se comparte con la consulta general, sin manipular textos
SQL en tiempo de ejecución. La cola especializada conserva el LIKE de `super::`,
su comportamiento de mayúsculas/minúsculas, el orden, la deduplicación y el
sentinel global de 20.001 filas. No hay cambio de esquema ni migración.

La comparación real se hace con el mismo ejecutable y un interruptor diagnóstico
`HARNESS_INDEX_PENDING_RUST=0` (referencia) o `1` (candidata). Se conserva el
interruptor legacy existente. El runner registra ambas configuraciones y
comprueba que el binario no cambie entre pasadas.

Corpus congelado completo: 549 fuentes copiadas, con los candidatos elegibles
determinados por el coordinador. Tres pasadas alternadas por variante e índice
temporal vacío cada vez; cachés del sistema operativo no vaciadas. No se toca
el índice original ni se comparan los tiempos con otra muestra de 200 archivos.

## Verificación previa

- `cargo check` mediante Harness: correcto.
- 7 pruebas SQL: matriz de equivalencia, NULL/JSON, orden, sentinel y selección
  del camino Rust, incluyendo prioridad legacy y desactivación de la candidata.
- 1 prueba de reintentos, 2 de invalidación/rollback y 2 del ciclo de vida Rust.
- Total: 12 pruebas seleccionadas correctas; no se ejecutó la suite completa.
- Sintaxis de los runners Node: correcta.

El profiler añade `pending_sql`, `pending_merge`, `pending_sql_rows` y
`pending_sql_vm_steps` para separar trabajo SQL de la mezcla en el conjunto de
IDs. Son mediciones anidadas; no deben sumarse a la fase que las contiene.

## Resultados reales

El release compiló correctamente y está congelado en
`target/cold-index-rust-pending-measured.exe`. El A/B se ejecuta con ese mismo
binario, el selector desactivado/activado y el corpus completo congelado.

| Tiempo | General (Rust desactivado) | Especializada (Rust activado) |
| --- | ---: | ---: |
| Pasada 1 | 70,196 s | 63,192 s |
| Pasada 2 | 84,839 s | 67,162 s |
| Pasada 3 | 59,440 s | 57,114 s |
| Mediana | 70,196 s | 63,192 s |

La mediana baja **7,003 segundos, un 9,98 % menos tiempo** (factor 1,111).
Hay dispersión importante: no se garantiza ese porcentaje en otros equipos o
corpus. Las tres parejas favorecen la candidata, pero no uniformemente.

- `pending_sql`, medianas: 12,279 s → 6,576 s (**46,44 % menos tiempo**).
- `pending_merge`, medianas: 20,796 ms → 19,741 ms.
- Pasos VM: 437.039.364 → 122.085.366 (**72,07 % menos pasos**).
- Filas seleccionadas: 235.460 → 235.460, en 537 publicaciones.

La fase estaba dominada por el SQL, no por insertar IDs en el conjunto. No se
eliminan reintentos ni referencias para conseguir la diferencia.

Se mantienen en las seis pasadas: 537 publicaciones, 6.435 nodos, 283.976
intentos de resolución, 1.227 referencias resueltas, cero fallos, pendientes o
archivos sin comprobar. Los 549 archivos copiados no se truncaron; el
coordinador determina sus 537 candidatos elegibles. El grafo es idéntico:
`202c2e102ba97ae2470d172fece3dba4e79c8f9c8e5e1af1c531a58f71488bbb`.

Todos los contadores salvo los pasos VM coinciden: mismos bytes leídos y
revalidados, mismo parsing Rust, aciertos de caché, filas seleccionadas y commits.
No se obtiene la diferencia omitiendo revalidaciones o referencias. Las medianas
de operaciones anidadas no se suman a sus fases contenedoras.

## Reproducir y límites

```powershell
cargo build --locked --release --example index_phase_probe
node scripts/benchmark-index-resolver.cjs target/cold-index-rust-pending-measured.exe target/cold-index-rust-pending-measured.exe target/cold-index-corpus-v1 target/NUEVA-carpeta-rust-pending all 3 rust-pending
```

El artefacto medido es una copia congelada del probe compilado. SHA-256 idéntico
en ambas variantes y comprobado antes/después de cada pasada:
`46d62e0ec575382be80f8aed8dce98bf6e13ed3a8c0db789c5c8dcc448ac8dd2`.
El runner registra explícitamente `HARNESS_INDEX_PENDING_RUST=0` para before y
`1` para after, manteniendo el resto de opciones iguales.

La especialización queda activada por defecto exclusivamente para Rust con
configuración sin cambios. `HARNESS_INDEX_PENDING_RUST=0` permite volver al
selector general para diagnóstico. Los demás lenguajes y el modo legacy
mantienen su selección anterior; no hubo migración ni cambio de durabilidad.

No se midieron CPU, RSS ni tráfico físico de disco. Los índices temporales son
nuevos, pero las cachés del SO no se vacían; no es una prueba de disco físicamente
frío. El corpus congelado no equivale a todos los archivos actuales del proyecto.
No se hizo commit/push ni se modificó el índice original.

Evidencia durable: [resultados-selector-rust-all.json](resultados-selector-rust-all.json)
y [log-selector-rust-tests.txt](log-selector-rust-tests.txt). Datos completos de
ejecución en `target/cold-index-rust-pending-all/`.
