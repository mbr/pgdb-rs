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
pgdb --data-dir .pgdb --user app --password secret --db app psql
pgdb --data-dir .pgdb psql
```

The first launch runs `initdb`, creates the user and database, verifies access, and then writes
`pgdb.json` inside the data directory. Later launches load this file and authenticate against the
existing cluster without rerunning setup. The server stops on exit, but the supplied directory is
never deleted by `pgdb`. Without this option, the cluster and its `pgdb.json` are temporary.

`pgdb.json` contains `superuser`, `superuser_pw`, and an application `database` object with `name`,
`user`, and `password`. It is written with permissions `0600`, but contains plaintext credentials.
Keep the directory private and add the entire directory to `.gitignore`. For example, to inspect
its database name:

```sh
jq -r '.database.name' .pgdb/pgdb.json
```

On restart, omitted credentials and database names come from this file. Explicit `--user`,
`--password`, `--db`, and `--superuser-pw` options (including their `PGDB_USER`, `PGDB_PASSWORD`,
`PGDB_DB`, and `PGDB_SUPERUSER_PW` environment equivalents) must match saved values. New clusters
still default to `dev` for the application database, username, and password, with a generated admin
password. Runtime options such as port, timeouts, and `--postgres-option` are not saved; supply them
on each launch as needed.

Invalid JSON, stale credentials, missing databases, or incompatible PostgreSQL versions cause an
error, not automatic repair. If `pgdb.json` is absent, `initdb` runs and refuses a nonempty directory.
This includes a crash before the setup file was written. Remove the directory explicitly to start
fresh, losing its data. Editing passwords in `pgdb.json` does not change passwords in PostgreSQL.

`--data-dir` cannot be combined with `--fast`, `--test`, `PGDB_FAST=true`, or `PGDB_TESTS_URL`.
Persistent clusters use normal durability settings unless explicitly overridden with server
options. This feature is intended for development, not production management, backups, or automatic
PostgreSQL major-version upgrades.

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
