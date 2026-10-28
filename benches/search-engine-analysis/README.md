# Análisis personal del motor de búsqueda

Comparación aislada preparada y ejecutada por el agente principal. No modifica
el servidor ni los algoritmos de producción.

## Reproducir

```powershell
cargo run --release --locked --offline --manifest-path benches/search-engine-analysis/Cargo.toml
cargo fmt --manifest-path benches/search-engine-analysis/Cargo.toml -- --check
```

El manifiesto define su propio workspace y Cargo.lock. El target está dentro de
este experimento; no reemplaza el agent-harness.exe del proyecto.

## Qué corrige respecto al primer benchmark

El benchmark anterior en benches/search-matcher usa regex::Regex::is_match por
línea y excluye la construcción. El servidor usa grep-regex 0.1.14 y
grep-searcher 0.1.17 sobre el buffer de cada archivo. Aquí se usan esos mismos
crates y las opciones de producción (line_number=true, bom_sniffing=false,
line_terminator=LF; fallback case_insensitive=true).

Se comparan:
- RegexMatcherBuilder::build con la alternancia de términos del fallback.
- fixed_strings(true).build_literals con los mismos términos.
- Aho-Corasick por archivo, saltando al final de cada línea coincidente, con
  memchr para números de línea. La ruta híbrida solo usa Aho cuando consulta y
  buffer son ASCII; en caso contrario usa el matcher Unicode original.

El filtro ASCII del archivo también se mide. Una consulta ASCII por sí sola no
garantiza equivalencia Unicode: el patrón s encuentra ſ y k encuentra K con
regex Unicode. El programa comprueba estas diferencias y la equivalencia del
híbrido. Los casos cubren CRLF, UTF-8, términos anidados, varias coincidencias
en una línea, archivos vacíos, ausencia y última línea sin salto.

## Método y resultados

Se usan 64 archivos sintéticos de 256 líneas, aproximadamente 0,8–1,0 MB por
corpus, 2/8/16 términos, tasas de líneas coincidentes de 0/1/25 %, y cuatro
archivos reales del código search (36.374 bytes al medir). Nueve muestras por
caso; cuatro pasadas por muestra de escaneo; el orden de motores rota.
El programa verifica las mismas listas de números de línea antes de medir.
Construcción y escaneo se miden por separado.

Resultados de la última ejecución, en microsegundos por corpus:

| Caso | grep actual | Híbrido Aho | Observación |
| --- | ---: | ---: | --- |
| 8 términos, 0 % coincidencias | 91,05 | 258,05 | híbrido 2,83 veces más lento |
| 8 términos, 1 % coincidencias | 133,43 | 286,18 | híbrido 2,14 veces más lento |
| 8 términos, 25 % coincidencias | 531,50 | 502,43 | híbrido 1,06 veces más rápido |
| 3 términos, fuentes reales | 37,12 | 28,08 | híbrido 1,32 veces más rápido |

Construir el matcher actual costó 239,97 / 252,50 / 264,01 µs para 2/8/16
términos; clonar uno compilado costó 0,778 / 0,288 / 0,277 µs. Esto mide
construcción y clone, NO una implementación de caché con clave, bloqueo y
evicción ni la mejora total de una búsqueda. build_literals no ganó en todos
los casos, así que no se recomienda sustituir automáticamente el constructor.

El prototipo de lectura doble frente a procesar ambos matchers sobre el mismo
buffer usa copias de Vec como coste mínimo de lectura, sin filesystem real.
Sin exactos, reutilizar el buffer mejoró 1,09–1,15 veces. Cuando el exacto está
en el último archivo, calcular candidatos anticipadamente tardó 187,05 µs
frente a 50,98 µs: 3,67 veces más lento. No es una mejora universal.

La salida completa está en RESULTS.txt. Hubo otras cargas en el equipo y las
ejecuciones mostraron variación; estos números no son garantías ni un
benchmark extremo a extremo.

## Hallazgos del código y recomendación

1. search.rs compila el plan exacto, ejecuta el scan, construye el matcher de
   términos y después vuelve a llamar plan::build. Esa segunda construcción
   recompila el matcher exacto para reemplazarlo inmediatamente. Evitar esta
   compilación descartada beneficia al fallback sin cambiar sus resultados.
   La construcción exacta descartada se midió por separado: 4,06 / 6,09 /
   670,55 µs en los tres tamaños de consulta sintética. Depende del patrón.
2. Una caché acotada de matchers compilados puede ahorrar construcción cuando
   se repiten consultas. La clave debe incluir patrón/modo y todas las opciones
   del matcher. Se cachea código compilado, nunca los resultados ni la fuente:
   los archivos deben leerse y comprobarse normalmente. lru es una opción de
   implementación; no se agregó ni se midió esa librería en este experimento.
3. ranking.rs tokeniza la línea para filtrar y el contexto para fit/embed BM25.
   Hay trabajo a estudiar, pero línea y contexto no son intercambiables y no
   se midió una optimización del ranking aquí.
4. No activar Aho indiscriminadamente ni calcular todo el fallback en paralelo
   al exacto: ambos prototipos presentan regresiones. Tampoco hay evidencia
   aquí para sustituir el motor por Tantivy.

## Límites de cobertura

No se miden traversal, permisos, rutas protegidas, hash, contexto, creación/
serialización JSON, ranking BM25, límites de metadata, cancelación ni disco.
El programa prueba matching por línea, no la semántica completa de harness_search.
Las estructuras de resultados son listas de líneas; producción crea objetos JSON.
Solo se ensayan términos alfanuméricos sin metacaracteres, como el subconjunto
válido del fallback. No es una sustitución para regex arbitrarias o búsqueda AST.
Esta primera medición precede a la implementación de la caché y la eliminación
de la compilación duplicada. La implementación posterior, sus garantías y sus
mediciones específicas están documentadas en [CACHE.md](CACHE.md).

Referencias:
- https://docs.rs/grep-regex/latest/grep_regex/struct.RegexMatcherBuilder.html
- https://docs.rs/aho-corasick/latest/aho_corasick/struct.AhoCorasickBuilder.html
- https://docs.rs/lru/latest/lru/
