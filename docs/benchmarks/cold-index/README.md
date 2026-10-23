# Registro de optimización de indexación inicial

Cada paso tendrá su propio Markdown. Los resultados se separan de las hipótesis;
un paso sin medición terminada no se presenta como mejora.

1. [Protocolo y línea base](01-protocolo-linea-base.md).
2. [Preparación con uno y cuatro trabajadores](02-trabajadores.md).
3. [Resolución: prioridad y siguiente experimento](03-resolucion.md).
4. [Selector puro, verificación y desglose del resolvedor](04-selector-puro.md).
5. [Revalidación sin hashes redundantes](05-revalidacion-bytes.md).
6. [Diagnóstico por operación, llamadas, bytes y repetición](06-diagnostico-costes.md).
7. [Lectura con una sola validación de ruta](07-lectura-ruta-validada.md).
8. [Reutilización del análisis Rust por contenido](08-cache-analisis-rust.md).
9. [Filtrado por índice antes de consultar el disco](09-filtrado-indice-antes-disco.md).
10. [Pruebas de selección SQL de referencias pendientes](10-pruebas-seleccion-pendientes.md).
11. [Selector Rust validado en indexación real](11-selector-rust-real.md).
12. [Lectura protegida con pinning unificado: experimento pendiente](12-lectura-pinning-unificado.md).

Después de medir la línea base se priorizará la fase dominante: selección de
pendientes, invalidación, resolución, escritura o commit. Cada cambio posterior
necesita comparación A/B y equivalencia del grafo; no basta con compilar.

Los pasos 01 y 02 ya tienen mediciones release: 200 archivos, seis pasadas,
misma huella de grafo y cero fallos. Cuatro trabajadores mejoraron la mediana
solo un 0,7 %, sin evidencia robusta de mejora; resolución es la prioridad nueva.
El paso 04 ya aisló y probó el selector: el A/B no mostró aceleración. El coste
está en resolver_special (imports/pruebas de lenguaje), no en el selector puro.

El estado previo incluye un refactor de frescura sin commit y experimentos de
hashing locales. El ejecutor registra HEAD, diff rastreado, estado y SHA-256 del
binario para no atribuir esas condiciones a otro código.
