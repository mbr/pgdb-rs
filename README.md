# pgdb-rs

A small rust crate that allow easy creation and running of temporary Postgres databases, typically used for unit tests
or similar things.

## Quick start

If a regular postgres database including tools like `initdb` is available on the path at runtime, the default
convenience function can be used:

```rust
let db_uri = pgdb::db_fixture();
assert_eq!(db_uri.as_str(), "postgres://fixture_user_1:fixture_pass_1@127.0.0.1:25432/fixture_db_1");
// You can now use `db_uri` in your ORM. The database will not be shut down before `db_uri` is dropped.
```

Note that databases are not cleaned up until the testing process exist.

## Detailed usage

`pgdb` supports configuring and starting a Postgres database instance through a builder pattern, with cleanup on `Drop`.

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