# Migración de CodeGraph a ejecución nativa Rust

Punto de control: 2026-10-17. Estado: implementación parcial, no migración terminada.

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

| Bloque | Implementado | Límite actual |
| --- | --- | --- |
| Lectura del grafo | Consulta SQLite, navegación acotada y comprobación de hashes de archivos contribuyentes | No certifica cobertura ni vigencia global |
| Planificador | Altas, cambios, reintentos, bajas explícitas y archivos no comprobados | Un inventario incompleto no prueba que un archivo fue eliminado |
| Inventario | Lectura por contenido, exclusiones conservadoras y descubrimiento de seis lenguajes | No reproduce toda la configuración del proveedor |
| Extracción | Núcleo Rust original para Rust, C#, JavaScript, JSX, TypeScript y TSX | No incluye todos los lenguajes ni todos los preprocesamientos |
| Publicación SQLite | Nodos, aristas, referencias, hashes y FTS en una transacción por archivo | Windows; índice existente, schema 11 y modo WAL |
| Referencias entrantes | Conservación por identidad única y recuperación mediante metadatos originales | Referencias legacy irrecuperables bloquean la escritura |
| Resolver nativo | Nombres exactos y primer subconjunto de imports relativos JS/TS con nombre y alias | No equivale al resolver completo de CodeGraph |
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

- Imports con nombre, rutas relativas internas y candidatos únicos en el índice.
- Mismo lenguaje entre origen y destino; aún no une TSX con TS, por ejemplo.
- Funciones declaradas y exportadas explícitamente con nombre, llamadas directas.
- Comprueba el contenido del importador y destino contra sus hashes indexados,
  y lo vuelve a comprobar antes del commit cuando intervino en la resolución.
- Si el alias aparece como parámetro, declaración u otro uso fuera de una llamada
  directa, se difiere todo ese binding. Es una restricción deliberada, no un
  análisis léxico completo de scopes.
- Reintenta alias pendientes cuando aparece un destino y reevalúa las aristas
  nativas de imports al publicar cambios JS/TS. Retira enlaces que dejan de ser
  válidos por cambios de exportación o nuevas alternativas de módulo.

Siguen pendientes paquetes externos, default/namespace/type-only, reexports,
aliases de tsconfig, mapeo de extensiones emitidas (`.js` a `.ts`), scopes
completos y compatibilidad de familias de lenguajes. Varias alternativas indexadas
se consideran ambiguas; todavía no se reproduce la prioridad de rutas del proveedor.
Los imports de otras formas bloquean el fallback global de ese archivo. El
descubrimiento sigue siendo parcial: no prueba que no existan alternativas sin indexar.

## Lo que falta

### 1. Resolver de relaciones — prioridad alta

- [ ] Inventariar las reglas del resolver original y definir el alcance de paridad por lenguaje.
- [x] Primer subconjunto de imports relativos con nombre/alias JS/TS y funciones exportadas.
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
- El último Clippy estricto registrado señaló warnings del bloque SQL concurrente;
  su estado actual debe volver a comprobarse, no se presupone resuelto.

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

Siguiente bloque recomendado: ampliar las pruebas diferenciales del resolver
completo antes de incorporar reexports, scopes léxicos o más formas de imports.

## Mapa de implementación

- [index_sync.rs](src/adapters/index_sync.rs): planificación y modo dry-run.
- [index_inventory.rs](src/adapters/index_inventory.rs): observación y descubrimiento.
- [index_extract.rs](src/adapters/index_extract.rs): extracción y decodificación del ABI.
- [index_store.rs](src/adapters/index_store.rs): transacciones, publicación y resolver parcial.
- [index_imports.rs](src/adapters/index_imports.rs): alcance de imports, observación de fuentes e invalidación.
- [index_store.rs — pruebas](tests/index_store.rs): escritura, protección y fallos.
- [index_extract.rs — pruebas](tests/index_extract.rs): extracción y comparación opt-in.
- [ARCHITECTURE.md](ARCHITECTURE.md): organización general del harness.
