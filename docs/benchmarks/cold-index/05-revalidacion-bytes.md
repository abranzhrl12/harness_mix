# Paso 05 — Evitar hashes redundantes durante la revalidación

## Hipótesis y cambio pequeño

VerifiedSources conserva los bytes que ya coincidieron con el hash del índice.
Al terminar la publicación, recheck vuelve a leer cada archivo y antes calculaba
dos SHA-256 hexadecimales: el de los bytes recién leídos y el de los conservados.

Se sustituye esa comparación por igualdad exacta de los dos buffers ya disponibles.
Elimina dos digests y sus strings por archivo revalidado; no añade caché ni estado.
Conserva la lectura protegida, límites, detección de desaparición, cancelación,
timeout y el mismo error STALE para un cambio de contenido.

No cambia hash(), sha2, los hashes persistidos ni la verificación inicial contra
files.content_hash. La comprobación exacta de contenido no depende de que tamaño
y fecha cambien. No se omiten comprobaciones de frescura.

## Alcance de rendimiento

El paso 04 encontró que imports y pruebas específicas dominan la resolución.
Esta mejora pequeña afecta a la revalidación posterior (recheck_sources), que
también fue relevante en las mediciones. No se presenta como solución al coste
de resolver_special; se evita una reestructuración mayor antes de desglosarlo.

## Pruebas

Se añaden pruebas del adaptador real para contenido idéntico (incluidos vacío y
bytes no UTF-8), cambios de igual tamaño con timestamp restaurado, desaparición
de un archivo vacío, cancelación y timeout. Debe conservarse el rollback de
publicaciones cuando cambian fuentes verificadas.

Harness cargo check del proyecto principal ya pasó. La herramienta no admite
filtros de tests Rust: se usa Cargo local para las pruebas focalizadas, con el
comando exacto registrado al terminar; no se declara ejecución de toda la suite.

Resultado: 16 pruebas aprobadas, cero fallidas:

- verified_sources::tests: 3 nuevas de comparación/revalidación.
- rust_ra_tests: 2 de ciclo de vida del grafo y rollback ante cancelación/timeout.
- index_store::imports::tests: 5 de imports, límites y revalidación.
- symbol_graph::freshness::tests: 6 del refactor de caché anterior, ahora ejecutadas.

Cada selección usó cargo test --locked -p agent-harness --lib FILTRO --
--test-threads=1. La primera compilación de pruebas tomó 1 min 09 s; las
compilaciones/pruebas no forman parte del benchmark. El archivo modificado fue
formateado con rustfmt y los tests posteriores compilaron el resultado.

## A/B

Se conserva el release instrumentado del paso 04 como
target/cold-index-before-byte-recheck.exe. Se usará el mismo corpus congelado
target/cold-index-corpus-v1/, con ambos binarios instrumentados de igual manera,
tres primeras pasadas por variante, orden alternado y cuatro trabajadores.

```powershell
cargo test --locked -p agent-harness --lib verified_sources::tests -- --test-threads=1
cargo build --locked --release --example index_phase_probe
node scripts/benchmark-index-resolver.cjs target/cold-index-before-byte-recheck.exe target/release/examples/index_phase_probe.exe target/cold-index-corpus-v1 target/cold-index-byte-recheck-ab-200 200 3
```

Exigir mismo grafo, fuentes, nodos, intentos/resoluciones y fallos. Mostrar tiempos
individuales y mediana de total y recheck_sources. No atribuir diferencias de
otras fases al cambio ni prometer mejoras por debajo de la variación observada.

## Resultado de indexación completa

El release compiló en 2 min 02 s, fuera del tiempo medido. Se completaron las
seis primeras pasadas, sin builds/tests lanzados por este trabajo en paralelo:

| Variante | Ensayo 1 | Ensayo 2 | Ensayo 3 | Mediana |
| --- | ---: | ---: | ---: | ---: |
| Dos SHA-256 | 13,540 s | 13,611 s | 13,717 s | 13,611 s |
| Comparación exacta de bytes | 14,488 s | 13,285 s | 14,280 s | 14,280 s |

La mediana total nueva fue un 4,91 % mayor. recheck_sources pasó de 1,778 s a
1,967 s; resolver_special (sin cambios) pasó de 6,080 s a 6,375 s. No se obtuvo
una mejora global demostrada ni se aisló la causa de esas diferencias. Tres
ensayos y carga externa no controlada no permiten descartar regresión real.

Las seis ejecuciones conservaron 200 archivos publicados, 40 928 intentos,
309 referencias resueltas y 2462 nodos, sin fallos/pending/unchecked/skips.
Huella de grafo: `74996aa068bb2442716c50b17fdd9fc68d3cf22a3e2e81e55c84f8443da84575`.
Fuente: `d08b32b3329fe107565cf93c0a5a926189f987778bc250925f89589f59208bd8`.
Binario anterior: `a0824445c562922c446951563b5175d8ba83a9e4295043a6f8f64cd349c250eb`.
Binario nuevo: `af66e274b5da8eef47fecf650061875019831b2f12ef7c6fb3c463d5f749812a`.

Ambos binarios tienen los mismos timers resolver_*; a diferencia del paso 04,
no se añadió instrumentación al binario nuevo de esta comparación.
Resumen durable: [resultados-recheck-200.json](resultados-recheck-200.json).
Informes raw: target/cold-index-byte-recheck-ab-200/ (excluidos de Git).

## Medición aislada del trabajo evitado

Para distinguir CPU de comparación frente a IO y otras fases, se creó el
experimento independiente experiments/recheck-comparison. Usa sha2 0.10.9 y el
mismo formateador hexadecimal anterior, perfil test opt-level=3, dos buffers
distintos precargados, un hilo, calentamiento y siete ensayos alternados de
al menos 100 ms, agrupados de 64 en 64 para amortizar el reloj. No incluye
lecturas, SQLite, verificaciones de rutas ni indexación.

| Buffers idénticos | Dos SHA-256 + hex | Igualdad de bytes | Cociente |
| --- | ---: | ---: | ---: |
| 8 KiB | 12,941 us | 0,589 us | 22,0 veces |
| 128 KiB | 157,638 us | 5,130 us | 30,7 veces |
| 1 MiB | 1298,602 us | 42,977 us | 30,2 veces |

También se midió una diferencia en el último byte: cocientes de 29,7, 31,4
y 28,8 veces, respectivamente. Son datos sintéticos de un equipo concreto;
no equivalen a acelerar esos factores la revalidación completa ni el indexador.
La prueba del experimento pasó mediante Harness, código cero (1 aprobada).
Cargo.lock fija las dependencias; results.json conserva los siete ensayos.

Reproducir: cargo test --locked --manifest-path
experiments/recheck-comparison/Cargo.toml. El resultado se guarda en ese directorio.

## Decisión

Conservar la comparación exacta por ser más directa, evitar dos hashes y reducir
el coste aislado de esa operación, manteniendo lectura protegida y comprobaciones.
No etiquetarla como aceleración de la indexación en frío: el A/B global no la
demostró. Para ganar segundos sigue siendo prioritario medir los componentes
internos de resolver_special. No se cambian sha2 ni los contratos SHA-256.
