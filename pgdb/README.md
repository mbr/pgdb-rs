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

`data_dir()` preserves the cluster on drop; `start_with_database()` creates or reuses its application
database and credentials:

```no_run
use pgdb::setup::DatabaseOptions;

let pg = pgdb::Postgres::build()
    .data_dir(".pgdb")
    .start_with_database(&DatabaseOptions::default())
    .expect("could not start development database");
```

`DatabaseOptions` defaults to saved settings, or `dev` for a new cluster. Explicit credentials must
match saved values. Plain `start()` initializes only the admin account; later setup isn't recorded.
Both modes save plaintext credentials in `pgdb.json` (`0600`). Keep the directory out of Git.
Invalid setup returns an error without repair or deletion. `data_dir()` cannot be combined with
`fast()`.

Note that `psql` does use the Postgres command line tools (`psql`, `initdb`) over a library, offering a higher range of
compatibility across Postgres versions.