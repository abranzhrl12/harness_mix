//! Named, independently indexed projects exposed through one Harness server.
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::adapters::NativeWorkspace;
use harness_application::ports::browser::BrowserInspection;
use harness_application::ports::workspace::{
    ArtifactQueries, IgnoreManagement, LogPersistence, SessionPersistence, SourceEditing,
    SourcePreviews, SourceQueries, SqlAccess, Verification, VersionControl, WorkspaceDiagnostics,
    WorkspaceMetadata,
};
use harness_domain::{Failure, execution::OperationContext};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use serde::Deserialize;

#[derive(Clone, Debug)]
pub struct WorkspaceProject {
    pub name: String,
    pub root: PathBuf,
}

/// One Application can serve several independently indexed projects. The
/// application keeps one result store and one session repository; this adapter
/// routes workspace operations to the owning project.
pub struct MultiNativeWorkspace {
    pub projects: Vec<(WorkspaceProject, std::sync::Arc<NativeWorkspace>)>,
    pub workspace_mode: bool,
    pub anchor: Option<std::sync::Arc<NativeWorkspace>>,
    pub reload_status: Option<std::sync::Arc<std::sync::Mutex<Value>>>,
}

// A browser connection is session-owned, not tied to a project's filesystem.
// Keep its registry on the primary workspace so IDs route consistently.
impl BrowserInspection for MultiNativeWorkspace {
    fn browser(
        &self,
        args: &harness_domain::browser::BrowserArgs,
        context: &OperationContext,
    ) -> Result<harness_domain::browser::BrowserResult, Failure> {
        self.primary().browser(args, context)
    }
}

impl MultiNativeWorkspace {
    fn primary(&self) -> &NativeWorkspace {
        self.anchor.as_deref().unwrap_or(&self.projects[0].1)
    }
    fn route<A: Serialize + DeserializeOwned>(
        &self,
        args: &A,
        fields: &[&str],
    ) -> Result<(usize, A), Failure> {
        let mut value = serde_json::to_value(args)
            .map_err(|_| ("INTERNAL", "Cannot serialize workspace arguments"))?;
        if !self.workspace_mode {
            let args = serde_json::from_value(value)
                .map_err(|_| ("ARGUMENT", "Cannot preserve workspace arguments"))?;
            return Ok((0, args));
        }
        let object = value
            .as_object_mut()
            .ok_or(("ARGUMENT", "Workspace arguments must be an object"))?;
        if object
            .get("externalScope")
            .and_then(Value::as_str)
            .is_some()
        {
            let args = serde_json::from_value(value)
                .map_err(|_| ("ARGUMENT", "Cannot preserve externally scoped arguments"))?;
            return Ok((0, args));
        }
        let mut owner = 0usize;
        let mut found = false;
        for field in fields {
            let Some(path) = object
                .get(*field)
                .and_then(Value::as_str)
                .filter(|path| !path.is_empty())
            else {
                continue;
            };
            let roots = self
                .projects
                .iter()
                .map(|(project, _)| project.clone())
                .collect::<Vec<_>>();
            let resolved = WorkspaceProject::resolve(&roots, path);
            let (index, relative) = match resolved {
                Ok((project, relative)) => (
                    self.projects
                        .iter()
                        .position(|(candidate, _)| candidate.name == project.name)
                        .ok_or(("INTERNAL", "Resolved project disappeared"))?,
                    relative,
                ),
                Err(_)
                    if found
                        && !Path::new(path).is_absolute()
                        && !path.contains(":/")
                        && !path.split(['/', '\\']).any(|part| part == "..") =>
                {
                    (owner, path.replace('\\', "/"))
                }
                Err(error) if error.starts_with("ARGUMENT:") => {
                    return Err((
                        "ARGUMENT",
                        "Relative path is ambiguous; prefix it with project:/path",
                    ));
                }
                Err(_) => {
                    return Err((
                        "PATH",
                        "Path is outside the registered projects or cannot be resolved",
                    ));
                }
            };
            if found && owner != index {
                return Err((
                    "ARGUMENT",
                    "All paths in one operation must belong to the same project",
                ));
            }
            owner = index;
            found = true;
            object.insert((*field).to_owned(), Value::String(relative));
        }
        let args = serde_json::from_value(value)
            .map_err(|_| ("ARGUMENT", "Cannot normalize project paths"))?;
        Ok((owner, args))
    }

    fn one<A: Serialize + DeserializeOwned>(
        &self,
        args: &A,
        fields: &[&str],
    ) -> Result<(usize, A), Failure> {
        self.route(args, fields)
    }
}

