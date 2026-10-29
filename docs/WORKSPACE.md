# Workspace de varios proyectos

Harness puede exponer varios proyectos por una sola conexión MCP. Cada raíz mantiene su propio repositorio, configuración e índice .codegraph. El servidor usa una sesión MCP y una tabla de resultados compartidas para que data.session y las referencias de detalle sigan funcionando al cambiar de proyecto.

## Manifiesto

### Configuración desde la terminal

Instala el ejecutable global una sola vez. Desde la carpeta que agrupa tus proyectos:

~~~powershell
agent-harness workspace init
agent-harness workspace add frontend ./frontend
agent-harness workspace add backend ./backend
agent-harness workspace add services ./services
agent-harness workspace list
agent-harness workspace check
~~~

Estos comandos administran `harness-workspace.json` y no crean índices ni modifican la configuración de los clientes MCP. `init` conserva un manifiesto existente; `add` rechaza reemplazar un alias con otra raíz. Las rutas de `add` se interpretan desde la carpeta del workspace. Usa `--root <carpeta>` al final de cualquiera de estos comandos para seleccionar esa carpeta explícitamente.

Por defecto, `workspace init` también crea `harness-workspace.example.json`, con ejemplos de frontend, backend y servicios, y `HARNESS-WORKSPACE.md`, con instrucciones para personas y agentes de IA. Conserva esos archivos si ya existen. Los ejemplos no se activan: edita el manifiesto activo con las rutas reales o usa `workspace add`. La guía se incluye dentro del ejecutable y no requiere descargar documentación.

El servidor recarga el manifiesto entre llamadas MCP, sin reiniciar. Comprueba cambios al recibir herramientas, como máximo cada 250 ms; no mantiene un sondeo de disco en reposo. Cada operación conserva su configuración inicial. Si el JSON es inválido o una raíz no existe, conserva la última configuración válida y expone el error en `workspaceReload` del bootstrap y en stderr. Los manifiestos se limitan a 1 MiB. Confirma los proyectos activos con `harness_bootstrap` reutilizando tu sesión.

`workspace init` prepara las skills de proyectos registrados y `workspace add` prepara el proyecto agregado. Al arrancar o recargar el manifiesto, el servidor también prepara proyectos registrados directamente en JSON. Se crea `.agents/harness-skills-sync.json` con `{"format_version":1,"auto_sync":true}` si no existe; cambia `auto_sync` a `false` para desactivar la sincronización de ese proyecto. Una preferencia existente se respeta. Los cambios se aplican en el siguiente arranque o reingreso; no hay sincronización periódica. Los conflictos conservan las ediciones locales y se informan en el resultado del comando o en stderr sin impedir el acceso al proyecto. No se crean índices ni se configuran clientes. El cliente debe descubrir las skills por separado.

Al retirar un proyecto se rechazan operaciones nuevas dirigidas a él y se detiene su watcher de índices. Las operaciones ya admitidas usan su configuración anterior. Sus trabajos siguen disponibles por ID para consultar o cancelar, y los recursos se conservan en caché durante la conexión para permitir recuperarlos al volver a registrar la raíz. El cierre del servidor drena los gestores de trabajos de todos los proyectos que utilizó. El almacén de sesiones, logs y navegador queda anclado al primer proyecto inicial, aunque después cambie el orden del manifiesto.

Sin argumentos, el servidor busca `harness-workspace.json` desde el directorio actual hacia sus padres y utiliza el más cercano. `--persist-logs` también permite este descubrimiento. Un manifiesto inválido produce un error; no se ignora para abrir otro proyecto. `--root <proyecto>` mantiene la selección explícita de un solo proyecto; `--workspace-config <archivo>` mantiene la selección explícita de un manifiesto.

El cliente MCP debe iniciar `agent-harness` con el directorio de trabajo del workspace. Si el cliente no permite fijarlo o no transmite esa carpeta, configura `--workspace-config` con la ruta absoluta. La ubicación del ejecutable global y la selección del workspace son independientes; no se deduce el proyecto desde la carpeta del ejecutable.

`workspace check` comprueba los alias y que sus carpetas existan. Informa si encuentra `.codegraph/codegraph.db`, pero no certifica compatibilidad ni frescura de los grafos. Un workspace vacío necesita al menos un `workspace add` antes de servirlo.

Crea un archivo JSON, por ejemplo E:/work/noctalia/harness-workspace.json:

~~~json
{
  "projects": {
    "backend": {
      "root": "E:/work/noctalia/backend"
    },
    "frontend": {
      "root": "E:/work/noctalia/frontend"
    }
  }
}
~~~

Las rutas relativas se resuelven desde la carpeta del manifiesto. Los nombres pueden contener letras ASCII, números, guion y guion bajo. Todas las raíces deben existir. El manifiesto registra los proyectos que el agente puede usar.

Harness guarda las sesiones persistentes y el archivo de logs en el primer proyecto del manifiesto por orden alfabético del alias. Los índices y los trabajos de verificación en segundo plano siguen siendo propios de cada proyecto.

Inicia el servidor con:

~~~powershell
agent-harness --workspace-config "E:/work/noctalia/harness-workspace.json"
~~~

En la configuración MCP del cliente, pasa --workspace-config y la ruta absoluta al manifiesto como argumentos del ejecutable. Reinicia la conexión y confirma el catálogo con harness_bootstrap.

## Elegir un proyecto

Las búsquedas de texto y archivos sin scope consultan todos los proyectos. Cada resultado usa una ruta calificada que puede pasarse directamente a harness_read, harness_replace, harness_write, harness_move, harness_symbols, harness_explore o harness_verify:

~~~json
{"kind":"text","query":"HealthController","contextLines":3}
~~~

Un resultado puede incluir frontend:/src/pages/index.vue. También puedes indicar la ruta de forma explícita:

~~~json
{"path":"backend:/src/main.rs","start":1,"end":40}
~~~

Las rutas relativas sin prefijo se aceptan cuando el archivo o carpeta existe en un solo proyecto. Si aparece en varios, Harness devuelve un error de ambigüedad con la instrucción de usar alias:/ruta. Para crear archivos nuevos o destinos de movimiento, usa siempre un prefijo, por ejemplo frontend:/src/components/NewCard.vue.

harness_explore consulta todos los grafos cuando no recibe filePath, y devuelve resultados identificados por proyecto. Si recibe filePath, consulta el grafo que corresponde a esa ruta. Los grafos permanecen separados; las relaciones entre una llamada frontend y un endpoint backend no se infieren automáticamente.

Las verificaciones seleccionan el proyecto con scope, por ejemplo frontend:/ o backend:/src. harness_jobs aplica la misma selección en verify.scope; los listados y eventos de trabajos incluyen el nombre del proyecto.

## Índices

El workspace usa los índices existentes en cada raíz y el mantenimiento automático actúa sobre cada proyecto con un índice compatible. No hace falta combinarlos ni mover .codegraph. Para reconstruir un índice, utiliza el comando CLI existente con la raíz explícita del proyecto.
