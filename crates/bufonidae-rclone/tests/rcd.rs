//! Integration tests against a real `rclone rcd`, using the `local` backend as
//! a stand-in for Mega accounts. They are skipped when rclone is missing,
//! unless `BUFONIDAE_REQUIRE_RCLONE=1` (set in CI).

use bufonidae_rclone::Error;
use bufonidae_rclone::rcd::Rcd;

async fn start() -> Option<Rcd> {
    match Rcd::start("rclone").await {
        Ok(rcd) => Some(rcd),
        Err(Error::Spawn(_)) if std::env::var_os("BUFONIDAE_REQUIRE_RCLONE").is_none() => {
            eprintln!("rclone not found, skipping");
            None
        }
        Err(e) => panic!("could not start rclone rcd: {e}"),
    }
}

fn local(dir: &tempfile::TempDir) -> String {
    format!(":local:{}", dir.path().display())
}

#[tokio::test]
async fn lists_files_and_reports_space() {
    let Some(rcd) = start().await else { return };
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), b"hello").unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();

    let mut entries = rcd.list(&local(&dir), "").await.unwrap();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(entries.len(), 2);
    assert_eq!((entries[0].name.as_str(), entries[0].size), ("a.txt", 5));
    assert!(!entries[0].is_dir);
    assert!(entries[1].is_dir);

    let about = rcd.about(&local(&dir)).await.unwrap();
    assert!(about.free.is_some());

    rcd.shutdown().await.unwrap();
}

#[tokio::test]
async fn copies_a_file_between_remotes() {
    let Some(rcd) = start().await else { return };
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    std::fs::write(src.path().join("a.txt"), b"payload").unwrap();

    rcd.copy_file(&local(&src), "a.txt", &local(&dst), "copy/a.txt")
        .await
        .unwrap();

    assert_eq!(
        std::fs::read(dst.path().join("copy/a.txt")).unwrap(),
        b"payload"
    );
    rcd.shutdown().await.unwrap();
}

#[tokio::test]
async fn surfaces_rpc_errors() {
    let Some(rcd) = start().await else { return };
    let dir = tempfile::tempdir().unwrap();

    let err = rcd.list(&local(&dir), "missing").await.unwrap_err();
    assert!(
        matches!(
            err,
            Error::Rpc {
                status: 404 | 500,
                ..
            }
        ),
        "{err}"
    );
    rcd.shutdown().await.unwrap();
}
