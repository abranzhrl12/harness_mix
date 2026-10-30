use crate::ports::SessionContext;
use harness_domain::{contracts::BootstrapArgs, tool_contract::ToolProfile};
use serde_json::{Value, json};

pub(super) struct ResponseInput {
    pub(super) session: SessionContext,
    pub(super) args: BootstrapArgs,
    pub(super) same_capabilities: bool,
    pub(super) compact_reuse: bool,
    pub(super) available: Value,
    pub(super) contract: Value,
    pub(super) tool_profile: ToolProfile,
}

pub(super) fn build_response(mut result: Value, input: ResponseInput) -> Value {
    result["session"] = json!(input.session.id);
    // Keep the old response key for existing clients; new examples use one name both ways.
    result["sessionId"] = json!(input.session.id);
    // Keep examples out of session reuse; they only help the first discovery.
    if input.args.session.is_none() {
        result["exampleCall"] = json!({"tool":"harness_search","arguments":{"kind":"files","query":"Cargo","session":input.session.id},"note":"Read-only discovery without an index. Replace Cargo with a file name."});
        result["examples"] = initial_examples(&input.session.id, input.tool_profile);
        result["templates"] = initial_templates(&input.session.id, input.tool_profile);
        result["responseContract"] = json!({
            "canonical":"structuredContent",
            "fields":["state","scope","data","diagnostics","detail","omitted"],
            "note":"Read state before data; preserve diagnostics, freshness and omissions. content is a compatible text view and may have a different shape. detail references recover complete stored results."
        });
        result["batchExample"] = json!({"tool":"harness_batch","arguments":{"jobs":[{"tool":"harness_search","arguments":{"kind":"files","query":"Cargo","scope":"."}},{"tool":"harness_git","arguments":{"action":"status"}}],"maxConcurrent":4,"session":input.session.id},"aliases":{"args":"arguments"}});
    }
    result["limitations"] = json!([
        "Edición y navegación dependen del lenguaje y backend; consultar las capacidades declaradas de cada herramienta",
        "Explore: índice existente con selectores exactos; syncIndex es opt-in y no certifica cobertura global",
        "Búsqueda text/files/file_size y syntax de declaraciones Rust (kind[:nombre]); sin patrones AST generales. Scopes Windows solo lectura",
        "Refresh solo de cobertura local; métricas por instancia, no históricas ni específicas de sesión",
        "Verificación Rust/Go/Python/JavaScript/.NET parcial; .NET exige .csproj/.sln, SDK y restore previo. Ejecuta código del proyecto, no es sandbox",
        "Git solo lectura; requiere git instalado y raíz Git o worktree vinculado registrado",
        "No sustituye el servidor actual"
    ]);
    result["toolContract"] = {
        // Catalog-only view: internal hashes/feature fields stay server-side.
        let mut contract = input.contract;
        if let Some(obj) = contract.as_object_mut() {
            for key in ["contractHash", "features", "tools", "toolProfile"] {
                obj.remove(key);
            }
        }
        contract
    };
    if input.args.session.is_none() {
        result["tasks"] = input.session.task_summary;
    }
    if let Some(warning) = input.session.persistence_warning {
        result["persistenceWarning"] = warning;
    }
    if input.args.session.is_none() || !input.same_capabilities {
        result["routing"] = json!(input.tool_profile.routing());
        result["workflow"] = input.tool_profile.workflows();
        result["usageRules"] =
            harness_domain::tool_contract::manifest()["llmPolicy"]["rules"].clone();
    }
    if input.same_capabilities {
        result["capabilitiesUnchanged"] = json!(true);
    } else {
        result["available"] = input.available;
    }
    if input.compact_reuse {
        // The capabilities hash was checked above; retain a retrievable
        // catalog without resending the same tool lists and policy text.
        let catalog = json!({"toolContract":result["toolContract"],"limitations":result["limitations"],"routing":harness_domain::tool_contract::routing("rust"),"workflow":harness_domain::tool_contract::manifest()["llmPolicy"]["workflows"],"usageRules":harness_domain::tool_contract::manifest()["llmPolicy"]["rules"]});
        result["toolContract"] = json!({"backend":"rust","contractVersion":catalog["toolContract"]["contractVersion"],"unchanged":true});
        result["catalogUnchanged"] = json!(true);
        for key in ["limitations"] {
            result
                .as_object_mut()
                .expect("bootstrap object")
                .remove(key);
        }
        result["_detail"]["catalog"] = catalog;
    }
    if input.args.compact && !input.compact_reuse {
        let mut catalog = json!({});
        for key in [
            "toolContract",
            "routing",
            "workflow",
            "available",
            "exampleCall",
        ] {
            if let Some(value) = result
                .as_object_mut()
                .expect("bootstrap object")
                .remove(key)
            {
                catalog[key] = value;
            }
        }
        result["toolContract"] = json!({"backend":"rust","contractVersion":catalog["toolContract"]["contractVersion"],
            "unchanged":input.same_capabilities});
        result["catalogCompact"] = json!(true);
        result["_detail"]["catalog"] = catalog;
    }
    result
}

