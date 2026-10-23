# Paso 08: reutilizar análisis Rust por contenido

## Objetivo

En el diagnóstico del paso 06 se ejecutaron 1.513 análisis Rust para una muestra
de 200 archivos; unos 3,16 segundos correspondían a `rust_analyze`. Cada
publicación ya reutilizaba sus hechos locales, pero la siguiente publicación
volvía a analizarlos. Ese coste no es un ahorro prometido.

El experimento reutiliza únicamente hechos sintácticos independientes de ruta,
no resoluciones de nodos ni veredictos de frescura. La caché pertenece al Writer
de una operación explícita; no es global ni persistente. El SHA-256 ubica el
contenido y una comparación exacta de bytes confirma su identidad. Se conserva
la observación contra el hash indexado y la relectura antes del commit.

## Implementación y límites

Implementación y revisión independiente delegadas a GPT-6 Luna; el agente
principal revisó la integración, corrigió con el implementador los errores de
visibilidad/import detectados al compilar y ejecutó las verificaciones.

- `rust_analysis_cache.rs`: caché FIFO de hasta 512 entradas y 32 MiB estimados,
  contando contenido, `Facts::reserved()` y 256 bytes de overhead por entrada.
  Esto no es una medición ni un límite exacto de RSS del proceso.
- `Writer` posee la caché; se pasa explícitamente por la transacción y publicación
  SQLite a `Sources`, sin introducirla en el dominio ni en los puertos de aplicación.
- Clave: identidad fija del analizador (`rust-facts-v1`) y SHA-256; se comparan
  además los bytes completos. Cambiar la semántica/configuración del analizador
  exige mantener coherente esa identidad. No hay caché persistida entre binarios.
- `Arc<Facts>` evita copiar mapas y cadenas. El mutex solo protege consulta e
  inserción, nunca el parsing. Se conserva el presupuesto local de 16 MiB.
- Solo se insertan análisis satisfactorios; no errores de parser, cancelación
  ni timeout. No se guardan decisiones sobre nodos o referencias de la BD.
- Se mantiene el atajo local por ruta dentro de una publicación: no se añaden
  miles de observaciones redundantes para acceder a la caché compartida.
- Las entradas mayores que los límites se omiten y se expulsan las más antiguas.
  Evicción o fallo de identidad implica volver a analizar, no asumir hechos.

## Verificación

`cargo check` mediante Harness y compilación release del probe correctos.
20 pruebas seleccionadas correctas:

- 5 de integración de la caché con RustResolution: reutilización, rechazo de
  contenido modificado/hash obsoleto/ausente, actualización del hash indexado,
  cancelación/timeout, presupuesto local y recheck después de un acierto.
- 3 de caché: igualdad exacta incluso ante digest idéntico simulado, FIFO,
  presupuesto de bytes y entradas demasiado grandes.
- 3 de VerifiedSources, 5 de imports, 2 del ciclo de vida Rust/rollback y
  2 de cierre/contención del Writer.

La nueva prueba de recheck cambia contenido conservando tamaño y fecha después
de reutilizar Facts: sigue devolviendo `STALE`. No se ejecutó la suite completa
del workspace ni una medición de memoria máxima del proceso.

## Medición

A/B de confirmación terminado. Se hizo una primera serie exploratoria mientras
concluían verificaciones; se conserva en `target/cold-index-rust-cache-ab-200/`.
La serie final se ejecutó sin iniciar compilaciones ni pruebas en paralelo por
parte del agente, en `target/cold-index-rust-cache-confirm-200/`.

| Métrica | Antes | Después |
| --- | ---: | ---: |
| Pasada 1 | 11,380 s | 8,789 s |
| Pasada 2 | 11,221 s | 9,408 s |
| Pasada 3 | 13,571 s | 8,741 s |
| Mediana total | 11,380 s | 8,789 s |
| Análisis efectivos por pasada | 1.513 | 173 |
| Reutilizaciones entre publicaciones | 0 | 1.340 |
| Tiempo mediano de análisis | 2.711,725 ms | 242,834 ms |
| Bytes analizados | 11.952.969 | 1.032.040 |
| Bytes leídos | 26.127.486 | 26.127.486 |
| Dependencias revalidadas | 1.513 | 1.513 |

La mediana baja **2,591 segundos, un 22,77 % menos tiempo** (factor 1,295).
Se evitan el 88,57 % de los análisis. Las consultas de caché (hash y comparación
de bytes incluidos) suman una mediana de 12,310 ms para 1.513 consultas; insertar
173 resultados suma 0,775 ms. Los timers de fase son anidados: no sumarlos con
sus padres ni presentar medianas independientes como una descomposición exacta.

Todas las pasadas conservaron la huella de grafo
`74996aa068bb2442716c50b17fdd9fc68d3cf22a3e2e81e55c84f8443da84575`,
2.462 nodos, 40.928 intentos y 309 referencias resueltas; cero fallos, saltados,
pendientes o archivos sin comprobar. Se verificaron iguales los contadores de
lectura, hash de validación, ausentes, revalidación, commits y llamadas locales
al resolvedor, salvo el trabajo analítico reducido deliberadamente.

La serie exploratoria también mostró mejora, pero de menor magnitud; sus
tiempos se conservan en la evidencia durable. Hay variación entre series y no se
garantiza este porcentaje en otros equipos, lenguajes, tamaños o corpus. No debe
compararse directamente con los tiempos de pasos anteriores ejecutados en otro
momento; el antes/después válido usa aquí los mismos dos binarios y corpus.
El recorte a 200 archivos no representa todo el proyecto ni una caché de SO fría.

Evidencia durable: [resultados-cache-rust-200.json](resultados-cache-rust-200.json).
El índice original no fue modificado y no se hizo commit/push.

Reproducir conservando antes el binario del paso 07:

```powershell
cargo build --locked --release --example index_phase_probe
node scripts/benchmark-index-resolver.cjs target/cold-index-before-rust-cache.exe target/release/examples/index_phase_probe.exe target/cold-index-corpus-v1 target/NUEVA-carpeta-cache-rust 200 3
```

SHA-256 del binario posterior:
`3aa33d4f2f41e344f2879de28bded0bfc5f2f247a4621da1d6f372376c6347e1`.

## Protocolo

- Binario anterior: `target/cold-index-before-rust-cache.exe`, copia del paso 07.
- SHA-256 anterior: `22b01317f9dc634debedd1edef5c53ca7c439177e5b92a9e4e84a8c1c19312d1`.
- Mismo corpus congelado e instrumentación en ambos binarios.
- Índice temporal vacío por ejecución; caché del SO no vaciada.
- Tres pasadas alternadas por variante; equivalencia de grafo y resultados.
- Contar aciertos/fallos de caché, análisis efectivos, bytes leídos y revalidados.
- El proyecto y su índice original no son el destino del experimento.
