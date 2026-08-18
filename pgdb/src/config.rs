//! Environment configuration for PostgreSQL instances.

use std::{env, num::ParseIntError, str::ParseBoolError, time::Duration};

use thiserror::Error;

use crate::PostgresBuilder;

/// Environment-derived overrides for a PostgreSQL instance.
#[derive(Debug, Default)]
pub struct PostgresEnvironment {
    /// Whether to use TCP.
    tcp: bool,
    /// TCP port to use.
    port: Option<u16>,
    /// Password for the superuser.
    superuser_pw: Option<String>,
    /// Maximum startup duration.
    startup_timeout: Option<Duration>,
    /// Maximum graceful shutdown duration.
    shutdown_timeout: Option<Duration>,
}

impl PostgresEnvironment {
    /// Reads PostgreSQL configuration from the process environment.
    pub fn read() -> Result<Self, EnvironmentError> {
        let tcp = read_value("PGDB_TCP")?
            .map(|value| {
                value
                    .parse()
                    .map_err(|source| EnvironmentError::InvalidBoolean {
                        name: "PGDB_TCP",
                        source,
                    })
            })
            .transpose()?
            .unwrap_or(false);
        let port = read_integer("PGDB_PORT")?;
        let superuser_pw = read_value("PGDB_SUPERUSER_PW")?;
        let startup_timeout = read_integer("PGDB_STARTUP_TIMEOUT")?.map(Duration::from_secs);
        let shutdown_timeout = read_integer("PGDB_SHUTDOWN_TIMEOUT")?.map(Duration::from_secs);

        Ok(Self {
            tcp,
            port,
            superuser_pw,
            startup_timeout,
            shutdown_timeout,
        })
    }

    /// Applies the configured overrides to a PostgreSQL builder.
    pub fn apply(&self, builder: &mut PostgresBuilder) {
        if self.tcp {
            builder.tcp();
        }
        if let Some(port) = self.port {
            builder.tcp().port(port);
        }
        if let Some(superuser_pw) = &self.superuser_pw {
            builder.superuser_pw(superuser_pw);
        }
        if let Some(startup_timeout) = self.startup_timeout {
            builder.startup_timeout(startup_timeout);
        }
        if let Some(shutdown_timeout) = self.shutdown_timeout {
            builder.shutdown_timeout(shutdown_timeout);
        }
    }

    /// Returns whether TCP was requested.
    pub fn tcp(&self) -> bool {
        self.tcp
    }

    /// Returns the configured TCP port.
    pub fn port(&self) -> Option<u16> {
        self.port
    }
}

/// An error encountered while reading PostgreSQL environment configuration.
#[derive(Debug, Error)]
pub enum EnvironmentError {
    /// An environment variable contains non-Unicode data.
    #[error("could not read {name}")]
    Read {
        /// Environment variable name.
        name: &'static str,
        /// Environment access error.
        #[source]
        source: env::VarError,
    },
    /// An environment variable does not contain a boolean.
    #[error("invalid value for {name}")]
    InvalidBoolean {
        /// Environment variable name.
        name: &'static str,
        /// Boolean parsing error.
        #[source]
        source: ParseBoolError,
    },
    /// An environment variable does not contain an integer.
    #[error("invalid value for {name}")]
    InvalidInteger {
        /// Environment variable name.
        name: &'static str,
        /// Integer parsing error.
        #[source]
        source: ParseIntError,
    },
}

/// Reads an optional Unicode environment variable.
fn read_value(name: &'static str) -> Result<Option<String>, EnvironmentError> {
    match env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(source) => Err(EnvironmentError::Read { name, source }),
    }
}

/// Reads an optional integer environment variable.
fn read_integer<T>(name: &'static str) -> Result<Option<T>, EnvironmentError>
where
    T: std::str::FromStr<Err = ParseIntError>,
{
    read_value(name)?
        .map(|value| {
            value
                .parse()
                .map_err(|source| EnvironmentError::InvalidInteger { name, source })
        })
        .transpose()
}
