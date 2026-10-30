# Rendimiento de arranque del servidor MCP

Fecha de medición: 2026-10-29 (hora de Lima; 2026-10-30 02:30 UTC).

## Resultado

Se midió el servidor MCP real por stdio. El intervalo principal va desde antes
de crear el proceso `agent-harness` hasta recibir su respuesta JSON-RPC a
`initialize`. Cada muestra usa un proceso nuevo y un workspace temporal nuevo.

| Hito | Muestras | Mín. | Mediana | p95* | Máx. |
| --- | ---: | ---: | ---: | ---: | ---: |
| Crear proceso → respuesta `initialize` | 15 | 37.662 ms | 43.924 ms | 55.942 ms | 55.942 ms |
| `notifications/initialized` → respuesta `tools/list` | 15 | 3.064 ms | 3.438 ms | 7.705 ms | 7.705 ms |
| Respuesta `tools/list` → respuesta `harness_bootstrap` | 15 | 34.936 ms | 40.017 ms | 43.777 ms | 43.777 ms |
| Crear proceso → respuesta `tools/list` | 15 | 41.561 ms | 47.086 ms | 60.621 ms | 60.621 ms |
| Crear proceso → respuesta `harness_bootstrap` | 15 | 81.605 ms | 87.035 ms | 102.701 ms | 102.701 ms |

`initialize` confirmó el protocolo `2025-11-25`. `tools/list` anunció 26
herramientas. La respuesta compacta de `harness_bootstrap` fue de 9,000 bytes.
El bootstrap es una operación MCP medida después del handshake; no forma parte
del resultado principal de arranque.

\* p95 usa nearest rank: con 15 observaciones, el p95 corresponde al máximo de
esta muestra. Es un resumen descriptivo de una muestra pequeña, no una garantía
de latencia.

## Entorno y alcance

- Windows 10 Home, versión 10.0.19045, x64.
- AMD Ryzen 5 3400G con Radeon Vega Graphics, 8 procesadores lógicos, 12 GiB RAM.
- Rust `1.99.0`; Node.js `v22.16.0`.
- Ejecutable `agent-harness 0.8.9`, compilado desde el árbol actual en perfil
  release con `cargo build --locked --release --bin agent-harness`. La
  compilación tardó 7 min 22 s; ese tiempo no está incluido en las muestras.
- 3 procesos de calentamiento excluidos y 15 muestras medidas. Cada servidor
  recibió un root temporal recién creado y eliminado al terminar.
- Perfil MCP `full`; `HARNESS_INDEX_WATCH=0`. La configuración temporal
  deshabilitó SQL Server y PostgreSQL. El workspace no contenía fuentes ni un
  índice al comienzo de la muestra. No se accedió a bases de datos externas,
  red ni modelos.
- El cronómetro usa `process.hrtime.bigint()`. Los hitos posteriores al
  handshake se cronometran como fases separadas.

Esto describe el arranque con un workspace vacío y sin conexiones externas. No
mide el arranque con un índice de proyecto grande, un watcher habilitado,
configuración SQL activa ni el tiempo de una operación de herramienta. No se
limpió la caché del sistema operativo; son arranques con las condiciones de
caché normales después de los calentamientos, no una medición controlada en
frío. Las cifras no permiten afirmar un tiempo para todos los equipos o
proyectos.

## Código y reproducibilidad

El ejecutable medido fue
`target/release/agent-harness.exe`, SHA-256
`bd68fe4e9fa4053c23a88a0981595de63f690d0543e6ed14334afe0099bf0f8c`.
El código parte de `cb91667`; el árbol estaba modificado al medir (62 rutas
registradas en el resumen de estado). La compilación incluyó los cambios que ya
estaban presentes en ese árbol. No se modificó ni reemplazó una instalación
fuera del workspace. El JSON adjunto guarda los hashes SHA-256 del ejecutable,
los archivos fuente principales y el script, además de las observaciones por
muestra.

Para repetir desde la raíz del repositorio:

```powershell
cargo build --locked --release --bin agent-harness
node --check scripts/benchmark-mcp-startup.cjs
node scripts/benchmark-mcp-startup.cjs target/release/agent-harness.exe docs/benchmarks/mcp-startup-2026-10-29.json 15 3
```

Script reproducible: [benchmark-mcp-startup.cjs](../scripts/benchmark-mcp-startup.cjs).
Observaciones y hashes de esta corrida:
[mcp-startup-2026-10-29.json](benchmarks/mcp-startup-2026-10-29.json).
