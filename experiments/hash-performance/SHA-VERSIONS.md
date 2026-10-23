# sha2 0.10.9 frente a 0.11.1

Se compararon ambas versiones en el mismo binario optimizado (perfil test,
opt-level=3, Windows x86_64). Dos ejecuciones completas, cada una con siete
ensayos por variante, calentamiento, orden rotado e invertido, intervalos de
al menos 75 ms y mediana por recorrido. Los datos se cargan antes de medir.
El corpus contiene 405 fuentes Rust y 2 446 568 bytes de src/ y crates/.
Hay también entradas sintéticas de 64 B, 1 KiB, 16 KiB, 64 KiB y 1 MiB.

## Corpus real: milisegundos por recorrido

| Variante | Ejecución 1 | Ejecución 2 |
| --- | ---: | ---: |
| 0.10.9, digest binario | 1,814 | 1,429 |
| 0.11.1, digest binario | 1,813 | 1,413 |
| 0.10.9, format actual | 1,859 | 1,555 |
| 0.11.1, mismo formateador adaptado | 1,882 | 1,475 |
| 0.10.9, tabla hexadecimal | 1,711 | 1,473 |
| 0.11.1, tabla hexadecimal | 1,832 | 1,523 |

La diferencia entre versiones en digest binario fue aproximadamente 0,08 % y
1,10 % a favor de 0.11.1; no es una mejora convincente frente a la variación.
En bloques de 1 MiB el digest fue 0,601 frente a 0,597 ms en la primera
ejecución y 0,571 frente a 0,572 ms en la segunda.

La tabla hexadecimal con 0.10.9 redujo el tiempo del corpus un 8,0 % y 5,3 %,
pero empeoró 64 B (0,288 frente a 0,337 us y 0,287 frente a 0,318 us) y 1 KiB.
La tabla con 0.11.1 tampoco ganó de forma consistente. No hay una sustitución
universalmente más rápida demostrada. Se midió digest más conversión; no se
midió el coste aislado de convertir un digest ya calculado.

Los intervalos cortos, la variación de frecuencia/carga y el temporizador en el
bucle para entradas pequeñas limitan las conclusiones. Son mediciones de hashing
en memoria, no del rendimiento de indexación completa ni del disco. No se debe
restar tiempos de variantes distintas para estimar el coste de conversión.

## Compatibilidad y CPU

La prueba verifica igualdad byte a byte de ambos SHA-256 para todo el corpus,
casos sintéticos y longitudes 0..256, además del vector conocido de abc.
Comprueba igualdad de la conversión hexadecimal alternativa con la actual.

El resultado de 0.11.1 no implementa LowerHex con sus features por defecto:
`format!("{:x}", Sha256::digest(bytes))` dejó de compilar en el primer intento.
La variante format de 0.11.1 adapta sus 32 bytes al tipo de salida de 0.10.9
para usar exactamente el mismo formateador. Sirve como comparación controlada,
no como propuesta de mantener dos versiones en producción.

La detección en ejecución confirmó soporte SHA, AVX2 y SSE4.1 en este equipo.
Se usaron las features por defecto de ambas librerías. Esto confirma capacidad
del procesador, no inspección del backend o ensamblado realmente seleccionado.
No se forzaron flags globales ni target-cpu=native.

## Verificación y reproducción

`cargo test --locked --manifest-path experiments/hash-performance/Cargo.toml`

Ejecutado dos veces por harness_verify: 1 prueba aprobada, 0 fallidas y 1
ignorada en cada ejecución. La ignorada es el benchmark BLAKE3 histórico,
preservado para ejecución explícita. Harness indicó freshness=not_tracked.

- sha-results-run1.json: primera ejecución.
- sha-results.json: segunda ejecución; se reemplaza al repetir el benchmark.
- src/sha_versions.rs: código del benchmark y comprobaciones de igualdad.
- Cargo.lock: versiones fijadas de dependencias del experimento.

No se cambiaron el dominio, dependencias de producción ni el índice. La evidencia
no justifica actualizar sha2 por velocidad ni adoptar esta tabla hexadecimal
globalmente. El siguiente candidato útil es reducir cálculos duplicados del mismo
contenido, conservando la revalidación cuando se vuelve a leer un archivo.

## Backend x86: runtime-dispatch frente a software forzado

El código upstream de sha2 0.10.9 selecciona el backend `x86` en builds x86/x86_64
sin `force-soft`. Ese backend usa `cpufeatures` para comprobar conjuntamente
`sha`, `sse2`, `ssse3` y `sse4.1`; con las cuatro capacidades ejecuta la función
con instrucciones SHA, y en caso contrario cae a `soft::compress`. La feature
`force-soft` selecciona directamente la implementación software. Fuentes:
[sha256.rs](https://github.com/RustCrypto/hashes/blob/sha2-v0.10.9/sha2/src/sha256.rs)
y [sha256/x86.rs](https://github.com/RustCrypto/hashes/blob/sha2-v0.10.9/sha2/src/sha256/x86.rs).

Para contrastarlos, el experimento añadió la feature local `force-soft`, que activa
`sha2/force-soft`; el modo predeterminado sigue siendo runtime-dispatch. En el
equipo Windows x86_64 todos los requisitos del backend estaban presentes. Con el
mismo corpus de 405 archivos/2 446 568 bytes y perfil `opt-level=3`, la mediana
de SHA-256 binario de 0.10.9 fue 9,594 ms con software forzado y 1,461 ms con
runtime-dispatch (6,6 veces más rápido con dispatch). Para la entrada sintética
de 1 MiB fueron 4,004 ms y 0,579 ms (6,9 veces). Cada modo usó siete ensayos;
el contraste sustenta que el dispatch mejora el rendimiento en esta máquina,
aunque no es una traza del branch ejecutado en cada llamada. Las variantes de
0.11.1 no se usan para ese contraste.

Los reportes se guardan por separado en `sha-results-force-soft.json` y
`sha-results-runtime-dispatch.json`. Se reproduce el software con
`cargo test --manifest-path experiments/hash-performance/Cargo.toml --features force-soft`;
el comando de verificación habitual conserva el modo runtime-dispatch.
El hallazgo respalda conservar el SHA-256 existente y sus features predeterminadas;
no se modificó código ni dependencias de producción.
