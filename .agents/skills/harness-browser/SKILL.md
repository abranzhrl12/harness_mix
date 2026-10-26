---
name: harness-browser
description: Diagnosticar y verificar aplicaciones web con Harness MCP mediante consola, red, DOM y capturas. Usar ante fallos visuales, de interacción o hidratación; no para navegación general.
---

# Diagnóstico web con evidencia

Descubre el esquema actual de `harness_browser`; reutiliza una sesión Harness
con raíz comprobada. Una herramienta disponible no significa navegador conectado.
No edites hasta que la tarea autorice corregir código.

## Conectar y reproducir

1. Reutiliza `connectionId` y `targetId` vigentes de esta sesión si existen.
   En caso contrario, `connect` requiere el WebSocket CDP real del navegador:
   `ws://127.0.0.1:<puerto>/devtools/browser/<id>`. No sirve la URL de la app
   ni un endpoint `/devtools/page/`; solo se admiten endpoints loopback.
   Si falta, pide el endpoint o la preparación del navegador; no adivines IDs,
   abras depuración pública ni cambies el perfil habitual del usuario.
2. Usa `targets`, elige la pestaña por URL/título y llama `attach` antes de
   reproducir. No selecciones la primera pestaña sin comprobarla.
3. Consulta `errors` acotado a `targetId`; conserva `nextCursor`. Reproduce la
   ruta e interacción afectadas, incluida una recarga cuando el fallo sea de
   carga inicial o hidratación. Esta herramienta no anuncia navegación,
   recarga, clics ni ejecución arbitraria: pide la acción al usuario o utiliza
   otra capacidad disponible y autorizada, sin inventar operaciones.

La captura comienza al adjuntarse; no garantiza recuperar errores anteriores.
No concluyas ausencia de errores si no se reprodujo con la captura activa.

## Investigar sin inundar el contexto

- Consulta `errors` con `after` igual al cursor guardado para separar eventos
  nuevos de históricos. `limit` admite 1..256; pagina usando `nextCursor` si
  llenas el límite. `dropped` avisa de eventos perdidos, no de una captura completa.
- Distingue warnings de consola, excepciones y fallos de red. Agrupa duplicados
  al informar, conservando mensaje, ubicación, secuencia y pasos relevantes.
  No ignores warnings de hidratación porque la pantalla parezca correcta.
- Usa `inspect` sobre el selector relevante para obtener DOM, medidas y estilos;
  amplía solo si falta evidencia. Respeta `truncated`. Usa `screenshot` cuando
  la apariencia importe; una captura no demuestra ausencia de errores runtime.
- Relaciona el mensaje/stack con fuente exacta: símbolo conocido directamente;
  `explore` para seguir dependencias; búsqueda literal para localizar el mensaje
  o componente. Diferencia causa confirmada de hipótesis.
- Ante hidratación, compara el estado usado al renderizar servidor y cliente:
  valor inicial, opciones disponibles y datos dependientes del navegador.
  El DOM inspeccionado es el actual, no prueba por sí solo el HTML original SSR.
  No ocultes el warning ni desactives SSR como arreglo automático.

## Verificar y entregar

Después de una corrección autorizada, guarda un cursor, repite la reproducción
con captura activa y consulta eventos nuevos. Comprueba el comportamiento esperado
y DOM/captura cuando aplique, además de las pruebas de código pertinentes.
Un build correcto o HMR sin errores no sustituyen una recarga para hidratación.

Informa qué ruta/acción verificaste, si reapareció el error original, otros fallos
relevantes y límites de captura o reproducción. No declares «sin errores» para
toda la aplicación a partir de una pestaña, intervalo o resultado limitado.
No copies credenciales o datos personales de DOM, URLs o logs al informe.
Desconecta solo conexiones creadas para esta tarea cuando ya no se necesiten;
no cierres el navegador del usuario. Ante transporte, timeout o runtime fallido,
detén reintentos equivalentes, diagnostica con `harness_doctor` y comunica el límite.
