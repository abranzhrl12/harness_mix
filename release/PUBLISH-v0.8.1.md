# Publicar Agent Harness 0.8.1 en npm

Paquete listo: `E:/harness_mix/target/dist/abranzhrl1212-agent-harness-0.8.1.tgz`.
Nombre: `@abranzhrl1212/agent-harness`. Plataforma: Windows x64.

Ejecuta en PowerShell con una cuenta que pueda publicar en este scope:

```powershell
npm login --registry=https://registry.npmjs.org
npm whoami --registry=https://registry.npmjs.org
npm publish "E:/harness_mix/target/dist/abranzhrl1212-agent-harness-0.8.1.tgz" --access public --registry=https://registry.npmjs.org
```

Para comprobar la publicación:

```powershell
npm view @abranzhrl1212/agent-harness@0.8.1 version --registry=https://registry.npmjs.org
```

## Verificación realizada

- La versión 0.8.1 estaba disponible en npm al comprobarla el 18 de octubre de 2026.
- El binario del tarball coincide por SHA-256 con `target/release/agent-harness.exe`
  y `npm-dist/bin/agent-harness.exe`.
- El tarball contiene solo siete archivos: ejecutable, launcher, manifiesto,
  README, verificador y los dos avisos de licencia.
- Pasó `node scripts/smoke-mcp.cjs npm-dist/bin/agent-harness.exe --watch-index`:
  catálogo de 28 herramientas, impacto indexado con un llamador, preview sin
  escritura, observe, PostgreSQL status sin conectar, aliases, jobs, cancelación
  y recuperación del historial después de reiniciar.
- Pasó la simulación `npm publish <tarball> --dry-run --access public`.
- Pasó la ejecución del tarball mediante `npm exec`: `agent-harness 0.8.1`.
- No se publicó el paquete. La sesión local de npm necesita autenticación.

SHA-256 del ejecutable:

```text
b3de477e211633f4885d8744d593fc7937ec489ccc6152ee526a7f38e18da3d7
```

Integridad del tarball devuelta por npm:

```text
sha512-Cc1VEIiAOdMMC7Z5aY9iA1M0l4K5nP+pIBlP8FPOHYc9eeykeczZf43krj9VYLlylZjm5RAB7krGJ7DKmj69iQ==
```

Después de instalar la actualización, reinicia la conexión MCP para refrescar el
catálogo. Los límites de impacto, preview y observe están en
`docs/PLANNING-TOOLS.md` y en el README incluido en npm.
