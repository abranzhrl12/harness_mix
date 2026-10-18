# Medición local: Rust release frente a CodeGraph Node

Fecha: 2026-10-17. Windows. Comparación CLI, no benchmark de la sesión MCP
persistente ni garantía de equivalencia semántica.

Proveedor: CodeGraph 1.6.2. Nativo: agent-harness 0.7.0 release.
SHA-256 del ejecutable nativo medido:
`863cbafd0189f2ea8988f426b6bd97fb2a7219a9d2251919b384ccc89ae6246f`.

Se copiaron 67 archivos Rust reales desde src, crates/domain/src y
crates/application/src a proyectos temporales independientes. Ambos comenzaron
con una base vacía schema 11. El original y su índice no se abrieron para escribir.
Rust utilizó una copia fijada del ejecutable release y PATH vacío. Node utilizó
la CLI instalada de CodeGraph. El ejecutor de pruebas puede compilar en debug:
lo medido es el proceso externo release, no los adaptadores del ejecutor.

Se alternó el orden de ejecución. Compilación y preparación de copias excluidas;
arranque del proceso incluido; caché del sistema de archivos NO vaciada.

| Operación | Rust release | CodeGraph Node |
| --- | ---: | ---: |
| Indexado completo, mediana de 3 ejecuciones | 5.934 s | 3.768 s |
| Sincronización sin cambios, mediana de 9 ejecuciones | 67.261 ms | 981.430 ms |

Indexado Rust (ms): 5859.202, 5934.466, 6746.912.
Indexado Node (ms): 3619.766, 3768.473, 5616.512.
Sync Rust (ms): 64.134, 66.509, 66.934, 67.229, 67.261, 70.200,
72.178, 72.608, 73.990.
Sync Node (ms): 945.964, 956.740, 960.117, 962.405, 981.430, 999.443,
1001.777, 1008.286, 1032.491.

Ambos indexaron 67 archivos y 1191 nodos. Rust produjo 2326 aristas; Node 3897.
Esto impide atribuir una equivalencia funcional a los tiempos. La diferencia
de aristas NO mide directamente capacidades faltantes del harness: hace falta
clasificar los casos que consumen sus consultas, no implementar todo upstream.

El ejecutable medido ocupa 26863104 bytes (25.62 MiB). No extrapolar estos
tiempos al harness completo: su versión antigua también cacheaba sincronizaciones
y reutilizaba conexiones. Las consultas persistentes todavía no se compararon.

Reproducción (rutas explícitas de ejecutables locales):

```powershell
$env:HARNESS_NATIVE_BINARY = 'E:\harnermix\.agents\harness\native\core\target\release\agent-harness.exe'
$env:HARNESS_CODEGRAPH_CLI = 'C:\Users\Abraham\AppData\Roaming\npm\codegraph.cmd'
cargo test --locked --test real_core_validation real_core_release_vs_provider_cli -- --ignored --nocapture
```

La validación previa debug del mismo perfil pasó creación, indexado, consulta de
resolve_exact, altas, cambios, bajas y reindexación: 11.640 s para el indexado y
15.188 s para reindexar. Son muestras separadas, no una razón de aceleración
controlada entre debug/release.
