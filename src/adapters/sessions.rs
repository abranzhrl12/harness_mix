use super::project::{Failure, Project};
use harness_domain::{
    results::utf16_len,
    workflow::{IDS, STATUSES, TITLES},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
pub struct Declared {
    pub goal: String,
    pub decisions: Vec<String>,
    pub pending: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct State {
    pub declared: Declared,
    pub phases: Vec<Value>,
    pub edits: Vec<Value>,
    pub checks: Vec<Value>,
}
#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    id: String,
    data: State,
}

pub struct Session {
    pub(super) tasks: harness_domain::workflow::TaskBoard,
    pub(super) workflow_gate: std::sync::Arc<std::sync::Mutex<()>>,
    pub(super) observed_edits: Vec<Value>,
    pub(super) observed_checks: Vec<Value>,
    in_flight: usize,
    pub state: State,
    pub rules_hash: Option<String>,
    pub capabilities_hash: Option<String>,
    pub restored: bool,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            tasks: harness_domain::workflow::TaskBoard::new(&now()),
            workflow_gate: std::sync::Arc::new(std::sync::Mutex::new(())),
            observed_edits: vec![],
            observed_checks: vec![],
            in_flight: 0,
            state: State {
                declared: Declared {
                    goal: String::new(),
                    decisions: vec![],
                    pending: vec![],
                },
                phases: IDS
                    .iter()
                    .zip(TITLES)
                    .map(|(id, title)| json!({"id":id,"title":title,"status":"pending"}))
                    .collect(),
                edits: vec![],
                checks: vec![],
            },
            rules_hash: None,
            capabilities_hash: None,
            restored: false,
        }
    }
}
impl Session {
    pub fn summary(&self) -> Value {
        self.tasks.summary()
    }
}

fn safe_metadata(path: &Path) -> Result<fs::Metadata, Failure> {
    let meta =
        fs::symlink_metadata(path).map_err(|_| ("PERSISTENCE", "ruta de sesión no accesible"))?;
    if meta.file_type().is_symlink() {
        return Err(("PERSISTENCE", "enlace en ruta de sesión"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(("PERSISTENCE", "reparse point en ruta de sesión"));
        }
    }
    Ok(meta)
}

fn valid_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id)
}

