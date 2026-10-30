use std::io::{self, BufRead, IsTerminal, Write};
use std::path::Path;

use crate::commands::AgentHost;

pub(super) fn config(executable: Option<&Path>) -> Result<(), Box<dyn std::error::Error>> {
    write_config(&mut io::stdout().lock(), executable)?;
    Ok(())
}

fn write_config(output: &mut impl Write, executable: Option<&Path>) -> io::Result<()> {
    writeln!(output, "Proveedores de agentes")?;
    match executable {
        Some(path) => writeln!(output, "Antigravity: disponible ({})", path.display())?,
        None => writeln!(output, "Antigravity: no instalado o no encontrado en PATH")?,
    }
    writeln!(output, "Inicio: agent-harness active antigravity")?;
    writeln!(
        output,
        "Antigravity gestiona su configuración y el inicio de sesión de Google."
    )?;
    writeln!(output, "Codex y Claude: integración pendiente.")
}

pub(super) fn run(
    root: &Path,
    provider: Option<&str>,
    host: &impl AgentHost,
) -> Result<(), Box<dyn std::error::Error>> {
    let executable = host.agent_executable();
    let selected = match provider {
        Some("antigravity") => Some("antigravity"),
        Some(_) => return Err("proveedor no soportado".into()),
        None => {
            if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
                return Err(
                    "El menú necesita una terminal. Usa: agent-harness active antigravity".into(),
                );
            }
            // Release both locks before handing the terminal to the child process.
            menu(
                &mut io::stdin().lock(),
                &mut io::stdout().lock(),
                executable.as_deref(),
            )?
        }
    };
    if let Some(provider) = selected {
        let code = host.launch_agent(root, provider)?;
        if code != 0 {
            return Err(format!("Antigravity terminó con código {code}").into());
        }
    }
    Ok(())
}

fn menu(
    input: &mut impl BufRead,
    output: &mut impl Write,
    executable: Option<&Path>,
) -> io::Result<Option<&'static str>> {
    writeln!(output, "Agent Harness — seleccionar agente")?;
    writeln!(
        output,
        "1. Antigravity{}",
        if executable.is_some() {
            ""
        } else {
            " (no encontrado)"
        }
    )?;
    writeln!(output, "/config  Ver proveedores y configuración")?;
    writeln!(output, "0. Salir")?;
    loop {
        write!(output, "> ")?;
        output.flush()?;
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        match line.trim() {
            "0" | "q" | "exit" => return Ok(None),
            "1" | "antigravity" if executable.is_some() => {
                return Ok(Some("antigravity"));
            }
            "1" | "antigravity" => writeln!(
                output,
                "No se encontró agy. Instala Antigravity CLI y vuelve a abrir la terminal."
            )?,
            "/config" | "config" => write_config(output, executable)?,
            _ => writeln!(output, "Elige 1, /config o 0.")?,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn config_and_invalid_choices_keep_menu_open_until_selection() {
        let mut input = Cursor::new(b"wrong\n/config\n1\n");
        let mut output = Vec::new();
        let selection = menu(&mut input, &mut output, Some(Path::new("/tools/agy"))).unwrap();
        assert_eq!(selection, Some("antigravity"));
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("Elige 1"));
        assert!(text.contains("Antigravity: disponible"));
    }

    #[test]
    fn unavailable_provider_is_not_selected_and_eof_cancels() {
        let mut output = Vec::new();
        assert_eq!(
            menu(&mut Cursor::new(b"1\n"), &mut output, None).unwrap(),
            None
        );
        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains("No se encontró agy")
        );
    }

    #[test]
    fn zero_exits_without_launching() {
        assert_eq!(
            menu(
                &mut Cursor::new(b"0\n"),
                &mut Vec::new(),
                Some(Path::new("/tools/agy"))
            )
            .unwrap(),
            None
        );
    }
}
