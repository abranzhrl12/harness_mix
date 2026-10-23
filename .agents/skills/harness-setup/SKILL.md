---
name: harness-setup
description: Instalar, inicializar o verificar el CLI y el servidor Harness en un proyecto. No para cambios normales de código ni mantenimiento del índice.
---

# Primer uso de Harness

Usa esta skill cuando la persona pida instalar Harness, conectar un proyecto o recuperar una conexión que aún no está configurada.

## Instalar y comprobar el CLI

- Si `agent-harness --version` ya funciona, úsalo y confirma la versión.
- Si la persona pide instalarlo, en Windows x64 usa el paquete `@abranzhrl1212/agent-harness` con la versión explícita que eligió o que figura en el registro actual: `npm install --global @abranzhrl1212/agent-harness@<version>`. Verifica después con `agent-harness --version`.
- Si el comando no existe, localiza primero una instalación conocida. No descargues ni ejecutes binarios de fuentes desconocidas.

## Preparar un proyecto

Desde una terminal, ejecuta `agent-harness init --root <ruta-absoluta-del-proyecto>`. El comando instala las skills incluidas, prepara `.agents/harness.config.json` si falta, guarda un ejemplo MCP del ejecutable que está corriendo y activa la sincronización local optativa de skills.

Revisa `.agents/harness-mcp.example.json` y copia sus valores a la configuración real del cliente elegido. El ejemplo no modifica ni registra ningún cliente. Conserva su configuración existente y añade el servidor sin sobrescribir otras entradas.

Reinicia o reconecta el cliente. Después llama `harness_bootstrap`, confirma la raíz del proyecto y reutiliza `data.session` en las llamadas posteriores. No afirmes que la conexión funciona si el bootstrap no terminó correctamente.

## Usar el servidor

- Usa Harness MCP para leer, buscar, editar y verificar código según las skills especializadas de este proyecto.
- Shell sirve para instalar o comprobar el ejecutable y preparar la entrada del cliente. No la uses para saltar un fallo de Harness, releer código fuera del MCP ni inicializar o reconstruir `.codegraph/` por rutina.
- Si un archivo local ya existe, no lo fuerces: `init` conserva configs JSON válidos y reporta conflicto para un ejemplo MCP distinto.
- Una sesión activa puede no volver a cargar skills tras la instalación; inicia una sesión nueva si hace falta.
