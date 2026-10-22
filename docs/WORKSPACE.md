# Workspace de varios proyectos

Harness puede exponer varios proyectos por una sola conexión MCP. Cada raíz mantiene su propio repositorio, configuración e índice .codegraph. El servidor usa una sesión MCP y una tabla de resultados compartidas para que data.session y las referencias de detalle sigan funcionando al cambiar de proyecto.

## Manifiesto

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
