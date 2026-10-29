# Configurar Harness para este workspace

Guía para personas y agentes de IA. La configuración activa está en
`harness-workspace.json`, junto a este documento. La plantilla
`harness-workspace.example.json` contiene ejemplos; no se carga automáticamente.

## Preparar los proyectos

1. Lee el manifiesto activo y conserva los proyectos ya registrados.
2. Identifica las carpetas reales de frontend, backend y servicios. No supongas
   que las rutas del ejemplo existen ni crees carpetas vacías para validarlas.
3. Edita `projects` en el manifiesto activo con los alias y rutas reales, o usa:

```powershell
agent-harness workspace add frontend ./frontend
agent-harness workspace add backend ./backend
agent-harness workspace add services ./services
agent-harness workspace list
agent-harness workspace check
```

Ejecuta esos comandos desde la carpeta de este documento. También puedes pasar
`--root <carpeta-del-workspace>` al final. Las rutas relativas de los proyectos
se resuelven desde la carpeta del manifiesto, incluso al usar `--root`.
Los alias admiten letras ASCII, números, guion y guion bajo. Registra únicamente
las carpetas autorizadas por el usuario; el ejemplo no concede acceso a ellas.

El JSON activo acepta este formato (ajusta las rutas antes de copiarlo):

```json
{
  "projects": {
    "frontend": "./frontend",
    "backend": "./backend",
    "services": "./services"
  }
}
```

No añadas comentarios al JSON. Puedes registrar uno, dos o más proyectos.
`workspace init` conserva el manifiesto y los documentos existentes.
Un manifiesto vacío necesita proyectos antes de arrancar el servidor.

## Conectar el cliente MCP

Usa el ejecutable global `agent-harness`. Si el cliente lo inicia con esta
carpeta como directorio de trabajo, el arranque sin argumentos descubre el
manifiesto. Si el cliente no transmite esa carpeta, configura sus argumentos:

```json
{
  "command": "agent-harness",
  "args": ["--workspace-config", "C:/ruta/real/harness-workspace.json"]
}
```

Este es el objeto del servidor; el contenedor depende del cliente MCP. Usa una
ruta absoluta real y el ejecutable o lanzador compatible con ese cliente.
`--root <proyecto>` selecciona un solo proyecto y omite el descubrimiento.
Los cambios del manifiesto se recargan entre llamadas MCP, sin reiniciar, con
una comprobación como máximo cada 250 ms y sin sondeo en reposo. Si el JSON queda
inválido, se conserva la última configuración válida. Comprueba los proyectos y
el campo `workspaceReload` mediante `harness_bootstrap` con la misma sesión.

## Trabajar desde el agente

Inicia con `harness_bootstrap`, comprueba las raíces devueltas y reutiliza
`data.session`. Usa rutas como `frontend:/src/...` y `backend:/src/...`.
Las búsquedas sin scope pueden consultar todos los proyectos; si una ruta es
ambigua, indica su alias. Para crear archivos utiliza siempre `alias:/ruta`.

Cada proyecto conserva su índice `.codegraph` y su configuración local.
Registrar un proyecto no construye ni actualiza su grafo. `workspace check`
informa de la existencia del índice, sin certificar compatibilidad ni frescura.
Lee archivos sin índice cuando sea suficiente. No inicialices ni reconstruyas
índices sin autorización del usuario. Respeta los AGENTS.md de cada proyecto.
Los grafos son independientes: no infieras automáticamente relaciones HTTP o
eventos entre repositorios como si fueran enlaces confirmados del grafo.

## Flujo de desarrollo y Superpowers

Si las skills están instaladas en el proyecto, lee
`.agents/skills/harness-workflow/SKILL.md` al implementar una función o corregir
un fallo. Enlaza las copias locales de Superpowers: diagnóstico sistemático para
hallar la causa, TDD antes de implementar comportamiento y verificación con
evidencia antes de declarar el trabajo terminado. Harness proporciona las
herramientas; las skills guían el método. Carga cada etapa cuando corresponda.

Para instalar o actualizar las skills sin reconstruir índices, usa
`agent-harness --root <proyecto> skills sync` dentro del alcance autorizado.
Revisa los conflictos; conserva las instrucciones locales modificadas. La
instalación de skills y su detección por el cliente son pasos distintos.

`workspace init` no instala las skills dentro de los proyectos registrados.
Para habilitar la sincronización al arrancar y al incorporar el proyecto, usa
`agent-harness --root <proyecto> skills sync --enable-auto` donde esté autorizado.
Sin esa preferencia o con ella desactivada, el servidor no escribe skills.
Conserva las ediciones locales que aparezcan como conflictos. No uses `agent-harness init`
solo para obtener skills: además configura entradas MCP para un proyecto y
prepara su índice. Si quieres una única conexión multiproyecto, conserva la
entrada que usa `--workspace-config`.

Lee una sola vez cada guía necesaria durante la tarea, aunque sus enlaces se
crucen. Antes de cambiar de proyecto, comprueba su alias y sus AGENTS.md; no
traslades las reglas de frontend a backend automáticamente. Informa por proyecto
qué cambió y qué verificaciones pasaron o siguen pendientes.
