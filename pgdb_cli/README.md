# pgdb_cli

A command-line interface for running temporary or persistent PostgreSQL databases for development and testing.

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

The server stops on exit, but the directory is preserved. After initial setup, `pgdb.json` stores
credentials for subsequent launches. Explicit credential options must match saved values; runtime
options such as ports are not saved. The file contains plaintext passwords with permissions `0600`:
keep the directory private and out of Git.

Missing `pgdb.json` triggers `initdb`, which refuses nonempty directories. Invalid setup or failed
authentication causes an error, without automatic repair or deletion. Remove the directory to start
fresh, losing its data.

Cannot be combined with `--fast`, `--test`, `PGDB_FAST=true`, or `PGDB_TESTS_URL`.

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
