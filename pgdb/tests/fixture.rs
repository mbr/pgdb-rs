//! Checks fixture defaults without inheriting a shared server or changing test environment variables.

use std::{env, process::Command};

use pgdb::{DbInstance, db_fixture};

/// Verifies fast fixture settings, the environment opt-out, and directory cleanup.
#[test]
fn local_fixture_defaults_to_fast_mode() {
    /// Selects the fixture configuration in an isolated subprocess.
    const MODE_ENV: &str = "PGDB_FIXTURE_TEST_MODE";
    let Ok(mode) = env::var(MODE_ENV) else {
        for mode in ["default", "durable"] {
            let mut command = Command::new(env::current_exe().expect("test executable"));
            command
                .args([
                    "--exact",
                    "local_fixture_defaults_to_fast_mode",
                    "--nocapture",
                ])
                .env(MODE_ENV, mode)
                .env_remove("PGDB_TESTS_URL")
                .env_remove("PGDB_FAST");
            if mode == "durable" {
                command.env("PGDB_FAST", "false");
            }
            let output = command.output().expect("run isolated fixture test");
            assert!(
                output.status.success(),
                "fixture mode {mode} failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let expected_shutdown = if mode == "default" {
                "received immediate shutdown request"
            } else {
                "received fast shutdown request"
            };
            assert!(
                String::from_utf8_lossy(&output.stderr).contains(expected_shutdown),
                "fixture mode {mode} did not request the expected shutdown: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        return;
    };

    let fixture = db_fixture();
    let DbInstance::Local { _arc: pg, .. } = &fixture else {
        panic!("fixture must create a local server");
    };
    let temporary_directory = pg
        .data_dir()
        .parent()
        .expect("temporary directory")
        .to_path_buf();
    let output = pg
        .as_superuser()
        .psql("postgres")
        .args([
            "-XAtc",
            "SELECT current_setting('fsync'), current_setting('synchronous_commit'), current_setting('full_page_writes')",
        ])
        .output()
        .expect("query fixture durability settings");
    assert!(output.status.success());
    let expected = if mode == "default" {
        "off|off|off"
    } else {
        "on|on|on"
    };
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), expected);

    drop(fixture);
    assert!(
        !temporary_directory.exists(),
        "fixture must remove its temporary directory"
    );
}
