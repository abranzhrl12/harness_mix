# Paso 09: consultar el índice antes de abrir dependencias

## Hipótesis

`VerifiedSources::observe` leía una dependencia desde el sistema de archivos
antes de buscar su hash en `files`. Un candidato no indexado nunca podía ser
aceptado, aunque existiera y se leyera correctamente. Los candidatos Rust
`mod.rs`, `lib.rs` y `main.rs` repetían parte de esas consultas.

El cambio propuesto consulta primero la misma fila indexada. Si no existe,
devuelve que no hay prueba sin consultar el disco. No se añade caché de ausencia
del sistema de archivos ni una dependencia nueva. Para filas existentes se
conservan lectura protegida, límites, hash y revalidación antes del commit.

## Alcance y semántica

La lista de archivos indexados pertenece a la misma transacción SQLite
`IMMEDIATE` de publicación: `Sources` se crea durante `resolve`, después de las
escrituras de la publicación, y no publica nuevas filas durante la resolución.
Esto no convierte el sistema de archivos en un snapshot; los bytes de las
dependencias aceptadas se siguen releyendo antes del commit.

La ordenación de errores cambia deliberadamente: candidatos no indexados ya no
se abren y por tanto no originan errores de ruta/lectura de dependencias; un error
al consultar la BD se propaga antes de consultar el disco. Los archivos indexados
mantienen sus rechazos de seguridad y lectura. No afecta a la lectura inicial del
archivo que el usuario solicita indexar. Cancelación y timeout deben comprobarse
antes del rechazo rápido.

## Protocolo

Implementación y revisión delegadas a GPT-6 Luna. Se preservan los cambios de OXC
del usuario. El baseline se compila después de esa migración y antes de modificar
`observe`: no se usan binarios anteriores para atribuirle ganancias a este cambio.

La comparación usa el corpus congelado existente, tres pasadas alternadas por
variante, profiler activo e índice temporal vacío por pasada. La opción `all` del
runner verifica que `sampleFiles == copiedSourceFiles` y que el corpus no alcanza
el límite de 2.000 del probe. No se vacían las cachés del sistema operativo ni se
modifica el índice del proyecto original.

Baseline release capturado tras compilar correctamente:
`target/cold-index-before-index-gate.exe`, SHA-256
`4ab48118e748c80bc2137a86fdc62bc3e4ed1a5c6a893aee1ed7a8367cc666b0`.

## Verificación

- `cargo check` mediante Harness: correcto antes y después del cambio.
- 7 pruebas de VerifiedSources correctas: candidatas no indexadas sin consultas
  al disco (comprobado con el profiler), archivo indexado ausente que aparece
  después, digest vacío distinto de ausencia de fila, error SQL, cancelación,
  timeout, rechazo de rutas indexadas y revalidación de bytes/mtime.
- Otras 12 pruebas correctas: 2 del ciclo Rust/rollback, 5 de imports y 5 de la
  caché de análisis Rust (incluye recheck tras un acierto).
- No se ejecutó la suite completa del workspace.

La consulta de hash utiliza una sentencia preparada cacheada de SQLite. No se
añade un mapa de negativos ni un escaneo adicional del árbol de archivos.

## Primera pareja completa (provisional)

Ambos binarios release compilaron. La primera pareja sobre todos los archivos
compatibles copiados dio 102,052 s antes y 74,407 s después. No es aún una
mediana ni una garantía. El corpus contiene 549 archivos copiados; el coordinador
procesó los 537 candidatos elegibles, sin fallos ni candidatos nativos restantes.

El volumen no es comparable con la muestra antigua de 200 archivos: esta pasada
intenta resolver 283.976 referencias frente a las 40.928 de aquella muestra.
La cifra de 102 segundos corresponde al baseline anterior a esta optimización,
no a un empeoramiento introducido por ella.

| Contador por pasada | Antes | Después |
| --- | ---: | ---: |
| Intentos de lectura de dependencias | 69.940 | 19.818 |
| Validaciones iniciales de ruta, todas las fases | 90.832 | 40.710 |
| Consultas de membresía/hash al índice | 30.012 | 69.940 |
| Dependencias revalidadas | 19.818 | 19.818 |
| Bytes revalidados | 182.843.207 | 182.843.207 |

Son contadores de las operaciones instrumentadas, no un conteo de todas las
llamadas Win32 individuales. El promedio de las lecturas restantes no es
comparable directamente: desaparecen los candidatos ausentes y solo quedan
lecturas de archivos existentes. No se afirma que cada lectura sea más rápida.

El grafo coincide: `202c2e102ba97ae2470d172fece3dba4e79c8f9c8e5e1af1c531a58f71488bbb`.
Ambas variantes conservan 1.227 referencias resueltas y los mismos análisis Rust
y aciertos de caché. La segunda y tercera parejas siguen en ejecución en
`target/cold-index-index-gate-all/`.
