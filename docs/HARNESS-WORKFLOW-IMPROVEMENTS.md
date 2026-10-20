# Verificación y contexto de trabajo

## ASP.NET Framework clásico

`harness_verify` acepta `engine: "auto" | "dotnet" | "msbuild"` para .NET.
El modo automático inspecciona el proyecto, o los proyectos de una solución, para
detectar el GUID de aplicación web clásica y el import de
`Microsoft.WebApplication.targets`. Estos proyectos usan MSBuild de Visual Studio,
localizado con `vswhere`. Si falta esa instalación, devuelve un diagnóstico;
la detección no instala componentes.

```json
{"language":"dotnet","action":"build","scope":"FAST.csproj","engine":"auto","warnings":"changed-files"}
```

El resultado incluye `passed`, `errors`, `warnings`, `newWarnings` y
`changedFilesWarnings`. `warnings` selecciona la salida visible:
`all` (predeterminado), `changed-files` o `none`.
Los totales siguen contando todas las advertencias observadas.
`newWarnings` es null sin una verificación completa anterior comparable de la
misma sesión, proyecto, motor y acción. No significa comparación con HEAD.
La salida parcial conserva la incertidumbre en vez de declarar cero.
Los comandos no restauran paquetes automáticamente.

## Búsqueda

`harness_sql` con `action: "search"` busca todos los esquemas autorizados cuando
`schema` se omite, es null o es `"*"`. Un nombre limita la búsqueda a ese esquema.
PostgreSQL conserva `allowedSchemas`.

`harness_search` acepta `scope` omitido, null, `"."` o `""` para la raíz.
Una búsqueda parcial incluye `skippedFiles: [{path, reason}]`.
`unvisitedFiles: true` indica que el corte impidió enumerar todos los archivos
restantes; esa lista no certifica cobertura total.

## Batch

```json
{
  "jobs": [
    {"tool":"harness_search","arguments":{"kind":"files","query":"Controller","scope":"."}},
    {"tool":"harness_git","arguments":{"action":"status"}}
  ],
  "maxConcurrent": 4,
  "session": "<data.session>"
}
```

Se aceptan entre uno y cuatro trabajos de lectura independientes.
`args` es alias de `arguments`; no se pueden usar ambos en el mismo trabajo.
`maxConcurrent` admite 2 a 4 y vale 2 por defecto.
Siguen vigentes los límites globales de operaciones y búsquedas.
El bootstrap incluye un ejemplo completo.

## Referencias de detalle

Las referencias creadas con sesión se conservan durante la vida del proceso
del servidor. Leer páginas no consume la referencia. `nextCall` conserva
referencia, sesión y desplazamiento; `expiresAt: null` indica retención de sesión.
Las referencias sin sesión tienen `expiresAt` en segundos Unix y pueden
descartarse por capacidad.

El almacén tiene un límite de 64 MiB y 1024 entradas. Al llenarse, rechaza
nuevos detalles antes de descartar referencias de sesión existentes;
`omitted.detailUnavailable` informa el rechazo.
Reiniciar el servidor no restaura los detalles.

## Git

```json
{"action":"diff","path":".","comparison":"auto"}
```

`auto` devuelve las comparaciones `HEAD:index`, `index:worktree` y
`HEAD:worktree` por separado. También se puede seleccionar una explícitamente.
El parámetro anterior `staged` sigue funcionando. Un diff de worktree vacío
advierte cuando hay cambios staged.

`status` con sesión añade `sessionChanges` para los archivos editados por Harness,
incluidos los ignorados por Git, con `path`, `gitStatus`, `xy` y `persisted`.
Un archivo persistido e ignorado sigue necesitando una decisión explícita para
incluirlo en un commit.
