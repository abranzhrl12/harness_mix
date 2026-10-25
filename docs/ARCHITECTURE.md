# Arquitectura y desarrollo

[Inicio](../README.md) · [Referencia](REFERENCE.md) · [Rendimiento](PERFORMANCE.md)

## Monolito modular y fronteras

~~~text
src/main.rs / cli.rs / transport/runtime.rs   composición y ciclo de vida
                    |
crates/mcp/src                               servidor MCP y traducción de protocolo
                    |
crates/application                          casos de uso y puertos
                    |
crates/domain                               contratos y reglas portables

src/adapters                                implementaciones nativas de puertos
~~~

| Ubicación | Responsabilidad |
| --- | --- |
| `crates/domain` | Contratos, identidad, políticas de edición, frescura, resultados y estados de trabajos |
| `crates/application` | Admisión, sesiones, coordinación y puertos; sin SDK MCP, Tokio ni drivers |
| `src/adapters` | Filesystem seguro, parsers, SQLite, SQL, procesos y persistencia |
| `crates/mcp` | SDK MCP, perfiles, respuesta, entrada acotada, jobs y cancelación |
| `crates/config` | Opciones tipadas y validación del entorno de arranque; no construye servicios |
| `crates/skills` | Catálogo, sincronización segura y preferencias de skills; sin CLI ni MCP |
| `crates/jobs` | Scheduler, cancelación, eventos y persistencia durable; ejecutor de verificaciones inyectado |
| `src/transport/runtime.rs` | Fachada de arranque; construcción en `runtime/single.rs` y `workspace.rs` |
| `src/transport/runtime/coordinator.rs` | Limpieza ante fallos de inicio, espera del servidor, cancelación y drenaje |

Los puertos viven en `application`; `crates/mcp` depende de `application` y `domain`, mientras que los adaptadores nativos permanecen en el binario. `Application<W,S>` recibe workspace y repositorio de sesiones. El dispatcher conserva un único match exhaustivo y la envoltura común; los módulos `dispatch/` reúnen políticas de consultas, edición, sesiones, SQL, artefactos y diagnósticos.

Los handlers de `application/handlers/` reciben únicamente la capacidad que utilizan: `SourceQueries`, `SourceEditing` o `Verification`, sin el workspace compuesto ni la aplicación completa. `dispatch/` les entrega las operaciones después de adquirir los leases y gates de sesión y conserva el registro de resultados. La indexación mantiene sus casos de uso genéricos existentes en `application/indexing`, sin un wrapper adicional.

`application/policies.rs` concentra autorización por perfil, creación del contexto con el plazo de la petición y comprobación de resultados de lectura. Cancelación y progreso siguen en `OperationContext`, que llega sin reemplazos a los adaptadores; las validaciones específicas y la seguridad de rutas permanecen en su frontera correspondiente. Un resultado parcial explícito se conserva y una mutación confirmada no se reclasifica como cancelada al volver. Los transportes entran por `Application`: llamar directamente un handler no aplica autorización, admisión ni políticas de sesión.

`NativeWorkspace` compone `SourceQueryService`, `SourceEditingService` y `SessionService`. Consultas sincronizadas y ediciones comparten un único bloqueo. Scopes externos comparten registro, propietario, TTL y pines durante la lectura. `sessions/repository.rs` implementa el repositorio de sesiones; persistir snapshots no sustituye el gate por sesión ni los leases de aplicación.

`JobService` y `JobEventStream` desacoplan MCP del scheduler y de Tokio. El runtime inyecta el gestor concreto y drena los trabajos al cerrar.

`harness-jobs` no conoce los verificadores de lenguajes ni el servidor MCP. El adaptador nativo prepara y ejecuta verificaciones mediante un contrato inyectado; el motor controla concurrencia, reintentos, cancelación y estados durables. La persistencia conserva su formato, exclusión de escritor y pines de archivos/directorios; la extracción no migra ni borra el historial existente.

El coordinador de ciclo de vida drena los gestores incluso si falla la creación de la aplicación o el handshake. En modo workspace se cancela a todos los gestores antes de esperar, y se intenta drenar cada uno aunque otro exceda su plazo. Los propietarios de watchers permanecen vivos hasta terminar esa limpieza; los errores del servicio y del drenaje se conservan juntos cuando ambos fallan.

La configuración de arranque se resuelve en `harness-config` y se entrega como valores al runtime. No es un contenedor global de servicios: MCP, jobs y adaptadores conservan contratos específicos. La configuración especializada de SQL, las políticas del índice y la inspección de archivos de configuración de proyectos siguen en sus adaptadores; no se cambia su precedencia ni su política de secretos.

