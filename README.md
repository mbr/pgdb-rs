# pgdb-rs

A small rust crate that allow easy creation and running of temporary Postgres databases, typically used for unit tests or similar things:

```rust
let user = "dev";
let pw = "devpw";
let db = "dev";

// Run a postgres instance on port `15432`.
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

See the [documentation](https://docs.rs/pgdb) for details.
