# Eficiencia para LLM — núcleo Rust, contrato 2.1.0

## Cambios

- Guía de selección y workflows en `contracts/tool-contract.json`, filtrados por
  backend. Investigar/revisar comienza en solo lectura; no prescribe una cadena
  fija de herramientas ni compilaciones por rutina. Se amplía solo la evidencia
  que falta y se reutilizan texto/hash vigentes.
- `explore(view="summary")` devuelve firmas y una vista estructurada pequeña:
  hasta cuatro relaciones por dirección, ubicación de participantes, inventario
  del subgrafo retornado y omisiones explícitas. No es un resumen inventado por
  otro modelo. `view="full"` sigue siendo el comportamiento por defecto para
  compatibilidad; el routing recomienda summary para navegación, no para editar.
- El grafo completo permanece en `detail.ref`. `data_query(resultRef=...,query=...)`
  permite recuperar solo sus campos necesarios sin reenviar el JSON por el LLM
  ni volver a ejecutar la consulta original. Es una instantánea con TTL, no una
  nueva certificación de frescura del código.
- Bootstrap reutilizado no reenvía el catálogo/políticas idénticos: devuelve
  hashes, flags de reutilización y una referencia al catálogo. Una sesión nueva,
  restaurada, un cambio de capacidades o `includeRules=true` conserva/restaura
  el contexto necesario. El catálogo recuperable incluye routing y workflows.
- Outline añade inventario total/por tipo y omisiones: para un nombre conocido
  se recomienda query y un presupuesto pequeño antes de cargar todas las firmas.
- Se declara la cobertura real de navegación nativa: Rust, JavaScript y TypeScript.
  Para otros formatos no se recomienda repetir parsers que no los admiten.
- Las preguntas de implementación/edición piden full directamente. Una firma o
  resumen jamás se propone como oldText; no se relajaron rutas, hashes, controles
  SQL, autorizaciones, cancelación ni avisos de vigencia/cobertura.

## Medidas reproducibles

Unidades: UTF-16 del JSON canónico del envelope. No son tokens de un modelo,
bytes totales del transporte MCP, facturación ni una medida de inteligencia.

| Escenario | Salida completa | Salida compacta | Reducción |
| --- | ---: | ---: | ---: |
| Navegación de un símbolo | 15556 | 3031 | 80,5 % |
| Bootstrap inicial + navegación, dos llamadas | 22189 | 9664 | 56,4 % |
| Reutilizar sesión frente a recargar contexto completo | 6106 | 1266 | 79,3 % |

La navegación usa un fixture con una función de implementación larga que no es
necesaria para responder quién la llama y a quién llama. Ambas respuestas dan
la respuesta relacional; se comprueba igualdad de todos los nodos/aristas
recuperados por referencia y disponibilidad del cuerpo exacto/hash al pedirlo.
No es una comparación con Node, con una versión anterior del binario ni una
garantía del mismo porcentaje en todas las tareas. El ahorro de la tarea de dos
llamadas incluye el mismo contexto inicial en ambas variantes, no solo el tool
result que más se beneficia.

## Validación dirigida

29 pruebas aprobadas: explore (11), contratos LLM (6), eficiencia (3), datos (4),
navegación JS/TS (2), protocolo MCP/schema discovery (1), contrato/documentación
y routing (2). Una prueba opt-in de índice real se dejó sin ejecutar. No se
ejecutó la suite completa ni se modificaron índices de producción.

Comandos principales:

```text
cargo test --locked -p agent-harness --test llm_efficiency -- --nocapture
cargo test --locked -p agent-harness --test stdio mcp_handshake_discovery_bootstrap_and_safe_read -- --exact
cargo test --locked -p harness-domain --features schema tool_contract::tests
```

## Activación y límites

Implementado y probado en Rust. La conexión de esta conversación sigue siendo
Node y no incorpora automáticamente estos cambios. Su integración sigue pendiente
de la extensión explícita del permiso de edición de los archivos Node protegidos
y, después, de pruebas y reconexión. No se cambió la configuración productiva.

