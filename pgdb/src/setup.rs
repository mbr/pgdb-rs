//! Saved cluster setup and credentials.

use std::{fs, io, os::unix::fs::OpenOptionsExt, path::Path};

use serde::{Deserialize, Serialize};

use crate::{generate_random_string, Error};

/// Optional overrides for the database created during cluster setup.
///
/// Unspecified values use saved settings, or `dev` for a new cluster.
#[derive(Default)]
pub struct DatabaseOptions {
    /// Database name.
    pub name: Option<String>,
    /// Database owner.
    pub user: Option<String>,
    /// Owner's password.
    pub password: Option<String>,
}

/// Credentials for the database created during cluster setup.
#[derive(Deserialize, Serialize)]
pub struct Database {
    /// Database name.
    pub name: String,
    /// Database owner.
    pub user: String,
    /// Owner's password.
    pub password: String,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Database")
            .field("name", &self.name)
            .field("user", &self.user)
            .finish_non_exhaustive()
    }
}

/// Completed cluster setup, including credentials needed on restart.
#[derive(Deserialize, Serialize)]
pub(crate) struct ClusterSetup {
    /// Administrative role.
    pub superuser: String,
    /// Administrative password.
    pub superuser_pw: String,
    /// Optional database created as part of initialization.
    pub database: Option<Database>,
}

impl ClusterSetup {
    /// Reads completed setup, distinguishing missing files from invalid ones.
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

    /// Resolves credentials for a new cluster.
    pub fn new(superuser_pw: Option<&str>, database: Option<&DatabaseOptions>) -> Self {
        Self {
            superuser: "postgres".to_string(),
            superuser_pw: superuser_pw
                .map(str::to_owned)
                .unwrap_or_else(generate_random_string),
            database: database.map(|options| Database {
                name: options.name.clone().unwrap_or_else(|| "dev".to_string()),
                user: options.user.clone().unwrap_or_else(|| "dev".to_string()),
                password: options
                    .password
                    .clone()
                    .unwrap_or_else(|| "dev".to_string()),
            }),
        }
    }

    /// Rejects explicit settings that differ from completed setup.
    pub fn validate(
        &self,
        superuser_pw: Option<&str>,
        database: Option<&DatabaseOptions>,
    ) -> Result<(), Error> {
        check_setting("superuser password", superuser_pw, &self.superuser_pw)?;
        if let Some(options) = database {
            let saved = self.database.as_ref().ok_or(Error::MissingSetupDatabase)?;
            check_setting("database name", options.name.as_deref(), &saved.name)?;
            check_setting("database user", options.user.as_deref(), &saved.user)?;
            check_setting(
                "database password",
                options.password.as_deref(),
                &saved.password,
            )?;
        }
        Ok(())
    }

    /// Writes credentials after all initialization has succeeded.
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

/// Checks an explicit setting without exposing credential values in errors.
fn check_setting(name: &'static str, supplied: Option<&str>, saved: &str) -> Result<(), Error> {
    if supplied.is_some_and(|value| value != saved) {
        return Err(Error::ConflictingSetup(name));
    }
    Ok(())
}
