//! `pgdb` errors.
//!
//! Contains all errors produced by any part of this crate.

use std::{io, process};

use thiserror::Error;

/// A Postgres server error.
#[derive(Debug, Error)]
pub enum Error {
    #[error("could not find `postgres` binary")]
    FindPostgres(which::Error),
    /// Failed to find the `initdb` binary.
    #[error("could not find `initdb` binary")]
    FindInitdb(which::Error),
    /// Failed to find the `postgres` binary.
    #[error("could not find `psql` binary")]
    FindPsql(which::Error),
    /// Could not create the temporary directory.
    #[error("could not create temporary directory for database")]
    CreateDatabaseDir(io::Error),
    /// Could not write the temporary password to a file.
    #[error("error writing temporary password")]
    WriteTemporaryPw(io::Error),
    /// Starting `initdb` failed.
    #[error("failed to run `initdb`")]
    RunInitDb(io::Error),
    /// Running `initdb` was not successful.
    #[error("`initdb` exited with status {}", 0)]
    InitDbFailed(process::ExitStatus),
    /// Postgres could not be launched.
    #[error("failed to launch `postgres`")]
    LaunchPostgres(io::Error),
    /// Postgres was launched but failed to bring up a TCP-connection accepting socket in time.
    #[error("timeout probing tcp socket")]
    StartupTimeout,
    /// `psql` could not be launched.
    #[error("failed to run `psql`")]
    RunPsql(io::Error),
    /// Running `psql` returned an error.
    #[error("`psql` exited with status {}", 0)]
    PsqlFailed(process::ExitStatus),
    /// Invalid external test URL.
    #[error("invalid PGDB_TESTS_URL")]
    InvalidExternalUrl(#[source] ExternalUrlError),
}

/// Errors that can occur when parsing an external database URL.
#[derive(Debug, Error)]
pub enum ExternalUrlError {
    /// URL parsing failed.
    #[error("invalid URL: {0}")]
    ParseError(#[source] url::ParseError),
    /// Wrong URL scheme.
    #[error("must use postgres:// scheme")]
    InvalidScheme,
    /// Missing host.
    #[error("must include a host")]
    MissingHost,
    /// Missing username.
    #[error("must include a username")]
    MissingUsername,
}
