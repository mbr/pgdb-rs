//! Cluster reuse and failed restarts.

use std::{error::Error, fs, os::unix::fs::PermissionsExt};

use pgdb::{state::State, Error as PgError, Postgres};

/// Rejects a different cluster answering on the requested TCP endpoint.
#[test]
fn occupied_tcp_endpoint_is_not_adopted() -> Result<(), Box<dyn Error>> {
    let pg = Postgres::build().tcp().superuser_pw("shared").start()?;
    let port = pgdb::connection_port(&pg.superuser_url()).expect("TCP port");
    let result = Postgres::build()
        .tcp()
        .port(port)
        .superuser_pw("shared")
        .start();
    assert!(
        matches!(result, Err(PgError::UnexpectedPostgres)),
        "{:?}",
        result
    );
    pg.as_superuser().run_sql("postgres", "SELECT 1")?;
    Ok(())
}

/// Checks credential reuse, data retention, and failure without repair.
#[test]
fn persistent_cluster() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let data_dir = directory.path().join("db");
    let mut builder = Postgres::build();
    builder.data_dir(&data_dir);
    let pg = builder.start()?;
    pg.as_superuser()
        .run_sql("postgres", "CREATE TABLE saved AS SELECT 42 AS answer")?;
    let path = pg.data_dir().join("pgdb.json");
    let saved = fs::read(&path)?;
    assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o600);
    drop(pg);

    let pg = builder.superuser_pw("ignored on restart").start()?;
    let output = pg
        .as_superuser()
        .psql("postgres")
        .args(["-XAtc", "SELECT answer FROM saved"])
        .output()?;
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");
    assert_eq!(fs::read(&path)?, saved);
    drop(pg);

    let mut state = State::load(&data_dir)?.expect("saved state");
    state.admin.password = "wrong".into();
    state.store(&data_dir)?;
    assert!(matches!(builder.start(), Err(PgError::PsqlFailed(_))));
    assert!(!data_dir.join("postmaster.pid").exists());
    fs::write(&path, "{")?;
    assert!(matches!(builder.start(), Err(PgError::ParseSetup(_))));
    assert_eq!(fs::read_to_string(&path)?, "{");
    fs::remove_file(&path)?;
    assert!(matches!(builder.start(), Err(PgError::InitDbFailed(_))));
    assert!(data_dir.join("PG_VERSION").exists());
    assert!(!path.exists());
    Ok(())
}
