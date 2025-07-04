# pgdb

A small Rust to create and run ephemeral Postgres databases, typically used as unit test fixtures.

## Quick start

Tests requiring a fresh database (but not cluster) instance can use `db_fixture`:

```rust
let db_uri = pgdb::db_fixture();
// You can now use `db_uri` in your ORM. The database will not be shut down before `db_uri` is dropped.
```

Note that databases are not cleaned up until the testing process exist.

Requires regular Postgres database utilities like `postgres` and `initdb` are available on the path at runtime.

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