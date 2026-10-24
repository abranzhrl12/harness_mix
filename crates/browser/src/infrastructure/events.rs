use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use chromiumoxide::{
    cdp::{
        browser_protocol::{
            log::EventEntryAdded,
            network::{
                EventLoadingFailed, EventLoadingFinished, EventRequestWillBeSent,
                EventResponseReceived,
            },
        },
        js_protocol::runtime::{EventConsoleApiCalled, EventExceptionThrown},
    },
    page::Page,
};
use futures_util::StreamExt;
use harness_domain::browser::{BrowserEvent, BrowserEventRecord};
use tokio::task::JoinHandle;

use super::cdp::MAX_EVENT_STACK;
use crate::domain::{MAX_EVENT_MESSAGE, MAX_EVENTS};

pub(super) struct EventBuffer {
    next: u64,
    dropped: u64,
    entries: VecDeque<BrowserEventRecord>,
}

impl Default for EventBuffer {
    fn default() -> Self {
        Self {
            next: 0,
            dropped: 0,
            entries: VecDeque::with_capacity(MAX_EVENTS),
        }
    }
}

impl EventBuffer {
    pub(super) fn push(&mut self, target_id: &str, event: BrowserEvent) {
        self.next = self.next.saturating_add(1);
        if self.entries.len() == MAX_EVENTS {
            self.entries.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.entries.push_back(BrowserEventRecord {
            sequence: self.next,
            target_id: target_id.to_owned(),
            event,
        });
    }

    pub(super) fn page(
        &self,
        target_id: Option<&str>,
        after: u64,
        limit: usize,
    ) -> (Vec<BrowserEventRecord>, u64, u64) {
        let records: Vec<_> = self
            .entries
            .iter()
            .filter(|record| {
                record.sequence > after && target_id.is_none_or(|id| id == record.target_id)
            })
            .take(limit)
            .cloned()
            .collect();
        let cursor = if records.len() == limit {
            records.last().map_or(after, |record| record.sequence)
        } else {
            self.next
        };
        (records, cursor, self.dropped)
    }
}

pub(super) type EventSink = Arc<Mutex<EventBuffer>>;
pub(super) type RequestUrls = Arc<Mutex<std::collections::HashMap<String, String>>>;

fn request_key(target_id: &str, request_id: &str) -> String {
    format!("{target_id}\0{request_id}")
}

/// Register listeners before enabling their domains, then drain them continuously.
pub(super) async fn subscribe_events(
    page: &Page,
    target_id: &str,
    sink: EventSink,
    request_urls: RequestUrls,
) -> Result<Vec<JoinHandle<()>>, harness_domain::Failure> {
    let mut log = page
        .event_listener::<EventEntryAdded>()
        .await
        .map_err(|_| ("BROWSER_CDP", "No se pudo suscribir el registro de consola"))?;
    let mut console = page
        .event_listener::<EventConsoleApiCalled>()
        .await
        .map_err(|_| {
            (
                "BROWSER_CDP",
                "No se pudo suscribir console.error y console.warn",
            )
        })?;
    let mut exceptions = page
        .event_listener::<EventExceptionThrown>()
        .await
        .map_err(|_| ("BROWSER_CDP", "No se pudo suscribir las excepciones"))?;
    let mut requests = page
        .event_listener::<EventRequestWillBeSent>()
        .await
        .map_err(|_| ("BROWSER_CDP", "No se pudo suscribir solicitudes de red"))?;
    let mut failures = page
        .event_listener::<EventLoadingFailed>()
        .await
        .map_err(|_| ("BROWSER_CDP", "No se pudo suscribir fallos de red"))?;
    let mut finished = page
        .event_listener::<EventLoadingFinished>()
        .await
        .map_err(|_| {
            (
                "BROWSER_CDP",
                "No se pudo suscribir el fin de solicitudes de red",
            )
        })?;
    let mut responses = page
        .event_listener::<EventResponseReceived>()
        .await
        .map_err(|_| ("BROWSER_CDP", "No se pudo suscribir respuestas HTTP"))?;
    page.enable_log()
        .await
        .map_err(|_| ("BROWSER_CDP", "No se pudo activar la consola del navegador"))?;
    page.enable_runtime().await.map_err(|_| {
        (
            "BROWSER_CDP",
            "No se pudo activar la captura de excepciones",
        )
    })?;
    page.execute(chromiumoxide::cdp::browser_protocol::network::EnableParams::default())
        .await
        .map_err(|_| ("BROWSER_CDP", "No se pudo activar la captura de red"))?;

    let target = target_id.to_owned();
    let event_sink = Arc::clone(&sink);
    let log_task = tokio::spawn(async move {
        while let Some(event) = log.next().await {
            let entry = &event.entry;
            let level = format!("{:?}", entry.level).to_ascii_lowercase();
            if level != "error" && level != "warning" {
                continue;
            }
            let source = format!("{:?}", entry.source).to_ascii_lowercase();
            if source.contains("javascript") || source.contains("console") {
                continue;
            }
            let stack = entry.stack_trace.as_ref().map(format_stack);
            push(
                &event_sink,
                &target,
                BrowserEvent::Console {
                    timestamp_ms: now_ms(),
                    level,
                    message: bounded_copy(&entry.text, MAX_EVENT_MESSAGE),
                    url: entry.url.as_deref().map(|value| bounded_copy(value, 1_024)),
                    line: entry
                        .line_number
                        .and_then(|value| u32::try_from(value).ok()),
                    column: None,
                    stack: stack.map(|value| bounded_text(value, MAX_EVENT_STACK)),
                },
            );
        }
    });

    let target = target_id.to_owned();
    let event_sink = Arc::clone(&sink);
    let console_task = tokio::spawn(async move {
        while let Some(event) = console.next().await {
            let kind = format!("{:?}", event.r#type).to_ascii_lowercase();
            if !matches!(kind.as_str(), "error" | "warning" | "assert") {
                continue;
            }
            let message = event
                .args
                .iter()
                .take(16)
                .map(remote_text)
                .collect::<Vec<_>>()
                .join(" ");
            let (url, line, column) = event
                .stack_trace
                .as_ref()
                .and_then(first_frame)
                .map(|(url, line, column)| (Some(url), Some(line), Some(column)))
                .unwrap_or((None, None, None));
            let stack = event.stack_trace.as_ref().map(format_stack);
            push(
                &event_sink,
                &target,
                BrowserEvent::Console {
                    timestamp_ms: now_ms(),
                    level: kind,
                    message: bounded_text(message, MAX_EVENT_MESSAGE),
                    url,
                    line,
                    column,
                    stack: stack.map(|value| bounded_text(value, MAX_EVENT_STACK)),
                },
            );
        }
    });

    let target = target_id.to_owned();
    let event_sink = Arc::clone(&sink);
    let exception_task = tokio::spawn(async move {
        while let Some(event) = exceptions.next().await {
            let details = &event.exception_details;
            let message = details
                .exception
                .as_ref()
                .and_then(|value| value.description.as_deref())
                .unwrap_or(&details.text);
            let stack = details.stack_trace.as_ref().map(format_stack);
            push(
                &event_sink,
                &target,
                BrowserEvent::Exception {
                    timestamp_ms: now_ms(),
                    message: bounded_copy(message, MAX_EVENT_MESSAGE),
                    url: details
                        .url
                        .as_deref()
                        .map(|value| bounded_copy(value, 1_024)),
                    line: u32::try_from(details.line_number).ok(),
                    column: u32::try_from(details.column_number).ok(),
                    stack: stack.map(|value| bounded_text(value, MAX_EVENT_STACK)),
                },
            );
        }
    });

    let target = target_id.to_owned();
    let urls = Arc::clone(&request_urls);
    let request_task = tokio::spawn(async move {
        while let Some(event) = requests.next().await {
            if let Ok(mut urls) = urls.lock() {
                if urls.len() >= MAX_EVENTS * 2
                    && !urls.contains_key(&request_key(&target, &event.request_id.to_string()))
                {
                    if let Some(key) = urls.keys().next().cloned() {
                        urls.remove(&key);
                    }
                }
                urls.insert(
                    request_key(&target, &event.request_id.to_string()),
                    bounded_copy(&event.request.url, 2_048),
                );
            }
        }
    });

    let event_target = target_id.to_owned();
    let event_sink = Arc::clone(&sink);
    let urls = Arc::clone(&request_urls);
    let target = target_id.to_owned();
    let failure_task = tokio::spawn(async move {
        while let Some(event) = failures.next().await {
            let key = request_key(&target, &event.request_id.to_string());
            let url = urls
                .lock()
                .ok()
                .and_then(|mut map| map.remove(&key))
                .unwrap_or_default();
            push(
                &event_sink,
                &event_target,
                BrowserEvent::NetworkFailure {
                    timestamp_ms: now_ms(),
                    url: bounded_text(url, 2_048),
                    error: bounded_copy(&event.error_text, MAX_EVENT_MESSAGE),
                },
            );
        }
    });

    let urls = Arc::clone(&request_urls);
    let target = target_id.to_owned();
    let finished_task = tokio::spawn(async move {
        while let Some(event) = finished.next().await {
            if let Ok(mut urls) = urls.lock() {
                urls.remove(&request_key(&target, &event.request_id.to_string()));
            }
        }
    });

    let target = target_id.to_owned();
    let event_sink = Arc::clone(&sink);
    let response_task = tokio::spawn(async move {
        while let Some(event) = responses.next().await {
            if event.response.status >= 400 {
                push(
                    &event_sink,
                    &target,
                    BrowserEvent::NetworkFailure {
                        timestamp_ms: now_ms(),
                        url: bounded_copy(&event.response.url, 2_048),
                        error: bounded_text(
                            format!(
                                "HTTP {} {}",
                                event.response.status, event.response.status_text
                            ),
                            MAX_EVENT_MESSAGE,
                        ),
                    },
                );
            }
        }
    });

    Ok(vec![
        log_task,
        console_task,
        exception_task,
        request_task,
        failure_task,
        finished_task,
        response_task,
    ])
}

fn push(sink: &EventSink, target_id: &str, event: BrowserEvent) {
    if let Ok(mut sink) = sink.lock() {
        sink.push(target_id, event);
    }
}

fn format_stack(trace: &chromiumoxide::cdp::js_protocol::runtime::StackTrace) -> String {
    let mut stack = trace
        .call_frames
        .iter()
        .take(16)
        .map(|frame| {
            format!(
                "{} ({}:{}:{})",
                bounded_copy(&frame.function_name, 256),
                bounded_copy(&frame.url, 1_024),
                frame.line_number,
                frame.column_number
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    if trace.call_frames.len() > 16 {
        stack.push_str("\n[truncated]");
    }
    stack
}

fn first_frame(
    trace: &chromiumoxide::cdp::js_protocol::runtime::StackTrace,
) -> Option<(String, u32, u32)> {
    trace.call_frames.first().map(|frame| {
        (
            bounded_copy(&frame.url, 1_024),
            u32::try_from(frame.line_number).unwrap_or_default(),
            u32::try_from(frame.column_number).unwrap_or_default(),
        )
    })
}

fn remote_text(value: &chromiumoxide::cdp::js_protocol::runtime::RemoteObject) -> String {
    if let Some(value) = &value.value {
        if let Some(text) = value.as_str() {
            return bounded_copy(text, 512);
        }
        if value.is_number() || value.is_boolean() || value.is_null() {
            return bounded_text(value.to_string(), 512);
        }
    }
    value
        .description
        .as_deref()
        .map(|text| bounded_copy(text, 512))
        .unwrap_or_else(|| format!("{:?}", value.r#type))
}

pub(super) fn bounded_text(text: String, max: usize) -> String {
    if text.len() <= max {
        return text;
    }
    bounded_copy(&text, max)
}

fn bounded_copy(text: &str, max: usize) -> String {
    const SUFFIX: &str = "[truncated]";
    if text.len() <= max {
        return text.to_owned();
    }
    if max <= SUFFIX.len() {
        let mut end = text.len().min(max);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        return text[..end].to_owned();
    }
    let mut end = text.len().min(max - SUFFIX.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &text[..end], SUFFIX)
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as f64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{EventBuffer, MAX_EVENTS, bounded_text, request_key};
    use harness_domain::browser::BrowserEvent;

    #[test]
    fn event_pages_advance_only_through_returned_events_when_limited() {
        let mut buffer = EventBuffer::default();
        for _ in 0..4 {
            buffer.push(
                "target",
                BrowserEvent::Exception {
                    timestamp_ms: 1.0,
                    message: "err".into(),
                    url: None,
                    line: None,
                    column: None,
                    stack: None,
                },
            );
        }
        let (page, cursor, _) = buffer.page(None, 0, 2);
        assert_eq!(page.len(), 2);
        assert_eq!(cursor, 2);
        let (page, cursor, _) = buffer.page(None, cursor, 2);
        assert_eq!(page.len(), 2);
        assert_eq!(cursor, 4);
    }

    #[test]
    fn truncation_keeps_utf8_boundaries() {
        assert_eq!(bounded_text("a🙂z".into(), 4), "a");
    }

    #[test]
    fn event_buffer_bounds_memory_and_reports_drops() {
        let mut buffer = EventBuffer::default();
        for _ in 0..=MAX_EVENTS {
            buffer.push(
                "target",
                BrowserEvent::Exception {
                    timestamp_ms: 1.0,
                    message: "err".into(),
                    url: None,
                    line: None,
                    column: None,
                    stack: None,
                },
            );
        }
        let (page, cursor, dropped) = buffer.page(None, 0, MAX_EVENTS);
        assert_eq!(page.len(), MAX_EVENTS);
        assert_eq!(page[0].sequence, 2);
        assert_eq!(cursor, MAX_EVENTS as u64 + 1);
        assert_eq!(dropped, 1);
    }

    #[test]
    fn filtered_event_pages_preserve_sequence_and_request_keys_include_target() {
        let mut buffer = EventBuffer::default();
        for target in ["one", "two", "one"] {
            buffer.push(
                target,
                BrowserEvent::Exception {
                    timestamp_ms: 1.0,
                    message: "err".into(),
                    url: None,
                    line: None,
                    column: None,
                    stack: None,
                },
            );
        }
        let (first, cursor, _) = buffer.page(Some("one"), 0, 1);
        assert_eq!(first[0].sequence, 1);
        assert_eq!(cursor, 1);
        let (second, cursor, _) = buffer.page(Some("one"), cursor, 2);
        assert_eq!(second[0].sequence, 3);
        assert_eq!(cursor, 3);
        assert_ne!(request_key("one", "42"), request_key("two", "42"));
    }
}
