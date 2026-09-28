//! CLI persistence and setup failures.

#[cfg(target_os = "linux")]
use std::os::unix::process::ExitStatusExt;
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

/// Rejects a second owner without publishing readiness or disturbing the first.
#[test]
fn running_cluster_is_not_adopted() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("db");
    let output = pgdb(
        &path,
        &[
            "--tcp",
            "sh",
            "-ec",
            r#"
            cp "$DIR/connection.json" "$CHECKS/connection"
            cp "$DIR/pgdb.json" "$CHECKS/state"
            if "$CLI" --data-dir "$DIR" --tcp --port "$PGPORT" touch "$CHECKS/child" \
                >"$CHECKS/stdout" 2>"$CHECKS/stderr"; then
                exit 1
            fi
            cmp "$DIR/connection.json" "$CHECKS/connection"
            cmp "$DIR/pgdb.json" "$CHECKS/state"
            psql -XAtc 'SELECT 42'
        "#,
        ],
    )
    .env("CLI", env!("CARGO_BIN_EXE_pgdb"))
    .env("DIR", &path)
    .env("CHECKS", directory.path())
    .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).ends_with("42\n"));
    let stderr = fs::read_to_string(directory.path().join("stderr"))?;
    assert!(
        stderr.contains("connected PostgreSQL server is not the process launched by this instance")
    );
    assert!(!stderr.contains("PGDB_READY:"));
    assert!(!directory.path().join("child").exists());
    assert!(!path.join("connection.json").exists());
    assert!(!path.join("postmaster.pid").exists());
    Ok(())
}

/// Checks kernel-triggered shutdown and recovery after the owning CLI is killed.
#[cfg(target_os = "linux")]
#[test]
fn killed_owner_stops_postgres() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("db");
    let output = pgdb(
        &path,
        &[
            "sh",
            "-ec",
            r#"
            trap '
                if test ! -f "$DIR/postmaster.pid" ||
                    pg_ctl -D "$DIR" -m immediate -w -t 10 stop >/dev/null 2>&1; then
                    rm -rf "$PGHOST"
                fi
            ' 0
            psql -Xqc 'CREATE TABLE saved AS SELECT 42 AS answer'
            kill -KILL "$PPID"
            attempts=0
            while test -f "$DIR/postmaster.pid"; do
                attempts=$((attempts + 1))
                test "$attempts" -lt 150
                sleep 0.1
            done
            printf 'postgres stopped\n'
        "#,
        ],
    )
    .env("DIR", &path)
    .output()?;
    assert_eq!(output.status.signal(), Some(libc::SIGKILL));
    assert!(
        String::from_utf8_lossy(&output.stdout).ends_with("postgres stopped\n"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = pgdb(&path, &["psql", "-XAtc", "SELECT answer FROM saved"]).output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");
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
