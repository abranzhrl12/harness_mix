---
name: harness-edit
description: Modificar código y verificar cambios con Harness MCP. No para investigación de solo lectura.
---

# Editar y verificar

Reutiliza una sesión con raíz comprobada por bootstrap. Revisa el estado Git y
preserva cambios ajenos. Obtén con Harness la fuente exacta que falta; no releas
contexto vigente ni cargues otra skill si ya tienes la evidencia necesaria.

## Aplicar el cambio

- `harness_replace`: usa `path`, `oldText` exacto y `newText`. `hash` es opcional;
  si se omite, usa la lectura interna del archivo. No pidas otra lectura solo
  para obtenerlo. Desambigua con `line` cuando corresponda; agrupa cambios del
  mismo original en `chunks` únicos y no solapados.
- No uses firmas, resúmenes ni números de línea como `oldText`. Ante `STALE`,
  recupera el contexto afectado y adapta el cambio; no fuerces la versión anterior.
- `harness_write`: distingue creación de reemplazo. Conserva `expectedHash`
  cuando sea la precondición de sobrescritura.
- `harness_move` no actualiza imports ni sobrescribe destinos. Si el movimiento
  ocurrió y falló el mantenimiento posterior del índice, no repitas el movimiento.
- Usa las herramientas anunciadas y autorizadas. Si instrucciones superiores
  exigen `apply_patch`, úsalo con contexto vigente obtenido mediante Harness;
  no eludas un rechazo de permisos.

## Verificar y entregar

Revisa el diff con `harness_git` y ejecuta la verificación proporcional al cambio
con `harness_verify`; usa jobs cuando necesites seguimiento en segundo plano.
Consulta el esquema antes de pedir filtros o perfiles. Sintaxis válida no demuestra
compilación ni corrección; la evidencia debe corresponder a la última edición.

Para corregir fallos web de interfaz, interacción o hidratación, usa también
`../harness-browser/SKILL.md`: reproduce con captura activa y comprueba consola,
red y comportamiento después del cambio. Si falta navegador conectado o no
puedes reproducir, declara pendiente la verificación runtime, aunque compile.

Si falta una capacidad, explica el límite antes de usar una alternativa autorizada.
Ante fallo de transporte, timeout o runtime, diagnostica con `harness_doctor`;
no lo eludas con shell, permisos más amplios ni reconstrucción del índice.

Entrega el cambio, verificaciones realizadas y pendientes. No hagas commit,
push ni sustituyas binarios instalados sin petición.
