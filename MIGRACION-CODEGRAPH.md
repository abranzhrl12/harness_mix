# Migración de CodeGraph a ejecución nativa Rust

Punto de control: 2026-10-17. Estado: implementación parcial, no migración terminada.

Plan de siguientes entregas y bibliotecas a reutilizar:
[PLAN-PENDIENTES-CODEGRAPH.md](PLAN-PENDIENTES-CODEGRAPH.md).

Este documento resume el bloque CodeGraph del harness. El estado general y el
historial de los demás bloques están en [MIGRATION-STATUS.md](MIGRATION-STATUS.md).
En caso de contradicción con una etapa histórica, este resumen describe el
alcance actual de este bloque; las pruebas y el código siguen siendo la evidencia.

## Objetivo y motivo

Queremos que el harness pueda consultar y actualizar un índice compatible con
CodeGraph sin necesitar Node ni node_modules para esas operaciones.

CodeGraph sí analiza código Rust. El problema no es el lenguaje analizado:
su implementación combina un núcleo Rust con coordinación, resolución de
referencias y síntesis implementadas en JavaScript/TypeScript. Reutilizar el
núcleo no elimina por sí solo todas las dependencias de Node.

No estamos creando otro formato de grafo ni otro parser desde cero. Reutilizamos
el núcleo de CodeGraph y su esquema SQLite, conservando explícitos los límites
de compatibilidad. La conexión Node activa no se ha sustituido automáticamente.

## Lo que tenemos

Última ampliación: aliases `paths/baseUrl` desde tsconfig explícito, JSONC y
`extends` local simple, con hashes de padres, límites, rollback y vigencia al
consultar. Diez pruebas del perfil pasaron, incluida comparación de seis casos
con CodeGraph v1.6.2. No incluye selección por importador, references, herencia de
paquetes/arrays ni reexports. Perfil completo: [TSCONFIG-PATHS.md](TSCONFIG-PATHS.md).

| Bloque | Implementado | Límite actual |
| --- | --- | --- |
| Lectura del grafo | Consulta SQLite, navegación acotada y comprobación de hashes de archivos contribuyentes | No certifica cobertura ni vigencia global |
| Planificador | Altas, cambios, reintentos, bajas explícitas y archivos no comprobados | Un inventario incompleto no prueba que un archivo fue eliminado |
| Inventario | Lectura por contenido, exclusiones conservadoras y descubrimiento de seis lenguajes | No reproduce toda la configuración del proveedor |
| Extracción | Núcleo Rust original para Rust, C#, JavaScript, JSX, TypeScript y TSX | No incluye todos los lenguajes ni todos los preprocesamientos |
| Publicación SQLite | Nodos, aristas, referencias, hashes y FTS en una transacción por archivo | Windows; índice existente, schema 11 y modo WAL |
| Referencias entrantes | Conservación por identidad única y recuperación mediante metadatos originales | Referencias legacy irrecuperables bloquean la escritura |
| Resolver nativo | Nombres exactos, imports relativos JS/TS con nombre y aliases de tsconfig explícito | No equivale al resolver completo de CodeGraph |
| Lotes | Hasta 32 candidatos por ejecución y reporte de commits previos ante un fallo | No existe una transacción global del workspace |
| Seguridad | Validación de rutas, enlaces, DB, sidecars, esquema, FTS, límites y cancelación | Las garantías de escritura implementadas son específicas de Windows |

Antes del commit se vuelve a comprobar la fuente. Cambios de contenido, errores
o cancelación provocan rollback; una confirmación incierta se informa como tal.
El escritor no modifica los archivos fuente ni crea o migra el índice.

Cuando cambia el grafo, se invalidan marcas globales del proveedor. El resultado
sigue declarando `synchronized=false` y `globalFreshnessCertified=false`.
Un commit confirmado no significa que todo el proyecto esté sincronizado.

## Procedencia del núcleo reutilizado

- Proveedor: CodeGraph v1.6.2.
- Commit fijado: `6560052a6f856855d3f71eee838fd66ccfa4285d`.
- Licencia MIT conservada.
- Copia y adaptación de entrada: [vendor/codegraph-native](vendor/codegraph-native/UPSTREAM.md).
- Los walkers seleccionados se conservan; la entrada N-API se sustituye por una biblioteca Rust ordinaria.
- No hay fallback oculto a Node en la extracción nativa.

