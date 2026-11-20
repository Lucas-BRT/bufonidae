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
async fn reports_transferred_bytes_before_a_throttled_copy_finishes() {
    let Some(rcd) = start().await else { return };
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let size = 2 * 1024 * 1024;
    std::fs::write(src.path().join("large.bin"), vec![42u8; size]).unwrap();
    let _: serde_json::Value = rcd
        .call("core/bwlimit", &serde_json::json!({"rate": "512k"}))
        .await
        .unwrap();
    rcd.reset_transfer_stats().await.unwrap();
    let source = local(&src);
    let destination = local(&dst);
    let mut observed_partial = false;
    {
        let copy = rcd.copy_file(&source, "large.bin", &destination, "large.bin");
        tokio::pin!(copy);
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                tokio::select! {
                    result = &mut copy => { result.unwrap(); break; }
                    () = tokio::time::sleep(std::time::Duration::from_millis(50)) => {
                        let stats = rcd.transfer_stats().await.unwrap();
                        observed_partial |= stats.bytes > 0 && stats.bytes < size as u64;
                        assert!(stats.speed.is_finite() && stats.speed >= 0.0);
                    }
                }
            }
        })
        .await
        .unwrap();
    }
    assert!(
        observed_partial,
        "no live bytes were observed before completion"
    );
    assert_eq!(rcd.transfer_stats().await.unwrap().bytes, size as u64);
    rcd.reset_transfer_stats().await.unwrap();
    assert_eq!(rcd.transfer_stats().await.unwrap().bytes, 0);
    rcd.shutdown().await.unwrap();
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

#[tokio::test]
async fn config_create_obscures_passwords_and_delete_forgets() {
    let Some(rcd) = start().await else { return };
    let mut params = serde_json::Map::new();
    params.insert("user".into(), "a@example.com".into());
    params.insert("pass".into(), "secret123".into());

    // Defining a mega remote does not log in: that happens on first use.
    rcd.config_create("acct-test", "mega", &params)
        .await
        .unwrap();
    let stored = rcd.config_get("acct-test").await.unwrap();
    assert_eq!(stored["user"], "a@example.com");
    assert_ne!(stored["pass"], "secret123");

    rcd.config_delete("acct-test").await.unwrap();
    assert!(rcd.config_get("acct-test").await.unwrap().is_empty());
    assert!(!format!("{rcd:?}").contains("pass"));
    rcd.shutdown().await.unwrap();
}

#[tokio::test]
async fn config_errors_never_echo_the_request() {
    let Some(rcd) = start().await else { return };
    let mut params = serde_json::Map::new();
    params.insert("pass".into(), "secret123".into());

    let err = rcd
        .config_create("bad", "no-such-backend", &params)
        .await
        .unwrap_err();
    assert!(!err.to_string().contains("secret123"), "{err}");
    rcd.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_missing_folder_is_recognised_but_a_missing_remote_is_not() {
    let Some(rcd) = start().await else { return };
    let dir = tempfile::tempdir().unwrap();

    let missing = rcd.list(&local(&dir), "no-such-folder").await.unwrap_err();
    assert!(missing.is_directory_not_found(), "{missing}");

    let no_remote = rcd.list("acct-undefined:", "").await.unwrap_err();
    assert!(!no_remote.is_directory_not_found(), "{no_remote}");
    rcd.shutdown().await.unwrap();
}

#[tokio::test]
async fn redistribution_calls_preserve_existing_files_and_report_staging_space() {
    let Some(rcd) = start().await else { return };
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    std::fs::write(src.path().join("file"), b"new content").unwrap();
    std::fs::write(dst.path().join("file"), b"keep existing").unwrap();
    let _: serde_json::Value = rcd
        .call(
            "operations/copyfile",
            &serde_json::json!({
                "srcFs":local(&src),"srcRemote":"file","dstFs":local(&dst),"dstRemote":"file",
                "_config":{"IgnoreExisting":true}
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(dst.path().join("file")).unwrap(),
        b"keep existing"
    );
    let disk: serde_json::Value = rcd
        .call("core/du", &serde_json::json!({"dir":dst.path()}))
        .await
        .unwrap();
    assert!(disk["info"]["Available"].as_u64().is_some());
    let rate: serde_json::Value = rcd
        .call(
            "core/bwlimit",
            &serde_json::json!({"rate":"125000B:250000B"}),
        )
        .await
        .unwrap();
    assert_eq!(rate["bytesPerSecondTx"], 125000);
    assert_eq!(rate["bytesPerSecondRx"], 250000);
    let _: serde_json::Value = rcd
        .call(
            "operations/deletefile",
            &serde_json::json!({"fs":local(&dst),"remote":"file"}),
        )
        .await
        .unwrap();
    assert!(!dst.path().join("file").exists());
    assert!(src.path().join("file").exists());
    rcd.shutdown().await.unwrap();
}
