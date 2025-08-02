#![doc = include_str!("../README.md")]

use std::{env, path::PathBuf, thread, time::Duration};

use structopt::StructOpt;
use url::Url;

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
    /// Postgresql binaries path.
    #[structopt(short, long, env = "PGDB_POSTGRES_BIN")]
    postgres_bin: Option<PathBuf>,
}

/// Main entry point, read the `README.md` instead.
fn main() -> anyhow::Result<()> {
    let opts = Opts::from_args();

    // Check for external database URL
    if let Ok(external_url_str) = env::var("PGDB_TESTS_URL") {
        // Parse and validate the external URL
        let external_url = Url::parse(&external_url_str)?;

        // Validate it's a PostgreSQL URL
        if external_url.scheme() != "postgres" {
            anyhow::bail!("PGDB_TESTS_URL must use postgres:// scheme");
        }

        // Create temporary directory even for external database (as per TODO)
        let _tmp_dir = tempfile::TempDir::new()?;

        // Use the pgdb library's unified functions to create user and database
        use pgdb::create_user_and_database;

        // Create the user and database on the external server
        create_user_and_database(&external_url, &opts.db, &opts.user, &opts.password)?;

        // Build the user URL
        let mut user_url = external_url.clone();
        user_url
            .set_username(&opts.user)
            .expect("Failed to set username");
        user_url
            .set_password(Some(&opts.password))
            .expect("Failed to set password");
        user_url.set_path(&opts.db);

        println!();
        println!("Connected to external PostgreSQL instance.");
        println!();
        println!(
            "PGHOST={}",
            external_url.host_str().expect("URL must have a host")
        );
        println!("PGPORT={}", external_url.port().unwrap_or(5432));

        println!("Superuser access:\n\n    {}", external_url.as_str());

        println!(
            "\nA database named `{}`, owned by a user `{}` has been created.\n",
            opts.db, opts.user
        );

        println!("Regular user access:\n\n    {}", user_url.as_str());

        println!("\nYou can run `psql` with either URL to connect.");
        println!("\n(Using external PostgreSQL instance from PGDB_TESTS_URL)");

        loop {
            thread::sleep(Duration::from_secs(60));
        }
    } else {
        // Original local database logic
        let mut builder = pgdb::Postgres::build();

        if let Some(postgres_bin) = opts.postgres_bin {
            builder.initdb_binary(postgres_bin.join("initdb"));
            builder.postgres_binary(postgres_bin.join("postgres"));
            builder.psql_binary(postgres_bin.join("psql"));
        }

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
}
