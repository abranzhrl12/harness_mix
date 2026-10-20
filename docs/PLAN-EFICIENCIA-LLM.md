# Plan de eficiencia de interacción LLM–Harness

Fecha: 2026-10-18. Estado: análisis y propuesta; no es una lista de funciones ya implementadas.

Actualización de implementación: se añadieron carpetas opt-in, reintentos seguros,
texto sin parser explícito, resumen de escritura y bootstrap compacto opt-in.
Ver [contrato de I/O](IO-EFFICIENCY.md). Este plan conserva el análisis original;
los lotes con recuperación y los diffs integrados siguen pendientes.

## 1. Conclusión ejecutiva

El objetivo es resolver una tarea con menos llamadas dependientes, menos contexto
irrelevante y menos reintentos, conservando evidencia suficiente y seguridad.
No se propone reescribir el motor Rust ni ocultar errores para aparentar rapidez.

La base ya incluye varias optimizaciones. El problema no es que falte todo:

- Bootstrap reutilizado compacto, catálogo recuperable y hashes de capacidades.
- Lectura por símbolo con cuerpo completo y recuperación de detalles.
- Reemplazo sin hash obligatorio y varios chunks sobre el mismo original.
- Escritura que ya valida sintaxis, publica sin sobrescribir y confirma hash.
- Verificación por paquete y herramientas de diagnóstico independientes.

Los siguientes cambios de mayor valor son completar el resultado de escritura,
resolver carpetas/reintentos dentro de Harness y hacer explícito el contrato de
información mínima. Crear varios archivos requiere una fase adicional de diseño
de recuperación; no debe bloquear innecesariamente las mejoras simples.

## 2. Alcance y evidencia de esta revisión

Se revisaron implementaciones mediante Harness MCP, no solo firmas o comentarios.
Raíz comprobada: `E:\harness_mix`. HEAD observado:
`62d3878d661050d3e8130c7f87adb6f7c992b60b`, con cambios locales previos.
El ejecutable usado para investigar reportó `0.8.5`.

El estado de una función debe distinguirse en tres niveles:

1. Existe en el código fuente del worktree.
2. Está incluida y probada en un artefacto concreto.
3. Está disponible en el proceso MCP realmente usado por el cliente.

Una versión semántica igual no prueba que dos artefactos contengan los mismos
cambios locales. Esta revisión no recompiló ni publicó un release y no cambió
la instalación global ni la configuración de Antigravity.

### Fuentes principales

| Evidencia | Fuente y símbolos revisados |
| --- | --- |
| Inicio y reutilización | [Application::bootstrap](../crates/application/src/lib.rs), [documento de eficiencia existente](LLM-EFFICIENCY.md) |
| Creación, edición y preview | [edit.rs](../src/adapters/edit.rs): `write`, `replace`, `syntax`, `preview` |
| Publicación y protección de ancestros | [edit_commit.rs](../src/adapters/edit_commit.rs): `pin_directory`, `publish_with` |
| Lecturas y contratos | [project.rs](../src/adapters/project.rs): `read_with_context`; [contracts.rs](../crates/domain/src/contracts.rs) |
| Presupuesto de entrada | [input.rs](../src/transport/input.rs): `MAX_MESSAGE_BYTES` |
| Límites del parser Rust | [rust_syntax.rs](../src/adapters/rust_syntax.rs): `with_ast` |
| Distribución | [package-npm.ps1](../scripts/package-npm.ps1), [verify-package.cjs](../npm-dist/scripts/verify-package.cjs), [package.json](../npm-dist/package.json) |

### Qué dicen y qué no dicen los tiempos

Los tiempos de lectura/reemplazo/búsqueda aportados en la conversación son
evidencia de otra sesión, no un benchmark reproducido aquí. No permiten atribuir
todo el tiempo total al LLM, a la red o al compilador sin instrumentar esas partes.
Una llamada rápida puede pertenecer a una tarea lenta si obliga a muchas llamadas
secuenciales, genera errores de parámetros o devuelve demasiado contexto.

El documento histórico `LLM-EFFICIENCY.md` contiene medidas propias y limitaciones.
Sus cifras corresponden a esos fixtures/versiones, no a una aceleración universal.

## 3. Inventario real: existente frente a pendiente

