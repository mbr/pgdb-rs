# pgdb_cli

A command-line interface for creating temporary PostgreSQL databases for development and testing.

## Installation

The easiest way to install is to do so straight from [crates.io](https://crates.io/crates/pgdb_cli):

```bash
cargo install pgdb_cli
```

## Usage

### Interactive mode

Start a temporary PostgreSQL instance:

```bash
pgdb
```

This will:

- Start PostgreSQL in a private temporary directory using a Unix socket
- Create a user `dev` with password `dev`
- Create a database `dev` owned by the user
- Display connection information
- Keep running until interrupted (Ctrl+C)

Pass `-t` or `--tcp` to use TCP instead. `--port` selects a TCP port and implies `--tcp`.
The generated socket URLs work with `psql` and SQLx.

Disposable local servers use fast mode by default: `fsync`, `synchronous_commit`, and
`full_page_writes` are disabled, and shutdown uses `SIGQUIT` without a checkpoint. This applies to
interactive mode, wrapped commands, and scripts. Set `PGDB_FAST=false` to use normal mode instead.
Explicit `--fast` or `--test` overrides that environment setting.

Servers using `--data-dir` or `PGDB_DATA_DIR` retain normal defaults. External servers and
`--connect` are never reconfigured. The shutdown grace period defaults to five seconds in fast mode
and twenty seconds otherwise; override it with `--shutdown-timeout` or `PGDB_SHUTDOWN_TIMEOUT`.

### Command mode

Run a command with a temporary database:

```bash
pgdb bash                     # Open a shell
pgdb psql                     # Open a PostgreSQL console
pgdb cargo sqlx migrate run   # Run a development task
```

In command mode, `pgdb` provides the configured database through `DATABASE_URL`, `PGHOST`,
`PGPORT`, `PGUSER`, `PGPASSWORD`, and `PGDATABASE`, and removes the database after the command
exits. Options must precede the command; arguments after the command are passed through unchanged.

Scripts can use `pgdb` as a shebang interpreter by selecting a POSIX shell as the wrapped command:

```sh
#!/usr/bin/env -S pgdb /bin/sh
set -eu

cargo sqlx migrate run
cargo sqlx prepare
```

## Persistent databases

Supply `--data-dir` (or `PGDB_DATA_DIR`) to retain a local cluster across launches:

```sh
pgdb --data-dir .pgdb psql
```

To run another service against a cluster that `pgdb` is already running:

```sh
pgdb --connect .pgdb ./worker
pgdb --connect .pgdb psql
```

`--connect` requires a command, exports the same connection variables as command mode, and fails
if the database is unavailable. It never initializes or stops the database. The owner publishes
private `connection.json` metadata and removes it on shutdown. In process-compose, wait for
`PGDB_READY:` before launching dependent services.

## External Database Support

You can use `pgdb_cli` with an existing PostgreSQL server by setting the `PGDB_TESTS_URL` environment variable:

```bash
PGDB_TESTS_URL=postgres://postgres:password@localhost:5432/postgres pgdb
```

When using an external database:
- The URL must use the `postgres://` scheme and include superuser credentials
- `pgdb_cli` will create the specified user and database on the external server
- The connection details (host, port) will match the external server
- A temporary directory is still created for consistency

## Requirements

PostgreSQL binaries (`postgres`, `initdb`, `psql`) must be available in your `PATH`, `pgdb_cli` does not ship or install
PostgreSQL.