impl WorkspaceMetadata for MultiNativeWorkspace {
    fn external_scope(
        &self,
        args: &harness_domain::contracts::ExternalScopeArgs,
        ctx: &OperationContext,
    ) -> Result<Value, Failure> {
        self.primary().external_scope(args, ctx)
    }
    fn config(
        &self,
        args: &harness_domain::contracts::ConfigArgs,
        ctx: &OperationContext,
    ) -> Result<Value, Failure> {
        let (i, args) = self.one(args, &["path"])?;
        self.projects[i].1.config(&args, ctx)
    }
    fn info(&self) -> Value {
        if !self.workspace_mode {
            return self.projects[0].1.info();
        }
        let mut info = json!({"workspace":true,"projects":self.projects.iter().map(|(p,w)| json!({"name":p.name,"root":p.root,"info":w.info()})).collect::<Vec<_>>()});
        if let Some(status) = &self.reload_status {
            info["workspaceReload"] = status.lock().unwrap_or_else(|e| e.into_inner()).clone();
        }
        info
    }
    fn rules(&self, ctx: &OperationContext) -> Result<String, Failure> {
        if !self.workspace_mode {
            return self.projects[0].1.rules(ctx);
        }
        let mut sections = vec![
            "Workspace routing: use alias:/relative/path for any registered project. Relative paths are inferred only when they exist in exactly one project. Searches without scope and graph exploration without filePath cover every registered project.".to_owned()
        ];
        for (project, workspace) in &self.projects {
            let rules = workspace.rules(ctx)?;
            if !rules.is_empty() {
                sections.push(format!("## Project {}\n{}", project.name, rules));
            }
        }
        Ok(sections.join("\n\n"))
    }
    fn capabilities(&self) -> Value {
        if !self.workspace_mode {
            return self.projects[0].1.capabilities();
        }
        json!({"workspace":true,"projects":self.projects.iter().map(|(p,w)| json!({"name":p.name,"info":w.info(),"capabilities":w.capabilities()})).collect::<Vec<_>>()})
    }
}

impl SourceQueries for MultiNativeWorkspace {
    fn lsp(&self, args: &harness_domain::lsp::LspArgs, context: &OperationContext) -> Result<Value, Failure> {
        let (owner, args) = self.route(args, &["path"])?;
        let mut result = self.projects[owner].1.lsp(&args, context)?;
        if self.workspace_mode {
            let name = &self.projects[owner].0.name;
            result["project"] = json!(name);
            result["path"] = json!(format!("{name}:/{}", args.path));
            if let Some(locations) = result["locations"].as_array_mut() {
                for location in locations {
                    if let Some(path) = location["path"].as_str() {
                        location["path"] = json!(format!("{name}:/{path}"));
                    }
                }
            }
        }
        Ok(result)
    }
    fn explore(
        &self,
        args: &harness_domain::contracts::ExploreArgs,
        ctx: &OperationContext,
    ) -> Result<Value, Failure> {
        if !self.workspace_mode {
            return self.projects[0].1.explore(args, ctx);
        }
        if args
            .file_path
            .as_deref()
            .is_some_and(|path| !path.is_empty())
        {
            let (i, args) = self.one(args, &["filePath"])?;
            let mut result = self.projects[i].1.explore(&args, ctx)?;
            qualify_paths(&mut result, &self.projects[i].0.name, self.workspace_mode);
            return Ok(result);
        }
        let mut all = Vec::new();
        let mut details = Vec::new();
        let mut failures = Vec::new();
        for (project, workspace) in &self.projects {
            let mut result = match workspace.explore(args, ctx) {
                Ok(result) => result,
                Err((code, message)) => {
                    failures.push(json!({"project":project.name,"code":code,"message":message}));
                    all.push(json!({"project":project.name,"state":"error","code":code,"message":message}));
                    continue;
                }
            };
            if result.get("ok") == Some(&Value::Bool(false)) {
                let code = result["diagnostics"]
                    .as_array()
                    .and_then(|items| items.first())
                    .and_then(|item| item["code"].as_str())
                    .unwrap_or("GRAPH_QUERY");
                let message = result["diagnostics"]
                    .as_array()
                    .and_then(|items| items.first())
                    .and_then(|item| item["message"].as_str())
                    .unwrap_or("Graph query failed for this project");
                failures.push(json!({"project":project.name,"code":code,"message":message}));
                all.push(
                    json!({"project":project.name,"state":"error","code":code,"message":message}),
                );
                continue;
            }
            qualify_paths(&mut result, &project.name, self.workspace_mode);
            if let Some(detail) = result
                .as_object_mut()
                .and_then(|object| object.remove("_detail"))
            {
                details.push(json!({"project":project.name,"detail":detail}));
            }
            all.push(json!({"project":project.name,"state":"ok","data":result}));
        }
        let mut output =
            json!({"projects":all,"complete":failures.is_empty(),"_complete":failures.is_empty()});
        if !details.is_empty() {
            output["_detail"] = json!({"projects":details});
        }
        if !failures.is_empty() {
            output["diagnostics"] = json!(failures);
        }
        if !output["projects"]
            .as_array()
            .is_some_and(|projects| projects.iter().any(|project| project["state"] == "ok"))
        {
            output["ok"] = Value::Bool(false);
        }
        Ok(output)
    }
    fn outline(
        &self,
        args: &harness_domain::contracts::OutlineArgs,
        ctx: &OperationContext,
    ) -> Result<Value, Failure> {
        let (i, args) = self.one(args, &["path"])?;
        let mut result = self.projects[i].1.outline(&args, ctx)?;
        qualify_paths(&mut result, &self.projects[i].0.name, self.workspace_mode);
        Ok(result)
    }
    fn read_symbol(
        &self,
        args: &harness_domain::contracts::ReadSymbolArgs,
        ctx: &OperationContext,
    ) -> Result<Value, Failure> {
        let (i, args) = self.one(args, &["path"])?;
        let mut result = self.projects[i].1.read_symbol(&args, ctx)?;
        qualify_paths(&mut result, &self.projects[i].0.name, self.workspace_mode);
        Ok(result)
    }
    fn read(
        &self,
        args: &harness_domain::contracts::ReadArgs,
        ctx: &OperationContext,
    ) -> Result<Value, Failure> {
        let (i, args) = self.one(args, &["path"])?;
        let mut result = self.projects[i].1.read(&args, ctx)?;
        qualify_paths(&mut result, &self.projects[i].0.name, self.workspace_mode);
        Ok(result)
    }
    fn search(
        &self,
        args: &harness_domain::contracts::SearchArgs,
        ctx: &OperationContext,
    ) -> Result<Value, Failure> {
        if !self.workspace_mode {
            return self.projects[0].1.search(args, ctx);
        }
        if args
            .scope
            .as_deref()
            .is_some_and(|scope| !scope.is_empty() && scope != ".")
        {
            let (i, args) = self.one(args, &["scope"])?;
            let mut result = self.projects[i].1.search(&args, ctx)?;
            qualify_paths(&mut result, &self.projects[i].0.name, self.workspace_mode);
            return Ok(result);
        }
        let mut combined = json!({"matches":[],"files":[],"paths":[],"skippedFiles":[],"projects":[],"complete":true,"_complete":true,"scanned":0,"bytesRead":0,"skipped":0,"omitted":0});
        let mut failures = Vec::new();
        for (project, workspace) in &self.projects {
            let mut per = serde_json::to_value(args)
                .map_err(|_| ("INTERNAL", "Cannot serialize search arguments"))?;
            per["scope"] = Value::String(".".into());
            let per: harness_domain::contracts::SearchArgs = serde_json::from_value(per)
                .map_err(|_| ("ARGUMENT", "Cannot normalize search arguments"))?;
            let mut result = match workspace.search(&per, ctx) {
                Ok(result) => result,
                Err((code, message)) => {
                    failures.push(json!({"project":project.name,"code":code,"message":message}));
                    combined["projects"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!({"project":project.name,"state":"error","code":code}));
                    combined["complete"] = Value::Bool(false);
                    combined["_complete"] = Value::Bool(false);
                    continue;
                }
            };
            let name = project.name.clone();
            qualify_paths(&mut result, &name, self.workspace_mode);
            for key in ["matches", "files", "paths", "skippedFiles"] {
                if let Some(items) = result.get_mut(key).and_then(Value::as_array_mut) {
                    combined[key]
                        .as_array_mut()
                        .expect("array initialized")
                        .append(items);
                }
            }
            for key in ["scanned", "bytesRead", "skipped"] {
                combined[key] =
                    json!(combined[key].as_u64().unwrap_or(0) + result[key].as_u64().unwrap_or(0));
            }
            combined["omitted"] = json!(
                combined["omitted"].as_u64().unwrap_or(0) + result["omitted"].as_u64().unwrap_or(0)
            );
            combined["complete"] =
                json!(combined["complete"] == true && result["complete"] == true);
            if result["_complete"] == false {
                combined["_complete"] = Value::Bool(false);
            }
            combined["projects"].as_array_mut().unwrap().push(json!({"project":name,"state":"ok","complete":result["complete"],"scanned":result["scanned"],"skipped":result["skipped"]}));
            if let Some(detail) = result.get_mut("_detail").and_then(Value::as_object_mut) {
                if !combined["_detail"].is_object() {
                    combined["_detail"] = json!({});
                }
                let target = combined["_detail"].as_object_mut().expect("detail object");
                for (key, value) in std::mem::take(detail) {
                    if let Some(items) = value.as_array() {
                        if let Some(current) = target.get_mut(&key).and_then(Value::as_array_mut) {
                            current.extend(items.clone());
                        } else {
                            target.insert(key, Value::Array(items.clone()));
                        }
                    } else if !target.contains_key(&key) {
                        target.insert(key, value);
                    }
                }
            }
        }
        let limit = args.limit.unwrap_or(
            if matches!(args.kind, harness_domain::contracts::Kind::FileSize) {
                30
            } else {
                40
            },
        );
        let mut dropped = 0usize;
        for key in ["matches", "files", "paths"] {
            if let Some(items) = combined[key].as_array_mut() {
                if items.len() > limit {
                    dropped += items.len() - limit;
                    items.truncate(limit);
                }
            }
        }
        if dropped > 0 {
            combined["complete"] = Value::Bool(false);
            combined["omitted"] = json!(combined["omitted"].as_u64().unwrap_or(0) + dropped as u64);
        }
        if !failures.is_empty() {
            combined["diagnostics"] = json!(failures);
            combined["_complete"] = Value::Bool(false);
        }
        if !combined["projects"]
            .as_array()
            .is_some_and(|projects| projects.iter().any(|project| project["state"] == "ok"))
        {
            combined["ok"] = Value::Bool(false);
        }
        Ok(combined)
    }
}

