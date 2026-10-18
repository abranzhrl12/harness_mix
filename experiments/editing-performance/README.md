# Evaluación de edición y búsqueda

Experimento aislado: `memchr` solo se añade a este crate, no al runtime del
harness. El editor de producción conserva la búsqueda estándar y cambia el
mapeo de posiciones CRLF para recorrer el original una sola vez.

## Variantes

- `baseline.rs`: copia congelada del editor anterior, que recorría el original
  desde el principio para convertir cada extremo de cada chunk CRLF.
- `current`: importa directamente el editor de producción. Ordena rangos
  normalizados, rechaza solapamientos y convierte los extremos con un cursor
  monotónico. Los archivos sin CRLF conservan el uso directo de offsets.
- `candidate`: el build genera una copia del editor actual que sustituye
  únicamente la búsqueda por `memchr::memmem::Finder`. El generador falla si
  cambia el código que sustituye. La segunda búsqueda comienza después del
  primer carácter, conservando el rechazo de coincidencias solapadas.

La conversión CRLF pasa de O(k × n) a O(n + k), para n bytes y k chunks. Esto
no convierte toda la edición en O(n): cada chunk sigue requiriendo una búsqueda.
Se usa memoria adicional O(k) para los rangos, sin tabla de offsets por byte.

## Reproducción

Desde la raíz del repositorio:

```powershell
cargo test --locked --manifest-path experiments/editing-performance/Cargo.toml
cargo run --release --locked --manifest-path experiments/editing-performance/Cargo.toml
```

Para repetir tiempos sin ejecutar Cargo, después de compilar:

```powershell
./experiments/editing-performance/target/release/harness-editing-performance.exe
```

## Metodología y alcance

Se prueban LF, CRLF y saltos mezclados, tamaños aproximados de 16 KiB, 128 KiB
y 1 MiB, y 1, 16 y 128 chunks en orden inverso. Las fuentes incluyen Unicode;
los chunks incluyen saltos de línea. Las tres variantes deben producir los
mismos bytes antes de medir cada caso.

Cada cifra es la mediana de cinco muestras, con al menos 40 ms por muestra
y un mínimo de una operación completa. Se rota el orden de las variantes.
`black_box` protege entradas y resultados de las optimizaciones del compilador.
Los tiempos están en microsegundos por operación. Los speedups mayores que 1
favorecen la variante nueva.

La sección `EDIT` incluye hash SHA-256, validaciones del dominio, búsquedas,
normalización, conversión de posiciones y construcción/destrucción del resultado.
Excluye disco, parser sintáctico, publicación, índice, transporte MCP y modelo LLM.
La sección `SEARCH` compara búsquedas literales por líneas, reutilizando un Finder
por consulta sobre un buffer CRLF cercano a 1 MiB. No mide el recorrido del
workspace ni sus comprobaciones de seguridad.

Las pruebas diferenciales cubren los modos de saltos de línea y el rechazo de
`aa` dentro de `aaa`. El dominio añade una prueba del mapeo contra una referencia
independiente basada en prefijos normalizados, incluidos Unicode y CR sueltos.

Las mediciones de esta máquina no constituyen una comparación con Claude Code
ni una garantía para cualquier proyecto o CPU. Los fixtures son sintéticos y
los tiempos de pared pueden variar con la carga del sistema.

## Resultados del 18 de octubre de 2026

Windows x86_64 MSVC, AMD Ryzen 5 3400G (4 núcleos/8 hilos), rustc 1.99.0,
LLVM 23.1.1, `release`, thin LTO y una unidad de generación de código.
`memchr` 2.8.3. Las dos pasadas finales se ejecutaron secuencialmente sobre el
binario compilado, después de terminar las compilaciones y pruebas previas.
No se aisló el sistema operativo ni se fijó afinidad de CPU; se descartaron las
pasadas preliminares realizadas con compilaciones concurrentes.

SHA-256 del `editing.rs` evaluado (bytes del checkout):
`E67242BBF0501D9D2AF1F3B061A8B895C1ABC239C857D0C9E30B49353277268E`.

Casos representativos cercanos a 1 MiB, en milisegundos por preparación:

| Caso | Pasada | Anterior | Una pasada | Una pasada + memchr | Mejora anterior / una pasada |
| --- | --- | ---: | ---: | ---: | ---: |
| CRLF, 1 chunk | 1 | 10.60 | 5.20 | 5.48 | 2.04× |
| CRLF, 1 chunk | 2 | 11.08 | 6.55 | 5.94 | 1.69× |
| CRLF, 16 chunks | 1 | 59.10 | 11.42 | 15.98 | 5.18× |
| CRLF, 16 chunks | 2 | 67.21 | 12.65 | 16.99 | 5.31× |
| CRLF, 128 chunks | 1 | 422.22 | 55.31 | 74.82 | 7.63× |
| CRLF, 128 chunks | 2 | 408.50 | 49.24 | 68.35 | 8.30× |
| Mixto, 128 chunks | 1 | 426.68 | 51.71 | 73.48 | 8.25× |
| Mixto, 128 chunks | 2 | 416.01 | 49.98 | 76.78 | 8.32× |
| LF, 128 chunks | 1 | 49.61 | 56.44 | 79.84 | 0.88× |
| LF, 128 chunks | 2 | 53.40 | 52.52 | 67.55 | 1.02× |

La mejora clara corresponde a CRLF y saltos mezclados. LF ya evitaba la
conversión; aquí los resultados varían, con una regresión observada del 14% en
ese caso de la primera pasada y una mejora del 2% en la segunda. No se afirma
una aceleración universal ni de las operaciones con disco/parser/MCP.

En el microbenchmark de búsqueda por líneas, Finder reutilizado mejoró la
consulta Unicode presente un 12–14% y la consulta de un carácter un 6–9%.
Fue más lento para el identificador ausente, el identificador poco frecuente
y la consulta al final de línea. En edición con 128 chunks, la variante memchr
fue más lenta que la de búsqueda estándar en las dos pasadas de todos los casos.

**Decisión:** conservar la optimización del cursor CRLF y la búsqueda estándar
del runtime. Los datos no justifican sustituir globalmente el buscador por
memchr. Ampliar a otros patrones, longitudes de línea y corpus reales antes de
tomar una decisión distinta sobre el buscador.

Datos completos, en microsegundos:

- [Edición, pasada 1](results/edit-run-1.csv).
- [Edición, pasada 2](results/edit-run-2.csv).
- [Búsqueda, pasada 1](results/search-run-1.csv).
- [Búsqueda, pasada 2](results/search-run-2.csv).