## Entradas disponibles

Desde este repositorio, reemplazar `RUTA_WORKSPACE` por un proyecto con un índice
compatible. Empezar con una copia de prueba del proyecto y su índice.

```powershell
# Solo plan y preview: no publica cambios en el índice.
cargo run --locked -- --root "RUTA_WORKSPACE" --sync-plan

# ESCRIBE el índice: archivo concreto, ruta relativa con barras /.
cargo run --locked -- --root "RUTA_WORKSPACE" --sync-file "src/ejemplo.rs"

# ESCRIBE el índice: lote de hasta 32 candidatos nativos.
cargo run --locked -- --root "RUTA_WORKSPACE" --sync
```

`--sync` rechaza proyectos con `codegraph.json` mientras no se porte su alcance.
Si quedan candidatos, se vuelve a planificar en la siguiente ejecución. Ante
fallos hay que revisar el JSON: otros archivos pueden haber sido confirmados.
La escritura también se rechaza cuando afectaría síntesis/frameworks no portados.
Estos comandos no implican una sincronización automática desde MCP.

## Nuevo bloque: imports relativos y alias JS/TS

El resolver ya puede enlazar una llamada como `execute()` tras
`import {run as execute} from './lib'` con una función exportada explícitamente
por el módulo indicado, sin elegir un homónimo global. Usa el AST de las
gramáticas ya incluidas; no incorpora un parser de imports basado en regex.

Alcance inicial conservador:

- Imports con nombre y rutas relativas internas sobre archivos del índice.
- Destinos entre Rust y C# no se mezclan con JS/TS; dentro de JS/TS/JSX/TSX ya
  se admiten imports entre esas variantes, por ejemplo TSX hacia TS.
- Funciones declaradas y exportadas explícitamente con nombre, llamadas directas.
- Comprueba el contenido del importador y destino contra sus hashes indexados,
  y lo vuelve a comprobar antes del commit cuando intervino en la resolución.
- `oxc_semantic` 0.90.0 comprueba cada llamada directa por binding y posición:
  un parámetro o variable local homónima queda pendiente sin bloquear llamadas
  válidas al import en otros scopes. Callbacks y accesos a miembros no se
  convierten automáticamente en llamadas.
- Reintenta alias pendientes cuando aparece un destino y reevalúa las aristas
  nativas de imports al publicar cambios JS/TS. Retira enlaces que dejan de ser
  válidos por cambios de exportación o nuevas alternativas de módulo.

Siguen pendientes paquetes externos, default/namespace/type-only, reexports,
selección de tsconfig por importador y semántica completa entre archivos. El escritor ahora aplica
prioridades explícitas del proveedor a rutas relativas con Oxc y soporta el
fallback de extensiones emitidas cuando no existe el archivo original.
Los imports de otras formas bloquean el fallback global de ese archivo. El
descubrimiento sigue siendo parcial: no prueba que no existan alternativas sin indexar.

## Lo que falta

### Evaluación de bibliotecas: Oxc sí ayuda

Se añadió [experiments/oxc-evaluation](experiments/oxc-evaluation/README.md), aislado
del runtime y con lockfile propio. Pasaron 14 pruebas ordinarias, una opt-in de
contraste con CodeGraph y Clippy estricto del experimento. El resolver funciona
sobre archivos sintéticos en memoria, sin acceso al proyecto original.

`oxc_semantic` permite conservar referencias válidas a imports aunque existan
homónimos locales en otros scopes; su integración descrita abajo sustituye el
guard global conservador para las llamadas directas importadas.
`oxc_resolver` resolvió relativos, índices, extensiones emitidas, tsconfig paths
y exports condicionales de paquetes. Recomendación: reutilizar ambas piezas
gradualmente, sin incorporar un bundler completo como Rolldown.

No es un reemplazo directo: se detectó y documentó una diferencia de prioridad
`.js`/`.ts` frente al proveedor y un caso de conflicto de bindings que requiere
revisar redeclaraciones. La evaluación inicial no habilitó capacidades del
runtime; la integración posterior de semantic se describe a continuación.
La integración posterior de `oxc_resolver` sobre un snapshot del índice está
descrita abajo; tsconfig explícito está integrado con el perfil enlazado arriba,
mientras que packages permanece fuera del alcance activo.

