use super::ports::{ClipboardFuture, ClipboardReader};
use super::state::{ImageCompletion, UiChannels, UiState, remove_pending_image};
use super::{ConsoleInput, ConsoleOutput, ConsoleResult, UiEvent};
use crate::{clipboard, images};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::mpsc as tokio_mpsc;

pub(super) struct SystemClipboardReader;

impl ClipboardReader for SystemClipboardReader {
    fn read(&self) -> ClipboardFuture<'_> {
        Box::pin(clipboard::read_content())
    }
}

pub(super) async fn enqueue_image(
    path: PathBuf,
    state: &mut UiState,
    tx: &tokio_mpsc::UnboundedSender<ImageCompletion>,
    output: &Arc<dyn ConsoleOutput>,
) -> ConsoleResult<()> {
    let id = state.next_image_id;
    state.next_image_id = state.next_image_id.wrapping_add(1);
    state.next_image_ids.push(id);
    let tx = tx.clone();
    tokio::spawn(async move {
        let _ = tx.send((id, images::load_image(path).await));
    });
    output
        .write_text("Cargando imagen para el siguiente mensaje…\n".into())
        .await
}

pub(super) fn request_clipboard(state: &mut UiState, channels: &UiChannels) -> ConsoleResult<()> {
    let id = state.next_image_id;
    state.next_image_id = state.next_image_id.wrapping_add(1);
    state.next_image_ids.push(id);
    let tx = channels.events_tx.clone();
    let reader = channels.clipboard.clone();
    tokio::spawn(async move {
        let result = reader.read().await;
        let _ = tx.send(UiEvent::Clipboard { id, result });
    });
    Ok(())
}

pub(super) async fn finish_clipboard<I: ConsoleInput + ?Sized>(
    id: u64,
    result: Result<Option<clipboard::ClipboardContent>, String>,
    input: &mut I,
    output: &Arc<dyn ConsoleOutput>,
    state: &mut UiState,
) -> ConsoleResult<()> {
    match result {
        Ok(Some(clipboard::ClipboardContent::Image(image))) => {
            state.image_results.insert(id, Ok(image));
            output
                .write_text("Imagen del portapapeles adjunta. Escribe tu mensaje.\n".into())
                .await
        }
        Ok(Some(clipboard::ClipboardContent::Text(text))) => {
            remove_pending_image(id, state);
            input.insert_text(&text)?;
            Ok(())
        }
        Ok(None) => {
            remove_pending_image(id, state);
            output
                .write_text("El portapapeles no contiene una imagen ni texto.\n".into())
                .await
        }
        Err(error) => {
            remove_pending_image(id, state);
            output
                .write_text(format!("No se pudo leer el portapapeles: {error}\n"))
                .await
        }
    }
}

pub(super) async fn finish_image(
    (id, result): ImageCompletion,
    output: &Arc<dyn ConsoleOutput>,
    state: &mut UiState,
) -> ConsoleResult<()> {
    let notice = match &result {
        Ok(_) => "Imagen adjunta. Escribe tu mensaje.\n".to_owned(),
        Err(error) => format!("No se pudo cargar la imagen: {error}\n"),
    };
    state.image_results.insert(id, result);
    output.write_text(notice).await
}

pub(super) fn image_path(line: &str) -> Option<PathBuf> {
    let rest = line.trim().strip_prefix("/image")?.trim();
    if rest.is_empty() {
        return None;
    }
    let path = if (rest.starts_with('"') && rest.ends_with('"'))
        || (rest.starts_with('\'') && rest.ends_with('\''))
    {
        &rest[1..rest.len().saturating_sub(1)]
    } else {
        rest
    };
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        ConsoleOptions,
        context::UiRuntime,
        state::{queue_prompt, ready_prompt},
    };
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    struct FakeClipboardReader(Arc<AtomicUsize>);

    impl ClipboardReader for FakeClipboardReader {
        fn read(&self) -> ClipboardFuture<'_> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(Some(clipboard::ClipboardContent::Text("copiado".into()))) })
        }
    }

    #[tokio::test]
    async fn injected_clipboard_is_read_only_on_request_and_reserves_the_attachment() {
        let calls = Arc::new(AtomicUsize::new(0));
        let UiRuntime {
            mut channels,
            mut state,
            ..
        } = UiRuntime::new(
            ConsoleOptions::default(),
            false,
            Arc::new(FakeClipboardReader(calls.clone())),
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        request_clipboard(&mut state, &channels).unwrap();
        queue_prompt("analiza".into(), &mut state);
        assert!(ready_prompt(&mut state).is_none());
        let event = tokio::time::timeout(Duration::from_secs(1), channels.events.recv())
            .await
            .unwrap()
            .unwrap();
        let UiEvent::Clipboard { id, result } = event else {
            panic!("expected clipboard completion");
        };
        assert_eq!(id, 0);
        assert!(
            matches!(result.unwrap(), Some(clipboard::ClipboardContent::Text(text)) if text == "copiado")
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
