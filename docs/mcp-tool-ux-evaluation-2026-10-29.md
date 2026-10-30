# Evaluación estática del UX de herramientas MCP para LLM

**Fecha:** 2026-10-29
**Alcance:** solo lectura; contrato Harness MCP Rust v3.3.0 publicado en esta sesión. No se ejecutaron pruebas de producto, builds ni cambios en código.

## Método reproducible

1. Llamar a harness_bootstrap, verificar data.root y conservar data.session.
2. Leer las descripciones y esquemas publicados; con harness_read leer contracts/tool-contract.json, tests/llm_contract.rs, crates/domain/tests/contracts_compatibility.rs y crates/application/src/logging/tests.rs.
3. Contabilizar ramas visibles del esquema y campos obligatorios; anotar para 11 tareas la secuencia mínima esperada. No inferir métricas de éxito de usuarios: son métricas estáticas.
4. Consultar harness_doctor con action=logs para eventos del proceso de esta evaluación.

Bootstrap confirmó raíz E:\harness_mix, índice existente y sesión válida. El contrato tenía SHA-256 b509f283ff0935ad8fe756429225d4c1ca28c302239c341e72b97d1f909f8373; tests/llm_contract.rs tenía 168a25c5739bc9adf84f1488312b44827b8d2fc3f67e3c4c95f58256c55b5768. El estado Git inicial estaba limpio.

## Tareas representativas y rutas

Los campos mínimos son los necesarios para la intención indicada. Se omite session de la lista porque se copia del bootstrap y se añade a llamadas según la convención publicada. Alternativas unidas por “o” son excluyentes.

| # | Intención | Herramientas y campos mínimos | Ambigüedad / error esperado | Ruta esperada |
|---|---|---|---|---|
| 1 | Encontrar archivo por nombre | harness_search: kind=files, query | query por sí solo no distingue ruta de contenido; elegir kind. Paginación mediante nextCall. | 1 llamada |
| 2 | Buscar texto literal | harness_search: kind=text, query; opcionales scope, contextLines | regex cambia semántica y desactiva recuperación por términos. Cero resultados acotados no prueba ausencia. | 1 llamada; paginar si nextCall |
| 3 | Encontrar patrón sintáctico | harness_search: kind=syntax, query, ast | ast no combina con regex ni caseSensitive; lenguaje debe ser compatible. | 1 llamada |
| 4 | Leer líneas de archivo | harness_read: path y (start con end o lines) | path/file son alias; lines es cantidad y excluye end. Combinación inválida da error de argumentos; respuesta puede truncarse. | 1 llamada; seguir nextStart si existe |
| 5 | Inspeccionar estructura y luego cuerpo | harness_symbols: outline(action,path), luego read(action,path,symbol) | La unión action cambia campos; symbol solo es válido en read. El símbolo debe ser exacto. | 2 llamadas |
| 6 | Leer símbolo con archivo/nombre conocido | harness_symbols: action=read, path, symbol; opcionales container, symbolId | El parámetro es symbol, no name. Homónimos requieren desambiguación; ID no encontrado da TARGET_NOT_RESOLVED. | 1 llamada; si ambiguo, outline y reintento con selector |
| 7 | Seguir llamadas/dependencias | harness_explore: selector exacto (filePath y símbolo, o query según intención) | Esquema publicado como unknown; endpoints exactos. Índice puede estar incompleto o truncado; inspeccionar frescura y ambigüedad. | 1 llamada; refinar selector o usar detail.ref |
| 8 | Buscar objeto SQL, describirlo y consultar filas | harness_sql: search(action,query), describe(action,name), query(action,sql) | Cinco acciones en unión. Engine por defecto SQL Server puede no coincidir; PostgreSQL no permite action=query. Buscar y describir antes de asumir columnas. | 3 llamadas en SQL Server; 2 hasta describe en PostgreSQL |
| 9 | Describir objeto conocido | harness_sql: action=describe, name; opcionales engine, schema, objectType, sections | Alias kind/objectType y alias de nombre. Autodetección sirve si no hay ambigüedad; objeto sin schema puede resolver incorrectamente. | 1 llamada; indicar schema/categoría si ambiguo |
| 10 | Verificar archivo o proyecto | harness_verify: language; scope para seleccionar archivo/proyecto | action build/test, varios motores y formatos de scope. Syntax por archivo no equivale a build/test. .NET requiere .csproj/.sln, SDK y restore previo. Ejecutar build/test requiere autorización. | 1 llamada; si tarea de fondo, seguir taskId con harness_jobs |
| 11 | Diagnosticar error reciente | harness_doctor: action=logs; opcionales limit, after, requestId, tool | Registro en memoria acotado; detalle memory-only y redacción heurística. No es histórico garantizado. | 1 llamada; para SQL puede añadirse harness_sql(action=status) |

