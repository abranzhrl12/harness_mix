# HarnessMix: chat nativo mediante ACP

`harnessmix` es un cliente de terminal escrito en Rust. Se comunica por ACP con
el servidor oficial de Antigravity; no abre la interfaz de `agy` ni usa el SDK de
Python. La autenticación la realiza el agente oficial con la cuenta de Google.

```powershell
harnessmix config
harnessmix login
harnessmix active --root "E:/mi-proyecto"
```

En `active`, selecciona Antigravity con `1`. Se intenta abrir el chat usando la
autenticación guardada por el servidor oficial, sin volver a pedir login.
`/menu` abre estas opciones:

| Opción | Acción |
| --- | --- |
| `1` | Iniciar sesión con Google |
| `2` | Abrir una sesión nueva |
| `3` | Ver configuración |
| `4` | Cambiar permisos |
| `5` | Salir |
| `0` | Volver al chat |

También puedes escribir `/login`, `/new`, `/config` o `/exit`. Para una
suscripción personal, elige el inicio de sesión con Google; no necesitas crear
una API key. Completa el flujo del navegador. `/exit` cierra el proceso sin
cerrar la sesión de Google. Si crear una sesión falla, `/new` permite reintentar
sin cambiar tu autenticación.

Al completar el login o abrir una sesión nueva vuelves al prompt `Tú >`.
No pulses `1` repetidamente mientras conecta: el indicador muestra la fase y
los segundos transcurridos. Conectar al servidor y abrir la sesión son fases
distintas; esta última también prepara la integración MCP del proyecto.

El prompt `Tú >` identifica tu entrada. Mientras Antigravity responde puedes
seguir escribiendo; los mensajes se envían en orden, uno por uno, al terminar
la respuesta activa. Usa `/cancel` o Ctrl+C para cancelar la respuesta actual.

Las solicitudes de permisos muestran las opciones que ofrece el agente.
`0`, una entrada inválida o el fin de la entrada cancelan la solicitud. Las
respuestas se muestran progresivamente por segmentos: las líneas de texto
aparecen conforme llegan y las líneas largas se dividen en puntos seguros.
Las tablas y bloques de código se agrupan hasta cerrar su estructura; el texto
pendiente también se muestra al finalizar, cancelar o antes de una herramienta.
Los pensamientos y mensajes JSON del protocolo no se imprimen en el chat.

## Modelo, esfuerzo y contexto

`/model` y `/effort` muestran únicamente los selectores que el servidor ACP
anuncia para la sesión. Usa `/model 1`, `/effort 2` o el id exacto mostrado
para cambiar una opción. No se presuponen modelos ni niveles disponibles.
Si el servidor no anuncia el selector, se muestra «No disponible».
Los cambios solicitados durante una respuesta se aplican al terminar el turno.

`/usage` muestra la última ocupación de contexto informada (`used` / `size`)
y el coste acumulado solo si el servidor lo comunica. No es el número de tokens
facturados por respuesta ni la cuota restante de la suscripción Google.
Sin eventos de uso se muestra «no disponible», nunca un cero inventado.

## Presentación Markdown

La terminal interactiva usa Termimad para estilos, tablas y ajuste de ancho.
En salida no interactiva, `TERM=dumb` o con `NO_COLOR`, se conserva Markdown
plano sin estilos ANSI. El texto recibido se filtra para impedir que secuencias
de control ajenas al renderizador muevan el cursor o modifiquen la terminal.

`src/harnessmix/rendering.rs` define el puerto `MessageRenderer`, sus adaptadores
Termimad y plano, y la detección del ancho. La consola recibe el renderizador
por inyección; ni dominio, ni aplicación, ni transporte ACP dependen de Termimad.
Cambiar el formateador no requiere cambiar esos componentes. Migrar a una TUI
completa con Ratatui sí requiere un adaptador de pantalla/entrada adicional:
no basta con sustituir una función de Markdown.

Por defecto, HarnessMix pregunta antes de responder a cada solicitud de
permiso ACP. Puedes cambiar la política solo para la sesión actual con
`/permissions auto` y volver al modo interactivo con `/permissions ask`.
También puedes iniciar la sesión en modo automático con:

```powershell
harnessmix active --yolo --root "E:/mi-proyecto"
harnessmix active --dangerously-skip-permissions --root "E:/mi-proyecto"
```

