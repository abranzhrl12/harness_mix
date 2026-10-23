# Paso 02 — Preparación con uno y cuatro trabajadores

## Hipótesis

El coordinador prepara fuentes en ondas de hasta cuatro archivos y publica con
un único escritor. Cuatro trabajadores podrían reducir preparación sin acelerar
las fases SQL seriales. También pueden añadir overhead si los archivos son chicos.

## Comparación

Mismo binario release, misma muestra y opciones; cambia solo
HARNESS_INDEX_WORKERS=1 o 4. Tres ejecuciones por variante, con orden alternado,
e índices temporales nuevos. Informar valores individuales y mediana; no p95
con tres muestras. Comparar tiempos de preparación y fases seriales por separado.

## Criterio de aceptación

Huella de fuentes y grafo idénticas, mismos fallos y contadores semánticos,
databaseWriters=1. Una diferencia semántica invalida la comparación de velocidad.
No extrapolar archivos/segundo de una muestra al proyecto completo.

## Resultados

| Trabajadores | Ensayo 1 | Ensayo 2 | Ensayo 3 | Mediana |
| --- | ---: | ---: | ---: | ---: |
| 1 | 12,307 s | 11,968 s | 12,025 s | 12,025 s |
| 4 | 12,084 s | 11,757 s | 11,941 s | 11,941 s |

La mediana con cuatro es un 0,70 % menor (1,007 veces). La diferencia es menor
que la variación de las ejecuciones; no demuestra una mejora robusta.
Las seis pasadas mantuvieron las huellas y contadores del paso 01.

## Fases: medianas acumuladas de cada pasada

| Fase | Un trabajador | Cuatro trabajadores |
| --- | ---: | ---: |
| Resolver referencias | 5,426 s | 5,365 s |
| Commit | 1,950 s | 1,850 s |
| Revalidar fuentes | 1,511 s | 1,481 s |
| Seleccionar pendientes | 0,952 s | 0,969 s |
| Escribir nodos/aristas | 0,447 s | 0,467 s |
| Preparación paralela | No aplica | 0,404 s |
| Extracción dentro de publicación | 0,538 s | 0,001 s |

Con cuatro trabajadores, la extracción se desplaza principalmente a preparación
paralela. La última fila no significa que el parser sea cientos de veces más
rápido. Las fases son medianas independientes y algunas etapas pueden contener
trabajo de otras; no se suman como un nuevo tiempo total medido.

## Decisión

Conservar el comportamiento de trabajadores existente. No aumentar concurrencia
ni tocar transacciones por este resultado. La resolución domina la muestra y
merece el siguiente desglose. Commit y revalidación también son relevantes,
pero no se reducirán fsync ni comprobaciones de frescura para ganar tiempo.

No extrapolar este resultado a SWSMA/JavaScript ni a todos los 548 archivos.
