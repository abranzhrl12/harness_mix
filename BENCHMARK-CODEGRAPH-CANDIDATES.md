# Bloque: búsquedas repetidas de candidatos

2026-10-17. Cambio: `index_candidates.rs`, usado solamente durante el bucle
de resolución de un archivo, después de escribir todos sus nodos. Se descarta
antes del archivo siguiente. Guarda filas sin filtrar: visibilidad, scope,
ambigüedad y resolución de exports siguen evaluándose por referencia.

Clave: nombre exacto, lenguaje y ruta importada. Límites: 128 entradas y 2 MiB
de asignaciones contabilizadas con margen por entrada; al saturarse se consulta
SQLite normalmente. Se conservan orden de filas y el sentinel de 65 candidatos.

## Medición aislada, debug, SQLite en memoria

4096 búsquedas distribuidas entre cuatro nombres repetidos, incluidos ausentes
y diferencias de mayúsculas. Ambos caminos usan la misma consulta preparada.
Tres rondas; no hay umbral de tiempo que vuelva frágil el test.

| Ronda | Sin caché | Con caché |
| --- | ---: | ---: |
| 1 | 155,5104 ms | 7,1188 ms |
| 2 | 110,0105 ms | 5,9732 ms |
| 3 | 101,1004 ms | 5,4862 ms |

Consultas SQL comprobadas: 4096 sin caché, 4 con caché. Estos números describen
un caso sintético con repetición alta, no la distribución del repositorio ni
una aceleración de indexación completa. No se extrapolan a nombres todos únicos.
El benchmark release anterior de 3,74 s no incluye este nuevo bloque.

## Validación acotada

Solo cinco tests del nuevo componente: claves, filas independientes del filtrado
del llamador, reinicio entre snapshots, orden/ambigüedad, presupuestos y diagnóstico
repetido. Pasaron en 0,525 s de ejecución.

Solo cuatro tests de integración: símbolos privados entre archivos,
receivers/ambigüedad, alias frente a homónimos y llamadas con cambios de línea y
renombrados. Pasaron en 2,677 s de ejecución. Formato verificado.
No se ejecutó la suite completa ni se recompiló release para este bloque.
