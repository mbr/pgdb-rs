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

fn main() -> anyhow::Result<()> {
    let opts = Opts::from_args();

    let mut builder = pgdb::Postgres::build();
    if let Some(superuser_pw) = opts.superuser_pw {
        builder.superuser_pw(superuser_pw);
    }
    if let Some(port) = opts.port {
        builder.port(port);
    }

    let pg = builder.start()?;
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