| Área | Estado observado | Trabajo realmente pendiente |
| --- | --- | --- |
| Bootstrap | Compacta una sesión reutilizada, no restaurada y con reglas/capacidades iguales | Reducir inicio inicial y evitar recuperar catálogo idéntico por rutina |
| Lectura | Fuente local permite 400 líneas y conserva 16000 bytes de fragmento | Distribuir ese cambio; evaluar recuperación que no fuerce tantear rangos |
| Símbolos | `harness_symbols` permite outline y read/full | Usar el cuerpo concreto directamente para implementar, no encadenar firmas y lecturas arbitrarias |
| Reemplazo | Hash opcional, chunks y comprobación del original antes de publicar | Mejorar resumen de cambio y diagnósticos; no reinventar multi_replace |
| Creación | Un archivo, Windows, padre existente, publicación no-clobber | Carpetas, reintentos seguros, formatos de texto y después lotes |
| Resultado de escritura | Ya incluye hash, verified, changed/created, commitStatus y syntax | Unificar estados, explicar nivel de validación, localizar errores y añadir resumen acotado |
| Diff | Preview separado con diff limitado y detalle recuperable | Reutilizar su lógica en la respuesta de edición cuando aporte evidencia |
| Alias I/O | Fuente local admite target_path/sessionId en read/write/replace | Verificar su inclusión en el release; no asumir alias en todas las herramientas |
| Publicación | Hook local prepara release y valida versión/esquemas MCP | Probar comportamientos nuevos y la ruta activa del cliente, no solo presencia de campos |

No hay que añadir una compilación a cada escritura: `write` y `replace` actuales
no llaman a Cargo. Validación sintáctica, comprobación del contenido escrito y
compilación son niveles distintos.

## 4. P0: entregar la versión correcta antes de comparar rendimiento

La conversación documentó un proceso de Antigravity usando un `.exe` standalone
0.7.0 mientras existían lanzadores npm y artefactos 0.8.5. Esto es un incidente de
resolución/distribución, no una medición del rendimiento del código actual.
No se supone que aquel proceso siga activo después de esta revisión.

Antes de publicar o medir:

- Comprobar ruta del proceso activo, versión de bootstrap y contrato anunciado.
- Construir desde un estado identificado y conservar hash del artefacto/commit.
- Validar el ejecutable que irá dentro del paquete, no otro encontrado en PATH.
- Probar read de 400 líneas, aliases, chunks, creación y package de forma funcional.
- Reiniciar la conexión del cliente después de actualizar y repetir el smoke test.

El verificador local actual comprueba versión exacta y presencia de write,
chunks/hash opcional y package. Todavía no prueba todos esos comportamientos ni
el límite de 400 líneas. El hook usa el repositorio y toolchain para preparar el
paquete; `--ignore-scripts` puede saltárselo. Una instalación global antigua o
un proceso no reiniciado no se corrigen solo publicando una versión nueva.

## 5. P1: información mínima, pero suficiente

### 5.1 Bootstrap inicial y reutilización

Se hicieron dos llamadas reales en esta revisión: una sesión nueva y su
reutilización inmediata con reglas/capacidades sin cambios.

| Resultado | Longitud de JSON.stringify(envelope) |
| --- | ---: |
| Inicio nuevo | 10230 unidades UTF-16 |
| Reutilización | 1643 unidades UTF-16 |

Son tamaños del envelope canónico observado, no bytes del protocolo, tokens,
facturación ni tiempos. El segundo resultado incluyó `catalogUnchanged=true`,
`rulesUnchanged=true` y referencia de detalle. No es una prueba estadística.

Propuesta: introducir una vista compacta explícita y compatible del inicio.
Mantener visibles raíz, sesión, versión, hashes, cambios de capacidades y avisos
obligatorios de seguridad/frescura. Catálogo extenso, ejemplos e instrucciones
generales deben ser recuperables bajo demanda. Las reglas del proyecto no se
pueden ocultar antes de que el agente disponga de ellas.

No quitar ni duplicar los schemas de tools/list sin medir el transporte del
cliente. Algunas interfaces muestran envoltorios/serializaciones además del
contenido: hay que medir el payload real antes de atribuir esa duplicación a Rust.
Una sesión restaurada o un hash distinto deben invalidar la reutilización.

