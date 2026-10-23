---
name: harness-sql
description: Consultar bases SQL Server o PostgreSQL mediante Harness MCP, descubrir objetos y obtener resultados acotados. No para editar SQL como código ni para SQLite local no soportado.
---

# SQL con contexto mínimo suficiente

Bootstrap si falta sesión; confirma raíz y consulta el esquema real de `harness_sql`. La conexión actual es de solo lectura. No supongas motores, credenciales ni autorización de escritura.

| Necesidad | Acción |
| --- | --- |
| Estado o motores desconocidos | `status` |
| Objeto desconocido | `search` con nombre concreto |
| Estructura de objeto conocido | `describe`, `name` y secciones necesarias |
| Relaciones de un esquema | `map`, esquema y límite acotados |
| Datos | `query`, columnas y filtros concretos |

Evita status/search/describe si ya tienes conexión y esquema vigentes suficientes. Si hay dos motores, especifica `engine`. En PostgreSQL, omitir schema en búsqueda incluye los permitidos por política; no presupongas `public`.

- Usa parámetros donde la conexión los soporte; limita filas con la sintaxis del motor. No ejecutes rutinas con efectos laterales bajo la apariencia de SELECT.
- Reutiliza metadatos mientras sean válidos. La caché depende del backend/configuración: no prometas 0 ms ni pases `schemaCacheSeconds` si el esquema de la llamada no lo anuncia. Usa `refresh` únicamente cuando esté soportado y necesites invalidar metadatos.
- Agrupa descripciones independientes con batch si están admitidas. No descargues todo el catálogo por rutina.
- Para resultados grandes guardados, usa `harness_data_query` con proyecciones JMESPath; recupera detalles omitidos solo cuando afecten la respuesta.
- Distingue cero filas, permisos insuficientes, truncamiento y error. Ante fallos de conexión/runtime, diagnostica y comunica; no improvises otra conexión.

Concluye con resultado, filtros y límites relevantes; no vuelques datos sensibles innecesarios.
