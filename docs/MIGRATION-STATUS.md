# Estado de la migración a Rust

Resumen específico de CodeGraph, estado actual y checklist pendiente:
[MIGRACION-CODEGRAPH.md](MIGRACION-CODEGRAPH.md).

## Punto de control

- Rama: `migration/rust-harness`.
- Núcleo: raíz de este repositorio independiente, versión del paquete `0.7.0`
  (ubicación anterior: `.agents/harness/native/core`).
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
| `harness_search` | Parcial | Text/files/file_size y syntax de declaraciones Rust; faltan patrones AST generales |
| `harness_config` | Parcial | Sección XML/JSON redactada; no settings runtime ni valores no sensibles |
| `harness_external_scope` | Parcial | Windows: archivo/carpeta, sesión, TTL, identidad y revocación; sin ejecución |
| `harness_verify` | Parcial | Rust/Go/Python/JS/.NET SDK; .NET .csproj/.sln sin restore; falta legacy y paridad |
| `harness_replace` | Parcial | Publicación segura en Windows; Rust/JSON/Markdown/texto |
| `harness_outline` | Parcial | Declaraciones Rust; sin relaciones semánticas ni expansión de macros |
| `harness_read_symbol` | Parcial | Rust, ambigüedad y detección de symbolId obsoleto |
| `harness_explore` | Parcial | Hasta cuatro saltos; hashes de archivos contribuyentes y modo estricto; sin sync nativo ni vigencia integral certificada |
| `harness_tasks` | Implementado | Fases, transiciones y timeline acotado en memoria |
| `harness_checkpoint` | Parcial | Objetivo, decisiones, pendientes e histórico; refresh e invalidación local acotados |
| `harness_git` | Implementado | `action=status` paginado y `action=diff` por archivo, staged/worktree y eliminados; solo lectura, sin filtros externos; admite worktrees vinculados registrados |
| `harness_doctor` | Implementado | Diagnóstico local, versiones opcionales y conectividad real SQL Server (`connection=true`) integrada |
| `harness_batch` | Implementado | 1..4 lecturas concurrentes; soporta archivos, símbolos, Git, herramientas SQL y XLSX |
| `harness_sql` | Implementado | TDS nativo directo con Tiberius; una herramienta con `action` = `status`, `search`, `describe` o `query`, de solo lectura auditada, tipos extendidos (Decimal/DateTime) y truncamiento acotado |
| `harness_xlsx_*` | Implementado | Inspección (`harness_xlsx_inspect`) y lectura paginada/acotada (`harness_xlsx_read`) nativa con calamine y SHA-256 |
| harness_svg | Implementado | Modos inspect y preview (rasterizacion PNG con resvg), seguridad y cuotas |
| harness_handover | Implementado | Renderizado puro en dominio de resumen/ADR/PR, preview acotado a 1600 caracteres o publicacion atomica con control SHA-256 |
| `harness_process_status` | Implementado | Inspección de estado PID, sondeo seguro de puertos TCP y healthcheck loopback HTTP(S) sin shell ni leaks |
| `harness_diagnostics` | Implementado | Diagnóstico sintáctico rápido por lotes en Rust (syn), JS/TS (oxc) y JSON, con soporte de cambios Git tracked |
| `harness_data_query` | Implementado | Filtrado, slicing y proyecciones estructuradas ultrarrápidas con motor nativo JMESPath embebido (RFC/estándar AWS), sin procesos externos |
| `harness_logs` | Parcial | 512 eventos, requestId y snapshots Windows opt-in; sin configuración/trazas completas |
| Métricas | Parcial | Operaciones, resultados, rechazos y tiempos por instancia/herramienta; no histórico por sesión |
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

### Último bloque: export-star sobre Oxc, sin incorporar el bundler

`export *` enlaza funciones originales, deduplica diamantes y deja ambiguos/ciclos
sin destino como pendientes. Los exports explícitos tienen prioridad. Se verifican
también hashes de ramas sin coincidencia; mutación, presupuesto o fallo de fuente
no publican un ganador supuesto. Adapter acotado de las reglas de Rolldown/ECMAScript,
con licencia preservada; parser/resolver Oxc y dependencias del runtime sin cambios.
Matriz de 11 casos comparada con Node ESM y bundles de Rolldown, más seis fixtures
positivos de CodeGraph v1.6.2. `export type *` sigue diferido por el kernel.
No se habilitan default/namespace ni variables-función. Ver [STAR-REEXPORTS.md](STAR-REEXPORTS.md).
Suite completa, Clippy estricto, formato y build release aprobados; siete tests
del perfil star pasaron incluyendo oráculos y CLI release con PATH vacío.

### Bloque anterior: exports/reexports nombrados con Oxc

ModuleRecord y bindings de Oxc 0.90.0 reutilizados para exports locales con alias,
reexports nombrados y cadenas de funciones. Resolución de archivos con el Oxc
cerrado existente; hasta 16 pares archivo/nombre, fuentes verificadas y rollback.
Aristas conservan refName/refKind y hashes de los archivos intermedios; cambios
de barrels provocan rebinding y Explore comprueba su vigencia. Los índices mínimos
sin metadata de aristas siguen siendo legibles. Tres fixtures contrastados con
indexación completa de CodeGraph v1.6.2. No incluye export *, default/namespace,
callbacks ni variables exportadas como función. Ver [NAMED-REEXPORTS.md](NAMED-REEXPORTS.md).
Validación final: suite completa del workspace, Clippy estricto y build release
aprobados; fixture CLI del ejecutable release con PATH vacío también aprobado.

