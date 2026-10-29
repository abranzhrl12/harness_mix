# Razor MVC legacy

Fecha: 2026-10-21. Harness indexa vistas `.cshtml` y resuelve sus destinos sin
Roslyn ni una aplicación ASP.NET en marcha. Este documento describe el
subconjunto pensado para proyectos **legacy** (ASP.NET MVC 5 y anteriores:
`Views/`, `Controllers/`, `Areas/`, `.cshtml` con `@helper`, `@section` y
llamadas `Html.*`/`Url.*`).

## Qué se resuelve hoy

| Evidencia en la vista | Tipo de referencia | Destino |
| --- | --- | --- |
| `@Url.Action("Edit", "Orders")` | `mvc_route` | acción `OrdersController.Edit` |
| `@Html.ActionLink("Home", "Index", "Home")` | `mvc_route` | acción `HomeController.Index` |
| `@Html.Action("Index", "Orders")` | `mvc_route` | acción `OrdersController.Index` |
| `@Html.Partial("_Orders")` | `razor_partial` | archivo de vista parcial |
| `@Html.RenderPartial("_Orders")` | `razor_partial` | archivo de vista parcial |
| `@Html.RenderAction("Details", "Orders")` | `mvc_route` | acción `OrdersController.Details` |
| `Layout = "~/Views/Shared/_Layout.cshtml"` | `razor_layout` | archivo de layout |
| `<a asp-action="Edit" asp-controller="Orders">` | `mvc_route` | acción `OrdersController.Edit` |

Las llamadas también se detectan dentro de regiones C# en línea (`@{ ... }`,
`@(...)`) y dentro de bloques `@code`/`@functions`. Es la forma habitual de las
vistas legacy: `@{ Html.RenderPartial("_Orders"); }`. Una llamada escrita dentro
de una cadena C# no es evidencia y se descarta.

## Formularios y JavaScript legacy

El enlace estático de formularios admite un único `<form id="...">` con
campos `<input name="...">` únicos y un botón con `onclick="handler()"`.
El handler debe ser una función inline única. Una referencia del handler al
campo solo se emite cuando el AST encuentra `document.querySelector('[name=campo]').value`
dentro de `JSON.stringify(...)` en `body` de una llamada `fetch`.
Las aristas incluyen `resolvedBy`, procedencia heurística y confianza;
no prueban el comportamiento en ejecución. Se omiten vistas con transiciones
Razor o comentarios en el markup anterior al script, formularios múltiples,
selectores dinámicos y scripts que declaran `document`, `JSON` o `fetch`.

Cuando el parser nativo rechaza un archivo JavaScript, la recuperación solo
reconoce sentencias completas independientes con URL literal y marca cada
referencia como `recovered` con confianza 0.5. No recupera el grafo completo,
los handlers ni expresiones dinámicas. Una extracción parcial no certifica
frescura y no debe sustituir un grafo completo anterior.

## Cómo se localiza una vista por nombre

Para una vista `Views/Orders/Index.cshtml` (o `Areas/Admin/Views/Orders/…`):

1. Nombres simples (`_Orders`): `Views/Orders/_Orders.cshtml` y después
   `Views/Shared/_Orders.cshtml` del área de la vista.
2. Rutas con `~/` (`~/Views/Shared/_Layout.cshtml`): desde la raíz de la
   aplicación y desde la raíz del área.

Solo se acepta un literal sin interpolación ni escapes. Si **dos** archivos
candidatos existen, la referencia queda pendiente: nunca se elige uno por
convención. Si el destino no está indexado, la referencia queda pendiente
también, y se resuelve sola en la siguiente sincronización o reindexación.
Si el destino se renombra o se mueve, hay que volver a sincronizar la vista
que lo referencia: la arista caducada no se adivina.

## Qué no se resuelve (y por qué)

- **Nombres dinámicos**: `Html.Partial(Model.PartialName)` o
  `Html.RenderPartial(name)` no son literales y no producen destino.
- **Argumentos con nombre**: `Url.Action(action: "Index", controller: "Home")`
  sigue sin binding semántico; la referencia queda pendiente.
- **`_ViewStart.cshtml`**: el layout heredado de una carpeta no se propaga
  automáticamente. Declara `Layout` en la vista si necesitas la arista.
- **`RenderSection`**: la vista que define `@section` y el layout que la
  renderiza se relacionan a través de la arista `razor_layout`, no de una
  arista por sección. La dirección layout→vistas es ambigua y no se infiere.
- **Vistas `Shared` sin controlador**: `asp-controller` explícito sigue siendo
  necesario para resolver una acción desde una vista compartida.
- **Routing dinámico, `asp-route-*`, transforms de runtime, DI**: fuera del
  subconjunto estático.
- No es un compilador Razor: no comprueba tipos, no evalúa expresiones y no
  sustituye a `dotnet build`.

## Cómo probarlo en tu proyecto

```sh
agent-harness --root <proyecto> --init-index
agent-harness --root <proyecto> --reindex
agent-harness --root <proyecto> --sync
```

Después, `harness_explore` con `filePath` de la vista devuelve sus destinos:
acciones de controlador, parciales y layout. Las referencias no resueltas
quedan como `pending`/`failed` en el índice, con su nombre literal.

Las vistas existentes necesitan una sincronización normal para adquirir las
referencias nuevas; no hace falta migración de esquema ni reindexar todo el
proyecto.

## Evidencia

- `tests/razor_legacy_mvc.rs`: fixture MVC 5 sintético (layout, parcial
  compartido, controlador con `Index`/`Edit`/`Details` y `HomeController`).
  Comprueba la extracción de `razor_partial`, `razor_layout` y `mvc_route`, y
  la resolución contra los archivos de vista indexados.
- `docs/razor-resolution.md`: reglas de resolución MVC y sus límites.
- `docs/razor-parser-evaluation.md`: evaluación de gramáticas Tree-sitter para
  Razor. Nada de ese trabajo está integrado: el parser propio cubre el
  subconjunto descrito aquí.
