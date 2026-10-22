# Navegación HTML nativa con Rust

Harness usa el crate `tl` 0.7.8 para construir una estructura HTML al consultar
`harness_symbols`. No se inicia Node. Las ediciones de texto habituales no
ejecutan este parser: siguen usando hashes y publicación segura directamente.

```json
{"action":"outline","path":"src/app/form.component.html","query":"guardar"}
```

El outline incluye símbolos `html-element` con nombres como
`html:button#guardar`, atributos originales, contenedor, líneas y `symbolId`.
`structure.elements` relaciona elementos y padres por ID. Los presupuestos de
salida y `detail.ref` se aplican también a esa estructura.

```json
{"action":"read","path":"src/app/form.component.html","symbol":"html:#guardar","view":"full"}
```

Los selectores de elementos usan el prefijo `html:`. Se admiten etiquetas, `*`,
`#id`, `.clase`, `[atributo]`, `[atributo=valor]`, combinaciones como
`button.action[data-testid=guardar]` y descendientes como `main button.action`.
No se admiten pseudo-clases, grupos, escapes ni combinadores `>`, `+` o `~`.
Los valores de atributos se comparan con el texto original, sin decodificar
entidades HTML. Los selectores de CSS dentro de `<style>` mantienen sus nombres
sin prefijo; por ejemplo `.action` sigue leyendo una regla CSS, mientras que
`html:.action` selecciona elementos HTML.

Si hay varias coincidencias, se devuelve `AMBIGUOUS_SYMBOL`: añade el `symbolId`
actual al mismo selector. El ID depende del hash del archivo; una edición lo
invalida y requiere consultar nuevamente el outline.

`view:"full"` devuelve el texto original, no una serialización del DOM. Ese
`text` y su `hash` se pueden pasar a `harness_replace` como `oldText` y `hash`.
Se conservan comillas, atributos, BOM, Unicode, CRLF y el contenido de los demás
elementos. Firmas y atributos del outline son vistas acotadas, no `oldText`.

La adaptación mantiene los nombres originales de atributos Angular, incluidos
`[propiedad]`, `(evento)`, `[(ngModel)]` y `*ngIf`. Interpolaciones y cabeceras de
`@if`, `@for`, `@switch`, `@case` y `@defer` se tratan como expresiones opacas;
sus operadores `<` no crean elementos HTML ficticios. Las etiquetas aparentes
dentro de comentarios, script, style y otros bloques de texto crudo no son
elementos seleccionables. La proyección conserva los offsets y nunca se escribe
sobre el archivo original.

`tl` es tolerante y no implementa todo el estándar HTML. La cobertura se limita
a los elementos visibles para ese parser; HTML mal formado y expresiones
incompletas pueden limitarla. No se afirma validez de HTML, semántica Angular ni
equivalencia con el DOM de un navegador. Para Angular utiliza después
`harness_verify` con `language:"angular"` y el proyecto correspondiente.

Presupuestos: 2 MiB de fuente, 20 000 nodos, 4 000 elementos y 256 atributos por
elemento. Las relaciones se recorren sin recursión; las líneas se calculan desde
una tabla de posiciones creada una sola vez por consulta.

Crate y alcance: [documentación oficial de tl](https://docs.rs/tl/0.7.8/tl/).
