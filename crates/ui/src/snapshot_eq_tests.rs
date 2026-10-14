use crate::state::FavoriteEntry;
use dbstudio_core::models::{ConnectionConfig, DatabaseType, Environment, SslMode};
use dbstudio_core::schema::{TableInfo, TableType};
use dbstudio_storage::types::QueryHistoryEntry;

fn table() -> TableInfo {
    TableInfo {
        name: "orders".to_string(),
        schema: Some("public".to_string()),
        database: Some("app".to_string()),
        table_type: TableType::Table,
        row_count: Some(12),
        comment: Some("audit trail".to_string()),
    }
}

fn history() -> QueryHistoryEntry {
    QueryHistoryEntry {
        id: 7,
        connection_id: "conn-1".to_string(),
        sql: "SELECT 1".to_string(),
        execution_time_ms: 42,
        row_count: Some(1),
        is_error: false,
        executed_at: "2026-09-30 10:00:00".to_string(),
    }
}

fn favorite() -> FavoriteEntry {
    FavoriteEntry {
        id: 3,
        name: "Top customers".to_string(),
        sql: "SELECT 1".to_string(),
        connection_id: Some("conn-1".to_string()),
    }
}

fn connection() -> ConnectionConfig {
    ConnectionConfig::new(DatabaseType::PostgreSQL, "prod-db".to_string())
}

type EditFn<T> = fn(&mut T);

/// Applies each edit to a clone of `base` and requires the result to differ
/// from `base`. Comparing against a saved clone (not a freshly built value)
/// matters: `ConnectionConfig::new` mints a random id and timestamp, so a
/// fresh second value would differ on its own and prove nothing.
fn each_edit_changes<T: Clone + PartialEq + std::fmt::Debug>(
    base: &T,
    edits: &[(&str, EditFn<T>)],
) {
    assert_eq!(base, &base.clone());
    for (field, edit) in edits {
        let mut v = base.clone();
        edit(&mut v);
        assert_ne!(&v, base, "field `{field}` is invisible to PartialEq");
    }
}

/// Every field of every mirrored type must participate in equality, so a
/// one-field edit can never be mistaken for "nothing changed".
#[test]
fn equality_is_field_sensitive() {
    each_edit_changes(
        &table(),
        &[
            ("name", |t| t.name = "orders_2".into()),
            ("schema", |t| t.schema = None),
            ("database", |t| t.database = None),
            ("table_type", |t| t.table_type = TableType::View),
            ("row_count", |t| t.row_count = None),
            ("comment", |t| t.comment = None),
        ],
    );

    each_edit_changes(
        &history(),
        &[
            ("id", |h| h.id += 1),
            ("connection_id", |h| h.connection_id = "conn-2".into()),
            ("sql", |h| h.sql.push(' ')),
            ("execution_time_ms", |h| h.execution_time_ms += 1),
            ("row_count", |h| h.row_count = None),
            ("is_error", |h| h.is_error = true),
            ("executed_at", |h| {
                h.executed_at = "2026-09-30 10:00:01".into()
            }),
        ],
    );

    each_edit_changes(
        &favorite(),
        &[
            ("id", |f| f.id += 1),
            ("name", |f| f.name = "Other".into()),
            ("sql", |f| f.sql = "SELECT 2".into()),
            ("connection_id", |f| f.connection_id = None),
        ],
    );

    each_edit_changes(
        &connection(),
        &[
            ("id", |c| c.id = "other".into()),
            ("name", |c| c.name = "stage-db".into()),
            ("db_type", |c| c.db_type = DatabaseType::MySQL),
            ("host", |c| c.host = "db.internal".into()),
            ("port", |c| c.port += 1),
            ("database", |c| c.database = "app".into()),
            ("username", |c| c.username = "readonly".into()),
            ("color", |c| c.color = Some("#000000".into())),
            ("environment", |c| c.environment = Environment::Production),
            ("ssl_mode", |c| c.ssl_mode = SslMode::Require),
            ("group", |c| c.group = Some("Prod".into())),
            ("tags", |c| c.tags = vec!["core".into()]),
            ("ssh_enabled", |c| c.ssh_enabled = true),
            ("ssh_host", |c| c.ssh_host = Some("bastion".into())),
            ("ssh_port", |c| c.ssh_port = Some(2222)),
            ("ssh_username", |c| c.ssh_username = Some("tunnel".into())),
            ("ssh_auth_type", |c| c.ssh_auth_type = Some("key".into())),
            ("ssh_key_path", |c| c.ssh_key_path = Some("/k".into())),
            ("extra_params", |c| {
                c.extra_params = Some("sslmode=require".into())
            }),
            ("plugin_name", |c| c.plugin_name = Some("duckdb".into())),
            ("created_at", |c| {
                c.created_at = "2026-01-01 00:00:00".into()
            }),
            ("updated_at", |c| {
                c.updated_at = "2026-09-30 10:00:00".into()
            }),
        ],
    );
}

/// The mirrored lists are compared as slices: the history panel prepends, so
/// a pure reordering has to count as a change.
#[test]
fn list_equality_is_order_sensitive() {
    let a = history();
    let mut b = history();
    b.id += 1;
    assert_ne!(vec![a.clone(), b.clone()], vec![b.clone(), a.clone()]);
    assert_eq!(vec![a.clone(), b.clone()], vec![a, b]);
}
