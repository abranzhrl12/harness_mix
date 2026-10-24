use super::ports::{ConsoleInput, ConsoleOutput, ConsoleResult, InputEvent, LocalFuture};
use rustyline_async::{Readline, ReadlineEvent, SharedWriter};
use std::{
    io::{self, BufRead, IsTerminal, Write},
    sync::{Arc, Mutex},
};
use tokio::sync::{Mutex as TokioMutex, mpsc as tokio_mpsc};

pub(super) struct ConsoleIo {
    pub(super) input: Box<dyn ConsoleInput>,
    pub(super) output: Arc<dyn ConsoleOutput>,
    pub(super) terminal: bool,
}

/// This is the only place that selects concrete terminal or redirected adapters.
pub(super) fn open(prompt: &str) -> ConsoleResult<ConsoleIo> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    if stdin.is_terminal() && stdout.is_terminal() {
        let (readline, writer) =
            Readline::new(prompt.to_owned()).map_err(|error| error.to_string())?;
        Ok(ConsoleIo {
            input: Box::new(TerminalInput(readline)),
            output: Arc::new(TerminalOutput(TokioMutex::new(writer))),
            terminal: true,
        })
    } else {
        let (sender, receiver) = tokio_mpsc::unbounded_channel();
        std::thread::spawn(move || input_thread(io::BufReader::new(stdin), sender));
        Ok(ConsoleIo {
            input: Box::new(ChannelInput(receiver)),
            output: Arc::new(PlainOutput(Arc::new(Mutex::new(stdout)))),
            terminal: false,
        })
    }
}

pub(super) struct TerminalInput(pub(super) Readline);

impl ConsoleInput for TerminalInput {
    fn next_event(&mut self) -> LocalFuture<'_, ConsoleResult<InputEvent>> {
        Box::pin(async move {
            match self.0.readline().await.map_err(|error| error.to_string())? {
                ReadlineEvent::Line(line) => Ok(InputEvent::Line(line)),
                ReadlineEvent::PasteRequested => Ok(InputEvent::PasteRequested),
                ReadlineEvent::Eof => Ok(InputEvent::Eof),
                ReadlineEvent::Interrupted => Ok(InputEvent::Interrupted),
            }
        })
    }

    fn update_prompt(&mut self, prompt: &str) -> ConsoleResult<()> {
        self.0
            .update_prompt(prompt)
            .map_err(|error| error.to_string())
    }

    fn set_clipboard_available(&mut self, available: bool) -> ConsoleResult<()> {
        self.0.set_paste_requested(available);
        Ok(())
    }

    fn insert_text(&mut self, text: &str) -> ConsoleResult<()> {
        self.0.insert_text(text).map_err(|error| error.to_string())
    }

    fn set_selector_mode(&mut self, active: bool, prompt: &str) -> ConsoleResult<bool> {
        self.0.should_print_line_on(!active, !active);
        self.0.set_paste_requested(!active);
        self.0
            .update_prompt(prompt)
            .map_err(|error| error.to_string())?;
        Ok(true)
    }
}

pub(super) struct ChannelInput(pub(super) tokio_mpsc::UnboundedReceiver<InputEvent>);

impl ConsoleInput for ChannelInput {
    fn next_event(&mut self) -> LocalFuture<'_, ConsoleResult<InputEvent>> {
        Box::pin(async move {
            self.0
                .recv()
                .await
                .ok_or_else(|| "console input closed".to_owned())
        })
    }

    fn update_prompt(&mut self, _prompt: &str) -> ConsoleResult<()> {
        Ok(())
    }
}

pub(super) struct PlainOutput<W>(pub(super) Arc<Mutex<W>>);

impl<W: Write + Send> ConsoleOutput for PlainOutput<W> {
    fn write_text(&self, text: String) -> LocalFuture<'_, ConsoleResult<()>> {
        Box::pin(async move {
            let mut output = self.0.lock().map_err(|_| "console output unavailable")?;
            output
                .write_all(text.as_bytes())
                .and_then(|_| output.flush())
                .map_err(|error| error.to_string())
        })
    }
}

pub(super) struct TerminalOutput(pub(super) TokioMutex<SharedWriter>);

impl ConsoleOutput for TerminalOutput {
    fn write_text(&self, text: String) -> LocalFuture<'_, ConsoleResult<()>> {
        Box::pin(async move {
            let mut output = self.0.lock().await;
            futures_util::io::AsyncWriteExt::write_all(&mut *output, text.as_bytes())
                .await
                .map_err(|error| error.to_string())?;
            futures_util::io::AsyncWriteExt::flush(&mut *output)
                .await
                .map_err(|error| error.to_string())
        })
    }
}

pub(super) fn input_thread<R: BufRead>(
    mut input: R,
    sender: tokio_mpsc::UnboundedSender<InputEvent>,
) {
    loop {
        let mut line = String::new();
        match input.read_line(&mut line) {
            Ok(0) => {
                let _ = sender.send(InputEvent::Eof);
                break;
            }
            Ok(_) => {
                let line = line.trim_end_matches(['\r', '\n']).to_owned();
                if sender.send(InputEvent::Line(line)).is_err() {
                    break;
                }
            }
            Err(_) => {
                let _ = sender.send(InputEvent::Eof);
                break;
            }
        }
    }
}
