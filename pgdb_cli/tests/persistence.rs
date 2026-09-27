//! CLI persistence and setup failures.

use std::{env, fs, path::Path, process::Command};

/// Builds a CLI invocation without inherited database settings.
fn pgdb(directory: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pgdb"));
    for (key, _) in env::vars_os() {
        if key.to_string_lossy().starts_with("PG") {
            command.env_remove(key);
        }
    }
    command.arg("--data-dir").arg(directory).args(args);
    command
}

/// Checks that saved credentials take precedence over CLI and environment values.
#[test]
fn reuse_database() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("db");
    assert!(pgdb(
        &path,
        &["psql", "-Xc", "CREATE TABLE saved AS SELECT 42 AS answer"]
    )
    .env("PGDB_USER", "owner")
    .env("PGDB_PASSWORD", "secret")
    .env("PGDB_DB", "app")
    .status()?
    .success());
    let saved = fs::read(path.join("pgdb.json"))?;
    let output = pgdb(
        &path,
        &[
            "--fast",
            "--user=dev",
            "--password=dev",
            "--db=dev",
            "--superuser-pw=dev",
            "psql",
            "-XAtc",
            "SELECT current_user, current_database(), answer FROM saved",
        ],
    )
    .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "owner|app|42"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .filter(|line| line.starts_with("PGDB_READY:"))
            .count(),
        1
    );
    assert!(pgdb(
        &path,
        &["psql", "-Xc", "ALTER ROLE owner PASSWORD 'changed'"]
    )
    .envs([
        ("PGDB_USER", "dev"),
        ("PGDB_PASSWORD", "dev"),
        ("PGDB_DB", "dev"),
        ("PGDB_SUPERUSER_PW", "dev")
    ])
    .status()?
    .success());
    let output = pgdb(&path, &["true"]).output()?;
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("PGDB_READY:"));
    assert_eq!(fs::read(path.join("pgdb.json"))?, saved);
    Ok(())
}

/// Checks incompatible modes and incomplete setup without deleting data.
#[test]
fn failed_setup() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("db");
    assert!(!pgdb(&path, &["true"])
        .env("PGDB_TESTS_URL", "postgres://postgres@localhost/postgres")
        .status()?
        .success());
    assert!(!path.exists());
    assert!(!pgdb(&path, &["--user", "postgres", "true"])
        .status()?
        .success());
    let saved = fs::read(path.join("pgdb.json"))?;
    assert!(!pgdb(&path, &["true"]).status()?.success());
    assert!(!path.join("postmaster.pid").exists());
    assert!(!path.join("connection.json").exists());
    assert_eq!(fs::read(path.join("pgdb.json"))?, saved);
    Ok(())
}
