#![doc = include_str!("../README.md")]

pub mod config;
mod db_instance;
mod error;

use std::{
    borrow::Cow,
    env, fs, io,
    net::TcpListener,
    path, process, thread,
    time::{Duration, Instant},
};

pub use db_instance::{db_fixture, DbInstance};
pub use error::{Error, ExternalUrlError};
use percent_encoding::{percent_decode_str, utf8_percent_encode, NON_ALPHANUMERIC};
use process_guard::{ProcessGuard, ShutdownPolicy, Signal, DEFAULT_FORCE_TIME};
use url::Url;

/// Default PostgreSQL port and Unix socket suffix.
const DEFAULT_POSTGRES_PORT: u16 = 5432;

/// Returns the connection host represented by a PostgreSQL URL.
pub fn connection_host(url: &Url) -> Option<Cow<'_, str>> {
    if let Some((_, host)) = url.query_pairs().find(|(key, _)| key == "host") {
        return Some(host);
    }

    url.host_str()
        .map(|host| percent_decode_str(host).decode_utf8_lossy())
}

/// Returns the connection port represented by a PostgreSQL URL.
pub fn connection_port(url: &Url) -> Option<u16> {
    url.query_pairs()
        .find(|(key, _)| key == "port")
        .and_then(|(_, port)| port.parse().ok())
        .or_else(|| url.port())
}

/// Executes SQL using psql with the given connection parameters.
pub fn run_psql_command(superuser_url: &Url, database: &str, sql: &str) -> Result<(), Error> {
    // TODO: Do not use which, allow passing in.
    let psql_binary = which::which("psql").unwrap_or_else(|_| "psql".into());
    let username = superuser_url.username();
    let password = superuser_url.password().unwrap_or_default();
    let host = connection_host(superuser_url).expect("URL must have a host");
    let port = connection_port(superuser_url).unwrap_or(5432);

    let status = process::Command::new(&psql_binary)
        .arg("-h")
        .arg(host.as_ref())
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
    /// PostgreSQL process and its temporary directory.
    #[allow(dead_code)] // Only used for its `Drop` implementation.
    process: PostgresProcess,
    /// Path to the `psql` binary.
    psql_binary: path::PathBuf,
}

/// Resources owned by a PostgreSQL process.
#[derive(Debug)]
struct PostgresProcess {
    /// Guard for the PostgreSQL process group leader.
    instance: ProcessGuard,
    /// Directory holding all temporary data.
    tmp_dir: tempfile::TempDir,
}

impl Drop for PostgresProcess {
    fn drop(&mut self) {
        if self.instance.shutdown().is_err() {
            self.tmp_dir.disable_cleanup(true);
            // Do not repeat the bounded shutdown when the guard is dropped.
            let _ = self.instance.take();
        }
    }
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
    /// TCP listening port.
    ///
    /// If not set, [`find_unused_port`] will be used to determine the port.
    port: Option<u16>,
    /// TCP bind host.
    host: String,
    /// Whether to connect over TCP.
    tcp: bool,
    /// Name of the superuser.
    superuser: String,
    /// Password for the superuser.
    superuser_pw: String,
    /// Path to `postgres` binary.
    postgres_binary: Option<path::PathBuf>,
    /// Path to `initdb` binary.
    initdb_binary: Option<path::PathBuf>,
    /// Path to `pg_isready` binary.
    pg_isready_binary: Option<path::PathBuf>,
    /// Path to `psql` binary.
    psql_binary: Option<path::PathBuf>,
    /// PostgreSQL server configuration overrides.
    postgres_options: Vec<(String, String)>,
    /// How long to wait between startup probe attempts.
    probe_delay: Duration,
    /// Time until giving up waiting for startup.
    startup_timeout: Duration,
    /// Time to allow graceful shutdown before forceful cleanup.
    shutdown_timeout: Duration,
    /// Time to wait for forceful cleanup to complete.
    force_shutdown_timeout: Duration,
}

