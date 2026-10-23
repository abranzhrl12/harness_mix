# Paso 07: evitar validar dos veces la ruta de cada fuente

## Hipótesis y cambio

El paso 06 midió 3.426 llamadas a `io_path_validation`, además de las 7.590
validaciones iniciales de `source_path_guard`. La segunda validación consumió
unos 839 ms en la mediana de aquel diagnóstico; no equivale a una promesa de
ahorro total.

En Windows, `Project::index_source_bytes` reúne la resolución de ruta, comprobación
de ausencia, pinning de directorios y lectura protegida. `read_source` delega en
esta operación, en lugar de resolver la ruta y después llamar a otra API que la
resuelve de nuevo. No acepta una ruta absoluta supuestamente validada ni expone
un token reutilizable entre lecturas.

Se mantienen los controles léxicos, secretos, scope de archivo único, ausencia
comprobada, límites, cancelación, comprobación de metadata antes/después y rechazo
de reparse points en el handle final. Los ancestros permanecen abiertos durante
la lectura, sin compartir borrado/renombrado y rechazando reparse points. La ruta
externa conserva su política de contenido y su protección adicional. Otras
plataformas conservan el camino anterior; no se presume que tengan pinning Windows.

No se añadió dependencia ni SIMD: el trabajo eliminado son recorridos con
consultas al sistema de archivos, no un cálculo numérico sobre un buffer.
Los flags de pinning/apertura corresponden al contrato de
[CreateFile de Windows](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew).

## Verificación

- `cargo check` mediante Harness: correcto.
- Pruebas filtradas de Project: 6 correctas; incluyen lectura indexada, cambios
  entre lecturas, límites, ausentes, directorios, rutas restringidas, scope,
  política externa y junctions.
- Revalidación de fuentes: 3 pruebas correctas, incluido contenido cambiado con
  tamaño y fecha conservados.
- Ciclo de vida Rust y rollback por cancelación/timeout: 2 pruebas correctas.
- No se ha ejecutado la suite completa del workspace.

## Medición

Comparación A/B release terminada: ambos binarios usan el mismo profiler y
corpus congelado; alternancia antes/después, tres ejecuciones por variante,
200 archivos e índice temporal vacío por ejecución. No se vacía la caché del
sistema operativo ni se modifica el índice del proyecto original.

| Métrica | Antes | Después |
| --- | ---: | ---: |
| Pasada 1 | 14,479 s | 13,579 s |
| Pasada 2 | 14,164 s | 13,864 s |
| Pasada 3 | 15,276 s | 15,639 s |
| Mediana total | 14,479 s | 13,864 s |
| Segunda validación: llamadas por pasada | 3.426 | 0 |
| Segunda validación: tiempo mediano | 900,754 ms | no ejecutada |
| Lectura de revalidación: tiempo mediano | 1.839,231 ms | 1.395,070 ms |
| Validaciones iniciales por pasada | 7.590 | 7.590 |
| Pinning de directorios por pasada | 3.426 | 3.426 |

La mediana total baja **615,325 ms, un 4,25 %** (factor 1,044). Es una
observación de esta muestra, no una garantía: hay dispersión y la tercera
pareja fue más lenta después. Sí queda demostrada la eliminación del trabajo
duplicado; no se afirma que cada llamada del sistema operativo sea ahora más
rápida. Las medianas de fases son independientes y los timers anidados no se suman.

Las seis pasadas conservan exactamente la huella del grafo
`74996aa068bb2442716c50b17fdd9fc68d3cf22a3e2e81e55c84f8443da84575`,
2.462 nodos, 40.928 intentos y 309 referencias resueltas; cero fallos, saltados,
pendientes o archivos sin comprobar. Los contadores de bytes, ausentes y
revalidaciones también coinciden. El recorte a 200 archivos introduce ausencias
artificiales: no extrapolar porcentajes a todo el proyecto.

Evidencia durable: [resultados-ruta-validada-200.json](resultados-ruta-validada-200.json).
Datos completos de ejecución en `target/cold-index-path-read-ab-200/`.

Reproducir, conservando primero el binario anterior y un corpus congelado:

```powershell
cargo build --locked --release --example index_phase_probe
node scripts/benchmark-index-resolver.cjs target/cold-index-before-path-read.exe target/release/examples/index_phase_probe.exe target/cold-index-corpus-v1 target/NUEVA-carpeta-rutas 200 3
```

Binarios SHA-256:

- Antes: `02bd1da1693eeb82663add7ff9fd32502a1b298157a64807b259cac278d76931`.
- Después: `22b01317f9dc634debedd1edef5c53ca7c439177e5b92a9e4e84a8c1c19312d1`.

El siguiente experimento debe atacar la repetición del análisis Rust o las
consultas de rutas ausentes, con cachés acotadas al contenido/snapshot correcto,
sin convertir esta protección en una caché global de frescura.
