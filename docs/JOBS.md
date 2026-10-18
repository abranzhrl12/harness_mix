# Verificaciones en segundo plano

`harness_jobs` está disponible en el perfil MCP `full`, sin configuración adicional.
Usa los mismos lenguajes, scopes y comandos de verificación cerrados que
`harness_verify`; no acepta comandos de shell arbitrarios.

## Lanzar y seguir

Argumentos de una llamada a `harness_jobs`:

```json
{"action":"start","verify":{"language":"rust","action":"test","scope":"."}}
```

La respuesta devuelve un `taskId` inmediatamente. El trabajo continúa mientras
el asistente atiende otras solicitudes. Hay dos ejecuciones simultáneas y hasta
32 trabajos activos entre cola y ejecución, independientes de las operaciones
MCP normales.

```json
{"action":"status","taskId":"<id>"}
{"action":"list"}
{"action":"events","taskId":"<id>","after":0}
{"action":"cancel","taskId":"<id>"}
```

Estados: `queued`, `running`, `retry_wait`, `succeeded`, `failed`, `cancelled` y
`timed_out`. Cancelar solicita detener el árbol de procesos: consulta el estado
terminal para confirmar el resultado. Un estado `succeeded` indica código de
salida cero; no garantiza que el runner haya descubierto pruebas ni certifica
vigencia de todas las fuentes.

## Avisos, plazo y reintentos

El servidor envía salida y cambios de estado por notificaciones MCP
`notifications/message`, con logger `harness.tasks` y nivel `info`.
`logging/setLevel` permite silenciarlas con un nivel superior a `info`.
El cliente debe recibir y mostrar las notificaciones; reactivar al LLM depende
del cliente y no está certificado para Antigravity.

`events` permite recuperar eventos sin repetir el trabajo: usa el `cursor`
devuelto como `after` en la siguiente consulta. Devuelve hasta 32 eventos por
página y señala `hasMore` y `gap` si se perdieron eventos antiguos. Se conservan
los últimos 256 eventos y hasta 64 trabajos; el resultado tiene un resumen de
salida limitado y el proceso conserva un presupuesto combinado de 2 MiB.
Los eventos de salida pueden contener contenido del proyecto, igual que la
salida de `harness_verify`.

No hay plazo de ejecución por defecto. Para añadirlo y permitir hasta tres
intentos totales:

```json
{"action":"start","verify":{"language":"rust","action":"build"},"timeoutSeconds":600,"maxAttempts":3}
```

El plazo opcional admite de 1 segundo a 24 horas y comienza al ejecutar el
proceso, no al entrar en cola. Los reintentos solo cubren errores temporales de
arranque anteriores a crear el proceso, con una espera breve entre intentos.
No se repiten automáticamente pruebas fallidas, errores de compilación,
cancelaciones, plazos vencidos ni fallos posteriores al arranque.

El estado se mantiene en memoria durante la conexión/instancia del servidor.
Al cerrarse, harness solicita cancelar sus trabajos; no continúa como servicio
del sistema ni restaura tareas después de reiniciar. Las verificaciones pueden
escribir artefactos y ejecutar código del proyecto. `verify.session` no está
admitido: estos resultados no actualizan la vigencia de una sesión histórica.
