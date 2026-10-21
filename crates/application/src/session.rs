//! Session leases, persistence and bootstrap context. No native I/O dependencies.
use crate::Application;
use crate::SessionLease;
use crate::admission;
use crate::ports;
use crate::ports::{SessionRepository, Workspace};
use harness_domain::{
    Failure, contracts::BootstrapArgs, execution::OperationContext, hash, results::prefix,
    results::utf16_len,
};
use serde_json::{Value, json};

impl<W: Workspace, S: SessionRepository> Application<W, S> {
    pub(super) fn with_session_write<T>(
        &self,
        id: Option<&str>,
        lease: &mut Option<SessionLease<S>>,
        context: &OperationContext,
        run: impl FnOnce(Option<&str>) -> Result<T, Failure>,
    ) -> Result<T, Failure> {
        self.lease(id, lease)?;
        let real_id = lease.as_ref().map(|l| l.id.as_str());
        let gate = real_id
            .map(|id| {
                self.sessions
                    .lock()
                    .map_err(|_| ("INTERNAL", "Sessions unavailable"))?
                    .workflow_gate(id)
            })
            .transpose()?;
        let _guard = gate
            .as_ref()
            .map(|gate| {
                gate.try_lock().map_err(|_| {
                    (
                        "SESSION_BUSY",
                        "Another workflow write or verification is active in this session",
                    )
                })
            })
            .transpose()?;
        context.check()?;
        run(real_id)
    }
    pub(super) fn persist_update(&self, id: &str, mut update: ports::SessionMutation) -> Value {
        if let Some(snapshot) = update.snapshot {
            match self
                .workspace
                .persist_session(id, &snapshot, &OperationContext::default())
            {
                Ok(()) => update.data["persisted"] = json!(true),
                Err((code, message)) => {
                    update.data["persisted"] = json!(false);
                    update.data["persistenceWarning"] =
                        json!({"code":code,"message":message,"memoryUpdated":true});
                }
            }
        }
        update.data
    }
    pub(super) fn record_result(
        &self,
        id: Option<&str>,
        tool: &str,
        args: &Value,
        result: &mut Value,
    ) {
        let Some(id) = id else {
            return;
        };
        let mut sessions = match self.sessions.lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        let real_id = match sessions.resolve_id(id) {
            Ok(id) => id,
            Err(_) => return,
        };
        let snapshot = sessions.record(&real_id, tool, args, result);
        drop(sessions);
        let warning = match snapshot {
            Ok(Some(snapshot)) => self
                .workspace
                .persist_session(&real_id, &snapshot, &OperationContext::default())
                .err(),
            Ok(None) => None,
            Err(failure) => Some(failure),
        };
        if let Some((code, message)) = warning {
            result["sessionWarning"] =
                json!({"code":code,"message":message,"toolOutcomeUnchanged":true});
        }
    }
    pub(super) fn lease(
        &self,
        id: Option<&str>,
        lease: &mut Option<SessionLease<S>>,
    ) -> Result<(), Failure> {
        if let Some(id) = id {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| ("INTERNAL", "sesiones no disponibles"))?;
            let real_id = sessions.resolve_id(id)?;
            sessions.acquire(&real_id)?;
            *lease = Some(SessionLease {
                sessions: self.sessions.clone(),
                id: real_id,
            });
        }
        Ok(())
    }
    pub(super) fn bootstrap(
        &self,
        args: BootstrapArgs,
        context: &OperationContext,
        lease: &mut Option<SessionLease<S>>,
    ) -> Result<Value, Failure> {
        let is_new = args.session.is_none();
        let session = {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| ("INTERNAL", "sesiones no disponibles"))?;
            let session = sessions.open(args.session.as_deref())?;
            sessions.acquire(&session.id)?;
            *lease = Some(SessionLease {
                sessions: self.sessions.clone(),
                id: session.id.clone(),
            });
            session
        };
        context.check()?;
        let rules = self.workspace.rules(context)?;
        context.check()?;
        let rules_hash = hash(rules.as_bytes());
        let unchanged = session.rules_hash.as_ref() == Some(&rules_hash)
            && !args.include_rules.unwrap_or(false);
        let available = self.workspace.capabilities();
        let contract = self.tool_profile.metadata();
        let capabilities_hash = hash(
            json!({"executables":available,"contract":contract})
                .to_string()
                .as_bytes(),
        );
        let same_capabilities = session.capabilities_hash.as_ref() == Some(&capabilities_hash);
        let compact_reuse = !is_new && !session.restored && same_capabilities && unchanged;
        context.check()?;
        self.sessions
            .lock()
            .map_err(|_| ("INTERNAL", "sesiones no disponibles"))?
            .update_hashes(&session.id, rules_hash.clone(), capabilities_hash.clone())?;
        let mut result = self.workspace.info();
        result["session"] = json!(session.id);
        // Keep the old response key for existing clients; new examples use one name both ways.
        result["sessionId"] = json!(session.id);
        result["sessionUsage"] = json!(
            "Copy data.session into the session argument of later calls. sessionId is a legacy response alias."
        );
        result["exampleCall"] = json!({"tool":"harness_explore","arguments":{"query":"main","session":session.id},"note":"Replace main with the exact symbol you want; other arguments have read-only context defaults."});
        result["batchExample"] = json!({"tool":"harness_batch","arguments":{"jobs":[{"tool":"harness_search","arguments":{"kind":"files","query":"Cargo","scope":"."}},{"tool":"harness_git","arguments":{"action":"status"}}],"maxConcurrent":4,"session":session.id},"aliases":{"args":"arguments"}});
        result["detailRetention"] = json!({"session":"References with a session remain available for this server process lifetime; restart does not restore them.","capacity":"Bounded store rejects new details rather than evict session references.","expiresAt":null});
        let available_tools = self.tool_profile.tools();
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
        result["quickGuide"] = json!(quick_guide);
        result["rulesHash"] = json!(rules_hash);
        result["capabilitiesHash"] = json!(capabilities_hash);
        result["sessionRestored"] = json!(session.restored);
        result["implementation"] = json!("rust-experimental");
        result["toolContract"] = contract;
        result["backend"] = json!({"name":"rust","version":result.get("implementationVersion").cloned().unwrap_or(Value::Null)});
        result["logging"] = self.metrics.logs.status();
        result["supportedTools"] = json!(self.tool_profile.tools());
        result["toolProfile"] = json!(self.tool_profile.name());
        result["limitations"] = json!([
            "Edición y navegación dependen del lenguaje y backend; consultar las capacidades declaradas de cada herramienta",
            "Explore: índice existente con selectores exactos; syncIndex es opt-in y no certifica cobertura global",
            "Búsqueda text/files/file_size y syntax de declaraciones Rust (kind[:nombre]); sin patrones AST generales. Scopes Windows solo lectura",
            "Refresh solo de cobertura local; métricas por instancia, no históricas ni específicas de sesión",
            "Verificación Rust/Go/Python/JavaScript/.NET parcial; .NET exige .csproj/.sln, SDK y restore previo. Ejecuta código del proyecto, no es sandbox",
            "Git solo lectura; requiere git instalado y raíz Git o worktree vinculado registrado",
            "No sustituye el servidor actual"
        ]);
        result["executionPolicy"] = json!({"maxActive":admission::MAX_ACTIVE,"maxSearches":admission::MAX_SEARCHES,"maxBatchCoordinators":admission::MAX_BATCHES,"maxChildrenPerBatch":4,"queue":"reject_when_full","timeoutMs":30000,"rustVerifyProcessTimeoutMs":60000,"verifyCleanupGraceMs":5000,"cancellation":"cooperative"});
        if is_new {
            result["tasks"] = session.task_summary;
        }
        if let Some(warning) = session.persistence_warning {
            result["persistenceWarning"] = warning;
        }
        if is_new || args.include_rules.unwrap_or(false) || !same_capabilities {
            result["routing"] = json!(self.tool_profile.routing());
            result["workflow"] = self.tool_profile.workflows();
            result["usageRules"] =
                harness_domain::tool_contract::manifest()["llmPolicy"]["rules"].clone();
        }
        if same_capabilities {
            result["capabilitiesUnchanged"] = json!(true);
        } else {
            result["available"] = available;
        }
        if unchanged {
            result["rulesUnchanged"] = json!(true);
        } else {
            result["rules"] = json!(prefix(&rules, 12000));
            if utf16_len(&rules) > 12000 {
                result["truncated"] = json!(true);
                result["_detail"] = json!({"rules":rules});
            }
        }
        if compact_reuse {
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
        if args.compact && !compact_reuse {
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
                "contractHash":catalog["toolContract"]["contractHash"],"unchanged":same_capabilities});
            result["catalogCompact"] = json!(true);
            result["_detail"]["catalog"] = catalog;
        }
        Ok(result)
    }
}
