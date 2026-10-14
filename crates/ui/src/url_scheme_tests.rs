use super::*;

#[test]
fn test_parse_connect_url() {
    let url = "dbstudio://connect?type=mysql&host=localhost&port=3306&database=mydb&username=root&password=secret";
    let parsed = DbStudioUrl::parse(url).unwrap();
    assert!(parsed.is_connect());
    assert_eq!(parsed.db_type(), Some("mysql"));
    assert_eq!(parsed.host(), Some("localhost"));
    assert_eq!(parsed.port(), Some(3306));
    assert_eq!(parsed.database(), Some("mydb"));
    assert_eq!(parsed.username(), Some("root"));
    assert_eq!(parsed.password(), Some("secret"));
}

#[test]
fn test_parse_sqlite_url() {
    let url = "dbstudio://connect?type=sqlite&database=/path/to/db.sqlite";
    let parsed = DbStudioUrl::parse(url).unwrap();
    assert!(parsed.is_connect());
    assert_eq!(parsed.db_type(), Some("sqlite"));
    assert_eq!(parsed.database(), Some("/path/to/db.sqlite"));
}

#[test]
fn test_invalid_scheme() {
    assert!(DbStudioUrl::parse("http://example.com").is_none());
}

#[test]
fn test_invalid_action() {
    let url = "dbstudio://invalid";
    let parsed = DbStudioUrl::parse(url).unwrap();
    assert!(!parsed.is_connect());
}
