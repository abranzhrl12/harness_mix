use crate::ports::SessionContext;
use harness_domain::{contracts::BootstrapArgs, tool_contract::ToolProfile};
use serde_json::{Value, json};

pub(super) struct ResponseInput {
    pub(super) session: SessionContext,
    pub(super) args: BootstrapArgs,
    pub(super) rules: String,
    pub(super) rules_hash: String,
    pub(super) capabilities_hash: String,
    pub(super) unchanged: bool,
    pub(super) same_capabilities: bool,
    pub(super) compact_reuse: bool,
    pub(super) available: Value,
    pub(super) contract: Value,
    pub(super) logging: Value,
    pub(super) tool_profile: ToolProfile,
}

pub(super) fn build_response(mut result: Value, input: ResponseInput) -> Value {
    result["session"] = json!(input.session.id);
    // Keep the old response key for existing clients; new examples use one name both ways.
    result["sessionId"] = json!(input.session.id);
    result["sessionUsage"] = json!(
        "Copy data.session into the session argument of later calls. sessionId is a legacy response alias."
    );
    result["exampleCall"] = json!({"tool":"harness_explore","arguments":{"query":"main","session":input.session.id},"note":"Replace main with the exact symbol you want; other arguments have read-only context defaults."});
    result["batchExample"] = json!({"tool":"harness_batch","arguments":{"jobs":[{"tool":"harness_search","arguments":{"kind":"files","query":"Cargo","scope":"."}},{"tool":"harness_git","arguments":{"action":"status"}}],"maxConcurrent":4,"session":input.session.id},"aliases":{"args":"arguments"}});
    result["detailRetention"] = json!({"session":"References with a session remain available for this server process lifetime; restart does not restore them.","capacity":"Bounded store rejects new details rather than evict session references.","expiresAt":null});

    let available_tools = input.tool_profile.tools();
    result["quickGuide"] = quick_guide(&available_tools);
    result["rulesHash"] = json!(input.rules_hash);
    result["capabilitiesHash"] = json!(input.capabilities_hash);
    result["sessionRestored"] = json!(input.session.restored);
    result["implementation"] = json!("rust-experimental");
    result["toolContract"] = input.contract;
    result["backend"] = json!({"name":"rust","version":result.get("implementationVersion").cloned().unwrap_or(Value::Null)});
    result["logging"] = input.logging;
    result["supportedTools"] = json!(input.tool_profile.tools());
    result["toolProfile"] = json!(input.tool_profile.name());
    result["limitations"] = json!([
        "Edición y navegación dependen del lenguaje y backend; consultar las capacidades declaradas de cada herramienta",
        "Explore: índice existente con selectores exactos; syncIndex es opt-in y no certifica cobertura global",
        "Búsqueda text/files/file_size y syntax de declaraciones Rust (kind[:nombre]); sin patrones AST generales. Scopes Windows solo lectura",
        "Refresh solo de cobertura local; métricas por instancia, no históricas ni específicas de sesión",
        "Verificación Rust/Go/Python/JavaScript/.NET parcial; .NET exige .csproj/.sln, SDK y restore previo. Ejecuta código del proyecto, no es sandbox",
        "Git solo lectura; requiere git instalado y raíz Git o worktree vinculado registrado",
        "No sustituye el servidor actual"
    ]);
    result["executionPolicy"] = json!({"maxActive":crate::admission::MAX_ACTIVE,"maxSearches":crate::admission::MAX_SEARCHES,"maxBatchCoordinators":crate::admission::MAX_BATCHES,"maxChildrenPerBatch":4,"queue":"reject_when_full","timeoutMs":30000,"rustVerifyProcessTimeoutMs":60000,"verifyCleanupGraceMs":5000,"cancellation":"cooperative"});
    if input.args.session.is_none() {
        result["tasks"] = input.session.task_summary;
    }
    if let Some(warning) = input.session.persistence_warning {
        result["persistenceWarning"] = warning;
    }
    if input.args.session.is_none() || input.args.include_rules.unwrap_or(false) || !input.same_capabilities {
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
    if input.unchanged {
        result["rulesUnchanged"] = json!(true);
    } else {
        result["rules"] = json!(harness_domain::results::prefix(&input.rules, 12000));
        if harness_domain::results::utf16_len(&input.rules) > 12000 {
            result["truncated"] = json!(true);
            result["_detail"] = json!({"rules":input.rules});
        }
    }
    if input.compact_reuse {
        // The contract/rules hash was checked above; retain a retrievable
        // catalog without resending the same tool lists and policy text.
        let catalog = json!({"supportedTools":result["supportedTools"],"toolContract":result["toolContract"],"executionPolicy":result["executionPolicy"],"limitations":result["limitations"],"routing":harness_domain::tool_contract::routing("rust"),"workflow":harness_domain::tool_contract::manifest()["llmPolicy"]["workflows"],"usageRules":harness_domain::tool_contract::manifest()["llmPolicy"]["rules"]});
        let compact = json!({"backend":"rust","contractVersion":result["toolContract"]["contractVersion"],"contractHash":result["toolContract"]["contractHash"],"unchanged":true});
        result["toolContract"] = compact;
        result["catalogUnchanged"] = json!(true);
        for key in ["supportedTools", "executionPolicy", "limitations"] {
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
            "supportedTools",
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
            "contractHash":catalog["toolContract"]["contractHash"],"unchanged":input.same_capabilities});
        result["catalogCompact"] = json!(true);
        result["_detail"]["catalog"] = catalog;
    }
    result
}

