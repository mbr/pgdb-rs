# pgdb

A small Rust library to create and run ephemeral Postgres databases, typically used as unit test fixtures.

## Quick start

Tests requiring a fresh database (but not cluster) instance can use `db_fixture`:

```rust
let db_url = pgdb::db_fixture();
// You can now use `db_url` in your ORM. The database will not be shut down before `db_url` is dropped.
```

Local databases share a server while their fixtures overlap. Dropping the last fixture shuts down
that server and removes its temporary directory.

Local fixtures use fast mode by default: `fsync`, `synchronous_commit`, and `full_page_writes` are
disabled, and teardown uses `SIGQUIT` without a shutdown checkpoint. Set `PGDB_FAST=false` before
creating fixtures when testing durability or crash recovery. External servers configured through
`PGDB_TESTS_URL` and instances created directly with `Postgres::build()` do not inherit this fast-mode
default.

The default shutdown grace period is five seconds in fast mode and twenty seconds otherwise.
Override it with `PGDB_SHUTDOWN_TIMEOUT` (seconds) or `PostgresBuilder::shutdown_timeout()`.

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

Use `Postgres::build().data_dir(".pgdb").start()` to retain and reuse a cluster.

Note that `psql` does use the Postgres command line tools (`psql`, `initdb`) over a library, offering a higher range of
compatibility across Postgres versions.