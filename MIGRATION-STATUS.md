# Estado de la migración a Rust

## Punto de control

- Rama: `migration/rust-harness`.
- Núcleo: `.agents/harness/native/core`, versión del paquete `0.7.0`.
- Objetivo: distribuir un ejecutable MCP sin Node ni node_modules en ejecución.
- Estado: migración parcial; Node sigue siendo la conexión activa para funciones pendientes.
- Este documento registra lo implementado y el trabajo restante antes de continuar.
- No existe todavía una medición fiable del porcentaje por esfuerzo; contar herramientas no mide paridad.

## Arquitectura disponible

Monolito modular con tres crates:

1. `harness-domain`: contratos, resultados, edición, navegación y fases de trabajo.
2. `harness-application`: casos de uso, puertos, admisión y coordinación de sesiones.
3. `agent-harness`: adaptadores nativos, archivos, procesos, persistencia y transporte MCP.

Hay pruebas que restringen las dependencias de los crates portables. El dominio no
depende del SDK MCP, Tokio ni APIs del sistema operativo. No hacen falta
microservicios para escalar las capacidades de este ejecutable.

## Funciones implementadas

| Herramienta / bloque | Estado actual | Límites importantes |
| --- | --- | --- |
| MCP stdio y CLI | Implementado | Entrada acotada; ejecutable experimental |
| `harness_bootstrap` | Implementado | Sesión, reglas, capacidades y guía de uso |
| `harness_read` | Implementado | UTF-8, hash SHA-256, hasta 100 líneas por fragmento |
| `harness_detail` | Implementado | Caché acotada, paginado UTF-16, referencias locales al proceso |
| `harness_search` | Parcial | Texto literal y archivos; faltan syntax y file_size |
| `harness_verify` | Parcial | Rust build/test mediante Cargo; faltan otros lenguajes |
| `harness_replace` | Parcial | Publicación segura en Windows; Rust/JSON/Markdown/texto |
| `harness_outline` | Parcial | Declaraciones Rust; sin relaciones semánticas ni expansión de macros |
| `harness_read_symbol` | Parcial | Rust, ambigüedad y detección de symbolId obsoleto |
| `harness_tasks` | Implementado | Fases, transiciones y timeline acotado en memoria |
| `harness_checkpoint` | Parcial | Objetivo, decisiones, pendientes e histórico; sin refreshFreshness |
| Sesiones en disco | Implementado | Formato v1 compatible con Node; histórico no equivale a vigencia |
| Procesos nativos | Implementado | Windows Job Objects, cancelación y límites; Unix con garantías menores |
| Huella antes/después de Cargo | Parcial | Detecta cambios locales observados; no certifica vigencia completa |

### Seguridad y coordinación existentes

- Validación de rutas; rechazo de traversal, secretos, enlaces y reparse points.
- Lecturas, búsquedas, resultados y procesos con presupuestos acotados.
- Admisión de operaciones y leases que conservan las sesiones mientras trabajan.
- Exclusión de ediciones concurrentes y gate de escritura/verificación por sesión.
- Guardado de snapshots fuera del mutex global del repositorio.
- Reemplazos con hash, validación previa, archivo temporal y verificación posterior.
- Preservación de DACL en Windows y artefactos de recuperación ante publicación incierta.
- Un resultado incierto no se presenta como archivo intacto ni edición confirmada.

### Último bloque implementado

`workspaceObservation` compara rutas/contenido SHA-256 antes y después de Cargo:
`unchanged`, `changed` o `unknown`. Incluye ocultos/ignorados, salvo salidas y
directorios excluidos documentados en README. Límites: 10000 entradas, 64 MiB,
2 MiB por archivo y 5 segundos cooperativos por observación.

No rastrea dependencias externas, entorno ni toolchain. Tampoco detecta todos los
cambios intermedios revertidos. `verificationFreshness` sigue `not_tracked` y los
checks mantienen `current=null`. Las observaciones se conservan en la sesión viva,
no en el formato persistido v1. `refreshFreshness=true` sigue sin soportarse.

## Verificación realizada antes de este documento

