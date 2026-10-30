# Latencia de solicitudes MCP por stdio

Medición del flujo solicitud→respuesta con el servidor ya abierto, usando la
salida estructurada predeterminada. No incluye latencia de un agente, modelo,
interfaz de usuario ni bases de datos.

## Método

Se ejecutaron dos corridas independientes. Cada corrida mantuvo un solo proceso
MCP abierto para el handshake, bootstrap y todas las solicitudes; cada
operación tuvo 20 calentamientos excluidos y 100 observaciones secuenciales.
Los 200 valores por operación se combinaron para el resumen. El cronómetro
monotónico empieza antes de serializar y escribir la solicitud JSON-RPC y se
detiene cuando el cliente Node ha leído y parseado la línea de respuesta. Las
cifras incluyen al cliente local, los pipes, el planificador y el servidor; no
representan tiempo de CPU del servidor ni transporte puro.

El workspace de cada corrida fue temporal y nuevo, con tres archivos pequeños
de prueba, sin índice. Se usó perfil `full`, `HARNESS_INDEX_WATCH=0` y una
configuración temporal que deshabilita SQL Server y PostgreSQL. El cliente
reutilizó la sesión creada por el bootstrap inicial. Las solicitudes medidas
fueron secuenciales; solo las tres lecturas dentro de cada `harness_batch` se
ejecutaron en paralelo (`maxConcurrent=3`). No se vació la caché del sistema
operativo. El sondeo `ping` se describe como round-trip observado por el cliente,
no como coste de transporte aislado.

El renderer structured estaba activado por defecto: el servidor crea con
`structured_presentation: true` y el proceso no recibió un override de
presentación. Cada respuesta reporta por separado el tamaño JSON de la respuesta,
del contenido de texto y de `structuredContent`.

## Resultados

Medianas, p95, p99, mínimo y máximo son milisegundos, calculados con nearest
rank sobre 200 observaciones por operación.

| Solicitud | Mín. | Mediana | p95 | p99 | Máx. | Respuesta mediana | `content` | `structuredContent` |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| MCP `ping` | 0.085 | 0.102 | 0.256 | 0.835 | 2.570 | 37 B | 0 B | 0 B |
| `tools/list`, catálogo caliente | 0.994 | 1.294 | 2.785 | 4.277 | 6.462 | 51,753 B | 0 B | 0 B |
| `harness_read`, archivo TypeScript de 3 líneas | 0.644 | 0.848 | 1.777 | 3.715 | 6.719 | 568 B | 89 B | 394 B |
| `harness_symbols read`, función TypeScript conocida | 0.690 | 0.869 | 2.435 | 3.686 | 7.812 | 724 B | 92 B | 547 B |
| `harness_search`, un match de texto | 2.545 | 2.921 | 6.312 | 7.744 | 11.681 | 1,380 B | 145 B | 1,150 B |
| `harness_batch`, 3 lecturas independientes | 1.192 | 1.454 | 4.427 | 6.426 | 15.815 | 1,720 B | 159 B | 1,476 B |

`tools/list` anunció 26 herramientas. El bootstrap inicial de preparación
devolvió 279 B de `content` y 4,244 B de `structuredContent`; no se incluye en
las tablas. La lista de herramientas serializa los esquemas completos; su
respuesta medida ocupó 51.8 KB.

## Investigación de optimización

No hice cambios de producción para esta medición. El round-trip `ping` está por
debajo de 0.11 ms de mediana en ambos runs. Lectura y símbolos quedan por debajo
de 0.9 ms, y el batch de tres lecturas tarda 1.45 ms de mediana. No encontré
trabajo duplicado de alto coste que pudiera eliminar conservando cancelación,
validación, hashes, diagnósticos, persistencia y concurrencia.

`McpServer::profile_router()` construye la ruta de herramientas al atender
listados y solicitudes, pero `tools/list` ya responde en 1.29 ms pese a los 26
esquemas. Mantener un router cacheado añadiría estado de ciclo de vida para
ahorrar una fracción pequeña y no mostró una oportunidad de valor suficiente.
La representación structured tampoco duplica el sobre completo en el texto:
para las herramientas medidas el texto ocupa 89–159 B, frente a 394–1,476 B de
`structuredContent`. La búsqueda de 2.92 ms incluye escaneo y procesamiento de
los archivos del workspace de prueba; no usé un índice ni agregué una caché que
cambiara la frescura de los resultados.

SQL no se midió: no se configuró un datasource de prueba seguro, y no se
consultaron conexiones de usuario. No se hicieron llamadas a red ni a modelos.
Estas mediciones describen un proceso local y un workspace pequeño; no predicen
la latencia con archivos grandes, otros equipos, proyectos reales, carga de
concurrencia o servidores externos.

## Entorno y reproducibilidad

- Windows 10 Home 10.0.19045 x64; AMD Ryzen 5 3400G, 8 procesadores lógicos,
  12 GiB RAM.
- `agent-harness 0.8.9`, release, SHA-256
  `1d0bb4267623a7392af6d990b969b9567d3426fa2af1e58519fbfcffcb371dd2`.
- Rust `1.99.0`, Node.js `v22.16.0`; compilación release actual con
  `cargo build --locked --release --bin agent-harness` (6 min 07 s; fuera de las
  mediciones).
- Revisión `cb91667`; árbol modificado por cambios previos. El JSON conserva
  hashes del binario, fuentes principales y script.
- El script completó la inicialización MCP real y verificó el estado `ok` de
  cada respuesta de herramienta. El proceso persistente se cerró al terminar.

Script reproducible: [benchmark-mcp-request-latency.cjs](../scripts/benchmark-mcp-request-latency.cjs).

Datos crudos, 100 muestras por operación en cada corrida:

- [Primera corrida](benchmarks/mcp-request-latency-before-2026-10-30.json)
- [Repetición](benchmarks/mcp-request-latency-repeat-2026-10-30.json)
