# Instalación y configuración

[Inicio](../README.md) · [Comandos](REFERENCE.md) · [Rendimiento](PERFORMANCE.md)

## 1. Elegir la instalación

### npm: Windows x64

Necesitas Node y npm para instalar y ejecutar el lanzador:

~~~powershell
npm install -g @abranzhrl1212/agent-harness
agent-harness --version
~~~

Para actualizar, repite la instalación y reinicia la conexión MCP. La versión consultada en el registro el 21 de octubre de 2026 es `0.8.9`; puedes fijarla con `npm install -g @abranzhrl1212/agent-harness@0.8.9`.

El paquete no configura automáticamente un cliente ni modifica proyectos. Para localizar el ejecutable nativo instalado, ejecuta `npm root -g` y añade `/@abranzhrl1212/agent-harness/bin/agent-harness.exe`. Usar ese EXE directamente evita depender del lanzador Node durante la sesión MCP.

### Ejecutable sin npm

Consulta los archivos disponibles en [GitHub Releases](https://github.com/abranzhrl12/harness_mix/releases). Si la versión deseada tiene un ZIP Windows x64, extráelo en una carpeta estable y contrasta su SHA-256 con el checksum de esa distribución:

~~~powershell
Get-FileHash .\agent-harness-VERSION-windows-x86_64.zip -Algorithm SHA256
C:/herramientas/agent-harness.exe --version
~~~

El EXE directo no requiere Node. Si Windows informa de bibliotecas MSVC faltantes, instala el runtime Microsoft Visual C++ x64 correspondiente.

### Código fuente

Requiere Git, Rust 1.88 o posterior y el linker de la plataforma. En Windows con Rust MSVC, prepara las herramientas de compilación de Visual C++.

~~~powershell
git clone https://github.com/abranzhrl12/harness_mix.git
cd harness_mix
cargo build --release --locked
.\target\release\agent-harness.exe --version
~~~

En Unix el nombre del binario es `target/release/agent-harness`. La publicación nativa del índice y las garantías de escritura dependen de Windows; no se afirma compatibilidad completa con otras plataformas.

## 2. Registrar el servidor en el cliente MCP

El formato y la ubicación del archivo dependen del cliente. Para clientes que utilizan `mcpServers`, añade esta entrada conservando los demás servidores:

~~~json
{
  "mcpServers": {
    "harness": {
      "command": "C:/herramientas/agent-harness.exe",
      "args": ["--root", "C:/proyectos/mi-app"]
    }
  }
}
~~~

Cambia ambas rutas. Puedes reutilizar un EXE con proyectos diferentes cambiando `--root`. El cliente inicia el proceso por stdio; abrirlo en una terminal no registra una conexión. Reconecta el servidor desde el cliente.

Después ejecuta `harness_bootstrap` con `{}` o `{"compact":true}`, comprueba `data.root` y reutiliza `data.session`. Bootstrap no acepta `action`.

## 3. Índice y configuración opcionales

~~~powershell
agent-harness --root "C:/proyectos/mi-app" --index init
agent-harness --root "C:/proyectos/mi-app" --index rebuild
agent-harness --root "C:/proyectos/mi-app" --init-config
~~~

- `init` solo crea el índice si no existe `.codegraph`; no sobrescribe ni migra.
- `rebuild` reextrae el alcance nativo en el índice existente; consulta el resultado parcial antes de reintentar.
- `--init-config` crea `.agents/harness.config.json` sin credenciales y sin sobrescribir. No crea índices.
- El uso básico de archivos no necesita índice ni ese JSON.
- SQL puede detectar una cadena de conexión del entorno o `.env`: [guía SQL](SQL-QUICKSTART.md).

## Perfiles y diagnóstico

Configura las variables en el entorno del servidor, según el formato de tu cliente:

| Variable | Valor predeterminado | Uso |
| --- | --- | --- |
| `HARNESS_TOOL_PROFILE` | `full` | `investigate`, `edit` o `full`; limita catálogo y operaciones |
| `HARNESS_INDEX_WATCH` | Automático en Windows/full con índice existente | `0` desactiva; `1` exige arranque del watcher |
| `HARNESS_MCP_EAGER_TOOLS` | Activado | `0` desactiva la sugerencia de carga inicial al cliente |

`investigate` restringe operaciones mutantes aunque una herramienta también tenga acciones de escritura. `edit` añade edición y verificación; `full` ofrece todo el catálogo. Un cambio de perfil requiere reiniciar el servidor y no concede autorización para modificar un proyecto.

Ante problemas usa `harness_doctor {"action":"env","versions":false}`. `versions:true` ejecuta sondas de versiones; `connection:true` añade comprobaciones de conexión cuando están configuradas. Los verificadores necesitan sus toolchains locales; no se descargan automáticamente.
