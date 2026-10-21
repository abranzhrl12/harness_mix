use super::config::SqlConfig;
use super::diagnostics::sql_error;
use crate::adapters::project::Project;
use harness_domain::{Failure, execution::OperationContext};
use std::collections::HashMap;
use tiberius::{AuthMethod, Client, Config};
use tokio::net::TcpStream;
use tokio_util::compat::TokioAsyncWriteCompatExt;

pub(in crate::adapters) fn block_on_safe<F, T>(future: F) -> T
where
    F: std::future::Future<Output = T> + Send,
    T: Send,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| handle.block_on(future))
        }
        Ok(_) => std::thread::scope(|scope| {
            scope
                .spawn(|| block_on_safe_standalone(future))
                .join()
                .expect("SQL worker panicked")
        }),
        Err(_) => block_on_safe_standalone(future),
    }
}

fn block_on_safe_standalone<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build standalone SQL runtime")
        .block_on(future)
}

/// Extract connection string from Web.config or environment
pub fn resolve_connection_string(project: &Project, config: &SqlConfig) -> Result<String, Failure> {
    if config.connection_source == "environment" {
        return crate::adapters::sql_credentials::resolve(project, config.connection_name.as_deref().unwrap_or("HARNESS_SQL_CONNECTION_STRING"), config.env_file.as_deref(), &OperationContext::default())
            .map_err(|(code, _)| match code { "CANCELLED" | "TIMEOUT" => (code, "Credential lookup interrupted"), _ => ("SQL_CONFIG", "Set the selected SQL Server connection variable in the process environment or configured project dotenv file") });
    }

    let rel_path = config.connection_path.as_deref().unwrap_or("Web.config");
    let target_name = config.connection_name.as_deref().unwrap_or("EFAST");
    let path = project.root.join(rel_path);

    let content =
        std::fs::read_to_string(&path).map_err(|_| ("SQL_CONFIG", "No se pudo leer Web.config"))?;

    let doc = roxmltree::Document::parse(&content)
        .map_err(|_| ("SQL_CONFIG", "Web.config tiene formato XML inválido"))?;

    for node in doc.descendants() {
        if node.has_tag_name("add")
            && let (Some(name), Some(conn_str)) =
                (node.attribute("name"), node.attribute("connectionString"))
            && name.eq_ignore_ascii_case(target_name)
        {
            return Ok(conn_str.to_string());
        }
    }

    Err((
        "SQL_CONFIG",
        "Connection string no encontrada en Web.config",
    ))
}

/// Parse standard ADO.NET connection string into Tiberius Config
fn parse_connection_string(conn_str: &str) -> Result<Config, Failure> {
    let mut parts = HashMap::new();
    for part in conn_str.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((k, v)) = part.split_once('=') {
            parts.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }

    let server_raw = parts
        .get("data source")
        .or_else(|| parts.get("server"))
        .or_else(|| parts.get("address"))
        .ok_or((
            "SQL_CONFIG",
            "Falta Data Source o Server en la cadena de conexión",
        ))?;

    let mut config = Config::new();

    let (host, port) = if let Some((h, p)) = server_raw.split_once(',') {
        (h.trim(), p.trim().parse::<u16>().unwrap_or(1433))
    } else if let Some((h, p)) = server_raw.split_once(':') {
        (h.trim(), p.trim().parse::<u16>().unwrap_or(1433))
    } else {
        (server_raw.as_str(), 1433)
    };

    config.host(host);
    config.port(port);

    if let Some(db) = parts
        .get("initial catalog")
        .or_else(|| parts.get("database"))
    {
        config.database(db);
    }

    let integrated = parts
        .get("integrated security")
        .map(|s| s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("sspi"))
        .unwrap_or(false);

    if integrated {
        #[cfg(windows)]
        {
            config.authentication(AuthMethod::Integrated);
        }
        #[cfg(not(windows))]
        {
            return Err((
                "SQL_CONFIG",
                "Integrated Security solo está soportada en Windows",
            ));
        }
    } else {
        let user = parts
            .get("user id")
            .or_else(|| parts.get("uid"))
            .or_else(|| parts.get("user"));
        let password = parts.get("password").or_else(|| parts.get("pwd"));
        if let (Some(u), Some(p)) = (user, password) {
            config.authentication(AuthMethod::sql_server(u, p));
        }
    }

    config.trust_cert();
    Ok(config)
}

/// Establish an active connection to SQL Server via Tiberius
pub(super) async fn connect(
    project: &Project,
    config: &SqlConfig,
) -> Result<Client<tokio_util::compat::Compat<TcpStream>>, Failure> {
    let conn_str = resolve_connection_string(project, config)?;
    let t_config = parse_connection_string(&conn_str)?;

    let tcp = TcpStream::connect(t_config.get_addr()).await.map_err(|_| {
        (
            "SQL_CONNECTION",
            "No se pudo conectar al servidor SQL; verifique la IP y puerto",
        )
    })?;

    let client = Client::connect(t_config, tcp.compat_write())
        .await
        .map_err(sql_error)?;

    Ok(client)
}
