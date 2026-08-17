# pgdb

PostgreSQL fixtures for tests and development, no containers!

`pgdb` starts PostgreSQL in a temporary directory, waits until it is ready, and cleans it up when the work is done. It is available both as a Rust test-fixture library and as a command that can give any program a fresh database.

## Rust test fixtures

`pgdb` began as a small library for database-backed tests. Each fixture gets a fresh database with random credentials, backed by a PostgreSQL instance managed by the test process:

```rust
let database = pgdb::db_fixture();

// Pass database.as_str() to SQLx, Diesel, or your application.
```

Fixtures use private Unix sockets by default, so separate tests and test processes can run without competing for TCP ports.

## Entire runtime setup

If your application's database is "just use Postgres", this may be all the development setup you need:

```sh
pgdb cargo run
```

`pgdb` supplies `DATABASE_URL`, `PGHOST`, `PGPORT`, `PGUSER`, `PGPASSWORD`, and `PGDATABASE`. It waits for the application and removes the database afterward.

The wrapped command does not have to be Rust. Any program that accepts a PostgreSQL URL or the standard `PG*` variables can use the same workflow.

## Script support

A script can declare its own temporary PostgreSQL database with a shebang:

```sh
#!/usr/bin/env -S pgdb /bin/sh
set -eu

cargo sqlx migrate run
cargo sqlx prepare
cargo test
```

Running the script causes `pgdb` to create the database, export its connection details, run the script remembering its exit status and cleaning up on exit.

## In a larger environment

For an application with a backend, workers, or other cooperating processes, `pgdb` can be the PostgreSQL service, e.g. in a [Process Compose](https://github.com/F1bonacc1/process-compose) setup:

```yaml
version: "0.5"

env_cmds:
  APP_DB_PORT: ephemeral-port-reserve

processes:
  postgres:
    command: pgdb --tcp --port $${APP_DB_PORT}
    readiness_probe:
      exec:
        command: pg_isready --host 127.0.0.1 --port $${APP_DB_PORT} --username dev --dbname dev

  backend:
    command: >-
      DATABASE_URL=postgres://dev:dev@127.0.0.1:$${APP_DB_PORT}/dev
      cargo run
    depends_on:
      postgres:
        condition: process_healthy
```

TCP gives every process in the stack a stable, shared connection address.

## Isolated migration tests

Run migration and schema tooling against a genuinely blank database:

```sh
pgdb cargo sqlx migrate run
```

## Ad-hoc disposable playgrounds

Need to quickly test something on postgres? Open `psql` directly:

```sh
pgdb psql
```

Or open a shell in which every PostgreSQL tool already knows how to reach the temporary database:

```sh
pgdb bash
```

## Reusing a postgres instance

Local PostgreSQL processes are the default, but they are not required. Set `PGDB_TESTS_URL` to a PostgreSQL server with superuser credentials and `pgdb` will create fixture databases there instead:

```sh
PGDB_TESTS_URL=postgres://postgres:secret@localhost/postgres cargo test
```

This lets the same tests use an ephemeral local instance on a developer machine and an existing PostgreSQL service in CI. The supplied URL identifies the server used to create fixtures; `pgdb` does not treat an existing application database as disposable.

## Getting started

Install the command-line application from crates.io:

```sh
cargo install pgdb_cli
```

Or add the library to a Rust project:

```sh
cargo add --dev pgdb
```

PostgreSQL executables such as `postgres`, `initdb`, `pg_isready`, and `psql` must be available in `PATH`. The repository's Nix flake exports `pgdb` for use in development shells; add PostgreSQL from your package set alongside it.

See the [`pgdb` library documentation](./pgdb/README.md) and [`pgdb_cli` documentation](./pgdb_cli/README.md) for the complete APIs and command-line options.
