# Paso 10: pruebas de selección SQL de referencias pendientes

## Alcance

Pruebas y revisión delegadas a GPT-6 Luna. No se modifica la consulta de
producción ni el esquema hasta validar una alternativa. No se escribe en la BD
original del proyecto ni se reconstruye su índice.

La fase `select_pending_references` del paso 09 costó una mediana de 12,618 s en
el corpus completo congelado. Incluye `retry::pending`, recoger sus IDs e
insertarlos en un BTreeSet con límite; no incluye la construcción del catálogo
de imports, que tiene su propia fase `retry_binding_catalog`.

## Referencia y condiciones

- La referencia es el `PENDING_SQL` actual, no el selector legacy ya optimizado
  en trabajos anteriores.
- Esquema e índices comprobados en `vendor/codegraph-native/schema.sql`.
- Se preservan orden, deduplicación y el sentinel de 20.001 filas.
- Se comprueban lenguajes, configuración, catálogos nulos/vacíos/duplicados,
  nombres ya intentados y ramas conservadoras de framework/proveedores.
- En el modo legacy, la selección efectiva incluye el escaneo separado de
  proveedores. No basta con comparar exclusivamente `LEGACY_PENDING_SQL`.
- Los fixtures nuevos son temporales y usan SQLite en memoria. Los planes y
  pasos de VM no equivalen por sí solos a acelerar la indexación completa.

Hipótesis inicial: separar las ramas conservadoras unidas por OR puede evitar
trabajo de ramas inactivas. También puede repetir escaneos y empeorar otros
lenguajes: se deben medir ambos casos antes de elegir.

## Resultados sintéticos iniciales

Se ejecutó `cargo test --locked -p agent-harness --lib optimization_tests --
--nocapture --test-threads=1`: 12 pruebas correctas (6 de este selector SQL y
6 existentes de optimización de imports), cero fallos. No se ejecutó la suite
completa ni un A/B de indexación real. Hubo un aviso de variable `index` sin uso
en la versión experimental de las pruebas.

La candidata separa la última condición OR conservadora en ramas UNION,
manteniendo la selección exacta, deduplicación, orden y límite global. Se
amplió la matriz con NULL/JSON null y se comprobó el sentinel de 20.001 filas.

Fixtures SQLite en memoria: 40.000 filas mixtas para los perfiles por lenguaje,
unas 20.000 para Rust y proveedores. Cinco repeticiones alternadas con warmup;
el tiempo excluye preparar la sentencia y crear el fixture, pero incluye
ejecutarla y recoger IDs. No es un porcentaje de mejora del indexado global.

| Caso | Actual (mediana) | Candidata (mediana) | Pasos VM actual → candidata |
| --- | ---: | ---: | ---: |
| Rust, mixto | 39,291 ms | 19,131 ms | 1.211.982 → 361.686 |
| JavaScript, config=false | 75,452 ms | 76,602 ms | 2.014.217 → 1.626.068 |
| JavaScript, config=true | 61,276 ms | 65,450 ms | 2.014.217 → 1.626.068 |
| C#, config=false | 27,364 ms | 4,480 ms | 1.116.716 → 150.085 |
| Rust, super:: | 13,003 ms | 4,151 ms | 680.426 → 160.517 |
| Proveedores | 23,287 ms | 22,041 ms | 795.498 → 563.549 |

Conclusión: no reemplazar globalmente la consulta. Rust muestra margen para
probar una selección especializada, pero JavaScript registra una regresión de
hasta 6,8 % en estas medianas. Menos pasos de VM no implica automáticamente
menos tiempo. Tras restaurar las pruebas, sigue pendiente validar la alternativa
especializada contra un corpus real antes de aplicarla.

## Incidencia de concurrencia y reproducibilidad

Durante compilación/ejecución, el archivo
`src/adapters/index_store/publication/pending_optimization_tests.rs` perdió las
adiciones experimentales y volvió a 273 líneas, SHA-256
`1576ad56f54dbcaaf8dc56dd4e8bbd77d0c5195d0d1bb0b4ccefd813cb033981`.
El implementador y el agente principal no hicieron esa restauración. Se
preservó ese estado hasta recibir autorización del usuario para recuperar las
pruebas. Los resultados iniciales corresponden al ejecutable que sí compiló
las adiciones originales, no al archivo de 273 líneas.

La consulta de producción y el esquema no se modificaron. Tampoco se hizo
commit/push ni se modificó el índice original.

Evidencia: [resultados-pendientes-sinteticos.json](resultados-pendientes-sinteticos.json)
y [log-pruebas-pendientes.txt](log-pruebas-pendientes.txt), incluidos planes
EXPLAIN de los fixtures. Los fixtures simplifican el esquema; sus índices
relevantes tienen formas equivalentes, no son una copia íntegra de la BD real.

## Restauración autorizada y nueva ejecución

Se reconstruyeron únicamente las pruebas experimentales; no es una recuperación
byte a byte del archivo perdido. Se conservan los tres tests previos, su matriz
ampliada de equivalencia y las tres pruebas sintéticas nuevas. Se eliminó una
variable sin uso y se invirtió el orden de ejecución entre variantes en las
repeticiones alternas, después de un warmup por variante.

Archivo restaurado: 632 líneas. SHA-256 comprobado antes y después de ejecutar:
`b7c6d73b93516c7270c4963b944b33e69581eda1cd324baf0789b6e12f097749`.
No cambió durante esta ejecución. `retry.rs` continúa sin diff de producción.

```powershell
cargo test --locked -p agent-harness --lib publication::retry::optimization_tests -- --nocapture --test-threads=1
```

Resultado: **6 pruebas SQL correctas, cero fallos y sin avisos de compilación**.
No se ejecutó la suite completa del workspace.

| Caso | Actual (mediana) | Candidata (mediana) |
| --- | ---: | ---: |
| Rust, mixto | 42,872 ms | 18,565 ms |
| JavaScript, config=false | 79,475 ms | 83,222 ms |
| JavaScript, config=true | 61,756 ms | 57,309 ms |
| C#, config=false | 28,339 ms | 4,545 ms |
| Rust, super:: | 17,820 ms | 6,671 ms |
| Proveedores | 18,642 ms | 19,032 ms |

Los pasos de VM coinciden con la ejecución inicial. Los tiempos varían; de
nuevo, menos pasos de VM no garantizan menor tiempo. No se adopta la variante
global: el siguiente experimento sería una selección especializada para Rust,
con equivalencia de grafo y medición sobre el corpus real.

Nueva evidencia:
[resultados-pendientes-restaurados.json](resultados-pendientes-restaurados.json)
y [log-pruebas-pendientes-restauradas.txt](log-pruebas-pendientes-restauradas.txt).
