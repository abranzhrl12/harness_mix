//! Terminal command parsing and UI orchestration.
use harness_application::{
    conversation::Conversation,
    ports::conversation::ConversationProvider,
};
use harness_domain::conversation::PermissionPolicy;
use std::{
    env,
    ffi::OsString,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug, PartialEq, Eq)]
enum Action {
    HookDeny,
    HookProfile,
    Active,
    Login,
    Config,
    Help,
    Version,
}

#[derive(Debug)]
struct Options {
    action: Action,
    root: PathBuf,
    command: Option<PathBuf>,
    args: Vec<OsString>,
    harness: bool,
    permissions: PermissionPolicy,
    animate: bool,
}

/// Runtime configuration passed from the terminal application to its composition root.
#[derive(Debug, Clone)]
pub struct ConversationConfig {
    pub executable: PathBuf,
    pub args: Vec<OsString>,
    pub workspace: PathBuf,
    pub harness_enabled: bool,
}

/// Boundary implemented by the binary composition root for a concrete agent transport.
pub trait ConversationFactory {
    type Provider: ConversationProvider + 'static;

    fn default_command(&self) -> Option<PathBuf> {
        None
    }
    fn discover(&self, explicit: Option<&Path>) -> Option<PathBuf>;
    fn create(&self, config: ConversationConfig) -> Result<Self::Provider, String>;
    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }
}

fn parse(args: Vec<OsString>) -> Result<Options, String> {
    let mut options = Options {
        action: Action::Help,
        root: env::current_dir().map_err(|e| e.to_string())?,
        command: None,
        args: Vec::new(),
        harness: true,
        permissions: PermissionPolicy::Ask,
        animate: true,
    };
    let Some(first) = args.first().and_then(|arg| arg.to_str()) else {
        return if args.is_empty() {
            Ok(options)
        } else {
            Err("El comando debe ser UTF-8".into())
        };
    };
    if first == "hook" {
        if args.len() == 2 && args[1] == "deny-native" {
            options.action = Action::HookDeny;
            return Ok(options);
        }
        if args.len() == 2 && args[1] == "profile" {
            options.action = Action::HookProfile;
            return Ok(options);
        }
        return Err("Uso interno: harnessmix hook deny-native".into());
    }
    options.action = match first {
        "active" => Action::Active,
        "login" => Action::Login,
        "config" | "/config" => Action::Config,
        "--help" | "-h" | "help" => Action::Help,
        "--version" | "-V" => Action::Version,
        _ => return Err("Uso: harnessmix <active|login|config> [--root <proyecto>] [--agent-command <servidor-ACP>]".into()),
    };
    let mut index = 1;
    let mut root_seen = false;
    let mut command_seen = false;
    while index < args.len() {
        match args[index].to_str() {
            Some("--root") if !root_seen && index + 1 < args.len() => {
                if args[index + 1].is_empty() {
                    return Err("--root necesita una ruta".into());
                }
                options.root = PathBuf::from(&args[index + 1]);
                root_seen = true;
                index += 2;
            }
            Some("--agent-command") if !command_seen && index + 1 < args.len() => {
                if args[index + 1].is_empty() {
                    return Err("--agent-command necesita una ruta".into());
                }
                options.command = Some(PathBuf::from(&args[index + 1]));
                command_seen = true;
                index += 2;
            }
            Some("--agent-arg") if index + 1 < args.len() => {
                options.args.push(args[index + 1].clone());
                index += 2;
            }
            Some("--no-harness") => {
                options.harness = false;
                index += 1;
            }
            Some("--dangerously-skip-permissions" | "--yolo") => {
                options.permissions = PermissionPolicy::AutoApprove;
                index += 1;
            }
            Some("--no-animation") => {
                options.animate = false;
                index += 1;
            }
            _ => {
                return Err(format!(
                    "Opción desconocida o incompleta: {}",
                    args[index].to_string_lossy()
                ));
            }
        }
    }
    Ok(options)
}

pub async fn run<F: ConversationFactory>(args: Vec<OsString>, factory: &F) -> Result<(), String> {
    let options = parse(args)?;
    match options.action {
        Action::HookDeny => return super::hooks::deny_native_tool(),
        Action::HookProfile => return super::hooks::print_profile(),
        Action::Help => {
            print_help();
            return Ok(());
        }
        Action::Version => {
            println!("harnessmix {}", factory.version());
            return Ok(());
        }
        _ => {}
    }
    let requested_command = options
        .command
        .clone()
        .or_else(|| factory.default_command());
    let executable = factory.discover(requested_command.as_deref());
    if options.action == Action::Config {
        print_config(executable.as_deref(), &options.root, options.harness, options.permissions, options.animate);
        return Ok(());
    }
    let executable = executable.ok_or_else(|| {
        let detail = requested_command.as_ref().map(|path| format!(" en {}", path.display())).unwrap_or_default();
        format!("No se encontró el servidor ACP de Antigravity{detail}. Usa --agent-command <ruta-a-agy_acp_server.exe> o HARNESSMIX_ANTIGRAVITY_ACP. El CLI agy no sustituye al servidor ACP.")
    })?;
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err("harnessmix active/login necesita una terminal interactiva".into());
    }
    let workspace = options
        .root
        .canonicalize()
        .map_err(|e| format!("Proyecto no disponible: {e}"))?;
    if !workspace.is_dir() {
        return Err("--root debe ser un directorio".into());
    }
    if options.action == Action::Active && !choose_agent(&executable, &workspace, options.harness, options.permissions, options.animate)?
    {
        return Ok(());
    }
    let provider = factory.create(ConversationConfig {
        executable,
        args: options.args,
        workspace: workspace.clone(),
        harness_enabled: options.harness && options.action == Action::Active,
    })?;
    let conversation = Arc::new(Conversation::new(provider));
    let result = if options.action == Action::Login {
        super::console::login(&conversation).await
    } else {
        super::console::run_with_options(conversation.clone(), &workspace, super::console::ConsoleOptions {
            permissions: options.permissions,
            animate: options.animate,
        }).await
    };
    let closed = conversation.close().await.map_err(|e| e.to_string());
    result.and(closed)
}

