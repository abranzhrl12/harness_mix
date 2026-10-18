# PostgreSQL en Harness: requisitos y plan

Estado: propuesta, todavía no implementada. Este documento permite planificar
el soporte de PostgreSQL equivalente al acceso de solo lectura de SQL Server.
No activa conexiones ni cambia el ejecutable actual.

## Qué existe hoy

`harness_sql` ofrece `action=status|search|describe|query` para SQL Server mediante
Tiberius. La configuración actual se lee de `sqlServer` en
`.agents/harness.config.json`. El contrato está en `crates/domain/src/sql.rs` y
el adaptador en `src/adapters/sql.rs`.

Tiberius y su cadena de conexión no sirven para PostgreSQL. Tampoco se puede
reutilizar directamente el validador T-SQL ni cambiar `dbo` por `public` y asumir
que el resto es compatible.

## 1. Datos que debe proporcionar el usuario

| Dato | Ejemplo sin credenciales reales |
| --- | --- |
| Motor y versión del servidor | PostgreSQL; indicar versión instalada |
| Host accesible desde el proceso MCP | `db.example.internal` o `localhost` |
| Puerto | `5432`, o el asignado por el servicio |
| Base de datos | `mi_app` |
| Usuario dedicado | `harness_reader` |
| Autenticación | Contraseña mediante variable de entorno o mecanismo acordado |
| Esquemas y objetos autorizados | `public.clientes`, `reporting.*` |
| TLS | Host del certificado y CA requerida por el servidor |
| Red | Acceso directo, VPN o túnel ya establecido por el usuario |
| Límites | Filas, tamaño máximo de salida y timeout |

