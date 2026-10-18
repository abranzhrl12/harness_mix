# Conexión sencilla a bases de datos

La instalación global de npm no modifica proyectos. Con esta implementación,
el JSON tampoco es necesario para el flujo básico: basta la cadena de conexión.
Estas capacidades requieren publicar e instalar el ejecutable actualizado.

1. Instala el paquete y configura tu cliente MCP para apuntar al proyecto.
2. En el `.env` de ese proyecto coloca una de estas alternativas:

```dotenv
# PostgreSQL
HARNESS_SQL_CONNECTION_STRING=postgresql://USUARIO:CLAVE@HOST:5432/BASE
```

```dotenv
# SQL Server, como alternativa a la anterior
HARNESS_SQL_CONNECTION_STRING="Server=HOST;Database=BASE;User ID=USUARIO;Password=CLAVE"
```

Harness detecta el motor por el formato. También reconoce
HARNESS_PG_CONNECTION_STRING y DATABASE_URL. Nunca subas credenciales a Git.
Los valores del proceso tienen prioridad sobre el `.env` para la misma variable.
No se ejecuta el archivo como script ni se expanden variables `${...}`.

## Configuración avanzada opcional

Desde el proyecto ejecuta:

```powershell
agent-harness --init-config
```

Genera `.agents/harness.config.json`, sin contraseñas y sin sobrescribir archivos.
Si ya hay configuración explícita de motores, esta tiene prioridad sobre la
detección: incluso enabled:false se respeta. Si hay dos motores habilitados,
indica engine en cada llamada. El comando no crea ni sincroniza índices.

El JSON sirve para opciones como límites, esquema, caché y TLS; las credenciales
siguen en variables. `connection.envFile` selecciona el archivo de respaldo
(por defecto `.env`; null desactiva el respaldo). PostgreSQL tiene TLS desactivado
por defecto por decisión explícita del usuario; el estado muestra una advertencia.
Para exigir cifrado y validación del certificado, usa tls.mode=verify-full.

Estado sin conectar: harness_sql action=status. Verificación real: connect=true.
PostgreSQL todavía ofrece catálogo/introspección, no consultas arbitrarias.