pub(crate) fn qualify_paths(value: &mut Value, project: &str, enabled: bool) {
    if !enabled {
        return;
    }
    prefix_paths(value, project, false);
}

fn prefix_paths(value: &mut Value, project: &str, in_metadata_list: bool) {
    match value {
        Value::Array(items) => {
            for item in items {
                prefix_paths(item, project, in_metadata_list);
            }
        }
        Value::Object(object) => {
            let metadata_record = object.contains_key("hash")
                || object.contains_key("file")
                || object.contains_key("filePath")
                || (object.contains_key("id") && object.contains_key("kind"))
                || (object.contains_key("symbol") && object.contains_key("startLine"));
            for (key, value) in object.iter_mut() {
                let metadata_array = matches!(
                    key.as_str(),
                    "matches"
                        | "diagnostics"
                        | "files"
                        | "paths"
                        | "skippedFiles"
                        | "nodes"
                        | "definitions"
                        | "callers"
                        | "callees"
                        | "related"
                        | "focus"
                );
                if matches!(key.as_str(), "file" | "filePath" | "file_path")
                    || ((in_metadata_list || metadata_record) && key == "path")
                {
                    if let Some(path) = value.as_str() {
                        if !path.contains(":/") && !Path::new(path).is_absolute() {
                            *value = Value::String(format!("{project}:/{path}"));
                        }
                        continue;
                    }
                }
                if key == "paths" {
                    if let Some(paths) = value.as_array_mut() {
                        for item in paths {
                            if let Some(path) = item.as_str() {
                                if !path.contains(":/") && !Path::new(path).is_absolute() {
                                    *item = Value::String(format!("{project}:/{path}"));
                                }
                            } else {
                                prefix_paths(item, project, true);
                            }
                        }
                        continue;
                    }
                }
                prefix_paths(value, project, metadata_array);
            }
        }
        _ => {}
    }
}

