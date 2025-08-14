//! Reference counted database handles.
//!
//! A [`DbInstance`] represents ownership of a running database instance with cleanup semantics.
//! Dropping the [`DbInstance`] will cause the underlying database to be dropped.

use std::{process, sync::Arc};

use url::Url;

use crate::Postgres;

/// A database instance.
///
/// Can be either a local database (with a reference to the running instance) or an external
/// database URL. Dropping the instance will cause the database to be deleted.
#[derive(Debug)]
pub enum DbInstance {
    /// A local database instance that will be kept alive as long as this DbUrl exists.
    Local {
        /// A reference to the running Postgres instance where this URL points.
        _arc: Arc<Postgres>,
        /// The actual URL.
        url: Url,
    },
    /// An external database URL with admin credentials to clean it up later.
    External {
        /// The database URL.
        url: Url,
        /// The superuser URL for cleanup operations.
        superuser_url: Url,
    },
}

impl DbInstance {
    /// Returns the URL as a string.
    pub fn as_str(&self) -> &str {
        match self {
            DbInstance::Local { url, .. } => url.as_str(),
            DbInstance::External { url, .. } => url.as_str(),
        }
    }

    /// Returns the URL.
    pub fn as_url(&self) -> &Url {
        match self {
            DbInstance::Local { url, .. } => url,
            DbInstance::External { url, .. } => url,
        }
    }
}

impl AsRef<str> for DbInstance {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Drop for DbInstance {
    fn drop(&mut self) {
        if let DbInstance::External { url, superuser_url } = self {
            // Extract database and usernames from the URL
            let db_name = url.path().trim_start_matches('/');
            let db_user = url.username();

            // Best effort cleanup - we don't want to panic in [`Drop::drop`].
            // TODO: Do not use `which` here if a different `psql` binary was configured.
            let psql_binary = which::which("psql").unwrap_or_else(|_| "psql".into());

            // Helper to run cleanup SQL
            let run_cleanup_sql = |sql: &str| {
                let username = superuser_url.username();
                let password = superuser_url.password().unwrap_or_default();

                // TODO: Should not assume defaults here, look up where `DbUrl` is actually built.
                let host = superuser_url.host_str().unwrap_or("localhost");
                let port = superuser_url.port().unwrap_or(5432);

                let _ = process::Command::new(&psql_binary)
                    .arg("-h")
                    .arg(host)
                    .arg("-p")
                    .arg(port.to_string())
                    .arg("-U")
                    .arg(username)
                    .arg("-d")
                    .arg("postgres")
                    .arg("-c")
                    .arg(sql)
                    .env("PGPASSWORD", password)
                    .output();
            };

            // Drop database first (this will fail if there are active connections)
            run_cleanup_sql(&format!(
                "DROP DATABASE IF EXISTS {};",
                crate::escape_ident(db_name)
            ));

            // Drop user
            run_cleanup_sql(&format!(
                "DROP ROLE IF EXISTS {};",
                crate::escape_ident(db_user)
            ));
        }
    }

    // TODO: Clean up database if local.
}