pub struct Sessions {
    project: Project,
    entries: VecDeque<(String, Session)>,
    limit: usize,
    active_id: Option<String>,
}
impl Sessions {
    pub fn new(project: Project) -> Self {
        Self {
            project,
            entries: VecDeque::new(),
            limit: 16,
            active_id: None,
        }
    }
    fn directory(&self, create: bool) -> Result<Option<PathBuf>, Failure> {
        let mut path = self.project.root.clone();
        for part in [".agents", "harness", ".state"] {
            path.push(part);
            match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if !create {
                        return Ok(None);
                    }
                    let builder = fs::DirBuilder::new();
                    #[cfg(unix)]
                    let builder = {
                        use std::os::unix::fs::DirBuilderExt;
                        let mut builder = builder;
                        builder.mode(0o700);
                        builder
                    };
                    match builder.create(&path) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                        Err(_) => {
                            return Err(("PERSISTENCE", "no se pudo crear directorio de sesiones"));
                        }
                    }
                }
                Err(_) => {
                    return Err((
                        "PERSISTENCE",
                        "no se pudo inspeccionar directorio de sesiones",
                    ));
                }
                Ok(_) => {}
            }
            if !safe_metadata(&path)?.is_dir()
                || path
                    .canonicalize()
                    .map_err(|_| ("PERSISTENCE", "directorio no accesible"))?
                    != path
            {
                return Err(("PERSISTENCE", "directorio de sesión no seguro"));
            }
        }
        Ok(Some(path))
    }
    fn save(&self, id: &str, state: &State) -> Result<(), Failure> {
        if !valid_id(id) {
            return Err(("SESSION", "identificador de sesión inválido"));
        }
        let dir = self.directory(true)?.expect("created directory");
        let file = dir.join(format!("{id}.json"));
        match fs::symlink_metadata(&file) {
            Ok(_) => {
                if !safe_metadata(&file)?.is_file() {
                    return Err(("PERSISTENCE", "archivo de sesión no seguro"));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let files = fs::read_dir(&dir)
                    .map_err(|_| ("PERSISTENCE", "no se pudo listar sesiones"))?;
                let mut count = 0;
                for entry in files {
                    let entry = entry.map_err(|_| ("PERSISTENCE", "no se pudo listar sesiones"))?;
                    if entry.path().extension().is_some_and(|ext| ext == "json") {
                        count += 1;
                    }
                }
                if count >= 256 {
                    return Err(("PERSISTENCE", "límite de 256 sesiones persistentes"));
                }
            }
            Err(_) => return Err(("PERSISTENCE", "archivo de sesión no accesible")),
        }
        let bytes = serde_json::to_vec(&Saved {
            version: 1,
            id: id.into(),
            data: state.clone(),
        })
        .map_err(|_| ("PERSISTENCE", "sesión no serializable"))?;
        if bytes.len() > 256 * 1024 {
            return Err(("PERSISTENCE", "sesión demasiado grande"));
        }
        let mut temporary = tempfile::NamedTempFile::new_in(&dir)
            .map_err(|_| ("PERSISTENCE", "no se pudo crear sesión temporal"))?;
        temporary
            .write_all(&bytes)
            .and_then(|()| temporary.as_file().sync_all())
            .map_err(|_| ("PERSISTENCE", "no se pudo escribir sesión"))?;
        temporary
            .persist(&file)
            .map_err(|_| ("PERSISTENCE", "no se pudo publicar sesión"))?;
        Ok(())
    }
    fn load(&self, id: &str) -> Result<Session, Failure> {
        if !valid_id(id) {
            return Err(("SESSION", "identificador de sesión inválido"));
        }
        let dir = self.directory(false)?.ok_or((
            "SESSION",
            "sesión inexistente o descartada; ejecutar harness_bootstrap",
        ))?;
        let file = dir.join(format!("{id}.json"));
        let meta =
            safe_metadata(&file).map_err(|_| ("SESSION", "sesión no accesible o no segura"))?;
        if !meta.is_file() || meta.len() > 256 * 1024 {
            return Err(("SESSION", "archivo de sesión no admitido"));
        }
        let bytes = self
            .project
            .bytes(&format!(".agents/harness/.state/{id}.json"))?;
        if bytes.len() > 256 * 1024 {
            return Err(("SESSION", "archivo de sesión no admitido"));
        }
        let saved: Saved = serde_json::from_slice(&bytes)
            .map_err(|_| ("SESSION", "estado persistente inválido"))?;
        if saved.version != 1 || saved.id != id {
            return Err(("SESSION", "formato de sesión incompatible"));
        }
        let mut state = saved.data;
        validate(&mut state)?;
        Ok(Session {
            tasks: harness_domain::workflow::TaskBoard::restore(state.phases.clone(), &now()),
            state,
            restored: true,
            ..Session::default()
        })
    }
    fn make_room(&mut self) -> Result<(), Failure> {
        if self.entries.len() >= self.limit {
            let index = self
                .entries
                .iter()
                .position(|(_, session)| session.in_flight == 0)
                .ok_or(("SESSION_LIMIT", "todas las sesiones están ocupadas"))?;
            self.entries.remove(index);
        }
        Ok(())
    }
    pub fn resolve_id(&mut self, spec: &str) -> Result<String, Failure> {
        match spec {
            "current" => {
                if let Some(active) = &self.active_id {
                    Ok(active.clone())
                } else if let Some((last, _)) = self.entries.back() {
                    let id = last.clone();
                    self.active_id = Some(id.clone());
                    Ok(id)
                } else {
                    Err((
                        "SESSION",
                        "No active session in this process; call harness_bootstrap first or provide a session ID",
                    ))
                }
            }
            "new" => {
                let (new_id, _) = self.create()?;
                self.active_id = Some(new_id.clone());
                Ok(new_id)
            }
            s if s.starts_with("resume:") => {
                let id = &s["resume:".len()..];
                if !valid_id(id) {
                    return Err((
                        "SESSION",
                        "Identificador de sesión inválido en resume:<uuid>",
                    ));
                }
                if !self.entries.iter().any(|(k, _)| k == id) && self.load(id).is_err() {
                    return Err(("SESSION", "La sesión a reanudar no existe o fue descartada"));
                }
                self.active_id = Some(id.to_string());
                Ok(id.to_string())
            }
            other => {
                if !valid_id(other) {
                    return Err((
                        "SESSION",
                        "Identificador de sesión inválido; use 'current', 'new', 'resume:<uuid>' o un UUID v4",
                    ));
                }
                Ok(other.to_string())
            }
        }
    }
    pub fn acquire(&mut self, id: &str) -> Result<(), Failure> {
        let real_id = self.resolve_id(id)?;
        self.get(&real_id)?.in_flight += 1;
        self.active_id = Some(real_id);
        Ok(())
    }
    pub fn release(&mut self, id: &str) {
        let real_id = self.resolve_id(id).unwrap_or_else(|_| id.to_string());
        if let Some((_, session)) = self.entries.iter_mut().find(|(key, _)| key == &real_id) {
            session.in_flight = session.in_flight.saturating_sub(1);
        }
    }
    pub fn create(&mut self) -> Result<(String, Option<Value>), Failure> {
        self.make_room()?;
        let id = uuid::Uuid::new_v4().to_string();
        let session = Session::default();
        let warning = self
            .save(&id, &session.state)
            .err()
            .map(|(_, message)| json!({"code":"PERSISTENCE","message":message}));
        self.entries.push_back((id.clone(), session));
        self.active_id = Some(id.clone());
        Ok((id, warning))
    }
    pub fn get(&mut self, id: &str) -> Result<&mut Session, Failure> {
        let real_id = self.resolve_id(id)?;
        self.active_id = Some(real_id.clone());
        if let Some(index) = self.entries.iter().position(|(key, _)| key == &real_id) {
            return Ok(&mut self.entries[index].1);
        }
        let session = self.load(&real_id)?;
        self.make_room()?;
        self.entries.push_back((real_id, session));
        Ok(&mut self.entries.back_mut().expect("inserted session").1)
    }
}

