# Preparar propuestas sin publicar

`harness_proposal` es una operación opcional nueva. En esta primera etapa solo
admite `action: "prepare"` para un archivo UTF-8 existente. No cambia el
comportamiento de `harness_replace`, `harness_write`, `harness_move` ni
`harness_delete`.

```json
{
  "action": "prepare",
  "path": "src/example.ts",
  "content": "export const value = 2;\n",
  "expectedHash": "SHA256_DE_LA_LECTURA",
  "session": "SESION"
}
```

`content` contiene el archivo propuesto completo. `expectedHash` y `session` son
opcionales. Un hash distinto impide guardar la propuesta. Se reutilizan las
reglas actuales de BOM (`bom` opcional), validación sintáctica y
`allowUnvalidatedText`. Original y candidato tienen un límite de 2 MiB cada uno;
el límite de mensajes MCP del cliente puede ser menor. No admite archivos nuevos,
directorios, binarios ni rutas protegidas. Disponible en Windows con perfiles
`full` y `edit`; no está disponible en `investigate` ni en `harness_batch`.

La respuesta contiene `proposalId`, `baseHash`, `candidateHash`, `patch`, `syntax`
y `storagePath`. `stored=true` indica que se guardó una propuesta pendiente;
`changed=false` y `published=false` indican que no se modificó el archivo.
`candidateChanged` compara el original con el candidato. Si ambos son iguales,
no se guarda nada y se devuelve `status="unchanged"` y `proposalId=null`.
El diff visible se limita a 12 000 caracteres y se indica con
`omitted.truncated=true`; `detail.ref` permite recuperar
el diff completo durante la sesión del servidor.

El original y el candidato exactos, incluidos BOM y saltos de línea, se guardan
en `.agents/harness/proposals/<proposalId>.json`, con formato versionado y hashes.
Cada preparación crea un ID independiente. El archivo se publica de forma
atómica sin sobrescribir otros registros; sus permisos de acceso se copian del
original antes de escribir el contenido. Los directorios se fijan contra
sustituciones por enlaces. El original se mantiene abierto sin permitir
escrituras o renombres mientras se prepara la propuesta.

Los registros sobreviven al reinicio del proceso. El almacenamiento se crea
solo al preparar una propuesta válida y tiene un `.gitignore` local; también se
excluye del índice nativo y del fingerprint de verificación del código. No se
crea ni actualiza el índice, ni se registran ediciones de código en la sesión.
Un fallo puede dejar directorios de metadatos vacíos, pero no publica el
candidato sobre el original.

Esta etapa todavía no implementa consultar por ID, aceptar, rechazar, aplicar,
restaurar ni limpiar propuestas. Tampoco proporciona botones del IDE. Los
checkpoints siguen siendo historial, no copias restaurables del workspace.
