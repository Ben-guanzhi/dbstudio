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
    let query = SqlResult::Query(QueryResult::new(
        Vec::new(),
        vec![vec![ResultCell::text("db_name"), ResultCell::text("extra")]],
        1,
        1,
        1,
        "SELECT DB_NAME()".to_string(),
        false,
    ));
    assert_eq!(
        query.first_cell().map(|c| c.value),
        Some("db_name".to_string())
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
    let query = QueryResult::new(
        Vec::new(),
        vec![vec![ResultCell::text("a")]; 5],
        0,
        0,
        1,
        "SELECT a".to_string(),
        false,
    );

    query.truncate_rows(2);
    assert_eq!(query.row_len(), 2);
    assert_eq!(query.row_count(), 2);
    assert_eq!(query.total_row_count(), 5);
    assert!(query.truncated());

    query.truncate_rows(10);
    assert_eq!(query.row_count(), 2);
    assert_eq!(query.total_row_count(), 2);
    assert!(!query.truncated());
}

#[test]
fn query_result_wire_shape_is_the_flat_seven_field_object() {
    let q = QueryResult::new(
        vec![crate::schema::ColumnInfo::new("id", "integer", 0)],
        vec![
            vec![ResultCell::integer(1), ResultCell::null()],
            vec![ResultCell::text("x"), ResultCell::float(1.5)],
        ],
        2,
        2,
        7,
        "SELECT 1".to_string(),
        true,
    );
    let json = serde_json::to_string(&SqlResult::Query(q.clone())).unwrap();
    // The wire is still the flat 7-field object the plugin protocol has always
    // spoken - no `grid`/`inner` leaked, `rows` still a 2-D array of 4-field cells.
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let obj = v.get("Query").unwrap();
    let keys: std::collections::BTreeSet<_> = obj.as_object().unwrap().keys().cloned().collect();
    let want: std::collections::BTreeSet<String> = [
        "columns",
        "execution_time_ms",
        "row_count",
        "rows",
        "sql",
        "total_row_count",
        "truncated",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    assert_eq!(
        keys, want,
        "the wire field set changed - plugin compat broken"
    );
    let rows = obj.get("rows").unwrap().as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0][0].get("value").unwrap(), "1");
    assert_eq!(rows[0][1].get("is_null").unwrap(), true);
    assert_eq!(
        obj.get("truncated").unwrap(),
        &serde_json::Value::Bool(true)
    );

    // The old-shape JSON deserializes back into a live grid with the same data.
    let back: SqlResult = serde_json::from_str(&json).unwrap();
    let SqlResult::Query(q2) = back else {
        panic!("round-trip lost the Query variant");
    };
    assert_eq!(q2.row_len(), 2);
    assert_eq!(q2.total_row_count(), 2);
    assert!(q2.truncated());
    assert_eq!(q2.execution_time_ms(), 7);
}

#[test]
fn append_rows_grows_in_place_without_copying_the_grid() {
    let base: Vec<Vec<ResultCell>> = (0..1000)
        .map(|r| vec![ResultCell::integer(r as i64)])
        .collect();
    let q = QueryResult::new(
        vec![crate::schema::ColumnInfo::new("v", "integer", 0)],
        base,
        1000,
        1000,
        1,
        "SELECT v".to_string(),
        true,
    );
    // The per-row cell-buffer pointers before the append: an in-place `extend`
    // moves the (small) row `Vec` headers but never deep-copies the cells, so
    // these stay put. A COW clone would have reallocated every cell buffer.
    let row_ptrs: Vec<*const ResultCell> = q.grid.read().rows.iter().map(|r| r.as_ptr()).collect();
    let page: Vec<Vec<ResultCell>> = (1000..1050)
        .map(|r| vec![ResultCell::integer(r as i64)])
        .collect();

    let added = q.append_rows(
        page,
        AppendMeta {
            execution_time_ms: 2,
            truncated: true,
        },
    );
    assert_eq!(added, 50);
    assert_eq!(q.row_len(), 1050);
    assert_eq!(
        q.total_row_count(),
        1051,
        "still truncated => total is len+1"
    );

    let after = q.grid.read();
    for (i, p) in row_ptrs.iter().enumerate() {
        assert_eq!(
            after.rows[i].as_ptr(),
            *p,
            "row {i} was deep-copied by the append, not extended in place"
        );
    }
    drop(after);
    assert_eq!(
        Arc::strong_count(&q.grid),
        1,
        "append must not mint a new grid handle"
    );
}

#[test]
#[ignore = "bench: load-more COW vs in-place append"]
fn bench_load_more_cow_vs_inplace() {
    // A 10k-row base grid; paged load-more appends 10 pages of 1000 rows each.
    // The COW path (what `Arc::make_mut` did) deep-copies the accumulated grid
    // every page - Θ(k²·P). The in-place `append_rows` is Θ(page) per page.
    const BASE: usize = 10_000;
    const PAGES: usize = 10;
    const PAGE: usize = 1_000;

    let q = QueryResult::new(
        vec![crate::schema::ColumnInfo::new("v", "integer", 0)],
        (0..BASE)
            .map(|r| {
                vec![
                    ResultCell::integer(r as i64),
                    ResultCell::text(format!("t{r}")),
                ]
            })
            .collect(),
        BASE,
        BASE,
        1,
        "SELECT v".to_string(),
        true,
    );

    // COW arm: each page deep-copies the whole accumulated grid, as the old
    // `Arc::make_mut` COW did.
    let t0 = std::time::Instant::now();
    let mut cow = q.rows_snapshot(); // Θ(total) copy, once
    for p in 0..PAGES {
        let page: Vec<Vec<ResultCell>> = (0..PAGE)
            .map(|i| {
                let v = BASE + p * PAGE + i;
                vec![
                    ResultCell::integer(v as i64),
                    ResultCell::text(format!("t{v}")),
                ]
            })
            .collect();
        let mut next = q.rows_snapshot(); // the COW: re-copy the accumulated grid
        next.append(&mut cow);
        next.extend(page);
        cow = next;
    }
    let cow_elapsed = t0.elapsed();
    std::hint::black_box(&cow);

    // In-place arm: the new path - append each page directly (Θ(page)).
    let t1 = std::time::Instant::now();
    for p in 0..PAGES {
        let page: Vec<Vec<ResultCell>> = (0..PAGE)
            .map(|i| {
                let v = BASE + p * PAGE + i;
                vec![
                    ResultCell::integer(v as i64),
                    ResultCell::text(format!("t{v}")),
                ]
            })
            .collect();
        q.append_rows(
            page,
            AppendMeta {
                execution_time_ms: 1,
                truncated: true,
            },
        );
    }
    let inplace_elapsed = t1.elapsed();
    std::hint::black_box(q.grid.read().rows.len());

    let ratio = cow_elapsed.as_secs_f64().max(1e-9) / inplace_elapsed.as_secs_f64().max(1e-9);
    println!(
        "load-more {PAGES}×{PAGE} onto {BASE}-row grid: COW={cow_elapsed:?} in-place={inplace_elapsed:?} ratio={ratio:.2}×"
    );
}
