# Paso 12: resolución dominada por lecturas protegidas

## Evidencia y objetivo

En la primera pasada after del paso 11, `resolve_references` rondaba 23 s;
`source_read_resolution` sumaba 17,894 s para 19.818 observaciones. No son
temporizadores que se sumen: la lectura está dentro de la resolución.

El coste dominante no es el selector puro ni volver a parsear Rust: son las
lecturas protegidas que se repiten entre publicaciones. Se propone evitar un
recorrido duplicado de ancestros dentro de cada lectura, no una caché global de
frescura ni la eliminación de la revalidación antes del commit.

## Candidata experimental Windows

- Se comparte la política léxica en un constructor privado. El camino anterior
  conserva su recorrido de metadata y canonicalización mediante un visitor.
- Para proyectos locales sin scope de archivo único, se abren los ancestros
  desde la raíz, rechazando reparse points y manteniendo los handles mientras
  dura la lectura. Se usa el pinning existente, sin retenerlo entre lecturas.
- Solo después de proteger los ancestros se consulta metadata del archivo. No
  se consulta un descendiente a través de un ancestro sin validar.
- Si el pinning falla, se utiliza el camino anterior para clasificar ausencia,
  errores y rutas inseguras. Un fallo de pinning nunca se convierte por sí solo
  en un resultado de archivo ausente.
- Scopes externos y de archivo único conservan el camino anterior y sus
  comprobaciones de identidad.
- La candidata comprueba también la ruta final del handle abierto antes de
  leer bytes; no sustituye esta comprobación por el nombre del archivo.
- Se mantienen límites, rechazo del reparse final, metadata antes/después,
  cancelación y lectura/hash/revalidación de contenido. No se promete un
  snapshot atómico del sistema de archivos.

Contratos nativos consultados:
[CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)
y [GetFinalPathNameByHandleW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfinalpathnamebyhandlew).
La protección de rename/delete no debe confundirse con impedir escrituras de
contenido; por eso se conserva la revalidación.

## Activación y método

`HARNESS_INDEX_PINNED_READ=1` selecciona explícitamente la candidata;
`HARNESS_INDEX_PINNED_READ=0` conserva el camino anterior. Tras la medición,
la candidata quedó activada por defecto en Windows: la variable ausente usa el
camino rápido. El runner `pinned-read` comparó las opciones `0` y `1` con el
mismo binario y Rust SQL habilitado en las dos.
Después del A/B, el selector cambió de opt-in a rápido por defecto en una sola
línea; las corridas ya fijaban `0`/`1` explícitamente, así que compararon las
mismas rutas de código.

La medición usó el corpus congelado completo (549 fuentes copiadas, 537
publicaciones elegibles), tres parejas alternadas en Windows, un índice
temporal vacío por ejecución y cachés del sistema operativo sin vaciar. No se
tocó el índice original. El artefacto release tuvo SHA-256
`fbea4298bcc1ba23ca45ee93291fdc6b3db097aeef179f2dcb4bdd4c391e3423`; antes y
después ejecutaron exactamente ese mismo binario. La evidencia resumida está en
[resultados-pinning-pinned-read-all.json](resultados-pinning-pinned-read-all.json).

| Medida | Camino anterior | Pinning | Diferencia |
| --- | ---: | ---: | ---: |
| Tiempo total mediano | 82,905 s | 58,548 s | −24,357 s (−29,38 %) |
| `resolve_references` | 29,006 s | 20,471 s | −29,4 % |
| `source_path_guard` | 17,381 s | 0 s observados | desaparece en la ruta rápida |
| `source_path_policy` | 0 s | 0,702 s | política léxica |
| `source_pin_directory` | 13,981 s | 13,601 s | similar |
| `source_handle_boundary` | 0 s | 3,710 s | coste nuevo de comprobación |
| `source_read_resolution` | 21,940 s | 14,934 s | −31,9 % |
| `source_read_recheck` | 21,674 s | 14,395 s | −33,6 % |

Los temporizadores de operaciones se superponen; no deben sumarse. Las seis
ejecuciones conservaron el mismo hash de fuentes
(`1e06bac31803b348635cb89fc39de63e386648a1f75f5a10fc0eb2abcec085a8`) y el
mismo grafo (`202c2e102ba97ae2470d172fece3dba4e79c8f9c8e5e1af1c531a58f71488bbb`):
6.435 nodos, 283.976 referencias examinadas, 1.227 resueltas, cero pendientes,
fallos, omisiones o rutas sin verificar. Se leyeron 372.863.670 bytes; las
19.818 revalidaciones (182.843.207 bytes) y los demás contadores de frescura
permanecieron iguales. La candidata registró 40.710 lecturas con ancestros
pineados. La mejora observada corresponde a esta máquina y este corpus; una
sola tanda de tres parejas no demuestra el mismo porcentaje en otras cargas.

## Verificación e incidencia de Vue

Para evitar mezclar cambios de Vue, las pruebas y el benchmark se ejecutaron
en `C:\harness_pinned_read_base`, basado en `1fd1d53`, superponiendo solo los
archivos Step 11/12 del commit `43bb107`. El commit de trabajo incluye también
cambios Vue y no compila en
`src/adapters/index_extract/vue_router/parse.rs:199` (E0609: el nodo
`ExportNamedDeclaration` de OXC 0.155.0 no tiene campo `declaration`). Ese
error es de Vue; no se editó ni revirtió código Vue.

En la copia aislada pasaron 30 pruebas: Project 15/15, `verified_sources` 7/7,
rollback/ciclo de Rust 2/2 y frescura de `symbol_graph` 6/6. Ambos scripts del
benchmark pasaron `node --check`. El probe release se compiló desde el código
final Step 12 antes de correr el A/B.

Con el selector rápido ya predeterminado, se repitieron las mismas cuatro
suites con `HARNESS_INDEX_PINNED_READ` explícitamente ausente: 30/30 pruebas
pasaron. No se alteraron el algoritmo de lectura, los límites, el hash ni las
revalidaciones de bytes; `HARNESS_INDEX_PINNED_READ=0` es el rollback operativo.
La instalación del MCP y su ejecutable no se reemplazaron.