### Integración realizada: Oxc semantic en el escritor nativo

`index_semantic.rs` ya se usa desde el resolver de `--sync-file`/`--sync` en
Windows. Oxc aporta análisis léxico de JS/TS; CodeGraph conserva la extracción,
los IDs, las referencias originales y el formato SQLite. No añade Node ni
modifica automáticamente la conexión MCP activa.

Se analiza una vez cada importador observado por transacción y se retienen solo
llamadas directas a imports con nombre. Los offsets UTF-8 se convierten en una
pasada a líneas LF 1-based y columnas UTF-16 0-based, compatibles con CodeGraph.
La pertenencia se comprueba por nombre, línea y columna; no basta una coincidencia
de nombre en el archivo. Las aristas de imports registran el motor de bindings.

Se conservan los hashes de fuentes y su revalidación antes del commit, los
límites de 1 MiB por fuente/64 archivos observados y el rollback existente.
Se limita el conjunto semántico a 20.000 llamadas por fuente y 16 MiB estimados
de datos retenidos por transacción; esto no es un límite duro de RSS del parser.
Cancelación/deadline se comprueban alrededor de parser/semantic y en recorridos;
las llamadas internas de Oxc no se interrumpen a mitad de análisis. Conflictos
de redeclaraciones o análisis inválido abortan con `SYNC_SEMANTIC` y rollback.

La versión 0.75 del experimento chocaba con `bumpalo` requerido por XLSX. Se fijó
la familia 0.90.0 (declara Rust mínimo 1.87), sin elevar el mínimo declarado 1.88
del harness. Cargo ajustó `bumpalo` a 3.19.0 y `zopfli` a 0.8.3 en el lockfile.
La validación usa el compilador instalado; no se certifica todavía Rust 1.88.

El bloque semántico por sí solo no cambia la política de rutas. Siguen pendientes
default/namespace/type-only, reexports y callbacks. Los archivos sin cambios conservan el no-op actual:
la actualización del motor no fuerza una reconstrucción global del índice.

### Integración realizada: Oxc resolver sobre el índice autorizado

`index_paths.rs` integra `oxc_resolver` 11.0.0 en el escritor Windows. Su filesystem
implementa únicamente una vista inmutable de rutas SQLite dentro de la transacción:
no utiliza `FileSystemOs`, no sigue enlaces ni consulta `package.json`, paquetes
externos o directorios del usuario. Solo recibe el tsconfig seleccionado como
contenido sanitizado en memoria; Project valida y lee sus archivos originales.
El destino final sigue requiriendo lectura autorizada, hash coincidente y
revalidación antes del commit por el harness.

Resuelve extensiones y directorios de imports relativos con perfiles explícitos
por lenguaje, contrastados con CodeGraph v1.6.2. Primero prueba las extensiones
del proveedor y el archivo original; solo después aplica remapeos `.js` a
`.ts`/`.tsx`/`.d.ts`, `.jsx` a `.tsx`, `.mjs` a `.mts` y `.cjs` a `.cts`, incluidos
sus candidatos de declaraciones. No sustituye estas prioridades por defaults de
bundler. La resolución permite destinos entre las cuatro variantes JS/TS y usa
el lenguaje real del destino para validar sus exports.

Una publicación JS/TS reevalúa las relaciones nativas de imports: un archivo de
mayor prioridad recién indexado puede cambiar el destino de una llamada y al
eliminarlo se recupera el fallback. Las aristas registran `moduleEngine` junto al
motor semántico. Las sobrecargas o símbolos ambiguos dentro del módulo siguen
pendientes; varias rutas ya no se consideran automáticamente ambiguas cuando
hay una prioridad definida y comprobada.

Límites por transacción: 20.000 filas de inventario, 40.000 directorios sintéticos,
16 MiB estimados de rutas, 1.024 consultas distintas memoizadas, 4 MiB estimados
de memo y 100.000 probes de filesystem. Excederlos aborta con `SYNC_LIMIT` y rollback;
cancelación/deadline se propagan. No son un límite duro de RSS de caches de Oxc.