`--yolo` es un alias de `--dangerously-skip-permissions`. En ese modo se
aprueban automáticamente las opciones de permiso ACP que conceden acceso,
incluidas las que permiten ejecutar comandos o editar archivos. Si la solicitud
no ofrece una opción que conceda permiso, se cancela. La política cambia solo
para la sesión actual y no se guarda en una configuración global. Esta opción
aprueba las solicitudes de permisos del agente de forma indiscriminada; no
equivale a `agy --mode auto` ni ofrece el sandbox o evaluador de ese modo.

La animación del indicador puede desactivarse al iniciar con
`--no-animation`. `harnessmix config --no-animation` muestra su estado sin
iniciar una sesión.

## Adjuntar imágenes

Usa `/image <ruta>` para adjuntar un archivo PNG, JPG, GIF o WEBP de hasta
20 MiB a tu siguiente solicitud. La imagen se prepara mientras puedes seguir
escribiendo. Pegar una imagen del portapapeles con Ctrl+V no está soportado.

## Servidor oficial

El servidor ACP es un componente distinto del CLI `agy`. En Windows se necesita
`agy_acp_server.exe` y su compañero `localharness_external.exe`, de la misma
distribución y conservados juntos. El cliente busca el servidor en:

1. `--agent-command <ruta>` o `HARNESSMIX_ANTIGRAVITY_ACP`.
2. Los directorios absolutos de `PATH`.
3. `%LOCALAPPDATA%/harnessmix/agents/antigravity/1.4.3/agy_acp_server.exe`.

Una ruta configurada explícitamente tiene prioridad: si no existe, el cliente
informa que falta en lugar de ejecutar otro agente. `--agent-arg <argumento>`
permite pasar argumentos adicionales sin un intérprete de comandos.

El registro oficial publica los archivos por plataforma y sus hashes:
<https://github.com/agentclientprotocol/registry/blob/main/antigravity-acp/agent.json>.
La instalación inicial de Windows x64 usa la distribución 1.4.3 de
`dl.google.com`, cuyo SHA-256 del ZIP es
`54c2e16378d9a4952e1da70e355f3c33c4f284308d6de48ab3e6e34a127bd325`.
El cliente no descarga ni actualiza ejecutables automáticamente.

## Harness y arquitectura

La sesión recibe Harness MCP por stdio, utilizando el `agent-harness` instalado
junto a `harnessmix` o encontrado en PATH y el proyecto seleccionado con
`--root`. `--no-harness` permite iniciar sin esta integración. Esto no modifica
la configuración global de Antigravity ni inicializa índices del proyecto.

- `crates/domain/src/conversation.rs`: eventos, autenticación y decisiones de
  permisos, sin JSON-RPC ni procesos.
- `crates/application/src/ports/conversation.rs`: puerto inyectable del proveedor.
- `crates/application/src/conversation.rs`: casos de uso de conversación.
- `src/adapters/acp/`: traducción ACP, proceso oficial y transporte stdio.
- `src/harnessmix/console.rs`: presentación y entrada de terminal.
- `src/harnessmix/config.rs`: composición de dependencias y opciones del comando.

El comando previo `agent-harness active` conserva su función de lanzar la
interfaz de `agy`. `harnessmix active` es el cliente ACP con interfaz propia.

## Hooks del proyecto

`harnessmix hook profile` instala el perfil local de hooks PreToolUse en
`.agents/hooks.json`. El perfil mantiene disponibles las herramientas MCP y
bloquea `view_file`, `run_command` y las herramientas nativas de archivo que
enumera. Solo afecta a este proyecto; no cambia configuraciones globales. El
archivo usa el helper interno `harnessmix hook deny-native`, ejecutable nativo
sin dependencia de PowerShell ni Node.js. Ese helper es para el hook y no es
un comando para escribir en el chat.

## Desarrollo

```powershell
cargo build --locked --bin harnessmix
cargo test --locked -p harness-application conversation::
cargo test --locked --bin harnessmix
cargo test --locked --lib adapters::acp::tests
cargo test --locked --test harnessmix
```

Las pruebas automatizadas usan proveedores falsos y no necesitan credenciales
ni consumen cuota de modelos. La compatibilidad real de autenticación y
herramientas debe verificarse también con el servidor oficial instalado.
