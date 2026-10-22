# Agent Harness

Servidor MCP nativo para Windows x64. Este paquete incluye un lanzador Node y el ejecutable Rust.

~~~powershell
npm install -g @abranzhrl1212/agent-harness
agent-harness --version
~~~

Registra el comando en tu cliente MCP con `--root` y la ruta absoluta del proyecto. Para usar el EXE directamente, localiza `bin/agent-harness.exe` dentro del paquete instalado (`npm root -g`). El formato de configuración depende del cliente; reinicia MCP después de actualizar.

Comienza con `harness_bootstrap` y reutiliza `data.session`. El backend Rust ofrece 22 herramientas unificadas para archivos, símbolos, grafo, Git, edición, verificación, trabajos, SQL, sesiones y datos.

- [Instalación y configuración](https://github.com/abranzhrl12/harness_mix/blob/main/docs/INSTALLATION.md)
- [Comandos y ejemplos actuales](https://github.com/abranzhrl12/harness_mix/blob/main/docs/REFERENCE.md)
- [Tiempos medidos y metodología](https://github.com/abranzhrl12/harness_mix/blob/main/docs/PERFORMANCE.md)
- [Conexión SQL, TLS y permisos](https://github.com/abranzhrl12/harness_mix/blob/main/docs/SQL-QUICKSTART.md)

Leer y buscar no necesitan índice. `--index init` y `--index rebuild` crean/preparan uno opcional en el proyecto; escriben en `.codegraph/` y no certifican cobertura global. El watcher automático requiere Windows, perfil full e índice existente compatible.

La instalación no configura clientes ni credenciales. Las verificaciones de proyectos requieren sus toolchains y pueden ejecutar código; no son un sandbox. Conserva los avisos de CodeGraph y Rolldown incluidos.
