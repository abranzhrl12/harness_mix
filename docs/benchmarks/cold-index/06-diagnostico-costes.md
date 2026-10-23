# Paso 06 — Explicar el coste antes de optimizar

## Pregunta

¿Qué operaciones explican el tiempo total, cuántas veces se ejecutan y cuánto
cuesta una llamada? Una etapa de varios segundos no identifica por sí sola el
problema. Hay que distinguir volumen, coste individual y repetición evitable.

## Estrategia implementada

1. Conservar el corpus congelado, índices temporales vacíos y huella de grafo.
2. Medir tiempo de pared, llamadas y bytes; no inferir CPU o disco solo del reloj.
3. Desglosar revalidación: scope, configuración, dependencias y fuente principal.
4. Desglosar lecturas: validación de ruta, metadata, protección de directorios,
   apertura, lectura de bytes y metadata posterior.
5. Desglosar Rust: solicitudes de target, hits/misses de hechos, análisis completo,
   parser y recorrido/análisis del AST. Contar bytes analizados y pruebas ausentes.
6. Contar solicitudes por ruta para ver qué fuentes se vuelven a visitar.
7. Seleccionar el siguiente cambio por tiempo acumulado y repetición, y exigir
   mejora A/B real con el mismo grafo. Evitar microbenchmarks ajenos al cuello real.

## Instrumentación

Colector opt-in en src/adapters/index_profile.rs, independiente de SQLite y del
filesystem. Estado local al hilo y a cada publicación; temporizadores sin borrow
persistente, contadores estáticos, máximo 256 rutas de hasta 1000 bytes por archivo.
La aplicación agrega todas las publicaciones correctas, independientemente del
preview de 32 archivos; conserva hasta 1024 rutas e informa intentos omitidos.

profileMetrics.operations informa calls, totalMicros y meanMicros. Los contadores
incluyen bytes leídos/analizados, misses de verificación, fuentes no indexadas,
hits de hechos Rust y commits intentados. Las rutas cuentan solicitudes de
read_source, incluidas las que terminan sin archivo; no equivalen a aperturas
efectivas. Las aperturas y lecturas reales tienen sus propios contadores/timers.

Los tiempos son inclusivos y anidados: no sumar un padre con sus hijos. La media
no es p95. El perfil agrega overhead y su tiempo total no debe compararse como
una optimización contra el binario anterior con menos instrumentación.

## Cobertura y límites explícitos

- Detalle por operación de publicaciones correctas en el hilo principal.
- Preparación paralela y planificación tienen tiempos agregados, sin desglose
  interno por trabajador. Sus IO/parseos no están en los contadores detallados.
- Fallos conservan diagnóstico por archivo, pero no se agregan como publicaciones
  correctas. Una pasada con fallos tiene cobertura parcial y debe tratarse así.
- Los timers de SQL actuales incluyen recuperar referencias y candidatos, además
  del hash indexado y fases de escritura/commit. No son un trace de todas las
  sentencias SQL ni contabilizan pasos VM de cada consulta.
- No se midieron CPU samples, esperas del kernel, antivirus o latencias físicas
  del almacenamiento. Una operación de IO lenta no identifica por sí sola la causa.

## Reproducción

```powershell
cargo build --locked --release --example index_phase_probe
node scripts/benchmark-index-diagnostics.cjs target/release/examples/index_phase_probe.exe target/cold-index-corpus-v1 target/cold-index-byte-recheck-ab-200/summary.json target/cold-index-diagnostics-200 200 3
```

El ejecutor compara las tres pasadas con las fuentes, el grafo y los contadores
semánticos del paso 05; preserva raw, metadata, listas de repeticiones y resumen.
Los directorios de salida deben ser nuevos y los artefactos quedan en target/.

## Verificación

Harness cargo check del proyecto principal pasó. Pasaron 67 pruebas de aplicación,
incluidas suma de todas las publicaciones, media ponderada y límites de rutas.
Pruebas focalizadas Cargo: 2 del colector (desactivado, reset, límites y guards
antiguos), 3 de diagnósticos previos y 2 de Rust/rollback, todas aprobadas.
No se ejecutó toda la suite del workspace. No se modifica el algoritmo de
resolución ni la política de revalidación en este paso.

