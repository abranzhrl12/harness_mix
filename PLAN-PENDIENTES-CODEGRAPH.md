# Plan de trabajo pendiente: migración nativa de CodeGraph

Fecha: 2026-10-17. Estado: plan pendiente, no funcionalidades ya terminadas.

Estado implementado y limitaciones: [MIGRACION-CODEGRAPH.md](MIGRACION-CODEGRAPH.md).
Este documento indica qué hacer a continuación y qué reutilizar para evitar
portar manualmente capacidades que ya ofrecen bibliotecas Rust.

## Objetivo

Completar el alcance acordado de consulta y sincronización compatible con
CodeGraph sin Node como coordinador. Conservar el formato SQLite, la procedencia
de relaciones, la seguridad de rutas y las garantías de publicación existentes.

No construir otro grafo desde cero. No prometer paridad total por completar una
lista de herramientas. Separar siempre compatibilidad demostrada de heurísticas.

## Base que NO debemos rehacer

- Núcleo Rust de CodeGraph v1.6.2: extracción de Rust, C#, JS, JSX, TS y TSX.
- Planificador, inventario acotado y consulta SQLite existentes.
- Escritura transaccional por archivo, rollback, FTS y recuperación de referencias.
- Oxc semantic 0.90.0: bindings y posiciones de llamadas directas a imports.
- Oxc resolver 11.0.0: rutas relativas sobre una vista cerrada del índice.
- Pruebas actuales de publicación, seguridad y comparación con el proveedor.

La escritura sigue limitada a Windows, índice existente schema 11/WAL. La
conexión MCP Node no se ha sustituido y no hay certificación de vigencia global.

## Bibliotecas y código a reutilizar

| Necesidad | Base preferida | Trabajo que sigue siendo nuestro |
| --- | --- | --- |
| Aliases, paths, baseUrl y configuración de módulos | `oxc_resolver` | Exponer configuración autorizada, fijar opciones y probar su comportamiento en la versión fijada |
| Imports, exports y reexports JS/TS | AST y registros de módulos de Oxc | Enlazar símbolos con IDs CodeGraph, seguir cadenas y detectar ciclos/ambigüedad |
| Binding local y shadowing | `oxc_semantic`, ya integrado parcialmente | Ampliar tipos de referencias sin confundir callbacks con llamadas |
| Patrones de frameworks | Reglas originales de CodeGraph; evaluar `ast-grep` si simplifica su expresión | Portar y validar reglas concretas, sus dependencias y su invalidación |
| Extracción multilenguaje | Kernel y gramáticas actuales de CodeGraph | Completar el alcance requerido sin cambiar IDs o ABI accidentalmente |

Rolldown completo no es necesario para estas piezas. SWC es una alternativa,
no una dependencia adicional por defecto: incorporarlo exigiría demostrar una
ventaja concreta que Oxc no cubra. No instalar `ast-grep` ni cambiar gramáticas
sin revisar compatibilidad, mantenimiento, licencia y coste de dependencias.

Las capacidades de upstream no prueban que estén habilitadas o sean correctas
en nuestro adaptador ni en nuestras versiones fijadas.

## Orden de ejecución

### 0. Fijar alcance y matriz de compatibilidad

- [ ] Inventariar lenguajes y frameworks realmente usados por el proyecto original, con lecturas autorizadas.
- [ ] Separar una primera entrega útil para ese proyecto de paridad amplia multilenguaje.
- [ ] Enumerar reglas pendientes de CodeGraph por lenguaje; no confundir resolver JS/TS con Rust o C#.
- [ ] Crear fixtures pequeños y resultados esperados del proveedor antes de ampliar cada regla.
- [ ] Registrar diferencias conocidas y qué situaciones deben permanecer pendientes.

Terminado cuando exista una matriz de casos y un alcance explícito. Las cifras
de horas y porcentajes comunicadas anteriormente son estimaciones preliminares,
no una planificación validada; revisarlas con esa matriz y trabajo medido.

### 1. Aliases básicos de tsconfig — perfil inicial implementado

