# Mediciones acotadas de otros apartados

## Actualización: consulta indexada aplicada

El prototipo fue promovido y ajustado en `index_candidates.rs`: dos búsquedas
independientes, por name y qualified_name, con los mismos filtros, orden y
LIMIT 65 por rama. Se combinan como máximo 130 filas, se deduplican y se aplica
de nuevo el orden y límite global. Cualquier fila del top-65 global pertenece
al top-65 de al menos una rama, por lo que esos límites conservan el resultado.
Se conservó el SQL antiguo bajo cfg(test) como oráculo independiente.
No se depende del alias interno rowid de SQLite.

Medianas debug en el mismo test, 4096 nodos y 512 búsquedas distintas:

| Caso | SQL antiguo | SQL aplicado |
| --- | ---: | ---: |
| Nombre existente | 1253,9239 ms | 42,4643 ms |
| Nombre ausente | 1612,6652 ms | 34,9604 ms |

EXPLAIN confirma SEARCH por idx_nodes_name y idx_nodes_qualified_name. Los
SCAN de CTEs/subconsultas se limitan a las listas de candidatos; no son el
barrido anterior de nodes por idx_nodes_file_line. Aunque la combinación queda
acotada, cada búsqueda todavía puede examinar muchos homónimos para ordenar.

Con 8192 homónimos se comprobaron exactamente las mismas filas, orden y límite
en Rust, TypeScript y búsquedas importadas. La primera búsqueda del SQL aplicado
costó 8–12 ms frente a 3–5 ms del antiguo: NO es una mejora universal de CPU.
El UNION de rowids sin límite anterior había costado 21–53 ms en otro ensayo;
se descartó para no materializar todos los candidatos. Esas ejecuciones tuvieron
carga variable y no permiten atribuir un porcentaje estable entre variantes.

Validación final acotada: 10 tests del componente (11,281 s de ejecución) y
8 casos adicionales de integración/seguridad (4,165 s), todos pasados. Incluyen
ramas disjuntas/solapadas, ambigüedad, mayúsculas, lenguaje, rutas importadas,
visibilidad, renombrados, caché, plazos, cancelación durante SQL, rollback tras
escrituras y un esquema con el nombre rowid sombreado. No se corrió la suite
completa ni se recompiló release. Estos números no actualizan la medición
global release anterior de 3,74 s. La medición end-to-end posterior ya pasó:
3170,677 ms Rust frente a 4205,607 ms CodeGraph, con 71 archivos. Método,
muestras, artefacto y límites en BENCHMARK-CODEGRAPH-STORAGE.md.

## Historial: mediciones previas a aplicar el cambio

2026-10-17. Solo tests diagnósticos y un prototipo SQL bajo cfg(test).
Sin cambios de producción en este bloque, sin suite completa y sin build release.
Todos los tiempos siguientes son debug; no representan indexación completa.

## Escritura: esquema real, FTS, WAL y synchronous=FULL

Tres rondas alternando el orden, bases temporales independientes. 1257 filas
por tabla para ejercitar lotes de 128/32/8/1. Nodos con las 21 columnas de
producción; siete columnas de aristas y ocho de referencias. Preparación de
filas y creación del esquema excluidas. Ambos caminos materializan parámetros
propios. El camino de referencias por lotes incluye RETURNING id, mientras el
individual usa execute + last_insert_rowid, como el escritor anterior.

| Fase | Fila a fila preparada, mediana | Lotes, mediana |
| --- | ---: | ---: |
| Nodos | 105,3598 ms | 29,2571 ms |
| Aristas | 17,6133 ms | 13,6850 ms |
| Referencias | 18,7361 ms | 15,1012 ms |
| Integridad FK | 1,9439 ms | 1,7548 ms |
| Commit | 18,6933 ms | 18,7822 ms |

Se comprobaron los mismos conteos de filas, unicidad de IDs devueltos, integridad
FK y una consulta MATCH sobre FTS. El commit permanece prácticamente igual:
no se atribuye una mejora de durabilidad/commit al batching.
Test: `real_schema_write_phase_diagnostic`, pasó en 4,249 s de ejecución.

## Nombres únicos: caché sin repetición y acceso por índice

Esquema real, SQLite en memoria, 4096 nodos, 512 búsquedas únicas por ronda.
Tres rondas por escenario, alternando el orden. Caché y consulta directa hacen
512 consultas: no hay ahorro de consultas sin repetición. La consulta actual
elige `SCAN nodes USING INDEX idx_nodes_file_line` más ordenación parcial;
es un barrido del índice por archivo/línea, no una búsqueda por nombre.

El prototipo selecciona primero rowids por nombre o nombre cualificado mediante
UNION y después aplica los mismos filtros, ORDER BY y LIMIT 65. EXPLAIN confirma
SEARCH sobre idx_nodes_name/idx_nodes_qualified_name y búsqueda por rowid.
El prototipo NO está conectado a la consulta de producción.

| 512 búsquedas, medianas | Directa actual | Caché actual | Prototipo por índices |
| --- | ---: | ---: | ---: |
| Nombres existentes | 1363,6136 ms | 1285,9493 ms | 28,7547 ms |
| Nombres ausentes | 1036,0583 ms | 1589,3044 ms | 20,5884 ms |

Hubo variación de carga importante: una ejecución anterior dio 625/661 ms
para existentes y 614/639 ms para ausentes (directa/caché). No se afirma un
porcentaje estable de sobrecosto de caché. El barrido SQL y el número de consultas
sí están comprobados. Tampoco se extrapola el ratio del prototipo al índice global.

El test comparativo adicional verifica filas completas, orden, nombre frente a
nombre cualificado, mayúsculas, lenguaje, ruta importada y límite de ambigüedad
en 60 combinaciones y muestras del conjunto grande. No certifica todos los
casos reales ni su rendimiento con miles de homónimos: deben probarse antes de
adoptar el SQL. Tests `indexed_query_prototype_preserves_rows_and_ambiguity_limit`
y `unique_lookup_diagnostic`: 2/2 pasados en 16,531 s, formato verificado.

Los primeros ensayos descartados detectaron un fixture sin columnas obligatorias
y un diagnóstico demasiado grande para el plazo de 30 s. Se corrigió el fixture
y se acotó a 512 búsquedas por ronda manteniendo los 4096 nodos y plazos activos.
