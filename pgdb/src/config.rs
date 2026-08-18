//! Environment configuration for PostgreSQL instances.

use std::time::Duration;

use serde::Deserialize;

use crate::PostgresBuilder;

/// Environment-derived overrides for a PostgreSQL instance.
#[derive(Debug, Default, Deserialize)]
pub struct PostgresEnvironment {
    /// Whether to use fast settings for disposable servers.
    #[serde(default)]
    fast: bool,
    /// Whether to use TCP.
    #[serde(default)]
    tcp: bool,
    /// TCP port to use.
    port: Option<u16>,
    /// Password for the superuser.
    superuser_pw: Option<String>,
    /// Maximum startup duration in seconds.
    startup_timeout: Option<u64>,
    /// Maximum graceful shutdown duration in seconds.
    shutdown_timeout: Option<u64>,
    /// Maximum forceful shutdown duration in seconds.
    force_shutdown_timeout: Option<u64>,
    /// Whether external test fixtures should be cleaned up on drop.
    tests_cleanup: Option<bool>,
}

impl PostgresEnvironment {
    /// Reads PostgreSQL configuration from the process environment.
    pub fn read() -> Result<Self, envy::Error> {
        envy::prefixed("PGDB_").from_env()
    }

    /// Applies the configured overrides to a PostgreSQL builder.
    pub fn apply(&self, builder: &mut PostgresBuilder) {
        if self.fast {
            builder.fast();
        }
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
            builder.startup_timeout(Duration::from_secs(startup_timeout));
        }
        if let Some(shutdown_timeout) = self.shutdown_timeout {
            builder.shutdown_timeout(Duration::from_secs(shutdown_timeout));
        }
        if let Some(force_shutdown_timeout) = self.force_shutdown_timeout {
            builder.force_shutdown_timeout(Duration::from_secs(force_shutdown_timeout));
        }
    }

    /// Returns whether external test fixtures should be cleaned up on drop.
    pub fn tests_cleanup(&self) -> bool {
        self.tests_cleanup.unwrap_or(true)
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

#[cfg(test)]
mod tests {
    use super::PostgresEnvironment;

    #[test]
    fn parses_prefixed_environment() {
        let environment = envy::prefixed("PGDB_")
            .from_iter::<_, PostgresEnvironment>(vec![
                ("PGDB_FAST".to_string(), "true".to_string()),
                ("PGDB_TCP".to_string(), "true".to_string()),
                ("PGDB_PORT".to_string(), "15432".to_string()),
                ("PGDB_SUPERUSER_PW".to_string(), "secret".to_string()),
                ("PGDB_STARTUP_TIMEOUT".to_string(), "11".to_string()),
                ("PGDB_SHUTDOWN_TIMEOUT".to_string(), "7".to_string()),
                ("PGDB_FORCE_SHUTDOWN_TIMEOUT".to_string(), "2".to_string()),
                ("PGDB_TESTS_CLEANUP".to_string(), "false".to_string()),
                ("PGDB_USER".to_string(), "ignored".to_string()),
            ])
            .expect("environment must be valid");

        assert!(environment.fast);
        assert!(environment.tcp);
        assert_eq!(environment.port, Some(15432));
        assert_eq!(environment.superuser_pw.as_deref(), Some("secret"));
        assert_eq!(environment.startup_timeout, Some(11));
        assert_eq!(environment.shutdown_timeout, Some(7));
        assert_eq!(environment.force_shutdown_timeout, Some(2));
        assert!(!environment.tests_cleanup());
        assert!(PostgresEnvironment::default().tests_cleanup());
    }
}