La vista incluye solo archivos JS/TS no restringidos ya registrados en el índice:
un archivo físico nuevo no se incorpora hasta su publicación. No prueba ausencia
de candidatos sin indexar ni certifica cobertura global. Oxc admite más variantes
de index de directorio que CodeGraph; las no equivalentes se dejan pendientes
(por ejemplo, si selecciona `index.d.ts` donde el proveedor no lo admite).
El soporte de aliases/extends local se detalla en TSCONFIG-PATHS.md. No activa
exports de paquetes, herencia de paquetes/arrays ni referencias fuera del workspace.

### 1. Resolver de relaciones — prioridad alta

- [ ] Inventariar las reglas del resolver original y definir el alcance de paridad por lenguaje.
- [x] Primer subconjunto de imports relativos con nombre/alias JS/TS y funciones exportadas.
- [x] Análisis Oxc por binding/posición para llamadas directas importadas y shadowing local.
- [x] Oxc resolver sobre snapshot del índice: rutas relativas, prioridades y remapeo de extensiones emitidas.
- [ ] Completar imports, exportaciones, alias y rutas de módulos hasta el alcance acordado.
- [ ] Resolver receptores, cadenas de miembros y referencias calificadas complejas.
- [ ] Cubrir sobrecargas, herencia y reglas de visibilidad más completas.
- [ ] Comparar aristas y referencias pendientes con resultados del proveedor.

No debe resolverse una referencia ambigua solo para reducir el número de pendientes.

### 2. Alcance y configuración — prioridad alta

- [ ] Portar las reglas aplicables de `codegraph.json`: inclusiones, exclusiones y overrides.
- [ ] Cubrir repositorios embebidos y las reglas de descubrimiento del proveedor.
- [ ] Definir tratamiento compatible de generados, archivos grandes y fuentes no soportadas.
- [ ] Distinguir explícitamente cobertura parcial de un inventario completo verificable.

### 3. Frameworks y síntesis — prioridad alta

- [ ] Inventariar qué frameworks necesita realmente el proyecto original.
- [ ] Portar sus reglas de rutas, componentes, registro e inyección según ese alcance.
- [ ] Mantener procedencia e invalidación cuando una relación depende de un tercer archivo.
- [ ] Probar actualización y eliminación sin perder relaciones sintetizadas.

### 4. Ciclo de vida y cobertura — según alcance acordado

- [ ] Decidir si se soportarán más lenguajes además de los seis actuales.
- [ ] Implementar creación inicial del índice si se requiere retirar Node también del bootstrap.
- [ ] Definir compatibilidad y migraciones de esquema; hoy la escritura exige schema 11.
- [ ] Definir soporte de escritura fuera de Windows, si se necesita.
- [ ] Integrar el sincronizador en el flujo MCP con política explícita de escritura y errores.

Estos puntos no están todos implementados ni todos son obligatorios para una
primera entrega limitada: requieren fijar el alcance antes de prometer paridad total.

### 5. Validación y retirada de Node

- [ ] Ejecutar pruebas diferenciales de extracción y resolución sobre proyectos representativos.
- [ ] Repetir la suite completa sobre el workspace final integrado.
- [ ] Validar copia del proyecto original, con altas, cambios, bajas y fallos parciales.
- [ ] Definir recuperación y retorno al proveedor original; puede necesitar reindexación completa.
- [ ] Cambiar la conexión activa solo después de aprobar el alcance y las pruebas.
- [ ] Documentar dependencias opcionales restantes: ejecutar tests de un proyecto JS puede necesitar Node aunque el harness sea nativo.

## Evidencia de validación registrada

Integración Oxc resolver 11.0.0: suite completa del workspace aprobada (ocho
opt-in omitidas); 31 pruebas de publicación ordinarias y cuatro unitarias nuevas
de rutas aprobadas. La comparación opt-in con CodeGraph aprobó once escenarios
de prioridad/extensiones. Incluye remapeo JS/TS, imports TSX hacia TS, rebinding
al añadir/eliminar candidatos, filesystem cerrado, cuotas y cancelación.
Clippy estricto conserva cuatro warnings SQL/XLSX ajenos a este bloque.