impl Postgres {
    /// Creates a new Postgres database builder.
    #[inline]
    pub fn build() -> PostgresBuilder {
        PostgresBuilder {
            data_dir: None,
            port: None,
            host: "127.0.0.1".to_string(),
            tcp: false,
            superuser: "postgres".to_string(),
            superuser_pw: generate_random_string(),
            postgres_binary: None,
            initdb_binary: None,
            pg_isready_binary: None,
            psql_binary: None,
            postgres_options: Vec::new(),
            probe_delay: Duration::from_millis(100),
            startup_timeout: Duration::from_secs(10),
            shutdown_timeout: Duration::from_secs(5),
            force_shutdown_timeout: DEFAULT_FORCE_TIME,
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

        let host = connection_host(&self.client_url).expect("Client URL must have a host");
        let port = connection_port(&self.client_url).expect("Client URL must have a port");

        cmd.arg("-h")
            .arg(host.as_ref())
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

    /// Sets the location of the `pg_isready` binary.
    #[inline]
    pub fn pg_isready_binary<T: Into<path::PathBuf>>(&mut self, pg_isready_binary: T) -> &mut Self {
        self.pg_isready_binary = Some(pg_isready_binary.into());
        self
    }

    /// Sets the TCP bind address and enables TCP connections.
    #[inline]
    pub fn host(&mut self, host: String) -> &mut Self {
        self.host = host;
        self.tcp = true;
        self
    }

    /// Sets the TCP listening port and enables TCP connections.
    ///
    /// If no port is set, the builder will attempt to find an unused port through binding to port `0`. This
    /// is somewhat racy, but the only recourse, since Postgres does not support binding to port
    /// `0`.
    #[inline]
    pub fn port(&mut self, port: u16) -> &mut Self {
        self.port = Some(port);
        self.tcp = true;
        self
    }

    /// Enables TCP connections on an automatically selected port.
    #[inline]
    pub fn tcp(&mut self) -> &mut Self {
        self.tcp = true;
        self
    }

    /// Sets the location of the `postgres` binary.
    #[inline]
    pub fn postgres_binary<T: Into<path::PathBuf>>(&mut self, postgres_binary: T) -> &mut Self {
        self.postgres_binary = Some(postgres_binary.into());
        self
    }

    /// Adds a PostgreSQL server configuration override.
    ///
    /// The option is passed to `postgres` as `-c NAME=VALUE`.
    #[inline]
    pub fn postgres_option<K, V>(&mut self, name: K, value: V) -> &mut Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        self.postgres_options.push((name.into(), value.into()));
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

    /// Sets the maximum time to wait for graceful shutdown.
    #[inline]
    pub fn shutdown_timeout(&mut self, shutdown_timeout: Duration) -> &mut Self {
        self.shutdown_timeout = shutdown_timeout;
        self
    }

    /// Sets the maximum time to wait for forceful shutdown.
    #[inline]
    pub fn force_shutdown_timeout(&mut self, force_shutdown_timeout: Duration) -> &mut Self {
        self.force_shutdown_timeout = force_shutdown_timeout;
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
    /// will only return once `pg_isready` reports the server is accepting connections.
    pub fn start(&self) -> Result<Postgres, Error> {
        let port = if self.tcp {
            self.port
                .unwrap_or_else(|| find_unused_port().expect("failed to find an unused port"))
        } else {
            DEFAULT_POSTGRES_PORT
        };

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
        let pg_isready_binary = self
            .pg_isready_binary
            .clone()
            .map(Ok)
            .unwrap_or_else(|| which::which("pg_isready").map_err(Error::FindPgIsready))?;
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
        for (name, value) in &self.postgres_options {
            postgres_command.arg("-c").arg(format!("{name}={value}"));
        }
        if self.tcp {
            postgres_command.arg("-h").arg(&self.host);
        } else {
            postgres_command.arg("-c").arg("listen_addresses=");
        }

        let instance = ProcessGuard::spawn_process_group(
            &mut postgres_command,
            ShutdownPolicy::Graceful {
                signal: Signal::SIGINT,
                grace_time: self.shutdown_timeout,
                force_time: self.force_shutdown_timeout,
            },
        )
        .map_err(Error::LaunchPostgres)?;
        let process = PostgresProcess { instance, tmp_dir };

        // Wait for the server to become ready to accept connections.
        let started = Instant::now();
        loop {
            let mut pg_isready_command = process::Command::new(&pg_isready_binary);
            pg_isready_command.arg("-h");
            if self.tcp {
                pg_isready_command.arg(&self.host);
            } else {
                pg_isready_command.arg(process.tmp_dir.path());
            }
            let status = pg_isready_command
                .arg("-p")
                .arg(port.to_string())
                .stdout(process::Stdio::null())
                .stderr(process::Stdio::null())
                .status();

            match status {
                Ok(exit_status) if exit_status.success() => break,
                _ => {
                    if started.elapsed() >= self.startup_timeout {
                        return Err(Error::StartupTimeout);
                    }
                    thread::sleep(self.probe_delay);
                }
            }
        }

        let mut superuser_url = if self.tcp {
            Url::parse(&format!("postgres://{}:{}", self.host, port))
                .expect("Failed to construct TCP URL")
        } else {
            let socket_dir = process.tmp_dir.path().to_string_lossy();
            let encoded_socket_dir = utf8_percent_encode(&socket_dir, NON_ALPHANUMERIC).to_string();
            Url::parse(&format!("postgres://{encoded_socket_dir}:{port}"))
                .expect("Failed to construct Unix socket URL")
        };
        superuser_url
            .set_username(&self.superuser)
            .expect("Failed to set superuser username");
        superuser_url
            .set_password(Some(&self.superuser_pw))
            .expect("Failed to set superuser password");

        Ok(Postgres {
            superuser_url,
            process,
            psql_binary,
        })
    }
}

/// Generates a random hex string 32 characters long.
fn generate_random_string() -> String {
    let raw: [u8; 16] = rand::random();
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

/// Parses the `PGDB_TESTS_URL` environment variable if set.
///
/// The URL must be a complete Postgres URL with superuser credentials.
///
/// Returns `Ok(Some(url))` if valid, `Ok(None)` if not set, or `Err` if invalid.
pub fn parse_external_test_url() -> Result<Option<Url>, Error> {
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use url::Url;

    use super::Postgres;

    #[test]
    fn connection_parameters_decode_socket_urls() {
        let url =
            Url::parse("postgres://dev@%2Ftmp%2Fpgdb:5432/dev").expect("socket URL must be valid");

        assert_eq!(super::connection_host(&url).as_deref(), Some("/tmp/pgdb"));
        assert_eq!(super::connection_port(&url), Some(5432));
    }

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
    fn instances_support_isolated_sockets_and_tcp() {
        let a = Postgres::build()
            .start()
            .expect("could not build postgres database");
        let b = Postgres::build()
            .start()
            .expect("could not build postgres database");
        let tcp = Postgres::build()
            .tcp()
            .start()
            .expect("could not build TCP postgres database");

        let a_host = super::connection_host(a.superuser_url()).expect("URL must have a host");
        let b_host = super::connection_host(b.superuser_url()).expect("URL must have a host");
        let tcp_host = super::connection_host(tcp.superuser_url()).expect("URL must have a host");

        assert!(a_host.starts_with('/'));
        assert!(b_host.starts_with('/'));
        assert_ne!(a_host, b_host);
        assert_eq!(super::connection_port(a.superuser_url()), Some(5432));
        assert_eq!(super::connection_port(b.superuser_url()), Some(5432));
        assert_eq!(tcp_host, "127.0.0.1");
    }

    #[test]
    fn forceful_shutdown_waits_before_removing_temporary_directory() {
        let pg = Postgres::build()
            .shutdown_timeout(Duration::ZERO)
            .start()
            .expect("could not build postgres database");
        let temporary_directory = pg.process.tmp_dir.path().to_path_buf();

        drop(pg);

        assert!(!temporary_directory.exists());
    }

    #[test]
    fn ensure_proper_db_reuse_when_using_fixtures() {
        let db_url = crate::db_fixture();
        let db_url2 = crate::db_fixture();

        match (&db_url, &db_url2) {
            (crate::DbInstance::Local { .. }, crate::DbInstance::Local { .. }) => {
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
            (crate::DbInstance::External { .. }, crate::DbInstance::External { .. }) => {
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
                crate::DbInstance::External { url, .. } => {
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
