# pgdb

A small Rust library to create and run Postgres databases, typically used as ephemeral unit test fixtures or persistent development databases.

## Quick start

Tests requiring a fresh database (but not cluster) instance can use `db_fixture`:

```rust
let db_url = pgdb::db_fixture();
// You can now use `db_url` in your ORM. The database will not be shut down before `db_url` is dropped.
```

Note that databases are not cleaned up until the testing process exits.

Local instances use isolated Unix sockets by default, avoiding TCP port allocation. Call
`PostgresBuilder::tcp()` or configure a host or port to use TCP instead.

Requires that regular Postgres database utilities like `postgres` and `initdb` are available on the path at runtime.

## Detailed usage

`pgdb` supports configuring and starting a Postgres database instance through a builder pattern, with cleanup on `Drop`:

```
let user = "dev";
let pw = "devpw";
let db = "dev";

// Run a postgres instance.
let pg = pgdb::Postgres::build()
    .start()
    .expect("could not build postgres database");

// We can now create a regular user and a database.
pg.as_superuser()
    .create_user(user, pw)
    .expect("could not create normal user");

pg.as_superuser()
    .create_database(db, user)
    .expect("could not create normal user's db");

// Now we can run DDL commands, e.g. creating a table.
let client = pg.as_user(user, pw);
client
    .run_sql(db, "CREATE TABLE foo (id INT PRIMARY KEY);")
    .expect("could not run table creation command");
```

## Persistent clusters

Set `PostgresBuilder::data_dir()` to retain and reuse a cluster. The process still shuts down on
`Drop`, but the supplied directory is preserved. Use `start_with_database()` to include an
application user/database in the saved setup:

```no_run
use pgdb::setup::DatabaseOptions;

let pg = pgdb::Postgres::build()
    .data_dir(".pgdb")
    .start_with_database(&DatabaseOptions::default())
    .expect("could not start development database");
let database = pg.database().expect("application database was requested");
let url = pg.as_user(&database.user, &database.password).url(&database.name);
```

`DatabaseOptions` accepts optional `name`, `user`, and `password` overrides. Unspecified values
come from saved setup, or default to `dev` for a new cluster. Explicit overrides, including
`superuser_pw()`, must match saved credentials on restart. `start()` also supports persistent
clusters, but initializes only the admin account; users/databases created afterward are not
recorded in the setup file.

Both temporary and persistent clusters write `pgdb.json` after successful setup. It stores admin
credentials and the optional application database credentials in plaintext with permissions
`0600`. Keep persistent directories private and out of version control. Runtime server settings
are not saved. An existing setup file is deserialized and authenticated, not used to reset passwords
or recreate missing databases.

Without `pgdb.json`, startup runs `initdb`, which refuses a nonempty directory. Corrupt JSON,
failed authentication, and interrupted initialization return errors without deleting or repairing
persistent data. Remove the directory explicitly to start fresh. `fast()` cannot be combined with
an explicit data directory. PostgreSQL major-version upgrades must be managed separately.

Note that `psql` does use the Postgres command line tools (`psql`, `initdb`) over a library, offering a higher range of
compatibility across Postgres versions.