impl SourcePreviews for MultiNativeWorkspace {
    fn vue_extract_preview(
        &self,
        a: &harness_domain::contracts::VueExtractPreviewArgs,
        c: &OperationContext,
    ) -> Result<Value, Failure> {
        let (i, a) = self.one(a, &["path", "targetPath"])?;
        let mut v = SourcePreviews::vue_extract_preview(self.projects[i].1.as_ref(), &a, c)?;
        qualify_paths(&mut v, &self.projects[i].0.name, self.workspace_mode);
        Ok(v)
    }
    fn patch_preview(
        &self,
        a: &harness_domain::contracts::ReplaceArgs,
        c: &OperationContext,
    ) -> Result<Value, Failure> {
        let (i, a) = self.one(a, &["path"])?;
        let mut v = SourcePreviews::patch_preview(self.projects[i].1.as_ref(), &a, c)?;
        qualify_paths(&mut v, &self.projects[i].0.name, self.workspace_mode);
        Ok(v)
    }
}

impl SourceEditing for MultiNativeWorkspace {
    fn vue_extract_preview(
        &self,
        args: &harness_domain::contracts::VueExtractPreviewArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        <Self as SourcePreviews>::vue_extract_preview(self, args, context)
    }
    fn patch_preview(
        &self,
        args: &harness_domain::contracts::ReplaceArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        <Self as SourcePreviews>::patch_preview(self, args, context)
    }
    fn replace(
        &self,
        a: &harness_domain::contracts::ReplaceArgs,
        c: &OperationContext,
    ) -> Result<Value, Failure> {
        let (i, a) = self.one(a, &["path"])?;
        let mut v = self.projects[i].1.replace(&a, c)?;
        qualify_paths(&mut v, &self.projects[i].0.name, self.workspace_mode);
        Ok(v)
    }
    fn move_file(
        &self,
        a: &harness_domain::contracts::MoveArgs,
        c: &OperationContext,
    ) -> Result<Value, Failure> {
        let (i, a) = self.one(a, &["from", "to"])?;
        let mut v = self.projects[i].1.move_file(&a, c)?;
        qualify_paths(&mut v, &self.projects[i].0.name, self.workspace_mode);
        Ok(v)
    }
    fn delete_file(
        &self,
        a: &harness_domain::contracts::DeleteArgs,
        c: &OperationContext,
    ) -> Result<Value, Failure> {
        let (i, a) = self.one(a, &["path"])?;
        let mut v = self.projects[i].1.delete_file(&a, c)?;
        qualify_paths(&mut v, &self.projects[i].0.name, self.workspace_mode);
        Ok(v)
    }
    fn prepare_proposal(
        &self,
        a: &harness_domain::contracts::ProposalArgs,
        c: &OperationContext,
    ) -> Result<Value, Failure> {
        let (i, a) = self.one(a, &["path"])?;
        let mut v = self.projects[i].1.prepare_proposal(&a, c)?;
        qualify_paths(&mut v, &self.projects[i].0.name, self.workspace_mode);
        Ok(v)
    }
    fn write(
        &self,
        a: &harness_domain::contracts::WriteArgs,
        c: &OperationContext,
    ) -> Result<Value, Failure> {
        let (i, a) = self.one(a, &["path"])?;
        let mut v = self.projects[i].1.write(&a, c)?;
        qualify_paths(&mut v, &self.projects[i].0.name, self.workspace_mode);
        Ok(v)
    }
    fn ignore(
        &self,
        args: &harness_domain::contracts::IgnoreArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        <Self as IgnoreManagement>::ignore(self, args, context)
    }
}

impl IgnoreManagement for MultiNativeWorkspace {
    fn ignore(
        &self,
        a: &harness_domain::contracts::IgnoreArgs,
        c: &OperationContext,
    ) -> Result<Value, Failure> {
        let (i, a) = self.one(a, &["scope"])?;
        let mut v = IgnoreManagement::ignore(self.projects[i].1.as_ref(), &a, c)?;
        qualify_paths(&mut v, &self.projects[i].0.name, self.workspace_mode);
        Ok(v)
    }
}

