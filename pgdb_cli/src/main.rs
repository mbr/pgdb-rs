use std::{thread, time::Duration};

use structopt::StructOpt;

/// Create a temporary postgres database with one user owning a single DB.
#[derive(Debug, StructOpt)]
struct Opts {
    /// Database port to use.
    #[structopt(short, long)]
    port: Option<u16>,
    /// Username for regular database user, defaults to `dev`.
    #[structopt(short, long, default_value = "dev")]
    user: String,
    /// Password for regular database user, defaults to `dev`.
    #[structopt(short = "P", long, default_value = "dev")]
    password: String,
    /// Name of regular user-owned database, defaults to `dev`.
    #[structopt(short, long, default_value = "dev")]
    db: String,
}

fn main() -> anyhow::Result<()> {
    let opts = Opts::from_args();

    let pg = pgdb::Postgres::build().start()?;
    pg.as_superuser().create_user(&opts.user, &opts.password)?;
    pg.as_superuser().create_database(&opts.db, &opts.user)?;

    println!();
    println!("Postgres is now running and ready to accept connections.");
    println!();
    println!("PGHOST={}", pg.host());
    println!("PGPORT={}", pg.port());

    println!(
        "Superuser access:\n\n    {}",
        pg.as_superuser().uri("postgres")
    );

    println!(
        "\nA database named `{}`, owned by a user `{}` has been created.\n",
        opts.db, opts.user
    );

    println!(
        "Regular user access:\n\n    {}",
        pg.as_user(&opts.user, &opts.password).uri(&opts.db)
    );

    println!("\nYou can run `psql` with either URI to connect.");
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}