### Bloque anterior: paths/baseUrl explícitos y vigencia de configuración

Perfil Windows de tsconfig JSONC seleccionado por CLI, leído con límites
y pins y entregado a Oxc como snapshot sanitizado, sin filesystem del SO. Soporta
paths exactos/wildcard/destinos alternativos y baseUrl relativo al declarante.
Hashes en metadata/aristas, revalidación antes de commit, invalidación/rebinding
sin cambios de fuente y comprobación de configuración al explorar relaciones.
Incluye extends local simple, ciclos, hasta 8 configuraciones/1 MiB agregado,
hashes heredados, invalidación y rollback al cambiar un padre antes del commit.
Diez pruebas tsconfig pasaron, incluida comparación con CodeGraph v1.6.2 en seis
casos de aliases/JSONC/herencia/overrides. Clippy estricto pasó después de corregir
cuatro avisos SQL/XLSX sin cambiar su comportamiento.
No se habilitan bare packages por baseUrl, proyectos anidados, references,
herencia de paquetes/arrays ni reexports. Ver [TSCONFIG-PATHS.md](TSCONFIG-PATHS.md).
La evidencia es específica de fixtures, no certificación de paridad global.

### Bloque anterior de runtime: Oxc resolver sobre snapshot del índice

`index_paths.rs` integra oxc_resolver 11.0.0 en el escritor nativo Windows. Usa
un filesystem de rutas SQLite inmutable y cerrado, sin FileSystemOs, contenidos,
enlaces, lectura de tsconfig/package.json ni paquetes externos. Cuotas de filas,
memoria estimada, directorios, consultas memoizadas y probes, con contexto de
cancelación/deadline; los fallos de presupuesto provocan rollback, no resultados
silenciosamente parciales.

Perfiles de prioridades relativos contrastados con CodeGraph v1.6.2 y remapeos de
extensiones emitidas solo después de buscar el archivo original. Imports entre
las variantes JS/TS/JSX/TSX usan el lenguaje real del destino para comprobar
exports. Aristas indican el motor de rutas. Nuevos archivos indexados de mayor
prioridad cambian el destino; eliminarlos recupera el fallback bajo la misma
transacción y revalidación de fuentes existentes.

No se certifica cobertura global: no consulta archivos físicos sin indexar ni
activa aliases de configuración, default/namespace/type-only o reexports.
Variantes de index de directorio de Oxc sin equivalencia comprobada quedan
pendientes. No se escribió sobre el índice original ni se cambió el MCP activo.

Validación: suite completa del workspace aprobada con ocho opt-in omitidas.
Publicación 31/31 ordinarias; cuatro unitarias nuevas de rutas aprobadas y una
comparación opt-in con CodeGraph aprobada en once escenarios. Clippy estricto
sigue señalando cuatro warnings SQL/XLSX ajenos al bloque.

### Bloque anterior de runtime: bindings Oxc para llamadas importadas JS/TS

`index_semantic.rs` integra Oxc semantic 0.90.0 en el escritor nativo Windows.
Verifica llamadas directas por binding y posición UTF-16 compatible con CodeGraph,
conservando IDs y extracción del proveedor. Parámetros/bloques homónimos ya no
invalidan otras llamadas legítimas del mismo archivo; los usos como callbacks o
miembros no se promueven a llamadas. Las aristas indican el motor de bindings.

Cache acotada por transacción, chequeos cooperativos de cancelación/deadline,
hashes de fuentes y revalidación antes del commit. Redeclaraciones conflictivas
se rechazan con `SYNC_SEMANTIC`, conservando el rollback. No hay acceso al índice
original ni cambio de la conexión MCP activa. Oxc resolver/tsconfig/packages
siguen fuera del runtime; el alcance de imports sigue parcial.

La familia 0.75 del experimento era incompatible con dependencias XLSX; 0.90.0
declara Rust mínimo 1.87 y no eleva el mínimo declarado del harness. El lockfile
ajusta bumpalo 3.19.0 y zopfli 0.8.3. No se certifica todavía el build en Rust 1.88.

Validación: suite completa del workspace aprobada, siete opt-in omitidas.
Cuatro pruebas unitarias y tres de publicación nuevas aprobadas; publicación
28/28 ordinarias, incluyendo CLI sin Node/Git y regresiones de rollback.
Clippy estricto solo señala cuatro warnings SQL/XLSX concurrentes, no alterados.

### Evaluación anterior: Oxc para scopes y rutas JS/TS

Experimento aislado en `experiments/oxc-evaluation`, sin modificar el runtime ni
el índice original. Catorce pruebas ordinarias y una opt-in contra el proveedor
aprobadas; Clippy estricto del experimento aprobado. Oxc distingue bindings
importados de homónimos locales y resuelve rutas, tsconfig y exports de paquetes
sobre un filesystem sintético. Se recomienda integrarlo gradualmente, no portar
esas reglas manualmente ni incluir Rolldown completo.

La evaluación también detecta diferencias de prioridad `.js`/`.ts` y un conflicto
de declaraciones TS que necesita validación adicional. No certifica paridad ni
habilita estas capacidades en el harness. Resultados, versiones, límites y
reproducción en [el informe](../experiments/oxc-evaluation/README.md).