## Resultados: ya se puede explicar el coste

Release compilado en 2 min 25 s (fuera de la medición). Tres pasadas: 14,300 s,
16,043 s y 14,378 s; mediana 14,378 s. Todas mantuvieron exactamente el grafo,
fuentes y resultados del paso 05: 200 archivos, 40 928 intentos, 309 resoluciones,
2462 nodos y cero fallos/unchecked/skips/remaining. Los contadores siguientes
fueron idénticos en las tres pasadas. Los tiempos son medianas independientes.

### Volumen de trabajo real

| Hecho | Cantidad |
| --- | ---: |
| Publicaciones / commits | 200 |
| Rutas distintas solicitadas a read_source | 209 |
| Solicitudes a read_source | 7590 |
| Solicitudes que terminaron sin archivo | 4164 |
| Aperturas/lecturas efectivas | 3426 |
| Bytes leídos, hilo de publicación | 26 127 486 |
| Análisis de hechos Rust ejecutados | 1513 |
| Bytes analizados en esos análisis | 11 952 969 |
| Hits de hechos Rust dentro de publicaciones | 12 637 |
| Dependencias releídas en revalidación | 1513 |
| Archivos principales releídos en revalidación | 200 |

No hubo rutas omitidas por los límites del colector. Las 209 rutas incluyen
ausencias: no equivalen a 209 archivos físicos. Las solicitudes se reconcilian:
5677 durante resolución + 1713 en revalidación + 200 lecturas primarias = 7590.
De ellas, 4164 no llegaron a abrir un archivo; las otras 3426 sí.

### Tiempo y coste por llamada

| Operación | Llamadas | Acumulado | Media por llamada |
| --- | ---: | ---: | ---: |
| Target/proof Rust, padre | 14 493 | 6,317 s | 435,88 us |
| Analizar hechos Rust, incluye parser/AST | 1513 | 3,156 s | 2085,96 us |
| Parser Rust | 1513 | 1,143 s | 755,75 us |
| Recorrido/análisis del AST | 1513 | 1,132 s | 748,45 us |
| Validación inicial de rutas | 7590 | 2,284 s | 300,92 us |
| Segunda validación al leer | 3426 | 0,839 s | 244,88 us |
| Proteger directorios Windows | 3426 | 0,916 s | 267,25 us |
| Abrir archivo | 3426 | 0,106 s | 30,89 us |
| Leer/copiar bytes | 3426 | 0,151 s | 44,07 us |
| Consultar referencia por ID | 40 928 | 0,226 s | 5,52 us |
| Consultar hash indexado | 1513 | 0,087 s | 57,44 us |
| Calcular hash inicial verificado | 1513 | 0,011 s | 7,16 us |
| Comparar bytes en revalidación | 1513 | 0,0017 s | 1,12 us |

El coste individual de algunas operaciones YA está en microsegundos, pero se
multiplica por miles. Analizar un archivo Rust cuesta aquí unos 2,09 ms de media
y ocurre 1513 veces. Leer bytes solo suma unos 151 ms: la validación/protección
de rutas y los análisis repetidos tienen mucho más peso en esta muestra.

Los timers de target, observación, lectura y análisis son padres/hijos; no
sumar toda esta tabla. Tampoco atribuir la diferencia entre parse + AST y el
análisis padre a un único trabajo: incluye preparación, validaciones y teardown.

### Por qué recheck_sources ocupa aproximadamente dos segundos

| Componente | Llamadas | Mediana acumulada |
| --- | ---: | ---: |
| Scope/configuración CodeGraph | 200 | 65,9 ms |
| Configuración de resolución | 200 | 54,8 ms |
| Revalidación de dependencias | 1513 archivos, en 200 publicaciones | 1,576 s |
| Revalidación del archivo principal | 200 | 223,3 ms |

