# Parser architecture

Vue/Angular graph extraction, publication and explicit limitations are described
in [FRAMEWORK-GRAPH.md](FRAMEWORK-GRAPH.md). Structural graph coverage is not
framework compilation or template type checking.

Los parsers de lenguaje viven bajo `src/adapters/parsers/`. Reciben texto en
memoria y producen validación sintáctica o símbolos; no son responsables de
abrir proyectos, recorrer el workspace ni hablar con MCP.

```text
src/adapters/
├── parsers/
│   ├── model.rs                 # reexport del modelo de dominio
│   ├── registry.rs              # ParserRegistry y registro nativo por extensión
│   ├── registry_tests.rs
│   ├── ecmascript/              # AST Oxc y extracción de símbolos JS/TS
│   ├── rust/                    # validación Rust y extracción de símbolos
│   ├── csharp/                  # parser C#
│   ├── vue/                     # parser de sintaxis y estructura Vue
│   ├── razor/                   # sintaxis, render y análisis de enlaces MVC
│   ├── astro/                   # parser Astro
│   ├── css/                     # extracción de símbolos CSS
│   └── scss/                    # símbolos y validación SCSS con Raffia
├── vue_extract.rs               # preview de extracción; usa Project
├── index_razor_mvc.rs           # resolución MVC y snapshot SQLite
└── index_razor_mvc_tests.rs
```

`harness_domain::parsing::Symbol` es el modelo común de declaraciones; el
módulo `parsers::model` solo lo reexporta dentro de los adaptadores nativos para
mantener sus imports existentes. El dominio también clasifica rutas con
`SourceFormat::from_path(&Path)`, una regla pura que usa únicamente la extensión
final, sin leer contenido ni seleccionar una implementación de parser.

La clasificación distingue Rust, JavaScript (`js`, `mjs`, `cjs`, `jsx`),
TypeScript (`ts`, `mts`, `cts`, `tsx`), Vue, Astro, C#, Razor, CSS, SCSS y HTML
(`html`, `htm`), sin distinguir mayúsculas en la extensión. JSX/TSX conservan
su formato propio, mientras que `SourceFormat::language()` los agrupa en los
idiomas de protocolo `javascript`/`typescript`. Clasificar un formato no
significa que exista un parser de validación para él.

SCSS usa el parser `raffia` (`=0.13.0`): `scss/symbols.rs` proyecta el árbol
real (reglas anidadas, variables, mixins, funciones, `@include`, `@use`,
`@forward`, `@extend`, `@each`/`@for`) conservando el rango exacto de origen, y
`scss/syntax.rs` valida sintaxis con posición de línea y columna. La validación
es únicamente sintáctica: no se resuelven módulos Sass ni se evalúan valores,
así que un archivo válido nunca falla por un import no localizado. Los errores
recuperables se reportan como avisos y nunca bloquean una edición; un
`allowUnvalidatedText` explícito tampoco bloquea un candidato válido.

En C#, `csharp/syntax.rs` conserva los puntos de entrada y delega en etapas
separadas: `lexer.rs`, `declarations/symbols.rs`, `razor.rs` y `validation.rs`.
Las pruebas de integración entre etapas están en `csharp/syntax_tests.rs` y
comprueban rangos originales, comentarios, literales, cancelación y límites.

## Contexto de análisis de código

La aplicación de DDD se limita aquí al contexto de análisis de código:
`crates/domain/src/parsing/` contiene el modelo `Symbol` y los valores
`SourceFormat`/`SourceLanguage`. La clasificación es una regla compartida por
la validación y la navegación de símbolos; el dominio no elige motores ni
consulta archivos. En navegación, un `match` exhaustivo por idioma obliga a
decidir cómo tratar cada idioma nuevo en lugar de enviarlo a un parser genérico.

El puerto `SyntaxParser` permanece en aplicación; Oxc, los analizadores nativos
y el registro que los compone permanecen en adaptadores. Los resultados JSON
existentes se conservan. No se introducen agregados ni repositorios adicionales:
la validación de texto no requiere gestionar un ciclo de vida persistente.

## Añadir una estrategia de sintaxis

El puerto de aplicación está en `crates/application/src/ports/parsing.rs`:

```rust
pub trait SyntaxParser: Send + Sync {
    fn supports(&self, path: &str) -> bool;
    fn validate(
        &self,
        path: &str,
        source: &str,
        context: &OperationContext,
    ) -> Result<Value, Failure>;
}
```

Una implementación nativa se define junto a la composición en
`src/adapters/parsers/registry.rs`, delega la validación al parser de lenguaje y
se añade a `default_syntax_registry()`. El `ParserRegistry` recibe referencias a
`dyn SyntaxParser`, selecciona la primera estrategia cuyo `supports(path)` da
verdadero y propaga su error; no intenta otro parser después de un fallo. Esta
inyección permite probar la selección con estrategias falsas sin acoplar el
puerto al parser concreto. El puerto depende del contrato; el registro nativo
inyecta las implementaciones elegidas durante la composición.

Hoy el registro predeterminado incluye Rust (`.rs`), ECMAScript (`.js`, `.mjs`,
`.cjs`, `.jsx`, `.ts`, `.mts`, `.cts`, `.tsx`), Vue (`.vue`), Astro (`.astro`) y
Razor (`.cshtml`). JavaScript, TypeScript, JSX y TSX comparten el parser Oxc de
`ecmascript`; la variante se deriva de la ruta. El módulo C# y el extractor CSS
existen, pero no forman parte de este registro de validación de edición. Otros
formatos siguen bajo sus políticas existentes.

## Límites actuales

- Los módulos de lenguaje en `parsers/` deben trabajar con el contenido recibido;
  no deben depender de `Project`, `Workspace`, transporte, acceso al filesystem
  ni SQLite. `tests/architecture.rs` mantiene esta frontera y requiere que las
  pruebas vivan en archivos externos `*_tests.rs`, declarados desde el módulo
  con `mod tests;` o un nombre explícito como `mod registry_tests;`.
- Las operaciones más amplias siguen en adaptadores nativos: `vue_extract.rs`
  coordina el preview con `Project`; `index_razor_mvc.rs` consulta su snapshot
  SQLite para resolver rutas MVC. Separar esas operaciones evita pasar sus
  dependencias al código de sintaxis, pero la aplicación aún coordina varios
  adaptadores y el modelo de resultados usa `serde_json::Value`.
- El puerto de sintaxis expresa validación, no una API universal para AST,
  extracción de símbolos o reescritura. Esas capacidades siguen expuestas por
  módulos nativos distintos. Añadir una extensión al registro no implica que
  todas las herramientas del harness la soporten.
- `OperationContext` y `Failure` pertenecen a domain y atraviesan el puerto para
  cancelar trabajo y devolver errores consistentes. El ejecutable sigue siendo
  un monolito modular: los parsers son módulos/adaptadores, no procesos ni crates
  desplegables por separado.
