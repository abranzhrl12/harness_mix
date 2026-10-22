# Medición de fases de indexación — 2026-10-22

Se midió el código de indexación con un ejecutable release y contadores por fase (`HARNESS_INDEX_PROFILE=1`). La compilación y la copia de las fuentes quedan fuera de los tiempos publicados.

## Condiciones

- Fuentes: `D:/backoffice/SWSMA PRODUCCION/SWSMA`, correspondientes a los nombres del registro facilitado.
- Copia temporal aislada: 2150 archivos de lenguajes compatibles, 53 564 385 bytes. Las fuentes originales y su índice no se modificaron.
- No había `.codegraph/codegraph.db` en esa raíz. Se creó un índice nuevo únicamente en la copia temporal.
- Muestras: primeros 40 y primeros 200 archivos por ruta ordenada; dos pasadas de publicación forzada por archivo.
- La segunda pasada reutiliza el contenido del grafo creado por la primera. Las fuentes permanecen iguales entre pasadas.
- Este ensayo llama a `sync_file_forced` secuencialmente, incluyendo extracción, apertura de conexión y commit. No mide el rendimiento global con cuatro trabajadores y conexión reutilizada.
- No se copiaron credenciales ni `.agents` del proyecto fuente. Por tanto, las exclusiones semánticas específicas de esa configuración no forman parte del ensayo.
- La segunda pasada de 200 archivos transcurrió sin la suite de pruebas ni otro proceso CLI de reconstrucción activo. La suite de pruebas coincidió con parte de la primera pasada de 200 archivos: ese tiempo es exploratorio y no una comparación controlada con la segunda pasada.

## Resultados

| Muestra | Primera pasada | Segunda pasada | Fallos por pasada |
| --- | ---: | ---: | ---: |
| 40 archivos | 2,597 s | 3,398 s | 2 |
| 200 archivos | 37,289 s | 56,002 s | 6 |

Segunda pasada de 200 archivos: 194 publicaciones correctas, 2523 nodos escritos y 1 077 348 intentos de resolución de referencias acumulados. Las fases instrumentadas sumaron 54,287 s; el resto del tiempo de pared corresponde al trabajo fuera de los intervalos instrumentados.

| Fase | Tiempo | Porcentaje instrumentado |
| --- | ---: | ---: |
| Resolver referencias | 25,707 s | 47,35 % |
| Invalidar referencias y seleccionar reintentos | 22,049 s | 40,62 % |
| Commit | 2,577 s | 4,75 % |
| Extraer código | 1,398 s | 2,57 % |
| Escribir nodos y aristas | 0,854 s | 1,57 % |
| Resto de fases | 1,701 s | 3,13 % |

Lectura inicial: 0,112 s. Validación del esquema: 0,019 s. Estas dos fases no dominan esta muestra.

## Interpretación

El 87,97 % del tiempo instrumentado se dedicó a invalidación y resolución. La extracción representó solo el 2,57 %: acelerar exclusivamente esa fase, incluso eliminándola por completo, no eliminaría el coste dominante.

El código de publicación ejecuta invalidación global para JavaScript/TypeScript y selecciona referencias pendientes que pueden proceder de otros archivos. `invalidate_engine` recorre aristas por `resolvedBy`, elimina las aristas correspondientes y recrea sus referencias. Después se vuelve a llamar a `resolve_exact` para las referencias seleccionadas.

La prioridad de una optimización posterior es medir y reducir esas revisiones repetidas: invalidación dirigida por dependencias y resolución deduplicada al nivel de lote. Estos cambios deben preservar la detección de ambigüedades y la recuperación de referencias. No se implementaron optimizaciones del algoritmo en esta medición.

Los resultados no prueban cuál era la distribución exacta del registro de 1292 archivos ni predicen su duración total: falta su índice inicial y el ensayo no utiliza el coordinador paralelo. Tampoco son una comparación controlada de escalabilidad entre muestras de 40 y 200 archivos, que tienen contenidos distintos.

## Fallos observados

Todos bajo `Content/Scripts/`:

- `Script_BandejaPlanAccion.js`: `ENCODING`, no UTF-8.
- `Script_CapacitacionReporte.js`: `ENCODING`, no UTF-8.
- `Script_ConsignarProducto.js`: `ENCODING`, no UTF-8.
- `Script_GenerarComprobante.js`: `ENCODING`, no UTF-8.
- `Script_KardexProductosGeneral.js`: `ENCODING`, no UTF-8.
- `Script_FirebaseInit.js`: `SYNC_SEMANTIC`, declaraciones múltiples que el resolvedor no puede desambiguar.

## Reproducción

```powershell
cargo build --release --locked --example index_phase_probe
$env:HARNESS_INDEX_PROFILE = "1"
& ./target/release/examples/index_phase_probe.exe "D:/backoffice/SWSMA PRODUCCION/SWSMA" 200 > ./target/index-phase-probe-swsma-200.json
```

La herramienta crea y elimina su propio directorio temporal. Si la raíz contiene un índice existente, toma una copia consistente mediante una conexión SQLite de solo lectura y `VACUUM INTO`; las publicaciones se hacen sobre esa copia.

Informes JSON de esta ejecución: `target/index-phase-probe-swsma.json` y `target/index-phase-probe-swsma-200.json` (artefactos locales excluidos de Git).

Verificación de la instrumentación: 115 pruebas de indexación aprobadas y 2 omitidas por sus condiciones previas.