## Métricas estáticas

Criterio: decisión es elección visible que altera herramienta, variante de acción o semántica de opciones excluyentes. Campo cuenta cada campo obligatorio por instancia en las rutas mínimas, sin session.

- 11 tareas; 7 herramientas involucradas: search, read, symbols, explore, sql, verify, doctor.
- 14 llamadas mínimas sumando las rutas de la tabla: la tarea 5 usa 2; SQL Server en tarea 8 usa 3; las restantes usan 1. PostgreSQL reduce una llamada en la tarea 8; fallback de homónimos agrega una llamada.
- 28 campos obligatorios por instancia de llamada: búsqueda 6; lectura 3; símbolos 8; explore 1; SQL 7; verificación 2; diagnóstico 1.
- Al menos 15 decisiones visibles: tipo de búsqueda, modo de patrón, forma del rango, acción de símbolos, selector de explore, acción/engine de SQL (incluida su incompatibilidad PostgreSQL-query), acción/motor/alcance de verificación y canal de diagnóstico. La cifra cuenta ramas, no complejidad humana validada.
- 4 incompatibilidades explícitas: start/end frente a lines; ast frente a regex/caseSensitive; campos variantes por action en símbolos/SQL; PostgreSQL frente a query SQL. Además hay alias potencialmente conflictivos: scope/path, query/pattern, file/path y session/sessionId.
- Brecha de publicación: el esquema MCP visible de harness_read, harness_search y harness_explore aparece como unknown, aunque bootstrap y contrato JSON describen semántica detallada. harness_symbols, harness_sql y harness_verify sí publican uniones tipadas.
- Batch acepta un subconjunto del catálogo. En la revisión, intentar incluir harness_doctor en harness_batch dio ARGUMENT y listó variantes permitidas, sin invocar hijo. La llamada directa a harness_doctor(action=logs) funcionó. Es fallo de selección de hijo, no del servidor.

## Tests y logs consultados

tests/llm_contract.rs cubre copia de sesión, compatibilidad sessionId y conflictos, perfiles, proyección de detalle, ambigüedad/IDs, truncamiento de relaciones y registro de SQL. crates/domain/tests/contracts_compatibility.rs cubre validación de uniones para símbolos/lectura, aliases de búsqueda e incompatibilidades JSON. crates/application/src/logging/tests.rs define semántica de eventos y límites del registro.

harness_doctor(action=logs, limit=10) mostró eventos de lectura y Batch de esta revisión como completados; el Batch rechazado por argumento no inició hijo. La captura corresponde a instancia activa de mejor esfuerzo, no persistente ni específica de sesión según el propio resultado; no se interpreta como histórico de producción o métrica de usuarios.

## Interfaz propuesta

1. Publicar JSON Schema tipado y discriminado para read, search y explore. Alinear campos requeridos, enums, defaults y exclusiones con runtime. Exponer las dos formas de lectura (lines o start/end) y la incompatibilidad ast/regex.
2. Normalizar alias en la frontera: path, query y session como nombres canónicos; conservar alias anteriores por compatibilidad, marcarlos deprecated y devolver diagnóstico claro si chocan. Si el cliente conserva la conexión, aceptar session del adaptador para evitar copiar token en cada llamada; mantener parámetro explícito para clientes sin estado.
3. Declarar capacidades por acción y motor (SQL Server tiene query, PostgreSQL no) y publicar la lista de hijos permitidos por Batch. Validar temprano con lista exacta de alternativas, o admitir todas las herramientas de solo lectura.
4. Unificar errores accionables: código estable, campo inválido, valores aceptados y siguiente llamada sugerida. Mantener structuredContent canónico y metadatos de truncamiento/frescura.
5. Mantener herramientas explícitas para búsqueda, lectura, relaciones, SQL y verificación. Reducir reintentos con esquemas fieles, alias canónicos y errores guiados; no enrutar por intent de texto libre.

## Límites

Mide complejidad expuesta, no tasa real de reintentos, preferencia, latencia o tokens de conversaciones. No se ejecutaron tests, builds, consultas SQL reales ni verificación. Lo observado es perfil full y backend Rust; el contrato registra diferencias Node. Los logs son volátiles y la respuesta Batch de diagnóstico omitió parte de metadatos del hijo por límite de presentación.
