# Salida a producción

Objetivo inicial: un binario Windows x64, conectado por MCP a un proyecto local.
El resultado de cada ejecución y los pendientes se registran en [CHECK.md](../CHECK.md).

## Puertas de salida

- [ ] `cargo test --locked --workspace -- --test-threads=1` termina sin fallos.
- [ ] `cargo build --release --locked` produce el binario que se distribuirá.
- [ ] Ese binario inicia MCP, enumera el perfil esperado y ejecuta lectura, búsqueda y doctor.
- [ ] Una tarea permite consultar su resultado, cancelarse y recuperar su historial tras reiniciar.
- [ ] El cliente real descubre las herramientas y completa una operación en un proyecto de prueba.
- [ ] El paquete incluye instrucciones, licencia y SHA256; se conserva el binario anterior para volver atrás.

Una casilla pendiente no significa una prueba aprobada. No se debe certificar un
binario release usando únicamente pruebas de un binario de desarrollo.

## Alcance de la primera entrega

SQL Server requiere configuración y validación contra un servidor real antes de
usarlo. La paridad CodeGraph/Node, Linux/macOS y las pruebas con proyectos externos
son ampliaciones independientes del núcleo Windows. Si se necesitan para un
despliegue concreto, pasan a ser requisitos de ese despliegue.

Los avisos MCP no garantizan que el cliente despierte al modelo ni que acepte
mensajes durante un turno activo. Los trabajos persisten su seguimiento; no se
reanuda automáticamente un proceso interrumpido.

## Compilador y dependencias

Los tres paquetes declaran Rust 1.88. La consulta local de metadatos del lockfile,
con `cargo metadata --offline --locked --format-version 1 --filter-platform
x86_64-pc-windows-msvc`, no encuentra dependencias seleccionadas que declaren un
mínimo mayor. Esto comprueba declaraciones, no certifica una compilación con 1.88:
ese toolchain no está instalado en el host de esta auditoría. Mantener la
certificación MSRV pendiente hasta ejecutar la compilación y las pruebas con él.

Para el primer despliegue, usar el compilador realmente probado y conservar su
versión junto al binario, el commit y el resultado de las pruebas. Validar primero
en una copia pequeña del proyecto; después ampliar a los proyectos habituales.
