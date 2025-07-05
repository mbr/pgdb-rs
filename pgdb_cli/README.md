# pgdb_cli

A command-line interface for creating temporary PostgreSQL databases for development and testing.

## Installation

The easiest way to install is to do so straight from [crates.io](https://crates.io/crates/pgdb_cli):

```bash
cargo install pgdb_cli
```

## Usage

Start a temporary PostgreSQL instance:

```bash
pgdb
```

This will:

- Start a PostgreSQL server on port 15432 (by default), with a data directory in your temporary directory
- Create a user `dev` with password `dev`
- Create a database `dev` owned by the user
- Display connection information
- Keep running until interrupted (Ctrl+C)

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