- [x] API y comportamiento manual comprobados mediante compilación/fixtures con Oxc 11.0.0, sin actualizar versión.
- [x] Lectura de tsconfig explícito mediante Project/pins, sin acceso libre de Oxc al disco.
- [x] Snapshot sanitizado y acotado; JSONC/BOM, comentarios y comas finales.
- [x] paths exactos/wildcard/destinos alternativos y baseUrl como ancla relativa al declarante.
- [x] Límite del workspace y restricciones de rutas en configuración y destino.
- [x] Configuraciones no soportadas rechazadas; no fallback a homónimos globales.
- [x] Dependencias/hashes en metadata y aristas, con invalidación al cambiar configuración.
- [x] Revalidación de config/fuentes antes del commit y rollback.
- [x] Rebinding por cambios/eliminación de config aunque no cambien las fuentes.
- [x] Comparación opt-in ejecutada con CodeGraph v1.6.2: seis casos de aliases/JSONC/herencia/overrides, sin afirmar paridad global.

Perfil y límites: [TSCONFIG-PATHS.md](TSCONFIG-PATHS.md). JSONC y extends local
simple están implementados; falta compatibilidad diferencial amplia y selección por importador.

Pruebas mínimas: alias exacto y wildcard, múltiples destinos, baseUrl, prioridad
frente a relativos, destino ausente, configuración inválida o modificada durante
la publicación, rutas sensibles, enlaces y escapes del workspace.

Terminado cuando los casos soportados coincidan con el proveedor y los fallos de
configuración no publiquen un resultado aparentemente completo.

### 2. Configuración heredada y límites entre proyectos

- [x] `extends` local simple: ciclos, hasta 8 archivos, 256 KiB por archivo y 1 MiB agregado; sin arrays/paquetes.
- [x] Herencia y reemplazo de paths/baseUrl comprobados con fixtures de CodeGraph v1.6.2 para el perfil simple.
- [ ] Añadir selección de configuración por importador y proyectos anidados.
- [ ] Evaluar project references y tsconfigs heredados de paquetes solo después de definir su alcance autorizado.
- [x] Revalidar todas las configuraciones contribuyentes; ciclos/límites/padres ausentes provocan rollback, no configuración vacía.

Terminado cuando la selección e invalidación entre proyectos esté cubierta por
fixtures diferenciales, sin leer configuraciones de padres externos por defecto.

### 3. Exports, reexports e imports restantes

- [x] Integrar ModuleRecord y bindings semánticos de Oxc 0.90.0, sin parser duplicado.
- [x] Exports locales con alias y reexports con nombre para funciones implementadas/direct calls.
- [x] Cadenas de hasta 16 pares archivo/nombre, con ciclos pendientes y límites que provocan rollback.
- [ ] Resolver `export *` sin inventar un ganador entre símbolos ambiguos.
- [ ] Añadir default y namespace de forma diferenciada; no convertir imports type-only en llamadas runtime.
- [ ] Mapear referencias usadas como valor/callback a su tipo de arista correcto.
- [x] Conservar refName/refKind y hashes de la cadena para recuperar referencias al renombrar/eliminar destinos.
- [x] Invalidar/reintentar imports nativos al publicar JS/TS; Explore incluye hashes de los barrels.

Perfil acotado y evidencia: [NAMED-REEXPORTS.md](NAMED-REEXPORTS.md).
Tres fixtures comparados con indexación completa de CodeGraph v1.6.2; no paridad global.

Pruebas mínimas: barrel simple, alias encadenados, export default, namespace,
colisiones, ciclos, tipos, altas/bajas y desplazamientos de líneas.

Terminado cuando los destinos y tipos de arista coincidan con el alcance probado,
no simplemente cuando baje el contador de referencias pendientes.

### 4. Paquetes y resolución avanzada

- [ ] Definir si se necesitan paquetes internos del monorepo, node_modules o ambos.
- [ ] Exponer solo manifests autorizados en el filesystem cerrado del resolver.
- [ ] Fijar y comparar condiciones de `exports`/`imports`, ESM/CJS y extensiones.
- [ ] Distinguir una dependencia externa conocida de un símbolo local resoluble.
- [ ] Definir política de dependencias de paquetes y su invalidación.
- [ ] Mantener fuera del alcance descargas, instalación de paquetes y ejecución de plugins.

