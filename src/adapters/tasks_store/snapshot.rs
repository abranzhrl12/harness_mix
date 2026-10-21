use super::{FORMAT, Failure, MAX_SNAPSHOT};
use serde_json::Value;

pub(super) fn validate(snapshot: &Value) -> Result<(), Failure> {
    let object = snapshot.as_object().ok_or(FORMAT)?;
    if object.len() != 3 {
        return Err(FORMAT);
    }
    let tasks = snapshot["tasks"].as_array().ok_or(FORMAT)?;
    let events = snapshot["events"].as_array().ok_or(FORMAT)?;
    let cursor = snapshot["cursor"]
        .as_u64()
        .filter(|value| *value < i64::MAX as u64)
        .ok_or(FORMAT)?;
    if tasks.len() > 64 || events.len() > 256 || snapshot.to_string().len() > MAX_SNAPSHOT {
        return Err(FORMAT);
    }
    if tasks
        .iter()
        .any(|value| !value.is_object() || value.to_string().len() > 128 * 1024)
    {
        return Err(FORMAT);
    }
    let mut previous = 0;
    for event in events {
        let sequence = event["cursor"].as_u64().ok_or(FORMAT)?;
        if sequence <= previous || sequence > cursor || event.to_string().len() > 128 * 1024 {
            return Err(FORMAT);
        }
        previous = sequence;
    }
    Ok(())
}
