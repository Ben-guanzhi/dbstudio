use super::*;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "dbstudio-store-test-{}-{}",
        tag,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn copy_if_missing_migrates_legacy_store() {
    let dir = temp_dir("migrate");
    let legacy = dir.join("legacy").join("dbclient.db");
    let target = dir.join("new").join("dbstudio.db");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, b"store").unwrap();

    copy_if_missing(&legacy, &target).unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"store");

    // Legacy original is kept so migrations can be retried.
    assert!(legacy.exists());

    // Idempotent: an existing target is never overwritten.
    std::fs::write(&target, b"newer").unwrap();
    copy_if_missing(&legacy, &target).unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"newer");
}

#[test]
fn copy_if_missing_tolerates_missing_legacy_store() {
    let dir = temp_dir("fresh");
    let target = dir.join("new").join("dbstudio.db");
    copy_if_missing(&dir.join("does-not-exist.db"), &target).unwrap();
    assert!(!target.exists());
}
