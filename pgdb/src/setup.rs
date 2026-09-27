//! Saved cluster credentials.

use std::{fs, io, os::unix::fs::OpenOptionsExt, path::Path};

use serde::{Deserialize, Serialize};

use crate::Error;

/// Administrative credentials needed to restart a cluster.
#[derive(Deserialize, Serialize)]
pub(crate) struct ClusterSetup {
    /// Administrative role.
    pub superuser: String,
    /// Administrative password.
    pub superuser_pw: String,
}

impl ClusterSetup {
    /// Reads saved credentials, distinguishing missing files from invalid ones.
    pub fn read(data_dir: &Path) -> Result<Option<Self>, Error> {
        let bytes = match fs::read(data_dir.join("pgdb.json")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(Error::ReadSetup(error)),
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(Error::ParseSetup)
    }

    /// Writes credentials after cluster initialization succeeds.
    pub fn write(&self, data_dir: &Path) -> Result<(), Error> {
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(data_dir.join("pgdb.json"))
            .map_err(Error::WriteSetup)?;
        serde_json::to_writer_pretty(file, self).map_err(Error::SerializeSetup)
    }
}
