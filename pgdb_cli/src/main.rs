#![doc = include_str!("../README.md")]

use std::{
    env,
    ffi::OsString,
    os::unix::process::ExitStatusExt,
    process::{self, ExitStatus},
    thread,
    time::Duration,
};

use signal_hook::{
    consts::{SIGHUP, SIGINT, SIGTERM},
    iterator::Signals,
};
use structopt::{clap::AppSettings, StructOpt};
use url::Url;

/// Create a temporary postgres database with one user owning a single DB.
#[derive(Debug, StructOpt)]
#[structopt(setting = AppSettings::TrailingVarArg)]
struct Opts {
    /// Database port to use.
    #[structopt(short, long)]
    port: Option<u16>,
    /// Username for regular database user.
    #[structopt(short, long, default_value = "dev")]
    user: String,
    /// Password for regular database user.
    #[structopt(short = "P", long, default_value = "dev")]
    password: String,
    /// Name of regular user-owned database.
    #[structopt(short, long, default_value = "dev")]
    db: String,
    /// Password for the superuser ("postgres") account, default is to generate randomly.
    #[structopt(short = "S", long)]
    superuser_pw: Option<String>,
    /// Command to run with the temporary database.
    #[structopt(name = "command", parse(from_os_str))]
    command: Vec<OsString>,
}

/// Runs an action while the configured database is available.
fn with_database<T>(
    opts: &Opts,
    local_port: Option<u16>,
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
        if let Some(port) = local_port {
            builder.port(port);
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
fn run_command(opts: &Opts, user_url: &Url) -> anyhow::Result<ExitStatus> {
    let (program, arguments) = opts
        .command
        .split_first()
        .expect("command must contain a program");

    let mut signals = Signals::new([SIGHUP, SIGINT, SIGTERM])?;
    let mut child = process::Command::new(program)
        .args(arguments)
        .env("DATABASE_URL", user_url.as_str())
        .env("PGHOST", user_url.host_str().expect("URL must have a host"))
        .env("PGPORT", user_url.port().unwrap_or(5432).to_string())
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
    let opts = Opts::from_args();

    if !opts.command.is_empty() {
        let status = with_database(&opts, opts.port, |_, user_url, _| {
            run_command(&opts, user_url)
        })?;
        exit_with_status(status);
    }

    with_database(
        &opts,
        Some(opts.port.unwrap_or(15432)),
        |superuser_url, user_url, external| {
            println!();
            if external {
                println!("Connected to external PostgreSQL instance.");
            } else {
                println!("Postgres is now running and ready to accept connections.");
            }
            println!();
            println!(
                "PGHOST={}",
                superuser_url.host_str().expect("URL must have a host")
            );
            println!("PGPORT={}", superuser_url.port().unwrap_or(5432));
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

            loop {
                thread::sleep(Duration::from_secs(60));
            }
        },
    )
}
