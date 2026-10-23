# Harness MCP

Usa Harness MCP como vía principal para investigar, leer, editar y verificar este
proyecto. Descubre sus nombres reales, ejecuta `harness_bootstrap`, comprueba la
raíz y reutiliza `data.session`. Sigue los esquemas actuales, no versiones anteriores.

## Carga solo la guía necesaria

| Trabajo | Skill local |
| --- | --- |
| Buscar, leer código o comprender relaciones | `.agents/skills/harness-read/SKILL.md` |
| Reemplazar, escribir, mover y verificar código | `.agents/skills/harness-edit/SKILL.md` |
| Consultar SQL Server o PostgreSQL | `.agents/skills/harness-sql/SKILL.md` |
| Leer un XLSX existente | `.agents/skills/harness-xlsx/SKILL.md` |
| Inspeccionar o previsualizar SVG | `.agents/skills/harness-svg/SKILL.md` |
| Instalar, inicializar o verificar el CLI/servidor Harness | `.agents/skills/harness-setup/SKILL.md` |

Lee completa la skill aplicable antes de actuar; combina guías solo cuando la
tarea lo requiera. Un símbolo conocido puede leerse directamente, sin búsqueda,
outline ni firma previos.

## Invariantes

- Conserva fuente necesaria, documentación, hashes y avisos de omisiones,
  truncamiento y frescura. Un resumen no equivale a toda la evidencia.
- Respeta cambios concurrentes, rutas protegidas, scopes externos y perfil.
  Investigar no autoriza editar; disponer de herramientas no amplía permisos.
- No inicialices, sincronices ni reconstruyas el índice por rutina. Lectura y
  búsqueda no requieren índice; las relaciones necesitan uno compatible.
  Indexar escribe en `.codegraph/` y requiere autorización. No borres índices
  para repararlos. Para un proyecto sin índice y con autorización, usa primero
  `--root <proyecto> --init-index` y después `--root <proyecto> --reindex`.
  Si ya existe, inspecciona compatibilidad y usa mantenimiento admitido.
- Ante transporte, timeout o runtime fallido, detén reintentos equivalentes,
  diagnostica con `harness_doctor` y comunica el bloqueo. No eludas el fallo con
  acceso directo o shell. Una capacidad no soportada es distinta de un fallo:
  explica el límite antes de utilizar una alternativa autorizada.
- Sintaxis válida no demuestra compilación ni corrección. Reporta cambios,
  verificaciones ejecutadas y pendientes. Los checkpoints no son snapshots.
  No hagas commit ni push sin petición.

## Si falta la conexión

El ejecutable conocido es `E:/harness_mix/target/release/agent-harness.exe`.
Comprueba existencia y `--version`; no descargues binarios desconocidos. La entrada
MCP usa ese comando con `args: ["--root", "RUTA_ABSOLUTA_DEL_PROYECTO"]`.

Configura solo si el usuario lo pidió: identifica cliente y archivo real,
preserva otros servidores y no cambies credenciales o permisos. Si falta el
binario, localiza una instalación conocida o compila con `cargo build --release
--locked` cuando la configuración forme parte de la tarea. No inicies stdio en
una terminal como sustituto de registrarlo en el cliente. Si falta reconexión,
informa el paso pendiente; no declares conexión operativa sin bootstrap real.
