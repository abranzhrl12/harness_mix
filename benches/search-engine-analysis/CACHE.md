# Caché de matchers integrada

La optimización de producción está en src/adapters/search/matcher_cache.rs.

- LRU de proceso, con hasta 32 entradas y claves de patrón de hasta 1024 bytes.
  Patrones mayores siguen compilándose normalmente sin retenerse en la caché.
  El límite de claves no representa una medida exacta de memoria compilada.
- Clave: patrón, literal/regex, sensibilidad a mayúsculas y modo multiline.
  El terminador LF y las demás opciones del builder son constantes.
- No almacena archivos, resultados, permisos, scopes ni índices. Cada búsqueda
  conserva la lectura de fuente actual, validación de rutas y cálculo de hash.
- La compilación ocurre fuera del mutex. Dos consultas frías concurrentes pueden
  compilar la misma clave; la inserción evita duplicar entradas.
- Los errores no se cachean. Una caché con mutex envenenado se omite y la
  compilación normal sigue funcionando.
- Exacto y recuperación por términos conservan sus opciones originales.
- plan::build_fallback recibe el matcher de términos ya construido y evita
  compilar el matcher exacto que antes se descartaba.
- walk.rs, scan.rs, formatos de respuesta y algoritmo BM25 permanecen intactos.
- La implementación usa std, sin nuevas dependencias.

## Verificación

```powershell
cargo test --locked --offline -p agent-harness --lib adapters::search::
cargo test --locked --offline -p agent-harness --lib
cargo test --locked --offline --manifest-path benches/search-engine-analysis/Cargo.toml --bin cache_checks
cargo run --release --locked --offline --manifest-path benches/search-engine-analysis/Cargo.toml --bin cache_checks
```

cache_checks importa directamente el módulo de producción, sin copiar su
implementación. Sus tests verifican reutilización mediante el número de
compilaciones reales, LRU, opciones, Unicode, patrones grandes, errores y
compilación fuera del mutex. Las pruebas de integración de search verifican
cambios de fuente/hash, scope, prioridad exacta y uso directo del matcher de fallback.

## Medición de preparación

Nueve muestras, cincuenta operaciones por muestra, orden alternado; mediana
en microsegundos. La versión release mide el get de producción con mutex,
búsqueda de clave, actualización de recencia y clonación. Compara contra construir
el mismo matcher con sus opciones anteriores.

| Caso | Compilar sin caché | Acierto de caché |
| --- | ---: | ---: |
| Literal corto | 6,360 µs | 0,898 µs |
| Fallback de 16 términos | 442,192 µs | 0,770 µs |

Salida completa en CACHE-RESULTS.txt. Estos son costes de preparación, no la
latencia total de harness_search. Los misses siguen compilando y pagan el pequeño
coste de consultar/insertar en caché. El beneficio depende de que las consultas se
repitan. No se deduce aquí una aceleración global ni se mide todo el tráfico.

Los ejecutables instalados requieren recompilación/actualización para ejecutar
estos cambios. Esta tarea no sustituye ni reinicia el servidor instalado.
