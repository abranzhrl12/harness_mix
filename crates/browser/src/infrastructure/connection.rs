use std::collections::HashMap;

use chromiumoxide::{browser::Browser, page::Page};
use tokio::task::JoinHandle;

use super::events;

pub(super) struct BrowserConnection {
    pub(super) owner: String,
    pub(super) browser: Browser,
    pub(super) launched: bool,
    pub(super) pages: HashMap<String, Page>,
    pub(super) events: events::EventSink,
    pub(super) request_urls: events::RequestUrls,
    pub(super) tasks: Vec<JoinHandle<()>>,
}

impl Drop for BrowserConnection {
    fn drop(&mut self) {
        for task in self.tasks.drain(..) {
            task.abort();
        }
        // External browsers are never closed. Owned browsers are closed explicitly
        // on disconnect/replacement; Chromiumoxide owns their process lifetime.
    }
}

pub(super) fn owned_connection<'a>(
    connections: &'a HashMap<String, BrowserConnection>,
    id: &str,
    owner: &str,
) -> Result<&'a BrowserConnection, harness_domain::Failure> {
    let connection = connections
        .get(id)
        .ok_or(("BROWSER_CONNECTION", "La conexión no existe"))?;
    crate::domain::validate_session_owner(&connection.owner, owner)?;
    Ok(connection)
}

pub(super) fn owned_connection_mut<'a>(
    connections: &'a mut HashMap<String, BrowserConnection>,
    id: &str,
    owner: &str,
) -> Result<&'a mut BrowserConnection, harness_domain::Failure> {
    let connection = connections
        .get_mut(id)
        .ok_or(("BROWSER_CONNECTION", "La conexión no existe"))?;
    crate::domain::validate_session_owner(&connection.owner, owner)?;
    Ok(connection)
}

pub(super) fn owned_page<'a>(
    connections: &'a HashMap<String, BrowserConnection>,
    connection_id: &str,
    target_id: &str,
    owner: &str,
) -> Result<&'a Page, harness_domain::Failure> {
    owned_connection(connections, connection_id, owner)?
        .pages
        .get(target_id)
        .ok_or((
            "BROWSER_TARGET",
            "Primero adjunta la pestaña con la acción attach",
        ))
}
