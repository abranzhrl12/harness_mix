//! Native handover report adapter.
//! Pure rendering in domain, safe atomic markdown file publication or bounded preview.

use crate::adapters::project::Project;
use harness_domain::{
    Failure,
    contracts::{HandoverArgs, HandoverKind},
    execution::OperationContext,
    handover::render_handover,
    hash,
};
use serde_json::{Value, json};
use std::{fs, io::Write};

pub fn handle_handover(
    project: &Project,
    args: &HandoverArgs,
    snapshot: &Value,
    context: &OperationContext,
) -> Result<Value, Failure> {
    let kind = args.kind.unwrap_or(HandoverKind::Summary);
    let markdown = render_handover(snapshot, kind);
    let characters = markdown.chars().count();
    let kind_str = match kind {
        HandoverKind::Summary => "summary",
        HandoverKind::Adr => "adr",
        HandoverKind::PrBody => "pr_body",
    };

    let Some(target_file) = &args.target_file else {
        let preview: String = markdown.chars().take(1600).collect();
        let truncated = characters > 1600;
        return Ok(json!({
            "kind": kind_str,
            "written": false,
            "characters": characters,
            "preview": preview,
            "truncated": truncated,
            "_detail": {
                "markdown": markdown
            }
        }));
    };

    if !target_file.to_ascii_lowercase().ends_with(".md") {
        return Err(("TARGET", "el destino debe terminar en .md"));
    }

    context.check()?;

    let target_path = project.target_optional(target_file)?;
    let parent = target_path
        .parent()
        .ok_or(("TARGET", "el directorio destino debe existir"))?;
    if !parent.is_dir() {
        return Err(("TARGET", "el directorio destino debe existir"));
    }

    if target_path.exists() {
        let expected = args.expected_hash.as_deref().ok_or((
            "EXISTS",
            "el archivo existe; indicar expectedHash vigente para reemplazarlo",
        ))?;
        let current_bytes =
            fs::read(&target_path).map_err(|_| ("IO", "No se pudo leer archivo existente"))?;
        let current_hash = hash(&current_bytes);
        if expected != current_hash {
            return Err((
                "STALE",
                "El archivo destino cambió; releer antes de continuar",
            ));
        }
    } else if args.expected_hash.is_some() {
        return Err(("STALE", "se esperaba un archivo existente"));
    }

    context.check()?;

    // Atomic write via tempfile in the same parent directory
    let temp_name = format!(".harness-report-{}", uuid::Uuid::new_v4());
    let temp_path = parent.join(&temp_name);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .map_err(|_| ("IO", "No se pudo crear archivo temporal del reporte"))?;

    file.write_all(markdown.as_bytes()).map_err(|_| {
        let _ = fs::remove_file(&temp_path);
        ("IO", "No se pudo escribir archivo temporal del reporte")
    })?;

    file.sync_all().map_err(|_| {
        let _ = fs::remove_file(&temp_path);
        ("IO", "No se pudo sincronizar archivo temporal del reporte")
    })?;
    drop(file);

    context.check()?;

    // Rename/replace tempfile to target_path
    if fs::rename(&temp_path, &target_path).is_err() {
        // If rename fails (e.g. atomic overwrite on some platforms without ReplaceFile), try remove + rename or cleanup
        let _ = fs::remove_file(&temp_path);
        return Err(("IO", "No se pudo publicar el reporte"));
    }

    let report_hash = hash(markdown.as_bytes());

    Ok(json!({
        "kind": kind_str,
        "written": true,
        "path": target_file,
        "hash": report_hash,
        "characters": characters,
        "_detail": {
            "markdown": markdown
        }
    }))
}
