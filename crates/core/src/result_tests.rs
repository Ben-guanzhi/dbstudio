use super::*;

fn row() -> Row {
    vec![
        ResultCell::text("alice"),
        ResultCell::null(),
        ResultCell::text("42"),
    ]
}

#[test]
fn row_ext_accessors_handle_missing_and_null_cells() {
    let row = row();
    assert_eq!(row.first_str(), "alice");
    assert_eq!(row.cell_str(2), "42");
    // NULL and out-of-range indices both read as empty
    assert_eq!(row.cell(1), "");
    assert_eq!(row.cell(99), "");
    assert_eq!(row.cell_str(99), "");
    assert_eq!(row.first_str(), "alice");
}

#[test]
fn null_payload_is_ignored_by_row_accessors() {
    let row = vec![
        ResultCell {
            value: "42".to_string(),
            is_null: true,
            kind: CellType::Other,
            sort_value: None,
        },
        ResultCell::text(""),
        ResultCell::text("42"),
    ];
    assert_eq!(row.cell(0), "");
    assert_eq!(row.cell_str(0), "");
    assert_eq!(row.first_str(), "");
    assert_eq!(row.cell_u32(0, 7), 7);
    assert!(!row.cell_is(0, "42"));
    assert!(!row.cell_is(0, ""));
    assert!(row.cell_is(1, ""));
    assert_eq!(row.cell_u32(2, 7), 42);
}

#[test]
fn row_ext_predicates_and_number_parsing() {
    let row = row();
    assert!(row.cell_is(0, "alice"));
    assert!(!row.cell_is(0, "bob"));
    // NULL never matches, not even the empty string
    assert!(!row.cell_is(1, ""));
    assert!(!row.cell_is(1, "alice"));
    assert!(!row.cell_is(99, ""));
    assert_eq!(row.cell_u32(2, 7), 42);
    assert_eq!(row.cell_u32(0, 7), 7);
    assert_eq!(row.cell_u32(99, 7), 7);
}

#[test]
fn value_if_not_null_filters_nulls() {
    assert_eq!(
        ResultCell::text("x").value_if_not_null().as_deref(),
        Some("x")
    );
    assert_eq!(ResultCell::null().value_if_not_null(), None);
}

#[test]
fn first_cell_and_first_row_cover_all_result_shapes() {
    let query = SqlResult::Query(QueryResult {
        columns: Vec::new(),
        rows: vec![vec![ResultCell::text("db_name"), ResultCell::text("extra")]],
        row_count: 1,
        total_row_count: 1,
        execution_time_ms: 1,
        sql: "SELECT DB_NAME()".to_string(),
        truncated: false,
    });
    assert_eq!(
        query.first_cell().map(|c| c.value.as_str()),
        Some("db_name")
    );
    assert_eq!(
        query.first_row().map(|r| r.cell_str(1)),
        Some("extra".to_string())
    );

    let empty = SqlResult::Query(QueryResult::empty("SELECT 1"));
    assert!(empty.first_cell().is_none());
    assert!(empty.first_row().is_none());

    let modified = SqlResult::Modified(ExecResult {
        rows_affected: 1,
        execution_time_ms: 1,
        sql: "DELETE FROM t".to_string(),
        message: "1 rows affected".to_string(),
    });
    assert!(modified.first_cell().is_none());
    assert!(modified.first_row().is_none());
}

