use super::{
    model::{Session, State},
    persistence,
    validation::valid_id,
};
use crate::adapters::project::Project;
use harness_domain::Failure;
use serde_json::{Value, json};
use std::collections::VecDeque;

pub struct Sessions {
    project: Project,
    pub(super) entries: VecDeque<(String, Session)>,
    pub(super) limit: usize,
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

    pub(super) fn save(&self, id: &str, state: &State) -> Result<(), Failure> {
        persistence::save(&self.project, id, state)
    }

    fn load(&self, id: &str) -> Result<Session, Failure> {
        persistence::load(&self.project, id)
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

pub(in crate::adapters) fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
