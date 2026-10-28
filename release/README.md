# Agent Harness — distribución Windows x64

El ejecutable Rust funciona sin Node ni npm. Verifica la versión con `agent-harness.exe --version` y compara el SHA-256 del ZIP con el checksum incluido en la distribución.

Configura tu cliente MCP para iniciar el EXE con:

~~~text
--root C:/proyectos/mi-app
~~~

El cliente controla stdin/stdout; iniciar el EXE en una terminal no registra el servidor. Reinicia MCP después de actualizar y llama a `harness_bootstrap`.

Las catorce skills incluidas en esta versión se instalan por proyecto: siete de Harness y siete de trabajo del agente (depuración, pruebas, verificación y arquitectura), con sus documentos auxiliares y licencias. Para preparar un proyecto nuevo, ejecuta:

~~~powershell
.\agent-harness.exe init --root "C:/proyectos/mi-app"
~~~

`init` instala las skills, crea `.agents/harness.config.json` con `database.autoDetect=true` si falta y deja la sincronización automática optativa activada en ese proyecto. No busca ni escribe credenciales. Si el config ya es un objeto JSON válido, lo conserva. También crea `.agents/harness-mcp.example.json` con la ruta canónica del ejecutable que corre y la ruta del proyecto. El ejemplo no modifica ni registra la configuración real del cliente; revísalo y añade el servidor al cliente elegido. Si ya existe un ejemplo MCP diferente, `init` lo conserva y reporta conflicto.

Al actualizar el EXE, el arranque de ese proyecto sincroniza las skills si el opt-in está activo. También puedes repetir `init` para comprobar la preparación del proyecto sin reemplazar su configuración personalizada, o ejecutar solo la sincronización de skills:

~~~powershell
.\agent-harness.exe skills sync --root "C:/proyectos/mi-app"
~~~

La fuente versionada está en `.agents/skills`; `crates/skills/src/catalog.rs` enumera los `SKILL.md`, documentos auxiliares y licencias que el ejecutable incluye al compilar. Las skills de trabajo proceden de `obra/superpowers` y `mattpocock/skills`; cada directorio conserva su licencia MIT. `init` copia estos archivos sin descargarlos ni ejecutar los scripts de ejemplo. Un ejecutable anterior conserva su catálogo hasta que se actualice. El manifiesto `.agents/skills/.harness-manifest.json` registra su SHA-256, la versión Cargo del ejecutable y `contractVersion` leído de `contracts/tool-contract.json`. Un formato de manifiesto desconocido detiene la sincronización sin escribir.

`--check` devuelve un código distinto de cero si faltan archivos, hay cambios o se detectan conflictos; `--dry-run` muestra el plan sin escribir. Los archivos locales divergentes se conservan y se reportan para resolución manual. Para sincronizar automáticamente al iniciar el servidor de un único proyecto, usa `--enable-auto`; esta preferencia se guarda en `.agents/harness-skills-sync.json` dentro de ese proyecto. Si no hay actualizaciones pendientes, no reescribe las skills ni el manifiesto. `--disable-auto` la desactiva. La configuración de servidores con `--workspace-config` no aplica este opt-in.

La instalación atómica se hace por archivo; un fallo a mitad puede dejar parte del paquete actualizado y el manifiesto se publica al final. El sincronizador rechaza symlinks/junctions en las rutas administradas, limita las lecturas y serializa otras ejecuciones de sync del mismo proyecto. No puede impedir cambios concurrentes hechos por otros editores o procesos que sustituyan directorios durante la operación. Si un cierre forzado deja `.agents/.harness-skills-sync.lock`, confirma que no haya otra sincronización antes de quitarlo.

`init` tampoco hace una transacción global: ante un error de escritura, revisa el informe JSON y conserva los archivos que ya se completaron.

Al recargar las skills puede ser necesario iniciar una sesión nueva: Harness no garantiza que la sesión activa vuelva a leerlas.

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
