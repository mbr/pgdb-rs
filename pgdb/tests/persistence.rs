//! Persistent cluster lifecycle and failure behavior.

use std::{fs, os::unix::fs::PermissionsExt};

use pgdb::{Error, Postgres};

/// Ensures restarts retain data and generated administrative credentials.
#[test]
fn persistent_cluster_reuses_data_and_credentials() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let data_dir = directory.path().join("db");
    let mut builder = Postgres::build();
    builder.data_dir(&data_dir);
    let pg = builder.start().expect("cluster must start");
    pg.as_superuser()
        .run_sql("postgres", "CREATE TABLE saved AS SELECT 42 AS answer")
        .expect("table must be created");
    let path = pg.data_dir().join("pgdb.json");
    let setup = fs::read(&path).expect("credentials must be saved");
    assert_eq!(
        fs::metadata(&path)
            .expect("metadata must exist")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    drop(pg);

    let pg = builder
        .initdb_binary("/nonexistent/initdb")
        .start()
        .expect("cluster must restart without initdb");
    let output = pg
        .as_superuser()
        .psql("postgres")
        .args(["-XAtc", "SELECT answer FROM saved"])
        .output()
        .expect("query must run");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");
    assert_eq!(fs::read(&path).expect("credentials must remain"), setup);
    drop(pg);
    assert!(data_dir.exists());
    assert!(matches!(
        builder.superuser_pw("wrong").start(),
        Err(Error::ConflictingSuperuserPassword)
    ));
}

/// Ensures invalid state fails without erasing or repairing the directory.
#[test]
fn invalid_state_is_preserved() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let data_dir = directory.path().join("db");
    let mut builder = Postgres::build();
    builder.data_dir(&data_dir);
    drop(builder.start().expect("cluster must start"));
    let path = data_dir.join("pgdb.json");
    let mut setup: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).expect("setup must exist"))
            .expect("setup must be JSON");
    setup["superuser_pw"] = "wrong".into();
    fs::write(
        &path,
        serde_json::to_vec(&setup).expect("setup must serialize"),
    )
    .expect("setup must be replaced");
    assert!(matches!(builder.start(), Err(Error::PsqlFailed(_))));
    assert!(!data_dir.join("postmaster.pid").exists());

    fs::write(&path, "{").expect("setup must be replaced");
    assert!(matches!(builder.start(), Err(Error::ParseSetup(_))));
    assert_eq!(fs::read_to_string(&path).expect("setup must remain"), "{");
    fs::remove_file(&path).expect("setup must be removed");
    assert!(matches!(builder.start(), Err(Error::InitDbFailed(_))));
    assert!(data_dir.join("PG_VERSION").exists());
    assert!(!path.exists());
    assert!(matches!(
        builder.fast().start(),
        Err(Error::PersistentFastMode)
    ));
}

/// Ensures temporary clusters also save credentials and remove them on drop.
#[test]
fn temporary_setup_is_removed_with_cluster() {
    let pg = Postgres::build()
        .start()
        .expect("temporary cluster must start");
    let data_dir = pg.data_dir().to_owned();
    assert!(data_dir.join("pgdb.json").is_file());
    drop(pg);
    assert!(!data_dir.exists());
}
