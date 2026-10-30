# Optimización del arranque MCP

## Cambio

`harness_bootstrap` calcula las capacidades locales del workspace. La ruta
`src/adapters/capabilities.rs` ya enumeraba los directorios absolutos de PATH y
guardaba sus nombres para responder parte de ese catálogo. Después volvía a
recorrer PATH siete veces con `process::executable` para confirmar `cargo`,
`rustc`, `git`, `go`, `python`, `node` y `dotnet`, repitiendo comprobaciones de
archivos que la enumeración podía descartar.

Ahora el sondeo nativo usa esos nombres para saltar directorios sin candidato;
solo comprueba con `is_file` y `canonicalize` las rutas que coinciden. Mantiene
el orden de búsqueda de PATH, acepta únicamente el ejecutable nativo con el
sufijo de plataforma y usa el sondeo directo si una enumeración falla o queda
incompleta. La detección de MSBuild mediante `vswhere`, los campos de
capacidades, el perfil full, los watchers y las conexiones de base de datos no
cambiaron.

La línea de código optimizada solo participa en `harness_bootstrap`: no reduce
el tiempo hasta `initialize` ni cambia el catálogo `tools/list`.

## Comparación

Compilé el ejecutable release desde el árbol actual y medí tres corridas de 15
muestras por versión; cada corrida tenía tres calentamientos excluidos. La
línea base usó el mismo binario conservado antes de compilar, SHA-256
`bd68fe4e9fa4053c23a88a0981595de63f690d0543e6ed14334afe0099bf0f8c`. El
ejecutable optimizado tiene SHA-256
`ac32d786ff5b23630ebfac5929ea894f5b2ab1070da851e622c68320193b13f3`.

Los valores agrupan 45 muestras por versión. El p95 usa nearest rank sobre las
45 observaciones. Los tiempos son de pared desde un reloj monotónico.

| Hito | Antes: mediana | Antes: p95 | Después: mediana | Después: p95 |
| --- | ---: | ---: | ---: | ---: |
| Proceso → respuesta `initialize` | 44.438 ms | 71.111 ms | 47.766 ms | 57.697 ms |
| `notifications/initialized` → `tools/list` | 3.782 ms | 7.705 ms | 3.903 ms | 7.082 ms |
| `tools/list` → respuesta `harness_bootstrap` | 42.080 ms | 59.423 ms | 36.729 ms | 45.666 ms |
| Proceso → respuesta `tools/list` | 48.204 ms | 76.665 ms | 51.687 ms | 63.755 ms |
| Proceso → respuesta `harness_bootstrap` | 91.034 ms | 141.360 ms | 89.869 ms | 104.842 ms |

El intervalo específico de `harness_bootstrap` bajó 5.351 ms (12.7 %) en la
mediana agrupada. El tiempo total hasta recibir bootstrap bajó 1.165 ms (1.3 %)
en esa misma agrupación. El handshake `initialize` no mejoró; `tools/list` se
mantuvo prácticamente igual. Las medianas de bootstrap variaron entre corridas:
40.017/40.987/50.675 ms antes y 30.789/37.976/38.839 ms después. Por eso el
ahorro más claro es la fase de bootstrap; la mejora total de arranque es
pequeña y la latencia depende del equipo y de las condiciones de carga.

El protocolo aceptado fue `2025-11-25` y todas las corridas anunciaron 26
herramientas. Cada muestra inició un proceso nuevo con un root temporal vacío,
`HARNESS_INDEX_WATCH=0`, perfil `full`, SQL Server y PostgreSQL deshabilitados, y
sin solicitudes externas. El benchmark midió también el bootstrap compacto,
que ocupó 9,000 bytes en las corridas. No limpié la caché del sistema operativo;
son arranques en condiciones de caché normales, no una medición controlada en
frío. Las variaciones de las corridas y sus valores completos están en los
JSON enlazados abajo.

## Entorno y verificación

- Windows 10 Home 10.0.19045, x64; AMD Ryzen 5 3400G, 8 procesadores lógicos,
  12 GiB RAM.
- `rustc 1.99.0`, Node.js `v22.16.0`.
- Binario `agent-harness 0.8.9`, perfil release. La compilación optimizada
  posterior tardó 5 min 05 s; ese tiempo no se incluye en los resultados.
- Revisión `cb91667`; el árbol estaba modificado por cambios previos y
  concurrentes. El benchmark guardó hashes de fuentes principales, del script
  y del binario en cada archivo JSON. El archivo de implementación compilado
  `src/adapters/capabilities.rs` tenía SHA-256
  `15b52ec2c1f47de34216e49104d18b7dab7885941af4da9cfff9f569da48eb71`. Después
  del build solo ajusté, bajo `cfg(test)`, los nombres de los ejecutables
  temporales para usar el sufijo de plataforma; la ruta de producción no cambió.
- Pasaron las cuatro pruebas focales de
  `cargo test --locked --lib adapters::capabilities::tests`. No se ejecutó la
  suite global.
- El script de medición existente se mantuvo sin cambios. Los tres JSON
  optimizados se midieron con ese mismo script después del build release.

Script de medición: [benchmark-mcp-startup.cjs](../scripts/benchmark-mcp-startup.cjs).

Línea base, tres corridas:

- [Corrida original, 2026-10-29](benchmarks/mcp-startup-2026-10-29.json)
- [Línea base antes del build](benchmarks/mcp-startup-before-2026-10-30.json)
- [Línea base de contraste posterior](benchmarks/mcp-startup-baseline-after-2026-10-30.json)

Binario optimizado, tres corridas:

- [Primera corrida](benchmarks/mcp-startup-after-2026-10-30.json)
- [Repetición](benchmarks/mcp-startup-after-repeat-2026-10-30.json)
- [Corrida final](benchmarks/mcp-startup-after-final-2026-10-30.json)
