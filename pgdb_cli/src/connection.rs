//! Connection discovery for running CLI instances.

use std::{
    fs::File,
    path::Path,
    process::{Command, Stdio},
};

use anyhow::Context;
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use tempfile::{NamedTempFile, TempPath};
use url::Url;

/// Application connection metadata.
#[derive(Deserialize, Serialize)]
struct Connection {
    /// Authenticated URL of the running application database.
    url: Url,
}

/// Publishes private connection metadata and removes it when the guard is dropped.
pub fn publish(directory: &Path, url: &Url) -> anyhow::Result<TempPath> {
    let path = TempPath::try_from_path(directory.join("connection.json"))?;
    let mut file = NamedTempFile::new_in(directory)?;
    serde_json::to_writer(file.as_file_mut(), &Connection { url: url.clone() })?;
    file.persist(&path)?;
    Ok(path)
}

/// Loads the running endpoint without modifying the state directory.
pub fn load(directory: &Path) -> anyhow::Result<Url> {
    let path = directory.join("connection.json");
    let file = File::open(&path).with_context(|| {
        format!(
            "cannot read {}; is the owning pgdb running?",
            path.display()
        )
    })?;
    let connection: Connection =
        serde_json::from_reader(file).with_context(|| format!("invalid {}", path.display()))?;
    let url = connection.url;
    anyhow::ensure!(
        url.scheme() == "postgres"
            && pgdb::connection_host(&url).is_some_and(|host| !host.is_empty())
            && !url.username().is_empty()
            && !url.path().trim_start_matches('/').is_empty(),
        "invalid connection URL in {}",
        path.display()
    );
    Ok(url)
}

/// Exports the application URL and its decoded PostgreSQL connection parameters.
pub fn configure(command: &mut Command, url: &Url) -> anyhow::Result<()> {
    command
        .env("DATABASE_URL", url.as_str())
        .env(
            "PGHOST",
            pgdb::connection_host(url)
                .context("missing connection host")?
                .as_ref(),
        )
        .env(
            "PGPORT",
            pgdb::connection_port(url).unwrap_or(5432).to_string(),
        )
        .env(
            "PGUSER",
            percent_decode_str(url.username()).decode_utf8()?.as_ref(),
        )
        .env(
            "PGPASSWORD",
            percent_decode_str(url.password().unwrap_or_default())
                .decode_utf8()?
                .as_ref(),
        )
        .env(
            "PGDATABASE",
            percent_decode_str(url.path().trim_start_matches('/'))
                .decode_utf8()?
                .as_ref(),
        )
        .env_remove("PGHOSTADDR")
        .env_remove("PGSERVICE");
    Ok(())
}

/// Checks authentication once, with bounded connection and statement timeouts.
pub fn check(url: &Url) -> anyhow::Result<()> {
    let mut command = Command::new("psql");
    configure(&mut command, url)?;
    let status = command
        .args(["-Xw", "-v", "ON_ERROR_STOP=1", "-c", "SELECT 1"])
        .env("PGCONNECT_TIMEOUT", "3")
        .env("PGOPTIONS", "-c statement_timeout=3000")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .context("could not check the running database with psql")?;
    anyhow::ensure!(
        status.success(),
        "cannot connect to the running database; connection metadata may be stale"
    );
    Ok(())
}