La guía orienta al modelo, no garantiza obediencia ni aumenta su capacidad
intrínseca. Los límites de texto no equivalen a un límite universal de tokens
para todos los campos; el tamaño depende del contenido y el modo solicitado.

## Tiempos reales por operación (2026-10-18)

Prueba opt-in `real_project_command_timings` en `tests/stdio.rs`, sobre el
índice existente de backend-noctalia. Tres procesos nuevos; cinco repeticiones
por operación y proceso (15 muestras). MCP stdio real, PATH normal, sin
sincronización, SQL ni compilación del proyecto probado. Binario del perfil
test de Cargo (optimized + debuginfo), no una comparación release/Node.

| Operación | Mediana antes (ms) | Mediana después (ms) | Muestras por ejecución |
| --- | ---: | ---: | ---: |
| Arranque + initialize MCP | 43.86 | 115.87 | 3 |
| Bootstrap inicial | 132.01 | 164.05 | 3 |
| Bootstrap reutilizado | 2.56 | 3.42 | 15 |
| Explore main | 51.65 | 56.23 | 15 |
| Read symbol main | 3.80 | 5.73 | 15 |
| Search files | 20.32 | 23.55 | 15 |

No hay mejora demostrada de latencia por operación: las muestras posteriores
fueron más lentas y no se aisló CPU, caché ni carga del sistema. La mejora
funcional validada elimina un corte arbitrario por 100 líneas en read_symbol:
un cuerpo de hasta 16000 bytes se devuelve completo, aunque tenga más líneas.
Los cuerpos mayores conservan detail/paginación; harness_read conserva 100 líneas.
La respuesta de main pasó de 844 bytes (sin cuerpo) a 5636 bytes (cuerpo exacto).
No se promete menos tokens por llamada: se evita obligar a una llamada adicional.

Sondas de proceso --version, cinco muestras: harness 54.41 ms; agy 246.81 ms.
No equivalen al arranque de un agente ni a tiempo del modelo.
La prueba anterior de Antigravity llegó al límite de 90 segundos y devolvió
respuesta parcial: SUCCESS del CLI no basta para declarar una ejecución completa.

Reproducir en PowerShell, sin compilar dentro de la medida:

```powershell
$env:HARNESS_BENCH_ROOT = "E:\PROYECTO RUST\backend-noctalia"
cargo test --locked -p agent-harness --test stdio real_project_command_timings -- --ignored --exact --nocapture
```

La prueba registra cada muestra y tamaño, separa arranque de consultas y no
imprime cuerpos de código. Los tiempos son observaciones locales, no SLAs.

Prueba corta de Antigravity sobre el mismo proyecto: 47.95 s de reloj, dos
llamadas MCP (bootstrap y explore), sin errores de argumentos ni lecturas extra
de símbolos. Hubo cuatro lecturas internas del cliente para schemas/salidas.
Los pasos MCP del cliente declararon 1.67 s y 1.30 s; no son las latencias
nativas directas de la tabla. El resto incluye arranque, modelo, red y gestión
de herramientas; no se aisló cada componente. El prompt limitó la respuesta a
60 palabras, distinto del intento de 90 s anterior: no es un speedup controlado
ni una garantía de calidad o consumo de tokens.

Validaciones dirigidas: 5 tests de símbolos Rust, 2 de JS/TS, 3 de eficiencia y
la sonda opt-in MCP de 3 procesos/60 consultas. Los cuerpos grandes conservan
recuperación por detail y el texto exacto/hash del cuerpo pequeño se comprobó.

## Auditoría automática de respuesta y logs

`real_project_payload_and_log_audit` en tests/stdio.rs correlaciona cada consulta
con su log terminal mediante requestId, sin LLM ni exportar código. En el índice
real de backend-noctalia, la prueba afirmó iguales nodos/aristas con ambos presupuestos:

