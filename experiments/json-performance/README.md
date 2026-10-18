# Evaluación de JSON y resultados guardados

Experimento aislado: no cambia dependencias, formato MCP ni caché del runtime.
Compara `serde_json` 1.0.154 y `simd-json` 0.18.1 con `mimalloc` 0.1.52,
los mismos tipos JSON-RPC de `rmcp` 3.5.1 y el código real del dominio.

## Reproducción

Desde la raíz:

```powershell
cargo test --locked --manifest-path experiments/json-performance/Cargo.toml
cargo build --release --locked --manifest-path experiments/json-performance/Cargo.toml
./experiments/json-performance/target/release/harness-json-performance.exe
```

El ejecutable admite `parse` o `cache` para ejecutar solo una sección. Compilar
y terminar otras pruebas antes de medir. Los fixtures contienen datos sintéticos;
no se capturan conversaciones, archivos privados ni claves reales.

## Qué mide

Cada cifra es la mediana de siete muestras de al menos 50 ms. Se rota el orden
de las variantes y se usa `black_box`. Los tiempos están en microsegundos por
operación e incluyen destrucción de los valores producidos. Son tiempos de pared
locales; no se fija afinidad de CPU ni se aísla el sistema operativo.

### Parseo y serialización

Se prueban solicitudes Git, símbolos, multi-chunk e inline JSON, y respuestas de
grafo pequeñas y grandes. Las solicitudes caben en el límite de entrada de
128 KiB del harness; las respuestas pueden ser mayores. Los tamaños de los
nombres de caso son objetivos aproximados: la columna `bytes` contiene el real.

Para cada mensaje se comparan `serde_json::Value` y el tipo real
`ClientJsonRpcMessage`/`ServerJsonRpcMessage` del SDK. Se comprueba equivalencia
de las representaciones deserializadas antes de medir.

- `serde_parse_us` y `simd_parse_us`: solo parseo y destrucción, con el mismo
  coste de reloj por operación. Se copia el mensaje a un buffer preasignado
  fuera del temporizador para dar entrada fresca al parser que la modifica.
- `simd_copy_parse_us`: copia/asignación del buffer mutable dentro de la medida,
  además de parseo y destrucción. Aproxima el coste cuando el origen es inmutable.
- `serde_encode_us`: serialización con asignación de salida del valor/tipo ya
  parseado. No es una comparación de serializadores.

Estas cifras no incluyen pipes, framing, despacho de herramientas, compatibilidad
de mensajes inválidos, UI, red ni inferencia. No prueban que cambiar el parser
mejore por el mismo factor una llamada MCP completa. `rmcp` sigue usando su codec
actual en producción.

### Caché y proyección

`serde_retrieve_us` usa `Results::save` y `Results::value` de producción bajo un
mutex: poda/lookup y reparseo del texto guardado. Las variantes experimentales
usan un mutex sobre el texto para copiarlo y parsearlo con simd-json, o sobre un
`Value` retenido para clonarlo. Estas dos variantes no implementan TTL, búsqueda
de referencias ni cuotas: son aproximaciones de coste, no reemplazos completos.

Las columnas `*_query_us` añaden el handler de `data_query.rs` de producción:
clonado, medición del JSON, redacción, compilación/evaluación JMESPath y formación
del resultado. Se compila directamente ese archivo con un adaptador de archivos
que rechaza toda lectura; solo se evalúa la ruta `data` usada tras resolver
`resultRef`. Se omiten la envoltura de respuesta y la admisión de la aplicación.

`actual_save_us` mide guardar texto mediante `Results::save`, incluidos UUID,
cómputo UTF-16 y expulsiones. `prototype_save_us` añade clonar el árbol como coste
aproximado de retener texto y Value. El clon de ese prototipo se descarta en la
misma iteración; no representa una implementación de la política de retención.
El almacén de estas escrituras es distinto del de recuperación para no expulsar
la referencia que se mide.

La equivalencia de resultados y la redacción se comprueban antes de medir.
No se mide el consumo de memoria retenida: guardar texto y árbol simultáneamente
lo aumentaría y exigiría rediseñar las cuotas antes de una integración.

## Pruebas

El experimento comprueba mensajes MCP válidos, equivalencia en los fixtures,
rechazo de un pequeño corpus inválido (incluido UTF-8), una diferencia observada
con un surrogate incompleto,
enteros de 64 bits, Unicode y que la proyección redacta sin modificar la copia
retenida. No sustituye pruebas exhaustivas de compatibilidad ni fuzzing del SDK.

## Resultados del 18 de octubre de 2026