### Bloque anterior de runtime: imports relativos con nombre y alias JS/TS

El resolver incorpora un primer subconjunto AST de imports relativos con nombre
y alias. Restringe el destino al módulo indexado único, al mismo lenguaje y a
funciones exportadas explícitamente con nombre; nunca elimina receptores ni usa
un homónimo global como fallback de un import no resuelto. Default, namespace,
type-only, reexports, aliases de configuración y scopes completos siguen pendientes.

Se comprueban fuentes de origen/destino contra sus hashes indexados y antes del
commit. Parámetros, declaraciones y usos no soportados del alias difieren todo
el binding, evitando capturar símbolos locales. Cada cambio JS/TS reevalúa las
aristas nativas de imports; también reintenta pendientes cuando aparece un
módulo, conservando el nombre original al retirar o recuperar una relación.
Módulos con varias alternativas indexadas quedan ambiguos, sin imitar todavía
las prioridades del proveedor. Límites de filas, declaraciones y fuentes
observadas; no se certifica cobertura ni sincronización global.

Implementación: `index_imports.rs`, integración en `index_store.rs` y helpers
propios en el entry point del vendor; walkers originales sin cambios.
Alcance y pendientes detallados en [MIGRACION-CODEGRAPH.md](MIGRACION-CODEGRAPH.md).

Validación: suite workspace completa aprobada; publicación 25 pruebas de
integración más cinco unitarias aprobadas. Una prueba opt-in compara mappings
y rutas no ambiguas con CodeGraph en tres casos, aprobada; no certifica paridad
del resolver completo. Clippy estricto aún falla por cuatro warnings SQL/XLSX
concurrentes, fuera de este bloque. No se escribió sobre el índice original.

### Bloque anterior: publicación transaccional y resolución nativa parcial

`index_store` implementa escrituras reales sin Node en un índice CodeGraph
existente schema 11/WAL, solo Windows. CLI `--sync-file <ruta>` reconstruye un
archivo; `--sync` aplica hasta 32 candidatos del plan, con commits independientes.
El lote conserva en JSON los commits confirmados si luego falla otro archivo;
no afirma rollback global y sale con error si queda trabajo nativo del lote.
La configuración codegraph.json bloquea el modo automático hasta portar su scope.

Una transacción incluye nodos, aristas, referencias, hashes y FTS. Empareja
identidades únicas por kind/QN/signature para conservar referencias entrantes
al cambiar líneas; las no emparejadas se recuperan desde refName/refKind, nunca
eliminando receptores. Una referencia legacy no recuperable bloquea la operación.
El resolver implementa un subconjunto de nombres exactos, contenedores/locales
y candidatos únicos accesibles del mismo lenguaje. Ambigüedad, receptores,
imports y reglas avanzadas quedan pendientes, no se presentan como paridad total.
Nuevas definiciones reintentan referencias previas de nombre exacto.

Se invalidan los stamps globales del proveedor al cambiar el grafo; no se
certifica sincronización global. Fuentes sin cambios no se reescriben. Antes del
commit se vuelven a comprobar fuentes y FK; errores, cancelación o drift provocan
rollback. Un resultado incierto de commit/rollback se distingue explícitamente.
No se editan fuentes ni se crea/migra el índice. Sidecars WAL pueden permanecer.

Valida los seis contratos de tablas, schema 11, FTS y los tres triggers exactos
del proveedor. Pines Windows protegen DB, ancestros y sidecars; rechaza reparse
points/hardlinks. Se niega a tocar datos de frameworks/síntesis no migrados,
incluidas aristas registradas por un tercer archivo y synthesis_inputs históricos.
Límites de filas, candidatos, memoria, busy timeout y deadline/cancelación.

Este bloque cierra el ciclo nativo básico de extracción/publicación por archivo,
NO la migración completa del sincronizador multilenguaje y sus resolvers/frameworks.
No se ejecutaron escrituras contra el índice original: pruebas en índices
temporales creados con el schema SQL del proveedor, conservando licencia MIT.

Validación: 19 pruebas de integración de publicación y dos unitarias de rollback
con drift/cancelación aprobadas; suite workspace completa aprobada (seis opt-in
omitidas). Incluye CLI sin Node/Git, límites/replanificación, fallo parcial de
lote, FTS, stamps, legado, síntesis, privacidad, busy y hardlinks. Formato de los
archivos nuevos y diff comprobados. El último Clippy estricto global sigue
señalando warnings en el adaptador SQL concurrente, no alterado por este bloque.

### Bloque anterior: kernel CodeGraph sin Node

Se reutiliza el kernel Rust original de CodeGraph v1.6.2, commit
6560052a6f856855d3f71eee838fd66ccfa4285d, no un extractor inventado desde cero.
El subconjunto vendorizado conserva sin cambios los walkers de Rust/C#/TS/JS,
buffers ABI v2, IDs, docstrings, texto y guardia de stack; incluye licencia MIT
y procedencia. El entry point y registry son Rust ordinario, sin N-API ni Node.
Las gramáticas están fijadas a versiones compatibles con las del proveedor.

`index_extract` decodifica nodos, aristas, referencias y atributos opcionales;
valida longitudes, índices, UTF-8 y ABI, con límites de arena y filas. Porta la
preparación condicional C# preservando posiciones UTF-16 y ambas ramas. Separadores
C# todavía no portados y deferrals del kernel rechazan el archivo, sin fallback.

