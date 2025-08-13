#![doc = include_str!("../README.md")]

use std::{
    env, fs, io, net,
    net::TcpListener,
    path, process,
    sync::{Arc, Mutex, Weak},
    thread,
    time::{Duration, Instant},
};

use process_guard::ProcessGuard;
use rand::{rngs::OsRng, Rng};
use thiserror::Error;
use url::Url;

/// A database URL keeping a database alive.
///
/// Can be either a local database (with a reference to the running instance) or an external
/// database URL.
#[derive(Debug)]
pub enum DbUrl {
    /// A local database instance that will be kept alive as long as this DbUrl exists.
    Local {
        /// A reference to the running Postgres instance where this URL points.
        _arc: Arc<Postgres>,
        /// The actual URL.
        url: Url,
    },
    /// An external database URL with cleanup information.
    External {
        /// The database URL.
        url: Url,
        /// The superuser URL for cleanup operations.
        superuser_url: Url,
    },
}

impl DbUrl {
    /// Returns the URL as a string.
    pub fn as_str(&self) -> &str {
        match self {
            DbUrl::Local { url, .. } => url.as_str(),
            DbUrl::External { url, .. } => url.as_str(),
        }
    }

    /// Returns the URL.
    pub fn as_url(&self) -> &Url {
        match self {
            DbUrl::Local { url, .. } => url,
            DbUrl::External { url, .. } => url,
        }
    }
}

impl AsRef<str> for DbUrl {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Drop for DbUrl {
    fn drop(&mut self) {
        if let DbUrl::External { url, superuser_url } = self {
            // Extract database and user names from the URL
            let db_name = url.path().trim_start_matches('/');
            let db_user = url.username();

            // Best effort cleanup - we don't want to panic in drop
            let psql_binary = which::which("psql").unwrap_or_else(|_| "psql".into());

            // Helper to run cleanup SQL
            let run_cleanup_sql = |sql: &str| {
                let username = superuser_url.username();
                let password = superuser_url.password().unwrap_or_default();
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
                escape_ident(db_name)
            ));

            // Drop user
            run_cleanup_sql(&format!("DROP ROLE IF EXISTS {};", escape_ident(db_user)));
        }
    }
}

/// Parses the `PGDB_TESTS_URL` environment variable if set.
///
/// The URL must be a complete Postgres URL with superuser credentials.
///
/// Returns `Ok(Some(url))` if valid, `Ok(None)` if not set, or `Err` if invalid.
fn parse_external_test_url() -> Result<Option<Url>, Error> {
    match env::var("PGDB_TESTS_URL") {
        Ok(url_str) => {
            let url = Url::parse(&url_str)
                .map_err(|e| Error::InvalidExternalUrl(ExternalUrlError::ParseError(e)))?;

            if url.scheme() != "postgres" {
                return Err(Error::InvalidExternalUrl(ExternalUrlError::InvalidScheme));
            }

            if url.host_str().is_none() {
                return Err(Error::InvalidExternalUrl(ExternalUrlError::MissingHost));
            }

            if url.username().is_empty() {
                return Err(Error::InvalidExternalUrl(ExternalUrlError::MissingUsername));
            }

            Ok(Some(url))
        }
        Err(_) => Ok(None),
    }
}

/// Executes SQL using psql with the given connection parameters.
pub fn run_psql_command(superuser_url: &Url, database: &str, sql: &str) -> Result<(), Error> {
    let psql_binary = which::which("psql").unwrap_or_else(|_| "psql".into());
    let username = superuser_url.username();
    let password = superuser_url.password().unwrap_or_default();
    let host = superuser_url.host_str().expect("URL must have a host");
    let port = superuser_url.port().unwrap_or(5432);

    let status = process::Command::new(&psql_binary)
        .arg("-h")
        .arg(host)
        .arg("-p")
        .arg(port.to_string())
        .arg("-U")
        .arg(username)
        .arg("-d")
        .arg(database)
        .arg("-c")
        .arg(sql)
        .env("PGPASSWORD", password)
        .status()
        .map_err(Error::RunPsql)?;

    if !status.success() {
        return Err(Error::PsqlFailed(status));
    }

    Ok(())
}

