---
name: harness-svg
description: Inspeccionar o previsualizar SVG estático existente con Harness MCP. No para generar imágenes raster ni editar código SVG.
---

# Inspección y vista previa SVG

Bootstrap si falta sesión y consulta el esquema de `harness_svg`. El servidor anunciado admite SVG estático de hasta 256 KiB; no supongas ejecución de scripts ni recursos externos.

- Usa `mode=inspect` para estructura, dimensiones y diagnóstico textual.
- Usa `mode=preview` directamente si necesitas juzgar apariencia. No hagas ambas llamadas por rutina.
- Proporciona `path` y sesión; `externalScope` solo si ya existe autorización válida. No amplíes permisos por un rechazo.
- Una inspección estructural no demuestra calidad visual. Examina la imagen devuelta antes de afirmar resultados visuales; si no existe preview, informa esa limitación.
- Para editar el XML, usa la capacidad de edición autorizada con fuente vigente y verifica después. Esta herramienta por sí sola no escribe archivos.
- Ante formato, tamaño o renderizado no soportado, informa el límite. Ante fallo de runtime/transporte, diagnostica sin reintentos equivalentes ni bypass.

Termina con los hallazgos relevantes; evita descargar el XML completo si estructura o preview ya bastan.
