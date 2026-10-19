# Parámetros compatibles y búsquedas acotadas

`harness_replace` usa `hash` como nombre canónico del SHA-256 y acepta `baseHash`
como alias. Usa solo uno; conserva la validación de frescura y los conflictos.
El `baseHash` devuelto por `preview:true` se puede pasar como `hash` al aplicar.

`harness_sql` permite enviar solo la consulta y sus parámetros:

```json
{"sql":"SELECT TOP (10) Id FROM dbo.Empresa"}
```

Si se envía `sql` sin `action`, se interpreta `action: "query"`. Para query, el
motor predeterminado es `sqlserver`. Un motor explícito se respeta; PostgreSQL
mantiene sus restricciones para consultas arbitrarias. Otras acciones siguen
requiriendo `action`: search requiere `query` y describe requiere `name`.

Para describir objetos, `kind` acepta `relation`/`routine` y el alias `objectType`:

```json
{"action":"describe","engine":"sqlserver","name":"dbo.MiProcedimiento","objectType":"procedure"}
```

`procedure` y `function` equivalen a `routine`; `table` y `view` equivalen a
`relation`. No envíes `kind` y `objectType` juntos. Los errores de forma indican
los campos necesarios y ejemplos estáticos sin reproducir el SQL enviado.

`harness_search` acepta `scope` o su alias `path` para limitar la búsqueda a un
archivo o directorio. `globs` (alias `includes`) filtra archivos antes de leerlos
en las modalidades `text`, `files` y `file_size`. Los patrones sin `/` se aplican
al nombre del archivo en cualquier subdirectorio. Se mantienen ignores, políticas
de rutas y presupuestos; revisa los indicadores de resultados parciales.

```json
{"kind":"text","query":"Controller","path":"Controllers","globs":["*.cs","*.cshtml"]}
```

`harness_read` acepta `path`, `file` o `target_path`. Conserva sus límites de rango:

```json
{"file":"Views/Home/Index.cshtml","start":1,"lines":30}
```

Checkpoint admite campos directos o dentro de `data`:

```json
{"action":"checkpoint","session":"SESION","data":{"goal":"Corregir vista","decisions":["Escapar arrobas literales"],"pending":["Compilar solución"]}}
```

No envíes el mismo campo con ambos nombres o en ambas ubicaciones; se rechaza
la ambigüedad. `action`, `session` y `refreshFreshness` permanecen en la raíz.

`harness_git` permite archivos, directorios y el workspace:

```json
{"action":"diff","path":".","maxLines":500,"maxChars":12000}
```

Usa `path: "src"` para un directorio y `staged: true` para los cambios del índice.
Solo muestra cambios rastreados; filtra rutas protegidas antes de obtener el patch.
`files` informa archivos incluidos, excluidos por política y omitidos por límite
(hasta 128 archivos y un presupuesto de argumentos). Las omisiones de archivos no
se recuperan desde `detail`; acota `path`. El detalle sí conserva el patch de los
archivos seleccionados cuando la vista previa supera el límite de líneas/caracteres.

En Razor se comprueban transiciones `@` en atributos, incluyendo regex `pattern`.
Escapa una arroba literal con `@@`. Se admiten expresiones habituales como
`@Model.Email` y `@(Model.Value)`, y se detectan delimitadores sin cerrar.
Esto sigue siendo un validador básico: la compilación real de Razor/C# se verifica
por separado con `harness_verify` indicando un `.csproj`/`.sln`, SDK disponible y
restore previo. No se lanza automáticamente una compilación por cada reemplazo.