Criterios: prueba de sesión nueva, reutilizada, restaurada, reglas cambiadas,
perfil cambiado y detalle caducado; ninguna política necesaria queda inaccesible.

### 5.2 Resultado suficiente de escritura en la misma llamada

Completar la respuesta actual en vez de inventar otra herramienta de validación
obligatoria. Contrato ilustrativo, todavía no implementado:

```json
{
  "path": "src/entities/profile.rs",
  "status": "created",
  "changed": true,
  "commitStatus": "committed",
  "hash": "<sha256-calculado-por-harness>",
  "verified": true,
  "syntax": {"status": "valid", "validation": "syntax_only"},
  "diagnostics": [],
  "changeSummary": {"addedLines": 24, "removedLines": 0},
  "typecheck": "not_run"
}
```

La IA aporta ruta y contenido. Harness calcula hash, resumen y diagnósticos;
el cliente serializa la llamada. No exigir al modelo reconstruir esos campos.

- Archivo nuevo: resumen por defecto, no eco completo del contenido enviado.
- Reemplazo: resumen y diff pequeño cuando sea útil; detalle bajo demanda si excede presupuesto.
- Reutilizar el original/candidato ya disponibles y el diff de preview; no lanzar Git ni releer desde el cliente por rutina.
- Mantener el postcheck interno y las protecciones: reducir llamadas del LLM no justifica eliminar comprobaciones de seguridad.
- Error de sintaxis: ruta, código y posición cuando el parser la proporcione;
  indicar unidades de columna y preservar posiciones con BOM/Unicode/CRLF.
- Error previo a publicación: `changed=false`. Error posterior: informar que
  hubo commit, aunque falle el postcheck; no presentarlo como escritura inexistente.
- No convertir sintaxis válida en afirmación de tipos, imports o compilación válida.

Presupuestos iniciales propuestos: resumen JSON sin eco de contenido; diff por
defecto ausente para creación y acotado a 4000 caracteres para reemplazo, con
límites de trabajo propios. Son parámetros a medir, no capacidades actuales.

### 5.3 Lecturas sin tanteo repetido

Conservar 400 líneas y el límite de 16000 bytes. No ampliar indiscriminadamente.
Para una implementación conocida, pedir `harness_symbols` read/full directamente.
Para localizar, outline filtrado o búsqueda literal; usar batch solo con lecturas
independientes que el contrato admita.

Hoy un fragmento mayor de 16000 bytes devuelve SIZE y pide reducir el rango.
Evaluar un error con propuesta concreta de siguiente rango o una vista paginada
explícita con referencia recuperable. Nunca recortar silenciosamente ni permitir
usar como oldText un cuerpo incompleto o una representación numerada.

## 6. P1: crear carpetas y reintentar sin investigación manual

### 6.1 Carpetas necesarias en la misma operación

Propuesta compatible: opción `createParents`, inicialmente explícita. Si luego
se cambia el valor por defecto, hacerlo con contrato/versionado y pruebas.

Orden propuesto:

1. Validar contenido, formato, límites y rutas antes de crear carpetas.
2. Normalizar destino dentro del workspace y rechazar rutas protegidas.
3. Fijar el ancestro existente; crear y validar cada nivel, manteniendo sus handles.
4. Rechazar enlaces/reparse points y ancestros que cambien concurrentemente.
5. Preparar archivo temporal, sincronizar, publicar sin sobrescritura y postverificar.
6. Devolver carpetas creadas y resultado, sin una llamada externa de confirmación.

`pin_directory` actual exige ancestros existentes; agregar `create_dir_all` antes
de esa función no resuelve por sí solo la carrera con cambios de directorios.
Reutilizar la política actual de rutas, instrucciones, secretos e índice.
Acotar profundidad y cantidad de carpetas. Ante fallo, retirar solo carpetas
creadas por esta operación que sigan siendo las mismas y estén vacías; informar
cualquier residuo. Nunca borrar directorios ajenos como supuesto rollback.

### 6.2 Reintentos seguros