Terminado cuando los paquetes del alcance elegido se resuelvan de forma
reproducible sin ampliar silenciosamente los permisos de lectura o ejecución.

### 5. Frameworks y relaciones avanzadas

- [ ] Priorizar frameworks de la matriz del paso 0, no todos los existentes.
- [ ] Reutilizar reglas y fixtures de CodeGraph respetando licencia y procedencia.
- [ ] Probar `ast-grep` solo si facilita patrones concretos frente al AST actual.
- [ ] Portar detección de rutas, componentes, registros e inyección según el alcance.
- [ ] Conservar `synthesizedBy`, archivos contribuyentes e información de registro.
- [ ] Invalidar la relación si cambia el archivo que la registra, aunque no cambie ninguno de sus extremos.
- [ ] Completar receptores, herencia, sobrecargas y visibilidad por lenguaje según prioridad real.
- [ ] Mantener el rechazo actual de síntesis no soportada hasta poder actualizarla sin pérdida.

Oxc no resuelve por sí mismo la semántica de Rust o C#. No asumir que una
biblioteca de patrones conoce las reglas de frameworks que debemos reproducir.

### 6. Ciclo de vida completo y retirada de Node

- [ ] Definir creación inicial del índice y política de versiones/migraciones de esquema.
- [ ] Completar reglas de `codegraph.json`, inventario y cobertura del proveedor para el alcance acordado.
- [ ] Diseñar el reprocesamiento al actualizar el motor: hoy un archivo sin cambios puede conservar el no-op.
- [ ] Integrar la sincronización en MCP con política explícita de escritura, cancelación y reporte de commits parciales.
- [ ] Mantener separados commit confirmado, cobertura comprobada y vigencia global.
- [ ] Documentar reconstrucción/recuperación y retorno al proveedor; una reindexación completa puede ser necesaria.
- [ ] Probar una copia del proyecto original antes de modificar su índice real.
- [ ] Sustituir la conexión Node activa únicamente después de aprobar el alcance y la validación.

Portabilidad de escritura fuera de Windows y lenguajes adicionales requieren una
decisión de alcance; no presentarlos como implementados por terminar JS/TS.

## Validación obligatoria en cada entrega

- [ ] Pruebas unitarias del caso y de sus fallos/ambigüedades.
- [ ] Pruebas de publicación y rollback con índices temporales.
- [ ] Comparación con CodeGraph en fixtures del alcance modificado.
- [ ] Regresión de rutas sensibles, enlaces, presupuestos y cambios concurrentes.
- [ ] Suite completa con `cargo test --workspace --locked`.
- [ ] Formato y `git diff --check`.
- [ ] Clippy estricto; distinguir avisos nuevos de los existentes ajenos al bloque.
- [ ] Actualizar estado, límites y evidencia antes de marcar una tarea terminada.

Node puede usarse como oráculo en pruebas opt-in del proveedor. Eso no debe
introducir una dependencia Node en ejecución del harness nativo.

## Criterio final de cierre

La migración del alcance elegido termina cuando extracción, resolución,
configuración, actualización e invalidación tienen evidencia de compatibilidad;
la recuperación está probada y el flujo real funciona sin el coordinador Node.
Hasta entonces, llamarla sincronización nativa parcial.

## Referencias para la implementación

- [Estado actual](MIGRACION-CODEGRAPH.md).
- [Evaluación Oxc y diferencias detectadas](experiments/oxc-evaluation/README.md).
- [Procedencia del kernel reutilizado](vendor/codegraph-native/UPSTREAM.md).
- [Oxc resolver: configuración y opciones](https://github.com/oxc-project/oxc-resolver/blob/main/README.md).
- [Oxc: registros de módulos](https://github.com/oxc-project/oxc/blob/main/crates/oxc_syntax/src/module_record.rs).
- [ast-grep: búsqueda estructural](https://ast-grep.github.io/guide/introduction).
- [CodeGraph: resolución y frameworks](https://colbymchenry.github.io/codegraph/core-concepts/resolution/).
