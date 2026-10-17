# Evaluación aislada de Oxc para CodeGraph

Fecha: 2026-10-17. Prueba de concepto, no runtime de producción.

Este crate tiene workspace y lockfile propios. No añade Oxc al ejecutable MCP,
no cambia el extractor CodeGraph y no escribe índices SQLite. Las resoluciones
usan un filesystem sintético en memoria; ni siquiera requieren crear archivos
del proyecto de prueba en disco. No ejecuta Node ni plugins JavaScript.

## Preguntas que comprueba

1. ¿`oxc_semantic` distingue referencias al import de parámetros y variables
   locales con el mismo nombre, conservando referencias válidas en otros scopes?
2. ¿`oxc_resolver` resuelve módulos relativos, índices de directorio, extensiones
   emitidas, aliases de tsconfig y exports condicionales de paquetes?
3. ¿Podemos suministrarle una vista acotada de archivos, en lugar de permitir
   acceso directo al sistema de archivos del usuario?

## Ejecutar

Desde la raíz del harness:

```powershell
cargo test --locked --manifest-path experiments/oxc-evaluation/Cargo.toml
cargo clippy --locked --manifest-path experiments/oxc-evaluation/Cargo.toml --all-targets -- -D warnings
```

## Dependencias y alcance

- Familia parser/semantic de Oxc fijada en `0.75.0`.
- `oxc_resolver` fijado en `11.0.0`.
- Ambas versiones declaran Rust mínimo 1.85; se escogieron sin elevar el Rust
  mínimo 1.88 declarado por el harness. Compilar con el compilador instalado no
  sustituye una comprobación específica en Rust 1.88.
- `codegraph-native` es dependencia de pruebas para comparar el guard conservador
  actual con el análisis de bindings de Oxc. No se cambia el código vendorizado.
- Los offsets de esta prueba son bytes UTF-8. No se han convertido todavía a
  línea/columna UTF-16 ni a IDs del formato CodeGraph.

## Límites de la prueba

El análisis semántico devuelve referencias al binding importado, no determina
por sí solo el destino de una llamada entre archivos. Una referencia puede ser
un valor pasado como callback, y no una llamada directa. El enlazado con nodos,
tipos de arista, exportaciones y metadatos de CodeGraph sigue siendo necesario.

La resolución de rutas utiliza opciones explícitas. No se afirma que sus defaults
coincidan con CodeGraph ni que todas las opciones representen todos los modos de
TypeScript/Node. Elegir `.ts` antes de `.js` difiere de nuestra política nativa
actual de dejar pendientes múltiples candidatos indexados.

El filesystem sintético es una prueba de extensibilidad, no el adaptador seguro
de producción: impide enlaces, lecturas fuera del snapshot y más de 4096 probes,
pero faltan integración con cancelación/deadlines, inventario, permisos, límites
de memoria, hashes y revalidación transaccional del harness. No sustituye sus
controles de rutas ni permite ampliar silenciosamente el scope.

No hay benchmark de rendimiento, evaluación de proyectos reales ni paridad
completa del resolver/frameworks. Rust y C# siguen fuera del alcance de Oxc.

## Documentación de referencia

- [Parser Rust](https://oxc.rs/docs/guide/usage/parser.html).
- [Resolver](https://oxc.rs/docs/guide/usage/resolver.html).
- [API semantic 0.75.0](https://docs.rs/oxc_semantic/0.75.0/oxc_semantic/).
- [API resolver 11.0.0](https://docs.rs/oxc_resolver/11.0.0/oxc_resolver/).