/// Creates a user and database with the given credentials using psql.
pub fn create_user_and_database(
    superuser_url: &Url,
    db_name: &str,
    db_user: &str,
    db_pw: &str,
) -> Result<(), Error> {
    // Create user
    run_psql_command(
        superuser_url,
        "postgres",
        &format!(
            "CREATE ROLE {} LOGIN ENCRYPTED PASSWORD {};",
            escape_ident(db_user),
            escape_string(db_pw)
        ),
    )?;

    // Create database
    run_psql_command(
        superuser_url,
        "postgres",
        &format!(
            "CREATE DATABASE {} OWNER {};",
            escape_ident(db_name),
            escape_ident(db_user)
        ),
    )?;

    Ok(())
}

/// Creates a new fixture database with random credentials.
fn create_fixture_db(superuser_url: &Url) -> Result<Url, Error> {
    // Generate unique credentials with random IDs
    let random_id = generate_random_string();
    let db_name = format!("fixture_db_{}", random_id);
    let db_user = format!("fixture_user_{}", random_id);
    let db_pw = format!("fixture_pass_{}", random_id);

    // Create user and database
    create_user_and_database(superuser_url, &db_name, &db_user, &db_pw)?;

    // Build the URL for the new database
    let mut url = superuser_url.clone();
    url.set_username(&db_user).expect("Failed to set username");
    url.set_password(Some(&db_pw))
        .expect("Failed to set password");
    url.set_path(&db_name);

    Ok(url)
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
/// [`DbUrl`] for details). The database may be shut down and recreated if the last [`DbUrl`] is
/// dropped during testing, e.g. when parallel tests are not spawned quick enough.
///
/// This construction is necessary because `static` variables will not have `Drop` called on them,
/// without this construction, the spawned Postgres server would not be stopped.
pub fn db_fixture() -> DbUrl {
    // Check for external database URL first
    if let Some(external_url) = parse_external_test_url().expect("invalid PGDB_TESTS_URL") {
        let url = create_fixture_db(&external_url).expect("failed to create external fixture DB");
        return DbUrl::External {
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
    let url = create_fixture_db(pg.superuser_url()).expect("failed to create local fixture DB");
    DbUrl::Local { _arc: pg, url }
}

/// Finds an unused port by binding to port 0 and letting the OS assign one.
///
/// This function has a race condition, there is no guarantee that the OS won't reassign the port as
/// soon as it is released again. Sadly this is our only recourse, as Postgres does not allow
/// passing `0` as the port number.
fn find_unused_port() -> io::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    Ok(port)
}

/// A wrapped postgres instance.
///
/// Contains a handle to a running Postgres process. Once dropped, the instance will be shut down
/// and the temporary directory containing all of its data removed.
#[derive(Debug)]
pub struct Postgres {
    /// URL for the instance with superuser credentials.
    superuser_url: Url,
    /// Instance of the postgres process.
    #[allow(dead_code)] // Only used for its `Drop` implementation.
    instance: ProcessGuard,
    /// Path to the `psql` binary.
    psql_binary: path::PathBuf,
    /// Directory holding all the temporary data.
    #[allow(dead_code)] // Only used for its `Drop` implementation.
    tmp_dir: tempfile::TempDir,
}

/// A virtual client for a running postgres.
///
/// Contains credentials and enough information to connect to its parent instance.
#[derive(Debug)]
pub struct PostgresClient<'a> {
    instance: &'a Postgres,
    /// Client URL with credentials.
    client_url: Url,
}

/// Builder for a postgres instance.
///
/// Usually constructed via [`Postgres::build`].
#[derive(Debug)]
pub struct PostgresBuilder {
    /// Data directory.
    data_dir: Option<path::PathBuf>,
    /// Listening port.
    ///
    /// If not set, [`find_unused_port`] will be used to determine the port.
    port: Option<u16>,
    /// Bind host.
    host: String,
    /// Name of the superuser.
    superuser: String,
    /// Password for the superuser.
    superuser_pw: String,
    /// Path to `postgres` binary.
    postgres_binary: Option<path::PathBuf>,
    /// Path to `initdb` binary.
    initdb_binary: Option<path::PathBuf>,
    /// Path to `psql` binary.
    psql_binary: Option<path::PathBuf>,
    /// How long to wait between startup probe attempts.
    probe_delay: Duration,
    /// Time until giving up waiting for startup.
    startup_timeout: Duration,
}

/// Errors that can occur when parsing an external database URL.
#[derive(Debug, Error)]
pub enum ExternalUrlError {
    /// URL parsing failed.
    #[error("invalid URL: {0}")]
    ParseError(#[source] url::ParseError),
    /// Wrong URL scheme.
    #[error("must use postgres:// scheme")]
    InvalidScheme,
    /// Missing host.
    #[error("must include a host")]
    MissingHost,
    /// Missing username.
    #[error("must include a username")]
    MissingUsername,
}

/// A Postgres server error.
#[derive(Debug, Error)]
pub enum Error {
    #[error("could not find `postgres` binary")]
    FindPostgres(which::Error),
    /// Failed to find the `initdb` binary.
    #[error("could not find `initdb` binary")]
    FindInitdb(which::Error),
    /// Failed to find the `postgres` binary.
    #[error("could not find `psql` binary")]
    FindPsql(which::Error),
    /// Could not create the temporary directory.
    #[error("could not create temporary directory for database")]
    CreateDatabaseDir(io::Error),
    /// Could not write the temporary password to a file.
    #[error("error writing temporary password")]
    WriteTemporaryPw(io::Error),
    /// Starting `initdb` failed.
    #[error("failed to run `initdb`")]
    RunInitDb(io::Error),
    /// Running `initdb` was not successful.
    #[error("`initdb` exited with status {}", 0)]
    InitDbFailed(process::ExitStatus),
    /// Postgres could not be launched.
    #[error("failed to launch `postgres`")]
    LaunchPostgres(io::Error),
    /// Postgres was launched but failed to bring up a TCP-connection accepting socket in time.
    #[error("timeout probing tcp socket")]
    StartupTimeout,
    /// `psql` could not be launched.
    #[error("failed to run `psql`")]
    RunPsql(io::Error),
    /// Running `psql` returned an error.
    #[error("`psql` exited with status {}", 0)]
    PsqlFailed(process::ExitStatus),
    /// Invalid external test URL.
    #[error("invalid PGDB_TESTS_URL")]
    InvalidExternalUrl(#[source] ExternalUrlError),
}

impl Postgres {
    /// Creates a new Postgres database builder.
    #[inline]
    pub fn build() -> PostgresBuilder {
        PostgresBuilder {
            data_dir: None,
            port: None,
            host: "127.0.0.1".to_string(),
            superuser: "postgres".to_string(),
            superuser_pw: generate_random_string(),
            postgres_binary: None,
            initdb_binary: None,
            psql_binary: None,
            probe_delay: Duration::from_millis(100),
            startup_timeout: Duration::from_secs(10),
        }
    }

    /// Returns a postgres client with superuser credentials.
    #[inline]
    pub fn as_superuser(&self) -> PostgresClient<'_> {
        PostgresClient {
            instance: self,
            client_url: self.superuser_url.clone(),
        }
    }

    /// Returns a postgres client that uses the given credentials.
    #[inline]
    pub fn as_user(&self, username: &str, password: &str) -> PostgresClient<'_> {
        let mut client_url = self.superuser_url.clone();
        client_url
            .set_username(username)
            .expect("Failed to set username");
        client_url
            .set_password(Some(password))
            .expect("Failed to set password");
        PostgresClient {
            instance: self,
            client_url,
        }
    }

    /// Returns the superuser URL for this instance.
    pub fn superuser_url(&self) -> &Url {
        &self.superuser_url
    }
}

impl<'a> PostgresClient<'a> {
    /// Runs a `psql` command against the database.
    ///
    /// Creates a command that runs `psql -h (host) -p (port) -U (username) -d (database)` with
    /// `PGPASSWORD` set.
    pub fn psql(&self, database: &str) -> process::Command {
        let mut cmd = process::Command::new(&self.instance.psql_binary);

        let username = self.client_url.username();
        let password = self.client_url.password().unwrap_or_default();

        let host = self
            .client_url
            .host_str()
            .expect("Client URL must have a host");
        let port = self.client_url.port().expect("Client URL must have a port");

        cmd.arg("-h")
            .arg(host)
            .arg("-p")
            .arg(port.to_string())
            .arg("-U")
            .arg(username)
            .arg("-d")
            .arg(database)
            .env("PGPASSWORD", password);

        cmd
    }

    /// Runs the given SQL commands from an input file via `psql`.
    pub fn load_sql<P: AsRef<path::Path>>(&self, database: &str, filename: P) -> Result<(), Error> {
        let status = self
            .psql(database)
            .arg("-f")
            .arg(filename.as_ref())
            .status()
            .map_err(Error::RunPsql)?;

        if !status.success() {
            return Err(Error::PsqlFailed(status));
        }

        Ok(())
    }

    /// Runs the given SQL command through `psql`.
    pub fn run_sql(&self, database: &str, sql: &str) -> Result<(), Error> {
        let status = self
            .psql(database)
            .arg("-c")
            .arg(sql)
            .status()
            .map_err(Error::RunPsql)?;

        if !status.success() {
            return Err(Error::PsqlFailed(status));
        }

        Ok(())
    }

    /// Creates a new database with the given owner.
    ///
    /// This typically requires superuser credentials, see [`Postgres::as_superuser`].
    #[inline]
    pub fn create_database(&self, database: &str, owner: &str) -> Result<(), Error> {
        self.run_sql(
            "postgres",
            &format!(
                "CREATE DATABASE {} OWNER {};",
                escape_ident(database),
                escape_ident(owner)
            ),
        )
    }

    /// Creates a new user on the system that is allowed to login.
    ///
    /// This typically requires superuser credentials, see [`Postgres::as_superuser`].
    #[inline]
    pub fn create_user(&self, username: &str, password: &str) -> Result<(), Error> {
        self.run_sql(
            "postgres",
            &format!(
                "CREATE ROLE {} LOGIN ENCRYPTED PASSWORD {};",
                escape_ident(username),
                escape_string(password)
            ),
        )
    }

    /// Returns the `Postgres` instance associated with this client.
    #[inline]
    pub fn instance(&self) -> &Postgres {
        self.instance
    }

    /// Returns a libpq-style connection URL.
    pub fn url(&self, database: &str) -> Url {
        let mut url = self.client_url.clone();
        url.set_path(database);
        url
    }

    /// Returns the client URL for this client.
    pub fn client_url(&self) -> &Url {
        &self.client_url
    }
}

impl PostgresBuilder {
    /// Sets the postgres data directory.
    ///
    /// If not set, a temporary directory will be used.
    #[inline]
    pub fn data_dir<T: Into<path::PathBuf>>(&mut self, data_dir: T) -> &mut Self {
        self.data_dir = Some(data_dir.into());
        self
    }

    /// Sets the location of the `initdb` binary.
    #[inline]
    pub fn initdb_binary<T: Into<path::PathBuf>>(&mut self, initdb_binary: T) -> &mut Self {
        self.initdb_binary = Some(initdb_binary.into());
        self
    }

    /// Sets the bind address.
    #[inline]
    pub fn host(&mut self, host: String) -> &mut Self {
        self.host = host;
        self
    }

    /// Sets listening port.
    ///
    /// If no port is set, the builder will attempt to find an unused port through binding to port `0`. This
    /// is somewhat racy, but the only recourse, since Postgres does not support binding to port
    /// `0`.
    #[inline]
    pub fn port(&mut self, port: u16) -> &mut Self {
        self.port = Some(port);
        self
    }

    /// Sets the location of the `postgres` binary.
    #[inline]
    pub fn postgres_binary<T: Into<path::PathBuf>>(&mut self, postgres_binary: T) -> &mut Self {
        self.postgres_binary = Some(postgres_binary.into());
        self
    }

    /// Sets the startup probe delay.
    ///
    /// Between two startup probes, waits this long.
    #[inline]
    pub fn probe_delay(&mut self, probe_delay: Duration) -> &mut Self {
        self.probe_delay = probe_delay;
        self
    }

    /// Sets the location of the `psql` binary.
    #[inline]
    pub fn psql_binary<T: Into<path::PathBuf>>(&mut self, psql_binary: T) -> &mut Self {
        self.psql_binary = Some(psql_binary.into());
        self
    }

    /// Sets the maximum time to probe for startup.
    #[inline]
    pub fn startup_timeout(&mut self, startup_timeout: Duration) -> &mut Self {
        self.startup_timeout = startup_timeout;
        self
    }

    /// Sets the password for the superuser.
    #[inline]
    pub fn superuser_pw<T: Into<String>>(&mut self, superuser_pw: T) -> &mut Self {
        self.superuser_pw = superuser_pw.into();
        self
    }

    /// Starts the Postgres server.
    ///
    /// Postgres will start using a newly created temporary directory as its data dir. The function
    /// will only return once a TCP connection to postgres has been made successfully.
    pub fn start(&self) -> Result<Postgres, Error> {
        let port = self
            .port
            .unwrap_or_else(|| find_unused_port().expect("failed to find an unused port"));

        let postgres_binary = self
            .postgres_binary
            .clone()
            .map(Ok)
            .unwrap_or_else(|| which::which("postgres").map_err(Error::FindPostgres))?;
        let initdb_binary = self
            .initdb_binary
            .clone()
            .map(Ok)
            .unwrap_or_else(|| which::which("initdb").map_err(Error::FindInitdb))?;
        let psql_binary = self
            .psql_binary
            .clone()
            .map(Ok)
            .unwrap_or_else(|| which::which("psql").map_err(Error::FindPsql))?;

        let tmp_dir = tempfile::tempdir().map_err(Error::CreateDatabaseDir)?;
        let data_dir = self
            .data_dir
            .clone()
            .unwrap_or_else(|| tmp_dir.path().join("db"));

        let superuser_pw_file = tmp_dir.path().join("superuser-pw");
        fs::write(&superuser_pw_file, self.superuser_pw.as_bytes())
            .map_err(Error::WriteTemporaryPw)?;

        let initdb_status = process::Command::new(initdb_binary)
            .args([
                // No default locale (== 'C').
                "--no-locale",
                // Require a password for all users.
                "--auth=md5",
                // Set default encoding to UTF8.
                "--encoding=UTF8",
                // Do not sync data, which is fine for tests.
                "--nosync",
                // Path to data directory.
                "--pgdata",
            ])
            .arg(&data_dir)
            .arg("--pwfile")
            .arg(&superuser_pw_file)
            .arg("--username")
            .arg(&self.superuser)
            .status()
            .map_err(Error::RunInitDb)?;

        if !initdb_status.success() {
            return Err(Error::InitDbFailed(initdb_status));
        }

        // Start the database.
        let mut postgres_command = process::Command::new(postgres_binary);
        postgres_command
            .arg("-D")
            .arg(&data_dir)
            .arg("-p")
            .arg(port.to_string())
            .arg("-k")
            .arg(tmp_dir.path());

        let instance = ProcessGuard::spawn_graceful(&mut postgres_command, Duration::from_secs(5))
            .map_err(Error::LaunchPostgres)?;

        // Wait for the server to come up.
        let socket_addr = format!("{}:{}", self.host, port);
        let started = Instant::now();
        loop {
            match net::TcpStream::connect(socket_addr.as_str()) {
                Ok(_) => break,
                Err(_) => {
                    let now = Instant::now();

                    if now.duration_since(started) >= self.startup_timeout {
                        return Err(Error::StartupTimeout);
                    }

                    thread::sleep(self.probe_delay);
                }
            }
        }

        let superuser_url = Url::parse(&format!(
            "postgres://{}:{}@{}:{}",
            self.superuser, self.superuser_pw, self.host, port
        ))
        .expect("Failed to construct base URL");

        Ok(Postgres {
            superuser_url,
            instance,
            psql_binary,
            tmp_dir,
        })
    }
}

/// Generates a random hex string 32 characters long.
fn generate_random_string() -> String {
    let raw: [u8; 16] = OsRng.gen();
    format!("{:x}", hex_fmt::HexFmt(&raw))
}

/// Escapes an identifier by wrapping in quote char. Any quote character inside the unescaped string
/// will be doubled.
fn quote(quote_char: char, unescaped: &str) -> String {
    let mut result = String::new();

    result.push(quote_char);
    for c in unescaped.chars() {
        if c == quote_char {
            result.push(quote_char);
            result.push(quote_char);
        } else {
            result.push(c);
        }
    }
    result.push(quote_char);

    result
}

/// Escapes an identifier.
fn escape_ident(unescaped: &str) -> String {
    quote('"', unescaped)
}

/// Escapes a string.
fn escape_string(unescaped: &str) -> String {
    quote('\'', unescaped)
}

#[cfg(test)]
mod tests {
    use super::Postgres;

    #[test]
    fn can_change_superuser_pw() {
        let pg = Postgres::build()
            .superuser_pw("helloworld")
            .start()
            .expect("could not build postgres database");

        let su = pg.as_superuser();
        su.create_user("foo", "bar")
            .expect("could not create normal user");

        // Command executed successfully, check we used the right password.
        assert_eq!(su.client_url().password(), Some("helloworld"));
    }

    #[test]
    fn instances_use_different_port_by_default() {
        let a = Postgres::build()
            .start()
            .expect("could not build postgres database");
        let b = Postgres::build()
            .start()
            .expect("could not build postgres database");
        let c = Postgres::build()
            .start()
            .expect("could not build postgres database");

        assert_ne!(
            a.superuser_url().port().expect("URL must have a port"),
            b.superuser_url().port().expect("URL must have a port")
        );
        assert_ne!(
            a.superuser_url().port().expect("URL must have a port"),
            c.superuser_url().port().expect("URL must have a port")
        );
        assert_ne!(
            b.superuser_url().port().expect("URL must have a port"),
            c.superuser_url().port().expect("URL must have a port")
        );
    }

    #[test]
    fn ensure_proper_db_reuse_when_using_fixtures() {
        let db_url = crate::db_fixture();
        let db_url2 = crate::db_fixture();

        match (&db_url, &db_url2) {
            (crate::DbUrl::Local { .. }, crate::DbUrl::Local { .. }) => {
                // When using local databases, verify they have fixture prefixes
                assert!(db_url.as_str().contains("fixture_user_"));
                assert!(db_url.as_str().contains("fixture_pass_"));
                assert!(db_url.as_str().contains("fixture_db_"));

                assert!(db_url2.as_str().contains("fixture_user_"));
                assert!(db_url2.as_str().contains("fixture_pass_"));
                assert!(db_url2.as_str().contains("fixture_db_"));

                // Verify they have different databases/users
                assert_ne!(db_url.as_str(), db_url2.as_str());
            }
            (crate::DbUrl::External { .. }, crate::DbUrl::External { .. }) => {
                // When using external database, verify separate databases are created
                assert!(db_url.as_str().contains("fixture_user_"));
                assert!(db_url.as_str().contains("fixture_pass_"));
                assert!(db_url.as_str().contains("fixture_db_"));

                assert!(db_url2.as_str().contains("fixture_user_"));
                assert!(db_url2.as_str().contains("fixture_pass_"));
                assert!(db_url2.as_str().contains("fixture_db_"));

                // Verify they have different databases/users
                assert_ne!(db_url.as_str(), db_url2.as_str());

                // But they should use the same host/port
                assert_eq!(db_url.as_url().host_str(), db_url2.as_url().host_str());
                assert_eq!(db_url.as_url().port(), db_url2.as_url().port());
            }
            _ => panic!("Inconsistent DbUrl types returned from db_fixture"),
        }
    }

    #[test]
    fn external_db_cleanup_on_drop() {
        // Only run this test when external database is configured
        if crate::parse_external_test_url().unwrap().is_none() {
            return;
        }

        let superuser_url = crate::parse_external_test_url().unwrap().unwrap();
        let psql_binary = which::which("psql").unwrap_or_else(|_| "psql".into());

        // Create a database fixture
        let (db_name, db_user) = {
            let db_url = crate::db_fixture();

            // Extract the database and user names from URL
            match &db_url {
                crate::DbUrl::External { url, .. } => {
                    let db_name = url.path().trim_start_matches('/').to_string();
                    let db_user = url.username().to_string();
                    (db_name, db_user)
                }
                _ => panic!("Expected external database"),
            }
        }; // db_url is dropped here, should trigger cleanup

        // Give Drop some time to execute
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Check if database was dropped
        let check_db_exists = |name: &str| -> bool {
            let username = superuser_url.username();
            let password = superuser_url.password().unwrap_or_default();
            let host = superuser_url.host_str().unwrap();
            let port = superuser_url.port().unwrap_or(5432);

            let output = std::process::Command::new(&psql_binary)
                .arg("-h")
                .arg(host)
                .arg("-p")
                .arg(port.to_string())
                .arg("-U")
                .arg(username)
                .arg("-d")
                .arg("postgres")
                .arg("-t")
                .arg("-c")
                .arg(format!(
                    "SELECT 1 FROM pg_database WHERE datname = '{}'",
                    name
                ))
                .env("PGPASSWORD", password)
                .output()
                .expect("Failed to check database existence");

            String::from_utf8_lossy(&output.stdout).trim() == "1"
        };

        // Check if user was dropped
        let check_user_exists = |name: &str| -> bool {
            let username = superuser_url.username();
            let password = superuser_url.password().unwrap_or_default();
            let host = superuser_url.host_str().unwrap();
            let port = superuser_url.port().unwrap_or(5432);

            let output = std::process::Command::new(&psql_binary)
                .arg("-h")
                .arg(host)
                .arg("-p")
                .arg(port.to_string())
                .arg("-U")
                .arg(username)
                .arg("-d")
                .arg("postgres")
                .arg("-t")
                .arg("-c")
                .arg(format!("SELECT 1 FROM pg_roles WHERE rolname = '{}'", name))
                .env("PGPASSWORD", password)
                .output()
                .expect("Failed to check user existence");

            String::from_utf8_lossy(&output.stdout).trim() == "1"
        };

        // Verify cleanup
        assert!(
            !check_db_exists(&db_name),
            "Database should have been dropped"
        );
        assert!(
            !check_user_exists(&db_user),
            "User should have been dropped"
        );
    }
}
