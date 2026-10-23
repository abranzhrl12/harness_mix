# Paso 01 — Protocolo y línea base

## Objetivo

Medir el tiempo real del coordinador de indexación inicial y localizar su fase
dominante antes de modificar el algoritmo. Frío significa un índice SQLite vacío
nuevo por ejecución; no significa vaciar las cachés del sistema operativo.

## Medidas

- Tiempo de pared de la indexación (planificación y publicación del coordinador).
- Preparación paralela y tiempos instrumentados de selección de pendientes,
  resolución, invalidación, escritura, commit y revalidación disponibles.
- Archivos candidatos/publicados, errores, fuentes no comprobadas, intentos y
  resoluciones de referencias, nodos escritos.
- Huellas SHA-256 de fuentes seleccionadas y del grafo canónico.
- Binario/perfil/CPU/HEAD/diff y opciones efectivas del experimento.

La copia, inicialización, compilación y huellas quedan fuera de elapsedMs del
probe existente; no es una medición del tiempo total de arranque del proceso.
No se modifica el índice original. El helper copia fuentes a su propio TempDir.

## Ejecución

```powershell
cargo build --locked --release --example index_phase_probe
node scripts/benchmark-index-cold.cjs target/release/examples/index_phase_probe.exe E:/harness_mix target/cold-index-baseline-200 200 3
```

El directorio de salida debe ser nuevo. Cada ejecución usa --batch --fresh
--cold-only, pendingQuery=indexed, memo y retryBindings activados. El ejecutor
alterna uno/cuatro trabajadores y rechaza diferencias de fuentes, grafo,
contadores semánticos o fallos. Mantiene synchronous/commit/rollback existentes.

Usar el repositorio actual permite reproducir sin depender de la ubicación SWSMA
de mediciones anteriores. No es el mismo corpus: no comparar sus segundos
directamente. La muestra se selecciona por orden de ruta; no representa todos
los lenguajes ni la escala completa.

## Resultado observado

Se compiló el ejemplo release con --locked (5 min 48 s, fuera del benchmark).
Se completaron seis primeras pasadas, tres por variante de trabajadores. Fecha
local: 2026-10-22; timestamp del informe: 2026-10-23T01:24:08.936Z.
CPU AMD Ryzen 5 3400G, ocho procesadores lógicos; Windows 10.0.19045 x64.

El helper copió 548 fuentes compatibles (3 918 178 bytes) y seleccionó las
primeras 200 por ruta. El grafo muestra referencias pendientes Rust y JavaScript;
la muestra incluye código de producción, pruebas y experimentos locales.

Las seis pasadas publicaron 200 archivos, sin fallos, archivos pendientes,
fuentes no comprobadas ni saltos. Todas conservaron exactamente:

- Huella de fuentes: `140ba29045b5349b084b74038a8c935a0d25f72eff4cbbd8e58cab05c7cb0b33`.
- Huella del grafo: `e8e5a4fcf49f63b21ea0a65a82d28c57a27ddc2840581955f3852097fbf55b75`.
- 41 247 intentos de resolución, 309 referencias resueltas y 2456 nodos escritos.
- Índice inicial: cero archivos, nodos y aristas; un escritor de base de datos.

Binario SHA-256: `7cc5d4f4ffe0555b0cb2957a14657fe1b435281ed4635212ebbf4a0975066e41`.
HEAD: `25c7e9fa893503cee098e81bfd135ade7c35e825`, con el refactor de frescura
local aún sin commit. Snapshot de entradas fuente rastreadas/no rastreadas:
`83cc58f34c08cc97a3e7c60a75a4d3f3e9c103905782c1ec44c0f066b8192fa6`.
El ejecutor comprobó que ese snapshot no cambió entre pasadas.

Resumen durable: [resultados-200.json](resultados-200.json).
Informes completos locales: target/cold-index-baseline-200/metadata.json,
runs.json, summary.json y workers-{1,4}-trial-{1,2,3}.json (excluidos de Git).

## Verificación y límites

El ejecutor pasó node --check y las seis ejecuciones terminaron con código cero.
El release compila el código de producción actual, incluido el refactor anterior
de frescura; no equivale a ejecutar sus nuevas pruebas del adaptador, que en ese
momento estaban pendientes por el límite de Harness. Esas seis pruebas ya
pasaron posteriormente en el [paso 05](05-revalidacion-bytes.md).
No hubo compilaciones ni
suites lanzadas por este agente durante las pasadas cronometradas. No se aisló
la carga de otros procesos del sistema.

No se modificaron algoritmos de indexación, durabilidad ni el índice original.
Este paso establece una línea base, no demuestra una aceleración nueva.
