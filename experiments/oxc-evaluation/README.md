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

## Resultado y decisión

**Sí aporta valor; recomendamos reutilizar Oxc para JS/TS, con integración
gradual. Todavía no está conectado al runtime del harness.**

Validación realizada con rustc 1.99.0 en Windows: 14 pruebas ordinarias aprobadas,
una prueba opt-in contra CodeGraph aprobada y Clippy estricto del experimento
aprobado. La prueba opt-in ejecuta Node únicamente como oráculo del proveedor.

| Aspecto | Evidencia | Decisión |
| --- | --- | --- |
| Binding y shadowing | Distingue parámetros/bloques y conserva las referencias al import en otros scopes | Integrar primero el análisis semántico, evitando el rechazo global del guard actual |
| Referencias usadas como valor | Reconoce el alias en un callback JSX | Mapear la referencia al tipo de arista correcto, no convertirla automáticamente en llamada |
| Rutas | Relativos, index, extensiones emitidas, tsconfig paths y exports condicionales aprobados | Reutilizar el resolver con opciones explícitas y el scope del harness |
| Filesystem | Resolver funciona íntegramente sobre un snapshot en memoria | Implementar el adaptador de producción sin permitir lecturas arbitrarias |
| Compatibilidad | Se detectó una diferencia de prioridad `.js`/`.ts` con CodeGraph | No sustituir el resolver sin pruebas diferenciales y política de opciones |

Dos hallazgos concretos:

1. Con `import './lib.js'` y ambos archivos existentes, el perfil inicial de
   extensión de Oxc escogía `lib.ts`; CodeGraph v1.6.2 escogió `lib.js`. Se añadió
   `prefer_existing_js()`, que reproduce ese caso y conserva el fallback a `.ts`
   cuando no existe `.js`. No certifica todas las prioridades de CodeGraph.
2. En la versión evaluada, comprobar solo `SemanticBuilder.errors` no rechazó
   el conflicto entre un import y una constante homónima TypeScript. El adaptador
   experimental también inspecciona `symbol_redeclarations` y lo rechaza. Oxc
   semantic no sustituye un type checker de TypeScript.

Siguiente entrega: integrar bindings semánticos con las referencias/IDs existentes
de CodeGraph, convirtiendo posiciones UTF-8 y conservando rollback y validaciones.
Después, incorporar resolución sobre el inventario autorizado y comparar las
opciones con el proveedor antes de habilitar tsconfig/packages en producción.
Rolldown completo no hace falta para estas piezas.

## Ejecutar

Desde la raíz del harness:

```powershell
cargo test --locked --manifest-path experiments/oxc-evaluation/Cargo.toml
cargo clippy --locked --manifest-path experiments/oxc-evaluation/Cargo.toml --all-targets -- -D warnings
```

Para la comparación opt-in, configurar `HARNESS_CODEGRAPH_PACKAGE` con la raíz
del paquete CodeGraph v1.6.2 y `HARNESS_CODEGRAPH_NODE` con su ejecutable Node;
ejecutar el test `detects_emitted_extension_priority_difference_from_codegraph`
con `-- --ignored`. No es requisito para las 14 pruebas ordinarias.

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
