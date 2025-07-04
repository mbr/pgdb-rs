#![doc = include_str!("../README.md")]

use std::{thread, time::Duration};

use structopt::StructOpt;

/// Create a temporary postgres database with one user owning a single DB.
#[derive(Debug, StructOpt)]
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
}

/// Main entry point, read the `README.md` instead.
fn main() -> anyhow::Result<()> {
    let opts = Opts::from_args();

    let mut builder = pgdb::Postgres::build();
    if let Some(superuser_pw) = opts.superuser_pw {
        builder.superuser_pw(superuser_pw);
    }

    // Select a default port that does not clash with the default port of `pgdb`, in case it is used
    // by unit tests.
    builder.port(opts.port.unwrap_or(15432));

    let pg = builder.start()?;
    pg.as_superuser().create_user(&opts.user, &opts.password)?;
    pg.as_superuser().create_database(&opts.db, &opts.user)?;

    println!();
    println!("Postgres is now running and ready to accept connections.");
    println!();
    let superuser_url = pg.superuser_url();
    println!(
        "PGHOST={}",
        superuser_url.host_str().expect("URL must have a host")
    );
    println!(
        "PGPORT={}",
        superuser_url.port().expect("URL must have a port")
    );

    println!(
        "Superuser access:\n\n    {}",
        pg.as_superuser().url("postgres")
    );

    println!(
        "\nA database named `{}`, owned by a user `{}` has been created.\n",
        opts.db, opts.user
    );

    println!(
        "Regular user access:\n\n    {}",
        pg.as_user(&opts.user, &opts.password).url(&opts.db)
    );

    println!("\nYou can run `psql` with either URL to connect.");
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}
