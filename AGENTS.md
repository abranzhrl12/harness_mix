# Instrucciones para el agente: usar Harness MCP

Usa Harness MCP como vía principal para investigar, leer, editar y verificar este
proyecto. Conserva el contexto útil: firmas, documentación, cuerpos necesarios,
relaciones, hashes y avisos de resultados parciales. No confundas una salida
resumida con toda la evidencia disponible.

## 1. Comprueba la conexión

Busca las herramientas Harness entre las herramientas MCP disponibles. Usa su
nombre real, incluido el namespace del cliente si lo añade.

Si están disponibles:

1. Llama a `harness_bootstrap` para iniciar o recuperar la sesión.
2. Comprueba que la raíz corresponde al proyecto solicitado.
3. Copia `data.session` en el argumento `session` de las llamadas posteriores que
   lo admitan. Reutiliza la sesión durante la tarea.
4. Consulta `harness_doctor` si hay dudas sobre disponibilidad, índice o entorno.
5. Sigue los esquemas anunciados por esta conexión; no inventes herramientas,
   parámetros ni capacidades basándote en una versión anterior.

Si no están disponibles, prepara la conexión del apartado siguiente. No afirmes
que usaste Harness si no recibiste resultados reales de sus herramientas.

## 2. Configura Harness si falta

El ejecutable optimizado de este equipo está en:

```text
E:/harness_mix/target/release/agent-harness.exe
```

Comprueba que existe y que `--version` funciona. Si falta, localiza una instalación
existente o compila el repositorio del harness con `cargo build --release --locked`
cuando ese repositorio esté disponible y la configuración forme parte de la tarea.
No descargues ni ejecutes un binario de origen desconocido.

Identifica el cliente MCP y su archivo real de configuración. Integra esta entrada
sin borrar otros servidores ni opciones:

```json
{
  "mcpServers": {
    "harness": {
      "command": "E:/harness_mix/target/release/agent-harness.exe",
      "args": ["--root", "RUTA_ABSOLUTA_DEL_PROYECTO"]
    }
  }
}
```

Sustituye el marcador por la raíz absoluta del proyecto actual. `--root` debe
apuntar al proyecto donde se hará el trabajo, no siempre a `E:/harness_mix`.
El mismo ejecutable sirve para distintos proyectos. Ajusta su ruta si está
instalado en otro lugar. El formato y la ubicación de la configuración dependen
del cliente: compruébalos, no los supongas.

Si el usuario pidió configurar la conexión y tienes acceso a su configuración,
haz el cambio conservando el contenido existente. No cambies configuración
global, credenciales ni permisos fuera del alcance autorizado. Si falta conocer
el cliente o no puedes modificar su configuración, entrega la entrada ya adaptada
y explica exactamente qué dato o acción falta.

El cliente inicia el servidor por stdio; no lo dejes abierto en una terminal
esperando que eso registre la conexión. Reconecta o reinicia el MCP desde el
cliente y comprueba `harness_bootstrap`. Si esa acción requiere intervención del
usuario, indícalo; no declares la conexión operativa antes de verificarla.

## 3. Elige la herramienta más directa

| Necesidad | Herramienta y selección |
| --- | --- |
| Localizar archivos o texto literal | `harness_search`, `kind=files` o `kind=text` |
| Ver estructura de un archivo | `harness_symbols`, `action=outline`, `path`; usa `query` para priorizar |
| Leer una implementación | `harness_symbols`, `action=read`, `path`, `symbol`, `view=full` |
| Consultar solo firma o ubicación | Misma acción `read`, `view=signature` |
| Leer un fragmento de texto | `harness_read`, con rango acotado |
| Comprender callers, callees y relaciones | `harness_explore`, preferiblemente con símbolo y archivo concretos |
| Recuperar contenido omitido | `harness_detail` usando el `ref` recibido |
| Proyectar un resultado guardado | `harness_data_query` con `resultRef` y consulta acotada |
| Estado o diff de Git | `harness_git`, `action=status` o `action=diff` |
| Reemplazar código | `harness_replace` con hash vigente y texto original exacto |
| Mover o renombrar archivos | `harness_move`, `from` y `to` relativos |
| Compilar o probar | `harness_verify`; para trabajo en segundo plano, `harness_jobs` |

