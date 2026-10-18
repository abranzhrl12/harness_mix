# Revisión: equivalencia con el CodeGraph del harness antiguo

Fecha: 2026-10-17. Conclusión: **no hay paridad de relaciones; no cerrar la
migración por una mejora de tiempo de indexación**. También se confirmaron
falsos positivos nativos, no solo relaciones pendientes.

## Método y alcance

Se añadió únicamente el test diagnóstico opt-in
`real_core_graph_parity_review` en `tests/real_core_validation.rs`.
Se ejecutó solo ese test: terminó correctamente en 23,43 s de ejecución,
sin recompilar release ni ejecutar la suite completa. Que el test termine
correctamente significa que completó la auditoría, NO que haya paridad.

71 archivos Rust de src, crates/domain/src y crates/application/src se copiaron
una sola vez a una instantánea temporal y de ahí a ambos backends. Se verificó
igualdad de hashes antes y después de indexar. No se abrió/modificó el índice
real. Es el corpus Rust de la medición previa, NO todos los proyectos históricos
ni todos los lenguajes; tampoco incluye Cargo.toml u otros archivos de contexto
fuera de esos fuentes Rust. CodeGraph no se trata como oráculo del compilador.

Escritor CLI Rust: release 0.7.0, SHA-256
`b3aad6925c161afdcd7d6ebfe4c95dafe5155f45165951b9b0a5976494b4870b`.
Proveedor: CodeGraph 1.6.2. Los índices temporales se leen solo en modo read-only.
Las consultas nativas de explore usan la biblioteca debug compilada del workspace;
las del proveedor usan su CLI JSON, como el fallback del harness antiguo.

## Qué consumía realmente el harness

`graph-json.cjs` consulta nombre exacto, archivo y callers/callees con límite 20.
`graph-library.cjs` usa getCallers/getCallees por ID. La implementación 1.6.2
de GraphTraverser incluye calls, references, imports, instantiates y navigates;
deduplica por nodo vecino y no reporta el propio foco. El filtro de aristas del
explore Rust incluye las mismas cinco clases. Por tanto, imports/references NO
pueden descartarse simplemente como capacidades que el harness nunca utilizaba.

## Identidades y diferencias de aristas

1258 nodos en cada índice: mismos IDs y mismos nombre, kind, nombre cualificado,
archivo, línea y columna inicial. No se comparó aquí todo el resto de metadatos.
Las 1439 relaciones contains coinciden exactamente. La diferencia se concentra
en la resolución, no en un faltante general de nodos del extractor.

Ocurrencias comparadas por source, target, kind, line y column:

| Clase | Rust | CodeGraph | Compartidas | Solo CodeGraph | Solo Rust |
| --- | ---: | ---: | ---: | ---: | ---: |
| calls | 811 | 1574 | 687 | 887 | 124 |
| references | 127 | 908 | 120 | 788 | 7 |
| imports | 0 | 48 | 0 | 48 | 0 |
| instantiates | 83 | 94 | 83 | 11 | 0 |
| contains | 1439 | 1439 | 1439 | 0 | 0 |
| implements | 0 | 4 | 0 | 4 | 0 |

contains/implements no forman parte de los callers/callees anteriores. Las
cinco clases relevantes tienen 1734 ocurrencias exclusivas del proveedor y
131 exclusivas de Rust: el delta total de 1607 aristas NO equivale simplemente
a 1607 relaciones correctas pendientes de portar.

Deduplicando vecinos y excluyendo autoenlaces, antes de límites de herramientas:
759 pares en Rust, 2121 en CodeGraph, 680 compartidos; 1441 solo proveedor y
79 solo Rust. Entre 650 focos function/method, difieren los conjuntos de callers
en 238 y los de callees en 481. Son diferencias respecto al proveedor, no medidas
de recall/precisión ni porcentajes de migración terminada.

## Hallazgos