#[test]
fn sort_cmp_orders_by_type_and_null_last() {
    use std::cmp::Ordering;
    assert_eq!(
        ResultCell::integer(2).sort_cmp(&ResultCell::integer(10)),
        Ordering::Less
    );
    assert_eq!(
        ResultCell::float(2.5).sort_cmp(&ResultCell::float(1.5)),
        Ordering::Greater
    );
    assert_eq!(
        ResultCell::integer(2).sort_cmp(&ResultCell::float(2.0)),
        Ordering::Equal
    );
    assert_eq!(
        ResultCell::text("a").sort_cmp(&ResultCell::text("b")),
        Ordering::Less
    );
    assert_eq!(
        ResultCell::null().sort_cmp(&ResultCell::integer(1)),
        Ordering::Greater
    );
    assert_eq!(
        ResultCell::integer(1).sort_cmp(&ResultCell::null()),
        Ordering::Less
    );
    assert_eq!(
        ResultCell::null().sort_cmp(&ResultCell::null()),
        Ordering::Equal
    );
}

#[test]
fn sort_cmp_separates_numeric_proxy_cells_from_text_cells() {
    use std::cmp::Ordering;
    // Both groups are internally ordered; a proxy-backed cell never
    // compares through the display string of a text-only cell.
    let num_2 = ResultCell::integer(2);
    let num_10 = ResultCell::integer(10);
    let text_10 = ResultCell::text("10");
    let text_2 = ResultCell::text("2");
    assert_eq!(num_2.sort_cmp(&num_10), Ordering::Less);
    assert_eq!(text_10.sort_cmp(&text_2), Ordering::Less);
    assert_eq!(num_10.sort_cmp(&text_10), Ordering::Less);
    assert_eq!(text_10.sort_cmp(&num_10), Ordering::Greater);
}

#[test]
fn sort_cmp_places_nan_deterministically() {
    use std::cmp::Ordering;
    let nan = ResultCell::float(f64::NAN);
    assert_eq!(nan.sort_cmp(&nan), Ordering::Equal);
    assert_eq!(nan.sort_cmp(&ResultCell::float(1.0)), Ordering::Greater);
    assert_eq!(ResultCell::float(1.0).sort_cmp(&nan), Ordering::Less);
}

/// The relation `sort_by` needs: irreflexive-transitive strict order plus a
/// consistent equality. Checked exhaustively over a corpus that mixes every
/// combination the old implementation could contradict itself on.
#[test]
fn sort_cmp_is_a_total_order() {
    use std::cmp::Ordering;
    let corpus = [
        ResultCell::null(),
        ResultCell::integer(2),
        ResultCell::integer(10),
        ResultCell::float(f64::NAN),
        ResultCell::text("10"),
        ResultCell::text("2"),
        ResultCell::typed(CellType::DateTime, "2026-01-05"),
    ];
    let lt = |a: &ResultCell, b: &ResultCell| a.sort_cmp(b) == Ordering::Less;
    let eq = |a: &ResultCell, b: &ResultCell| a.sort_cmp(b) == Ordering::Equal;

    for a in &corpus {
        assert!(eq(a, a), "a cell must compare equal to itself: {a:?}");
        for b in &corpus {
            assert!(!(lt(a, b) && lt(b, a)), "asymmetry violated: {a:?} / {b:?}");
            for c in &corpus {
                if lt(a, b) && lt(b, c) {
                    assert!(lt(a, c), "transitivity violated: {a:?} < {b:?} < {c:?}");
                }
                if eq(a, b) {
                    assert_eq!(
                        a.sort_cmp(c),
                        b.sort_cmp(c),
                        "equal cells must be interchangeable: {a:?} / {b:?} vs {c:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn truncate_rows_reports_totals() {
    let mut query = QueryResult {
        columns: Vec::new(),
        rows: vec![vec![ResultCell::text("a")]; 5],
        row_count: 0,
        total_row_count: 0,
        execution_time_ms: 1,
        sql: "SELECT a".to_string(),
        truncated: false,
    };

    query.truncate_rows(2);
    assert_eq!(query.rows.len(), 2);
    assert_eq!(query.row_count, 2);
    assert_eq!(query.total_row_count, 5);
    assert!(query.truncated);

    query.truncate_rows(10);
    assert_eq!(query.row_count, 2);
    assert_eq!(query.total_row_count, 2);
    assert!(!query.truncated);
}
