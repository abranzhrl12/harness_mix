# SQL Server y PostgreSQL

[Inicio](../README.md) · [Comandos](REFERENCE.md) · [Rendimiento](PERFORMANCE.md)

## Conexión básica

La instalación no crea credenciales ni modifica proyectos. Define una variable en el proceso del servidor o en el `.env` del proyecto (nunca subas su valor a Git):

~~~dotenv
HARNESS_SQL_CONNECTION_STRING=postgresql://USUARIO:CLAVE@HOST:5432/BASE
~~~

Alternativa SQL Server:

~~~dotenv
HARNESS_SQL_CONNECTION_STRING="Server=HOST;Database=BASE;User ID=USUARIO;Password=CLAVE"
~~~

Se detecta el motor por el formato. También se reconocen `HARNESS_PG_CONNECTION_STRING` y `DATABASE_URL`. Para una misma variable, el entorno del proceso tiene prioridad sobre `.env`; no se ejecuta ese archivo como script ni se expanden variables.

~~~json
{"action":"status","engine":"postgresql","connect":false}
~~~

Envía estos argumentos a `harness_sql`. `connect:false` solo inspecciona configuración: no mide acceso real a la base. Para comprobar una conexión usa `connect:true`. Si ambos motores están habilitados, proporciona `engine` explícitamente.

## Operaciones

| Acción | SQL Server | PostgreSQL |
| --- | --- | --- |
| `status` | Configuración y conexión opcional | Configuración y conexión opcional |
| `search` | Catálogo visible | Catálogo visible dentro de esquemas permitidos |
| `map` | Relaciones y columnas | Mapa de catálogo soportado por su backend |
| `describe` | Relaciones y rutinas visibles | Relaciones/esquemas dentro del alcance admitido; no definición de rutinas |
| `query` | Consulta de solo lectura dentro de límites | No habilitada: `PG_QUERY_UNAVAILABLE` |

~~~json
{"action":"describe","engine":"sqlserver","name":"dbo.Customer","kind":"relation"}
~~~

~~~json
{"action":"search","engine":"postgresql","query":"customer"}
~~~

En SQL Server, un nombre sin esquema se resuelve entre objetos visibles del tipo solicitado; la ambigüedad devuelve candidatos. `describe` de rutinas consulta metadatos y parámetros, no ejecuta la rutina. Una definición nula no identifica por sí sola su causa.

## JSON opcional, TLS y límites

`agent-harness --root "C:/proyectos/mi-app" --init-config` crea un esqueleto sin sobrescribir. La configuración explícita prevalece sobre autodetección, incluido `enabled:false`.

Ejemplo PostgreSQL con validación TLS; el secreto sigue en el entorno:

~~~json
{
  "postgresql": {
    "enabled": true,
    "defaultSchema": "public",
    "allowedSchemas": ["public"],
    "connection": {"source": "environment", "name": "HARNESS_PG_CONNECTION_STRING"},
    "tls": {"mode": "verify-full"},
    "schemaCacheSeconds": 0
  }
}
~~~

El modo TLS predeterminado del proyecto es `disable`: no cifra el tráfico. `verify-full` verifica certificado y hostname; `disable-loopback` restringe plaintext a loopback. Para CA privada configura el certificado admitido por el schema de conexión.

PostgreSQL reutiliza hasta cuatro conexiones y sentencias de catálogo preparadas. Errores, cancelación o timeout descartan la conexión cuando no se puede confirmar su estado. Las identidades incluyen configuración y credenciales; no se registran secretos.

`schemaCacheSeconds` (0..300; predeterminado 0) permite cachear snapshots completos de `describe`. `refresh:true` pide renovarlos. Una respuesta cacheada informa edad/TTL y no revalida DDL o permisos durante ese intervalo.

Usa un rol lector con permisos mínimos. Las restricciones léxicas y de transacción no sustituyen los permisos del motor. Los resultados se acotan por filas, tamaño y deadline; consulta truncamientos y diagnósticos sanitizados. No se habilitan escrituras ni consultas PostgreSQL arbitrarias para acelerar una medición.
