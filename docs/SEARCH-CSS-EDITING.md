# Búsqueda con contexto y edición directa (Rust)

`harness_search` acepta `contextLines` (alias `context_lines`) entre 0 y 20 para
`kind="text"`. Cada coincidencia añade `hash` (SHA-256 del archivo completo) y
`context: {start, end, text}` con líneas inclusivas y fuente exacta, incluidos
CRLF y espacios. El contexto se calcula de los mismos bytes que el hash.

```json
{"kind":"text","query":".fast-tabs__item","scope":"Views","contextLines":3}
```

Sin el parámetro se conserva la respuesta habitual. El presupuesto acumulado de
metadatos es 4 MiB: al agotarse, `complete=false` y `SEARCH_INCOMPLETE` indican
cobertura parcial. Los resultados omitidos de la vista compacta se recuperan con
`harness_detail`; no se corta silenciosamente el texto de contexto retenido.

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
