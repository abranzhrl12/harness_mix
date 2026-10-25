---
name: harness-read
description: Leer e investigar código con Harness MCP. Usa explore para entender cómo se conecta una funcionalidad, sus consumidores y dependencias; symbols para estructura o fuente exacta. Evita búsquedas redundantes y lecturas fragmentadas. No para SQL, XLSX ni SVG.
---

# Lectura eficiente con Harness

Usa los nombres y esquemas reales anunciados por la conexión. Si aún no hay sesión, ejecuta bootstrap, comprueba la raíz y reutiliza `data.session`.

## Elige según lo que falta

| Lo que necesitas | Llamada directa |
| --- | --- |
| Entender una funcionalidad, componente o flujo entre archivos | `harness_explore` si tienes un selector válido |
| Encontrar consumidores y dependencias | `harness_explore` |
| Archivo desconocido | `harness_search`, `kind=files` |
| Texto o mensaje de error | `harness_search`, `kind=text`, contexto acotado |
| Implementación conocida | `harness_symbols`, `action=read`, `path`, `symbol`, `view=full` |
| Solo contrato o ubicación | Misma lectura, `view=signature` |
| Elegir entre símbolos de un archivo | `harness_symbols`, `action=outline` |
| Callers, callees o impacto | `harness_explore` con archivo y símbolo; consulta el esquema de `view` |
| Texto sin símbolo o formato no soportado | `harness_read`, rango acotado |
| Contenido omitido | `harness_detail`, con el `ref` recibido |
| Campos concretos de un resultado grande | `harness_data_query`, proyección JMESPath |

No conviertas la tabla en una secuencia: un símbolo conocido no necesita búsqueda, outline ni firma previos. Para editar, pide el cuerpo exacto directamente. Las lecturas devuelven el fragmento limpio con su `range: [inicio, fin]`.

Ejemplo de lectura de implementación, sustituyendo archivo, símbolo y sesión:

```json
{"action":"read","path":"src/modulo.rs","symbol":"validar","view":"full","session":"SESION"}
```

## Ruta directa según intención

Para preguntas como “cómo funciona el perfil”, “quién usa este componente” o “qué archivos afecta este cambio”, prioriza `explore`: evita reconstruir las relaciones abriendo archivos uno por uno.

- Si no tienes ubicación, realiza una búsqueda específica, elige un candidato y explóralo. No encadenes búsquedas genéricas por sinónimos cuando ya encontraste el objetivo.
- Usa nombres y selectores reales: no presupongas que el nombre de un componente Vue coincide con un símbolo indexado. Usa `query` u otros parámetros únicamente si el esquema conectado los admite.
- Path y símbolo conocidos, para leer o modificar: usa `symbols` directamente; no consultes el grafo si no necesitas relaciones.
- Path y símbolo conocidos, para llamadas, dependencias o impacto: usa `explore` directamente. No antepongas búsqueda ni `symbols` como requisito. Si solo conoces el path y falta seleccionar el símbolo, usa outline o una búsqueda local acotada.
- Sin path ni contexto suficiente: busca candidatos primero. Después elige `symbols` para fuente o `explore` para relaciones; no recorras obligatoriamente ambas herramientas.

Para una primera exploración compacta, sustituye archivo, símbolo y sesión por valores reales y comprueba el esquema anunciado:

```json
{"filePath":"src/Comprar.vue","symbol":"comprar","view":"summary","maxDepth":1,"maxFiles":2,"maxChars":2000,"session":"SESION"}
```

Amplía profundidad o cuerpos solo cuando falte evidencia. `maxFiles` limita las fuentes cargadas, no certifica que el grafo tenga ese número de nodos. Para impacto entrante usa `view=impact`; no presupongas que admite los mismos presupuestos que summary.

`summary` orienta; no sustituye los cuerpos. Lee las relaciones y ubicaciones devueltas y recupera únicamente las implementaciones necesarias. No repitas una exploración sin cambiar qué evidencia necesitas.

Selecciona los candidatos relevantes antes de explorar. Agrupa exploraciones independientes con `harness_batch`, `maxConcurrent=2..4` y `syncIndex=false` explícito cuando su esquema lo admita; no explores automáticamente todas las coincidencias ni solicites sincronización por rutina.

Reutiliza hashes y comprobaciones ya obtenidos por archivo cuando sigan vigentes. Solo usa pistas del índice si la conexión realmente las devuelve: un hash local o `syntaxOnly=true` no certifica el grafo. Revisa `fileFreshness`, cobertura y omisiones de `explore`; una coincidencia observada no demuestra completitud ni todas las relaciones. No asumas que la actualización en segundo plano ya terminó.

## Menos viajes, misma evidencia

- Reserva `harness_read` para texto sin símbolo, formatos no soportados o rangos concretos necesarios. Evita leer 100 líneas y después el remanente por costumbre: pide el símbolo completo o un rango suficiente según el tamaño conocido y los límites anunciados.
- Agrupa solo lecturas independientes con `harness_batch`, `maxConcurrent=2..4`, si el esquema las admite. No adivines un símbolo para evitar una dependencia real.
- Conserva fuente, documentación, rutas, relaciones y límites. No releas evidencia vigente ya recibida; si hay indicios de cambios concurrentes, comprueba frescura con la capacidad anunciada o relee el fragmento afectado.
- Recupera cuerpos o detalles omitidos que puedan cambiar la conclusión. Una salida vacía limitada no prueba ausencia; un índice no prueba cobertura completa.
- No reindexes ni sincronices por rutina: lectura y búsqueda pueden funcionar sin índice; las relaciones necesitan uno compatible.
- Ante transporte, timeout o runtime fallido, diagnostica con doctor y comunica el bloqueo. No repitas equivalentes ni saltes a shell para eludirlo.

## Salidas grandes y detalles

- Comprueba si la respuesta está completa antes de solicitar más contenido.
- Usa `harness_detail` para contenido relevante omitido y `harness_data_query` para campos concretos.
- Si un batch devuelve referencias o marcadores en lugar de resultados inline, recupera los resultados necesarios; no repitas las llamadas originales.
- No leas buffers auxiliares cuando la respuesta inline ya contiene la evidencia.
- Si el cliente externaliza o trunca la salida y no tienes otra vía anunciada para recuperarla, lee solo la parte necesaria mediante la capacidad real del cliente.
- No reduzcas tanto el presupuesto que provoques varias recuperaciones previsibles.
- Si el grafo está obsoleto, no disponible o no resuelve el objetivo, usa fuente local para avanzar y declara la limitación. Esto no autoriza eludir fallos de transporte o runtime.

## Criterio de cierre

Antes de otra lectura, identifica qué pregunta concreta sigue sin respuesta. Si la evidencia actual basta, termina.

Evalúa eficiencia por llamadas, tokens, tiempo y corrección; no por cumplir una cantidad fija de herramientas. El recorrido habitual para comprender una funcionalidad es localizar si hace falta, explorar sus relaciones y leer los cuerpos relevantes, no abrir archivos sucesivamente para reconstruir el grafo. Informa limitaciones relevantes.
