use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use chromiumoxide::{
    browser::Browser,
    cdp::browser_protocol::target::{TargetId, TargetInfo},
};
use futures_util::StreamExt;
use uuid::Uuid;

use harness_domain::{
    Failure,
    browser::{BrowserArgs, BrowserResult, BrowserTarget},
    execution::OperationContext,
};

use crate::{
    domain::{MAX_EVENT_MESSAGE, required_session, validate_connect_endpoint},
    infrastructure::{
        cdp::{MAX_ATTACHED_PAGES, MAX_CONNECTIONS, MAX_TARGETS},
        discovery,
        connection::{BrowserConnection, owned_connection, owned_connection_mut, owned_page},
        events, inspect,
    },
};

pub(super) async fn dispatch(
    args: BrowserArgs,
    context: OperationContext,
    connections: &mut HashMap<String, BrowserConnection>,
    sessions: &mut HashMap<String, String>,
) -> Result<BrowserResult, Failure> {
    context.check()?;
    match args {
        BrowserArgs::Connect { endpoint, session } => {
            let owner = required_session(session)?;
            validate_connect_endpoint(&endpoint)?;
            if sessions.contains_key(&owner) {
                return Err((
                    "BROWSER_SESSION_EXISTS",
                    "Esta sesión ya tiene una conexión de navegador",
                ));
            }
            if connections.len() >= MAX_CONNECTIONS {
                return Err((
                    "BROWSER_LIMIT",
                    "Se alcanzó el límite de conexiones de navegador",
                ));
            }
            let endpoint = discovery::resolve_endpoint(&endpoint).await?;
            let (browser, mut handler) = Browser::connect(endpoint).await.map_err(|_| {
                (
                    "BROWSER_CONNECT",
                    "No se pudo conectar al endpoint CDP local",
                )
            })?;
            let handler_task = tokio::spawn(async move {
                while let Some(message) = handler.next().await {
                    if message.is_err() {
                        break;
                    }
                }
            });
            let id = Uuid::new_v4().to_string();
            connections.insert(
                id.clone(),
                BrowserConnection {
                    owner: owner.clone(),
                    browser,
                    pages: HashMap::new(),
                    events: Arc::new(Mutex::new(events::EventBuffer::default())),
                    request_urls: Arc::new(Mutex::new(HashMap::new())),
                    tasks: vec![handler_task],
                },
            );
            sessions.insert(owner, id.clone());
            Ok(BrowserResult::Connected { connection_id: id })
        }
        BrowserArgs::Targets {
            connection_id,
            session,
        } => {
            let owner = required_session(session)?;
            let connection = owned_connection_mut(connections, &connection_id, &owner)?;
            let targets = connection
                .browser
                .fetch_targets()
                .await
                .map_err(|_| ("BROWSER_CDP", "No se pudieron listar las pestañas"))?;
            Ok(BrowserResult::Targets {
                targets: map_targets(targets)?,
            })
        }
        BrowserArgs::Attach {
            connection_id,
            target_id,
            session,
        } => {
            let owner = required_session(session)?;
            let connection = owned_connection_mut(connections, &connection_id, &owner)?;
            if !connection.pages.contains_key(&target_id) {
                if connection.pages.len() >= MAX_ATTACHED_PAGES {
                    return Err(("BROWSER_LIMIT", "Se alcanzó el límite de pestañas adjuntas"));
                }
                connection.browser.fetch_targets().await.map_err(|_| {
                    (
                        "BROWSER_CDP",
                        "No se pudieron descubrir las pestañas existentes",
                    )
                })?;
                let page = connection
                    .browser
                    .get_page(TargetId::from(target_id.clone()))
                    .await
                    .map_err(|_| ("BROWSER_TARGET", "La pestaña no existe o aún no está lista"))?;
                let tasks = events::subscribe_events(
                    &page,
                    &target_id,
                    Arc::clone(&connection.events),
                    Arc::clone(&connection.request_urls),
                )
                .await?;
                connection.tasks.extend(tasks);
                connection.pages.insert(target_id.clone(), page);
            }
            Ok(BrowserResult::Attached { target_id })
        }
        BrowserArgs::Inspect {
            connection_id,
            target_id,
            selector,
            max_chars,
            session,
        } => {
            let owner = required_session(session)?;
            let page = owned_page(connections, &connection_id, &target_id, &owner)?;
            let limit = inspect::html_limit(max_chars);
            let snapshot = inspect::inspect_page(page, selector.as_deref(), limit).await?;
            Ok(BrowserResult::Inspected { snapshot })
        }
        BrowserArgs::Errors {
            connection_id,
            target_id,
            after,
            limit,
            session,
        } => {
            let owner = required_session(session)?;
            let connection = owned_connection(connections, &connection_id, &owner)?;
            if connection.pages.is_empty()
                || target_id
                    .as_ref()
                    .is_some_and(|id| !connection.pages.contains_key(id))
            {
                return Err((
                    "BROWSER_TARGET",
                    "Primero adjunta la pestaña con la acción attach",
                ));
            }
            let after = after.unwrap_or(0);
            let limit = limit.unwrap_or(100) as usize;
            let events = connection
                .events
                .lock()
                .map_err(|_| ("BROWSER_RUNTIME", "No se pudo leer el búfer de eventos"))?;
            let (records, next_cursor, dropped) = events.page(target_id.as_deref(), after, limit);
            Ok(BrowserResult::Errors {
                events: records,
                next_cursor,
                dropped,
            })
        }
        BrowserArgs::Screenshot {
            connection_id,
            target_id,
            selector,
            session,
        } => {
            let owner = required_session(session)?;
            let page = owned_page(connections, &connection_id, &target_id, &owner)?;
            let (bytes, width, height) =
                inspect::capture_screenshot(page, selector.as_deref()).await?;
            Ok(BrowserResult::Screenshot {
                mime_type: "image/png".into(),
                data_base64: STANDARD.encode(bytes),
                width,
                height,
            })
        }
        BrowserArgs::Disconnect {
            connection_id,
            session,
        } => {
            let owner = required_session(session)?;
            owned_connection(connections, &connection_id, &owner)?;
            connections.remove(&connection_id);
            sessions.remove(&owner);
            Ok(BrowserResult::Disconnected { connection_id })
        }
    }
}

fn map_targets(targets: Vec<TargetInfo>) -> Result<Vec<BrowserTarget>, Failure> {
    let mut pages = Vec::with_capacity(MAX_TARGETS);
    for target in targets.into_iter().filter(|target| target.r#type == "page") {
        if pages.len() == MAX_TARGETS {
            return Err((
                "BROWSER_LIMIT",
                "La lista de pestañas excede el límite de 128; no se truncó",
            ));
        }
        pages.push(BrowserTarget {
            id: target.target_id.as_ref().to_owned(),
            title: events::bounded_text(target.title, MAX_EVENT_MESSAGE),
            url: events::bounded_text(target.url, MAX_EVENT_MESSAGE),
            target_type: target.r#type,
        });
    }
    Ok(pages)
}
