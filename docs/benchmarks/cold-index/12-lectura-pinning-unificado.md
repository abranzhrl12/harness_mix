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

## Activación y medición prevista

**Desactivada por defecto**, pendiente de verificar la versión final y medirla.
`HARNESS_INDEX_PINNED_READ=1` selecciona la candidata;
`HARNESS_INDEX_PINNED_READ=0` conserva el camino anterior.

El runner admite `pinned-read` para comparar el mismo binario, selector apagado
y encendido, con corpus congelado completo y Rust SQL habilitado en ambos.
Las demás comparaciones mantienen este experimento apagado. No se modifica el
índice original ni se cambia el esquema o la durabilidad de SQLite.

## Verificación e incidencia actual

La primera versión pasó 14 pruebas de Project (6 previas y 8 nuevas), incluidas
restricciones, bytes/límites, cambios con tamaño/mtime conservados, scopes,
cancelación, junction introducida tras construir el nombre, errores de pinning
y retención/liberación de handles. Una prueba inicial usaba una ruta no
canonicalizada al llamar directamente al pinning; se corrigió el fixture y se
repitieron las 14 correctamente.

Después se añadió la verificación del handle abierto y una novena prueba
específica para rechazar un handle de fuera del proyecto antes de leer bytes.
**Esa versión final aún no tiene sus 15 pruebas ejecutadas ni benchmark A/B.**
El release construido antes de esa ampliación no representa el código final y
no debe usarse para atribuir resultados a esta candidata.

La verificación por Harness quedó bloqueada por cambios paralelos de Vue. Al
principio había dos errores E0433 por un módulo `vue_router` inexistente. Tras
detectarse su incorporación se comprobó otra vez: ahora queda E0609 en
`src/adapters/index_extract/vue_router/bindings.rs:130` (`arrow.body.statements`,
campo inexistente en ArrowFunctionBody), además de un import sin uso en
`vue_router.rs`. No se modificó ni revirtió ese trabajo. La combinación de flags
Windows de la nueva API ya usa el tipo explícito requerido por windows 0.62.2;
el último diagnóstico no contiene errores de los archivos de esta candidata.

Siguiente paso: cuando Vue compile, ejecutar las 15 pruebas de Project y las
regresiones de frescura/rollback, recompilar el probe, congelar el artefacto y
medir tres parejas. No hay una mejora de rendimiento nueva que declarar aún.
