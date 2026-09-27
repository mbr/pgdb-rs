//! Persistent cluster lifecycle and failure behavior.

use std::{fs, os::unix::fs::PermissionsExt, path::Path};

use pgdb::{setup::DatabaseOptions, Error, Postgres};

/// Reads a scalar query result from PostgreSQL.
fn query(pg: &Postgres, sql: &str) -> String {
    let output = pg
        .as_superuser()
        .psql("postgres")
        .args(["-Atc", sql])
        .output()
        .expect("query must run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("query output must be UTF-8")
        .trim()
        .to_string()
}

/// Ensures restarts retain data and generated administrative credentials.
#[test]
fn persistent_cluster_reuses_data_and_credentials() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let data_dir = directory.path().join("db");
    let pg = Postgres::build()
        .data_dir(&data_dir)
        .start()
        .expect("cluster must start");
    pg.as_superuser()
        .run_sql("postgres", "CREATE TABLE saved AS SELECT 42 AS answer")
        .expect("table must be created");
    let password = pg
        .superuser_url()
        .password()
        .expect("password must exist")
        .to_owned();
    let setup_path = data_dir.join("pgdb.json");
    let setup = fs::read(&setup_path).expect("setup must be saved");
    assert_eq!(
        fs::metadata(&setup_path)
            .expect("setup metadata must exist")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    drop(pg);
    assert!(data_dir.exists());

    let pg = Postgres::build()
        .data_dir(&data_dir)
        .initdb_binary("/nonexistent/initdb")
        .start()
        .expect("cluster must restart without initdb");
    assert_eq!(pg.superuser_url().password(), Some(password.as_str()));
    assert_eq!(query(&pg, "SELECT answer FROM saved"), "42");
    assert_eq!(query(&pg, "SHOW fsync"), "on");
    assert_eq!(
        fs::read(&setup_path).expect("setup must remain readable"),
        setup
    );
    drop(pg);

    assert!(matches!(
        Postgres::build()
            .data_dir(&data_dir)
            .superuser_pw("wrong")
            .start(),
        Err(Error::ConflictingSetup(_))
    ));
    let mut invalid_credentials: serde_json::Value =
        serde_json::from_slice(&setup).expect("setup must be JSON");
    invalid_credentials["superuser_pw"] = "wrong".into();
    fs::write(
        &setup_path,
        serde_json::to_vec(&invalid_credentials).expect("setup must serialize"),
    )
    .expect("setup must be replaced");
    assert!(matches!(
        Postgres::build().data_dir(&data_dir).start(),
        Err(Error::PsqlFailed(_))
    ));
    assert!(!data_dir.join("postmaster.pid").exists());
    fs::write(&setup_path, b"{").expect("setup must be replaced");
    assert!(matches!(
        Postgres::build().data_dir(&data_dir).start(),
        Err(Error::ParseSetup(_))
    ));
    assert_eq!(
        fs::read(&setup_path).expect("invalid setup must remain"),
        b"{"
    );
    fs::remove_file(&setup_path).expect("setup must be removed");
    assert!(matches!(
        Postgres::build().data_dir(&data_dir).start(),
        Err(Error::InitDbFailed(_))
    ));
    assert!(data_dir.join("PG_VERSION").exists());
    fs::write(&setup_path, setup).expect("setup must be restored");
    let pg = Postgres::build()
        .data_dir(&data_dir)
        .start()
        .expect("data must survive failures");
    assert_eq!(query(&pg, "SELECT answer FROM saved"), "42");
}

/// Ensures saved application credentials are reused and stale passwords fail.
#[test]
fn application_setup_is_verified_on_restart() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let data_dir = directory.path().join("db");
    let options = DatabaseOptions {
        name: Some("app".to_string()),
        user: Some("app:user".to_string()),
        password: Some("p@ss:% word'".to_string()),
    };
    let pg = Postgres::build()
        .data_dir(&data_dir)
        .superuser_pw("admin:@ %")
        .start_with_database(&options)
        .expect("application setup must succeed");
    drop(pg);
    let pg = Postgres::build()
        .data_dir(&data_dir)
        .start_with_database(&DatabaseOptions::default())
        .expect("saved application setup must be reused");
    let database = pg.database().expect("application database must exist");
    assert_eq!(database.user, "app:user");
    assert_eq!(database.password, "p@ss:% word'");
    pg.as_superuser()
        .run_sql("postgres", "ALTER ROLE \"app:user\" PASSWORD 'changed'")
        .expect("password must change");
    drop(pg);
    let setup = fs::read(data_dir.join("pgdb.json")).expect("setup must exist");
    assert!(matches!(
        Postgres::build().data_dir(&data_dir).start(),
        Err(Error::PsqlFailed(_))
    ));
    assert_eq!(
        fs::read(data_dir.join("pgdb.json")).expect("setup must remain"),
        setup
    );
    assert!(!data_dir.join("postmaster.pid").exists());
}

/// Ensures temporary clusters also save setup and remove it on drop.
#[test]
fn temporary_setup_is_removed_with_cluster() {
    let pg = Postgres::build()
        .start()
        .expect("temporary cluster must start");
    let socket_dir =
        pgdb::connection_host(pg.superuser_url()).expect("socket directory must exist");
    let data_dir = Path::new(socket_dir.as_ref()).join("db");
    assert!(data_dir.join("pgdb.json").is_file());
    drop(pg);
    assert!(!data_dir.exists());
}

/// Ensures unsafe modes and failed initialization never produce completed setup.
#[test]
fn failed_setup_leaves_directory_untouched() {
    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let data_dir = directory.path().join("db");
    assert!(matches!(
        Postgres::build().data_dir(&data_dir).fast().start(),
        Err(Error::PersistentFastMode)
    ));
    assert!(!data_dir.exists());
    let options = DatabaseOptions {
        user: Some("postgres".to_string()),
        ..Default::default()
    };
    assert!(matches!(
        Postgres::build()
            .data_dir(&data_dir)
            .start_with_database(&options),
        Err(Error::PsqlFailed(_))
    ));
    assert!(!data_dir.join("pgdb.json").exists());
    assert!(data_dir.join("PG_VERSION").exists());
    assert!(!data_dir.join("postmaster.pid").exists());

    let unrelated = directory.path().join("unrelated");
    fs::create_dir(&unrelated).expect("directory must be created");
    fs::write(unrelated.join("keep"), "untouched").expect("file must be written");
    assert!(matches!(
        Postgres::build().data_dir(&unrelated).start(),
        Err(Error::InitDbFailed(_))
    ));
    assert_eq!(
        fs::read_to_string(unrelated.join("keep")).expect("file must remain"),
        "untouched"
    );
}
