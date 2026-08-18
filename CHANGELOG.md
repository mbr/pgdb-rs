# CHANGELOG

## [Unreleased]

- Added an option to skip cleanup of external test fixtures.
- Added an option to export a temporary server through `PGDB_TESTS_URL` to wrapped commands.
- Added builder and CLI support for PostgreSQL server configuration overrides.

## [0.8.0] - 2026-08-19

- Added the `--startup-timeout` CLI option.
- Made PostgreSQL graceful and forceful shutdown timeouts configurable through `PostgresBuilder` and CLI
  options.
- Added environment variable aliases for CLI configuration options.
- Applied PostgreSQL environment configuration to instances created by `db_fixture()`.
- External fixture cleanup now disconnects active clients before dropping databases.
- PostgreSQL shutdown now waits for bounded process-group cleanup before removing temporary directories.

## [0.7.0] - 2026-08-18

- Replaced deprecated `structopt` usage with `clap`.
- Local PostgreSQL servers now use dedicated process groups and request fast shutdown before bounded forceful cleanup.
- The CLI now handles ordinary termination signals through normal cleanup in interactive mode.
- `pgdb` can now wrap commands and scripts in a temporary database environment.
- Local PostgreSQL instances now use isolated Unix sockets by default. Use `PostgresBuilder::tcp()`
  or `pgdb --tcp` to use TCP.

## [0.6.0] - 2026-05-28

- `DbUrl` is now `DbInstance`.
- Use `pg_isready` instead of TCP probing for startup readiness detection.
- Added `PostgresBuilder::pg_isready_binary()` method to customize the binary path.

## [0.5.0] - 2025-08-02

- Added a `flake.nix` to allow for easier integration into other projects
- Added external PostgreSQL database support via `PGDB_TESTS_URL` environment variable.
- `DbUrl` is now an enum with `Local` and `External` variants for better external database handling.
- External databases are automatically cleaned up when `DbUrl` is dropped.
- `pgdb_cli` now supports external databases when `PGDB_TESTS_URL` is set.
- Added `ExternalUrlError` enum for proper error handling of external database URLs.
- Database fixtures now use random IDs instead of incremental counters for better parallel test support.
- Unified internal and external database creation code paths for consistency.
- Added public `run_psql_command()` and `create_user_and_database()` functions.
- Refactored internal connection handling to use `url::Url` instead of separate fields.
- `PostgresClient::uri()` renamed to `url()` and returns `Url` instead of `String`.
- Removed `host()` and `port()` methods from `Postgres`. Use `superuser_url()` and extract values from the URL.
- Removed `username()` and `password()` methods from `PostgresClient`. Use `client_url()` and extract values from the
  URL.

## [0.4.0] - 2025-07-04

- The library now uses a random, unused port when launching postgres instances. CLI still defaults to `15432`.
- `PostgresBuilder` no longer derives `Default` to avoid accidentally building nonsensical builders with no root pw.
- Added `db_fixture` function for easier database creation and sharing.

## [0.3.0] - 2024-04-09

- Sequential ports will now be assigned if multiple databases are created from one process.

## [0.2.0] - 2024-04-01

- Added the `--superuser-pw` option to set the `postgres` user's password.
- Changed the `startup_timeout` and `probe_delay` builder methods to take `&mut self`.
- Fixed the `--port` option so it changes the port.

## [0.1.2] - 2021-06-15

- Added `PostgresClient::load_sql`.

## [0.1.1] - 2021-06-12

- Added the `pgdb` CLI for running PostgreSQL instances from the command line.
- Added access to `host`, `port`, and similar information from `Postgres` and `PostgresClient`.
- Converted the repository into a multi-crate project.

## [0.1.0] - 2021-06-12

- Initial release of `pgdb`.
