use super::{empty_tables, ConnectionSession};
use dbstudio_core::schema::{TableInfo, TableType};
use std::sync::Arc;

fn table(name: &str) -> TableInfo {
    TableInfo {
        name: name.to_string(),
        schema: Some("public".to_string()),
        database: Some("app".to_string()),
        table_type: TableType::Table,
        row_count: Some(1),
        comment: None,
    }
}

#[test]
fn a_shared_table_list_copy_on_writes_onto_a_new_address() {
    let mut tables = Arc::new(vec![table("orders")]);
    let mirror = Arc::clone(&tables);
    let before = Arc::as_ptr(&tables);

    Arc::make_mut(&mut tables).push(table("users"));

    assert_ne!(Arc::as_ptr(&tables), before, "make_mut reused the address");
    assert_ne!(Arc::as_ptr(&tables), Arc::as_ptr(&mirror));
    assert_eq!(
        mirror.len(),
        1,
        "the mirrored list was mutated through the handle"
    );
    assert_eq!(tables.len(), 2);
}

#[test]
fn an_unshared_table_list_mutates_in_place() {
    let mut tables = Arc::new(vec![table("orders")]);
    let before = Arc::as_ptr(&tables);

    Arc::make_mut(&mut tables).push(table("users"));

    assert_eq!(Arc::as_ptr(&tables), before);
    assert_eq!(tables.len(), 2);
}

#[test]
fn a_session_starts_on_its_own_empty_handle() {
    let session = ConnectionSession::new(1, "main");

    assert!(session.tables.is_empty());
    assert_ne!(Arc::as_ptr(&session.tables), Arc::as_ptr(empty_tables()));
}

#[test]
fn the_shared_empty_handle_does_not_move() {
    assert!(empty_tables().is_empty());
    assert_eq!(Arc::as_ptr(empty_tables()), Arc::as_ptr(empty_tables()));
}