pub(super) fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub(super) fn persist_snapshot(
    project: &Project,
    id: &str,
    snapshot: &Value,
    context: &harness_domain::execution::OperationContext,
) -> Result<(), Failure> {
    context.check()?;
    let mut state: State = serde_json::from_value(snapshot.clone())
        .map_err(|_| ("PERSISTENCE", "Invalid session snapshot"))?;
    validate(&mut state)?;
    Sessions::new(project.clone()).save(id, &state)
}

pub(super) fn validate(state: &mut State) -> Result<(), Failure> {
    let valid_strings =
        |items: &[String], max| items.len() <= 20 && items.iter().all(|s| utf16_len(s) <= max);
    if utf16_len(&state.declared.goal) > 1000
        || !valid_strings(&state.declared.decisions, 500)
        || !valid_strings(&state.declared.pending, 500)
        || state.phases.len() != 4
        || state.edits.len() > 50
        || state.checks.len() > 20
    {
        return Err(("SESSION", "estado persistente fuera de límites"));
    }
    let mut phases = Vec::new();
    for (id, title) in IDS.iter().zip(TITLES) {
        let matches: Vec<_> = state.phases.iter().filter(|p| p["id"] == *id).collect();
        if matches.len() != 1 {
            return Err(("SESSION", "fases persistentes inválidas"));
        }
        let phase = matches[0];
        let status = phase["status"]
            .as_str()
            .filter(|s| STATUSES.contains(s))
            .ok_or(("SESSION", "estado de fase inválido"))?;
        let note = match phase.get("note") {
            None => "",
            Some(note) => note.as_str().ok_or(("SESSION", "nota inválida"))?,
        };
        if utf16_len(note) > 300
            || status == "blocked" && harness_domain::workflow::note_is_blank(note)
        {
            return Err(("SESSION", "nota persistente inválida"));
        }
        let mut clean = json!({"id":id,"title":title,"status":status});
        if !note.is_empty() {
            clean["note"] = json!(note);
        }
        phases.push(clean);
    }
    if phases
        .iter()
        .filter(|p| p["status"] == "in_progress")
        .count()
        > 1
    {
        return Err(("SESSION", "múltiples fases activas"));
    }
    state.phases = phases;
    for (items, keys) in [
        (&mut state.edits, &["path", "hash", "time"][..]),
        (&mut state.checks, &["language", "scope", "time"][..]),
    ] {
        for item in items {
            let mut clean = json!({});
            for key in keys {
                let value = item[*key]
                    .as_str()
                    .filter(|s| utf16_len(s) <= 1000)
                    .ok_or(("SESSION", "hechos persistentes inválidos"))?;
                clean[*key] = json!(value);
            }
            if keys.contains(&"language") {
                clean["ok"] = json!(
                    item["ok"]
                        .as_bool()
                        .ok_or(("SESSION", "verificación persistente inválida"))?
                );
            }
            *item = clean;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn active_session_is_not_evicted_and_capacity_recovers_after_release() {
        let dir = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::new(Project::new(dir.path()).unwrap());
        sessions.limit = 1;
        let (first, _) = sessions.create().unwrap();
        sessions.acquire(&first).unwrap();
        assert_eq!(sessions.create().unwrap_err().0, "SESSION_LIMIT");
        assert!(!sessions.get(&first).unwrap().restored);
        sessions.release(&first);
        sessions.create().unwrap();
        assert!(sessions.get(&first).unwrap().restored);
    }
    #[test]
    fn restores_node_v1_and_drops_freshness_and_untrusted_fields() {
        let dir = tempfile::tempdir().unwrap();
        let project = Project::new(dir.path()).unwrap();
        let mut first = Sessions::new(project.clone());
        let (id, warning) = first.create().unwrap();
        assert!(warning.is_none());
        let mut data = Session::default().state;
        data.declared.goal = "Continuar migración".into();
        data.checks = vec![
            json!({"language":"rust","scope":"core","time":"now","ok":true,"current":true,"version":"untrusted"}),
        ];
        first.save(&id, &data).unwrap();
        let mut second = Sessions::new(project);
        let loaded = second.get(&id).unwrap();
        assert!(loaded.restored);
        assert_eq!(loaded.state.declared.goal, "Continuar migración");
        assert!(loaded.state.checks[0].get("current").is_none());
        assert!(loaded.state.checks[0].get("version").is_none());
        assert!(loaded.rules_hash.is_none());
        assert!(second.get("../outside").is_err());
    }
    #[test]
    fn memory_eviction_can_restore_from_disk_and_reject_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::new(Project::new(dir.path()).unwrap());
        sessions.limit = 1;
        let (first, _) = sessions.create().unwrap();
        sessions.create().unwrap();
        assert!(sessions.get(&first).unwrap().restored);
        sessions.entries.clear();
        fs::write(
            dir.path()
                .join(format!(".agents/harness/.state/{first}.json")),
            b"{}",
        )
        .unwrap();
        assert!(sessions.get(&first).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn persistence_does_not_write_through_junctions() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".agents/harness")).unwrap();
        let junction = dir.path().join(".agents").join("harness").join(".state");
        let result = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&junction)
            .arg(outside.path())
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let mut sessions = Sessions::new(Project::new(dir.path()).unwrap());
        let (_, warning) = sessions.create().unwrap();
        assert_eq!(warning.unwrap()["code"], "PERSISTENCE");
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
        fs::remove_dir(junction).unwrap();
    }
}
