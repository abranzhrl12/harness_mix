# Paso 03 — Resolución: prioridad y siguiente experimento

## Evidencia de los pasos anteriores

Con cuatro trabajadores, resolver referencias consumió una mediana de 5,365 s
en pasadas de 11,941 s. Hubo 41 247 intentos y 309 referencias resueltas. La
selección de pendientes ocupó 0,969 s en esta muestra; ya no domina como en el
informe SWSMA anterior, que usó otro corpus y perfil.

La relación entre intentos y resoluciones no prueba por sí sola trabajo
redundante: existen referencias externas o no soportadas. Tampoco justifica
omitir las revisiones necesarias para detectar ambigüedad.

## Qué medir antes de modificar

1. Separar dentro de resolve_exact el coste de recuperar la referencia por ID,
   consultar candidatos genéricos y resolver imports/frameworks.
2. Contar consultas ejecutadas y pasos de VM SQLite, incluyendo hits sin candidato
   y resultados ambiguos; no medir solo referencias resueltas.
3. Contar repetición de entradas equivalentes (nombre, origen, lenguaje y scope)
   dentro de la misma publicación, y vidas útiles de snapshots/cachés.
4. Medir reintentos por archivo y tamaño del conjunto pendiente a lo largo de la
   publicación, para detectar crecimiento de trabajo que un promedio escondería.

La implementación actual hace un SELECT por ID al comienzo de resolve_exact
(src/adapters/index_store/resolution.rs). Es un candidato para instrumentar, no
evidencia de que agruparlo o cachearlo vaya a ser más rápido o correcto.

RetryMemo retiene nombres públicos de JS/TS después de considerar pendientes;
esta muestra tiene muchas referencias Rust. Una prueba solo de memo on/off no
aislaría necesariamente el coste dominante. Primero se necesita el desglose.

## Criterio para el siguiente cambio

Un solo cambio por experimento. Comparación A/B con el mismo corpus, perfil y
opciones; al menos tres pasadas por variante y orden alternado. Exigir mismo
grafo, fuentes y fallos. Los intentos de resolución pueden variar si esa es la
optimización, pero deben mostrarse por variante y conservar resultados semánticos.

Si aparece una caché, debe estar limitada y vinculada al snapshot/transacción
correctos. Cambios de declaraciones, configuración, commits externos y rollback
deben invalidarla. No introducir cachés globales basadas solo en metadata.

## Estado

El [paso 04](04-selector-puro.md) ya implementó una primera separación pura y
el desglose resolver_*, con pruebas y seis pasadas A/B equivalentes. La mediana
de las reglas puras fue 0,652 ms, frente a 6,366 s de resolver_special.
El siguiente foco es desglosar imports/pruebas de lenguaje; no acelerar el
selector. Todavía no se instrumentaron pasos VM, consultas individuales ni
hits/misses de snapshots. El A/B no mostró una mejora de tiempo total.
