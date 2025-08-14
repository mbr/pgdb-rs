//! Reference counted database handles.
//!
//! A [`DbInstance`] represents ownership of a running database instance with cleanup semantics.
//! Dropping the [`DbInstance`] will cause the underlying database to be dropped.

use std::{
    process,
    sync::{Arc, Mutex, Weak},
};

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

/// A convenience function for regular applications.
///
/// Some applications just need a clean database instance and can afford to share the underlying
/// database.
///
/// If the `PGDB_TESTS_URL` environment variable is set, it will be used as an external database
/// URL instead of creating a local instance. The URL must include superuser credentials. A new
/// database will be created for each call, just like with local instances.
///
/// Otherwise, uses a shared database instance if multiple tests are running at the same time (see
/// [`DbInstance`] for details). The database may be shut down and recreated if the last [`DbInstance`] is
/// dropped during testing, e.g. when parallel tests are not spawned quick enough.
///
/// This construction is necessary because `static` variables will not have `Drop` called on them,
/// without this construction, the spawned Postgres server would not be stopped.
pub fn db_fixture() -> DbInstance {
    // Check for external database URL first
    if let Some(external_url) = crate::parse_external_test_url().expect("invalid PGDB_TESTS_URL") {
        let url =
            crate::create_fixture_db(&external_url).expect("failed to create external fixture DB");
        return DbInstance::External {
            url,
            superuser_url: external_url,
        };
    }

    static DB: Mutex<Weak<Postgres>> = Mutex::new(Weak::new());

    let pg = {
        let mut guard = DB.lock().expect("lock poisoned");
        if let Some(arc) = guard.upgrade() {
            // We still have an instance we can reuse.
            arc
        } else {
            let arc = Arc::new(
                Postgres::build()
                    .start()
                    .expect("failed to start global postgres DB"),
            );
            *guard = Arc::downgrade(&arc);
            arc
        }
    };

    // Use unified fixture creation for local databases too
    let url =
        crate::create_fixture_db(pg.superuser_url()).expect("failed to create local fixture DB");
    DbInstance::Local { _arc: pg, url }
}