recheck_sources completo sumó 1,911 s. Sus lecturas de fuentes fueron 1713,
acumulando 1,779 s, alrededor de 1038 us por solicitud de lectura protegida.
La comparación de bytes de dependencias ocupó solo 1,7 ms. Esta distribución
explica por qué hacer esa comparación 22–31 veces más rápida no recortó segundos.
Las medianas de filas independientes no deben sumarse como un total exacto.

### Dónde se repite el trabajo

| Ruta solicitada | Solicitudes en una pasada |
| --- | ---: |
| src/adapters/lib.rs | 1127 |
| src/adapters/main.rs | 1127 |
| src/adapters/mod.rs | 1127 |
| crates/application/src/indexing/file.rs | 360 |
| crates/application/src/indexing/lib.rs | 179 |
| crates/application/src/indexing/main.rs | 179 |
| crates/application/src/indexing/mod.rs | 179 |

La ruta Rust prueba padres convencionales mod.rs/lib.rs/main.rs. Se vuelve a
intentar analizar los ausentes porque los resultados negativos no se conservan
en ese camino. También se vuelven a crear los hechos positivos en publicaciones
posteriores: el snapshot de análisis es local a cada publicación.

Importante: el helper elimina fuentes fuera de las primeras 200 de la muestra.
Parte de las ausencias son efecto de ese recorte. Este volumen concreto no
describe necesariamente el proyecto completo; debe repetirse con cobertura
completa antes de extrapolar el beneficio de optimizar módulos ausentes.

### Reconciliación del total y componente BD

Los tiempos de publicación, planificación, preparación y finalización explican
el 98,4–98,6 % del tiempo de pared de cada ensayo. Quedan 219–223 ms por ensayo
en bookkeeping, serialización de métricas y trabajo no instrumentado. El informe
JSON conserva los tres cálculos, sin sumar subfases anidadas dos veces.

Planificación: mediana 312 ms; preparación paralela: 507 ms. Los 200 commits
sumaron una mediana de 2,129 s (cociente orientativo de 10,65 ms por commit).
Seleccionar pendientes sumó 1,106 s. Las consultas de referencia/candidatos son
solo parte del trabajo SQL; no se concluye que SQLite sea gratis ni culpable
de todo. No se midieron por separado fsync, antivirus o esperas físicas del disco.

## Estrategia priorizada a partir de estos datos

1. Evitar reanalizar hechos Rust para los mismos bytes ya verificados: evaluar
   reutilización acotada por digest/identidad del parser. Mantener lectura y
   revalidación de fuentes actuales y comprobar qué hechos son neutrales a la ruta.
   El techo medido de ese componente es 3,156 s, no una promesa de ahorro.
2. Reducir pruebas de módulos ausentes dentro del snapshot inmutable de la BD.
   Un resultado negativo debe estar ligado a esa publicación y no convertirse
   en una caché global de frescura basada en metadata. Medir primero el corpus
   completo porque el recorte de la muestra produce ausencias artificiales.
3. Revisar las dos validaciones de ruta por lectura y el ciclo de protección de
   directorios. Diseñar una lectura protegida que comparta el estado validado,
   sin quitar garantías frente a symlinks/reparse points, carreras o cancelación.
4. Medir el coste de durabilidad/commit antes de proponer lotes. Agrupar commits
   cambia límites de publicación y rollback; no hacerlo como ajuste invisible.

Cambiar una sola cosa por experimento, fijar contadores esperados y exigir mismo
grafo/fallos. Rechazar una optimización si su mejora está por debajo del ruido o
si solo acelera una operación que ocupa milésimas del total. No volver a perseguir
el hash para resolver varios segundos del indexador.

## Evidencia y decisión de este paso

Se conserva la instrumentación y se documenta el diagnóstico; no se implementó
otra optimización especulativa. [Resumen durable](resultados-diagnostico-200.json).
Raw: target/cold-index-diagnostics-200/trial-{1,2,3}.json y summary.json.
Binary SHA-256: 02bd1da1693eeb82663add7ff9fd32502a1b298157a64807b259cac278d76931.
La huella de grafo es 74996aa068bb2442716c50b17fdd9fc68d3cf22a3e2e81e55c84f8443da84575.
El total instrumentado no es una medición de aceleración frente a pasos anteriores.
