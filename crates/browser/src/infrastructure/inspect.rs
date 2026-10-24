use chromiumoxide::{
    cdp::{
        browser_protocol::page::{CaptureScreenshotFormat, Viewport},
        js_protocol::runtime::{CallArgument, CallFunctionOnParams},
    },
    page::{Page, ScreenshotParams},
};
use harness_domain::browser::{BrowserElementSnapshot, BrowserRect};
use serde::Deserialize;
use serde_json::json;

use harness_domain::Failure;

use super::cdp::{DEFAULT_HTML_CHARS, MAX_SCREENSHOT_BYTES, MAX_SCREENSHOT_DIMENSION};
use crate::domain::MAX_HTML_CHARS;

#[derive(Deserialize)]
struct SnapshotWire {
    tag: String,
    html: Option<String>,
    text: Option<String>,
    x: Option<f64>,
    y: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
    style: Option<std::collections::BTreeMap<String, String>>,
    truncated: bool,
}

pub(super) async fn inspect_page(
    page: &Page,
    selector: Option<&str>,
    max_chars: usize,
) -> Result<BrowserElementSnapshot, Failure> {
    let function = r#"(selector, maxChars) => {
      const node = selector ? document.querySelector(selector) : document.documentElement;
      if (!node) return null;
      const rect = node.getBoundingClientRect();
      const html = node.outerHTML || '';
      const text = node.innerText || node.textContent || '';
      const style = getComputedStyle(node);
      const keys = ['display','position','boxSizing','color','backgroundColor','fontFamily','fontSize','fontWeight','lineHeight','margin','padding','overflow'];
      const cut = (value, limit) => {
        let result = value.slice(0, limit);
        const last = result.charCodeAt(result.length - 1);
        if (result.length < value.length && last >= 0xD800 && last <= 0xDBFF) result = result.slice(0, -1);
        return result;
      };
      const computed = {};
      let styleTruncated = false;
      for (const key of keys) {
        const value = style[key] || '';
        computed[key] = cut(value, 1024);
        styleTruncated = styleTruncated || value.length > 1024;
      }
      return { tag: node.tagName.toLowerCase(), html: cut(html, maxChars), text: cut(text, maxChars), x: rect.x, y: rect.y, width: rect.width, height: rect.height, style: computed, truncated: html.length > maxChars || text.length > maxChars || styleTruncated };
    }"#;
    let params = CallFunctionOnParams::builder()
        .function_declaration(function)
        .argument(CallArgument::builder().value(json!(selector)).build())
        .argument(CallArgument::builder().value(json!(max_chars)).build())
        .return_by_value(true)
        .build()
        .map_err(|_| ("BROWSER_CDP", "No se pudo preparar la inspección"))?;
    let result = page
        .evaluate_function(params)
        .await
        .map_err(|_| ("BROWSER_CDP", "No se pudo inspeccionar la página"))?;
    let wire: Option<SnapshotWire> = result
        .into_value()
        .map_err(|_| ("BROWSER_CDP", "La página devolvió un snapshot no válido"))?;
    let wire = wire.ok_or((
        "BROWSER_SELECTOR",
        "No se encontró el elemento del selector",
    ))?;
    let bounds = match (wire.x, wire.y, wire.width, wire.height) {
        (Some(x), Some(y), Some(width), Some(height)) => Some(BrowserRect {
            x,
            y,
            width,
            height,
        }),
        _ => None,
    };
    Ok(BrowserElementSnapshot {
        tag: super::events::bounded_text(wire.tag, 64),
        outer_html: wire.html,
        text: wire.text,
        bounds,
        computed_style: wire.style,
        truncated: wire.truncated,
    })
}

pub(super) async fn capture_screenshot(
    page: &Page,
    selector: Option<&str>,
) -> Result<(Vec<u8>, u32, u32), Failure> {
    let bytes = if let Some(selector) = selector {
        let element = page.find_element(selector).await.map_err(|_| {
            (
                "BROWSER_SELECTOR",
                "No se encontró el elemento del selector",
            )
        })?;
        let bounds = element
            .bounding_box()
            .await
            .map_err(|_| ("BROWSER_CDP", "No se pudo medir el elemento"))?;
        let _width = valid_dimension(bounds.width.ceil())?;
        let _height = valid_dimension(bounds.height.ceil())?;
        let bytes = element
            .screenshot(CaptureScreenshotFormat::Png)
            .await
            .map_err(|_| ("BROWSER_CDP", "No se pudo capturar el elemento"))?;
        bytes
    } else {
        let metrics = page.layout_metrics().await.map_err(|_| {
            (
                "BROWSER_CDP",
                "No se pudieron leer las dimensiones de la página",
            )
        })?;
        let viewport = metrics.css_visual_viewport;
        let width = valid_dimension(viewport.client_width.ceil())?;
        let height = valid_dimension(viewport.client_height.ceil())?;
        let clip = Viewport {
            x: viewport.page_x,
            y: viewport.page_y,
            width: f64::from(width),
            height: f64::from(height),
            scale: 1.0,
        };
        let bytes = page
            .screenshot(
                ScreenshotParams::builder()
                    .format(CaptureScreenshotFormat::Png)
                    .clip(clip)
                    .capture_beyond_viewport(false)
                    .build(),
            )
            .await
            .map_err(|_| ("BROWSER_CDP", "No se pudo capturar la pestaña"))?;
        bytes
    };

    if bytes.len() > MAX_SCREENSHOT_BYTES {
        return Err(("BROWSER_LIMIT", "La captura excede el límite de bytes"));
    }
    let (actual_width, actual_height) = png_dimensions(&bytes)
        .ok_or(("BROWSER_CDP", "Chrome devolvió una captura PNG inválida"))?;
    if actual_width > MAX_SCREENSHOT_DIMENSION || actual_height > MAX_SCREENSHOT_DIMENSION {
        return Err((
            "BROWSER_LIMIT",
            "La captura excede las dimensiones permitidas",
        ));
    }
    Ok((bytes, actual_width, actual_height))
}

fn valid_dimension(value: f64) -> Result<u32, Failure> {
    if !value.is_finite() || value < 1.0 || value > f64::from(MAX_SCREENSHOT_DIMENSION) {
        return Err((
            "BROWSER_LIMIT",
            "El elemento o viewport excede las dimensiones permitidas",
        ));
    }
    Ok(value as u32)
}

fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    Some((
        u32::from_be_bytes(bytes[16..20].try_into().ok()?),
        u32::from_be_bytes(bytes[20..24].try_into().ok()?),
    ))
}

pub(super) fn html_limit(value: Option<u32>) -> usize {
    value
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_HTML_CHARS)
        .clamp(1, MAX_HTML_CHARS)
}