impl Verification for MultiNativeWorkspace {
    fn verify(
        &self,
        a: &harness_domain::contracts::VerifyArgs,
        c: &OperationContext,
    ) -> Result<Value, Failure> {
        if !self.workspace_mode {
            return self.projects[0].1.verify(a, c);
        }
        if a.scope
            .as_deref()
            .is_none_or(|scope| scope.is_empty() || scope == ".")
        {
            let mut projects = Vec::new();
            let mut details = Vec::new();
            let mut failures = Vec::new();
            for (project, workspace) in &self.projects {
                let mut value = serde_json::to_value(a)
                    .map_err(|_| ("INTERNAL", "Cannot serialize verification arguments"))?;
                value["scope"] = Value::String(".".into());
                let mapped = serde_json::from_value(value)
                    .map_err(|_| ("ARGUMENT", "Cannot normalize verification scope"))?;
                let mut result = match workspace.verify(&mapped, c) {
                    Ok(result) => result,
                    Err((code, message)) => {
                        failures
                            .push(json!({"project":project.name,"code":code,"message":message}));
                        projects.push(json!({"project":project.name,"state":"error","code":code,"message":message}));
                        continue;
                    }
                };
                qualify_paths(&mut result, &project.name, true);
                let state = if result.get("ok") == Some(&Value::Bool(false)) {
                    "error"
                } else {
                    "ok"
                };
                if state == "error" {
                    failures.push(json!({"project":project.name,"code":result["code"],"message":result["message"]}));
                }
                if let Some(detail) = result
                    .as_object_mut()
                    .and_then(|object| object.remove("_detail"))
                {
                    details.push(json!({"project":project.name,"detail":detail}));
                }
                projects.push(json!({"project":project.name,"state":state,"data":result}));
            }
            let complete = failures.is_empty();
            let mut output = json!({"projects":projects,"complete":complete,"_complete":complete});
            if !details.is_empty() {
                output["_detail"] = json!({"projects":details});
            }
            if !failures.is_empty() {
                output["ok"] = Value::Bool(false);
                output["diagnostics"] = json!(failures);
            }
            return Ok(output);
        }
        let (i, a) = self.one(a, &["scope"])?;
        let mut v = self.projects[i].1.verify(&a, c)?;
        qualify_paths(&mut v, &self.projects[i].0.name, true);
        Ok(v)
    }
}

impl VersionControl for MultiNativeWorkspace {
    fn git_status(
        &self,
        a: &harness_domain::contracts::GitStatusArgs,
        c: &OperationContext,
    ) -> Result<Value, Failure> {
        if !self.workspace_mode {
            return self.projects[0].1.git_status(a, c);
        }
        let mut results = Vec::new();
        let mut failures = Vec::new();
        for (project, workspace) in &self.projects {
            match workspace.git_status(a, c) {
                Ok(mut data) => {
                    qualify_paths(&mut data, &project.name, true);
                    results.push(json!({"project":project.name,"state":"ok","data":data}));
                }
                Err((code, message)) => {
                    failures.push(json!({"project":project.name,"code":code,"message":message}));
                    results.push(json!({"project":project.name,"state":"error","code":code,"message":message}));
                }
            }
        }
        let complete = failures.is_empty();
        let mut output = json!({"projects":results,"complete":complete,"_complete":complete});
        if !failures.is_empty() {
            output["diagnostics"] = json!(failures);
        }
        if !output["projects"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["state"] == "ok"))
        {
            output["ok"] = Value::Bool(false);
        }
        Ok(output)
    }
    fn git_session_changes(&self, edits: &[Value], c: &OperationContext) -> Result<Value, Failure> {
        if !self.workspace_mode {
            return self.projects[0].1.git_session_changes(edits, c);
        }
        let mut results = Vec::new();
        let mut failures = Vec::new();
        for (project, workspace) in &self.projects {
            match workspace.git_session_changes(edits, c) {
                Ok(mut data) => {
                    qualify_paths(&mut data, &project.name, true);
                    results.push(json!({"project":project.name,"state":"ok","data":data}));
                }
                Err((code, message)) => {
                    failures.push(json!({"project":project.name,"code":code,"message":message}));
                    results.push(json!({"project":project.name,"state":"error","code":code,"message":message}));
                }
            }
        }
        let complete = failures.is_empty();
        let mut output = json!({"projects":results,"complete":complete,"_complete":complete});
        if !failures.is_empty() {
            output["diagnostics"] = json!(failures);
        }
        Ok(output)
    }
    fn git_diff(
        &self,
        a: &harness_domain::contracts::GitDiffArgs,
        c: &OperationContext,
    ) -> Result<Value, Failure> {
        let (i, a) = self.one(a, &["path"])?;
        let mut result = self.projects[i].1.git_diff(&a, c)?;
        qualify_paths(&mut result, &self.projects[i].0.name, self.workspace_mode);
        Ok(result)
    }
}

impl WorkspaceDiagnostics for MultiNativeWorkspace {
    fn doctor(
        &self,
        a: &harness_domain::contracts::DoctorArgs,
        c: &OperationContext,
    ) -> Result<Value, Failure> {
        self.projects[0].1.doctor(a, c)
    }
    fn process_status(
        &self,
        a: &harness_domain::contracts::ProcessStatusArgs,
        c: &OperationContext,
    ) -> Result<Value, Failure> {
        self.projects[0].1.process_status(a, c)
    }
    fn diagnostics(
        &self,
        a: &harness_domain::contracts::DiagnosticsArgs,
        c: &OperationContext,
    ) -> Result<Value, Failure> {
        self.projects[0].1.diagnostics(a, c)
    }
    fn observe_workspace(&self, c: &OperationContext) -> Result<Value, Failure> {
        if !self.workspace_mode {
            return self.projects[0].1.observe_workspace(c);
        }
        let mut observations = Vec::new();
        let mut complete = true;
        for (project, workspace) in &self.projects {
            let observation = workspace.observe_workspace(c)?;
            if observation["status"] == "unknown" {
                complete = false;
            }
            observations.push(json!({"project":project.name,"observation":observation}));
        }
        Ok(json!({"projects":observations,"complete":complete,"_complete":complete}))
    }
}

