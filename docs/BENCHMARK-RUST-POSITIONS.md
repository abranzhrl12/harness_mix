# Primera optimización tras corregir las relaciones Rust

## Cambio

`index_rust.rs` calculaba cada línea contando todos los saltos desde el inicio
del archivo. Ahora construye una tabla de inicios de línea una sola vez, busca
la línea mediante búsqueda binaria y cuenta UTF-16 solo dentro de esa línea.
La tabla vive durante el análisis; no añade una caché global ni cambia bindings,
visibilidad, resolución de módulos o límites de seguridad.

## Comparación A/B sobre fuentes congeladas

Perfil dev optimizado existente, no release. Un único snapshot de 72 archivos
Rust de `src`, `crates/domain/src` y `crates/application/src`, copiado a índices
temporales nuevos. Tres rondas por binario, alternando orden antes/después.
Se excluye la compilación y la inicialización; se incluye el CLI `--reindex`.

| Ronda | Antes ms | Después ms |
| --- | ---: | ---: |
| 1 | 10220,461 | 8210,388 |
| 2 | 8645,229 | 8413,628 |
| 3 | 10258,066 | 10434,123 |
| Mediana | 10220,461 | 8413,628 |

La mediana bajó 17,7 % en esta muestra. La última ronda no mejoró. No hubo
aislamiento de carga del sistema ni vaciado de cachés; no es una garantía de
rendimiento universal ni una comparación con Node.

En las seis ejecuciones coincidieron las proyecciones ordenadas de identidad de
nodos (`id`, nombre, tipo, nombre cualificado, archivo, línea), aristas completas
de resolución (origen, destino, tipo, línea, columna, metadata, procedencia) y
referencias pendientes (origen, nombre, tipo, línea, columna, archivo, lenguaje).
No se comparan timestamps, rowids, todos los metadatos de nodo ni los bytes del DB.

SHA256 de los binarios:

- Antes: `266caa19286672084633156544adaa1791e324a7447634a8e025b82daa42b03f`
- Después: `e5aeffd7e714b98de48a48b476e990994ec3d5342eca69fed328e10f93a86b3f`

## Medición aislada y corrección

La prueba de fases, sin base de datos, ejecuta diez rondas de cuatro archivos
reales. En la repetición sin compilación simultánea: extracción 548,825 ms y
análisis de bindings Rust 340,945 ms. Estos totales no separan aún resolución SQL
de escritura y no representan el reindexado completo.

En el mismo proceso, para idénticos offsets de esos archivos, el cálculo anterior
costó 125,589 ms y el nuevo 3,992 ms, incluida la construcción de tablas. Es una
mejora local aproximada de 31,5 veces; NO es una aceleración global de 31 veces.

Seis tests dirigidos aprobaron en 0,68 s: métodos, shadowing, módulos, caso real
`synchronized -> sync_workspace_all`, Unicode/BOM/CRLF y equivalencia de posiciones
en todos los offsets de fixtures, incluidos offsets UTF-8 inválidos y fuera de rango.
La auditoría del índice nativo real volvió a aprobar en 0,22 s, sin reintroducir
los tres pares falsos confirmados. No se ejecutó la suite completa.

## Reproducción

- `cargo test --locked -p agent-harness --lib rust_free_calls -- --nocapture`
- `cargo test --locked -p agent-harness --lib rust_resolution_phase_probe -- --ignored --nocapture`
- Establecer `HARNESS_POSITION_BASELINE_BINARY` a un binario anterior y ejecutar
  `cargo test --locked -p agent-harness --test native_position_benchmark -- --ignored --nocapture`.

El benchmark A/B nunca abre el índice de producción. El siguiente apartado a
medir es la resolución/publicación SQLite; todavía no se afirma que ambas estén
optimizadas ni que exista paridad completa con CodeGraph Node.

## Límite operativo detectado

Al reconstruir el core completo (118 archivos, distinto del corpus A/B de 72),
el CLI encontró dos veces su deadline global de 30 segundos: 97 y 100 commits
confirmados, respectivamente. No eran transacciones globales ni rebuilds completos.
La segunda ejecución tampoco tenía compilaciones simultáneas.

Se amplió exclusivamente el contexto del CLI `--reindex` a cinco minutos, usando
el constructor existente `OperationContext::with_timeout`. No cambia el timeout
por defecto de consultas MCP ni elimina cancelación, revalidaciones, límites de
filas o transacciones por archivo. Los hashes A/B de arriba corresponden a los
binarios medidos ANTES de esta corrección de presupuesto; no se atribuye a ella
una mejora de velocidad.

Con ese presupuesto, el reindexado nativo real terminó: 118/118 commits, cero
pendientes, cero fallos, 67 022 ms. El plan posterior mostró 118 archivos sin
cambios, cero añadidos/modificados/borrados/reintentos/sin comprobar. La auditoría
de relaciones se repitió después del rebuild y aprobó en 0,19 s.

Los 67,02 s NO son una mejora respecto al reindexado histórico de 116 archivos
en 19,19 s. Cambiaron el corpus y las fuentes, y existe variación de carga; no
se ha aislado todavía la causa del mayor coste del core completo. La afirmación
de mejora se limita al A/B congelado de 72 archivos. Falta separar y optimizar
resolución y publicación SQLite en ese escenario completo.
