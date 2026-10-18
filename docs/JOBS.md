# Verificaciones en segundo plano

`harness_jobs` está disponible en el perfil MCP `full`, sin configuración adicional.
Usa los mismos lenguajes, scopes y comandos de verificación cerrados que
`harness_verify`; no acepta comandos de shell arbitrarios.

Para `.ts` y `.tsx`, `action: build` comprueba únicamente sintaxis con el parser
nativo: no necesita Node, no ejecuta el archivo y no comprueba tipos.
Los tests TypeScript/TSX se rechazan explícitamente como no soportados.

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

`harness_process_status` comparte este registro: `runId` acepta el mismo
`taskId`; `list: active` y `list: recent` devuelven trabajos retenidos, también
después de reiniciar. Para salud de servicios admite HTTP loopback; una URL
HTTPS se rechaza explícitamente, no se convierte en HTTP sin TLS.

`harness_doctor` compara el registro MCP real con el perfil y consulta el
historial de trabajos existente sin crearlo. `connection: true` añade una
consulta de solo lectura al índice y un ping SQL cuando están configurados.
`allToolsVerified: false` y la cobertura indican que no certifica todas las
operaciones, clientes o plataformas; una advertencia de cobertura es normal.

Estados: `queued`, `running`, `retry_wait`, `succeeded`, `failed`, `cancelled` y
`timed_out` e `interrupted`. Cancelar solicita detener el árbol de procesos: consulta el estado
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

## Persistencia y reinicios

Los estados, resultados y eventos se guardan automáticamente en SQLite dentro
de `.agents/harness/jobs/`, excluido de Git. No requiere configurar un servidor
de base de datos. El almacenamiento se crea al lanzar el primer trabajo; abrir
un proyecto sin trabajos previos no crea archivos de historial.

Un trabajo se acepta después de guardar su estado, y los eventos se publican
después de confirmar su escritura. Se mantienen los mismos límites de historial
y los cursores continúan después de reiniciar. Solo una instancia del servidor
puede escribir el historial de trabajos de un proyecto a la vez.

Tras un cierre inesperado, los trabajos que estaban en cola, ejecutándose o
esperando un reintento se recuperan como `interrupted`. No se vuelven a ejecutar
automáticamente: podrían haber producido efectos antes del cierre. Puedes
consultar su configuración y lanzar un trabajo nuevo de forma explícita.

Si el almacenamiento no está disponible o falla al escribir, harness informa
del error de persistencia, impide nuevos trabajos y solicita cancelar los activos.
Un fallo durante la ejecución también genera un aviso `persistence_error` con
`durable:false`, sin cursor: ese aviso de control no pudo guardarse en disco.
Las demás herramientas MCP siguen disponibles. La persistencia conserva el
seguimiento; todavía no convierte harness en un servicio que siga ejecutándose
después de cerrar el servidor MCP.

La salida almacenada puede contener contenido del proyecto. Las verificaciones
pueden escribir artefactos y ejecutar código del proyecto. `verify.session` no
está admitido: estos resultados no actualizan la vigencia de una sesión histórica.
