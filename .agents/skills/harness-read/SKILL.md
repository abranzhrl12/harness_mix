---
name: harness-read
description: Investigar código, leer símbolos y consultar dependencias con Harness MCP. No para SQL, XLSX ni SVG.
---

# Leer e investigar

Usa los nombres y esquemas anunciados por la conexión; no inventes parámetros.
Esta guía cubre lectura e investigación, no instalación ni preparación inicial.
Si el problema ocurre en una aplicación web (visual, interacción, consola o
hidratación), carga también `../harness-browser/SKILL.md` para obtener evidencia
del navegador; la lectura de código sola no verifica lo que ocurre en la pestaña.

## Elige por la información que falta

Prioriza `harness_explore` para investigar cómo funciona código y seguir sus
relaciones; evita cadenas de búsquedas y lecturas archivo por archivo. Para
localizar texto literal o una ruta desconocida, usa `harness_search`; para leer
un símbolo ya conocido sin necesitar relaciones, usa `harness_symbols` directamente.

| Necesidad | Herramienta |
| --- | --- |
| Encontrar un archivo o texto | `harness_search`, `kind=files` o `text` (`kind=files` coincide con substring del nombre, no con globs: `*.cs` no encuentra nada; usa `Almacen` o filtra con `globs`) |
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

### Lectura por bloques

`harness_read` usa `path`, `start` y `end`: líneas inclusivas, base 1, máximo
420 líneas por llamada (`end - start + 1 <= 420`). Pide solo el rango necesario,
no 420 por defecto. Por ejemplo: `{"path":"src/modulo.rs","start":80,"end":160}`.
Si necesitas más, continúa desde el último `end + 1`, sin releer bloques ni
recorrer todo el archivo si un símbolo o `explore` responde la pregunta.

## Relaciones y límites

Para llamadas, dependencias o impacto, empieza con `explore`, no con búsquedas
de referencias textuales ni lecturas archivo por archivo. Con archivo y símbolo
conocidos, llámalo directamente: no necesita búsqueda ni outline previos.

```json
{"filePath":"src/modulo.rs","symbol":"validar","view":"summary"}
```

Los selectores de archivo aceptan separadores nativos (`src\\modulo.rs` en Windows)
y rutas relativas con `./`; el servidor normaliza antes de consultar el índice, y
si no hay coincidencia exacta reintenta con minúsculas antes de reportar
`TARGET_NOT_RESOLVED`.

`summary` devuelve relaciones y firmas, sin cuerpos por diseño; no recuperes
cuerpos si no cambian la respuesta. `full` (por defecto si omites `view`) añade
cuerpos: su texto llega en un bloque lean cuyo código numerado (`N | `) usa un
fence con el lenguaje de la extensión (` ```csharp `, ` ```javascript `…), en
orden natural de lectura (título, código, relaciones al final); una meta-línea
compacta lleva estado, frescura, cobertura y `detail.ref` — nunca busques JSON
en ese texto: el sobre íntegro entra por `structuredContent` y `detail.ref`
recupera el resultado completo sin repetir el análisis. `impact` consulta
impacto entrante. Si falta ubicación, usa `search` para encontrar un candidato;
si hay homónimos, acota con `filePath` y los selectores devueltos. Una búsqueda
literal sigue siendo apropiada para texto. Añade presupuestos solo si hacen
falta y la vista los admite. `maxFiles` limita archivos objetivo y fuentes
relacionadas cargadas, no el número de nodos.

La misma lectura lean aplica a las demás herramientas de solo lectura: 
`harness_read` renderiza el fragmento con fence de lenguaje y líneas
absolutas `N | ` (el título lleva la línea inicial); `harness_symbols`
action=read reutiliza ese formato y action=outline devuelve una tabla
`Line | Kind | Name | Signature`; `harness_sql` action=query/`describe`
entregan tablas Markdown (con QUERY/SCHEMA en readonly siempre) y las
definiciones de rutinas llegan en un fence `sql`; `harness_git` action=diff
envuelve el parche en un fence `diff` y action=status agrupa tracked/
conflicts/untracked; `harness_search` kind=text agrupa coincidencias por
archivo con líneas numeradas. En todos los casos el sobre canónico
(rectángulos completos, hashes, filas crudas) sigue en `structuredContent`.
Los errores mantienen el JSON estructurado con diagnósticos intactos.

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
