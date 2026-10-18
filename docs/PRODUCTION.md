# Salida a producción

Objetivo inicial: un binario Windows x64, conectado por MCP a un proyecto local.
El resultado de cada ejecución y los pendientes se registran en [CHECK.md](../CHECK.md).

## Puertas de salida

- [x] `cargo test --locked --workspace -- --test-threads=1` termina sin fallos (381/0/23; evidencia en CHECK).
- [x] `cargo build --release --locked` produce el binario que se distribuirá.
- [x] Ese binario inicia MCP, enumera el perfil esperado y ejecuta lectura, búsqueda y doctor.
- [x] Una tarea permite consultar su resultado, cancelarse y recuperar su historial tras reiniciar.
- [ ] El cliente real descubre las herramientas y completa una operación en un proyecto de prueba.
- [x] El paquete incluye instrucciones, licencias y SHA256; se conservan los binarios existentes sin reemplazar instalaciones.

El ZIP y su checksum están en `target/dist`. La sonda ampliada también pasó sobre
el ejecutable extraído del ZIP. Los logs y el SHA256 exacto constan en CHECK;
la casilla del cliente real permanece pendiente hasta probar esa conexión.

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
mínimo mayor. Además se instaló Rust 1.88.0 y se ejecutó
`cargo +1.88.0 test --locked --workspace --no-fail-fast --jobs 4 --target-dir
target/msrv-1.88 -- --test-threads=1`: 383 aprobadas, 0 fallidas y 23 omitidas,
código de salida 0. Evidencia: `target/audit/msrv-1.88-tests.log`.
Esto verifica el MSRV en Windows x64; no extiende esa comprobación a Linux/macOS.
El ZIP distribuible se compiló con Rust 1.99.0, antes de los cambios SQL posteriores.

Para el primer despliegue, usar el compilador realmente probado y conservar su
versión junto al binario, el commit y el resultado de las pruebas. Validar primero
en una copia pequeña del proyecto; después ampliar a los proyectos habituales.