fn quick_guide(available_tools: &[String]) -> Value {
    let quick_guide: Vec<Value> = [
        ("harness_bootstrap", "compact=true; reutiliza data.session"),
        ("harness_explore", "view=impact exige filePath+symbol; summary firmas; full cuerpos"),
        ("harness_symbols", "outline=estructura; read=fuente exacta + hash; selectores CSS en CSS/HTML/Razor"),
        ("harness_read", "start/lines o end; path o file"),
        ("harness_search", "kind=text|files|file_size|syntax; scope omitido/null/./vacío=raíz; contextLines=0..20 añade fuente y hash; kind=syntax consulta el índice de símbolos cuando lo cubre y repara en vivo lo obsoleto (discovery en la respuesta)"),
        ("harness_ignore", "action=list|add|remove|init; scope opcional; .harnessignore con sintaxis gitignore"),
        ("harness_detail", "ref de salida truncada; offset/maxChars"),
        ("harness_replace", "oldText/newText o chunks; line opcional; hash omitido=lectura interna; preview=true no publica"),
        ("harness_write", "expectedHash permite reemplazo atómico"),
        ("harness_move", "from/to relativos; el padre debe existir"),
        ("harness_verify", "language; action=build|test; package/scope; dotnet engine=auto|dotnet|msbuild; warnings=all|changed-files|none"),
        ("harness_jobs", "start devuelve taskId; observe sigue stdout/stderr"),
        ("harness_git", "diff exige path; comparison=auto muestra staged/worktree/total; status incluye sessionChanges"),
        ("harness_sql", "indica engine si hay dos motores; search schema omitido o *=todos los esquemas autorizados"),
        ("harness_xlsx", "sin sheet=metadatos; range/rowOffset/pageSize"),
        ("harness_svg", "mode=inspect|preview"),
        ("harness_config", "section obligatoria; raw=true IIS/ASP.NET"),
        ("harness_doctor", "env|syntax|logs|process|health"),
        ("harness_external_scope", "grant exige root y userAuthorized=true"),
        ("harness_data_query", "exactamente uno de path|data|resultRef"),
        ("harness_batch", "jobs[1..4]; maxConcurrent 2..4"),
        ("harness_session", "checkpoint goal/decisions/pending; tasks timeline"),
    ]
    .iter()
    .copied()
    .filter(|(tool, _)| available_tools.iter().any(|name| name.as_str() == *tool))
    .map(|(tool, usage)| json!({"tool": tool, "usage": usage}))
    .collect();
    json!(quick_guide)
}
