# Optimización de pendientes — 2026-10-22

## Resultado observado

En dos primeras pasadas sobre índices temporales vacíos, una muestra de 200
archivos produjo las mismas 194 publicaciones, 1 fallo semántico y 5 archivos
no comprobados por codificación. Se conservaron exactamente la huella de fuentes,
la huella canónica del grafo, los 78 355 intentos de resolución, las 3084
referencias resueltas y los 2523 nodos escritos.

| Ejecución | Consulta antigua | Consulta indexada |
| --- | ---: | ---: |
| Primera pasada en frío, ensayo 1 | 196,963 s | 43,769 s |
| Primera pasada en frío, ensayo 2 | 261,966 s | 17,855 s |
| Tiempo total / 194 publicaciones | 1,02–1,35 s | 0,09–0,23 s |

El último cociente incluye planificación, preparación y trabajo fallido: no es
una distribución de latencias por archivo ni un p95. Solo hay dos ensayos por
variante, con variación apreciable. No se extrapola al proyecto de 1292 archivos
ni se promete una aceleración universal de 4–15 veces.

## Cuello de botella y cambios

La selección antigua comienza por el índice de estado de las referencias y
evalúa múltiples ramas OR y subconsultas para cada pendiente. En el primer ensayo,
seleccionar pendientes consumió 106,856 s y recorrerlos otra vez para providers,
13,918 s: juntos, aproximadamente el 61 % de los 196,963 s totales.

La consulta nueva:

- materializa nombres del archivo, nombres públicos y nombres aún no cubiertos;
- separa las búsquedas selectivas por nombre/ruta con UNION para aprovechar
  los índices existentes, evitando que el índice de estado desplace esos seeks;
- conserva las reglas de imports, memo, visibilidad, referencias locales,
  homónimos, Angular, Vue, Rust y HTML;
- incluye providers en el recorrido conservador compartido, sin añadir un filtro
  de lenguaje que la consulta antigua no tenía;
- deduplica IDs y conserva 20 001 como señal de superar el presupuesto de 20 000.

No se añadieron índices, migraciones ni dependencias. No se cambiaron el writer
único, synchronous=FULL, los límites, la revalidación ni commit/rollback por archivo.
El caso de uso y sus puertos permanecen desacoplados del SQL.

## Fases medidas

Tiempos acumulados de publicaciones correctas, salvo preparación paralela
(tiempo de pared que incluye lecturas y extracción, no solo parser).

| Fase | Antigua, rango de dos ensayos | Indexada, rango de dos ensayos |
| --- | ---: | ---: |
| Seleccionar pendientes + providers | 120,774–189,734 s | 8,373–19,189 s |
| Resolver referencias | 17,810–19,648 s | 1,492–4,123 s |
| Escribir nodos y aristas | 16,001–17,257 s | 1,653–3,452 s |
| Preparación paralela | 9,381–13,358 s | 1,599–6,292 s |
| Commit | 5,797–7,314 s | 1,735–4,820 s |
| Invalidar aristas de motores | 4,992–7,308 s | 0,854–1,804 s |
| Construir catálogo de bindings | 3,012–3,506 s | 0,388–0,850 s |
| Leer fuente | 0,854–1,018 s | 0,175–0,320 s |
| Revalidar fuentes | 1,616–1,764 s | 0,309–0,588 s |

Las implementaciones de las otras fases no cambiaron: sus tiempos también
variaron. No se atribuye cada diferencia al selector; cachés y ruido del entorno
no están aislados. La prueba de pasos VM ofrece evidencia independiente del
tiempo de pared: en una fixture con 20 000 pendientes irrelevantes y dos hits,
SQLite pasó de 2 500 478 a 960 835 pasos, un 61,6 % menos.
Esa fixture no contiene providers; su baseline cuenta solo la consulta pendiente
antigua, no su consulta adicional de providers.

Los contadores de todas las fases y las cuatro ejecuciones están en
[el resumen JSON](index-pending-2026-10-22.json). Los informes detallados quedan
en target/index-cold-{legacy,indexed}-200-{1,2}.json, excluidos de Git.

## Condiciones y límites

- Windows, SQLite 3.53.2, perfil dev: crate principal opt-level=1,
  dependencias opt-level=3 y debug assertions activadas. No es un benchmark release.
- HARNESS_INDEX_WORKERS=4, resolución memo y catálogo de bindings habilitados.
- SWSMA: 2150 fuentes compatibles copiadas (53 564 385 bytes); primeros 200
  caminos ordenados como muestra. Predominan scripts JavaScript.
- Cada ejecución usa un índice SQLite vacío nuevo en su propio TempDir.
  No se modifica el proyecto ni su índice original.
- Frío significa índice vacío, no cachés del sistema operativo vacías. La copia,
  inicialización, huellas y compilación quedan fuera del tiempo de indexación.
- No hubo compilaciones ni suites de pruebas concurrentes con las pasadas medidas.
- El primer ensayo antiguo usó la compilación inmediatamente anterior de la misma
  lógica antigua y perfil; los otros tres usaron el mismo ejecutable final.
- La segunda pasada forzada del primer ensayo antiguo agotó el deadline compartido
  de 600 s y quedó parcial. Se excluye por completo de esta comparación; el helper
  ahora permite --cold-only.
- Se preservan SYNC_SEMANTIC en Script_FirebaseInit.js y los cinco casos no UTF-8.
  No se ocultan fallos para mejorar el tiempo ni se certifica frescura global.

## Pruebas

Selección ejecutada: 292 aprobadas, 0 fallidas y 6 ignoradas
(5 oráculos externos y un probe manual); no es toda la suite del workspace.

- 128 unitarias de indexación, incluidas tres nuevas pruebas diferenciales/VM.
- 49 de aplicación: etapas, cancelación, límites y decisiones transaccionales.
- 22 de arquitectura.
- 93 integraciones: rollback real, imports/reexports, cambios de configuración,
  ambigüedad, providers, Angular y Vue.

La matriz compara la selección nueva contra la consulta antigua MÁS su selección
separada de providers. Incluye lenguaje de origen, cambios de configuración,
memo, catálogos nulos/vacíos/duplicados, estados y providers en referencias de
otros lenguajes. El test de pasos VM exige más del 50 % de reducción en su fixture;
no impone tiempos de pared inestables en CI.

## Reproducción

```powershell
cargo build --locked --example index_phase_probe
$env:HARNESS_INDEX_PROFILE = "1"
$env:HARNESS_INDEX_WORKERS = "4"
$env:HARNESS_INDEX_RESOLUTION_MEMO = "1"
$env:HARNESS_INDEX_RETRY_BINDINGS = "1"
$env:HARNESS_INDEX_PENDING_QUERY = "legacy"
./target/debug/examples/index_phase_probe.exe "D:/backoffice/SWSMA PRODUCCION/SWSMA" 200 --batch --fresh --cold-only
$env:HARNESS_INDEX_PENDING_QUERY = "indexed"
./target/debug/examples/index_phase_probe.exe "D:/backoffice/SWSMA PRODUCCION/SWSMA" 200 --batch --fresh --cold-only
```

Sin el selector legacy se usa por defecto la consulta nueva. El modo legacy
conserva también el recorrido separado de providers para diagnóstico A/B.

Falta medir el release y la escala completa de 1292 archivos. Siguen existiendo
recorridos conservadores de pendientes/frameworks y de aristas de motores; este
cambio no demuestra haber eliminado todo crecimiento cuadrático.
No se compiló ni instaló un release en npm durante esta tarea.