### P1: asociaciones falsas por nombre, verificadas en fuente

- `process.rs:355`: `command.get_envs().collect()` se enlaza con
  `rust_symbols::collect(source, digest, context)` de rust_symbols.rs:286.
  Es una llamada a Iterator::collect, no a esa función de navegación.
- `index_store.rs:1020–1021`: execute() sobre sentencias preparadas de SQLite
  se enlaza con `Application::execute(Request)` de application/src/lib.rs:103.
- `index_store.rs:389`: la llamada a la clausura local run() se enlaza con
  Operation::run de application/src/lib.rs:39.

El fallback exact-name de resolve_exact acepta un candidato público único sin
demostrar el receptor o binding léxico Rust. En estos casos el extractor entrega
un nombre corto: que no contenga punto o :: no demuestra que sea función libre.
No se debe corregir únicamente copiando todas las aristas adicionales de Node.

### P1: relaciones verdaderas ausentes que sí afectan al harness

`explore::synchronized` llama explícitamente a
`super::index_store::sync_workspace_all` (explore.rs:121). La consulta callers
del proveedor devuelve synchronized; Rust devuelve cero. Se confirmó además
con harness_explore sobre el workspace actual y la definición actual.

También quedan pendientes imports Rust reales, por ejemplo
`crate::ports::SessionRepository` en application/src/batch.rs:3, cuyo destino
es application/src/ports.rs. El motor nativo actual no publica esos imports.

### Comparaciones reales de las interfaces: sin truncación de grafo

Focos con una sola definición, profundidad 1, mismo símbolo y límites:

| Foco / dirección | Vecinos Rust | Vecinos CodeGraph | Mismo conjunto |
| --- | ---: | ---: | --- |
| plan_workspace / callers | 0 | 0 | Sí |
| plan_workspace / callees | 4 | 11 | No |
| sync_workspace_all / callers | 0 | 1 | No |
| sync_workspace_all / callees | 2 | 4 | No |
| resolve_exact / callers | 1 | 1 | Sí |
| resolve_exact / callees | 3 | 10 | No |

Ninguno de los seis grafos estaba truncado. Cuatro respuestas diferentes:
no es solo un efecto de orden o de superar el límite 20. Igualdad en los dos
casos restantes tampoco certifica ausencia o completitud frente al código.
Las referencias de tipos forman parte de los callees del proveedor por diseño,
no deben confundirse todas con invocaciones ejecutables.

## Clasificación preliminar de diferencias relevantes

Buscando la referencia original pendiente en el índice nativo por origen, clase,
línea y columna: 811 casos de nombre simple, 629 de receptor/miembro, 241 de ruta
con :: y 53 sin referencia pendiente coincidente. Estas categorías no prueban
que cada arista del proveedor sea correcta. Sus etiquetas fueron exact-match
824, instance-method 453, qualified-name 296, import 117, function-ref 1 y
43 sin etiqueta. No son una estimación de esfuerzo.

## Prioridad recomendada

1. Bloquear falsos positivos de receptores/métodos y bindings locales Rust.
   Ante evidencia insuficiente, conservar la referencia pendiente, no adivinar.
2. Recuperar imports, rutas de módulos y referencias necesarias reutilizando
   las reglas del CodeGraph original donde sean aplicables; no implementar
   todos sus lenguajes/frameworks por esta revisión.
3. Convertir los contraejemplos confirmados en regresiones pequeñas y repetir
   los seis contratos anteriores antes de nuevas optimizaciones generales.
4. Ampliar después a proyectos representativos con sus configuraciones reales
   y a los lenguajes usados por el harness. No declarar paridad global por este
   único corpus ni exigir copiar errores potenciales del proveedor.

No se corrigió producción en esta revisión. Solo se añadió el diagnóstico y
este informe. Los fallos iniciales del test fueron de montaje (constructor de
aplicación y nombres inbound/outbound); se corrigieron antes del ensayo final.
