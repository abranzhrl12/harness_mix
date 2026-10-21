use super::model::State;
use harness_domain::{
    Failure,
    results::utf16_len,
    workflow::{IDS, STATUSES, TITLES},
};
use serde_json::json;

pub(super) fn valid_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id)
}

pub(in crate::adapters) fn validate(state: &mut State) -> Result<(), Failure> {
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
