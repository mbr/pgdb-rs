//! Checks CLI durability defaults and explicit overrides.

use std::{env, process::Command};

/// Builds a CLI invocation without inherited PostgreSQL settings.
fn pgdb() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pgdb"));
    for (key, _) in env::vars_os() {
        if key.to_string_lossy().starts_with("PG") {
            command.env_remove(key);
        }
    }
    command
}

/// Queries the durability settings selected by the server owner.
const SETTINGS_SQL: &str = "SELECT current_setting('fsync'), current_setting('synchronous_commit'), current_setting('full_page_writes')";

/// Keeps persistent defaults durable and honors environment and flag precedence.
#[test]
fn disposable_defaults_and_overrides() -> anyhow::Result<()> {
    for (data_dir_source, environment, flag, fast) in [
        (None, None, None, true),
        (None, Some("false"), None, false),
        (None, Some("false"), Some("--fast"), true),
        (None, Some("false"), Some("--test"), true),
        (Some("flag"), None, None, false),
        (Some("environment"), None, None, false),
        (Some("flag"), Some("true"), None, true),
    ] {
        let directory = tempfile::tempdir()?;
        let data_dir = directory.path().join("db");
        let mut command = pgdb();
        match data_dir_source {
            Some("flag") => {
                command.arg("--data-dir").arg(&data_dir);
            }
            Some("environment") => {
                command.env("PGDB_DATA_DIR", &data_dir);
            }
            None => {}
            _ => unreachable!("test cases specify the data directory source"),
        }
        if let Some(value) = environment {
            command.env("PGDB_FAST", value);
        }
        if let Some(flag) = flag {
            command.arg(flag);
        }
        let output = command.args(["psql", "-XAtc", SETTINGS_SQL]).output()?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{command:?}: {stderr}");
        let expected = if fast { "off|off|off" } else { "on|on|on" };
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).lines().last(),
            Some(expected),
            "{command:?}: {stderr}"
        );
        let shutdown = if fast {
            "received immediate shutdown request"
        } else {
            "received fast shutdown request"
        };
        assert!(stderr.contains(shutdown), "{command:?}: {stderr}");
        assert!(stderr.contains("database system is shut down"), "{stderr}");
        if data_dir_source.is_some() {
            assert!(data_dir.join("PG_VERSION").is_file());
            assert!(!data_dir.join("postmaster.pid").exists());
        }
    }
    Ok(())
}

/// Leaves an externally owned server's durability settings unchanged.
#[test]
fn external_server_is_not_switched_to_fast_mode() -> anyhow::Result<()> {
    let server = pgdb::Postgres::build().start()?;
    let output = pgdb()
        .env("PGDB_TESTS_URL", server.superuser_url().as_str())
        .env("PGDB_FAST", "true")
        .args(["--fast", "psql", "-XAtc", SETTINGS_SQL])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).lines().last(),
        Some("on|on|on")
    );
    server.as_superuser().run_sql("postgres", "SELECT 1")?;
    Ok(())
}