| maxChars (UTF-16) | Cuerpos completos/parciales | JSON-RPC total (bytes) | Envelope estructurado (bytes) | Log nativo (ms) |
| --- | --- | ---: | ---: | ---: |
| 6000 | 1 / 2 | 32731 | 15665 | 25.242 |
| 12000 | 3 / 0 | 37393 | 17967 | 22.600 |

Ambos: 11 nodos y 11 relaciones, estado ok. El segundo entregó los tres cuerpos
sin recortar Markdown a cambio de aproximadamente 14.2% más bytes de transporte.
6000 fue una elección de la auditoría anterior; 12000 ya es el valor normal del
harness. No se demuestra ahorro de tokens ni latencia del agente con esta sonda.

El resultado MCP contiene el envelope tanto en content como en structuredContent;
la duplicación de transporte no demuestra que el modelo consuma ambos canales.
No se eliminó ningún canal: requiere una prueba de compatibilidad por cliente.
El JSON completo pesa más que el Markdown por grafo, metadatos y escapes.
Los logs registran estado/duración, no el cuerpo: la prueba combina ambas fuentes.

## Presentación experimental por cliente

`HARNESS_MCP_PRESENTATION=context` habilita solo para explore exitoso view=full
un canal textual orientado a lectura: cobertura/límites primero, relaciones y
código después. Conserva exactamente el envelope completo en structuredContent.
Sin esa variable, errores y summary mantienen la respuesta JSON compatible.
No cambia consultas, indexación, reglas, hashes ni permisos.

Prueba `real_project_context_presentation`, mismo índice/proyecto, dos perfiles:

| Perfil | Canal textual (bytes UTF-8) | JSON-RPC total (bytes) | Envelope estructurado (bytes) |
| --- | ---: | ---: | ---: |
| legacy | 17967 | 37393 | 17967 |
| context | 12564 | 31408 | 17967 |

La prueba afirmó iguales nodos/aristas (11/11) y tres cuerpos completos.
Reducción aproximada: 30% del texto y 16% del transporte, no tokens certificados.
Dos tests unitarios comprueban metadatos de seguridad y compatibilidad; una
prueba MCP comprueba ambas presentaciones y otra el handshake normal.

La primera evaluación real de Antigravity recibió el Markdown experimental,
pero leyó dos veces el archivo de salida y alcanzó 60 s sin respuesta. No se
considera éxito ni ahorro de tiempo. Después se adelantaron cobertura/límites
para que no queden al final de salidas largas; esa versión está validada por
pruebas MCP; el piloto posterior se detalla abajo.
La opción quedó desactivada por defecto y no se modificó configuración global.
No se promete que Antigravity consuma solo content o evite archivos temporales.

## Investigación y piloto A/B del agente (2026-10-18)

Fuentes primarias consultadas:

- Google, Prompt design strategies: delimitadores consistentes, Markdown/XML e instrucciones directas:
  https://ai.google.dev/gemini-api/docs/prompting-strategies
- Anthropic, Writing effective tools: contexto relevante, nombres inequívocos, ejemplos, recortes accionables y medición por agente:
  https://www.anthropic.com/engineering/writing-tools-for-agents
- Anthropic, Demystifying evals for AI agents: múltiples intentos, criterios verificables y revisión de trazas:
  https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents
- He et al., Does Prompt Formatting Have Any Impact on LLM Performance?:
  https://arxiv.org/abs/2411.10541
- Chroma, Context Rot (18 modelos y tareas de recuperación; no benchmark de nuestro harness):
  https://www.trychroma.com/research/context-rot

No hay evidencia universal de que Markdown gane a JSON. Los estudios de formato
incluyen modelos/tareas distintos de nuestro piloto; no se trasladan porcentajes.
Calidad, latencia, tokens y bytes son métricas diferentes.

Piloto de agente sobre backend-noctalia, mismo encargo de cuatro preguntas sobre
main y mismo modelo explícito gemini-3.8-flash-low. Secuencia ABBA: legacy,
context, context, legacy. Procesos/sesiones nuevos, solo lectura, sin indexar.
Prompt: bootstrap y grafo de main en crates/api/src/main.rs, maxFiles 2,
maxChars 12000, view full; responder configuración previa a AppState, scheduler,
señales y límites de exhaustividad, máximo 80 palabras.

