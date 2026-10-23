# SHA-256 frente a BLAKE3

La comparación nueva de versiones SHA-256 está documentada en
[SHA-VERSIONS.md](SHA-VERSIONS.md). Ahora `cargo test` ejecuta esa comparación;
para repetir el ensayo histórico BLAKE3 usar
`cargo test --locked --manifest-path experiments/hash-performance/Cargo.toml -- --ignored`.

Medición local del 22 de octubre de 2026: Windows x86_64, sha2 0.10.9 y
blake3 1.8.7. El Cargo.lock conserva las dependencias del experimento aislado.
No se modifican dependencias de producción, hashes existentes ni el índice.

## Método

Ejecutar `cargo test --locked --manifest-path experiments/hash-performance/Cargo.toml`.
También se puede usar harness_verify con language=rust, action=test y
scope=experiments/hash-performance. El perfil test de este experimento usa
opt-level=3; no es el perfil release del binario principal.

Una sola prueba verifica vectores conocidos y que ambos algoritmos producen
valores diferentes para las mismas entradas. Después mide un solo hilo, con
datos previamente cargados en memoria, calentamiento y siete ensayos de al menos
150 ms por variante, alternando el orden. Se informa la mediana del tiempo por
recorrido. Los ensayos individuales ordenados están en results.json.

El corpus reúne 405 archivos .rs de src/ y crates/, 2 446 568 bytes, hasta 1 MiB
por archivo. Los casos sintéticos usan bytes deterministas de distintos tamaños.
Se separan digest binario y hexadecimal: SHA-256 usa exactamente el format!
de harness_domain::hash; BLAKE3 usa to_hex().to_string(). Esto último compara
las rutas de salida de las librerías, no exclusivamente sus algoritmos.

## Resultados con salida hexadecimal

| Entrada | SHA-256 | BLAKE3 | Menor tiempo |
| --- | ---: | ---: | --- |
| Corpus de 405 fuentes | 1,678 ms | 2,044 ms | SHA-256 |
| 64 bytes | 0,377 us | 0,445 us | SHA-256 |
| 1 KiB | 0,960 us | 1,582 us | SHA-256 |
| 16 KiB | 10,529 us | 9,336 us | BLAKE3 |
| 64 KiB | 41,728 us | 33,835 us | BLAKE3 |
| 1 MiB | 669,576 us | 581,541 us | BLAKE3 |

En el corpus BLAKE3 tarda un 21,8 % más. Midiendo solo el digest, SHA-256
también gana: 1,818 ms frente a 2,215 ms. Las muestras binarias y hexadecimales
se miden en momentos distintos: sus diferencias no permiten aislar el coste
de conversión; hay variación del entorno. No se atribuye el resultado a una
instrucción de CPU concreta: no se inspeccionó el backend seleccionado.

Es una ejecución local con siete ensayos internos, no una conclusión universal.
No mide disco, extracción, SQLite, indexación completa ni BLAKE3 multihilo.
El coste observado del hashing del corpus es pequeño; no demuestra una mejora
apreciable de indexación al cambiar el algoritmo.

## Compatibilidad revisada

- crates/domain/src/lib.rs: hash() usa SHA-256 y devuelve hexadecimal.
- crates/application/src/indexing/planning.rs: source_hash() reutiliza hash();
  valid_hash() acepta 64 caracteres hexadecimales sin identificar el algoritmo.
  Por ello un hash BLAKE3 puede pasar la validación de forma sin ser equivalente.
- src/adapters/index_store/publication/storage.rs: escribe hash(bytes) en
  files.content_hash. Cambiarlo altera las huellas persistidas.
- src/adapters/index_store/framework.rs: compara hash de fuente y destino con
  files.content_hash; mezclar algoritmos invalida estas comparaciones de frescura.
- tests/node-compat.test.cjs: calcula explícitamente SHA-256 con Node para
  comprobar lectura y proporcionar hashes a las ediciones. Cambiar la función
  global rompe esta interoperabilidad y los tokens de edición existentes.
- tests/architecture.rs: la lista permitida de dependencias del dominio contiene
  sha2; introducir blake3 ahí requiere revisar esa regla de arquitectura.

No es una sustitución directa compatible. Una migración necesitaría identificar
el algoritmo/versionar las huellas, conservar SHA-256 para contratos externos o
versionarlos, y coordinar la actualización de hashes del índice y consumidores
CodeGraph. No se probó una migración ni un proveedor CodeGraph externo.

Recomendación con la evidencia local: conservar SHA-256. Si se explora BLAKE3
en una caché interna independiente, medir primero su impacto en el flujo completo.

## Verificación

harness_verify ejecutó el experimento: 1 prueba aprobada, 0 fallidas, código 0.
Compilación: 21,34 s; prueba: 28,96 s. Estos tiempos no forman parte de las
mediciones de hashes. Harness informó verificationFreshness=not_tracked.
La ejecución en segundo plano no estuvo disponible por TASK_STORE_BUSY;
la verificación directa de Harness terminó correctamente.
