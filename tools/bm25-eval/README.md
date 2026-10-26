# Ensayo local de BM25 para Harness

Experimento aislado: `bm25 = 2.3.2`, sin features predeterminadas. No añade
dependencias al ejecutable ni activa ranking en `harness_search`.

## Reproducir

Desde la raíz del proyecto, sustituir `<raíz absoluta>` por su ruta:

```powershell
cargo run --release --locked --manifest-path tools/bm25-eval/Cargo.toml --target-dir target/bm25-eval -- '<raíz absoluta>' 'tools/bm25-eval/results.json'
```

El evaluador lee los archivos Rust versionados de `src/` y `crates/`, excluye
archivos de tests y usa Syn para excluir módulos/funciones de tests incrustados.
No escribe en esos archivos. Los documentos son ventanas de hasta cinco líneas,
conservando ventanas solapadas. El tokenizador preserva identificadores completos
y añade sus componentes camelCase/snake_case; no aplica stemming ni sinónimos.

Se comparan tres variantes con las mismas etiquetas de archivo esperado:

1. Coincidencia literal, ordenada por ruta y línea: referencia aproximada del
   recorrido actual, no una llamada real al motor/MCP.
2. Las mismas coincidencias literales, reordenadas por BM25.
3. Candidatos con al menos un término de consulta, ordenados por BM25. Esta
   variante modifica la recuperación, por lo que su mejora no se atribuye
   exclusivamente al ranking.

## Resultado exploratorio

Dos ejecuciones sobre 435 archivos y 62 344 ventanas produjeron los mismos
aciertos de archivo esperado en las primeras cinco filas:

| Consultas | Orden por ruta | BM25 sobre coincidencias exactas | Por términos + BM25 |
| --- | ---: | ---: | ---: |
| 9 consultas cortas/ambiguas | 1/9 | 2/9 | 2/9 |
| 5 consultas descriptivas de varias palabras | 0/5 | 0/5 | 3/5 |
| 5 identificadores precisos | 5/5 | 5/5 | 5/5 |
| Total | 6/19 | 7/19 | 10/19 |

Ejemplos: `search cursor pagination` llevó el archivo esperado al puesto 1 con
recuperación por términos; `metadata bytes budget`, al 1; `regex case sensitive`,
al 3. En cambio, reordenar la consulta literal `cursor` empeoró su archivo
etiquetado del puesto 23 al 43, y `session`, del 12 al 30.

El ajuste y ranking de candidatos literales no vacíos tuvo medianas de 5,62 y
6,40 ms, con casos individuales de hasta 178 ms. Estos tiempos no incluyen leer
el repositorio ni filtrar candidatos. El prototipo de recuperación por términos
retokeniza el corpus por consulta y tarda cientos de milisegundos o más; no es
una integración optimizada ni demuestra reducción de latencia del LLM.

Conclusión: este ensayo no respalda activar BM25 globalmente por defecto como
simple reordenador de grep. Sí justifica evaluar recuperación por términos para
consultas descriptivas, conservando la búsqueda exacta para identificadores.

## Límites de la evidencia

- Son 19 consultas elegidas manualmente, con un archivo esperado por consulta;
  otros archivos también pueden ser relevantes, especialmente con términos
  ambiguos como `context` o `session`.
- Las etiquetas y el corpus pertenecen a este proyecto Rust, no a web-fast-v1.
- Dos repeticiones comprueban consistencia, no significancia estadística.
- Había compilaciones concurrentes: los tiempos son orientativos.
- No se midieron tareas completas del LLM, tokens consumidos ni número real de
  llamadas MCP. Tampoco se evaluó ranking de símbolos o diversidad por archivo.

Los resultados completos, posiciones y primeras cinco filas están en
`results.json` y `results-repeat.json`.