/// First-call examples use canonical parameter names and do not require an index.
fn initial_examples(session: &str, profile: ToolProfile) -> Value {
    let mut examples = json!([
        {"tool":"harness_read","arguments":{"path":"Cargo.toml","start":1,"end":80,"session":session}},
        {"tool":"harness_search","arguments":{"kind":"text","query":"fn main","scope":"src","contextLines":2,"session":session}},
        {"tool":"harness_symbols","arguments":{"action":"outline","path":"src/main.rs","session":session}},
        {"tool":"harness_symbols","arguments":{"action":"read","path":"src/main.rs","symbol":"main","session":session}},
        {"tool":"harness_word","arguments":{"path":"docs/generated.docx","document":{"title":"Generated document","blocks":[{"type":"paragraph","text":"Add content here."}]},"session":session},"note":"Creates a new workspace-relative .docx; existing destinations are not overwritten."},
        {"tool":"harness_verify","arguments":{"language":"dotnet","scope":"src/App/App.csproj","action":"build","engine":"auto","session":session},"note":"Runs project code; explicit authorization, SDK and prior restore required. Replace scope with an existing .csproj or .sln file."},
        {"tool":"harness_jobs","arguments":{"action":"list"}},
        {"tool":"harness_jobs","arguments":{"action":"start","dedupe":true,"verify":{"language":"rust","action":"test"}},"note":"Runs tests; requires authorization."}
    ]);
    examples.as_array_mut().expect("example array").retain(|example| {
        example["tool"].as_str().is_some_and(|tool| profile.allows(tool))
    });
    examples
}

/// Task-shaped call arguments. Replace only values marked with angle brackets;
/// templates are guidance for the caller, never executed by bootstrap.
fn initial_templates(session: &str, profile: ToolProfile) -> Value {
    let mut templates = json!([
        {"tool":"harness_search","task":"Find a literal in a relevant folder","fill":["query","scope"],"arguments":{"kind":"text","query":"<literal>","scope":"<folder-or-.>","contextLines":2,"session":session}},
        {"tool":"harness_read","task":"Read a small line range from a known file","fill":["path"],"arguments":{"path":"<workspace-relative-path>","start":1,"end":120,"session":session}},
        {"tool":"harness_symbols","task":"List declarations in a known file","fill":["path"],"arguments":{"action":"outline","path":"<workspace-relative-path>","session":session}},
        {"tool":"harness_symbols","task":"Read one exact symbol from a known file","fill":["path","symbol"],"arguments":{"action":"read","path":"<workspace-relative-path>","symbol":"<exact-symbol-name>","session":session}},
        {"tool":"harness_batch","task":"Run up to four independent reads together","fill":["jobs[].arguments"],"arguments":{"jobs":[{"tool":"harness_search","arguments":{"kind":"files","query":"<file-name>"}},{"tool":"harness_search","arguments":{"kind":"text","query":"<literal>","scope":"<folder-or-.>"}}],"maxConcurrent":2,"session":session}},
        {"tool":"harness_detail","task":"Continue or recover a stored result","fill":["ref"],"arguments":{"ref":"<detail.ref>","offset":0,"maxChars":16000,"session":session},"note":"Use detail.ref from the prior response; for a next page, use its detail.nextCall arguments."},
        {"tool":"harness_word","task":"Create a Word document in the workspace","fill":["path","document"],"arguments":{"path":"<workspace-relative-path.docx>","document":{"title":"<title>","blocks":[{"type":"heading","level":1,"text":"<heading>"},{"type":"paragraph","text":"<paragraph>"}]},"createParents":false,"session":session},"note":"Creates a new .docx file only; existing files are not overwritten."}
    ]);
    templates.as_array_mut().expect("template array").retain(|template| {
        template["tool"].as_str().is_some_and(|tool| profile.allows(tool))
    });
    templates
}

#[cfg(test)]
#[path = "bootstrap_tests.rs"]
mod tests;
