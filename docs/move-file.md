# Mover y renombrar archivos

`harness_move` mueve un archivo regular dentro del workspace. El contrato para
el agente solo requiere dos rutas relativas:

```json
{
  "from": "src/old.ts",
  "to": "src/components/new.ts"
}
```

`session` es opcional. La herramienta está disponible en los perfiles `full` y
`edit`, queda excluida de `investigate` y no se permite dentro de `harness_batch`.

## Garantías y límites

- Esta versión funciona en Windows, dentro del mismo volumen y con archivos
  regulares. No mueve directorios ni crea carpetas padre.
- El destino debe estar libre. Se rechazan también destinos creados por otro
  proceso entre la validación y la publicación. Un nombre intermedio libre permite
  realizar un renombre que solo cambia mayúsculas/minúsculas en dos llamadas.
- Se rechazan rutas fuera del workspace, credenciales, rutas protegidas de Git,
  `.agents`, `.codegraph` y `AGENTS.md`, además de symlinks, reparse points,
  hardlinks y archivos de solo lectura. Las rutas admiten hasta 1000 bytes.
- No se interpreta ni reescribe el contenido: pueden moverse archivos binarios.
  No se reescriben imports ni se valida la sintaxis según la extensión nueva.
- El movimiento comparte la exclusión de escritura con `harness_replace`.
  Se fijan ambos árboles de directorios y se abre el origen con acceso de
  renombrado, permitiendo lectores pero bloqueando escrituras y renombres
  competidores. Se renombra el handle abierto, no una ruta sustituible.
- Se usa `SetFileInformationByHandle` con `FileRenameInfo` y
  `ReplaceIfExists=false`. No hay fallback de copia/borrado entre volúmenes.
  [Semántica oficial de FILE_RENAME_INFO](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_rename_info).

## Resultado e índice

`changed: true` y `commitStatus: "committed"` confirman el movimiento. `verified`
comprueba que el destino conserva la identidad del archivo y que el origen ya no
existe. Una cancelación posterior al commit no se presenta como si nada hubiera
ocurrido. Si el postcheck falla, se devuelve el movimiento confirmado junto con
`MOVE_POSTCHECK`; deben inspeccionarse ambas rutas antes de reintentar.

El campo `index.status` puede ser:

| Estado | Significado |
| --- | --- |
| `not_present` | No existía un índice; no se crea uno. |
| `updated` | El indexador aplicó su alcance nativo seleccionado. |
| `needs_sync` | El archivo se movió, pero el índice requiere atención. |

Se reutiliza el mantenimiento incremental para las rutas original y nueva,
incluida la eliminación de símbolos antiguos y la revisión de referencias
entrantes. Si se mueve JSON o `.gitignore`, se usa la reconciliación del workspace
para contemplar cambios de configuración o descubrimiento. El informe del
indexador conserva sus límites y no certifica frescura global.

El movimiento del archivo y las transacciones del índice no forman una única
transacción. Si la sincronización falla o queda incompleta, se devuelve
`MOVE_INDEX` y `needs_sync` sin deshacer el movimiento ni ocultarlo. No se debe
reintentar el movimiento; hay que actualizar el índice antes de confiar en el grafo.

Con una sesión, se invalidan las verificaciones observadas, se retiran las
observaciones de edición de ambas rutas y se añade el evento `file_moved`.