Si el destino existe y contiene exactamente los bytes solicitados, devolver
`already_present`, `changed=false` y el hash observado, tras aplicar los mismos
controles de ruta/tipo/estabilidad. Si difiere, conflicto y cero sobrescritura.
Igualdad es de bytes: no normalizar saltos de línea para justificar equivalencia.

Esto expresa que el contenido solicitado estaba presente al comprobarlo, no que
Harness haya creado ese archivo en una llamada anterior. Una edición posterior
de otro proceso sigue siendo posible y no debe ocultarse.

Una clave de idempotencia persistida solo se justifica al diseñar recuperación
de lotes: vincularla a workspace y hash del payload; rechazar reutilización con
otro contenido. No almacenar el código completo ni secretos por comodidad.

### 6.3 Formatos de texto sin parser

Actualmente se admiten Rust, JS/TS y variantes, JSON, Markdown y texto; otros
formatos fallan con PARSER_UNAVAILABLE. Proponer para creación un modo explícito
de texto sin validación sintáctica, manteniendo UTF-8, límites y rutas seguras.

Devolver `syntax.status=not_checked` y motivo. Un error real de un parser
disponible, un timeout o PARSER_LIMIT no se deben convertir automáticamente en
éxito mediante un fallback. No habilitar un bypass implícito para modificar
archivos existentes o rutas protegidas. La política de formatos se decide
separadamente de la publicación física del archivo.

## 7. P2: varios archivos, sin prometer atomicidad inexistente

Propuesta de entrada ilustrativa:

```json
{
  "files": [
    {"path": "src/entities/profile.rs", "content": "..."},
    {"path": "src/services/profile.rs", "content": "..."}
  ],
  "createParents": true,
  "session": "<sesion>"
}
```

Mantener la entrada actual path/content. Excluir mezclarla con files mediante
schema y deserialización. Adaptar también el verificador npm: actualmente exige
path/content en required y rechazaría un esquema nuevo que solo declare oneOf.
No convertir `harness_batch`, orientado a lecturas, en un lote de mutaciones
implícitamente transaccional.

Antes de modificar: validar todos los contenidos, destinos, colisiones sin
distinción de mayúsculas en Windows, aliases de ruta, ancestros y conflictos
archivo/directorio. Compartir comprobaciones de padres, no duplicarlas por archivo.
Mantener un resultado identificable por archivo y un estado global inequívoco.

Garantías que deben distinguirse:

- Fallo de validación previo: ningún archivo ni carpeta se publica.
- Fallo durante publicación: puede haber un subconjunto publicado.
- Caída del proceso: requiere protocolo durable de recuperación para explicar
  lo que quedó; un try/catch con rollback no alcanza.
- Publicar cada archivo atómicamente no da visibilidad atómica al conjunto.

Si se requiere realmente evitar parciales tras fallos, diseñar y probar antes
un journal con estados, propiedad/identidad de los archivos y recuperación
interrumpible. No eliminar archivos cambiados por terceros para simular rollback.
Si un lote crea exclusivamente un directorio nuevo, evaluar staging y rename de
ese directorio como caso restringido; no generalizarlo a carpetas existentes.

Sin ese diseño, no anunciar esta fase como «todo o nada». Exponer `partial`,
resultados individuales y `recovery_required` cuando corresponda. El objetivo
del usuario de ausencia de parciales no se considera cumplido con ese modo.

### Límites que no deben contradecirse

El transporte limita cada mensaje entrante a 128 KiB. La creación anuncia un
límite interno de 2 MiB y el parser Rust impone 256 KiB. El límite efectivo es
el más restrictivo, incluidos wrapper y escapes JSON. Un lote no puede anunciar
16 archivos de 2 MiB en una sola llamada del transporte actual.

Punto de partida a evaluar: hasta 16 archivos, 64 KiB de contenido agregado y
además mensaje serializado dentro de 128 KiB. Esos topes no sustituyen el control
del mensaje: caracteres escapados pueden aumentar mucho el JSON. No subir el
límite global de entrada sin una evaluación de memoria, cancelación y abuso.

## 8. Reestructuración mínima del código

No se recomienda una reescritura arquitectónica. Extraer un pipeline interno
compartido solo cuando empiece la implementación de estas funciones:

`validar solicitud → preparar candidatos → validar sintaxis → publicar → verificar → presentar resultado`

