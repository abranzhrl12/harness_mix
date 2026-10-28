use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use chromiumoxide::{
    browser::Browser,
    cdp::browser_protocol::target::{GetTargetsParams, TargetId, TargetInfo},
};
use futures_util::StreamExt;
use std::time::Duration;
use uuid::Uuid;

use harness_domain::{
    Failure,
    browser::{BrowserArgs, BrowserResult, BrowserSelectedTab, BrowserTarget},
    execution::OperationContext,
};

use crate::{
    domain::{
        MAX_EVENT_MESSAGE, is_localhost_url, rank_targets, required_session, select_target,
        validate_connect_endpoint,
    },
    infrastructure::{
        cdp::{MAX_ATTACHED_PAGES, MAX_CONNECTIONS, MAX_TARGETS},
        connection::{BrowserConnection, owned_connection, owned_connection_mut, owned_page},
        discovery, events, inspect,
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
        BrowserArgs::Discover {
            endpoint,
            tab,
            session,
        } => {
            let _owner = required_session(session)?;
            if let Some(endpoint) = endpoint.as_deref() {
                validate_connect_endpoint(endpoint)?;
            }
            let discovered =
                discovery::discover_session(endpoint.as_deref(), tab.as_deref()).await?;
            let selected_tab = select_target(
                &discovered.targets,
                discovered.effective_tab_filter.as_deref(),
            )
            .map(BrowserSelectedTab::from);
            Ok(BrowserResult::Discovered {
                endpoint: discovered.ws_endpoint,
                selected_tab,
                available_tabs: discovered.targets,
            })
        }
        BrowserArgs::Connect {
            endpoint,
            tab,
            session,
        } => {
            let owner = required_session(session)?;
            validate_connect_endpoint(&endpoint)?;
            if !sessions.contains_key(&owner) && connections.len() >= MAX_CONNECTIONS {
                return Err((
                    "BROWSER_LIMIT",
                    "Se alcanzó el límite de conexiones de navegador",
                ));
            }
            let discovered = discovery::discover_session(Some(&endpoint), tab.as_deref()).await?;
            let ws_endpoint = discovered.ws_endpoint.clone();
            let (browser, mut handler) = Browser::connect(ws_endpoint.clone()).await.map_err(|_| {
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
            let mut connection = BrowserConnection {
                owner: owner.clone(),
                browser,
                pages: HashMap::new(),
                events: Arc::new(Mutex::new(events::EventBuffer::default())),
                request_urls: Arc::new(Mutex::new(HashMap::new())),
                tasks: vec![handler_task],
            };

            let mut available_tabs = match fetch_target_infos(&connection.browser).await {
                Ok(raw_targets) => {
                    map_targets(raw_targets, discovered.effective_tab_filter.as_deref())
                        .unwrap_or_else(|_| discovered.targets.clone())
                }
                Err(_) => discovered.targets.clone(),
            };
            if available_tabs.is_empty() && !discovered.targets.is_empty() {
                available_tabs = discovered.targets;
            }

            let candidate_tab = select_target(
                &available_tabs,
                discovered.effective_tab_filter.as_deref(),
            )
            .cloned();

            let mut selected_tab = None;
            if let Some(target) = candidate_tab {
                if attach_target_to_connection(&mut connection, &target.id)
                    .await
                    .is_ok()
                {
                    selected_tab = Some(BrowserSelectedTab::from(&target));
                }
            }

            if let Some(old_id) = sessions.remove(&owner) {
                connections.remove(&old_id);
            }
            let id = Uuid::new_v4().to_string();
            connections.insert(id.clone(), connection);
            sessions.insert(owner, id.clone());
            Ok(BrowserResult::Connected {
                connection_id: id,
                endpoint: Some(ws_endpoint),
                selected_tab,
                available_tabs,
            })
        }
        BrowserArgs::Targets {
            connection_id,
            tab,
            session,
        } => {
            let owner = required_session(session)?;
            let connection = owned_connection_mut(connections, &connection_id, &owner)?;
            let targets = fetch_target_infos(&connection.browser)
                .await
                .map_err(|_| ("BROWSER_CDP", "No se pudieron listar las pestañas"))?;
            let mapped = map_targets(targets, tab.as_deref())?;
            let selected_tab = select_target(&mapped, tab.as_deref()).map(BrowserSelectedTab::from);
            Ok(BrowserResult::Targets {
                available_tabs: mapped.clone(),
                targets: mapped,
                selected_tab,
            })
        }
        BrowserArgs::Attach {
            connection_id,
            target_id,
            session,
        } => {
            let owner = required_session(session)?;
            let connection = owned_connection_mut(connections, &connection_id, &owner)?;
            let raw_targets = fetch_target_infos(&connection.browser).await.map_err(|_| {
                (
                    "BROWSER_CDP",
                    "No se pudieron descubrir las pestañas existentes",
                )
            })?;
            let available_tabs = map_targets(raw_targets, Some(&target_id)).unwrap_or_default();

            let resolved_target = if available_tabs.iter().any(|t| t.id == target_id) {
                available_tabs.iter().find(|t| t.id == target_id).cloned()
            } else {
                select_target(&available_tabs, Some(&target_id)).cloned()
            };

            let actual_target_id = resolved_target
                .as_ref()
                .map(|t| t.id.clone())
                .unwrap_or_else(|| target_id.clone());

            attach_target_to_connection(connection, &actual_target_id).await?;
            let selected_tab = resolved_target.as_ref().map(BrowserSelectedTab::from);

            Ok(BrowserResult::Attached {
                target_id: actual_target_id,
                selected_tab,
                available_tabs,
            })
        }
        BrowserArgs::Inspect {
            connection_id,
            target_id,
            selector,
            max_chars,
            session,
        } => {
            let owner = required_session(session)?;
            let actual_target_id = {
                let connection = owned_connection_mut(connections, &connection_id, &owner)?;
                ensure_target_attached(connection, &target_id).await?
            };
            let page = owned_page(connections, &connection_id, &actual_target_id, &owner)?;
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
            let resolved_target_id = if let Some(tid) = target_id.as_deref() {
                let connection = owned_connection_mut(connections, &connection_id, &owner)?;
                Some(ensure_target_attached(connection, tid).await?)
            } else {
                None
            };
            let connection = owned_connection(connections, &connection_id, &owner)?;
            if connection.pages.is_empty()
                || resolved_target_id
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
            let (records, next_cursor, dropped) =
                events.page(resolved_target_id.as_deref(), after, limit);
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
            let actual_target_id = {
                let connection = owned_connection_mut(connections, &connection_id, &owner)?;
                ensure_target_attached(connection, &target_id).await?
            };
            let page = owned_page(connections, &connection_id, &actual_target_id, &owner)?;
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

async fn fetch_target_infos(browser: &Browser) -> Result<Vec<TargetInfo>, chromiumoxide::error::CdpError> {
    let response = browser.execute(GetTargetsParams { filter: None }).await?;
    Ok(response.result.target_infos)
}

async fn ensure_target_attached(
    connection: &mut BrowserConnection,
    target_id_or_filter: &str,
) -> Result<String, Failure> {
    if connection.pages.contains_key(target_id_or_filter) {
        return Ok(target_id_or_filter.to_owned());
    }
    let actual_target_id = if let Ok(raw_targets) = fetch_target_infos(&connection.browser).await
        && let Ok(available_tabs) = map_targets(raw_targets, Some(target_id_or_filter))
    {
        if available_tabs.iter().any(|t| t.id == target_id_or_filter) {
            target_id_or_filter.to_owned()
        } else {
            select_target(&available_tabs, Some(target_id_or_filter))
                .map(|t| t.id.clone())
                .unwrap_or_else(|| target_id_or_filter.to_owned())
        }
    } else {
        target_id_or_filter.to_owned()
    };
    attach_target_to_connection(connection, &actual_target_id).await?;
    Ok(actual_target_id)
}

async fn attach_target_to_connection(
    connection: &mut BrowserConnection,
    target_id: &str,
) -> Result<(), Failure> {
    if !connection.pages.contains_key(target_id) {
        if connection.pages.len() >= MAX_ATTACHED_PAGES {
            return Err(("BROWSER_LIMIT", "Se alcanzó el límite de pestañas adjuntas"));
        }
        let tid = TargetId::from(target_id.to_owned());
        let mut page_opt = None;
        for _ in 0..40 {
            if let Ok(found) = connection.browser.get_page(tid.clone()).await {
                page_opt = Some(found);
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let page = page_opt
            .ok_or(("BROWSER_TARGET", "La pestaña no existe o aún no está lista"))?;
        let tasks = events::subscribe_events(
            &page,
            target_id,
            Arc::clone(&connection.events),
            Arc::clone(&connection.request_urls),
        )
        .await?;
        connection.tasks.extend(tasks);
        connection.pages.insert(target_id.to_owned(), page);
    }
    Ok(())
}

fn map_targets(
    targets: Vec<TargetInfo>,
    filter: Option<&str>,
) -> Result<Vec<BrowserTarget>, Failure> {
    let mut pages = Vec::with_capacity(MAX_TARGETS);
    for target in targets.into_iter().filter(|target| target.r#type == "page") {
        if pages.len() == MAX_TARGETS {
            return Err((
                "BROWSER_LIMIT",
                "La lista de pestañas excede el límite de 128; no se truncó",
            ));
        }
        let id = target.target_id.as_ref().to_owned();
        let title = events::bounded_text(target.title, MAX_EVENT_MESSAGE);
        let url = events::bounded_text(target.url, MAX_EVENT_MESSAGE);
        let is_localhost = is_localhost_url(&url);
        pages.push(BrowserTarget::new(
            id,
            title,
            url,
            target.r#type,
            is_localhost,
        ));
    }
    rank_targets(&mut pages, filter);
    Ok(pages)
}
