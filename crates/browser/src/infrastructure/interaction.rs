use chromiumoxide::{
    cdp::js_protocol::runtime::{CallArgument, CallFunctionOnParams},
    page::Page,
};
use harness_domain::{
    Failure,
    browser::{BrowserInteractiveElement, BrowserWaitState},
    execution::OperationContext,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;

async fn evaluate(page: &Page, function: &str, args: Vec<Value>) -> Result<Value, Failure> {
    let mut builder = CallFunctionOnParams::builder()
        .function_declaration(function)
        .return_by_value(true);
    for arg in args {
        builder = builder.argument(CallArgument::builder().value(arg).build());
    }
    let params = builder
        .build()
        .map_err(|_| ("BROWSER_CDP", "No se pudo preparar la acción"))?;
    page.evaluate_function(params)
        .await
        .map_err(|_| ("BROWSER_CDP", "No se pudo ejecutar la acción"))?
        .into_value()
        .map_err(|_| ("BROWSER_CDP", "La página devolvió un resultado inválido"))
}

pub(super) async fn click(page: &Page, selector: &str) -> Result<(), Failure> {
    let result = evaluate(page, r#"(s) => { try { const e = document.querySelector(s); if (!e) return 'missing'; if (e.disabled) return 'disabled'; e.click(); return 'ok'; } catch (_) { return 'invalid'; } }"#, vec![json!(selector)]).await?;
    match result.as_str() {
        Some("ok") => Ok(()),
        Some("missing") => Err(("BROWSER_SELECTOR", "No se encontró el elemento")),
        Some("disabled") => Err(("BROWSER_ELEMENT", "El elemento está deshabilitado")),
        _ => Err(("BROWSER_SELECTOR", "Selector o elemento inválido")),
    }
}

pub(super) async fn fill(page: &Page, selector: &str, value: &str) -> Result<(), Failure> {
    let result = evaluate(page, r#"(s, value) => { try { const e = document.querySelector(s); if (!e) return 'missing'; if (e.disabled || e.readOnly) return 'disabled'; if (e instanceof HTMLInputElement || e instanceof HTMLTextAreaElement) { if (e instanceof HTMLInputElement && !['text','search','url','tel','email','password','number'].includes(e.type)) return 'invalid'; const proto = e instanceof HTMLInputElement ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype; Object.getOwnPropertyDescriptor(proto, 'value').set.call(e, value); } else if (e.isContentEditable) { e.textContent = value; } else return 'invalid'; e.dispatchEvent(new Event('input', {bubbles:true})); e.dispatchEvent(new Event('change', {bubbles:true})); return 'ok'; } catch (_) { return 'invalid'; } }"#, vec![json!(selector), json!(value)]).await?;
    match result.as_str() {
        Some("ok") => Ok(()),
        Some("missing") => Err(("BROWSER_SELECTOR", "No se encontró el elemento")),
        Some("disabled") => Err(("BROWSER_ELEMENT", "El elemento no se puede editar")),
        _ => Err(("BROWSER_ELEMENT", "Selector o campo no editable")),
    }
}

pub(super) async fn wait(
    page: &Page,
    selector: &str,
    state: Option<BrowserWaitState>,
    timeout_ms: Option<u32>,
    context: &OperationContext,
) -> Result<(), Failure> {
    let state = match state.unwrap_or(BrowserWaitState::Visible) {
        BrowserWaitState::Present => "present",
        BrowserWaitState::Visible => "visible",
        BrowserWaitState::Hidden => "hidden",
    };
    let deadline =
        tokio::time::Instant::now() + Duration::from_millis(u64::from(timeout_ms.unwrap_or(5000)));
    loop {
        context.check()?;
        let result = tokio::time::timeout_at(deadline, evaluate(page, r#"(s, state) => { try { const e = document.querySelector(s); const visible = !!e && !!(e.getClientRects().length) && getComputedStyle(e).visibility !== 'hidden' && getComputedStyle(e).display !== 'none'; return state === 'present' ? !!e : state === 'visible' ? visible : !visible; } catch (_) { return null; } }"#, vec![json!(selector), json!(state)])).await.map_err(|_| ("BROWSER_TIMEOUT", "Tiempo de espera agotado"))??;
        if result.is_null() {
            return Err(("BROWSER_SELECTOR", "Selector inválido"));
        }
        if result == json!(true) {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err((
                "BROWSER_TIMEOUT",
                "Se agotó el tiempo de espera del selector",
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[derive(Deserialize)]
struct Snapshot {
    elements: Vec<BrowserInteractiveElement>,
    truncated: bool,
}
pub(super) async fn snapshot(
    page: &Page,
    limit: u32,
) -> Result<(Vec<BrowserInteractiveElement>, bool), Failure> {
    let result = evaluate(page, r#"(limit) => {
      const nodes = document.querySelectorAll('a[href],button,input,select,textarea,[role="button"],[role="link"],[role="checkbox"],[role="textbox"],[tabindex],[contenteditable="true"]');
      let truncated = nodes.length > limit;
      const cut = value => { value = String(value || '').replace(/\s+/g, ' ').trim(); if (value.length > 200) truncated = true; return Array.from(value).slice(0, 200).join(''); };
      const path = e => { const parts = []; while (e && e.nodeType === 1) { let n = 1, p = e; while ((p = p.previousElementSibling)) if (p.tagName === e.tagName) n++; parts.unshift(e.tagName.toLowerCase() + ':nth-of-type(' + n + ')'); e = e.parentElement; } return parts.join(' > '); };
      const elements = Array.from(nodes).slice(0, limit).map(e => { const tag = e.tagName.toLowerCase(); const labels = e.labels ? Array.from(e.labels).map(l => l.innerText).join(' ') : ''; return {selector:path(e), tag, role:cut(e.getAttribute('role') || (tag === 'a' ? 'link' : tag === 'button' ? 'button' : tag === 'input' ? ({checkbox:'checkbox',radio:'radio',button:'button',submit:'button',reset:'button',range:'slider'}[e.type] || 'textbox') : tag)), name:cut(e.getAttribute('aria-label') || (e.getAttribute('aria-labelledby') || '').split(/\s+/).map(id => document.getElementById(id)?.innerText || '').join(' ') || labels || e.getAttribute('title') || e.getAttribute('placeholder') || e.innerText), visible:!!e.getClientRects().length && getComputedStyle(e).visibility !== 'hidden', disabled:!!e.disabled || e.getAttribute('aria-disabled') === 'true'}; });
      const bounded = elements.filter(e => { if (e.selector.length > 4096) { truncated = true; return false; } return true; });
      return {elements:bounded, truncated};
    }"#, vec![json!(limit)]).await?;
    let wire: Snapshot =
        serde_json::from_value(result).map_err(|_| ("BROWSER_CDP", "Snapshot inválido"))?;
    Ok((wire.elements, wire.truncated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chromiumoxide::{Browser, BrowserConfig};
    use futures_util::StreamExt;

    #[tokio::test]
    #[ignore = "requires a local Chromium installation"]
    async fn interactions_change_real_dom_and_snapshot_is_bounded() {
        let mut builder = BrowserConfig::builder();
        if let Some(path) = std::env::var_os("HARNESS_CHROMIUM_EXECUTABLE") { builder = builder.chrome_executable(path); }
        let config = builder.build().unwrap();
        let (mut browser, mut handler) = Browser::launch(config).await.unwrap();
        let task = tokio::spawn(async move { while handler.next().await.is_some() {} });
        let page = browser.new_page("about:blank").await.unwrap();
        page.evaluate(r#"document.body.innerHTML = '<label for="name">Name</label><input id="name"><button id="go">Go</button><div id="out"></div>'; document.querySelector('#name').addEventListener('input', e => document.querySelector('#out').textContent = e.target.value); document.querySelector('#go').addEventListener('click', () => document.querySelector('#out').textContent += '!');"#).await.unwrap();
        fill(&page, "#name", "Ada").await.unwrap();
        click(&page, "#go").await.unwrap();
        let text: String = page.evaluate("document.querySelector('#out').textContent").await.unwrap().into_value().unwrap();
        assert_eq!(text, "Ada!");
        let (elements, truncated) = snapshot(&page, 1).await.unwrap();
        assert!(truncated);
        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0].name, "Name");
        assert_eq!(evaluate(&page, "s => !!document.querySelector(s)", vec![json!(elements[0].selector)]).await.unwrap(), json!(true));
        let context = OperationContext::without_deadline();
        assert!(wait(&page, "#name", Some(BrowserWaitState::Visible), Some(500), &context).await.is_ok());
        assert_eq!(wait(&page, "#missing", Some(BrowserWaitState::Present), Some(100), &context).await.unwrap_err().0, "BROWSER_TIMEOUT");
        context.cancel();
        assert_eq!(wait(&page, "#missing", None, Some(500), &context).await.unwrap_err().0, "CANCELLED");
        browser.close().await.unwrap();
        task.abort();
    }
}