- 66 pruebas Rust pasan en la ejecución final secuencial.
- Clippy de todo el workspace y todos los targets: sin warnings.
- Formato Cargo comprobado.
- Compilación release completada con las últimas modificaciones.
- Dos pruebas de procesos fallaron al solaparse con la compilación optimizada;
  pasaron sin esa carga. Pendiente sustituir esperas temporales frágiles por
  sincronización determinista; no ocultar esta limitación.
- Las dos pruebas de compatibilidad con Node pasaron en la etapa 0.7.0 anterior;
  no se volvieron a ejecutar en el último bloque de huellas.
- El diagnóstico del arnés reportó un error en la comprobación del índice;
  no debe interpretarse como índice validado ni conexión restablecida.

## Pendiente de migrar o completar

### 1. Vigencia y observabilidad

- [ ] Seguimiento de versiones del workspace y cobertura explícita por verificación.
- [ ] Invalidación de checks por cambios posteriores y tratamiento del estado restaurado.
- [ ] `refreshFreshness`, incluida coordinación con verificaciones activas.
- [ ] Métricas y estadísticas reales, sin valores simulados.
- [ ] Diagnóstico nativo equivalente a `harness_doctor`.
- [ ] Logs acotados y consulta equivalente a `harness_logs`.
- [ ] Estabilizar pruebas temporizadas de procesos bajo carga.

### 2. Herramientas de repositorio

- [ ] `harness_git_status` y `harness_git_diff` nativos de solo lectura.
- [ ] `harness_batch` con límites y cancelación coherentes.
- [ ] Configuración nativa y redacción de secretos.
- [ ] Alcances externos autorizados y su validación.
- [ ] Búsqueda AST (`syntax`) y tamaño (`file_size`).
- [ ] Navegación, validación y edición para los demás lenguajes requeridos.
- [ ] Verificación de .NET, Go, Python, JavaScript y otros adaptadores usados.

### 3. Relaciones e integraciones

- [ ] Integración CodeGraph para relaciones y flujos mediante `harness_explore`.
- [ ] SQL Server: configuración segura, estado, catálogo, descripción y consultas auditadas de solo lectura.
- [ ] Integrar o sustituir las utilidades XLSX existentes desde el núcleo Rust.
- [ ] Inventariar y migrar utilidades SVG, navegador/regresión web y otras herramientas auxiliares.
- [ ] Auditar el catálogo publicado de Node frente al catálogo Rust antes de declarar paridad.

Que una utilidad auxiliar ya tenga un ejecutable Rust no significa que su
herramienta MCP esté integrada en este núcleo.

### 4. Distribución y retirada de Node

- [ ] Pruebas de compatibilidad por herramienta y errores, no solo camino exitoso.
- [ ] Pruebas de instalación y ejecución en una máquina sin Node/npm.
- [ ] Definir plataformas soportadas y garantías de procesos/edición por plataforma.
- [ ] Empaquetado reproducible, documentación de instalación y actualización.
- [ ] Revisar licencias, dependencias y tamaño final del artefacto distribuible.
- [ ] Cambiar la conexión MCP solo después de cumplir los criterios de aceptación.
- [ ] Mantener un procedimiento de reversión comprobado.
- [ ] Retirar Node y node_modules únicamente cuando no quede ninguna dependencia funcional.

El ejecutable MCP puede ser autónomo sin Node, pero verificar proyectos sigue
requiriendo sus toolchains (por ejemplo, Cargo/Rust) y puede ejecutar código del
proyecto. No es un sandbox para código hostil.

## Orden recomendado para continuar

1. Completar vigencia del código y estabilizar pruebas de procesos.
2. Añadir métricas, doctor y logs.
3. Migrar Git, batch, configuración y alcances externos.
4. Completar búsqueda, parsers y verificadores por lenguaje.
5. Integrar CodeGraph, SQL y utilidades restantes según uso real.
6. Ejecutar paridad integral, empaquetar y probar sin Node.
7. Cambiar la conexión y retirar dependencias antiguas con reversión disponible.

## Criterio de finalización

La migración termina cuando todas las capacidades requeridas tienen reemplazo
probado y documentado, el cliente usa el ejecutable Rust, y las pruebas de
distribución confirman que Node/node_modules ya no son necesarios. Un build
exitoso o tener diez herramientas nativas no demuestra por sí solo esa paridad.
