# Benchmark opt-in de extracción y publicación framework

`tests/framework_benchmark.rs` contiene dos pruebas ignoradas que imprimen
muestras JSON para Vue y Angular. No es un target Criterion ni una medición de
arranque en frío del proceso. El fixture se crea en un directorio temporal con
una base CodeGraph aislada; no se abre ni modifica el índice del proyecto.

## Línea base del 2026-10-21

Se ejecutaron las dos suites en release: dos procesos por suite y tres
iteraciones por tamaño/operación, con todas las comprobaciones aprobadas.
Son 540 tiempos, agrupados en 90 casos con seis muestras cada uno.
[Muestras agrupadas, rangos, hardware y picos de proceso](benchmarks/framework-2026-10-21.json).
Equipo: Ryzen 5 3400G, 8 procesadores lógicos, aproximadamente 12 GB de RAM,
Windows 10 y Rust 1.99.0. No se ejecutaron builds concurrentes durante las
muestras finales; la compilación release quedó fuera del intervalo medido.

Medianas en milisegundos para las funciones nuevas:

| Caso | Tamaño (N) | Extracción | Alta del target + retries | Reextracción forzada |
| --- | --- | ---: | ---: | ---: |
| Métodos Vue | small (1) | 1.45 | 41.81 | 46.98 |
| Métodos Vue | medium (16) | 24.45 | 46.53 | 70.70 |
| Métodos Vue | large (64) | 1078.07 | 131.71 | 1162.94 |
| Providers Angular | small (1) | 0.44 | 40.83 | 41.05 |
| Providers Angular | medium (16) | 17.77 | 51.94 | 79.98 |
| Providers Angular | large (64) | 976.14 | 196.33 | 1204.83 |

N significa métodos Vue o registros Angular; cada registro Angular aporta dos
relaciones. Los casos large tienen aproximadamente 160 KB de fuente, no 64
archivos ni un proyecto grande. Las suites anteriores también muestran coste de
extracción elevado en large (Vue 985.88 ms y Angular 1280.47 ms); son fixtures
distintos, no una comparación A/B del coste de las funciones nuevas.

Picos de working set observados por proceso: suite base 12.73 / 12.64 MiB;
suite de funciones nuevas 26.24 / 14.41 MiB. Son medidas del ejecutable de test
completo, no memoria por operación ni del CLI con mimalloc.

La revisión medida corresponde al candidato sin commit sobre `5b82d5f`, con
huella de fuentes `7b7dbead0c3d2c3ef4b36257c6ca214fc47f9a7da83c6fe037449af65001add9`.
El JSON conserva la revisión, perfil y huella emitidos por cada proceso.

El primer candidato de optimización es perfilar la ruta de extracción en fuentes
con muchas líneas: parsing nativo JS/TS, proyección y enriquecimientos. La medición
no identifica por sí sola la función responsable. No se ha aplicado una
optimización basándose únicamente en esta correlación ni se promete latencia
de proyectos reales.

## Ejecución

En Windows, con el wrapper que construye y ejecuta el test binario, muestreando
el working set mientras el proceso vive:

```powershell
.\scripts\benchmark-framework.ps1 -Profile release -Iterations 3 -Rounds 3
```

La prueba también se puede ejecutar directamente:

```powershell
$env:HARNESS_FRAMEWORK_BENCH_ITERATIONS = '3'
cargo test --locked -p agent-harness --test framework_benchmark framework_extract_and_incremental_publication_benchmark -- --ignored --exact --nocapture
```

El segundo caso mide los motores nuevos para métodos retornados por composables
y registros estáticos Angular:

```powershell
.\scripts\benchmark-framework.ps1 -Profile release -Iterations 3 -Rounds 3 -Filter framework_returned_methods_and_providers_benchmark
```

Comando directo equivalente:

```powershell
$env:HARNESS_FRAMEWORK_BENCH_ITERATIONS = '3'
cargo test --locked -p agent-harness --test framework_benchmark framework_returned_methods_and_providers_benchmark -- --ignored --exact --nocapture
```

El test profile configura `agent-harness` con `opt-level = 0`, por lo que el
comando Cargo sirve para inspeccionar el comportamiento y los asserts, pero no
para una comparación de velocidad release. El wrapper debe medir el binario ya
construido, excluyendo compilación; consulte el propio script para el perfil y
la salida de memoria que reporta.

El registro `framework-process-sample` incluye revisión Git, estado dirty,
versión de Rust, perfil y una huella SHA-256 de las fuentes relevantes (también
las nuevas sin commit), el fixture y el wrapper. Así se puede identificar una
medición candidata antes de publicarla. La huella se calcula fuera del intervalo
medido. El máximo de working set pertenece al proceso completo, no a cada fase
ni a memoria privada; si no se logra observar una lectura válida se informa null.

`HARNESS_FRAMEWORK_BENCH_ITERATIONS` acepta de 1 a 20 y por defecto usa 3. El
wrapper también recibe `-Iterations` y `-Rounds`; las rondas organizan el
muestreo del working set a nivel de proceso y no generan mediciones por fase.
Cada extracción o publicación recibe un `OperationContext` nuevo, así el
deadline predeterminado es por operación y no acumulativo para toda la suite.
Cada repetición produce líneas JSON independientes. `elapsedMs` mide solo el cuerpo
de la operación indicado por `operation`; creación del directorio temporal,
base de datos, fuentes y schema quedan fuera de esos intervalos.