| Intento | Perfil | Tiempo de reloj (s) | Llamadas MCP | Errores de argumentos | Lecturas internas CLI | Relecturas por ruta |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | legacy | 29.11 | 3 | 1 | 2 | 0 |
| 2 | context | 24.82 | 4 | 2 | 1 | 0 |
| 3 | context | 24.51 | 3 | 1 | 1 | 0 |
| 4 | legacy | 18.16 | 3 | 1 | 1 | 0 |

Las cuatro respuestas incluyeron configuración/telemetría, spawn_listing_scheduler,
shutdown_signal y la advertencia de que no se demuestra impacto exhaustivo.
Checks de palabras clave son solo auxiliares; también se revisaron las respuestas.
No se ejecutó la API ni se validó funcionalidad de negocio.

Medias descriptivas: legacy 23.635 s; context 24.665 s. No hay mejora demostrada
de velocidad (context fue aproximadamente 4.4% más lento en este piloto).
Sumas del contador total_tokens reportado por CLI: legacy 64852; context 61506
(aproximadamente 5.2% menos). cache_read_tokens, separado: legacy 139707;
context 138459. No convertir estos contadores en coste monetario ni sumar
campos sin comprobar la definición del proveedor.

Los fallos ocurrieron antes de recibir la presentación: path por filePath,
prompt por query, query ausente y query con texto no admitido. El agente pudo
repararlos. No se deben atribuir esos fallos al formato de salida.
El agente eligió/reparó argumentos; es una prueba de flujo completo, no un
experimento de paquetes de entrada idénticos ni una comparación pura de formatos.
Dos intentos por perfil, una tarea, sin aislamiento de red/caché/carga y sin
temperatura configurable: insuficiente para significancia o generalización.

Decisión: mantener el modo compacto opt-in. Priorizar simplificar entradas y
ejemplos de explore, y después ampliar el banco con tareas no usadas para ajustar
el formato. El entorno HARNESS_MCP_PRESENTATION se restauró al terminar cada lote;
no se cambió configuración global ni se tocó código de backend-noctalia.

## Perfiles de herramientas y carga directa (2026-10-18)

Se reutilizó rmcp, sin añadir un framework ni serializador. Las listas se
definen en contracts/tool-contract.json y se aplican en catálogo, bootstrap,
routing, opciones del batch y validación de Application::prepare:

| HARNESS_TOOL_PROFILE | Herramientas | Alcance |
| --- | ---: | --- |
| full (predeterminado) | 29 | Compatibilidad con capacidades existentes |
| investigate | 13 | Navegación, búsqueda, detalle/proyección, Git de lectura, logs/doctor y batch permitido |
| edit | 16 | investigate más replace, verify y diagnostics |

No es una autorización del usuario ni un sandbox. verify ejecuta código del
proyecto; las ediciones siguen requiriendo hash y validación. Los perfiles
limitados no permiten SQL ni explore syncIndex=true. Un batch que incluya una
herramienta no permitida se rechaza entero antes de iniciar hijos. Un nombre
de perfil desconocido impide iniciar el servidor; no cae silenciosamente a full.
La selección pertenece al dueño del proceso, no a argumentos del modelo.

Bootstrap/explore llevan _meta antigravity/alwaysLoad=true para que clientes
compatibles puedan cargar sus schemas directamente. No es una política de
seguridad ni obliga a otros clientes. HARNESS_MCP_EAGER_TOOLS=0 desactiva la pista.
Se conservan las otras anotaciones y el catálogo completo por defecto.

Prueba real única, CLI 1.4.2 y gemini-3.8-flash-low, backend-noctalia:
perfil investigate, presentación JSON compatible, carga directa habilitada.
15.80 s de reloj; dos herramientas MCP directas
(mcp_agent-harness_harness_bootstrap y mcp_agent-harness_harness_explore),
cero errores y cero lecturas de archivos de esquema. Hubo una lectura interna
del resultado. La respuesta identificó el perfil, configuración/telemetría,
spawn_listing_scheduler y shutdown_signal.
Contadores CLI: input_tokens 30624, output_tokens 382, cache_read_tokens 57971,
total_tokens 31006. No se tradujeron a coste ni se sumaron campos.

