# Evaluación de bibliotecas Razor, HTML y JavaScript

Fecha: 2026-10-20. Código de Harness revisado: `d3e9135`.

## Objetivo y estado

Evaluar bibliotecas existentes para mejorar la cobertura de vistas `.cshtml`
sin mantener desde cero un parser completo de Razor, HTML y JavaScript.
Esta investigación no integra parsers nuevos en Harness ni demuestra una mejora
de rendimiento frente a su implementación actual.

El siguiente candidato a probar es **jlcrochet/tree-sitter-razor + Oxc**.
Su código contiene reglas para elementos HTML vacíos, scripts como texto crudo
y expresiones Razor/C#. Todavía no se ha compilado ni medido ese candidato.

## Lo que ya tiene el proyecto

- `vendor/codegraph-native/Cargo.toml`: Tree-sitter `0.25.10`, C# `0.23.5`,
  JavaScript `0.25.0` y TypeScript `0.23.2`.
- `Cargo.toml`: Oxc `0.90.0`, incluido parser y análisis semántico.
- `src/adapters/index_extract.rs`: extracción Razor propia; valida la vista,
  recoge directivas, proyecta miembros y expresiones a C#, y recoge enlaces MVC.
- `src/adapters/razor_render.rs`: proyección C# con conservación de ubicaciones.
- `src/adapters/razor_mvc.rs`: extracción y resolución del subconjunto MVC
  documentado en [razor-resolution.md](razor-resolution.md).

La función Razor revisada no incorpora una pasada de extracción de JavaScript
embebido. No confundir tener Oxc como dependencia con usarlo ya para esos scripts.

## Candidatos y fuentes