Windows x86_64 MSVC, AMD Ryzen 5 3400G (4 núcleos/8 hilos), rustc 1.99.0.
Release con thin LTO y una unidad de generación de código. Dos pasadas
secuenciales del ejecutable compilado, tras terminar las compilaciones. La
proyección usa el contexto y las comprobaciones de deadline de producción.
Todos los tiempos siguientes son medianas en microsegundos por operación.

### Solicitudes recibidas por el servidor

Tipos MCP del SDK; la columna simd incluye copia del buffer:

| Caso | Bytes | Pasada | serde_json | simd-json + copia |
| --- | ---: | --- | ---: | ---: |
| Git status | 110 | 1 | 4.238 | 3.452 |
| Git status | 110 | 2 | 3.928 | 3.219 |
| Leer símbolo | 221 | 1 | 6.423 | 4.835 |
| Leer símbolo | 221 | 2 | 4.034 | 3.951 |
| Multi-chunk | 5817 | 1 | 32.646 | 33.157 |
| Multi-chunk | 5817 | 2 | 39.308 | 36.529 |
| JSON inline | 121997 | 1 | 34.724 | 53.893 |
| JSON inline | 121997 | 2 | 34.647 | 56.679 |

Las ganancias en solicitudes pequeñas son del orden de microsegundos, sin una
mejora uniforme de 2–4×. El mensaje inline grande, dominado por un string,
empeora en ambas pasadas.

En respuestas de grafo de 821896 bytes, el parseo del tipo MCP pasa de
2851–3295 µs a 1708–1849 µs incluyendo copia: 1.67–1.78×. Esas respuestas las
parsea el cliente: sustituir el parser del servidor no acelera automáticamente
el cliente de Antigravity/Claude Code. El encoder es una etapa distinta que no
se sustituye en esta comparación.

### Resultados guardados y proyección

Caso de 821789 bytes de JSON, comparando el recorrido actual con retener un
Value y clonarlo:

| Etapa | Pasada | Actual | Value retenido + clon |
| --- | --- | ---: | ---: |
| Recuperar referencia | 1 | 1042.237 | 120.060 |
| Recuperar referencia | 2 | 1613.535 | 255.845 |
| Recuperar y proyectar con handler real | 1 | 3780.186 | 2817.505 |
| Recuperar y proyectar con handler real | 2 | 4705.955 | 3804.321 |
| Guardar (aproximación del prototipo) | 1 | 2192.674 | 2542.370 |
| Guardar (aproximación del prototipo) | 2 | 2589.815 | 3171.088 |

Recuperar es 6.3–8.7× más rápido en este caso; la consulta completa reduce su
tiempo un 19–25% (1.24–1.34×). Guardar el prototipo cuesta un 16–22% adicional.
La memoria del árbol retenido no se ha cuantificado: las cuotas actuales
cuentan texto, no memoria del DOM.

No hay ganancia uniforme en los casos menores: con 165917 bytes, la consulta
pasa de 1813.686 a 1879.667 µs en la primera pasada (regresión) y de 1621.974 a
1282.810 µs en la segunda. Sustituir solo el parser de caché por simd-json tampoco
mejora consistentemente la consulta completa. Los datos adjuntos incluyen todos
los casos, no solo los favorables.

### Diferencia de compatibilidad

Con las versiones/configuración fijadas, para `{"x":"\uD800"}`, serde_json
devuelve error por surrogate sin pareja; simd-json devuelve `{"x":"\u0000"}`.
La prueba `pinned_parsers_differ_on_unpaired_surrogate` registra esa diferencia.
Esto impide tratar el cambio del codec MCP como sustitución transparente sin
resolver primero la compatibilidad. Parsear texto producido por serde_json en
la caché es un escenario distinto de recibir entrada externa no confiable.

## Decisión

Conservar el parser y el protocolo actuales del runtime. Las mediciones no
justifican reemplazar globalmente serde_json ni prometen acelerar la UI.
La caché sin reparses merece una evaluación separada, con contabilidad de
memoria, TTL, expulsiones y paginación por detalle; el prototipo no implementa
esas garantías. No se modifica la caché de producción con este experimento.

Datos completos: [pasada 1](measurements-run-1.txt) y
[pasada 2](measurements-run-2.txt). Las cuatro pruebas del experimento pasan.

SHA-256 de los archivos de producción usados (bytes del checkout):

- `crates/domain/src/results.rs`: `461EE06B6CC4E0971A1326CDD113295BB279091C562F2DC31731504B77118B57`.
- `src/adapters/data_query.rs`: `1540576DBF09B539361C738D8F45FB5290A1F52C13136EC26A82368AB54A7A9B`.
