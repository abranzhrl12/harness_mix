# Paso 04 — Selector puro y medición interna del resolvedor

## Objetivo

Separar una regla pequeña del resolvedor para probarla sin SQLite, archivos,
parsers ni cachés; medir subfases antes de elegir una optimización del algoritmo.

## Alcance

El selector genérico pasa al dominio (resolution_policy). Recibe vistas prestadas
de los candidatos y un contexto de selección, y devuelve el índice del candidato
único. El adaptador conserva consultas, pruebas de lenguaje/imports, configuración,
snapshots, metadata, inserción de aristas y eliminación de referencias pendientes.

Se mantiene el orden de reglas: presupuesto de 64 candidatos antes de filtrar,
tipos permitidos, ámbito Razor/import/local, visibilidad, pruebas exactas,
nombres cualificados, prioridad del contenedor y preferencia local. Un resultado
ambiguo sigue pendiente. No se declara desacoplado todo el resolvedor.

## Instrumentación

Los temporizadores resolver_* son opt-in con HARNESS_INDEX_PROFILE=1 y acumulan
nanosegundos antes de convertir a microsegundos. No sustituyen la etapa padre
resolve_references; no sumar los subtiempos otra vez al total de publicación.

- resolver_reference_lookup: cargar la referencia por ID.
- resolver_special: selección de rama, imports y pruebas específicas de lenguaje;
  incluye publicación interna en las ramas especializadas que retornan pronto.
- resolver_candidate_lookup: cargar candidatos del camino genérico.
- resolver_policy: aplicar las reglas puras.
- resolver_edge_write: INSERT/DELETE del resultado genérico.

La fase especial sigue siendo amplia. Si domina, el próximo paso deberá medir
sus motores por separado; esta instrumentación no mide todavía pasos VM SQL,
cache hits, asignaciones ni consultas individuales.

## Diseño del A/B

Se congeló el corpus antes de editar código en target/cold-index-corpus-v1/,
con un manifiesto de rutas y SHA-256; se conservó el ejecutable anterior en
target/cold-index-before-resolver.exe. Evita que nuevas funciones/pruebas del
refactor cambien el contenido que el propio benchmark está indexando.

```powershell
cargo build --locked --release --example index_phase_probe
node scripts/benchmark-index-resolver.cjs target/cold-index-before-resolver.exe target/release/examples/index_phase_probe.exe target/cold-index-corpus-v1 target/cold-index-resolver-ab-200 200 3
```

Tres primeras pasadas por binario, orden alternado, cuatro trabajadores y escritor
único, --batch --fresh --cold-only. El ejecutor rechaza diferencias en el manifiesto,
las fuentes seleccionadas, grafo, intentos/resultados, nodos, fallos o pendientes.
El índice original no se modifica. Las cachés del SO no se vacían.

El binario nuevo contiene a la vez refactor e instrumentación adicional: cualquier
diferencia de tiempo no demuestra por sí sola la velocidad del selector puro.
La copia congelada/manifiesto y ambos ejecutables deben conservarse para repetir
exactamente esta medición; el repositorio vivo seguirá cambiando.

## Verificación

Harness ejecutó las pruebas de harness-domain: 53 unitarias aprobadas y las
suites de integración/doc del crate aprobadas. El oráculo test-only conserva la
política anterior y la compara con el selector nuevo en 64 subconjuntos de seis
candidatos y seis contextos, además de tamaños 0..65 y escenarios específicos.
No se presenta como prueba exhaustiva de todos los programas y lenguajes.

Harness también completó cargo check -p agent-harness (18,22 s). El ejemplo
release nuevo compiló con --locked en 2 min 49 s. La revisión independiente
detectó una ruta incorrecta a sync_diagnostic, que fue corregida antes del check
y del build release. No se ejecutó toda la suite de pruebas del crate principal.

## Resultados A/B

Timestamp: 2026-10-23T01:40:32.450Z; Windows 10.0.19045 x64, Ryzen 5 3400G.

| Binario | Ensayo 1 | Ensayo 2 | Ensayo 3 | Mediana |
| --- | ---: | ---: | ---: | ---: |
| Anterior | 13,488 s | 13,755 s | 14,124 s | 13,755 s |
| Selector puro + instrumentación | 16,424 s | 14,150 s | 14,083 s | 14,150 s |

La mediana nueva fue un 2,88 % mayor; no hay evidencia de una mejora de velocidad.
No se atribuye la diferencia solo al selector: el binario también añade profiling,
hubo variación del entorno y algunas fases no modificadas también cambiaron.
Esto no demuestra ausencia de regresión ni mide el coste sin profiling.

Ambas variantes y todos los ensayos conservaron: 200 archivos publicados,
40 928 intentos, 309 referencias resueltas y 2462 nodos escritos. Cero fallos,
archivos pendientes, fuentes no comprobadas o saltos. Huellas:

- Fuentes: `d08b32b3329fe107565cf93c0a5a926189f987778bc250925f89589f59208bd8`.
- Grafo: `74996aa068bb2442716c50b17fdd9fc68d3cf22a3e2e81e55c84f8443da84575`.
- Manifiesto congelado: `d95cc4d6e2723bf0524e77c4438fd3fbf8d57765305eb3063be053fbbdcc22ab`.
- Binario anterior: `7cc5d4f4ffe0555b0cb2957a14657fe1b435281ed4635212ebbf4a0975066e41`.
- Binario nuevo: `a0824445c562922c446951563b5175d8ba83a9e4295043a6f8f64cd349c250eb`.

Esta muestra congelada difiere de la del paso 01: se añadieron scripts entre
ambos snapshots. Por eso cambia la selección por ruta y sus huellas/contadores.
El A/B de este paso usa el mismo snapshot para ambos binarios; no comparar sus
segundos con los del paso 01 para atribuir diferencias al resolvedor.

## Desglose nuevo: medianas acumuladas por pasada

| Fase | Tiempo |
| --- | ---: |
| resolve_references (padre) | 6,700 s |
| resolver_special | 6,366 s |
| resolver_reference_lookup | 249,380 ms |
| resolver_candidate_lookup | 43,831 ms |
| resolver_edge_write | 16,132 ms |
| resolver_policy | 0,652 ms |

Son medianas independientes; no se suman como un total medido. Las subfases están
incluidas en el tiempo padre y resolver_special sigue incluyendo trabajo de
ramas específicas. Aproximadamente el 95 % del tiempo de resolución se concentra
en esa fase amplia. La selección pura no es el cuello de botella de la muestra.

Resumen durable: [resultados-resolver-200.json](resultados-resolver-200.json).
Raw y diagnósticos locales: target/cold-index-resolver-ab-200/ (excluido de Git).
No hubo builds ni suites lanzadas por este trabajo durante las pasadas A/B.

## Decisión y siguiente paso

Conservar la separación por testabilidad, sin presentarla como optimización de
rendimiento. No cachear candidatos ni agrupar su SQL por intuición: su tiempo
medido es pequeño frente a resolver_special.

El siguiente experimento debe separar ahí el coste de pruebas Rust, imports y
pruebas de otros lenguajes; dentro de la ruta dominante, distinguir lectura/hash,
parseo, acceso al grafo y hits/misses de snapshots. Solo después elegir una
optimización pequeña y repetir equivalencia de grafo y pruebas de invalidación.
