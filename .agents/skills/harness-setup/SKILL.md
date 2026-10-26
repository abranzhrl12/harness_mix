---
name: harness-setup
description: Instalar o retirar Harness del proyecto, conectar MCP y preparar su uso inicial. No para lectura habitual ni mantenimiento rutinario del índice.
---

# Preparar Harness

Configura el proyecto y el cliente una vez; no repitas la preparación antes de
cada lectura. Reutiliza una conexión operativa y la sesión vigente.

## Ejecutable

Comprueba la instalación conocida con `agent-harness --version`. La versión sirve
para diagnóstico; no es contexto que deba repetirse en cada respuesta.
Si el comando falta, localiza el binario existente antes de instalar.

Si se pidió instalar en Windows x64, usa
`npm install --global @abranzhrl1212/agent-harness@<version>` con la versión elegida
por el usuario o comprobada en el registro actual. No descargues binarios desconocidos.

## Configuración

Solo si se pidió instalar en el proyecto, ejecuta `agent-harness init` desde su
raíz, o `agent-harness init --root <ruta-absoluta>`. Instala las skills, configura
los MCP locales de Codex y Antigravity y prepara el índice. Si ya existe, lo
sincroniza sin borrarlo. Repetir `init` reanuda la instalación; no requiere flags
adicionales. Conserva servidores ajenos, permisos, credenciales y archivos editados.

`agent-harness uninstall` retira lo registrado en
`.agents/harness-installation.json`, restaura archivos anteriores y elimina solo
el índice creado por esa instalación. Conserva índices anteriores y archivos
modificados; informa los conflictos. No elimina el binario compartido. Si el
índice está en uso, cierra las conexiones MCP de ese proyecto antes de reintentar.

Para conectar, identifica el cliente y su archivo real. Usa el comando y argumentos
registrada por `init`; conserva otros servidores, credenciales y permisos.
La entrada stdio usa el ejecutable con `args: ["--root", "RUTA_ABSOLUTA"]`.
No abras stdio en una terminal como sustituto de registrarlo en el cliente.

## Primera conexión y sesión

Tras conectar, ejecuta `harness_bootstrap`: confirma `root` y reutiliza
`data.session` en las llamadas posteriores. En la implementación actual, bootstrap
abre o restaura la sesión; no inicializa el índice. No lo repitas por lectura:
solo al necesitar una sesión nueva o restaurar una conocida con `session`.
No declares la conexión operativa sin bootstrap correcto. Si no puedes reconectar
desde aquí, informa ese paso pendiente. Reiniciar la sesión del cliente puede ser
necesario para descubrir skills nuevas o cargar instrucciones actualizadas.

## Índice inicial, solo para relaciones

Leer y buscar código no requieren índice. Si se necesitan relaciones y el usuario
autoriza crear uno en un proyecto sin índice, ejecuta primero
`agent-harness --root <ruta-absoluta> --init-index` y después
`agent-harness --root <ruta-absoluta> --reindex`.
Si ya existe, comprueba compatibilidad antes de elegir mantenimiento admitido;
no lo borres ni repitas la inicialización. `indexed=true` solo indica que existe
`.codegraph/`, no que sea compatible, completo o vigente. La preparación inicial
no garantiza frescura tras cambios posteriores; actualizarlo requiere autorización.

## Uso posterior

Las skills enseñan el flujo; las herramientas devuelven datos y avisos de la
operación. Bootstrap no carga `AGENTS.md` ni el antiguo archivo de reglas.
Para código, consulta `harness-read` o `harness-edit` según la tarea.

Shell se usa aquí para instalación y configuración; no para eludir fallos de
Harness. No inicialices ni reconstruyas `.codegraph/` por rutina: el índice
no es requisito para leer o buscar código.
