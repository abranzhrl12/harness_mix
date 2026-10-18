# Native LLM tooling preview — 0.7.1-rc.1

Prerelease Windows x86-64 del núcleo Rust. No requiere Node/node_modules y no
cambia automáticamente el backend MCP activo.

## Incluye

- Destinos exactos y motivos de truncamiento separados en explore.
- Modo compacto con firmas/relaciones principales y grafo completo por referencia.
- Bootstrap con backend, contrato 2.1.0, workflows y reutilización de contexto.
- Errores recuperables, inventario de símbolos y cobertura de parsers explícita.
- Proyecciones JSON por resultRef, Unicode seguro y redacción heurística.
- Corrección de asociaciones Rust falsas y recuperación de llamadas probadas.

## Evidencia y alcance

Las pruebas dirigidas comprueban seguridad, preservación de respuestas, redacción,
selectores exactos y schemas publicados por MCP. Los fixtures midieron 80,5 % menos
texto en navegación y 56,4 % contando el bootstrap inicial, sin afirmar el mismo
ahorro en tokens facturados o en todas las tareas.

Migración incompleta: no hay paridad total con CodeGraph Node. Usar --root explícito
y revisar los límites antes de reconstruir un índice legado. Ver README incluido
y LLM-EFFICIENCY.md en el repositorio. Los assets incluyen ejecutable, ZIP y SHA256.
