//! Persisted cluster state.

use std::{fs, io, os::unix::fs::OpenOptionsExt, path::Path};

use serde::{Deserialize, Serialize};

use crate::Error;

/// PostgreSQL login credentials.
#[derive(Clone, Deserialize, Serialize)]
pub struct Credentials {
    /// Role name.
    pub user: String,
    /// Role password.
    pub password: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("user", &self.user)
            .finish_non_exhaustive()
    }
}

/// Saved cluster and database settings.
#[derive(Deserialize, Serialize)]
pub struct State {
    /// Administrative credentials.
    pub admin: Credentials,
    /// Optional regular-user credentials.
    pub user: Option<Credentials>,
    /// Optional database name.
    pub database: Option<String>,
}

impl State {
    /// Loads `pgdb.json`, returning `None` when it does not exist.
    pub fn load(data_dir: &Path) -> Result<Option<Self>, Error> {
        let bytes = match fs::read(data_dir.join("pgdb.json")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(Error::ReadSetup(error)),
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(Error::ParseSetup)
    }

    /// Writes state to `pgdb.json`.
    pub fn store(&self, data_dir: &Path) -> Result<(), Error> {
        let file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(data_dir.join("pgdb.json"))
            .map_err(Error::WriteSetup)?;
        serde_json::to_writer_pretty(file, self).map_err(Error::SerializeSetup)
    }
}