- Domain: modelos de solicitud/resultado y exclusión de modos de entrada.
- Application: sesión, admisión, bloqueo de mutación, frescura e invalidación de checks.
- Adapters: filesystem, pinning, staging, no-clobber, parsers y postcheck.
- Presentación: diagnósticos y diff/resumen con presupuesto; sin una segunda IA.
- Transport: schema fiel, límites, cancelación y serialización.

Reutilizar el bloqueo y registro de edición existentes. Un lote debe invalidar
checks y observar todos sus archivos publicados, incluso cuando termine parcial.
No conservar hashes de lecturas pasadas como sustituto de comprobar el original
actual; no eliminar controles internos de integridad para ahorrar I/O sin medir.

## 9. Medición y criterios de aceptación

Medir tareas completas con el mismo estado inicial, no solo duración del motor.
Separar escenarios deterministas de herramientas y evaluación con un LLM real.

| Escenario | Resultado esperado |
| --- | --- |
| Crear archivo con dos carpetas faltantes | Una llamada de mutación; sin shell/mkdir externo ni lectura posterior de rutina |
| Repetir solicitud de creación ya completada | already_present; no modifica bytes ni necesita diagnosticar manualmente |
| Crear tres archivos relacionados | Una llamada solo cuando el protocolo de lote esté implementado; resultados explícitos |
| Modificar imports/campos/constructor del mismo archivo | Un replace con chunks y confirmación suficiente; no releer para cada hash |
| Error de sintaxis | Diagnóstico en la misma respuesta, sin publicación y sin build automático |
| Reutilizar sesión | No reenviar catálogo/reglas idénticos; avisar cambios y permitir recuperar detalle |

Registrar por tarea: llamadas totales y secuenciales, reintentos, errores de
parámetros, bytes de entrada/salida, tamaño canónico y tiempo total. Separar
tool time, espera de cola y tiempo del cliente/modelo cuando sea observable.
No guardar contenido de archivos o credenciales para obtener estas métricas.

Preservar ambiente/fixtures y reportar número de muestras. Para microbenchmarks,
usar al menos 20 repeticiones por condición, frío/caliente por separado, mediana
y p95. En evaluación LLM, fijar modelo/prompt y reportar variabilidad y éxito;
un cambio en bytes no demuestra un ahorro equivalente en tokens o segundos.

Objetivos iniciales: eliminar llamadas mecánicas identificadas sin empeorar
correctitud, visibilidad de fallos ni seguridad. No prometer porcentajes antes
de medir el flujo completo. Comparar tanto el camino exitoso como el error.

Pruebas obligatorias para cada fase: Unicode/BOM/CRLF, límites, rutas protegidas,
links/junctions, permisos, destino existente, cambio concurrente, cancelación
antes/después del commit y respuesta perdida. Para lotes: duplicados/case,
conflictos de ancestros, fallos en cada publicación y reinicio tras caída.

## 10. Orden de entrega recomendado

1. Cerrar distribución/identificación y smoke tests funcionales de lo ya existente.
2. Entregar resultado de escritura suficiente, diagnóstico localizado y lectura 400.
3. Añadir carpetas seguras, reintentos de creación y modo de texto explícito.
4. Ampliar bootstrap compacto inicial de forma compatible y medir contexto real.
5. Implementar lotes únicamente con garantías de recuperación definidas y probadas.

Las fases 2–4 pueden entregarse independientemente; la fase 5 no debe empujar una
promesa de atomicidad falsa ni bloquear todas las mejoras de bajo riesgo.
Verificar el artefacto final y la reconexión del cliente en cada release.

## 11. Qué no se propone

- Compilar todo el workspace por crear un archivo o por una revisión documental.
- Crear un caché que certifique resultados vigentes sin invalidación demostrable.
- Devolver siempre el archivo completo o siempre un diff completo.
- Resumir u ocultar reglas necesarias, errores, omisiones o incertidumbre.
- Subir todos los límites indiscriminadamente o confiar en hashes obsoletos.
- Medir éxito solo en microsegundos de Rust o adjudicar al arnés la latencia total del LLM.

En esta tarea solo se genera este plan. No se implementan las propuestas, no se
ejecutan builds y no se afirma que estén disponibles en el binario distribuido.
