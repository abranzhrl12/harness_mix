---
name: harness-read
description: Investigar código, leer símbolos y consultar dependencias con Harness MCP. No para SQL, XLSX ni SVG.
---

# Leer e investigar

Usa los nombres y esquemas anunciados por la conexión; no inventes parámetros.
Esta guía cubre lectura e investigación, no instalación ni preparación inicial.

## Elige por la información que falta

| Necesidad | Herramienta |
| --- | --- |
| Encontrar un archivo o texto | `harness_search`, `kind=files` o `text` |
| Leer una implementación conocida | `harness_symbols`, `action=read`, `path`, `symbol` |
| Elegir un símbolo del archivo | `harness_symbols`, `action=outline`, `path` |
| Obtener solo la firma | `harness_symbols`, `action=read`, `view=signature` |
| Entender llamadas, dependencias o impacto | `harness_explore`, selector real y vista apropiada |
| Leer texto sin símbolo o formato no soportado | `harness_read`, rango suficiente |
| Recuperar contenido omitido | `harness_detail`, referencia recibida |
| Proyectar campos de un resultado grande | `harness_data_query`, JMESPath |

Un símbolo conocido se lee directamente, sin búsqueda, outline ni firma previos:

```json
{"action":"read","path":"src/modulo.rs","symbol":"validar"}
```

La lectura devuelve fuente exacta por defecto; no necesita `view=full` ni flags
de compactación. Los rangos son inclusivos, base 1. Si hay ambigüedad, selecciona
un `container` o `symbolId` devuelto; no los adivines. Una firma no es fuente para
editar. Conserva los hashes que la herramienta requiera, como `expectedHash`
para extracción Vue; no busques un hash solo para usar `replace`, donde es opcional.

## Relaciones y límites

Para llamadas, dependencias o impacto, empieza con `explore`, no con búsquedas
de referencias textuales ni lecturas archivo por archivo. Con archivo y símbolo
conocidos, llámalo directamente: no necesita búsqueda ni outline previos.

```json
{"filePath":"src/modulo.rs","symbol":"validar","view":"summary"}
```

`summary` devuelve relaciones y firmas, sin cuerpos por diseño; no recuperes
cuerpos si no cambian la respuesta. `full` (por defecto si omites `view`) añade
cuerpos; `impact` consulta impacto entrante. Si falta ubicación, usa `search`
para encontrar un candidato; si hay homónimos, acota con `filePath` y los
selectores devueltos. Una búsqueda literal sigue siendo apropiada para texto.
Añade presupuestos solo si hacen falta y la vista los admite. `maxFiles` limita
archivos objetivo y fuentes relacionadas cargadas, no el número de nodos.

Revisa cobertura, frescura y omisiones antes de concluir. Un hash de fuente o
`syntaxOnly=true` no certifica el grafo; un resultado vacío limitado no prueba
ausencia. Recupera solo cuerpos o detalles que puedan cambiar la respuesta.
Si el grafo no está disponible o vigente, avanza con fuente y declara ese límite.
No sincronices ni reindexes por rutina: lectura y búsqueda no requieren índice;
las relaciones necesitan uno compatible y el mantenimiento requiere autorización.

Reutiliza evidencia vigente. Agrupa únicamente lecturas independientes con
`harness_batch`; para explore usa `syncIndex=false` si el esquema lo admite.
Si un batch devuelve referencias, recupera sus detalles sin repetir los trabajos.
No reduzcas el presupuesto hasta provocar recuperaciones previsibles.

Ante fallo de transporte, timeout o runtime, detén reintentos equivalentes,
diagnostica con `harness_doctor` y comunica el bloqueo; no lo eludas con shell.
Una capacidad no soportada es distinta: explica el límite antes de una alternativa
autorizada. Termina cuando la evidencia responda la pregunta y explicita los
límites que afecten la conclusión.