Es una prueba de integración/flujo, no un A/B aislado: cambiaron catálogo y
carga de herramientas y el encargo incluyó reportar el perfil. No se atribuye un
porcentaje de aceleración. Las variables se restauraron al finalizar la prueba;
no se cambió configuración global ni código del proyecto real.

Verificación dirigida: nueve tests de contrato de aplicación; cuatro pruebas
MCP (catálogo/llamadas directas/batch, perfil inválido, opt-out de eager y
handshake compatible). Los rechazos se probaron también sin pasar por MCP.

## Casos complejos de investigación (2026-10-18)

Tres casos nuevos sobre backend-noctalia, gemini-3.8-flash-low, perfil investigate,
eager habilitado y presentación compatible. Sin ejecutar API, SQL, builds ni
indexación. Las variables del proceso se restauraron al terminar.

| Caso | Tiempo (s) | Llamadas MCP | Errores de herramienta | Lecturas internas | Rutas internas repetidas |
| --- | ---: | ---: | ---: | ---: | ---: |
| Arranque entre main, AppState::build y build_router | 42.70 | 7 | 2 | 1 | 0 |
| Scheduler, errores y señales Windows/Unix | 16.81 | 2 | 0 | 0 | 0 |
| Homónimos de main y límites | 19.41 | 3 | 0 | 0 | 0 |

Ningún caso leyó archivos de esquema. No son comparaciones A/B ni repeticiones
suficientes para estimar tasas generales.

- Arranque: localizó los tres archivos y el orden general. Recuperó dos errores:
  falta de toolAction en el puente genérico del cliente y path ausente en outline.
  La explicación situó el scheduler bajo el bloque de composición sin dejar clara
  la llamada visible en main. No se considera una respuesta impecable ni se toma
  su afirmación de "código verificado" como prueba independiente de cada detalle.
- Scheduler: contrastado con las definiciones actuales (hash del archivo
  2be2220dc7072b7b777ee8c21de6f9af57cccae8ec5fc4875cfa94516da45d34):
  None retorna sin spawn; publish_due(50), intervalo nominal de un segundo,
  MissedTickBehavior::Skip, errores registrados sin salir del loop; SIGTERM
  solo en Unix, pending en no-Unix y Ctrl-C común. El helper no guarda un
  JoinHandle ni muestra cancelación coordinada explícita. No se probó ejecución.
- Homónimos: identificó API, arch-graph, check-error-docs y e2e_auth.py y usó
  selección exacta. Reportó correctamente PARSER_UNAVAILABLE para Python.
  Interpretó relationsOmitted como truncamiento del grafo, cuando era una
  omisión de presentación del resumen. Calidad parcial.

Se añadió la prueba opt-in real_project_summary_omissions_are_not_graph_loss.
Recupera el grafo guardado por resultRef y compara identidades completas de
nodos/aristas, no solo contadores, entre summary y full:

| Selección | Nodos/aristas conservados | Relaciones omitidas solo del resumen |
| --- | --- | ---: |
| main sin filePath | 26 / 24 | 19 |
| main de la API exacto | 11 / 11 | 7 |

Hallazgo de contrato pendiente: graph.truncated pasa de false en summary a true
en full por truncation.definitions=true, aunque las identidades del grafo son
idénticas. Esto mezcla recorte de cuerpos con recorte del grafo. La prueba
diagnóstica lo registra explícitamente; no se corrigió producción en esta ronda.
Full amplio también expuso cuerpos auxiliares no resueltos (Python, SYMBOL y
RANGE); se informaron como unresolved y no se inventó código.

Siguiente prioridad sugerida: separar omisión del resumen, límites del recorrido
y recorte de cuerpos, y evaluar carga directa para las lecturas de símbolos y
outline que todavía pasan por el puente genérico. No ampliar herramientas
cargadas sin medir el coste de contexto.