El rol debe tener `CONNECT`, `USAGE` sobre los esquemas y `SELECT` sobre los
objetos autorizados. No debe ser superusuario, dueño de las tablas ni tener
`BYPASSRLS`, permisos de escritura o roles heredados que los concedan. Respetar
las políticas RLS existentes. Un administrador debe revisar los permisos
efectivos, incluidos privilegios concedidos a `PUBLIC` y ejecución de funciones.
[Privilegios de PostgreSQL](https://www.postgresql.org/docs/current/ddl-priv.html).

No pegar contraseñas ni cadenas completas con secretos en el chat, AGENTS.md,
el repositorio, logs ni respuestas MCP. El agente no debe crear usuarios,
conceder permisos, cambiar el firewall o abrir túneles sin autorización expresa.

## 2. Driver y dependencias

Primera opción a evaluar: `tokio-postgres`, compatible con la arquitectura Tokio
del harness. Añadir un conector TLS con validación de certificados y hostname.
No se requiere `psql`, pgAdmin, ODBC ni libpq para el adaptador nativo; esas
herramientas pueden servir para diagnóstico externo autorizado.

Fijar versiones compatibles con el Rust mínimo declarado y comprobar el árbol
de dependencias. Elegir el backend criptográfico deliberadamente: que el driver
esté escrito en Rust no garantiza que todas las dependencias TLS sean Rust puro.
[Documentación del driver](https://docs.rs/tokio-postgres/latest/tokio_postgres/).

No añadir un pool de conexiones en la primera versión por rutina. Empezar con
conexiones acotadas por operación; evaluar reutilización después de medirla.
El trabajo de conexión del driver debe tener un ciclo de vida gestionado y
propagar sus fallos al resultado de la operación.

## 3. Configuración propuesta, aún no admitida

Extender `.agents/harness.config.json` conservando el bloque `sqlServer`:

```json
{
  "postgresql": {
    "enabled": true,
    "defaultSchema": "public",
    "allowedSchemas": ["public"],
    "maxRows": 50,
    "maxChars": 8000,
    "timeoutSeconds": 15,
    "schemaCacheSeconds": 300,
    "connection": {
      "source": "environment",
      "name": "HARNESS_PG_CONNECTION_STRING"
    },
    "tls": {
      "mode": "verify-full",
      "caFile": "C:/certificados/postgresql-ca.pem"
    }
  }
}
```

`HARNESS_PG_CONNECTION_STRING` sería una variable nueva, separada de
`HARNESS_SQL_CONNECTION_STRING`. Debe existir en el entorno del proceso MCP;
si se configura después de arrancarlo, reconectar el servidor. No habilitar
lecturas generales de `.env` ni devolver su contenido para resolver credenciales.

`tls.mode=verify-full` es una política propuesta del harness, no un valor que
deba pasarse sin traducción al driver. `tokio-postgres` 0.7.18 expone
`Disable`, `Prefer` y `Require`: exigir TLS y verificar CA/hostname son aspectos
que el adaptador y su conector deben implementar y comprobar conjuntamente.
[Modos del driver](https://docs.rs/tokio-postgres/latest/tokio_postgres/config/enum.SslMode.html).

Para conexiones remotas, exigir cifrado, cadena de confianza y coincidencia
del hostname; no desactivar la validación para resolver un error de certificado.
La semántica objetivo equivale a la de `verify-full` en libpq.
[TLS de PostgreSQL](https://www.postgresql.org/docs/current/libpq-ssl.html).

## 4. Una sola herramienta para ambos motores

Mantener `harness_sql` y sus cuatro acciones. Añadir un selector explícito
`engine="sqlserver"|"postgresql"`, sin crear cuatro herramientas PostgreSQL.

Preservar las llamadas existentes de SQL Server: si solo ese motor está
habilitado, pueden seguir omitiendo `engine`. Si únicamente PostgreSQL está
habilitado, el motor configurado puede ser el predeterminado. Si hay ambos y no
se ha establecido un predeterminado, exigir el selector; nunca consultar una
base distinta por una elección silenciosa.

Todos los ejemplos siguientes son del contrato FUTURO. El esquema actual
rechaza `engine` y no puede ejecutarlos todavía.

```json
{"action":"status","engine":"postgresql","connect":false}
```

```json
{"action":"status","engine":"postgresql","connect":true}
```

```json
{"action":"search","engine":"postgresql","query":"clientes","objectType":"table","schema":"public","limit":10}
```

```json
{"action":"describe","engine":"postgresql","name":"public.clientes","kind":"relation"}
```

```json
{
  "action": "query",
  "engine": "postgresql",
  "sql": "SELECT id, nombre FROM public.clientes WHERE id = $1 LIMIT 10",
  "parameters": [123]
}
```

Para PostgreSQL, proponer parámetros posicionales como array y placeholders
`$1`, `$2`, etc. El contrato actual de SQL Server usa un mapa; ampliar el tipo
de `parameters` conservando ese formato y validando la representación según el
motor. No interpolar valores en SQL ni mezclar parámetros T-SQL con PostgreSQL.
No hace falta traducir consultas entre dialectos.

Un identificador de esquema/tabla/columna no es un parámetro de valor. Resolver
nombres mediante catálogos con parámetros y, cuando sea inevitable generar un
identificador SQL, usar quoting revisado. Nunca concatenar nombres sin validar.

## 5. Solo lectura: controles necesarios

Cada operación de consulta debe abrir una transacción explícitamente
`READ ONLY`. La cuenta con permisos mínimos sigue siendo la frontera principal;
una transacción read-only no impide todos los efectos sobre disco ni convierte
cualquier función en inocua.
[Semántica de READ ONLY](https://www.postgresql.org/docs/current/sql-set-transaction.html).

Implementar además:

- Política PostgreSQL propia: una única sentencia admitida; rechazar escrituras,
  DDL, `CALL`, `DO`, `COPY`, cambios de sesión y CTE que modifiquen datos.
- No confiar solo en un prefijo `SELECT` ni en buscar palabras prohibidas.
  Elegir y probar un análisis de sintaxis adecuado; rechazar lo que no pueda
  clasificarse con seguridad, incluidos comentarios, strings y dollar quoting.
- Restringir funciones ejecutables a un conjunto revisado. Auditar funciones
  de usuario/SECURITY DEFINER y extensiones; un SELECT puede invocar código.
- Bloquear en esta primera versión consultas de usuario que cambien configuración,
  adquieran locks explícitos o usen `EXPLAIN ANALYZE`. Las sentencias internas
  fijas para configurar límites no se exponen como SQL arbitrario del usuario.
- Usar `statement_timeout` y `lock_timeout`, además del plazo local de la operación.
  Cancelar la consulta en el servidor cuando vence el plazo; no basta con dejar
  de esperar su resultado en Rust.
- Gestionar rollback/cierre y el token de cancelación del driver. Si la cancelación
  no puede confirmarse, informarlo y no devolver la conexión a un pool como limpia.
  [CancelToken](https://docs.rs/tokio-postgres/latest/tokio_postgres/struct.CancelToken.html).
- Leer filas mediante streaming y detenerse con límites de filas/bytes/caracteres.
  No hacer `collect` sin límites. Un LIMIT no sustituye un timeout ni un presupuesto
  de memoria para una celda grande.
- Redactar secretos antes de mostrar, registrar o guardar resultados; sanitizar
  errores sin filtrar DSN, credenciales ni consultas sensibles.

Empezar por `status`, `search` y `describe` con SQL interno fijo. Habilitar
`query` arbitraria solo después de validar sus controles PostgreSQL.

## 6. Metadatos y contexto que necesita el LLM

Usar `pg_catalog` e `information_schema`, no las vistas `sys.*` de SQL Server.
La descripción debe devolver, dentro de límites explícitos:

- Esquema y nombre exactos; tipo de objeto y visibilidad para el usuario conectado.
- Columnas, tipos, nulabilidad, defaults, claves primarias/foráneas e índices.
- Comentarios de tablas y columnas; definición de vistas cuando esté permitida.
- Firmas y definiciones de funciones/procedimientos accesibles. Distinguir
  sobrecargas por esquema y argumentos de identidad, no solo por nombre.
- Relaciones entre objetos y omisiones por permisos, filtros o límites.

No interpretar ausencia por permisos como inexistencia. Conservar datos grandes
en `harness_detail` y permitir proyecciones mediante `harness_data_query`; no
reemplazar silenciosamente todo el contexto por un resumen.

Definir conversiones sin pérdida: NUMERIC como texto decimal cuando corresponda,
enteros fuera del rango seguro de JavaScript como strings identificados,
UUID/JSON/JSONB/arrays y NULL con tipos claros. Distinguir `timestamp` sin zona
de `timestamptz`; no inventar una zona para el primero. Acotar BYTEA y reportar
tipos no soportados en lugar de convertirlos silenciosamente a NULL.

## 7. Archivos y componentes a modificar

| Componente | Trabajo |
| --- | --- |
| `crates/domain/src/sql.rs` | Selector de motor, parámetros, tipos y política PostgreSQL |
| `crates/application/src/ports.rs` y `lib.rs` | Mantener admisión y sesiones; despachar explícitamente por motor |
| `src/adapters/sql.rs` | Conservar comportamiento de SQL Server y separar configuración/conectividad |
| Nuevo adaptador PostgreSQL | Driver, TLS, catálogos, transacciones, cancelación y conversiones |
| `src/transport.rs` y `contracts/tool-contract.json` | Publicar el contrato ampliado de `harness_sql` y batch |
| Doctor y bootstrap | Estado configurado por motor; conexión real solo cuando se solicite; no exponer secretos |
| Pruebas y documentación | Compatibilidad SQL Server, PostgreSQL, perfiles y MCP real |

La configuración debe cargarse con errores claros y límites válidos, sin ampliar
scopes externos ni permitir rutas arbitrarias para certificados o secretos.
No copiar automáticamente todos los comportamientos del adaptador actual:
revisar especialmente el tratamiento de errores y credenciales.

## 8. Criterios para darlo por terminado

1. Las llamadas actuales de SQL Server siguen funcionando sin cambios obligatorios.
2. `status` distingue configurado, conectado y fallido; `connect=false` no abre red.
3. Se prueba autenticación, TLS válido/inválido, permisos mínimos y schemas permitidos.
4. Se verifican metadatos, Unicode, sobrecargas y conversiones numéricas/temporales.
5. Se rechazan DML, DDL, múltiples sentencias, CTE de escritura y funciones no admitidas.
6. Un timeout cancela o informa correctamente el estado pendiente en el servidor.
7. Se prueban límites, truncamiento, redacción y recuperación por detalle/resultRef.
8. MCP y batch respetan el motor, perfiles y sesiones sin crear herramientas redundantes.
9. Las pruebas con PostgreSQL real usan una base de prueba y credenciales dedicadas,
   nunca una base de producción por defecto. Registrar las versiones efectivamente probadas.
10. Se compila un `.exe` nuevo después de implementar; el binario actual no gana
    soporte PostgreSQL solo por añadir la configuración de este documento.

## Instrucción breve para el agente que lo implemente

> Añade PostgreSQL a la herramienta unificada harness_sql según este documento.
> Conserva SQL Server y los controles existentes. Empieza por status/search/describe,
> separa el dialecto y no habilites query hasta validar solo lectura, TLS,
> cancelación y límites. No solicites ni escribas secretos en el chat. Si faltan
> datos de conexión, completa primero el código y las pruebas locales posibles;
> informa qué verificación real queda pendiente. No declares soporte operativo
> sin haber probado la conexión y las acciones implementadas.
