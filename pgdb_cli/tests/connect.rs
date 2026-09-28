//! Connection-only execution and runtime metadata ownership.

use std::{
    env, fs,
    net::TcpListener,
    os::unix::fs::PermissionsExt,
    process::Command,
    time::{Duration, Instant},
};

/// Builds a CLI invocation without inherited database settings.
fn pgdb() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pgdb"));
    for (key, _) in env::vars_os() {
        if key.to_string_lossy().starts_with("PG") {
            command.env_remove(key);
        }
    }
    command
}

/// Checks socket and TCP attachment, command behavior, and owner-only cleanup.
#[test]
fn connect_to_running_database() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let data_dir = directory.path().join("db");
    let snapshot = directory.path().join("connection.json");
    let saved_state = directory.path().join("pgdb.json");
    let child_marker = directory.path().join("child-ran");

    for extra in [&[][..], &["--tcp"][..]] {
        let output = pgdb()
            .arg("--data-dir")
            .arg(&data_dir)
            .args([
                "--user",
                "owner",
                "--password",
                "secret",
                "--db",
                "app db",
                "--export-tests-url",
            ])
            .args(extra)
            .args([
                "sh",
                "-ec",
                r#"
                cp -p "$DIR/connection.json" "$SNAPSHOT"
                cp "$DIR/pgdb.json" "$SAVED_STATE"
                "$CLI" --connect "$DIR" --user ignored --password ignored --db ignored sh -ec '
                    test "$PGUSER" = owner
                    test "$PGPASSWORD" = secret
                    test "$PGDATABASE" = "app db"
                    test -n "$PGHOST"
                    test -n "$PGPORT"
                    psql -XAtc "SELECT current_user, current_database()"
                    psql "$DATABASE_URL" -XAtc "SELECT 42"
                '
                status=0
                "$CLI" --connect "$DIR" sh -c 'exit 37' || status=$?
                test "$status" -eq 37
                status=0
                "$CLI" --connect "$DIR" sh -c '
                    trap "exit 42" TERM
                    kill -TERM "$PPID"
                    sleep 1
                    exit 99
                ' || status=$?
                test "$status" -eq 42
                test -f "$DIR/connection.json"
                psql -Xqc "ALTER ROLE owner PASSWORD 'changed'"
                if "$CLI" --connect "$DIR" touch "$CHILD_MARKER"; then exit 1; fi
                test ! -e "$CHILD_MARKER"
                psql "$PGDB_TESTS_URL" -Xqc "ALTER ROLE owner PASSWORD 'secret'"
                psql -XAtc 'SELECT 7'
            "#,
            ])
            .env("CLI", env!("CARGO_BIN_EXE_pgdb"))
            .env("DIR", &data_dir)
            .env("SNAPSHOT", &snapshot)
            .env("SAVED_STATE", &saved_state)
            .env("CHILD_MARKER", &child_marker)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).ends_with("owner|app db\n42\n7\n"));
        assert_eq!(fs::metadata(&snapshot)?.permissions().mode() & 0o777, 0o600);
        assert!(!data_dir.join("connection.json").exists());
        assert!(!data_dir.join("postmaster.pid").exists());
        assert_eq!(
            fs::read(data_dir.join("pgdb.json"))?,
            fs::read(&saved_state)?
        );

        let saved = fs::read(&snapshot)?;
        fs::write(data_dir.join("connection.json"), &saved)?;
        let output = pgdb()
            .arg("--connect")
            .arg(&data_dir)
            .arg("touch")
            .arg(&child_marker)
            .output()?;
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("cannot connect to the running database")
        );
        assert!(!String::from_utf8_lossy(&output.stderr).contains("PGDB_READY:"));
        assert!(!child_marker.exists());
        assert_eq!(fs::read(data_dir.join("connection.json"))?, saved);
    }
    Ok(())
}

/// Rejects absent or malformed metadata without initializing or repairing anything.
#[test]
fn invalid_connection_metadata() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let data_dir = directory.path().join("db");
    let output = pgdb()
        .arg("--connect")
        .arg(&data_dir)
        .arg("true")
        .output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("is the owning pgdb running?"));
    assert!(!data_dir.exists());

    fs::create_dir(&data_dir)?;
    for contents in ["{", r#"{"url":"postgres:///"}"#] {
        fs::write(data_dir.join("connection.json"), contents)?;
        let output = pgdb()
            .arg("--connect")
            .arg(&data_dir)
            .arg("true")
            .output()?;
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("invalid"));
        assert_eq!(
            fs::read_to_string(data_dir.join("connection.json"))?,
            contents
        );
        assert_eq!(fs::read_dir(&data_dir)?.count(), 1);
    }

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let metadata = serde_json::json!({
        "url": format!("postgres://owner:secret@{}/app", listener.local_addr()?)
    });
    fs::write(
        data_dir.join("connection.json"),
        serde_json::to_vec(&metadata)?,
    )?;
    let started = Instant::now();
    let output = pgdb()
        .arg("--connect")
        .arg(&data_dir)
        .arg("true")
        .env("PGCONNECT_TIMEOUT", "60")
        .output()?;
    assert!(!output.status.success());
    assert!(started.elapsed() < Duration::from_secs(8));
    Ok(())
}