En `harness_symbols` el nombre se selecciona con `symbol`, no con `name`.
Usa las entradas unificadas actuales; el servidor Rust ya no anuncia
`harness_outline`, `harness_read_symbol`, `harness_git_status` ni `harness_git_diff`.
SQL usa `harness_sql` y hojas de cálculo usan `harness_xlsx`, con las acciones de
sus esquemas actuales.

Agrupa con `harness_batch` únicamente lecturas independientes admitidas por su
esquema. No agrupes pasos donde el segundo necesita el resultado del primero.

## 4. Conserva contexto y edita con precisión

- Empieza con una consulta concreta. Amplía solo lo que falte para resolver la
  tarea; no vuelques todo el repositorio ni releas contenido vigente ya recibido.
- Si necesitas implementar o editar, pide `view=full` directamente. Una firma,
  resumen o Markdown con números de línea no es texto válido para `oldText`.
- No omitas documentación, cuerpos o relaciones relevantes solo para ahorrar
  tokens. Recupera los detalles necesarios mediante `detail`, `resultRef` o el
  `nextCall` devuelto por la herramienta.
- Respeta truncamiento, omisiones, ambigüedad y frescura. Un resultado vacío y
  acotado no demuestra ausencia; un índice no garantiza cobertura completa.
- Para varias modificaciones del mismo archivo, prefiere una llamada a
  `harness_replace` con `chunks`. Todos se refieren al mismo original y hash;
  cada bloque debe ser único y los rangos no deben solaparse.
- Ante un hash obsoleto, relee y replantea la edición. No fuerces una escritura
  externa para saltarte el rechazo.
- `harness_move` no sobrescribe destinos ni actualiza imports. Comprueba
  `changed`, `verified` e `index.status`: si el archivo ya se movió y solo falló
  el índice, no repitas el movimiento.
- Una edición con sintaxis válida no demuestra compilación ni corrección.
  Ejecuta las verificaciones apropiadas al cambio autorizado y distingue sus
  resultados de las comprobaciones exclusivamente sintácticas.

## 5. Índice, límites y fallos

No necesitas índice para empezar a leer o buscar. Usa símbolos locales según el
lenguaje soportado. Para relaciones del grafo hace falta un índice compatible.
No inicialices, reconstruyas ni sincronices el índice por rutina durante una
consulta de solo lectura; hacerlo escribe en `.codegraph/`.

Cuando el usuario autorice indexar un proyecto sin `.codegraph`, usa el ejecutable
con `--root <proyecto> --init-index` y después `--root <proyecto> --reindex`.
`--init-index` exige que `.codegraph` no exista. Si ya existe, inspecciona su
compatibilidad y usa el mantenimiento admitido; no borres ni sobrescribas el
índice para resolver un error. La actualización automática requiere Windows,
perfil `full` e índice existente compatible.

Investigar o revisar no autoriza editar código. Respeta las rutas protegidas,
los scopes externos y el perfil de herramientas anunciado. No amplíes permisos
ni cambies de perfil para eludir un rechazo.

Ante fallos de transporte, timeout o runtime, detén los reintentos equivalentes,
diagnostica la conexión y comunica el bloqueo. No sustituyas silenciosamente la
operación fallida por acceso directo a archivos o shell. Un formato no soportado
puede investigarse con otras herramientas de lectura admitidas por Harness.

Los checkpoints actuales registran objetivos, decisiones e historial: no son
snapshots restaurables. No inventes operaciones de undo/redo ni restaures archivos
con comandos destructivos.

Al terminar, informa qué cambió, qué se verificó y qué limitación relevante queda.
