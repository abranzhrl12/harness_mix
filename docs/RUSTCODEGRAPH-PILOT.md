# RustCodeGraph: piloto de adopción

Estado: **evaluado, no activado como backend de producción**. La biblioteca no
pasa los contratos seleccionados de Vue/Razor. No se han cambiado el kernel,
el esquema SQLite, los puertos ni las políticas de publicación de Harness.

## Identidad y ejecución

- Donante: [hunzhiwange/rustcodegraph](https://github.com/hunzhiwange/rustcodegraph).
- Revisión fija: `cfd4d74132219f498d5ae611d7e94990a27c2c0d`, versión 1.2.10.
- Licencia raíz MIT; aviso conservado en el experimento.
- CLI compilado desde el checkout exacto con su Cargo.lock upstream.
- Biblioteca invocada desde un proceso separado mediante un puente de extracción
  JSON, con dependencia Git fijada y Cargo.lock propio. El lock del puente
  resuelve Tree-sitter 0.26.13; el CLI upstream bloquea 0.26.10. No se afirma
  igualdad de builds ni se comparan tiempos.
- [Resultado registrado](evaluations/rustcodegraph-cfd4d741.json), con hashes de
  ambos ejecutables y del lock experimental.
- [Comandos y límites del piloto](../experiments/rustcodegraph-probe/README.md).

La prueba es opt-in y está ignorada por defecto. Crea ocho fuentes controladas
en un proyecto temporal; el CLI inicializa exclusivamente su índice temporal.
La base se consulta en modo read-only. La biblioteca recibe únicamente la fuente
de cada fixture y no abre ningún índice. No se ejecuta install, serve, watch ni
uninit, ni se modifica configuración MCP/global.

## Lo que impide añadir el crate directamente

Se ensayó resolución Cargo en manifiestos descartables, sin compilar ni abrir
bases de datos. Los dos conflictos se observaron independientemente:

| Dependencia nativa | Harness | Donante | Resultado |
| --- | --- | --- | --- |
| Tree-sitter | 0.25.10 | 0.26.10 en el lock upstream | Cargo rechaza dos paquetes con `links = "tree-sitter"`. |
| SQLite | rusqlite 0.40.2 / libsqlite3-sys 0.38.2 | rusqlite 0.37.0 / libsqlite3-sys 0.35.0 | Cargo rechaza dos paquetes con `links = "sqlite3"`. |

Un proceso separado evita estos conflictos para evaluar. No demuestra que sus
IDs, referencias o schema puedan mezclarse con `.codegraph`, ni autoriza una
migración de dependencias o del índice real.

## CLI y biblioteca no son el mismo camino

En esta revisión `src/bin/rustcodegraph/indexer/mod.rs` utiliza
`extract_lightweight_symbols` y `extract_lightweight_call_edges`. El indexador
CLI es una ruta ligera por regex. Por otro lado,
`src/extraction/tree_sitter/public_api.rs` despacha a `VueExtractor` y
`RazorExtractor`, que delegan scripts/bloques a Tree-sitter.

Por ello el piloto ejecuta **ambas rutas**. Probar solo el CLI no permitiría
evaluar correctamente los módulos que queremos reutilizar. El rename incremental
del CLI sí pasó: elimina el nombre anterior y publica el nuevo en su índice.
Eso no verifica snapshots concurrentes ni el resto de la semántica incremental.

## Contratos observados

| Fixture | Contrato Harness | CLI | Biblioteca |
| --- | --- | --- | --- |
| Vue básico, declaración en línea propia | Una función | Cumple | Cumple |
| Script Vue inline tras markup con emoji | Función en línea 1, columna UTF-16 42 | No publica esa función | Publica columna 7, relativa al script |
| Dos bloques Vue con helper homónimo | Dos declaraciones con identidades únicas | Cumple el conteo | Dos declaraciones pero dos IDs duplicados en el resultado |
| Script externo con cuerpo no utilizable | No interpretar el cuerpo como script inline | Cumple | Extrae una función del cuerpo |
| Script y template en la misma línea | Observar el candidato de componente del template | No lo observa | No lo observa |
| Miembro C# en `@functions` | Publicar la declaración del miembro | No publica el miembro | No publica el miembro; emite una referencia a su nombre |
| `@model App.Models.Person` | Conservar referencia cualificada | No emite la referencia | Emite `Person`, no la referencia cualificada |
| Código dentro de comentario Razor | Cero refs al callback comentado | Cumple | Emite dos refs a `CommentOnlyCall` |

Resultado: cuatro diferencias en el CLI y siete en la biblioteca. Los ocho
oráculos del Harness pasaron. Son casos de adopción seleccionados para detectar
regresiones; **no equivalen a una tasa general de precisión del donante**.
Las diferencias de representación tampoco prueban por sí solas que todos los
resolutores posteriores fallen: exigen un adaptador y pruebas específicas.

La prueba de auditoría termina en verde si ejecuta correctamente y registra los
hallazgos. El resultado de adopción se expresa aparte:
`cliReplacementGatePassed=false`, `libraryAdoptionGatePassed=false`.

## Decisión y alcance siguiente

No sustituir los extractores Vue/Razor actuales por los módulos del donante sin
adaptación. No migrar SQLite ni Tree-sitter únicamente para poder enlazarlo.
El experimento sirve para elegir porciones concretas y comprobar futuras revisiones.

Antes de portar una pieza deben resolverse los contratos que afecte: coordenadas
originales, IDs por bloque, comentarios/directivas, declaraciones C# preservadas
y binding cualificado. La salida debe pasar por los adaptadores actuales de
Harness; la lectura de filesystem/DB y la certificación de hashes siguen fuera
del dominio y de la extracción pura. Los tests permanecen separados.

La búsqueda en `src/resolution` y `src/extraction` de esta revisión no encontró
un resolver Angular equivalente a los módulos Angular del Codegraph original.
No se presume que portar este fork recupere automáticamente esas capas.

No se ha habilitado un nuevo motor, reemplazado el esquema ni copiado código del
donante a producción. Decidir qué módulo nuevo aporta una ventaja real es el
siguiente paso; este piloto no justifica sustituir funcionalidades que ya pasan
nuestros contratos.
