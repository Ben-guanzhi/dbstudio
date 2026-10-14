use super::*;

#[test]
fn connect_is_explicit_and_not_a_lost() {
    let e = DbError::connect("could not reach host");
    assert!(matches!(e, DbError::Connect { .. }));
    assert!(!e.is_connection_lost());
    assert_eq!(e.message(), "could not reach host");
}

#[test]
fn lost_is_reported_as_connection_lost() {
    let e = DbError::lost("connection reset by peer");
    assert!(e.is_connection_lost());
}

#[test]
fn from_anyhow_promotes_timeout_markers() {
    let err = anyhow::anyhow!("request to server failed: deadline exceeded");
    assert!(matches!(
        DbError::from_anyhow(&err),
        DbError::Timeout { .. }
    ));
}

#[test]
fn from_anyhow_promotes_reset_to_lost() {
    let err = anyhow::anyhow!("tcp connect error: connection reset by peer");
    let classified = DbError::from_anyhow(&err);
    assert!(matches!(classified, DbError::Lost { .. }));
    assert!(classified.is_connection_lost());
}

#[test]
fn from_anyhow_keeps_unknown_as_other() {
    let err = anyhow::anyhow!("weird driver bug");
    assert!(matches!(DbError::from_anyhow(&err), DbError::Other { .. }));
    assert!(!DbError::from_anyhow(&err).is_connection_lost());
}

#[test]
fn sql_problems_are_not_misread_as_lost() {
    // A SQL error that merely contains the word "connection" in its text
    // must not be promoted to Lost (no loss marker present).
    let err = anyhow::anyhow!("relation \"connection_log\" does not exist");
    let classified = DbError::from_anyhow(&err);
    assert!(!classified.is_connection_lost());
}

#[test]
fn db_error_converts_into_anyhow() {
    let e = DbError::other("boom");
    let a: anyhow::Error = e.into();
    assert_eq!(a.to_string(), "boom");
}