impl SqlAccess for MultiNativeWorkspace {
    fn sql_status(
        &self,
        a: &harness_domain::sql::SqlStatusArgs,
        c: &OperationContext,
    ) -> Result<Value, Failure> {
        self.projects[0].1.sql_status(a, c)
    }
    fn sql_search(
        &self,
        a: &harness_domain::sql::SqlSearchArgs,
        c: &OperationContext,
    ) -> Result<Value, Failure> {
        self.projects[0].1.sql_search(a, c)
    }
    fn sql_map(
        &self,
        a: &harness_domain::sql::SqlMapArgs,
        c: &OperationContext,
    ) -> Result<Value, Failure> {
        self.projects[0].1.sql_map(a, c)
    }
    fn sql_describe(
        &self,
        a: &harness_domain::sql::SqlDescribeArgs,
        c: &OperationContext,
    ) -> Result<Value, Failure> {
        self.projects[0].1.sql_describe(a, c)
    }
    fn sql_query(
        &self,
        a: &harness_domain::sql::SqlQueryArgs,
        c: &OperationContext,
    ) -> Result<Value, Failure> {
        self.projects[0].1.sql_query(a, c)
    }
}

impl ArtifactQueries for MultiNativeWorkspace {
    fn xlsx_inspect(
        &self,
        a: &harness_domain::xlsx::XlsxInspectArgs,
        c: &OperationContext,
    ) -> Result<Value, Failure> {
        let (i, a) = self.one(a, &["path"])?;
        let mut v = self.projects[i].1.xlsx_inspect(&a, c)?;
        qualify_paths(&mut v, &self.projects[i].0.name, self.workspace_mode);
        Ok(v)
    }
    fn xlsx_read(
        &self,
        a: &harness_domain::xlsx::XlsxReadArgs,
        c: &OperationContext,
    ) -> Result<Value, Failure> {
        let (i, a) = self.one(a, &["path"])?;
        let mut v = self.projects[i].1.xlsx_read(&a, c)?;
        qualify_paths(&mut v, &self.projects[i].0.name, self.workspace_mode);
        Ok(v)
    }
    fn svg(
        &self,
        a: &harness_domain::svg::SvgArgs,
        c: &OperationContext,
    ) -> Result<Value, Failure> {
        let (i, a) = self.one(a, &["path"])?;
        let mut v = self.projects[i].1.svg(&a, c)?;
        qualify_paths(&mut v, &self.projects[i].0.name, self.workspace_mode);
        Ok(v)
    }
    fn data_query(
        &self,
        a: &harness_domain::contracts::DataQueryArgs,
        c: &OperationContext,
    ) -> Result<Value, Failure> {
        let (i, a) = self.one(a, &["path"])?;
        self.projects[i].1.data_query(&a, c)
    }
}

impl SessionPersistence for MultiNativeWorkspace {
    fn handover(
        &self,
        a: &harness_domain::contracts::HandoverArgs,
        snapshot: &Value,
        c: &OperationContext,
    ) -> Result<Value, Failure> {
        if a.target_file.is_some() {
            let (i, a) = self.one(a, &["targetFile"])?;
            self.projects[i].1.handover(&a, snapshot, c)
        } else {
            self.primary().handover(a, snapshot, c)
        }
    }
    fn persist_session(
        &self,
        id: &str,
        snapshot: &Value,
        c: &OperationContext,
    ) -> Result<(), Failure> {
        self.primary().persist_session(id, snapshot, c)
    }
}