Discovery admite seis lenguajes: Rust, C#, JavaScript, JSX, TypeScript y TSX.
`--sync-plan` incluye extractionPreview para hasta cuatro candidatos, revalida
el hash del inventario al releer y devuelve contadores, no cuerpos. Indica
relationsResolved=false/published=false. SIGUEN PENDIENTES el resolver entre
archivos, síntesis/frameworks, publicación transaccional y alcance completo del
proveedor. No hay sincronización nativa completa ni escrituras en el índice.

Validación: suite workspace aprobada; prueba opt-in contra el extractor wasm de
CodeGraph aprobada en siete fuentes de los seis lenguajes (igualdad de nodos,
aristas y referencias, omitiendo solo timestamps/duración). Node se usa solamente
en esa referencia de prueba. El último Clippy global encontró dos warnings
collapsible_if en el bloque SQL concurrente, fuera de esta migración; los tres
warnings propios del decoder fueron corregidos. No se modifica ese bloque SQL.

### Bloque anterior: inventario nativo y dry-run ejecutable

`--root <workspace> --sync-plan` conecta el planificador con SQLite de solo
lectura y un inventario real sin Node/Git. Requiere índice existente y schema
1..11; valida rutas/sidecars, rechaza enlaces, limita SQLite y fija una
transacción de lectura. No crea el índice ni publica hashes/nodos/aristas.

Comprueba archivos rastreados no restringidos por contenido y descubre nuevos Rust.
Las rutas sensibles se omiten sin leer ni exponer sus nombres; se contabilizan
como restrictedIndexedFiles, nunca como borrados ni contenido vigente.
Aplica gitignores locales y exclusiones conservadoras; los rastreados se revisan
directamente aun fuera de discovery. Solo ausencias explícitas generan borrados.
Errores, enlaces, UTF-8 inválido y oversize quedan no comprobados. Límites:
40.000 entradas, 20.000 archivos, 64 MiB reservados y diez segundos cooperativos
de inventario, con cancelación/deadline global. No usa el shortcut size/mtime.

La cobertura es deliberadamente parcial: no porta aún ScopeIgnore completo,
configuración include/exclude/overrides, repositorios embebidos, descubrimiento
de nuevos archivos de otros lenguajes, parsers ni publicación/resolución. El
resultado siempre declara fullProviderScope=false, inventoryComplete=false y
synchronized=false. Observaciones de fuentes deben revalidarse antes de escribir.

Validación: siete pruebas de integración nuevas aprobadas, incluida CLI con PATH
vacío, gitignores anidados, junctions, oversize y cambios con mismo tamaño/mtime;
prueba unitaria de presupuesto agregado aprobada. Suite workspace completa
aprobada y Clippy de todos los targets sin warnings. Dry-run contra el índice
real schema 11, sin Node/Git: observó nueve modificados y 320 coincidentes;
agotó el presupuesto de inventario y dejó 1.720 sin comprobar, además de una
entrada restringida omitida. Son observaciones parciales, no un inventario global
ni una sincronización. No se publicaron cambios en el índice.

### Bloque anterior: planificación del sincronizador nativo

Comienza el port compatible con CodeGraph en `src/adapters/index_sync.rs`.
Lee `files(path, content_hash, errors)` sin escribir y produce un plan determinista
de altas, modificaciones, borrados, coincidencias, reintentos de extracción y
archivos no comprobados. Un inventario incompleto no convierte ausencias en
borrados; una ausencia explícita debe venir de una comprobación del scanner.
Errores de lectura nunca equivalen a eliminación. Los hashes iguales con errores
de extracción requieren reintento. Hay cancelación y un límite de 20.000 archivos
por inventario/índice; rutas y metadatos inválidos abortan sin plan parcial.

El helper de hash conserva UTF-8, BOM y finales de línea, con el límite de
extracción de 1 MiB del proveedor. UTF-8 inválido y fuentes mayores quedan fuera
de ese helper: no se sustituyen por markers de tamaño ni se certifican como
contenido vigente. El plan siempre informa synchronized=false.

Es un componente interno probado, aún no conectado a MCP/CLI. El llamador futuro
debe aportar un inventario del mismo alcance y una conexión/snapshot segura.
Faltan scanner compatible con exclusiones/configuración, parsers, publicación
transaccional y resolución de referencias. En particular, CodeGraph recupera
referencias entrantes entre archivos antes de eliminar nodos; se debe portar
ese comportamiento antes de habilitar escrituras. No se modifica el índice real
ni se elimina la dependencia del formato de CodeGraph.

Validación: seis pruebas específicas aprobadas (clasificación sin escrituras,
inventario incompleto, errores/metadatos inválidos, cancelación, presupuesto y
hashes). Suite workspace aprobada antes de añadir el último caso específico;
Clippy de todos los targets sin warnings y formato comprobado tras ese caso.

### Bloque anterior: comprobación de archivos contribuyentes

