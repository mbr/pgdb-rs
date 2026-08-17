#![doc = include_str!("../README.md")]

use std::{
    env,
    ffi::OsString,
    os::unix::process::ExitStatusExt,
    process::{self, ExitStatus},
    thread,
};

use clap::Parser;
use signal_hook::{
    consts::{SIGHUP, SIGINT, SIGTERM},
    iterator::Signals,
};
use url::Url;

/// Create a temporary postgres database with one user owning a single DB.
#[derive(Debug, Parser)]
#[command(trailing_var_arg = true)]
struct Opts {
    /// Use TCP instead of a Unix socket.
    #[arg(short, long)]
    tcp: bool,
    /// TCP port to use; implies --tcp.
    #[arg(short, long)]
    port: Option<u16>,
    /// Username for regular database user.
    #[arg(short, long, default_value = "dev")]
    user: String,
    /// Password for regular database user.
    #[arg(short = 'P', long, default_value = "dev")]
    password: String,
    /// Name of regular user-owned database.
    #[arg(short, long, default_value = "dev")]
    db: String,
    /// Password for the superuser ("postgres") account, default is to generate randomly.
    #[arg(short = 'S', long)]
    superuser_pw: Option<String>,
    /// Command to run with the temporary database.
    #[arg(name = "command")]
    command: Vec<OsString>,
}

/// Runs an action while the configured database is available.
fn with_database<T>(
    opts: &Opts,
    action: impl FnOnce(&Url, &Url, bool) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    if let Ok(external_url_str) = env::var("PGDB_TESTS_URL") {
        let external_url = Url::parse(&external_url_str)?;
        if external_url.scheme() != "postgres" {
            anyhow::bail!("PGDB_TESTS_URL must use postgres:// scheme");
        }

        let _tmp_dir = tempfile::TempDir::new()?;
        pgdb::create_user_and_database(&external_url, &opts.db, &opts.user, &opts.password)?;

        let mut user_url = external_url.clone();
        user_url
            .set_username(&opts.user)
            .expect("Failed to set username");
        user_url
            .set_password(Some(&opts.password))
            .expect("Failed to set password");
        user_url.set_path(&opts.db);

        action(&external_url, &user_url, true)
    } else {
        let mut builder = pgdb::Postgres::build();

        if let Some(superuser_pw) = &opts.superuser_pw {
            builder.superuser_pw(superuser_pw);
        }
        if opts.tcp || opts.port.is_some() {
            builder.tcp();
            if opts.command.is_empty() {
                builder.port(opts.port.unwrap_or(15432));
            } else if let Some(port) = opts.port {
                builder.port(port);
            }
        }

        let pg = builder.start()?;
        pg.as_superuser().create_user(&opts.user, &opts.password)?;
        pg.as_superuser().create_database(&opts.db, &opts.user)?;

        let superuser_url = pg.as_superuser().url("postgres");
        let user_url = pg.as_user(&opts.user, &opts.password).url(&opts.db);
        action(&superuser_url, &user_url, false)
    }
}

/// Runs a command with connection details for the configured database.
fn run_command(opts: &Opts, user_url: &Url, mut signals: Signals) -> anyhow::Result<ExitStatus> {
    let (program, arguments) = opts
        .command
        .split_first()
        .expect("command must contain a program");

    let host = pgdb::connection_host(user_url).expect("URL must have a host");
    let port = pgdb::connection_port(user_url).unwrap_or(5432);
    let mut child = process::Command::new(program)
        .args(arguments)
        .env("DATABASE_URL", user_url.as_str())
        .env("PGHOST", host.as_ref())
        .env("PGPORT", port.to_string())
        .env("PGUSER", &opts.user)
        .env("PGPASSWORD", &opts.password)
        .env("PGDATABASE", &opts.db)
        .spawn()?;

    let child_pid = child.id() as libc::pid_t;
    let signal_handle = signals.handle();
    let signal_forwarder = thread::spawn(move || {
        for signal in signals.forever() {
            // SAFETY: The PID comes from the child and `Signals` only yields registered signals.
            unsafe {
                libc::kill(child_pid, signal);
            }
        }
    });

    let status = child.wait();
    signal_handle.close();
    signal_forwarder
        .join()
        .expect("signal forwarding thread must not panic");
    Ok(status?)
}

/// Exits with the status returned by a wrapped command.
fn exit_with_status(status: ExitStatus) -> ! {
    let code = status
        .code()
        .unwrap_or_else(|| 128 + status.signal().expect("exit status must have a signal"));
    process::exit(code)
}

/// Main entry point, read the `README.md` instead.
fn main() -> anyhow::Result<()> {
    let opts = Opts::parse();
    let signals = Signals::new([SIGHUP, SIGINT, SIGTERM])?;

    if !opts.command.is_empty() {
        let status = with_database(&opts, |_, user_url, _| {
            run_command(&opts, user_url, signals)
        })?;
        exit_with_status(status);
    }

    let mut signals = signals;
    with_database(&opts, |superuser_url, user_url, external| {
        println!();
        if external {
            println!("Connected to external PostgreSQL instance.");
        } else {
            println!("Postgres is now running and ready to accept connections.");
        }
        println!();
        println!(
            "PGHOST={}",
            pgdb::connection_host(superuser_url).expect("URL must have a host")
        );
        println!(
            "PGPORT={}",
            pgdb::connection_port(superuser_url).unwrap_or(5432)
        );
        println!("Superuser access:\n\n    {superuser_url}");
        println!(
            "\nA database named `{}`, owned by a user `{}` has been created.\n",
            opts.db, opts.user
        );
        println!("Regular user access:\n\n    {user_url}");
        println!("\nYou can run `psql` with either URL to connect.");
        if external {
            println!("\n(Using external PostgreSQL instance from PGDB_TESTS_URL)");
        }

        let _ = signals.forever().next();
        Ok(())
    })
}
