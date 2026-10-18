# Medición posterior: conexión reutilizada y lotes

> Advertencia de equivalencia: la auditoría posterior encontró relaciones
> ausentes y falsos positivos Rust que afectan callers/callees del harness.
> La ventaja temporal de abajo no certifica paridad funcional. Véase
> REVISION-PARIDAD-CODEGRAPH.md.

## Medición actual tras aplicar la consulta indexada

2026-10-17, release. Mismo método CLI indicado abajo, ahora con 71 archivos
Rust del core y tres rondas independientes por backend.

| Operación CLI | Rust, mediana | CodeGraph 1.6.2, mediana |
| --- | ---: | ---: |
| Indexación | 3170,677 ms | 4205,607 ms |
| Sync sin cambios | 73,778 ms | 1047,933 ms |

Rust empleó un 24,6 % menos tiempo de indexación en la mediana de este ensayo.
Muestras de indexación Rust: 5495,913 / 3170,677 / 2726,294 ms.
CodeGraph: 4548,743 / 4064,128 / 4205,607 ms.
Sin cambios Rust: 68,225 / 69,631 / 69,637 / 73,051 / 73,778 / 75,969 /
76,286 / 92,893 / 145,932 ms.
Sin cambios CodeGraph: 948,944 / 962,850 / 985,166 / 1007,808 / 1047,933 /
1052,616 / 1083,103 / 1135,683 / 1744,797 ms.

Ambos: 71 archivos, 1258 nodos. Rust: 2460 aristas; CodeGraph: 4067. NO hay
paridad semántica completa ni igualdad de trabajo de resolución. La primera
muestra Rust fue más lenta que Node; no se afirma ventaja universal. No se
vació la caché ni se controló toda la carga externa. El sync CLI no representa
el flujo persistente/cacheado del antiguo harness Node. Los 70 archivos del
ensayo anterior no son una base A/B idéntica para calcular mejora entre versiones.

También pasó el ciclo real completo con este ejecutable release y PATH vacío:
init 501 ms, indexación de 71 archivos 2834 ms, borrado incremental 112 ms y
reindexación 2736 ms. Altas, cambios, borrado y exploración comprobados; índice
original no abierto. El runner de aceptación usa además su biblioteca debug
para las comprobaciones de explore/syncIndex; no es una certificación nueva del
transporte MCP release.

Compilación única release: 2m 27s, excluida de los tiempos. Solo dos tests
end-to-end, ambos pasados; no se ejecutó la suite completa.
Ejecutable medido: 27432448 bytes. SHA-256:
`b3aad6925c161afdcd7d6ebfe4c95dafe5155f45165951b9b0a5976494b4870b`.

## Historial: ensayo anterior a aplicar la consulta indexada

Fecha: 2026-10-17. Windows, ejecutable Rust release frente a CodeGraph 1.6.2.
Cada ronda usa copias temporales independientes del mismo conjunto de 70
archivos Rust del core. Ningún índice original se abre o modifica.
Se excluyen compilación, copiado y creación del esquema vacío; se incluye el
arranque del proceso. No se vacía la caché del sistema ni se controla toda la
carga externa. Tres rondas de indexación y nueve muestras sin cambios.

| Operación CLI | Rust, mediana | CodeGraph, mediana |
| --- | ---: | ---: |
| Indexación | 3736,918 ms | 3940,061 ms |
| Sync sin cambios | 82,977 ms | 1085,251 ms |

Indexación Rust: 3567,181 / 3736,918 / 5090,454 ms.
Indexación CodeGraph: 3937,141 / 3940,061 / 4086,020 ms.
Rust sin cambios: 72,868 / 74,155 / 76,596 / 77,262 / 82,977 /
87,113 / 89,610 / 89,709 / 93,963 ms.
CodeGraph sin cambios: 1037,110 / 1037,724 / 1046,084 / 1067,566 /
1085,251 / 1116,269 / 1127,889 / 1156,044 / 1173,287 ms.

Rust queda aproximadamente un 5,2 % por debajo en la mediana de indexación de
esta prueba. La muestra más lenta de Rust supera a todas las del proveedor:
no se afirma que siempre sea más rápido ni se extrapola a otros lenguajes.

## Cobertura y límites

Ambos indexan 70 archivos y 1228 nodos. Rust produce 2404 aristas y CodeGraph
3989: los perfiles de resolución NO son equivalentes. La medición no certifica
paridad semántica ni que todas las relaciones del harness estén cubiertas.
El harness Node antiguo además evita algunos sync mediante caché y usa APIs
persistentes: el dato CLI sin cambios no equivale a una aceleración global de
ese harness.

Ejecutable nativo medido: 27441664 bytes. SHA-256:
`6d487f52153e3afcbcfd36a06dfdf7c45cb54047ba29ec11ac2d479a7cfaf72d`.
Runner:
`real_core_release_vs_provider_cli`, con HARNESS_NATIVE_BINARY apuntando al
ejecutable release y HARNESS_CODEGRAPH_CLI al proveedor oficial.

## Regresión detectada y corregida

Una versión intermedia mantenía los sidecars bloqueados durante sqlite3_close.
En Windows provocaba reintentos de borrado WAL/SHM y un borrado de fuente tardó
3089 ms. Esa versión dio 7122,347 ms de mediana de indexación frente a
4587,493 ms del proveedor; no se considera una mejora aceptada.
La corrección libera los sidecars tras commit/rollback definitivo antes del
cierre, pero mantiene base/directorios protegidos y conserva todos los pins si
queda una transacción activa. Dos pruebas específicas pasaron en 0,186 s.
La suite acotada de indexación anterior a esa corrección pasó 87/87; la prueba
de limpieza comprueba que WAL y SHM se pueden eliminar al cerrar.
Después de corregir el cierre también pasó el ciclo real completo sin Node:
indexación de 70 archivos 3728 ms, borrado incremental 130 ms y reindexación
4520 ms; altas, modificaciones, borrado y exploración verificados.

Un primer intento comparativo terminó con sincronización incompleta sin
detalle suficiente y se descartó. Se mejoró el diagnóstico del test para
incluir el reporte JSON si vuelve a suceder. Las dos comparaciones posteriores
completaron sus tres rondas; no se atribuye una causa no demostrada al fallo
inicial.