`harness-skills` posee el catálogo y el motor de sincronización, incluidos manifiesto, conflictos y preferencias. El CLI conserva la presentación y la inicialización del proyecto (ejemplos de configuración y MCP). Ninguno de esos crates depende del ejecutable, del CLI o del servidor MCP. Se separan por responsabilidad, sin imponer capas DDD artificiales a utilidades de infraestructura.

En ambos crates, `lib.rs` declara módulos y expone la fachada pública, sin implementar servicios. Configuración separa la lectura del entorno de las opciones y su validación. Skills separa catálogo, sincronización, manifiesto, preferencias, almacenamiento seguro, bloqueo e informes; los detalles internos no se exportan por comodidad.

## Parsers y límites

Los parsers reciben texto en memoria; la lectura de archivos y las relaciones del índice pertenecen a adaptadores. `SyntaxParser` y `ParserRegistry` permiten seleccionar estrategias sin un trait por helper.

| Familia | Implementación y alcance |
| --- | --- |
| Rust | `ra_ap_syntax`: sintaxis y símbolos; resolución entre módulos acotada |
| JavaScript / TypeScript / JSX / TSX | Oxc; las relaciones importadas usan bindings y rutas del índice |
| C# | Parser/walkers nativos; no sustituye a Roslyn ni a MSBuild |
| Razor | Declaraciones y bloques C# proyectados; enlaces MVC literales y comprobaciones básicas de markup |
| Vue | Estructura SFC, scripts y metadatos de componentes; no compila templates |
| Angular | Metadatos/relaciones sobre TS y HTML; no ejecuta Angular ni comprueba tipos de templates |
| Astro | Estructura y sintaxis soportadas por su adaptador |
| CSS / SCSS | Símbolos; SCSS usa Raffia y validación sintáctica, sin resolver módulos Sass |

El soporte varía por operación; no se promete que cada formato tenga typecheck, compilación y resolución completa. Cualquier campo de cobertura parcial es parte del contrato.

El índice usa un subconjunto fijado del kernel CodeGraph 1.6.2 en `vendor/codegraph-native`. Las extensiones Angular/Vue conservan procedencia y los avisos MIT de ese kernel. La resolución JS/TS usa Oxc con una vista acotada del índice; aliases de tsconfig y reexports tienen límites explícitos y no habilitan resolución arbitraria de paquetes.

## Persistencia y concurrencia

- `index_store` conserva ownership de conexión, pines y transacciones por archivo. No hay rollback global de un lote que ya confirmó otros archivos.
- `publication/plan.rs` decide reutilización y recuperación de referencias sin I/O; `model.rs` prepara vistas tipadas; `storage.rs` persiste en la transacción recibida.
- Los hashes de fuentes/configuración se revalidan antes del commit. `committed:true` no implica frescura global ni paridad completa con CodeGraph.
- La fachada SQL elige el motor. SQL Server y PostgreSQL tienen backends separados; la presentación SQL recibe datos sin acceder al driver.
- El mantenimiento automático del índice usa eventos agrupados (600 ms, máximo 2 s), una cola de hasta 1024 rutas y reconciliación cada 300 s. Esos intervalos no son tiempos garantizados de indexación.

## Pruebas y distribución

~~~powershell
cargo test --locked --workspace
cargo test --locked --test architecture
cargo build --release --locked
powershell -File scripts/package-release.ps1
~~~

La suite de arquitectura protege las dependencias portables, presenters sin I/O, parsers y delegación de capacidades. Las pruebas de comportamiento cubren cambios concurrentes, rollback, cancelación, scopes, sesiones, SQL y protocolo. Los tests ignorados requieren herramientas, servidores o benchmarks opt-in; una comprobación de sintaxis no reemplaza esas pruebas.

`scripts/package-release.ps1` produce ZIP y checksum en `target/dist`; conserva avisos de terceros. `release/README.md` es la guía incluida en ese ZIP. El lanzador npm y su README están en `npm-dist/`.

Para medir sin tocar un índice real:

~~~powershell
node scripts/benchmark-tools.cjs target/release/agent-harness.exe target/benchmarks/tools.json 7
powershell -File scripts/benchmark-framework.ps1 -Profile release -Iterations 3 -Rounds 2
~~~

## Experimentos y procedencia

`experiments/` conserva workspaces aislados y sus instrucciones reproducibles; no son backends activados por el mero hecho de compilar. El piloto RustCodeGraph no está habilitado en producción y no permite mezclar sus IDs/esquemas con el índice existente. Sus resultados registrados permanecen en `docs/evaluations/`. Las comparaciones históricas de parsers conservan sus baselines congelados.

Los reportes y planes de implementación anteriores se retiraron de la documentación de uso. Se conservan en Git, junto con los JSON de benchmarks/evaluaciones que siguen siendo útiles. No elimines licencias de `vendor/`, avisos de Rolldown ni instrucciones `AGENTS.md` al limpiar documentación.