impl LogPersistence for MultiNativeWorkspace {
    fn log_persistence_enabled(&self) -> bool {
        self.primary().log_persistence_enabled()
    }
    fn load_log_archive(&self) -> Result<Option<Value>, Failure> {
        self.primary().load_log_archive()
    }
    fn persist_log_archive(&self, archive: &Value) -> Result<u64, Failure> {
        self.primary().persist_log_archive(archive)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    projects: std::collections::BTreeMap<String, ProjectEntry>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ProjectEntry {
    Root(String),
    Object(ProjectRoot),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectRoot {
    root: String,
}

impl WorkspaceProject {
    pub fn from_manifest(path: &Path) -> std::io::Result<Vec<Self>> {
        let bytes = fs::read(path)?;
        Self::from_manifest_bytes(path, &bytes)
    }

    pub fn from_manifest_bytes(path: &Path, bytes: &[u8]) -> std::io::Result<Vec<Self>> {
        let manifest: Manifest = serde_json::from_slice(bytes)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        let mut projects = Vec::with_capacity(manifest.projects.len());
        for (name, entry) in manifest.projects {
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "project names must use ASCII letters, digits, '_' or '-'",
                ));
            }
            let root = match entry {
                ProjectEntry::Root(root) => root,
                ProjectEntry::Object(entry) => entry.root,
            };
            let root = PathBuf::from(root);
            let root = if root.is_absolute() {
                root
            } else {
                base.join(root)
            }
            .canonicalize()?;
            if !root.is_dir() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("project {name} root is not a directory"),
                ));
            }
            projects.push(Self { name, root });
        }
        if projects.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "workspace manifest must register at least one project",
            ));
        }
        Ok(projects)
    }

    /// Resolve an absolute path, `project:/relative/path`, or an unqualified
    /// relative path that exists in exactly one registered project.
    pub fn resolve<'a>(projects: &'a [Self], path: &str) -> Result<(&'a Self, String), String> {
        let drive_path = path.as_bytes().get(1) == Some(&b':')
            && path.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
            && path.as_bytes().get(2) == Some(&b'/');
        if !drive_path {
            if let Some((name, relative)) = path.split_once(":/") {
                if let Some(project) = projects.iter().find(|project| project.name == name) {
                    let relative = relative.replace('\\', "/");
                    if Path::new(&relative).components().any(|component| {
                        matches!(
                            component,
                            std::path::Component::ParentDir
                                | std::path::Component::RootDir
                                | std::path::Component::Prefix(_)
                        )
                    }) {
                        return Err(
                            "PATH: project-prefixed paths cannot traverse outside their project"
                                .into(),
                        );
                    }
                    return Ok((
                        project,
                        if relative.is_empty() {
                            ".".to_owned()
                        } else {
                            relative
                        },
                    ));
                }
            }
        }
        let supplied = PathBuf::from(path);
        if supplied.is_absolute() {
            let canonical = canonicalize_with_missing(&supplied)
                .map_err(|_| "PATH: requested absolute path cannot be resolved")?;
            let candidates = projects
                .iter()
                .filter_map(|project| {
                    canonical
                        .strip_prefix(&project.root)
                        .ok()
                        .map(|relative| (project, relative.to_string_lossy().replace('\\', "/")))
                })
                .collect::<Vec<_>>();
            return match candidates.as_slice() {
                [(project, relative)] => Ok((
                    project,
                    if relative.is_empty() {
                        ".".to_owned()
                    } else {
                        relative.clone()
                    },
                )),
                [] => {
                    Err("PATH: requested path is outside the registered workspace projects".into())
                }
                _ => Err(
                    "ARGUMENT: path belongs to overlapping registered projects; use project:/path"
                        .into(),
                ),
            };
        }
        let normalized = path.replace('\\', "/");
        if Path::new(&normalized).components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        }) {
            return Err("PATH: relative paths cannot traverse outside a project".into());
        }
        let candidates = projects
            .iter()
            .filter(|project| project.root.join(&normalized).exists())
            .collect::<Vec<_>>();
        match candidates.as_slice() {
            [project] => Ok((project, normalized)),
            [] => Err(
                "PATH: relative path was not found in any registered project; use project:/path"
                    .into(),
            ),
            _ => Err(format!(
                "ARGUMENT: relative path is ambiguous across projects: {}; prefix it with project:/",
                candidates
                    .iter()
                    .map(|p| p.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }
}

fn canonicalize_with_missing(path: &Path) -> std::io::Result<PathBuf> {
    let mut existing = path;
    let mut suffix = Vec::new();
    while !existing.exists() {
        let name = existing.file_name().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "no existing path ancestor")
        })?;
        suffix.push(name.to_os_string());
        existing = existing.parent().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "no existing path ancestor")
        })?;
    }
    let mut resolved = existing.canonicalize()?;
    for name in suffix.into_iter().rev() {
        resolved.push(name);
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_alias_and_unique_relative_path_and_rejects_ambiguity() {
        let temp = tempfile::tempdir().unwrap();
        let backend = temp.path().join("backend");
        let frontend = temp.path().join("frontend");
        fs::create_dir_all(backend.join("src")).unwrap();
        fs::create_dir_all(frontend.join("src")).unwrap();
        fs::write(backend.join("src/main.rs"), "").unwrap();
        fs::write(frontend.join("src/App.vue"), "").unwrap();
        fs::write(frontend.join("README.md"), "").unwrap();
        let projects = vec![
            WorkspaceProject {
                name: "backend".into(),
                root: backend.canonicalize().unwrap(),
            },
            WorkspaceProject {
                name: "frontend".into(),
                root: frontend.canonicalize().unwrap(),
            },
        ];
        let (project, relative) =
            WorkspaceProject::resolve(&projects, "frontend:/src/App.vue").unwrap();
        assert_eq!(project.name, "frontend");
        assert_eq!(relative, "src/App.vue");
        let (project, relative) = WorkspaceProject::resolve(&projects, "README.md").unwrap();
        assert_eq!(project.name, "frontend");
        assert_eq!(relative, "README.md");
        assert!(WorkspaceProject::resolve(&projects, "src/main.rs").is_ok());
        fs::write(backend.join("README.md"), "").unwrap();
        assert!(
            WorkspaceProject::resolve(&projects, "README.md")
                .unwrap_err()
                .contains("ambiguous")
        );
        let new_file = frontend.join("new/deep/File.vue");
        let (project, relative) =
            WorkspaceProject::resolve(&projects, &new_file.to_string_lossy()).unwrap();
        assert_eq!(project.name, "frontend");
        assert_eq!(relative, "new/deep/File.vue");
        assert!(
            WorkspaceProject::resolve(&projects, "frontend:/../backend/src/main.rs")
                .unwrap_err()
                .contains("traverse")
        );
        let outside = temp.path().join("outside.rs");
        assert!(
            WorkspaceProject::resolve(&projects, &outside.to_string_lossy())
                .unwrap_err()
                .contains("outside")
        );
    }

    #[test]
    fn one_application_routes_reads_edits_search_and_graphs_across_two_indexed_projects() {
        use crate::adapters::{NativeApplicationOptions, composition};
        use harness_application::Application;
        use harness_domain::contracts::{
            BatchArgs, BootstrapArgs, DataQueryArgs, DetailArgs, ExploreArgs, ReadArgs, Request,
            SearchArgs, WriteArgs,
        };

        let temp = tempfile::tempdir().unwrap();
        let backend = temp.path().join("backend");
        let frontend = temp.path().join("frontend");
        let unindexed = temp.path().join("unindexed");
        fs::create_dir_all(&backend).unwrap();
        fs::create_dir_all(&frontend).unwrap();
        fs::create_dir_all(&unindexed).unwrap();
        fs::write(unindexed.join("other.rs"), "pub fn other() {}\n").unwrap();
        let backend_db = graph_fixture::fixture(&backend);
        let frontend_db = graph_fixture::fixture(&frontend);
        drop(backend_db);
        drop(frontend_db);

        let (backend_workspace, sessions) =
            composition::dependencies(&backend, NativeApplicationOptions::default()).unwrap();
        let (frontend_workspace, _) =
            composition::dependencies(&frontend, NativeApplicationOptions::default()).unwrap();
        let (unindexed_workspace, _) =
            composition::dependencies(&unindexed, NativeApplicationOptions::default()).unwrap();
        let app = Application::new(
            MultiNativeWorkspace {
                projects: vec![
                    (
                        WorkspaceProject {
                            name: "backend".into(),
                            root: backend.canonicalize().unwrap(),
                        },
                        std::sync::Arc::new(backend_workspace),
                    ),
                    (
                        WorkspaceProject {
                            name: "frontend".into(),
                            root: frontend.canonicalize().unwrap(),
                        },
                        std::sync::Arc::new(frontend_workspace),
                    ),
                    (
                        WorkspaceProject {
                            name: "unindexed".into(),
                            root: unindexed.canonicalize().unwrap(),
                        },
                        std::sync::Arc::new(unindexed_workspace),
                    ),
                ],
                workspace_mode: true,
                anchor: None,
                reload_status: None,
            },
            sessions,
        );

        let bootstrap = app.execute(Request::Bootstrap(BootstrapArgs {
            compact: false,
            session: None,
            include_rules: None,
        }));
        assert_eq!(bootstrap["state"], "ok", "{bootstrap}");
        let session = bootstrap["data"]["session"].as_str().unwrap().to_owned();

        let read: ReadArgs = serde_json::from_value(
            json!({"path":"frontend:/lib.rs","start":1,"end":1,"session":session}),
        )
        .unwrap();
        let read = app.execute(Request::Read(read));
        assert_eq!(read["state"], "ok", "{read}");
        assert_eq!(read["data"]["path"], "frontend:/lib.rs");

        let explore: ExploreArgs =
            serde_json::from_value(json!({"query":"target","view":"summary","session":session}))
                .unwrap();
        let explore = app.execute(Request::Explore(explore));
        assert_eq!(explore["state"], "ok", "{explore}");
        assert_eq!(explore["data"]["projects"].as_array().unwrap().len(), 3);
        assert_eq!(explore["data"]["complete"], false);
        assert_eq!(
            explore["data"]["projects"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|project| project["state"] == "ok")
                .count(),
            2
        );
        assert_eq!(
            explore["data"]["projects"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|project| project["state"] == "error")
                .count(),
            1
        );

        let search: SearchArgs = serde_json::from_value(
            json!({"kind":"text","query":"pub fn target","contextLines":1,"session":session}),
        )
        .unwrap();
        let search = app.execute(Request::Search(search));
        assert_eq!(search["state"], "ok", "{search}");
        let files = search["data"]["matches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["file"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert!(files.contains(&"backend:/lib.rs"));
        assert!(files.contains(&"frontend:/lib.rs"));

        let batch: BatchArgs = serde_json::from_value(json!({
            "maxConcurrent":2,
            "session":session,
            "jobs":[
                {"tool":"read","arguments":{"path":"backend:/lib.rs","start":1,"end":1}},
                {"tool":"read","arguments":{"path":"frontend:/lib.rs","start":1,"end":1}}
            ]
        }))
        .unwrap();
        let batch = app.execute(Request::Batch(batch));
        assert_eq!(batch["state"], "ok", "{batch}");
        assert_eq!(
            batch["data"]["results"].as_array().unwrap().len(),
            2,
            "{batch}"
        );

        let query = DataQueryArgs {
            path: None,
            data: Some(json!({"payload":"x".repeat(2000)})),
            result_ref: None,
            query: "payload".into(),
            max_chars: Some(500),
            session: Some(session.clone()),
        };
        let large = app.execute(Request::DataQuery(query));
        assert_eq!(large["state"], "ok", "{large}");
        let reference = large["detail"]["ref"]
            .as_str()
            .expect("saved detail reference");
        let detail = app.execute(Request::Detail(DetailArgs {
            reference: reference.to_owned(),
            offset: Some(0),
            max_chars: Some(4000),
            session: Some(session.clone()),
        }));
        assert_eq!(detail["state"], "ok", "{detail}");
        assert!(detail["data"]["text"].as_str().unwrap().contains("xxxx"));

        for project in ["backend", "frontend"] {
            let write: WriteArgs = serde_json::from_value(json!({
                "path":format!("{project}:/created.txt"),
                "content":format!("written-{project}"),
                "allowUnvalidatedText":true
            }))
            .unwrap();
            let result = app.execute(Request::Write(write));
            assert_eq!(result["state"], "ok", "{result}");
            assert_eq!(
                fs::read_to_string(temp.path().join(project).join("created.txt")).unwrap(),
                format!("written-{project}")
            );
        }

        let ambiguous: ReadArgs =
            serde_json::from_value(json!({"path":"lib.rs","start":1,"end":1})).unwrap();
        let ambiguous = app.execute(Request::Read(ambiguous));
        assert_eq!(ambiguous["state"], "error", "{ambiguous}");
        assert_eq!(ambiguous["diagnostics"][0]["code"], "ARGUMENT");
    }

    #[path = "../../../../tests/support/codegraph.rs"]
    mod graph_fixture;
}
