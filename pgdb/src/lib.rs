#![doc = include_str!("../README.md")]

use std::{
    fs, io, net,
    net::TcpListener,
    path, process,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex, Weak,
    },
    thread,
    time::{Duration, Instant},
};

use process_guard::ProcessGuard;
use rand::{rngs::OsRng, Rng};
use thiserror::Error;
use url::Url;

/// A database URI keeping a database alive.
///
/// Contains the output of [`PostgresClient::uri`] and a reference to the database it points to. As
/// a result, as long as the [`DbUri`] is alive, the database it points to will also be kept
/// running.
#[derive(Debug)]
pub struct DbUri {
    /// A reference to the running Postgres instance where this URI points.
    _arc: Arc<Postgres>,
    /// The actual URI.
    uri: String,
}

impl DbUri {
    /// Returns the
    pub fn as_str(&self) -> &str {
        self.uri.as_str()
    }
}

impl AsRef<str> for DbUri {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// A convenience function for regular applications.
///
/// Some applications just need a clean database instance and can afford to share the underlying
/// database.
///
/// Uses a shared database instance if multiple tests are running at the same time (see [`DbUri`]
/// for details). The database may be shut down and recreated if the last [`DbUri`] is dropped
/// during testing, e.g. when parallel tests are not spawned quick enough.
///
/// This construction is necessary because `static` variables will not have `Drop` called on them,
/// without this construction, the spawned Postgres server would not be stopped.
pub fn db_fixture() -> DbUri {
    static DB: Mutex<Weak<Postgres>> = Mutex::new(Weak::new());

    static FIXTURE_COUNT: AtomicUsize = AtomicUsize::new(1);

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

    let count = FIXTURE_COUNT.fetch_add(1, Ordering::Relaxed);
    let db_name = format!("fixture_db_{}", count);
    let db_user = format!("fixture_user_{}", count);
    let db_pw = format!("fixture_pass_{}", count);
    pg.as_superuser()
        .create_user(&db_user, &db_pw)
        .expect("failed to create user for fixture DB");
    pg.as_superuser()
        .create_database(&db_name, &db_user)
        .expect("failed to create database for fixture DB");
    let uri = pg.as_user(&db_user, &db_pw).uri(&db_name);
    DbUri { _arc: pg, uri }
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

/// A Postgres server error.
#[derive(Error, Debug)]
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

    /// Returns the hostname the Postgres database can be connected to at.
    #[inline]
    pub fn host(&self) -> &str {
        self.superuser_url
            .host_str()
            .expect("Postgres URL must have a host")
    }

    /// Returns the port the Postgres database is bound to.
    #[inline]
    pub fn port(&self) -> u16 {
        self.superuser_url
            .port()
            .expect("Postgres URL must have a port")
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
        let password = self
            .client_url
            .password()
            .expect("Client URL must have a password");

        cmd.arg("-h")
            .arg(self.instance.host())
            .arg("-p")
            .arg(self.instance.port().to_string())
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

    /// Returns the username used by this client.
    pub fn username(&self) -> &str {
        self.client_url.username()
    }

    /// Returns a libpq-style connection URI.
    pub fn uri(&self, database: &str) -> String {
        let mut url = self.client_url.clone();
        url.set_path(database);
        url.to_string()
    }

    /// Returns the password used by this client.
    #[inline]
    pub fn password(&self) -> &str {
        self.client_url
            .password()
            .expect("Client URL must have a password")
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
    /// If no port is set, the builder will attempt to find an unused through binding port `0`. This
    /// is somewhat racing, but the only recourse, since Postgres does not support binding to port
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

/// Generates a random hex string string 32 characters long.
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
        assert_eq!(su.password, "helloworld");
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

        assert_ne!(a.port(), b.port());
        assert_ne!(a.port(), c.port());
        assert_ne!(b.port(), c.port());
    }

    #[test]
    fn ensure_proper_db_reuse_when_using_fixtures() {
        let db_uri = crate::db_fixture();
        assert_eq!(
            &db_uri.as_str()[..51],
            "postgres://fixture_user_1:fixture_pass_1@127.0.0.1:"
        );

        // Calling `db_fixture` multiple times reuses the postgres process, but creates a fresh database instance and role.
        let db_uri2 = crate::db_fixture();
        assert_eq!(
            &db_uri2.as_str()[..51],
            "postgres://fixture_user_2:fixture_pass_2@127.0.0.1:"
        );
    }
}
