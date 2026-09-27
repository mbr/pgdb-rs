//! Cluster reuse and failed restarts.

use std::{error::Error, fs, os::unix::fs::PermissionsExt};

use pgdb::{Error as PgError, Postgres};

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

    let pg = builder.start()?;
    let output = pg
        .as_superuser()
        .psql("postgres")
        .args(["-XAtc", "SELECT answer FROM saved"])
        .output()?;
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");
    assert_eq!(fs::read(&path)?, saved);
    drop(pg);

    let mut invalid: serde_json::Value = serde_json::from_slice(&saved)?;
    invalid["superuser_pw"] = "wrong".into();
    fs::write(&path, serde_json::to_vec(&invalid)?)?;
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
