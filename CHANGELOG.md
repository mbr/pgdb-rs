# CHANGELOG

## [Unrelead]

### Changed

- `startup_timeout` and `probe_delay` builder method signatures brought in line with the rest, taking a `&mut` receiver.

## [0.1.2] 2021-06-15

### Added

- New method `PostgresClient::load_sql`.

## [0.1.1] - 2021-06-12

### Added

- CLI tool `pgdb` that allows running Postgres instances from the command line.
- Can now retrieve `host`, `port` and similar information from `Postgres`/`PostgresClient`.

### Changed

- Repository is now multi-crate.

## [0.1.0] - 2021-06-12

### Added

- Initial release of `pgdb`.
