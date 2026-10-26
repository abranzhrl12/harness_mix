# Búsqueda con contexto y edición directa (Rust)

Para buscar estructuras de código se usa la misma herramienta y `query`,
añadiendo únicamente `ast` con el lenguaje. El motor es `ast-grep-core`;
el patrón se compila una vez y se buscan nodos AST, sin índice persistente.

```json
{"query":"fetch($URL)","ast":"javascript","scope":"src","contextLines":2}
```

`$NAME` captura un nodo y `$$$ARGS` una secuencia. Reconoce código multilínea
y variaciones de formato. `ast` admite los lenguajes incorporados de ast-grep,
incluidos Rust, JavaScript/JSX, TypeScript, TSX, C#, Python, Go, HTML, CSS y JSON.
Selecciona archivos por su extensión; no extrae JavaScript/CSS de Vue, Svelte,
HTML o Razor. No se combina con `regex`, `caseSensitive` ni otros `kind`.

Cada coincidencia estructural incluye `file`, `line`, `endLine` (inclusivo),
`startByte`/`endByte` (fin exclusivo), `text`, `hash` y `context` exacto,
incluso sin `contextLines`. El contexto abarca todas las líneas del nodo más
las líneas adicionales solicitadas. La vista compacta puede acortar `text`;
la fuente íntegra se conserva en el detalle y el contexto. La presentación
MCP compacta omite el hash de cada fila; `harness_detail` lo conserva junto
con el resultado completo. Los archivos con
errores sintácticos se reportan como `parse_error` con `complete=false`.

Se fija ast-grep 0.39.9 porque comparte Tree-sitter 0.25 con el grafo existente;
las versiones recientes requieren otro runtime nativo y entran en conflicto.

La búsqueda de texto usa `grep-searcher` y `grep-regex`, las bibliotecas de
ripgrep. Por defecto conserva coincidencias literales y distingue mayúsculas.
`regex=true` habilita expresiones regulares de Rust por línea;
`caseSensitive=false` permite coincidencias sin distinguir mayúsculas, incluido
Unicode. Ambos parámetros son exclusivos de `kind="text"`.
No admite consultas multilínea, lookaround ni backreferences. Una expresión
inválida devuelve `ARGUMENT` antes de recorrer archivos. El matcher se compila
una vez por consulta y se reutiliza entre archivos, sin índice persistente.

```json
{"kind":"text","query":"\\b(validate|verify)\\w*\\s*\\(","regex":true,"caseSensitive":false,"contextLines":3}
```

`harness_search` acepta `contextLines` (alias `context_lines`) entre 0 y 20 para
`kind="text"`. Cada coincidencia añade `hash` (SHA-256 del archivo completo) y
`context: {start, end, text}` con líneas inclusivas y fuente exacta, incluidos
CRLF y espacios. El contexto se calcula de los mismos bytes que el hash.

`limit` selecciona una página compacta. Si hay más resultados guardados,
`nextCall` incluye una llamada a `harness_search` con `cursor` y `offset`; esa
continuación pagina el snapshot del primer scan y no vuelve a recorrer el árbol.
El cursor comparte la retención de `detail` (10 minutos o hasta cerrar su sesión).
Los snapshots parciales también se pueden recorrer, pero cada página conserva
`scanComplete=false` y `snapshotOnly=true`: las páginas siguientes solo contienen
filas encontradas antes del presupuesto, no descubren archivos que quedaron fuera.
`complete` describe si terminó el scan; `contentTruncated` indica texto recortado
en las filas visibles y `presentationTruncated` indica filas que siguen en otras
páginas. `truncated` conserva la señal combinada por compatibilidad. La búsqueda
está acotada a 15 s, 10 000 archivos, 2 MiB por archivo, 64 MiB leídos y 4 MiB
de metadatos. `scanIncompleteReasons` distingue límites de tiempo/cancelación,
presupuesto de archivos, bytes o coincidencias, y archivos omitidos.

```json
{"kind":"text","query":".fast-tabs__item","scope":"Views","contextLines":3}
```

Sin el parámetro se conserva la respuesta habitual. El presupuesto acumulado de
metadatos es 4 MiB: al agotarse, `complete=false` y `SEARCH_INCOMPLETE` indican
cobertura parcial. Las filas que siguen después de `limit` se recuperan mediante
`nextCall`; `harness_detail` conserva el snapshot completo, incluido el texto de
contexto retenido, sin recortarlo silenciosamente.

`harness_symbols` ofrece `outline` y `read` para reglas CSS en `.css` y bloques
`<style>` en `.html`, `.htm` y `.cshtml`. En Razor conserva también los símbolos
C#. El selector completo es el nombre del símbolo:

```json
{"action":"read","path":"Views/Registro.cshtml","symbol":"#orden-servicio-tabs .fast-tabs__item","view":"full"}
```

En `.scss` el outline usa el árbol SCSS real (Raffia): reglas anidadas con su
contenedor, variables, mixins, funciones, `@include`, `@use`/`@forward` y
`@extend`. La lectura devuelve el rango exacto de origen.

```json
{"action":"outline","path":"app/widget.component.scss"}
```

La lectura devuelve el cuerpo exacto y el hash del archivo. Los selectores
repetidos requieren desambiguación con `container` o `symbolId` del outline.
El extractor CSS reconoce bloques y agrupaciones habituales; no es un compilador
CSS ni amplía la cobertura del grafo CodeGraph. SCSS sí tiene validación
sintáctica con posición: un candidato inválido se rechaza y el archivo original
queda intacto, sin necesidad de `allowUnvalidatedText`.

`harness_replace` acepta parámetros camelCase, snake_case y variantes de
mayúsculas, incluidos los campos de `chunks`. Por ejemplo, `new_text` y
`allow_unvalidated_text` equivalen a `newText` y `allowUnvalidatedText`.
Dos nombres para el mismo argumento son un error, igual que los campos desconocidos.
Los esquemas anuncian las formas camelCase y snake_case habituales.

El hash es opcional: al omitirlo, Harness lee el archivo y vincula la publicación
a ese contenido. Un `oldText` único basta. `line` permite seleccionar su línea
inicial (desde 1) cuando se repite; sigue siendo obligatorio proporcionar el texto
original. No se combina `line` con `chunks`.

```json
{"path":"styles.css","line":12,"oldText":"padding: 5px;","newText":"padding: 8px;","allowUnvalidatedText":true}
```

`preview=true` permite revisar el diff sin publicar. Un hash explícito obsoleto
se rechaza; la publicación sigue comprobando cambios concurrentes. Omitir el hash
opera sobre la lectura interna actual, no certifica que coincida con una lectura
anterior de la sesión. Para exigir esa precondición, envía `hash` o `baseHash`.
