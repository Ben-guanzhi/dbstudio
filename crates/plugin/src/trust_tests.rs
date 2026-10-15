use std::fs;
use std::path::{Path, PathBuf};

use crate::loader::{PluginFile, PluginManager};
use crate::trust::{require_trusted, PluginTrust, TrustDecision};

/// 每个测试一个独立目录，避免并行测试互相踩清单。
fn sandbox(tag: &str) -> (PathBuf, PluginTrust) {
    let dir =
        std::env::temp_dir().join(format!("dbstudio-trust-test-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let trust = PluginTrust::at(dir.join("plugin_trust.json"));
    (dir, trust)
}

fn write_plugin(dir: &Path, name: &str, body: &[u8]) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).unwrap();
    path
}

#[test]
fn fingerprint_matches_the_reference_vector() {
    // 锁死 SHA-256 本身，免得哪天换实现把摘要算错而没人发现。
    let (dir, _) = sandbox("vector");
    let file = write_plugin(&dir, "a.dll", b"abc");

    let (hash, _) = PluginTrust::fingerprint(&file).unwrap();
    assert_eq!(
        hash,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn fingerprint_is_reproducible_and_reports_the_size() {
    let (dir, _) = sandbox("repro");
    let file = write_plugin(&dir, "a.dll", b"driver bytes");

    let (hash, size) = PluginTrust::fingerprint(&file).unwrap();
    let (again, _) = PluginTrust::fingerprint(&file).unwrap();

    assert_eq!(hash, again, "摘要必须可复现");
    assert_eq!(size, 12);
    assert_eq!(hash.len(), 64);
    assert!(
        hash.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "摘要应为小写十六进制: {hash}"
    );
}

#[test]
fn fingerprint_moves_when_the_file_changes() {
    let (dir, _) = sandbox("content");
    let file = write_plugin(&dir, "a.dll", b"v1");
    let first = PluginTrust::fingerprint(&file).unwrap().0;

    fs::write(&file, b"v2").unwrap();
    let second = PluginTrust::fingerprint(&file).unwrap().0;

    assert_ne!(first, second);
}

#[test]
fn a_missing_store_means_nothing_is_trusted() {
    let (_, trust) = sandbox("empty");
    assert!(trust.list().unwrap().is_empty());
}

#[test]
fn a_corrupt_store_is_an_error_not_an_empty_list() {
    // 半截或写坏的清单不能被当成"没信任任何插件"，那会静默放过恶意文件。
    let (_, trust) = sandbox("corrupt");
    fs::write(trust.file_path(), b"{ this is not json").unwrap();

    let err = trust.load_store().unwrap_err().to_string();
    assert!(err.contains("JSON"), "报错要点明清单坏了: {err}");
}

#[test]
fn an_unknown_plugin_is_not_trusted() {
    let (dir, trust) = sandbox("unknown");
    let file = write_plugin(&dir, "rogue.dll", b"\x00\x01\x02");

    assert_eq!(trust.check(&file).unwrap(), TrustDecision::Unknown);
}

#[test]
fn require_trusted_refuses_an_unknown_plugin() {
    let (dir, trust) = sandbox("refuse");
    let file = write_plugin(&dir, "rogue.dll", b"\x00\x01\x02");

    let err = require_trusted(&trust, &file).unwrap_err().to_string();
    assert!(
        err.contains("信任清单"),
        "报错要说明是被信任清单拦下的: {err}"
    );
    assert!(
        err.contains("plugin_trust.json"),
        "报错要告诉用户清单在哪: {err}"
    );
}

#[test]
fn trust_then_check_succeeds() {
    let (dir, trust) = sandbox("trust");
    let file = write_plugin(&dir, "driver.dll", b"signed driver");

    let recorded = trust.trust(&file).unwrap();
    let (actual, _) = PluginTrust::fingerprint(&file).unwrap();
    assert_eq!(recorded, actual, "登记的应当是当前内容的摘要");

    assert_eq!(trust.check(&file).unwrap(), TrustDecision::Trusted);
    assert!(require_trusted(&trust, &file).is_ok());
}

#[test]
fn a_replaced_file_is_reported_as_tampered() {
    let (dir, trust) = sandbox("tamper");
    let file = write_plugin(&dir, "driver.dll", b"original");
    let expected = trust.trust(&file).unwrap();

    fs::write(&file, b"something else entirely").unwrap();
    let actual = PluginTrust::fingerprint(&file).unwrap().0;

    match trust.check(&file).unwrap() {
        TrustDecision::Tampered {
            expected: got_expected,
            actual: got_actual,
        } => {
            assert_eq!(got_expected, expected);
            assert_eq!(got_actual, actual);
        }
        other => panic!("替换后的文件应判为 tampered，实际 {other:?}"),
    }
    assert!(require_trusted(&trust, &file).is_err());
}

#[test]
fn an_intentional_update_can_be_retrusted() {
    let (dir, trust) = sandbox("refresh");
    let file = write_plugin(&dir, "driver.dll", b"v1");
    trust.trust(&file).unwrap();

    fs::write(&file, b"v2").unwrap();
    assert!(matches!(
        trust.check(&file).unwrap(),
        TrustDecision::Tampered { .. }
    ));

    let refreshed = trust.trust(&file).unwrap();
    assert_eq!(refreshed, PluginTrust::fingerprint(&file).unwrap().0);
    assert_eq!(trust.check(&file).unwrap(), TrustDecision::Trusted);
    assert_eq!(trust.list().unwrap().len(), 1, "重复登记不应留下重复条目");
}

#[test]
fn revoke_drops_the_entry() {
    let (dir, trust) = sandbox("revoke");
    let file = write_plugin(&dir, "driver.dll", b"bytes");
    trust.trust(&file).unwrap();
    assert_eq!(trust.check(&file).unwrap(), TrustDecision::Trusted);

    trust.revoke(&file).unwrap();
    assert_eq!(trust.check(&file).unwrap(), TrustDecision::Unknown);
    assert!(trust.list().unwrap().is_empty());
}

#[test]
fn the_store_survives_being_read_by_a_fresh_handle() {
    // 登记与加载之间进程会重启，清单必须真的落在磁盘上。
    let dir = std::env::temp_dir().join(format!(
        "dbstudio-trust-test-{}-persist",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    let file = write_plugin(&dir, "driver.dll", b"persist me");
    PluginTrust::at(dir.join("plugin_trust.json"))
        .trust(&file)
        .unwrap();

    let reloaded = PluginTrust::at(dir.join("plugin_trust.json"));
    assert_eq!(reloaded.check(&file).unwrap(), TrustDecision::Trusted);
}

#[cfg(windows)]
#[test]
fn an_equivalent_path_spelling_still_matches() {
    // 登记时用规范路径，检查时可能带多余分隔符或大小写差异。
    // 只限 Windows：反斜杠在 POSIX 上是普通文件名字符。
    let (dir, trust) = sandbox("spelling");
    let file = write_plugin(&dir, "driver.dll", b"bytes");
    trust.trust(&file).unwrap();

    let lossy = dir.to_string_lossy();
    let loose = Path::new(&format!("{lossy}\\")).join("driver.dll");
    assert_eq!(trust.check(&loose).unwrap(), TrustDecision::Trusted);
}

/// The library extension is platform-specific, so the scan tests write a file
/// that the current platform actually recognises as a plugin.
fn plugin_ext() -> &'static str {
    match std::env::consts::OS {
        "windows" => "dll",
        "macos" => "dylib",
        _ => "so",
    }
}

fn write_file(dir: &Path, name: &str, body: &[u8]) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).unwrap();
    path
}

/// A manager whose scan only has to see files this test created. `sandbox`
/// returns a throwaway directory, which is where the trust store lands; the
/// built-in default directories stay untouched.
fn scan_sandbox(tag: &str) -> (PathBuf, PluginManager) {
    let (dir, trust) = sandbox(tag);
    let mut manager = PluginManager::with_trust(trust);
    manager.add_plugin_dir(dir.clone());
    (dir, manager)
}

fn found(mgr: &PluginManager, path: &Path) -> Option<PluginFile> {
    let stem = path.file_name()?;
    mgr.scan_plugins()
        .into_iter()
        .find(|f| f.path.file_name() == Some(stem))
}

#[test]
fn a_scan_reports_every_plugin_file_on_disk() {
    let (dir, manager) = scan_sandbox("scan-unknown");
    let file = write_file(
        &dir,
        &format!("driver.{ext}", ext = plugin_ext()),
        b"original",
    );

    let plugin = found(&manager, &file).unwrap();
    assert_eq!(plugin.decision, TrustDecision::Unknown);
    assert_eq!(plugin.bytes, 8);

    manager.trust_plugin(&file).unwrap();
    let plugin = found(&manager, &file).unwrap();
    assert_eq!(plugin.decision, TrustDecision::Trusted);

    fs::write(&file, b"modified").unwrap();
    let plugin = found(&manager, &file).unwrap();
    match plugin.decision {
        TrustDecision::Tampered { expected, actual } => {
            assert_ne!(expected, actual);
            assert_eq!(actual, PluginTrust::fingerprint(&file).unwrap().0);
        }
        other => panic!("expected Tampered after the file changed, got {other:?}"),
    }

    manager.revoke_plugin(&file).unwrap();
    let plugin = found(&manager, &file).unwrap();
    assert_eq!(plugin.decision, TrustDecision::Unknown);
}

#[test]
fn a_scan_only_reports_plugin_libraries() {
    let (dir, manager) = scan_sandbox("scan-noise");

    // Anything that is not a library of the right extension is invisible to
    // the loader, so it must also be invisible to the trust UI.
    write_file(&dir, "README.md", b"how to install drivers");
    write_file(&dir, "driver.txt", b"not a library");
    write_file(
        &dir,
        &format!("driver.{other}", other = other_ext()),
        b"wrong platform",
    );
    write_file(&dir, "notes", b"no extension at all");
    fs::create_dir_all(dir.join("nested")).unwrap();

    let real = write_file(&dir, &format!("driver.{ext}", ext = plugin_ext()), b"real");

    let plugins = manager.scan_plugins();
    assert!(plugins.iter().all(|p| p.file_name() != "README.md"));
    assert!(plugins.iter().all(|p| p.file_name() != "driver.txt"));
    assert!(plugins.iter().all(|p| p.file_name() != "notes"));
    assert!(
        found(&manager, &real).is_some(),
        "the real plugin must still be found"
    );
}

/// An extension the current platform deliberately does *not* treat as a plugin.
fn other_ext() -> &'static str {
    match std::env::consts::OS {
        "windows" => "so",
        "macos" => "dll",
        _ => "dll",
    }
}

#[test]
fn a_precomputed_hash_agrees_with_a_direct_check() {
    let (dir, trust) = sandbox("decide");
    let file = write_file(
        &dir,
        &format!("driver.{ext}", ext = plugin_ext()),
        b"stable bytes",
    );

    trust.trust(&file).unwrap();

    let (hash, _) = PluginTrust::fingerprint(&file).unwrap();
    assert_eq!(
        trust.decide(&file, &hash).unwrap(),
        trust.check(&file).unwrap()
    );

    let bogus = "0".repeat(hash.len());
    assert!(
        matches!(
            trust.decide(&file, &bogus).unwrap(),
            TrustDecision::Tampered { .. }
        ),
        "a wrong digest must still be caught"
    );
}