| Candidato | Rust e integración | Estado de evaluación |
| --- | --- | --- |
| [jlcrochet/tree-sitter-razor](https://github.com/jlcrochet/tree-sitter-razor) | Binding `LANGUAGE`; su manifest declara Tree-sitter `0.25.10` para pruebas | Fuente revisada; pendiente compilar y medir |
| [tree-sitter-razor-modern 0.1.0](https://github.com/meloncholera/tree-sitter-razor-modern) | Crate publicado; parser C compilado mediante `cc`; API Rust | Compilado y probado con Tree-sitter `0.25.10` |
| [tree-sitter-html 0.23.2](https://github.com/tree-sitter/tree-sitter-html) | Crate con binding Rust y parser C | Compilado y probado con Tree-sitter `0.25.10` |
| [dsbraz/tree-sitter-razor](https://github.com/dsbraz/tree-sitter-razor) | Captura regiones C# opacas; no se encontró un binding Rust preparado en la revisión | Alternativa para separar regiones; sin benchmark |
| [tree-sitter-razor publicado](https://docs.rs/crate/tree-sitter-razor/0.1.0) | Crate de otro fork, enlazado a jamie8johnson | No confundir con jlcrochet; sin benchmark |
| [tree-sitter-blazor](https://docs.rs/crate/tree-sitter-blazor/0.1.3) | Crate Rust; se consultó documentación 0.1.3 | Sin benchmark; la página enumera también 0.2.0, no inspeccionada |
| [RazorSharp](https://github.com/jlcrochet/RazorSharp) | Servidor LSP con .NET/Roslyn/Razor, no biblioteca Rust nativa | Posible complemento semántico, no probado |

No se encontró una biblioteca verificada que ya reúna toda la integración
Razor + HTML + Oxc + resolución MVC.

Los bindings Rust de Tree-sitter no convierten las gramáticas en parsers escritos
íntegramente en Rust: compilan su parser C y lo llaman desde Rust, sin necesitar
Node en ejecución.

### Por qué probar jlcrochet antes de integrar tres parsers

Su [gramática](https://github.com/jlcrochet/tree-sitter-razor/blob/main/grammar.js)
extiende C# y contiene `void_element`, tratamiento especial de script/style y
expresiones Razor. Sus
[queries de inyección](https://github.com/jlcrochet/tree-sitter-razor/blob/main/queries/injections.scm)
identifican contenido JavaScript/CSS. Esas queries no ejecutan Oxc automáticamente:
Harness tendría que obtener las regiones y delegarlas.

Su [manifest](https://github.com/jlcrochet/tree-sitter-razor/blob/main/Cargo.toml)
declara Tree-sitter `0.25.10` en dev-dependencies. Esto favorece el encaje, pero no
sustituye una prueba local. No se confirmó soporte de `@helper` clásico ni la
corrección de expresiones Razor incrustadas dentro de scripts.

## Experimentos ejecutados

Proyecto temporal independiente, Windows, compilación release, Rust `1.99.0`.
Dependencias fijadas: `tree-sitter = 0.25.10`,
`tree-sitter-razor-modern = 0.1.0`, `tree-sitter-html = 0.23.2`.
No se cambiaron las dependencias de Harness ni el índice de web-fast-v1.

El experimento temporal quedó en:
`C:/Users/AbrahamRodriguez/AppData/Local/Temp/harness-razor-bench-30337007f4d9421e90426b8fce8bf447`.
No es un artefacto versionado ni se garantiza su conservación.

### Corrección observada

| Caso sintético | razor-modern | tree-sitter-html |
| --- | --- | --- |
| `@model ExampleModel` y `@Model.Name` | Árbol esperado sin error | No evaluado |
| Atributo con `@Url.ContentVersioned("~/Content/logo.png")` y comillas anidadas | Reconoce la expresión sin error | Error: no entiende Razor |
| `@Url.Action("Edit", "Orders")` en atributo | Reconoce llamada y argumentos | No evaluado separadamente |
| `@functions` y `@foreach` con HTML | Árbol esperado sin error | No evaluado |
| Script con `if (a < b)` | Error | Cuerpo `raw_text` sin error |
| Script con `@Url.Action` dentro de una cadena | Sin error; no equivale a analizar JS | Cuerpo `raw_text` sin error |
| `<input>`, `<br>`, `<img>` sin cierre explícito | Error | Sin error |
| `@helper Row(string name) { ... }` clásico | Sin error, pero no representa el helper correctamente | No evaluado |
| Expresión Razor deliberadamente incompleta | Reporta error | No evaluado |

Un árbol sin `ERROR` no demuestra extracción correcta ni resolución de referencias.

### Rendimiento observado

Todos los datos son parseos completos de textos sintéticos, con reutilización del
parser y sin reutilización de árboles. No son mediciones de edición incremental.

| Parser y entrada | Repeticiones | Tiempo medio por documento |
| --- | ---: | ---: |
| razor-modern, 20.433 bytes con 300 filas de HTML y expresiones Razor | 1.000 | 6,81 ms; 6,76 ms; 9,71 ms en tres ejecuciones |
| razor-modern, 13.528 bytes con 500 líneas JS y comparaciones `<` | 100 | 17,01–17,54 ms, con errores |
| razor-modern, 13.028 bytes con espacios en lugar de cuerpos JS | 100 | 0,161–0,169 ms, sin errores |
| tree-sitter-html, 13.535 bytes con scripts y un input | 1.000 | 0,869 ms |

La entrada con espacios no tiene exactamente el mismo tamaño que la de JS.
Su tiempo excluye localizar/enmascarar scripts y analizar JavaScript con Oxc.
Las cifras excluyen resolución MVC, escritura SQLite y extracción completa del
grafo. No permiten sumar tiempos y afirmar un rendimiento final de la combinación.
La variación entre ejecuciones obliga a medir varias muestras antes de comparar.

## Composición posible

Primera opción a evaluar: **jlcrochet/tree-sitter-razor + Oxc existente**.
Podría evitar una segunda pasada HTML si sus regiones y atributos son suficientes.

Alternativa: **tree-sitter-html + razor-modern + Oxc**. Son complementarios, pero
necesitan coordinar regiones: HTML puro falla con comillas Razor anidadas,
razor-modern falla con ciertos scripts y elementos vacíos, y Oxc no entiende
transiciones Razor. No basta con pasar el archivo completo a cada parser.

En ambas opciones se debe conservar un mapa de posiciones al proyectar código,
separar scripts ejecutables de JSON/plantillas y registrar los casos dinámicos o
ambiguos sin inventar destinos. La resolución MVC existente sigue siendo necesaria:
un AST de llamada no determina por sí solo qué acción del controlador se ejecuta.

## Próxima prueba: debug y después release

Estimación orientativa comunicada: 5–15 minutos para una prueba debug básica si
compila directamente; 20–30 minutos si faltan fuentes generadas o requiere ajustes.
No es una medición ni una garantía de duración.

1. Fijar una revisión de jlcrochet y compilar su binding con Tree-sitter 0.25.10
   en un proyecto aislado. Verificar parser/scanner generados y submódulos.
2. Repetir los casos anteriores, inspeccionando el árbol, especialmente helpers,
   comillas anidadas, etiquetas vacías, scripts, comentarios y caracteres Unicode.
3. Conectar experimentalmente las regiones JS con Oxc, incluyendo Razor dentro
   de cadenas y expresiones JS. Conservar posiciones y fallos parciales.
4. Comparar debug y release bajo la misma entrada y configuración. El proyecto
   Harness tiene optimizaciones en su perfil dev; no compararlo con debug sin
   optimizar de un proyecto temporal como si fueran perfiles equivalentes.
5. Si las vistas reales están disponibles, evaluar las 543 sin modificar su
   índice. Contabilizar fallos y referencias verificables, no solo has_error.
6. Medir separadamente lectura, parseo, extracción, resolución y publicación.
   Registrar mediana/p95 y memoria si se dispone de una medición fiable.

Decidir la integración únicamente después de comprobar cobertura y rendimiento
frente al código actual de Harness. Todavía no se ha hecho ese benchmark comparativo.

## Aclaración sobre los proyectos llamados CodeGraph

- La instalación consultada de `colbymchenry/codegraph` era 1.6.2. En una prueba
  de su extractor con gramáticas cargadas extrajo tipos de directivas y bloques
  C#, pero no referencias del JS embebido ni del ejemplo inline `Url.Action`.
- [Jakedismo/codegraph-rust](https://github.com/Jakedismo/codegraph-rust) es otro
  proyecto. El [registro revisado](https://github.com/Jakedismo/codegraph-rust/blob/main/crates/codegraph-parser/src/language.rs)
  asigna C# a `.cs`, sin registrar `.cshtml`/`.razor`, y su
  [dispatch de extractores](https://github.com/Jakedismo/codegraph-rust/blob/main/crates/codegraph-parser/src/languages/mod.rs)
  no incorpora Razor. No aporta una integración Razor lista para reutilizar.

No trasladar capacidades entre repositorios o forks por compartir el nombre.
