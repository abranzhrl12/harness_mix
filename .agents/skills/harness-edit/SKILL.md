---
name: harness-edit
description: Aplicar cambios de código con Harness MCP, incluyendo reemplazos, archivos nuevos, movimientos y verificación. No se activa para revisiones de solo lectura.
---

# Edición precisa y verificación

Usa sesión y raíz verificadas mediante bootstrap. Inspecciona el estado Git y preserva cambios ajenos. Lee solo el contexto necesario con la skill de lectura del proyecto; no necesitas cargarla si ya dispones de esa evidencia.

## Aplicar

- Usa las herramientas de edición anunciadas y autorizadas. Si instrucciones de mayor prioridad exigen `apply_patch`, úsalo después de obtener contexto vigente con Harness; no eludas un rechazo de hash o permisos.
- Para `harness_replace`, usa fuente original exacta y hash vigente. No uses firmas, resúmenes ni números de línea como `oldText`. Agrupa cambios independientes del mismo original en `chunks` únicos y no solapados.
- Ante hash obsoleto, recupera el contexto afectado y replantea: nunca fuerces la versión anterior sobre cambios concurrentes.
- `harness_write` solo si está anunciado; comprueba la semántica de creación/sobrescritura y `expectedHash` antes de reemplazar.
- `harness_move` no actualiza imports ni sobrescribe destinos. Distingue movimiento aplicado de fallo posterior de indexación; no repitas una operación ya realizada.

## Comprobar

Usa `harness_git` con `action=diff` y `path` para revisar el alcance. Ejecuta `harness_verify` con el build/test adecuado a los cambios; consulta su esquema antes de pedir filtros o perfiles. Usa jobs solo si está anunciado y operativo.

Si falta una capacidad concreta, explica el límite antes de usar una alternativa autorizada. Un fallo de transporte/runtime no autoriza bypass. No amplíes permisos ni reconstruyas el índice para sortear errores.

La verificación corresponde al código posterior a la última edición: sintaxis válida no significa compilación ni corrección. Reporta qué cambió, pruebas ejecutadas y pendientes. No hagas commit, push ni sustituyas binarios instalados sin petición.
