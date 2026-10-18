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