## Segunda tanda de casos difíciles (2026-10-18)

Mismo cliente/modelo/perfil que la tanda anterior, sesiones nuevas, sin código
ni configuración del proyecto real modificados. Presentación compatible y eager
bootstrap/explore; sin SQL, terminal, builds ni indexación.

| Caso | Tiempo (s) | Llamadas MCP | Errores | Lecturas internas | Repeticiones detectadas |
| --- | ---: | ---: | ---: | ---: | ---: |
| Símbolo inexistente noctalia_symbol_missing_918274 | 14.66 | 2 | 0 | 0 | 0 |
| Router con maxChars=1000 y recuperación del final | 16.23 | 3 | 0 | 0 | 0 |
| Firma e identidad de AppState::build | 20.29 | 3 | 1 | 2 | 0 |

- Inexistente: devolvió unresolved/TARGET_NOT_RESOLVED con cero candidatos, nodos
  y relaciones, sin inventar otra coincidencia. Explicó que no prueba ausencia
  global ni cobertura de archivos sin indexar. No hizo búsqueda masiva.
- Router: una exploración de build_router con filePath exacto y maxChars=1000,
  seguida de harness_read(start=535,end=555). El archivo termina en 553, por lo
  que obtuvo el final disponible sin error. Identificó las capas finales
  from_fn_with_state(state.config.clone(), csrf_middleware) y timeout, más
  with_state(state), coherentes con el fragmento actual inspeccionado. No pidió
  el cuerpo entero ni repitió explore. Su explicación de "grafo incompleto"
  no se puntúa como demostración de causa: la ambigüedad de graph.truncated
  sigue pendiente y requiere inspeccionar cada flag, no solo el booleano global.
- Firma: usó query build + filePath crates/api/src/state/mod.rs + view summary.
  Devolvió pub async fn build(cfg: &shared::config::Config) -> anyhow::Result<Self>,
  contenedor impl AppState y distinguió CodeGraph node ID de AST symbolId.
  Reconoció signature_only/editReady=false y la necesidad del cuerpo/hash para
  editar. No descargó el cuerpo. Primero añadió toolAction indebidamente a la
  función MCP directa de bootstrap; luego se recuperó usando el puente genérico.

Las repeticiones se contaron por parámetros idénticos en view_file; en MCP se
comparó una selección de campos (sin session), por lo que es un indicador
auxiliar y no una prueba universal de ausencia de lecturas redundantes.
Se revisaron las trazas de estos tres casos: no hay repetición de explore ni de
lecturas de código, y ninguna tarea terminó por timeout. SUCCESS no sustituye
la revisión semántica. Una ejecución por tarea no estima tasas generales.

Contadores total_tokens CLI: 23458, 27478 y 29439 respectivamente; cache_read_tokens
32958, 56208 y 85343, separados. No se calcularon costes ni se compararon como
un A/B. Siguiente fallo de interacción a estudiar: distinguir claramente
parámetros del puente genérico de los parámetros de herramientas MCP directas.

## Corrección posterior del contrato de recortes

Se corrigió la ambigüedad documentada arriba: `graph.truncated` depende ahora
solo de selección de objetivos y límites de aristas. La profundidad alcanzada
se informa aparte. `contextTruncation` distingue `definitionBodies`,
`textPreview`, `relatedBodies` y `summaryRelations`; el resumen declara
`relationsOmittedScope: summary_display_only`. Las identidades y los indicadores
del grafo deben coincidir entre vistas sobre el mismo índice.
También se actualizaron los mensajes de disponibilidad del indexador nativo
y de navegación local Rust/JavaScript/TypeScript. Estos cambios no activan
sincronización ni conceden permiso para escribir.





```powershell
$env:HARNESS_BENCH_ROOT = "E:\PROYECTO RUST\backend-noctalia"
cargo test --locked -p agent-harness --test stdio real_project_payload_and_log_audit -- --ignored --exact --nocapture
```
