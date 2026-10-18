# Edición multi-chunk y evolución del harness

## Evaluación

Oxc y syn ya validan la sintaxis de las ediciones JS/TS y Rust. No hace falta
introducir otro parser para esta operación. Tree-sitter sí está presente en
`vendor/codegraph-native`; eliminarlo exige evaluar la cobertura del indexador,
incluido C#. El proyecto también utiliza SQLite bundled y mimalloc, por lo que
no está libre de componentes nativos.

El reemplazo determinista con coincidencia única, tolerancia LF/CRLF y SHA-256
vigente es la base adecuada. Los algoritmos de diff pueden servir para presentar
cambios; no deben convertir una coincidencia ambigua en una edición automática.

No hay benchmarks que permitan afirmar que el harness supera a Claude Code,
que Oxc siempre sea decenas de veces más rápido o que el consumo de memoria sea
casi nulo. Deben medirse operaciones completas: búsqueda, hashing, parseo,
publicación e indexación.

## Multi-chunk implementado

`harness_replace` acepta el formato anterior `oldText`/`newText`, o `chunks`:

```json
{
  "path": "src/example.ts",
  "hash": "<SHA-256 del archivo original>",
  "chunks": [
    { "oldText": "const first = 1;", "newText": "const first = 10;" },
    { "oldText": "const second = 2;", "newText": "const second = 20;" }
  ]
}
```

Todos los bloques se buscan contra el mismo original. El orden de la lista no
afecta el resultado: no se encadenan reemplazos. Cada texto original debe aparecer
exactamente una vez, contando apariciones solapadas. Los rangos de chunks distintos
no pueden solaparse; sí pueden ser adyacentes.

Se admiten entre 1 y 128 chunks, con un máximo acumulado de 64 000 bytes entre
textos originales y nuevos. El archivo original y el resultado no pueden superar
2 MiB. Se rechazan originales vacíos, chunks sin cambios efectivos, NUL,
archivos binarios, hashes obsoletos y la combinación de chunks con campos de
reemplazo individual (incluso vacíos) en el JSON de entrada. El formato individual
exige ambos campos. Un `newText` vacío permite eliminar el bloque.

La normalización LF/CRLF usa `std::borrow::Cow` para evitar copiar los textos
que no contienen CRLF. Se conserva la política existente: cada reemplazo toma
el estilo del primer salto de línea dentro del bloque coincidente; si no tiene
saltos, toma el primero del archivo. Los segmentos no editados conservan sus bytes.

Después de validar los rangos, se calcula el tamaño y se construye un único buffer
de salida recorriendo el original en orden ascendente. El adaptador valida la
sintaxis del resultado completo y utiliza una sola publicación segura, con
revalidación del hash antes de publicar y comprobación posterior. Si un chunk o
la sintaxis falla antes de publicar, el original queda intacto.

Se mantiene la búsqueda estándar. `memchr` puede evaluarse con benchmarks, pero
`memmem::find_iter` no es sustituto directo: omite coincidencias solapadas, como
la segunda aparición de `aa` en `aaa`. No hacen falta `bstr` ni `cow-utils` para
esta mejora.

## Siguiente mejora: mover archivos (pendiente)

Un futuro `harness_move` debe validar origen y destino dentro del workspace,
respetar rutas protegidas, controlar enlaces y carreras, definir qué sucede si
el destino existe y cubrir renombres de mayúsculas/minúsculas en Windows.
`std::fs::rename` es la operación base, pero no garantiza movimientos entre
volúmenes. `dunce` solo sería necesario ante problemas concretos de
interoperabilidad de rutas; no sustituye las comprobaciones de seguridad.

La actualización o invalidación del índice debe contemplar el origen, el destino
y las relaciones afectadas. Si el movimiento se completa y la actualización del
índice falla, la respuesta debe comunicar ambos hechos. Mover archivos no implica
reescribir imports automáticamente. Esta operación queda fuera de este cambio.
