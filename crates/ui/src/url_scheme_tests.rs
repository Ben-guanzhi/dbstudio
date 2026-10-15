use super::*;

#[test]
fn test_parse_connect_url() {
    let url = "dbstudio://connect?id=9f2c1a3e-77d0-4b8e-9c51-2f4a6b8c0d11&nonce=0e8d3b9a4f7c6d5e4c3b2a190817f6e5";
    let parsed = DbStudioUrl::parse(url).unwrap();
    assert!(parsed.is_connect());
    assert_eq!(parsed.connection_id, "9f2c1a3e-77d0-4b8e-9c51-2f4a6b8c0d11");
    assert_eq!(parsed.nonce, "0e8d3b9a4f7c6d5e4c3b2a190817f6e5");
}

#[test]
fn test_parse_url_without_nonce_is_still_a_url() {
    // 缺 nonce 不算解析失败：解析端把它留空，由处理端给出可读的报错。
    // 静默丢弃一个形似深链的启动参数会让排障更难。
    let parsed = DbStudioUrl::parse("dbstudio://connect?id=some-connection").unwrap();
    assert!(parsed.is_connect());
    assert_eq!(parsed.connection_id, "some-connection");
    assert_eq!(parsed.nonce, "");
}

#[test]
fn test_legacy_credential_url_is_rejected() {
    // 旧格式把明文凭据拼在 URL 里；新格式必须解析失败，不能静默回退成
    // "凭据仍然生效"。
    let legacy =
        "dbstudio://connect?type=mysql&host=localhost&port=3306&username=root&password=secret";
    assert!(
        DbStudioUrl::parse(legacy).is_none(),
        "a credential-bearing URL has no id and must not parse"
    );
}

#[test]
fn test_missing_id_is_rejected() {
    assert!(DbStudioUrl::parse("dbstudio://connect?nonce=abc").is_none());
    assert!(DbStudioUrl::parse("dbstudio://connect?id=%20&nonce=abc").is_none());
}

#[test]
fn test_build_url_roundtrip() {
    let url = build_url("conn-123", "nonce-456");
    let parsed = DbStudioUrl::parse(&url).unwrap();
    assert_eq!(parsed.connection_id, "conn-123");
    assert_eq!(parsed.nonce, "nonce-456");
}

#[test]
fn test_nonce_matches() {
    assert!(nonce_matches("abc123", "abc123"));
    assert!(!nonce_matches("abc123", "abc124"));
    assert!(!nonce_matches("abc123", "abc12"));
    assert!(!nonce_matches("abc123", "abc1234"));
    // 空期望值必须拒绝，否则一个不带 nonce 的深链就能匹配空令牌。
    assert!(!nonce_matches("", ""));
    assert!(!nonce_matches("", "abc"));
}

#[test]
fn test_invalid_scheme() {
    assert!(DbStudioUrl::parse("http://example.com").is_none());
}

#[test]
fn test_invalid_action() {
    let url = "dbstudio://invalid?id=x&nonce=y";
    let parsed = DbStudioUrl::parse(url).unwrap();
    assert!(!parsed.is_connect());
}
