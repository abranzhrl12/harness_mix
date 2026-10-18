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
