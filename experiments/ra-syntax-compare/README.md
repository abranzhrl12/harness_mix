# Experimento local: `syn` frente a `ra_ap_syntax`

Este experimento compara llamadas de parser reales en un proceso release. No
cambia el parser de producción ni mide publicación/indexación del Harness.

## Reproducir

```powershell
$raBenchTarget = Join-Path $env:TEMP 'harness-ra-syntax-compare-target'
$env:CARGO_TARGET_DIR = $raBenchTarget
$env:CARGO_BUILD_JOBS = '2'
cargo run --locked --release --manifest-path experiments/ra-syntax-compare/Cargo.toml
```

El workspace es independiente (`[workspace]` local) y el target va a `%TEMP%`,
fuera del target compartido. `Cargo.lock` fija dependencias. El primer build
observado, incluyendo descargas y espera por el lock del cache Cargo, tardó
127.98 s. No es tiempo de ejecución. Los resultados se tomaron tras compilar y
con warm-up por caso. Ambos parsers corrieron en el mismo ejecutable `--release`
(Cargo release, `opt-level=3`).

## Límites

- `syn` está fijado a `2.0.119`, la versión del lock de Harness; `ra_ap_syntax`
  a `0.0.289`. El paquete RA declara edition 2024 y MSRV 1.86; la raíz declara
  Rust 1.88. El experimento se compiló con `rustc 1.99.0`.
- RA analiza con `Edition::Edition2024`. `syn::parse_file` no recibe edición.
  Fixtures comunes se pasan con el mismo texto; BOM y shebang usan la
  normalización que aplica Harness antes de ambas llamadas.
- `synParseDropReset` mide `syn::parse_file`, acceso a `items.len`, drop y la
  invalidación `proc_macro2` de spans usada por el adaptador. `synWithPolicy`
  replica límite 256 KiB, escaneo de profundidad 64, normalización y parser;
  no invoca `OperationContext` ni el callback real. RA mide
  `SourceFile::parse` más consulta de errores. El conteo de nodos CST RA es
  fuera del cronómetro y no equivale a `synTopLevelItems`.
- Cada punto usa 3 warm-ups y nueve muestras de 20 parseos por parser. El
  cronómetro cubre las 20 operaciones y se divide por 20; se reporta mediana
  de ns/operación. `black_box` envuelve fuente/resultado. El AST syn se destruye
  dentro de cada operación y luego se invalidan sus spans; el CST RA, su lista
  de errores y su nodo raíz también se destruyen dentro de cada operación. La
  consulta `errors()` de RA sí cuenta como parte de su validación. Fixtures
  válidos incluyen docs, attrs, CRLF y emoji. Inválidos
  agregan una función truncada al final. RA produce CST con errores; `syn`
  rechaza sin AST. Esa diferencia de política domina el tiempo de rechazo.
- El mayor fixture es 196,020 B, bajo el límite actual de 256 KiB. No se usó
  el rechazo por tamaño como ventaja de velocidad.
- No se midió extracción equivalente: falta portar al CST navegación de items
  anidados, firmas, docs, spans/UTF-16, IDs y filtros de `index_rust.rs`.
  Tampoco se midieron memoria/RSS, diagnósticos completos, resolución ni SQLite.
  Esto no demuestra paridad ni mejora end-to-end del indexador.
- Las corridas ocurrieron mientras había otros procesos `cargo` en el equipo;
  no se detuvieron. Son mediciones exploratorias bajo carga concurrente, no un
  benchmark controlado. No se observó RSS/peak working set.
- Para identificar los fixtures reproducidos: el generador determinista está
  en `src/main.rs`; registra sus hashes y el de `Cargo.lock` con
  `Get-FileHash` antes de repetir.

Hashes de esta captura:

- `src/main.rs`: `768BCEBF9CB29D11682D45DB0C9945F782EE8EEA1A765450FC62A506E9398E29`
- `Cargo.lock`: `C7AD74D09BCCEEF7C1A916D2B1418219FA2581B6ABC3041F8EA867BC2AF95981`

Resultados capturados: [`results-2026-10-21.jsonl`](results-2026-10-21.jsonl).
