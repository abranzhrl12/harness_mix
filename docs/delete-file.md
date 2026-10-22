# Eliminar archivos

`harness_delete` elimina permanentemente un archivo regular cuando el usuario lo
solicita. Disponible en Windows con perfiles `full` y `edit`.

```json
{"path":"src/obsolete.ts","expectedHash":"SHA256_DE_LA_LECTURA","session":"SESION"}
```

Solo `path` es obligatorio. `expectedHash` es opcional y comprueba SHA-256 sobre
los bytes del archivo, incluidos BOM y saltos de línea; admite hasta 64 MiB.
Sin hash no se lee el contenido y no se impone ese límite de tamaño.
En un workspace con varios proyectos se admite `proyecto:/src/obsolete.ts`.

La operación fija los directorios y elimina mediante el handle del archivo para
evitar sustituciones concurrentes. Rechaza directorios, enlaces, hard links,
rutas externas, archivos de solo lectura y las rutas protegidas del arnés,
instrucciones, índice, credenciales y control de versiones. No admite glob,
borrado recursivo ni ejecución en `harness_batch`.

El resultado incluye `changed`, `commitStatus`, `verified`, `permanent` e
`index.status`. Si `changed=true`, el borrado ya fue solicitado al sistema:
no repetirlo para resolver un fallo de índice. `DELETE_INDEX` requiere
actualizar el índice; `DELETE_POSTCHECK` requiere inspeccionar la ruta (otro
lector puede mantener pendiente el borrado). No crea un índice si no existe.

El borrado no pasa por la papelera ni crea una copia recuperable. Los checkpoints
registran el evento `file_deleted` e invalidan verificaciones anteriores;
no restauran el archivo. Los imports de otros archivos no se modifican.