Comprobación de archivos contribuyentes a CodeGraph: lee hashes y estado de
extracción de la misma transacción SQLite que el grafo, y compara las fuentes
antes/después de leer definiciones locales. Cubre todos los archivos de nodos
devueltos, incluidos niveles posteriores y otros lenguajes. fileFreshness distingue
matches_observed_files/stale/unknown, con motivo por archivo y cobertura explícita.
requireMatchingFiles=true rechaza cambios con INDEX_STALE e incertidumbre con
INDEX_UNCHECKED, sin devolver relaciones; el modo normal conserva evidencia
indexada con warning y detail.complete=false. Observaciones comparten 64 MiB,
2 MiB/archivo y cinco segundos cooperativos, bajo el deadline global existente.
Metadatos ausentes, fallos de extracción, hashes inválidos, límites, UTF-8 inválido
y markers de tamaño no se presentan como coincidencia. No se cambian los hashes
del proveedor ni se ejecuta Node. indexFreshness sigue not_certified: no hay
inventario de archivos nuevos, revalidación de aristas ni vigencia global.
La sincronización completa requiere portar el indexador multilenguaje: la
instalación inspeccionada solo ofrece un sincronizador Node. Sigue pendiente y
se informa synchronization.performed=false/nativeIndexerAvailable=false.
Validación en Windows: 157 pruebas Rust aprobadas y cinco opt-in omitidas en la
suite normal; prueba de índice real aprobada por separado. Clippy sin warnings,
formato y diff comprobados. Incluye coincidencia inicial, cambios/borrados,
archivos contribuyentes de otros lenguajes y saltos posteriores, rechazo estricto,
metadatos ausentes/invalidables, fallos de extracción sanitizados, límites,
reversiones dentro de la operación y MCP estricto con PATH vacío.

### Bloque anterior: recorridos CodeGraph de varios saltos

Recorridos CodeGraph de varios saltos: `direction=inbound/outbound/both` y
`maxDepth=1..4`, conservando both/1 por defecto. BFS independiente por dirección,
sin cambios de dirección en nodos intermedios, con control de ciclos y distancias
mínimas desde los objetivos seleccionados. graph.traversal publica profundidad,
distancias por nodo/dirección, expansiones y llegada al borde solicitado. Se
mantienen 160 aristas globales, 176 nodos máximos, 20 aristas por dirección/nodo,
cinco segundos cooperativos para el grafo y validación de cada ruta visitada.
Recortes siguen explícitos y las definiciones locales siguen limitadas a los
archivos objetivo. Es un vecindario indexado acotado, no una traza de ejecución,
lista de todas las rutas ni prueba de impacto completo. No añade herramientas.
Validación en Windows: 152 pruebas Rust aprobadas y cinco opt-in omitidas en la
suite normal; la opt-in de índice real pasó por separado con uno y dos saltos.
Clippy sin warnings, formato y diff comprobados. Incluye separación de sentidos,
ciclos/autoaristas, rutas convergentes, profundidad inválida, rutas restringidas
en niveles posteriores, recorte global y MCP de dos saltos con PATH vacío.

### Bloque anterior: primera exploración CodeGraph

