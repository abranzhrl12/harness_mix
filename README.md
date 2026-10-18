# Agent Harness

Servidor MCP en Rust para que tu asistente explore, lea, edite y verifique proyectos. Un ejecutable, sin Node ni npm en ejecución.

## Instalación

Descarga el ejecutable Windows x64 desde [Releases](https://github.com/abranzhrl12/harness_mix/releases) y guárdalo en una carpeta estable. También puedes compilarlo con Rust 1.88 o posterior:

```sh
cargo build --release --locked
```

El binario queda en `target/release/agent-harness.exe` (Windows) o `target/release/agent-harness` (Unix). La distribución precompilada está orientada a Windows; otras plataformas requieren validación.

## Configuración

Añade esta entrada a la configuración MCP de tu cliente; cambia únicamente las dos rutas:

```json
{
  "mcpServers": {
    "harness": {
      "command": "C:/herramientas/agent-harness.exe",
      "args": ["--root", "C:/proyectos/mi-app"]
    }
  }
}
```

Reinicia la conexión MCP. Puedes reutilizar el mismo ejecutable en varios proyectos cambiando `--root`. No necesitas copiar este repositorio al proyecto ni crear archivos de configuración para el uso básico. El formato de registro puede variar según el cliente.

Si el cliente inicia el proceso dentro del proyecto, puedes omitir `args`: harness detecta la raíz desde las variables del entorno o busca `.git`/`.agents` desde el directorio actual. Con `--root` eliges la carpeta explícitamente.

## Índice de código (opcional)

Para usar la navegación del grafo, ejecuta desde la raíz de tu proyecto, con el binario en PATH:

```sh
agent-harness --init-index
agent-harness --reindex
```

Después usa `agent-harness --sync` para actualizarlo. También puedes añadir `--root <proyecto>` antes de la opción. Estos comandos escriben en `.codegraph/`; `--init-index` exige que esa carpeta no exista. No hace falta un índice para leer archivos o buscar texto.

## Límites y desarrollo

Versión experimental `0.7.1-rc.1`. Puede trabajar con carpetas de distintos proyectos, pero el soporte de símbolos, grafo y verificadores depende del lenguaje; no ofrece paridad completa con CodeGraph Node. Los verificadores requieren las herramientas del lenguaje instaladas.

```sh
cargo test --locked --workspace -- --test-threads=1
agent-harness --help
```

[Referencia completa](docs/REFERENCE.md) · [Estado y límites](docs/MIGRATION-STATUS.md) · [Arquitectura](docs/ARCHITECTURE.md) · [Distribución Windows](release/README.md)
