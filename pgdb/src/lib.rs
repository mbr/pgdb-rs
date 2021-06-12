//! Runs Postgres instances.
//!
//! `pgdb` supports configuring and starting a Postgres database instance through a builder pattern,
//! with shutdown and cleanup on `Drop`.
//!
//! # Example
//!
//! ```
//! let user = "dev";
//! let pw = "devpw";
//! let db = "dev";
//!
//! // Run a postgres instance on port `15432`.
//! let pg = pgdb::Postgres::build()
//!     .start()
//!     .expect("could not build postgres database");
//!
//! // We can now create a regular user and a database.
//! pg.as_superuser()
//!   .create_user(user, pw)
//!   .expect("could not create normal user");
//!
//! pg.as_superuser()
//!   .create_database(db, user)
//!   .expect("could not create normal user's db");
//!
//! // Now we can run DDL commands, e.g. creating a table.
//! let client = pg.as_user(user, pw);
//! client
//!     .run_sql(db, "CREATE TABLE foo (id INT PRIMARY KEY);")
//!     .expect("could not run table creation command");
//! ```

use std::{
    fs, io, net, path, process, thread,
    time::{Duration, Instant},
};

use process_guard::ProcessGuard;
use rand::{rngs::OsRng, Rng};
use thiserror::Error;

/// A wrapped postgres instance.
///
/// Contains a handle to a running Postgres process. Once dropped, the instance will be shut down
/// and the temporary directory containing all of its data removed.
#[derive(Debug)]
pub struct Postgres {
    /// Host address of the instance.
    host: String,
    /// Port the instance is running on.
    port: u16,
    /// Instance of the postgres process.
    instance: ProcessGuard,
    /// Path to the `psql` binary.
    psql_binary: path::PathBuf,
    /// Superuser name.
    superuser: String,
    /// Superuser's password.
    superuser_pw: String,
    /// Directory holding all the temporary data.
    tmp_dir: tempfile::TempDir,
}

/// A virtual client for a running postgres.
///
/// Contains credentials and enough information to connect to its parent instance.
#[derive(Debug)]
pub struct PostgresClient<'a> {
    instance: &'a Postgres,
    /// Superuser name.
    username: String,
    /// Superuser password.
    password: String,
}

/// Builder for a postgres instance.
///
/// Usually constructed via [`Postgres::build`].
#[derive(Debug, Default)]
pub struct PostgresBuilder {
    /// Data directory.
    data_dir: Option<path::PathBuf>,
    /// Listening port.
    port: Option<u16>,
    /// Bind host.
    host: String,
    /// Name of the super user.
    superuser: String,
    /// Password for the super user.
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
        self.as_user(&self.superuser, &self.superuser_pw)
    }

    /// Returns a postgres client that uses the given credentials.
    #[inline]
    pub fn as_user(&self, username: &str, password: &str) -> PostgresClient<'_> {
        PostgresClient {
            instance: self,
            username: username.to_string(),
            password: password.to_string(),
        }
    }

    /// Returns the hostname the Postgres database can be connected to at.
    #[inline]
    pub fn host(&self) -> &str {
        self.host.as_str()
    }

    /// Returns the port the Postgres database is bound to.
    #[inline]
    pub fn port(&self) -> u16 {
        self.port
    }
}

impl<'a> PostgresClient<'a> {
    /// Runs a `psql` command against the database.
    ///
    /// Creates a command that runs `psql -h (host) -p (port) -U (username) -d (database)` with
    /// `PGPASSWORD` set.
    pub fn psql(&self, database: &str) -> process::Command {
        let mut cmd = process::Command::new(&self.instance.psql_binary);

        cmd.arg("-h")
            .arg(&self.instance.host)
            .arg("-p")
            .arg(self.instance.port.to_string())
            .arg("-U")
            .arg(&self.username)
            .arg("-d")
            .arg(database)
            .env("PGPASSWORD", &self.password);

        cmd
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
        self.username.as_str()
    }

    /// Returns a libpq-style connection URI.
    pub fn uri(&self, database: &str) -> String {
        format!(
            "postgres://{}:{}@{}:{}/{}",
            self.username,
            self.password,
            self.instance.host(),
            self.instance.port(),
            database
        )
    }

    /// Returns the password used by this client.
    #[inline]
    pub fn password(&self) -> &str {
        self.password.as_str()
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
    pub fn probe_delay(mut self, probe_delay: Duration) -> Self {
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
    pub fn startup_timeout(mut self, startup_timeout: Duration) -> Self {
        self.startup_timeout = startup_timeout;
        self
    }

    /// Starts the Postgres server.
    ///
    /// Postgres will start using a newly created temporary directory as its data dir. The function
    /// will only return once a TCP connection to postgres has been made successfully.
    pub fn start(&self) -> Result<Postgres, Error> {
        // If not set, we will use the default port of 15432.
        let port = self.port.unwrap_or(15432);

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
            .args(&[
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
            .arg("-c")
            .arg(format!("port={}", port))
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

        Ok(Postgres {
            host: self.host.clone(),
            port,
            instance,
            psql_binary,
            superuser: self.superuser.clone(),
            superuser_pw: self.superuser_pw.clone(),
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