Integración Oxc semantic 0.90.0: suite completa del workspace aprobada con siete
opt-in omitidas. Cuatro pruebas unitarias nuevas y tres de publicación nuevas
aprobadas; 28 pruebas de publicación ordinarias pasan. Incluyen bindings mixtos,
shadowing de bloque/parámetro, callbacks que no se promueven a llamadas, BOM,
emoji/CRLF y posiciones CodeGraph, conflictos semánticos con rollback y CLI sin
Node/Git en PATH. Clippy estricto sigue fallando únicamente por cuatro warnings
de SQL/XLSX fuera de esta migración, no modificados.

Último bloque de imports/alias: suite completa del workspace aprobada; 25 pruebas
de integración de publicación y cinco unitarias de ese bloque aprobadas. Incluyen
CLI con PATH vacío, shadowing, homónimos, altas/bajas, cambios de exportación,
ambigüedad de módulos y rollback por cambios concurrentes del importador.
La prueba opt-in de mappings y rutas no ambiguas frente al proveedor aprobó
tres casos; no es una comparación del resolver completo. Clippy estricto sigue
fallando por cuatro warnings de SQL/XLSX ajenos a este bloque, no modificados.

Registro anterior, conservado como historial:

- Publicación: 19 pruebas de integración y dos unitarias de rollback aprobadas.
- La última comprobación de esas 21 pruebas, incluida la corrección de booleanos
  SQLite, se ejecutó en una copia aislada por cambios concurrentes SQL/XLSX.
  No equivale a certificar el workspace integrado actual.
- Hubo una ejecución completa del workspace aprobada antes de esa última corrección.
- La comparación opt-in de extracción con CodeGraph aprobó siete fuentes de los
  seis lenguajes; se omitieron timestamps y duración. No prueba paridad del resolver.
- Las pruebas de escritura usaron índices temporales; no se publicó contra el índice original.
- Clippy estricto del workspace pasó tras corregir cuatro collapsible_if de SQL/XLSX.

## Complejidad y porcentaje

Estimación orientativa comunicada: complejidad restante alta, alrededor de 8/10,
y aproximadamente 40–50 % de esfuerzo pendiente para la sincronización equivalente
a CodeGraph. **No es una medición de avance ni un porcentaje de todo el harness.**

La incertidumbre principal está en resolver relaciones y frameworks, no en guardar
filas SQLite. La cifra debe revisarse tras inventariar reglas y fijar el alcance.
No conviene convertirla en una fecha de entrega ni contar herramientas como paridad.

## Cuándo podremos decir «terminado»

Para el alcance acordado: extracción y relaciones comparadas con el proveedor,
configuración respetada, cobertura y limitaciones explícitas, recuperación
verificada, suite integrada aprobada y ejecución real sin depender del coordinador
Node. Hasta entonces, el nombre correcto es **sincronización nativa parcial**.

Siguiente bloque recomendado: selección de configuración por importador y
reexports/formas restantes de imports, con invalidación transitiva y fixtures
diferenciales, sin habilitar acceso libre al disco.

## Mapa de implementación

- [index_sync.rs](src/adapters/index_sync.rs): planificación y modo dry-run.
- [index_inventory.rs](src/adapters/index_inventory.rs): observación y descubrimiento.
- [index_extract.rs](src/adapters/index_extract.rs): extracción y decodificación del ABI.
- [index_store.rs](src/adapters/index_store.rs): transacciones, publicación y resolver parcial.
- [index_imports.rs](src/adapters/index_imports.rs): alcance de imports, observación de fuentes e invalidación.
- [index_semantic.rs](src/adapters/index_semantic.rs): bindings Oxc y posiciones de llamadas importadas.
- [index_paths.rs](src/adapters/index_paths.rs): Oxc resolver y filesystem cerrado del índice.
- [index_tsconfig.rs](src/adapters/index_tsconfig.rs): JSONC, herencia local, límites y procedencia.
- [index_store.rs — pruebas](tests/index_store.rs): escritura, protección y fallos.
- [index_extract.rs — pruebas](tests/index_extract.rs): extracción y comparación opt-in.
- [ARCHITECTURE.md](ARCHITECTURE.md): organización general del harness.
