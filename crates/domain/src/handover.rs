//! Structured handover report rendering. Pure domain logic, no filesystem.
use crate::contracts::HandoverKind;
use serde_json::Value;

/// Escape Markdown inline characters to prevent broken syntax or injection.
pub fn inline_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut prev_whitespace = false;
    for ch in value.chars() {
        if ch.is_whitespace() {
            if !prev_whitespace {
                out.push(' ');
                prev_whitespace = true;
            }
            continue;
        }
        prev_whitespace = false;
        if matches!(ch, '\\' | '`' | '*' | '_' | '{' | '}' | '[' | ']' | '(' | ')' | '#' | '+' | '.' | '!' | '|' | '<' | '>') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

pub fn render_handover(snapshot: &Value, kind: HandoverKind) -> String {
    let empty_obj = serde_json::json!({});
    let empty_vec = vec![];

    let session_id = snapshot["sessionId"].as_str().unwrap_or("unknown");
    let declared = snapshot.get("declaredByAgent").unwrap_or(&empty_obj);
    let observed = snapshot.get("observedByHarness").unwrap_or(&empty_obj);
    let tasks = snapshot.get("tasks").unwrap_or(&empty_obj);
    let stats = snapshot.get("stats").unwrap_or(&empty_obj);

    let title = match kind {
        HandoverKind::Adr => "ADR — borrador",
        HandoverKind::PrBody => "Resumen para PR",
        HandoverKind::Summary => "Resumen de sesión",
    };

    let mut rows: Vec<String> = vec![
        format!("# {title}"),
        String::new(),
        format!("Sesión: {}", inline_escape(session_id)),
        String::new(),
        "> Alcance: solo operaciones registradas por este proceso MCP, con un máximo retenido de 50 archivos y 20 comprobaciones. Las declaraciones del agente no se certifican como hechos. La vigencia del código no se comprobó para este export.".to_string(),
        String::new(),
        "## Objetivo declarado".to_string(),
        String::new(),
        inline_escape(declared["goal"].as_str().unwrap_or("No registrado")),
        String::new(),
        "## Decisiones declaradas".to_string(),
        String::new(),
    ];

    let decisions = declared["decisions"].as_array().unwrap_or(&empty_vec);
    if decisions.is_empty() {
        rows.push("- No registradas".to_string());
    } else {
        for d in decisions {
            if let Some(s) = d.as_str() {
                rows.push(format!("- {}", inline_escape(s)));
            }
        }
    }

    if kind == HandoverKind::Adr {
        rows.push(String::new());
        rows.push("## Alternativas y consecuencias".to_string());
        rows.push(String::new());
        rows.push("No registradas; requieren revisión humana o información explícita.".to_string());
    }

    rows.push(String::new());
    rows.push("## Archivos con ediciones observadas".to_string());
    rows.push(String::new());

    let modified_files = observed["modifiedFiles"].as_array().unwrap_or(&empty_vec);
    if modified_files.is_empty() {
        rows.push("- Ninguno registrado".to_string());
    } else {
        for f in modified_files {
            let path = f["path"].as_str().unwrap_or("");
            let hash = f["hash"].as_str().unwrap_or("");
            let syntax_status = f["syntax"]["status"].as_str().unwrap_or("no registrada");
            rows.push(format!(
                "- {} — hash observado: {}; sintaxis: {}",
                inline_escape(path),
                hash,
                inline_escape(syntax_status)
            ));
        }
    }

    if let Some(omitted) = observed["omittedFiles"].as_u64().filter(|&n| n > 0) {
        rows.push(format!("- {omitted} archivos adicionales fuera del resumen"));
    }

    rows.push(String::new());
    rows.push("## Comprobaciones ejecutadas".to_string());
    rows.push(String::new());

    let checks = observed["checks"].as_array().unwrap_or(&empty_vec);
    if checks.is_empty() {
        rows.push("- Ninguna registrada; no se afirma que los tests hayan pasado".to_string());
    } else {
        for c in checks {
            let lang = c["language"].as_str().unwrap_or("");
            let scope = c["scope"].as_str().unwrap_or("");
            let outcome = if c["ok"] == true { "éxito" } else { "fallo" };
            let time = c["time"].as_str().unwrap_or("");
            rows.push(format!(
                "- {} / {}: {} ({}); vigencia no comprobada",
                inline_escape(lang),
                inline_escape(scope),
                outcome,
                inline_escape(time)
            ));
        }
    }

    if let Some(omitted) = observed["omittedChecks"].as_u64().filter(|&n| n > 0) {
        rows.push(format!("- {omitted} comprobaciones adicionales fuera del resumen"));
    }

    rows.push(String::new());
    rows.push("## Pendientes declarados".to_string());
    rows.push(String::new());

    let pending = declared["pending"].as_array().unwrap_or(&empty_vec);
    if pending.is_empty() {
        rows.push("- No registrados; esto no certifica que el trabajo esté terminado".to_string());
    } else {
        for p in pending {
            if let Some(s) = p.as_str() {
                rows.push(format!("- {}", inline_escape(s)));
            }
        }
    }

    rows.push(String::new());
    rows.push("## Fases declaradas".to_string());
    rows.push(String::new());

    let phases = tasks["phases"].as_array().unwrap_or(&empty_vec);
    for phase in phases {
        let title = phase["title"].as_str().unwrap_or("");
        let status = phase["status"].as_str().unwrap_or("");
        rows.push(format!("- {}: {}", inline_escape(title), status));
    }

    rows.push(String::new());
    rows.push("## Métricas observadas".to_string());
    rows.push(String::new());

    let ops = stats["operations"].as_u64().unwrap_or(0);
    let edits = stats["editsApplied"].as_u64().unwrap_or(0);
    let syntax_rej = stats["syntaxRejections"].as_u64().unwrap_or(0);
    rows.push(format!(
        "Operaciones: {ops}; ediciones aplicadas: {edits}; rechazos sintácticos: {syntax_rej}."
    ));
    rows.push(String::new());

    rows.join("\n")
}
