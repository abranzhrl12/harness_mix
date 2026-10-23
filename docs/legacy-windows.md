# Proyectos legacy en Windows

El indexador conserva el hash de los bytes originales y decodifica las fuentes
como UTF-8 estricto o, si falla, Windows-1252. No convierte archivos en disco.
UTF-16/32, NUL y bytes no decodificables siguen produciendo un diagnóstico;
el fallback no garantiza reconocer cualquier archivo binario como tal.

Las declaraciones JavaScript repetidas con la misma identidad se consolidan.
Si el análisis semántico no puede demostrar sus relaciones, se conservan los
nodos recuperables y se informa cobertura parcial; un fallo SQLite no se oculta
como una advertencia del parser.

`available.msbuild` usa el mismo descubrimiento de MSBuild que `harness_verify`,
incluido `vswhere` y la selección de instalaciones con WebApplication.targets.
Visual Studio o sus Build Tools deben estar instalados. Esto no añade un runner
de VSTest ni garantiza un tiempo fijo de compilación.

Las aperturas del índice esperan hasta cinco segundos por conflictos de bloqueo
de Windows o SQLite. Los errores ajenos al bloqueo conservan su diagnóstico;
un proceso que mantenga el bloqueo indefinidamente todavía provocará un timeout.

Razor enlaza helpers MVC existentes y ahora reconoce `Url.Content("~/ruta.js")`
hacia un archivo JavaScript ya indexado. Las rutas dinámicas, inseguras o
ambiguas quedan sin resolver; no se añade extracción CSS mediante este helper.

Un constructor `new SqlCommand("nombre", ...)` puede enlazarse con una definición
única `CREATE [OR ALTER] PROC[EDURE]` en un `.sql` local. El grafo añade nodos
`stored_procedure` y `table`, con referencias a tablas declaradas por `CREATE TABLE`
y usadas literalmente en FROM, JOIN, UPDATE o INTO. Los metadatos y la tabla de
archivos conservan hashes originales de la evidencia SQL. No se consulta la base
remota ni se infieren nombres dinámicos, temporales o entre bases de datos.
Los nombres sin esquema se resuelven solo si hay una definición local única.
Desde el método C#, usa `harness_explore` con `maxDepth: 2` para incluir también
las tablas: la llamada al procedimiento es el primer salto y sus referencias
son el segundo. `view: "full"` incluye las definiciones SQL locales.

El inventario SQL es por operación, respeta exclusiones y no sigue symlinks:
máximo 512 archivos, 1 MiB por archivo, 16 MiB totales y 100000 entradas visitadas.
Si no puede completarse, no establece nuevos vínculos a procedimientos.
Los eventos `.sql` del watcher y la sincronización explícita de un `.sql`
reevalúan hasta 256 fuentes C# dependientes; superar ese límite requiere una
operación más amplia. Sin watcher activo, se debe sincronizar explícitamente el
SQL o forzar la republicación del C# llamador para actualizar esas relaciones.