fn choose_agent(executable: &Path, root: &Path, harness: bool, permissions: PermissionPolicy, animate: bool) -> Result<bool, String> {
    println!(
        "HarnessMix — seleccionar agente\n1. Antigravity (ACP · cuenta Google)\n/config  Ver configuración\n0. Salir"
    );
    loop {
        print!("> ");
        io::stdout().flush().map_err(|e| e.to_string())?;
        let mut line = String::new();
        if io::stdin()
            .read_line(&mut line)
            .map_err(|e| e.to_string())?
            == 0
        {
            return Ok(false);
        }
        match line.trim() {
            "1" | "antigravity" => return Ok(true),
            "0" | "q" | "/exit" => return Ok(false),
            "/config" | "config" => print_config(Some(executable), root, harness, permissions, animate),
            _ => println!("Elige 1, /config o 0."),
        }
    }
}

fn print_config(executable: Option<&Path>, root: &Path, harness: bool, permissions: PermissionPolicy, animate: bool) {
    println!(
        "HarnessMix · cliente ACP nativo\nProyecto: {}",
        root.display()
    );
    match executable {
        Some(path) => println!("Antigravity: disponible\nServidor: {}", path.display()),
        None => println!(
            "Antigravity: no disponible; instala el servidor ACP oficial o indica --agent-command."
        ),
    }
    println!("Login: harnessmix login (Google mediante el agente oficial)");
    println!("Permisos: {}", match permissions {
        PermissionPolicy::Ask => "preguntar",
        PermissionPolicy::AutoApprove => "automáticos",
    });
    println!("Animación: {}", if animate { "activada" } else { "desactivada" });
    println!(
        "Harness MCP: {}",
        if harness {
            "habilitado para la nueva sesión"
        } else {
            "deshabilitado"
        }
    );
}

fn print_help() {
    println!("Permisos y actividad:\n  --yolo, --dangerously-skip-permissions  Aprobar automáticamente permisos de herramientas durante esta sesión\n  --no-animation                       Ocultar el indicador animado\n  /permissions auto | /permissions ask  Cambiar el modo dentro del chat\n");
    println!(
        "HarnessMix — cliente ACP nativo\n\nUso:\n  harnessmix active    Elegir Antigravity y conversar\n  harnessmix login     Iniciar sesión con Google\n  harnessmix config    Mostrar configuración local\n\nOpciones:\n  --root <directorio>       Proyecto (por defecto, directorio actual)\n  --agent-command <ruta>    Servidor ACP oficial de Antigravity\n  --agent-arg <argumento>   Argumento del servidor (repetible)\n  --no-harness              No conectar Harness MCP\n  --help, --version\n\nAlternativa de ruta: HARNESSMIX_ANTIGRAVITY_ACP.\nDentro del chat: /login, /config, /exit. Ctrl+C cancela la respuesta."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_is_explicit_and_animation_can_be_disabled() {
        let defaults = parse(vec![OsString::from("active")]).unwrap();
        assert_eq!(defaults.permissions, PermissionPolicy::Ask);
        assert!(defaults.animate);
        for flag in ["--yolo", "--dangerously-skip-permissions"] {
            let options = parse(["active", flag, "--no-animation"].map(OsString::from).to_vec()).unwrap();
            assert_eq!(options.permissions, PermissionPolicy::AutoApprove);
            assert!(!options.animate);
        }
    }
    #[test]
    fn preserves_argument_boundaries_and_rejects_incomplete_flags() {
        let options = parse(
            [
                "active",
                "--root",
                "project with spaces",
                "--agent-command",
                "C:/agent path/agent.exe",
                "--agent-arg",
                "--flag=value",
            ]
            .map(OsString::from)
            .to_vec(),
        )
        .unwrap();
        assert_eq!(options.root, PathBuf::from("project with spaces"));
        assert_eq!(options.args, vec![OsString::from("--flag=value")]);
        for args in [
            vec!["active", "--agent-command"],
            vec!["login", "--agent-arg"],
            vec!["active", "--root", ""],
            vec!["config", "--unknown"],
        ] {
            assert!(parse(args.into_iter().map(OsString::from).collect()).is_err());
        }
    }
}
