use super::super::project::Project;
use super::config::{CONFIG, Config};
use harness_domain::{Failure, execution::OperationContext};
use serde_json::json;
use std::time::Duration;
use tokio_postgres::{
    Config as DriverConfig,
    config::{Host, SslMode},
};

fn driver_config(
    project: &Project,
    config: &Config,
    context: &OperationContext,
) -> Result<(DriverConfig, String), Failure> {
    let connection = config.connection.as_ref().ok_or(CONFIG)?;
    if connection.source != "environment"
        || connection.name.is_empty()
        || !connection
            .name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(CONFIG);
    }
    let dsn = super::super::sql_credentials::resolve(
        project,
        &connection.name,
        connection.env_file.as_deref(),
        context,
    )?;
    let mut driver: DriverConfig = dsn.parse().map_err(|_| {
        (
            "PG_CONNECTION_CONFIG",
            "Invalid PostgreSQL connection string",
        )
    })?;
    if driver.get_hosts().is_empty() || !driver.get_hostaddrs().is_empty() {
        return Err(CONFIG);
    }
    if config.tls.mode == "disable-loopback" {
        if config.tls.ca_file.is_some()
            || driver.get_hosts().iter().any(|h| match h {
                Host::Tcp(host) => {
                    host != "localhost"
                        && !host
                            .parse::<std::net::IpAddr>()
                            .is_ok_and(|ip| ip.is_loopback())
                }
                #[cfg(unix)]
                Host::Unix(_) => true,
            })
        {
            return Err((
                "PG_TLS",
                "Unencrypted PostgreSQL is allowed only with explicit loopback hosts",
            ));
        }
        driver.ssl_mode(SslMode::Disable);
    } else if config.tls.mode == "disable" {
        driver.ssl_mode(SslMode::Disable);
    } else {
        driver.ssl_mode(SslMode::Require);
    }
    driver.connect_timeout(Duration::from_secs(config.timeout_seconds));
    // Session policy travels in the startup packet, not a SET round-trip on every read.
    // It is verified after authentication before exposing this connection to catalog calls.
    let options = driver.get_options().unwrap_or("").to_owned();
    driver.options(format!("{options}  -c default_transaction_read_only=on -c search_path=pg_catalog -c statement_timeout={} -c lock_timeout={}",
        config.timeout_seconds * 1000, (config.timeout_seconds * 1000).min(2000)));
    if driver.get_application_name().is_none() {
        driver.application_name(format!("agent-harness:{}", std::process::id()));
    }
    Ok((driver, harness_domain::hash(dsn.as_bytes())))
}
fn certificate(project: &Project, config: &Config) -> Result<Option<Vec<u8>>, Failure> {
    if let Some(file) = &config.tls.ca_file {
        // Certificates must be workspace-relative; do not open arbitrary credential paths.
        let path = project.target(file)?;
        let metadata = std::fs::metadata(&path).map_err(|_| CONFIG)?;
        if !metadata.is_file() || metadata.len() > 256 * 1024 {
            return Err(CONFIG);
        }
        let pem = std::fs::read(path).map_err(|_| CONFIG)?;
        return Ok(Some(pem));
    }
    Ok(None)
}
pub(super) fn tls(
    certificate: Option<&[u8]>,
) -> Result<postgres_native_tls::MakeTlsConnector, Failure> {
    let mut builder = native_tls::TlsConnector::builder();
    if let Some(pem) = certificate {
        builder.add_root_certificate(native_tls::Certificate::from_pem(pem).map_err(|_| CONFIG)?);
    }
    Ok(postgres_native_tls::MakeTlsConnector::new(
        builder
            .build()
            .map_err(|_| ("PG_TLS", "Unable to initialize certificate validation"))?,
    ))
}

pub(super) fn pg_error(error: tokio_postgres::Error) -> Failure {
    match error.code().map(|c| c.code()) {
        Some("28P01" | "28000") => ("PG_AUTH", "PostgreSQL authentication failed"),
        Some("42501") => (
            "PG_PERMISSION",
            "PostgreSQL role cannot access the requested metadata",
        ),
        Some("57014") => (
            "PG_STATEMENT_TIMEOUT",
            "PostgreSQL statement cancelled or server timeout reached",
        ),
        Some("55P03") => ("PG_LOCK_TIMEOUT", "PostgreSQL lock wait exceeded its limit"),
        _ => (
            "PG_OPERATION",
            "PostgreSQL connection, TLS or catalog operation failed; raw server errors are withheld",
        ),
    }
}

pub(super) struct ConnectionPlan {
    pub(super) driver: DriverConfig,
    pub(super) certificate: Option<Vec<u8>>,
    pub(super) identity: String,
}
pub(super) fn connection_plan(
    project: &Project,
    config: &Config,
    context: &OperationContext,
) -> Result<ConnectionPlan, Failure> {
    let (driver, credentials) = driver_config(project, config, context)?;
    let certificate = certificate(project, config)?;
    let identity = harness_domain::hash(
        json!([
            project.root,
            config.fingerprint,
            credentials,
            certificate.as_deref().map(harness_domain::hash)
        ])
        .to_string()
        .as_bytes(),
    );
    Ok(ConnectionPlan {
        driver,
        certificate,
        identity,
    })
}
