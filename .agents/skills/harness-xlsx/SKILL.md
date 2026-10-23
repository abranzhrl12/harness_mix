---
name: harness-xlsx
description: Inspeccionar o leer archivos XLSX existentes mediante Harness MCP por hoja, rango o página. No para crear libros ni controlar Excel abierto.
---

# Lectura XLSX acotada

Bootstrap si falta sesión y usa el esquema anunciado de `harness_xlsx`. No cargues todo el libro para descubrir una hoja.

- Si desconoces las hojas, llama con `path` sin `sheet` para metadatos.
- Si conoces hoja y rango, ve directamente a ellos: `path`, `sheet`, `range`. Para tablas extensas usa páginas acotadas con `rowOffset` y `pageSize` según la respuesta.
- Solicita fórmulas mediante `formulas` solo cuando sean necesarias. No confundas fórmulas, valores almacenados y recálculo: no afirmes haber recalculado con una herramienta de lectura.
- Conserva hash y cobertura. Usa `expectedHash` cuando requieras coherencia entre páginas y esté disponible; ante cambio del archivo, reinicia la parte afectada, no mezcles versiones.
- Proyecta resultados voluminosos con `harness_data_query` cuando lo permita la salida. No conviertas omisiones o celdas ausentes en ceros sin evidencia.
- Respeta scopes; un rechazo no autoriza leer por otra vía. La herramienta no modifica libros: si se pide creación o edición, selecciona una capacidad apropiada y autorizada.

Ejemplo, sustituyendo valores reales:

```json
{"path":"datos.xlsx","sheet":"Ventas","range":"A1:F30","session":"SESION"}
```

Informa hoja/rango, resultados y límites de interpretación relevantes.
