//! Persistent command-line database behavior.

use std::{
    env, fs,
    process::{Command, Output},
};

/// Builds an invocation without inherited database configuration.
fn pgdb() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pgdb"));
    for (name, _) in env::vars_os() {
        if name.to_string_lossy().starts_with("PG") {
            command.env_remove(name);
        }
    }
    command
}

/// Runs an invocation and checks success with captured diagnostics.
fn success(command: &mut Command) -> Output {
    let output = command.output().expect("pgdb must run");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// Ensures saved settings reach wrapped commands and conflicting overrides fail.
#[test]
fn commands_reuse_saved_database_and_credentials() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let data_dir = directory.path().join("db");
    success(pgdb().arg("--data-dir").arg(&data_dir).args([
        "--user",
        "owner",
        "--password",
        "secret:@ %",
        "--db",
        "app",
        "psql",
        "-X",
        "-v",
        "ON_ERROR_STOP=1",
        "-c",
        "CREATE TABLE saved AS SELECT 42 AS answer",
    ]));
    let setup = fs::read(data_dir.join("pgdb.json")).expect("setup must exist");
    let output = success(pgdb().env("PGDB_DATA_DIR", &data_dir).args([
        "psql",
        "-XAtc",
        "SELECT current_user, current_database(), answer FROM saved",
    ]));
    assert!(String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line == "owner|app|42"));
    let output = success(pgdb().arg("--data-dir").arg(&data_dir)
        .args(["sh", "-c", "test \"$PGUSER\" = owner && test \"$PGDATABASE\" = app && test \"$PGPASSWORD\" = 'secret:@ %' && psql -XAt \"$DATABASE_URL\" -c 'SELECT answer FROM saved'"]));
    assert!(String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line == "42"));

    for option in ["--user", "--password", "--db", "--superuser-pw"] {
        let output = pgdb()
            .arg("--data-dir")
            .arg(&data_dir)
            .args([option, "different", "true"])
            .output()
            .expect("pgdb must run");
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("conflicts with pgdb.json"));
    }
    assert_eq!(
        fs::read(data_dir.join("pgdb.json")).expect("setup must remain"),
        setup
    );
}

/// Ensures incompatible modes fail before creating a directory.
#[test]
fn incompatible_modes_fail_before_initialization() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let data_dir = directory.path().join("db");
    for flag in ["--fast", "--test"] {
        let output = pgdb()
            .arg("--data-dir")
            .arg(&data_dir)
            .args([flag, "true"])
            .output()
            .expect("pgdb must run");
        assert!(!output.status.success());
    }
    let output = pgdb()
        .arg("--data-dir")
        .arg(&data_dir)
        .arg("true")
        .env("PGDB_FAST", "true")
        .output()
        .expect("pgdb must run");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("fast mode cannot"));
    let output = pgdb()
        .arg("--data-dir")
        .arg(&data_dir)
        .arg("true")
        .env(
            "PGDB_TESTS_URL",
            "postgres://postgres:password@localhost/postgres",
        )
        .output()
        .expect("pgdb must run");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cannot be combined with PGDB_TESTS_URL")
    );
    assert!(!data_dir.exists());
}
