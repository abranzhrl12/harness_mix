//! Composition root: terminal options, local installation discovery and adapter wiring.
use agent_harness::adapters::acp::{AntigravityAcpAdapter, AntigravityAcpConfig, protocol_path};
use harness_application::conversation::Conversation;
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

fn parse(args: Vec<OsString>) -> Result<Options, String> {
    let mut options = Options {
        action: Action::Help,
        root: env::current_dir().map_err(|e| e.to_string())?,
        command: env::var_os("HARNESSMIX_ANTIGRAVITY_ACP").map(PathBuf::from),
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

pub async fn run(args: Vec<OsString>) -> Result<(), String> {
    let options = parse(args)?;
    match options.action {
        Action::HookDeny => return super::hooks::deny_native_tool(),
        Action::HookProfile => return super::hooks::print_profile(),
        Action::Help => {
            print_help();
            return Ok(());
        }
        Action::Version => {
            println!("harnessmix {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        _ => {}
    }
    let executable = discover(options.command.as_deref());
    if options.action == Action::Config {
        print_config(executable.as_deref(), &options.root, options.harness, options.permissions, options.animate);
        return Ok(());
    }
    let executable = executable.ok_or_else(|| {
        let detail = options.command.as_ref().map(|path| format!(" en {}", path.display())).unwrap_or_default();
        format!("No se encontró el servidor ACP de Antigravity{detail}. Usa --agent-command <ruta-a-agy_acp_server.exe> o HARNESSMIX_ANTIGRAVITY_ACP. El CLI agy no sustituye al servidor ACP.")
    })?;
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err("harnessmix active/login necesita una terminal interactiva".into());
    }
    let workspace = options
        .root
        .canonicalize()
        .map_err(|e| format!("Proyecto no disponible: {e}"))?;
    let workspace = protocol_path(&workspace);
    if !workspace.is_dir() {
        return Err("--root debe ser un directorio".into());
    }
    if options.action == Action::Active && !choose_agent(&executable, &workspace, options.harness, options.permissions, options.animate)?
    {
        return Ok(());
    }
    let mcp_servers = if options.harness && options.action == Action::Active {
        let harness = harness_executable().ok_or(
            "No se encontró agent-harness. Instálalo junto a harnessmix o usa --no-harness.",
        )?;
        let harness = protocol_path(&harness);
        vec![serde_json::json!({
            "name":"harness", "command":harness, "args":["--root", workspace.to_string_lossy()], "env":[]
        })]
    } else {
        Vec::new()
    };
    let helper_name = if cfg!(windows) {
        "localharness_external.exe"
    } else {
        "localharness_external"
    };
    let helper = executable
        .parent()
        .map(|p| p.join(helper_name))
        .filter(|p| p.is_file());
    let adapter = AntigravityAcpAdapter::new(AntigravityAcpConfig {
        executable,
        args: options.args,
        mcp_servers,
        harness_helper: helper,
    });
    let conversation = Arc::new(Conversation::new(adapter));
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

fn discover(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return executable_file(path);
    }
    let name = if cfg!(windows) {
        "agy_acp_server.exe"
    } else {
        "agy_acp_server.par"
    };
    if let Some(found) = from_path(name) {
        return Some(found);
    }
    if cfg!(windows) {
        env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .and_then(|p| {
                executable_file(&p.join("harnessmix/agents/antigravity/1.4.3").join(name))
            })
    } else {
        None
    }
}

fn executable_file(path: &Path) -> Option<PathBuf> {
    let path = path.canonicalize().ok()?;
    if !path.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if path.metadata().ok()?.permissions().mode() & 0o111 == 0 {
            return None;
        }
    }
    Some(path)
}

fn from_path(name: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|value| {
        env::split_paths(&value)
            .filter(|p| p.is_absolute())
            .find_map(|p| executable_file(&p.join(name)))
    })
}

fn harness_executable() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "agent-harness.exe"
    } else {
        "agent-harness"
    };
    env::current_exe()
        .ok()
        .and_then(|p| p.parent().and_then(|d| executable_file(&d.join(name))))
        .or_else(|| from_path(name))
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
