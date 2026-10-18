# Agent Harness 0.7.1-rc.1 — Windows x86-64

Prerelease nativa para Windows x86-64 (MSVC). No requiere Node ni node_modules.
No reemplaza automáticamente una conexión MCP existente ni configura credenciales.
Probada en el host de compilación; no certifica todas las versiones de Windows ni
el MSRV declarado por Cargo. Si Windows informa de un runtime MSVC faltante,
instalar el redistribuible oficial Microsoft Visual C++ x64.

## Uso

Descomprimir en una carpeta local y comparar `Get-FileHash archivo.zip -Algorithm
SHA256` con el asset `*-SHA256SUMS.txt`. Adaptar las dos rutas de
`mcp.example.json` y copiar esa entrada a la configuración MCP del cliente.

```powershell
.\agent-harness.exe --version
.\agent-harness.exe --root 'E:\ruta\al\proyecto'
```

El segundo comando inicia el servidor MCP por stdin/stdout; configurarlo como
comando del cliente MCP, con `--root` explícito para evitar ambigüedad de workspace.
No usar la salida stdio como consola de logs ni activar dos escritores de índice.

Para un proyecto nuevo sin índice:

```powershell
.\agent-harness.exe --root 'E:\ruta\al\proyecto' --init-index
.\agent-harness.exe --root 'E:\ruta\al\proyecto' --reindex
```

No apuntar estas operaciones a un índice legado asumiendo compatibilidad total.
El rebuild tiene presupuesto de cinco minutos, límites de alcance y transacciones
por archivo: un fallo puede dejar commits parciales confirmados, que se reportan.
No se desactivan controles de seguridad para completar un índice incompatible.

## Cambios y límites

- Contrato de herramientas 2.1.0: routing por tarea y capacidades reales.
- Explore con filePath/container/symbolId exactos y view=summary opcional.
- Respuestas pequeñas con relaciones principales y grafo recuperable por ref.
- Bootstrap reutilizado evita repetir el catálogo; el contexto completo se recupera.
- data_query: proyecciones JSON/resultRef, Unicode seguro y redacción heurística.
- Navegación de símbolos: Rust/JavaScript/TypeScript; no todos los lenguajes.
- No hay paridad completa con CodeGraph Node ni certificación de frescura global.
- Los porcentajes de LLM-EFFICIENCY.md miden texto en fixtures, no tokens facturados.
- La redacción por claves/patrones no es detección universal de secretos.

La configuración de SQL Server permanece local y deshabilitada/no conectada si no
está preparada; no se incluyen secretos, índices, logs o estado de sesiones.

El código del kernel CodeGraph conserva su licencia MIT en CodeGraph-LICENSE.txt.
ROLLDOWN-LICENSE.txt conserva los avisos de Rolldown. Cargo.lock y
dependencies.json identifican las dependencias y sus licencias declaradas en
metadatos; no constituyen una auditoría legal ni conceden una licencia general
al proyecto. No se incluye código fuente, configuración local ni estado.

## Generar y verificar el paquete

Desde el repositorio, `powershell -File scripts/package-release.ps1` compila con
Cargo.lock y genera ZIP y checksum en target/dist. Para reutilizar una compilación:
`powershell -File scripts/package-release.ps1 -SkipBuild -BinaryPath RUTA_EXE`.
El script conserva el staging y falla si ya existen artefactos de esa versión.
Node solo se usa en desarrollo para `node scripts/smoke-mcp.cjs RUTA_EXE`; no es
una dependencia de ejecución del paquete. CI prueba el workspace, extrae el ZIP
y comprueba el protocolo MCP real. Los artefactos se suben a CI sin publicación
automática ni afirmación de compatibilidad con todos los clientes MCP.
