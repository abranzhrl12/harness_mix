# Agent Harness — distribución Windows x64

El ejecutable Rust funciona sin Node ni npm. Verifica la versión con `agent-harness.exe --version` y compara el SHA-256 del ZIP con el checksum incluido en la distribución.

Configura tu cliente MCP para iniciar el EXE con:

~~~text
--root C:/proyectos/mi-app
~~~

El cliente controla stdin/stdout; iniciar el EXE en una terminal no registra el servidor. Reinicia MCP después de actualizar y llama a `harness_bootstrap`.

Para un proyecto nuevo sin `.codegraph/`:

~~~powershell
.\agent-harness.exe --root "C:/proyectos/mi-app" --index init
.\agent-harness.exe --root "C:/proyectos/mi-app" --index rebuild
~~~

La publicación del índice utiliza transacciones por archivo y puede informar commits parciales. Leer archivos y buscar texto no requieren índice. No se migra ni sobrescribe un índice incompatible.

- [Instalación](https://github.com/abranzhrl12/harness_mix/blob/main/docs/INSTALLATION.md)
- [Comandos y límites](https://github.com/abranzhrl12/harness_mix/blob/main/docs/REFERENCE.md)
- [Rendimiento medido](https://github.com/abranzhrl12/harness_mix/blob/main/docs/PERFORMANCE.md)
- [SQL y TLS](https://github.com/abranzhrl12/harness_mix/blob/main/docs/SQL-QUICKSTART.md)

Se necesitan las herramientas del lenguaje para sus verificadores. Si Windows informa de bibliotecas MSVC faltantes, instala el runtime Visual C++ x64 correspondiente.

Conserva los avisos CodeGraph y Rolldown, Cargo.lock y el inventario de dependencias incluidos en el paquete. La existencia de un binario no certifica todos los clientes, plataformas ni paridad completa con CodeGraph.
