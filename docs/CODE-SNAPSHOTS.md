# Imágenes de código en documentos

[Inicio](../README.md) · [Arquitectura](ARCHITECTURE.md)

Harness integra la biblioteca [CodeSnap](https://github.com/codesnap-rs/codesnap) en el generador de documentos Word. Usa un bloque `code_image` para que Harness renderice el fragmento como PNG y lo inserte en el DOCX:

~~~json
{
  "path": "docs/code-example.docx",
  "document": {
    "title": "Ejemplo de código",
    "blocks": [
      {
        "type": "code_image",
        "code": "fn main() {\n    println!(\"Hola\");\n}",
        "language": "rust",
        "alt": "Función main de ejemplo en Rust"
      }
    ]
  }
}
~~~

El campo `language` es opcional y selecciona la sintaxis para el resaltado. `width_px` opcional limita el ancho de la imagen dentro de la página. Aunque el modelo acepta `alt`, la versión actual de `docx-rs` no permite escribir texto alternativo en la imagen; añade un bloque `paragraph` cercano para describir su contenido. Agrega además un bloque `code` si el lector necesita copiar el fragmento.

La operación se hace localmente a través de la biblioteca Rust de CodeSnap; no requiere instalar su CLI ni iniciar un navegador. Para guardar capturas independientes en archivos de documentación Markdown, se puede usar el CLI opcional:

~~~powershell
cargo install codesnap-cli
$outputDir = ".\docs\assets\code-snapshots"
New-Item -ItemType Directory -Force -Path $outputDir
codesnap -f .\src\main.rs -o "$outputDir\main.png"
~~~

Consulta las opciones de la versión instalada con `codesnap --help`. Mantén los recursos bajo `docs/assets/code-snapshots/`, usa texto alternativo y conserva el fragmento como texto seleccionable cuando corresponda.

## Selección

| Opción | Uso | Coste de integración |
| --- | --- | --- |
| [CodeSnap](https://github.com/codesnap-rs/codesnap) | CLI o biblioteca Rust; temas, números de línea, fondos y formatos PNG, SVG y HTML | La biblioteca permite renderizar e insertar imágenes sin invocar otro proceso |
| [Silicon](https://github.com/Aloxaf/silicon) | CLI Rust para crear imágenes PNG con resaltado de sintaxis | Requiere instalar un ejecutable por separado; su guía documenta dependencias nativas que varían por sistema |

CodeSnap es la opción integrada porque ofrece una biblioteca Rust que devuelve la imagen en memoria y se puede conectar directamente al renderizador DOCX. Silicon sigue siendo una buena alternativa para flujos editoriales locales que ya lo tengan instalado; su interfaz CLI necesitaría un proceso externo para integrarse en la generación de documentos.

## Fuentes del proyecto

- [CodeSnap: repositorio, CLI y API](https://github.com/codesnap-rs/codesnap)
- [CodeSnap: documentación de la biblioteca](https://docs.rs/codesnap/latest/codesnap/)
- [Silicon: repositorio, CLI e instalación](https://github.com/Aloxaf/silicon)