## Qué cubre

Las tres escalas tienen 1, 16 y 64 referencias. Además, los importers medium y
large llevan 512 y 4096 líneas deterministas de comentarios para variar el
tamaño de fuente sin añadir más bindings. La prueba registra bytes de importer
y target para que la etiqueta de tamaño no sustituya al tamaño observable.

Para cada framework y escala se miden estas operaciones:

| `operation` | Alcance del intervalo |
| --- | --- |
| `extract_only` | Extracción de la fuente en memoria; no abre SQLite ni resuelve imports. |
| `new_importer_publication_target_missing` | Primer sync del importer mientras falta `bridge.ts`; extrae, registra refs pendientes y publica la transacción. |
| `target_publication_incremental_retry` | Sync de `bridge.ts` después de crear el archivo fuera del timer; extrae el target y reintenta las refs pendientes del importer. |
| `forced_reextract_new_publication_caches` | `sync_file_forced` del importer sin cambiar su contenido; fuerza nueva extracción/publicación y verifica los bindings ya disponibles. |
| `unchanged_importer_noop` | Sync ordinario del mismo importer; mide la ruta sin cambios y verifica que no vuelva a extraer ni invoque el resolver. |
| `target_edit_and_recheck` | Edición determinista del target escrita antes del timer; mide publicación e invalidación/revisión de sus referencias. |
| `target_deletion_invalidation` | Borrado del target antes del timer; mide publicación del borrado y comprueba que las referencias vuelvan a pendientes. |
| `target_readd_and_recovery` | Reescritura del target antes del timer; mide su publicación y la recuperación de aristas. |

Cada fase comprueba cantidades de referencias o aristas antes de emitir su
muestra. El target Vue exporta funciones composable; el Angular exporta clases
inyectadas desde parámetros de constructor. Ambos usan import directo al módulo
`bridge.ts`; el fixture no incorpora un barrel de reexports.

El caso forced reextract no reutiliza las cachés internas de `Sources`: se crea
una instancia por publicación. El nombre de la operación hace explícito ese
límite. Puede beneficiarse de la caché del sistema operativo, que no se vacía.
`extract_only` repite extracción en el mismo proceso, con allocator y parser
nuevos por llamada; esos samples tampoco son procesos fríos.
Después de la llegada del target, el benchmark guarda una huella de aristas
(ruta, nombre, kind, posición, referencia original, token form y contributors)
y la compara tras la reextracción forzada. También verifica que esa huella se
recupere después de borrar y volver a publicar el target editado.

El segundo caso emite `probe=framework-feature-subsets-v1` para dos motores que
no mide la primera prueba:

| `feature` | Fixture y aristas esperadas cuando llega el target |
| --- | --- |
| `vue_returned_methods` | Importa `useClockN`, invoca `clockN.resetN()` y el target exporta una función composable que devuelve su función local `resetN`; se esperan N aristas `vue-composable-return-method-subset-v1`. |
| `angular_static_providers` | `@NgModule` registra `{ provide: TokenN, useClass: ImplN }` con imports nombrados; se esperan 2N referencias `angular-static-provider-registration-subset-v1` (token e implementación). |

Este caso mide extracción, publicación inicial con target ausente, llegada,
reextracción forzada, edición, borrado y recuperación. Repite las mismas escalas
1/16/64. Los dos fixtures usan import directo a `bridge.ts`; no incluyen barrel,
resolución de provider en runtime ni selección del injector Angular. Los asserts
comparan huellas de edges para comprobar que forced reextract y re-add mantienen
el mismo grafo semántico.

## Límites de interpretación

- Las muestras de publicación incluyen extracción, lecturas/hashes, consultas
  SQLite, resolución, escritura y commit de una transacción por archivo. No
  aíslan el tiempo del resolver ni el costo puro de SQLite.
- El target arrival mide la publicación del target y sus retries incrementales;
  no mide únicamente la resolución de referencias.
- Las pruebas solo miden duración de pared con `Instant`; no calculan memoria.
  El wrapper puede reportar `PeakWorkingSet64` muestreado mientras el proceso
  vive. Ese máximo es una cota inferior si el sampler no observa un pico breve.
  Una lectura posterior al cierre que da cero no representa el pico.
- Las escalas son fixtures sintéticos y pequeños, no muestras representativas
  de un workspace ni garantías de latencia.
- Se mide la biblioteca desde un ejecutable de pruebas de integración release,
  no una petición MCP completa ni el watcher. El test usa su allocator
  predeterminado; el CLI declara mimalloc en `src/main.rs`. Por eso el working
  set y los tiempos no deben presentarse como memoria/latencia del CLI desplegado.

Las operaciones usan los presupuestos normales del indexador: source de hasta
1 MiB para sincronización general, SFC Vue hasta 512 KiB, presupuesto de 20 000
filas de publicación, snapshot de rutas con hasta 20 000 archivos/40 000
directorios y presupuesto de 100 000 probes. La prueba se mantiene muy por
debajo de esos límites; no pretende medir el comportamiento cerca de saturación.