Exploración nativa CodeGraph (2026-10-17): 18 herramientas publicadas. Lee
directamente el contrato de tablas SQLite del proveedor, schema_versions 1..11,
con rusqlite 0.40.2/SQLite integrado solo en el adaptador nativo. Sin Node, CLI,
indexación, migraciones, sync ni SQL libre. Consultas fijas parametrizadas,
conexión read-only/query_only, transacción, límites de tiempo/filas/candidatos/
aristas y validación de rutas antes de publicar metadatos. Reutiliza sesiones,
admisión, cancelación, métricas/logs y detail. Comprueba definiciones Rust en
fuentes actuales; otros lenguajes o ambigüedad quedan unresolved. El índice no
se presenta como vigente, completo ni prueba de ausencia. El contrato SQLite
interno es una dependencia explícita; cambios del proveedor requieren revisión.
Faltan sync/frescura, consultas naturales sin símbolo explícito, hints de archivos,
flujos de varios saltos, otros parsers y paridad con el backend público Node.
La prueba opt-in contra el índice real instalado (schema 11, proyecto C#) pasó
sin ejecutar Node. Ver README para límites y uso.
Validación: 149 pruebas Rust aprobadas, cinco opt-in omitidas en la suite normal;
la opt-in de índice real se ejecutó por separado y pasó. También pasaron dos
pruebas diferenciales con Node, Clippy con warnings como errores y formato.
Incluye MCP con PATH vacío, índices simulados, base intacta, definiciones
actuales frente a índice obsoleto, rutas inválidas/junction, esquema incompatible,
cancelación, límites de archivos/aristas y recorte UTF-16.

### Bloque anterior: Git para worktrees vinculados

Git para worktrees vinculados (2026-10-17): status/diff aceptan .git archivo con
registro recíproco bajo common/worktrees/id, referencias relativas o absolutas,
y validación de raíz y directorio Git efectivo. Los archivos de registro están
acotados a 8 KiB y las referencias a 4096 bytes; enlaces/reparse points y
registros incompletos se rechazan con errores sanitizados. Conserva la política
de solo lectura, exclusión de submódulos y neutralización de filtros/comandos
externos, incluyendo config por worktree. No concede scopes externos generales
ni soporta gitfiles arbitrarios/submódulos. No es un sandbox frente a cambios
hostiles concurrentes en metadatos; mantiene el límite de confianza del adaptador.
Validación en Windows: 143 pruebas Rust aprobadas y cuatro opt-in omitidas;
Clippy sin warnings y formato comprobado. Los nueve tests Git incluyen un
worktree real con espacios/Unicode, diff staged/worktree, índices principal y
vinculado intactos, config por worktree con filtros maliciosos, gitfile relativo,
backlink/commondir inválidos, límites, junction y redirección por core.worktree.

### Bloque anterior: invalidación local de verificaciones

Invalidación local de verificaciones (2026-10-17): los checks vivos ahora conservan
current=false e invalidation (razón, hora, requiresRerun) cuando refresh detecta
un cambio con igual cobertura, se confirma una edición del harness o cambia el
workspace durante la propia verificación. Una verificación posterior también
contrasta su huella final con los baselines estables anteriores. Revertir archivos
o un escaneo unknown no revive un check invalidado; una nueva ejecución añade un
check nuevo. Las evaluaciones positivas siguen siendo por respuesta, sin current=true.
Los restaurados llevan historical=true y current=null; el formato Node v1 no cambia.
El refresh conserva gate/lease y escanea fuera del mutex global. Este refresh de
verificaciones no tiene watcher (el watcher automático del índice se describe en AUTO-SYNC.md),
dependencias por archivo ni cobertura de inputs externos, entorno o toolchain.
Validación de este bloque: 139 pruebas Rust aprobadas, cuatro opt-in omitidas,
dos pruebas diferenciales con Node aprobadas, Clippy sin warnings y formato
comprobado. Incluye edición real, cambio externo, reversión, nueva verificación,
escaneo unknown, proyección detail y restauración histórica.

### Bloque anterior: checkout independiente y pruebas de procesos

Continuación en el repositorio independiente (2026-10-17): estabilización de
pruebas de procesos y reparación de la comparación con Node tras mover el núcleo.
La cancelación espera una señal privada por prueba, emitida tras crear el
descendiente y vaciar su PID a stdout; incluye watchdog de 10 segundos con
cancelación al fallar el arranque. El caso de timeout ya no exige que el proceso
reciba CPU antes de vencer: la creación de descendientes se comprueba en los
casos de cancelación y salida normal del líder. Los plazos reales del ejecutor
no cambian y siguen existiendo límites temporales en las pruebas.

`HARNESS_NODE_ROOT` permite cargar los módulos y el SDK MCP del arnés Node de
referencia desde otro checkout, conservando el layout anterior como valor por
defecto. Pasaron `cargo test --workspace --locked` (cuatro pruebas opt-in
omitidas), las dos pruebas diferenciales con Node, Clippy con warnings como
errores y diez repeticiones de los nueve tests de procesos durante la revisión
de Clippy. Esto aporta evidencia local bajo carga, no una garantía para toda
carga ni paridad integral. No se añadieron herramientas ni cambió la conexión MCP.

### Bloque anterior: verificación .NET SDK

Verificación .NET SDK por dotnet/csharp, .csproj/.sln explícitos y argumentos
cerrados de build/test, sin restore implícito ni servidores de build compartidos.
Reutiliza ejecutor, gate, Job Objects, límites y observación local existentes.
Doctor/bootstrap descubren dotnet opcionalmente, sin hacerlo dependencia del
harness. Falta MSBuild legacy/.NET Framework y paridad real de frameworks de test.
No hay instalación de SDK/paquetes ni cambios de conexión MCP.

### Bloque anterior: verificación JavaScript

Verificador JavaScript por argumentos cerrados, archivo .js/.cjs/.mjs y Node
opcional: --check o --test, sin npm ni instalación. Entorno del hijo sin preloads
NODE_OPTIONS/NODE_PATH y sin cobertura heredada. Rutas relativas validadas y
glob rechazado; conserva ejecutor, gate, límites y huella existentes. El núcleo
Rust no incorpora dependencias Node. Faltan TS/JSX, runners npm y build de bundles.

### Bloque anterior: verificación Python

Verificador Python nativo por comandos stdlib cerrados, con -I/-S: compilación
sintáctica de archivo explícito o unittest discovery de directorio explícito.
No instala paquetes ni evalúa fuente en build; test sí ejecuta código del proyecto.
Reutiliza ejecutor, gate, huella local y límites existentes. Doctor y bootstrap
detectan python por la política de PATH absoluto. No hay nuevos crates/herramientas.
Siguen pendientes pytest, entornos/dependencias configurables, lint/type checking
y paridad completa. Una salida cero de unittest no certifica que existan tests.

### Bloque anterior: verificación Go

Verificador Go nativo con argumentos cerrados, módulo/ruta local validada y
política de entorno del hijo sin GOFLAGS/hook de caché, descargas, cgo o go.work.
Comparte ejecutor, límites, gate de sesión, resultados y observación local con
Rust. Build/test solo del paquete raíz; faltan subpaquetes, workspaces, cgo y
paridad multilenguaje completa. Doctor y bootstrap detectan Go sin exigirlo para
las demás herramientas. No se añaden dependencias ni herramientas MCP.

### Bloque anterior: búsqueda syntax

Búsqueda syntax de declaraciones Rust por selector kind[:nombre exacto], usando
el colector AST existente sin dependencias nuevas ni Node. Respeta ignores y
permisos externos, admite batch y devuelve ubicación/hash/symbolId/firma, sin
cuerpos. Errores del parser quedan sanitizados y hacen explícita la cobertura
incompleta. No incluye expresiones, patrones ast-grep, items locales, expansión
de macros ni relaciones semánticas. Se mantienen 17 herramientas.

### Bloque anterior: configuración JSON

Configuración JSON redactada en el mismo adaptador nativo, sin dependencias nuevas.
Sección de primer nivel exacta; nombres de propiedades y escalares ocultos antes
de preview/detail. Presupuestos 1 MiB/10000 nodos/profundidad 64 y salida acotada.
Compatible con batch, scopes Windows y logs persistidos. Las claves duplicadas
usan el último valor; no sigue referencias ni ofrece texto editable. Se mantienen
17 herramientas y sigue pendiente la configuración dinámica del runtime.

### Bloque anterior: XML y alcances externos

Configuración XML por parser nativo y alcances externos Windows: 17 herramientas.
Config oculta todos los valores antes del recorte y no devuelve fuente en errores
ni detail; rechaza DTD/entidades, múltiples coincidencias y límites excedidos.
Scopes requieren consentimiento explícito declarado, sesión propietaria y ruta
concreta; caducan, se revocan y comprueban identidad sin conservar pins ociosos.
Pueden usarse en read/search/navegación/config, también dentro de batch compatible,
pero nunca conceden edición, procesos o SQL. Archivos únicos no autorizan vecinos.
Son coberturas parciales: valores no sensibles también redactados; faltan scopes
Unix y configuración dinámica del harness. JSON se añadió después.

### Bloque anterior: file_size

file_size nativo dentro de search, también usable por batch, sin Node/ripgrep ni
CodeGraph. Cuenta LF/CRLF, vacío y última línea; filtra >minLines y devuelve solo
metadata ordenada. Reutiliza validación de rutas/lecturas estables, presupuestos,
ignores y resultados parciales. Límites explícitos más estrictos que Node:
2 MiB/archivo, 64 MiB de reservas, 10000 archivos, 4 MiB de metadata y 5 segundos.
Se mantiene el total de 15 herramientas, ampliando una existente.

### Bloque anterior: persistencia de logs

Persistencia y rotación Windows con --persist-logs (apagada por defecto). Archivo
nativo separado de Node: current/previous y staging fijo, 256 KiB por snapshot,
512 eventos. Directorios fijados, hardlinks/reparse points rechazados, escritor
exclusivo y formato estricto. Recupera staging válido y marca eventos restaurados
como históricos. Un fallo de archivo no modifica el resultado de la herramienta;
se publica loggingWarning y el estado de persistencia. No se restauran métricas.
Se probaron reinicio del ejecutable sin Node, rotación, locks, archivos ajenos,
hardlinks, recuperación y rechazo de staging obsoleto. No hay garantías de energía
ni de I/O con plazo duro. La conexión activa sigue sin cambios.

### Bloque anterior: correlación

Correlación requestId UUID v4 generada por la aplicación antes de admisión.
El envelope y su evento comparten ID, también para rechazo/cancelación/abandono;
el transporte conserva el identificador cuando responde antes que el worker.
Batch conserva IDs propios de hijos, también en salida recortada. Logs acepta
filtro UUID canónico. No es el ID JSON-RPC ni una garantía de entrega; faltan
trazas completas de transporte previas a prepare.

### Bloque anterior: logs volátiles

Logs de metadatos nativos con cursor, niveles, herramienta y lentitud: 15
herramientas publicadas. Eventos de resultado/rechazo/abandono enlazados al guard
de medición. Buffer acotado de 512 entradas, códigos permitidos, sin contenidos,
argumentos ni rutas. Doctor/bootstrap muestran su estado. No se leen logs Node;
el modo por defecto no escribe en disco y las trazas completas siguen sin soporte.

### Bloque anterior: batch

Batch integra las lecturas migradas (14 herramientas publicadas). Tipos de todos
los argumentos validados antes de ejecutar, resultados ordenados por índice,
errores parciales y detalle agregado acotado. Máximo dos coordinadores fuera de
los cuatro cupos normales; hijos sujetos a admisión, cancelación, plazo, leases y
métricas comunes. No mutaciones, recursión, verify ni SQL. No es transacción.

### Bloque anterior: métricas

Métricas acotadas de ejecución en doctor.metrics y checkpoint.stats (también en
detail). Compartidas entre clones, no persistidas ni específicas de una sesión.
Contabilizan admisión/rechazo, resultados terminales, abandono, actividad/picos y
duración total/máxima. Un guard RAII evita dobles conteos y fugas al descartar
operaciones. No retienen argumentos ni rutas. No equivalen a RPC entregadas,
facturación o estadísticas de lectura de Node; esas coberturas siguen pendientes.

### Bloque anterior: diagnóstico

Diagnóstico nativo de runtime, ejecutables y rutas, sin cambios en disco. Hay 13
herramientas publicadas. Por defecto solo localiza; versions=true ejecuta --version
con 2 segundos/4 KiB por herramienta y publica solo versión numérica sanitizada.
La presencia del índice no valida su contenido. CodeGraph/SQL siguen skipped.
La detección de herramientas nativas comparte la política real del ejecutor.

### Bloque anterior: Git

Git nativo añade dos herramientas (12 publicadas). Status usa porcelain v2/NUL,
paginación y contadores. Diff admite archivos eliminados y conserva recortes en
detail. Ambos usan el ejecutor nativo, no Node ni shell; anulan filtros,
fsmonitor, diff externo y textconv. Comprueban que Git no cambie de workspace,
rechazan rutas sensibles y no hacen mutaciones. Falta linked worktrees (.git
archivo). Las pruebas usan repositorios temporales, índices sin cambios, nombres
Unicode, binarios, límites de salida y configuraciones que detectarían ejecución
accidental de filtros. El Git del sistema sigue siendo requisito externo.

### Bloque anterior: huellas y refresh local

`workspaceObservation` compara rutas/contenido SHA-256 antes y después de Cargo:
`unchanged`, `changed` o `unknown`. Incluye ocultos/ignorados, salvo salidas y
directorios excluidos documentados en README. Límites: 10000 entradas, 64 MiB,
2 MiB por archivo y 5 segundos cooperativos por observación.

No rastrea dependencias externas, entorno ni toolchain. Tampoco detecta todos los
cambios intermedios revertidos. `verificationFreshness` sigue `not_tracked` para
vigencia total. Los checks llevan `current=null` salvo invalidación local
(`current=false`). Las observaciones se conservan en la sesión viva,
no en el formato persistido v1. `refreshFreshness=true` compara esas huellas con
una observación nueva: matches_local_workspace, local_workspace_changed,
no_stable_baseline o unknown. La evaluación solo existe en la respuesta y su
detail; no se reutiliza como vigencia positiva permanente. El bloque nuevo conserva
las invalidaciones en memoria. No revalida checks restaurados
ni convierte current en true. Se serializa con verificaciones/escrituras de la
misma sesión, sin retener el mutex global durante el escaneo.

## Verificación realizada antes de este documento

- 138 pruebas Rust cubren configuración, alcances, búsqueda y planes de verificación incluyendo .NET SDK.
- Prueba .NET opt-in pasó: C# real y fallos; despacho test con target VSTest controlado, sin NuGet test framework.
- Las cuatro pruebas opt-in (.NET, Go, Python y JavaScript) pasaron juntas con runtimes instalados.
- Prueba JavaScript opt-in pasó con Node real: check, tests y fallos, sin paquetes adicionales.
- Prueba Python opt-in pasó con intérprete real: compilación, unittest y fallos esperados.
- Prueba real Go opt-in: build/test/fallo de compilación pasaron en Windows con caché preparada.
- La primera compilación de tests Go agotó 30 s; no se amplió el límite ni se marcó éxito.
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
- [x] Invalidación local conservadora en sesión viva por edición, refresh o verify;
  restaurados explícitamente históricos. Vigencia total sigue pendiente.
- [x] Refresh de cobertura local, serializado con verificaciones de la sesión.
- [ ] Completar vigencia total (inputs externos, entorno y toolchain).
- [x] Métricas reales de ejecución por instancia y herramienta, sin valores simulados.
- [ ] Paridad de estadísticas por sesión, bytes y lecturas/repeticiones de Node.
- [x] Diagnóstico nativo local (runtime, herramientas y rutas).
- [ ] Completar diagnóstico de índice/conexiones CodeGraph, SQL y logs/métricas.
- [x] Logs acotados en memoria y consulta nativa con filtros/cursor.
- [x] Correlación por requestId de operaciones preparadas, resultados y eventos.
- [x] Logging persistido/rotado Windows opt-in, con recuperación y validación estricta.
- [ ] Persistencia Unix, configuración dinámica y trazas completas de transporte.
- [x] Sustituir la espera fija de cancelación por señal de descendiente listo;
  separar timeout de supuestos de arranque y repetir pruebas bajo carga local.

### 2. Herramientas de repositorio

- [x] `harness_git_status` y `harness_git_diff` nativos de solo lectura.
- [x] Soporte Git para linked worktrees registrados con .git archivo.
- [x] `harness_batch` para lecturas nativas con admisión y cancelación compartidas.
- [ ] Extender batch a SQL y otras lecturas cuando sus adaptadores estén migrados.
- [x] Lectura XML nativa con redacción de todos los valores y límites explícitos.
- [x] Configuración JSON redactada, incluyendo nombres de propiedades.
- [ ] Configuración runtime y paridad de valores no sensibles con política segura.
- [x] Alcances externos Windows autorizados, acotados por sesión/TTL/identidad.
- [ ] Alcances externos Unix y cobertura de herramientas todavía no migradas.
- [x] Búsqueda de tamaño (`file_size`) nativa con límites explícitos.
- [x] Búsqueda syntax de declaraciones Rust con selectores kind[:nombre].
- [ ] Patrones AST generales/multilenguaje y equivalencia completa de límites/cobertura.
- [ ] Navegación, validación y edición para los demás lenguajes requeridos.
- [x] Verificación Go parcial del paquete raíz de módulo, sin descargas/cgo/workspaces.
- [x] Verificación Python parcial: py_compile de archivo y unittest de directorio.
- [x] Verificación JavaScript parcial con node --check/--test de archivo explícito.
- [x] Verificación .NET SDK parcial por .csproj/.sln, sin restore automático.
- [ ] MSBuild clásico/.NET Framework y paridad de runners .NET/JavaScript/Python/Go.

### 3. Relaciones e integraciones

- [x] Primera integración nativa de `harness_explore`: índice existente y relaciones de un salto.
- [x] Recorridos indexados de varios saltos, dirigidos y acotados, con control de ciclos.
- [x] Comprobar contenido de archivos contribuyentes y modo estricto sin certificar índice completo.
- [ ] Portar sincronizador/indexador multilenguaje, vigencia global, parsers y consultas naturales.
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
