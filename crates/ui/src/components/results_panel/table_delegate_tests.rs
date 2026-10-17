use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use dbstudio_core::result::{AppendMeta, QueryResult, ResultCell, SqlResult};
use dbstudio_core::schema::ColumnInfo;
use gpui::px;

use super::{
    operand_preparations, reset_operand_preparations, ColumnFilter, FilterOp, PreparedFilter,
    ResultsTableDelegate, WIDTH_SAMPLE_ROWS,
};

fn delegate() -> ResultsTableDelegate {
    ResultsTableDelegate::new(
        Rc::new(RefCell::new(None)),
        Rc::new(RefCell::new(None)),
        Rc::new(RefCell::new(None)),
        Rc::new(RefCell::new(None)),
    )
}

fn grid(columns: &[&str], rows: Vec<Vec<String>>) -> QueryResult {
    QueryResult::new(
        columns
            .iter()
            .enumerate()
            .map(|(ix, name)| ColumnInfo::new(*name, "text", ix as u32))
            .collect(),
        rows.into_iter()
            .map(|row| row.into_iter().map(ResultCell::text).collect())
            .collect(),
        0,
        0,
        0,
        String::new(),
        false,
    )
}

fn result(columns: &[&str], rows: Vec<Vec<String>>) -> Option<Arc<SqlResult>> {
    Some(Arc::new(SqlResult::Query(grid(columns, rows))))
}

/// The delegate's shared live grid, as an `Arc` handle (its address is the
/// "same grid" identity the panel compares).
fn delegate_grid(d: &ResultsTableDelegate) -> Option<Arc<dbstudio_core::result::Grid>> {
    match d.source.as_deref() {
        Some(SqlResult::Query(q)) => Some(Arc::clone(&q.grid)),
        _ => None,
    }
}

#[test]
fn cell_named_resolves_headers_like_the_scan_did() {
    let mut d = delegate();
    d.update(result(
        &["id", "name", "id"],
        vec![vec![
            String::from("1"),
            String::from("a"),
            String::from("9"),
        ]],
    ));
    let found = |d: &ResultsTableDelegate, name: &str, row: usize| {
        d.cell_named(row, name).map(|c| c.value.clone())
    };
    assert_eq!(found(&d, "id", 0).as_deref(), Some("1"));
    assert_eq!(found(&d, "name", 0).as_deref(), Some("a"));
    assert_eq!(found(&d, "#", 0), None);
    assert_eq!(found(&d, "missing", 0), None);
    assert_eq!(found(&d, "name", 1), None);
    for col in d.columns.iter() {
        let first = d.columns.iter().position(|c| c.name == col.name).unwrap();
        assert_eq!(d.column_ix[&col.name.to_string()], first);
    }
}

#[test]
fn width_measurement_stops_at_the_sample_and_the_clamp() {
    let mut rows: Vec<Vec<String>> = (0..WIDTH_SAMPLE_ROWS)
        .map(|_| vec![String::from("short")])
        .collect();
    rows.push(vec!["x".repeat(5000)]);
    let mut d = delegate();
    d.update(result(&["v"], rows));
    assert_eq!(d.columns[1].width, px(80.0));

    let mut d = delegate();
    d.update(result(&["v"], vec![vec!["x".repeat(5000)]]));
    assert_eq!(d.columns[1].width, px(3000.0));
}

#[test]
fn the_operand_is_prepared_once_per_pass_not_once_per_cell() {
    let cells: Vec<ResultCell> = (0..1000)
        .map(|ix| ResultCell::text(format!("row {ix}")))
        .collect();
    let filter = |op: FilterOp, value: &str| ColumnFilter {
        column: 0,
        op,
        value: value.to_string(),
    };

    reset_operand_preparations();
    let loose = PreparedFilter::prepare(&filter(FilterOp::GreaterThan, "2026-01-01"));
    assert_eq!(
        operand_preparations(),
        2,
        "one parse judgement + one fold, once per pass"
    );
    for cell in &cells {
        assert!(!loose.matches(Some(cell)));
    }
    assert_eq!(
        operand_preparations(),
        2,
        "1,000 cells must not add operand work"
    );

    reset_operand_preparations();
    let strict = PreparedFilter::prepare(&filter(FilterOp::LessThan, "12"));
    assert_eq!(
        operand_preparations(),
        1,
        "the fold is only paid when the parse fails"
    );
    for cell in &cells {
        assert!(!strict.matches(Some(cell)));
    }
    assert_eq!(operand_preparations(), 1);

    let rows: Vec<Vec<String>> = (0..500).map(|ix| vec![format!("row {ix}")]).collect();
    let mut d = delegate();
    d.update(result(&["v"], rows));
    reset_operand_preparations();
    d.set_column_filters(&[filter(FilterOp::GreaterThan, "2026-01-01")]);
    assert_eq!(
        operand_preparations(),
        2,
        "a 500-row page is one pass, not 500"
    );
    d.set_column_filters(&[]);
    assert_eq!(operand_preparations(), 2, "clearing prepares nothing");
}

/// Ingesting a result shares the session's live grid handle: one `Arc<Grid>` is
/// read by the delegate, so a landing costs a refcount bump, not a copy of the
/// up-to-10 000 rows. The grid handle (not a per-row slice pointer) is the
/// shared identity now.
#[test]
fn ingesting_a_result_shares_the_handle_instead_of_copying_the_grid() {
    let rows: Vec<Vec<String>> = (0..500)
        .map(|r| vec![format!("r{r}c0"), format!("r{r}c1")])
        .collect();
    let arc = Arc::new(SqlResult::Query(grid(&["a", "b"], rows)));
    let mut d = delegate();
    d.update(Some(Arc::clone(&arc)));

    assert!(Arc::ptr_eq(d.source.as_ref().unwrap(), &arc));
    assert_eq!(Arc::strong_count(&arc), 2, "one handle, two owners");
    // The delegate and the session read the very same live grid handle.
    let SqlResult::Query(q) = arc.as_ref() else {
        unreachable!("the fixture is a Query");
    };
    assert!(
        Arc::ptr_eq(delegate_grid(&d).as_ref().unwrap(), &q.grid),
        "the delegate must read the session's grid, not a copy"
    );
    assert_eq!(q.grid.len(), 500);
}

/// The transient semantics: a load-more append grows the shared grid in place,
/// and the delegate reads it live - no re-ingest needed. Before this design the
/// delegate held a frozen snapshot and the append was a copy-on-write deep copy
/// of the whole accumulated grid.
#[test]
fn a_load_more_append_is_visible_live_and_grows_the_order() {
    let rows: Vec<Vec<String>> = (0..50)
        .map(|r| vec![format!("r{r}c0"), format!("r{r}c1")])
        .collect();
    let session = Arc::new(SqlResult::Query(grid(&["a", "b"], rows)));
    let mut d = delegate();
    d.update(Some(Arc::clone(&session)));

    // In-place append into the shared grid (the new load-more path).
    if let SqlResult::Query(q) = session.as_ref() {
        q.append_rows(
            vec![vec![ResultCell::text("late"), ResultCell::text("later")]],
            AppendMeta {
                execution_time_ms: 0,
                truncated: false,
            },
        );
    }

    // Live read: the delegate sees the new row WITHOUT a re-ingest.
    assert_eq!(
        delegate_grid(&d).as_ref().unwrap().len(),
        51,
        "the live grid grew without re-ingest"
    );
    // Its derived display order is still the pre-append length until it grows.
    assert_eq!(
        d.filtered_order.len(),
        50,
        "the display order lags until grown"
    );
    // The incremental grow extends it to the live row count (Θ(new rows)).
    let grew = d.grow_to(51);
    assert!(grew, "grow_to must extend the derived order");
    assert_eq!(
        d.filtered_order.len(),
        51,
        "the display order now matches the grid"
    );
    // The appended row is addressable through the live grid + display order.
    assert_eq!(
        d.cell_named(50, "a").map(|c| c.value),
        Some("late".to_string()),
    );
}

/// `grow_to` is a no-op when the grid has not outgrown the delegate's order, so
/// the panel's observer can call it on every `AppState` write cheaply.
#[test]
fn grow_to_is_a_noop_when_the_grid_has_not_grown() {
    let rows: Vec<Vec<String>> = (0..10).map(|r| vec![format!("r{r}")]).collect();
    let mut d = delegate();
    d.update(result(&["v"], rows));
    assert!(!d.grow_to(10), "same length is not a grow");
    assert!(!d.grow_to(5), "a smaller live count is not a grow");
}

/// A filtered view extends only the matching new rows, not a full re-scan of
/// the whole grid.
#[test]
fn grow_to_extends_a_filtered_view_with_only_matching_new_rows() {
    // 10 rows, all matching "keep" except even-index rows which hold "drop".
    let rows: Vec<Vec<String>> = (0..10)
        .map(|r| {
            vec![if r % 2 == 0 {
                "drop".to_string()
            } else {
                "keep".to_string()
            }]
        })
        .collect();
    let mut d = delegate();
    d.update(result(&["v"], rows));
    d.set_filter("keep");
    // 5 rows match ("keep" at odd indices 1,3,5,7,9).
    assert_eq!(d.filtered_order.len(), 5);

    // The session grid grew by two new rows, both matching.
    let session = d.source.as_ref().unwrap().clone();
    if let SqlResult::Query(q) = session.as_ref() {
        q.append_rows(
            vec![
                vec![ResultCell::text("keep")],
                vec![ResultCell::text("drop")],
            ],
            AppendMeta {
                execution_time_ms: 0,
                truncated: false,
            },
        );
    }
    d.grow_to(12);
    // Only the new "keep" row is added: 5 -> 6.
    assert_eq!(
        d.filtered_order.len(),
        6,
        "a filtered grow admits only matching rows"
    );
}

#[test]
fn a_column_drag_permutes_instead_of_rewriting_rows() {
    let rows: Vec<Vec<String>> = (0..5)
        .map(|r| vec![format!("a{r}"), format!("b{r}"), format!("c{r}")])
        .collect();
    let mut d = delegate();
    d.update(result(&["a", "b", "c"], rows));

    d.reorder_column(1, 2);
    assert_eq!(
        d.cell_named(3, "a").map(|c| c.value.clone()),
        Some("c3".to_string())
    );
    assert_eq!(
        d.cell_named(3, "b").map(|c| c.value.clone()),
        Some("a3".to_string())
    );
    assert_eq!(
        d.cell_named(3, "c").map(|c| c.value.clone()),
        Some("b3".to_string())
    );

    d.sort_col = Some(2);
    d.sort_asc = true;
    d.apply_sort();
    let g = d.guard().unwrap();
    let by_b: Vec<String> = d
        .filtered_order
        .iter()
        .map(|&ix| g.rows[ix][1].value.clone())
        .collect();
    assert_eq!(by_b, vec!["b0", "b1", "b2", "b3", "b4"]);
}

#[test]
#[ignore = "bench: grid-copy timings"]
fn bench_grid_copy_paths() {
    use std::time::Instant;
    const ROWS: usize = 10_000;
    const COLS: usize = 15;
    let q = QueryResult::new(
        (0..COLS)
            .map(|ix| ColumnInfo::new(format!("c{ix}"), "text", ix as u32))
            .collect(),
        (0..ROWS)
            .map(|r| {
                (0..COLS)
                    .map(|c| ResultCell::text(format!("v{r}-{c}")))
                    .collect()
            })
            .collect(),
        ROWS,
        ROWS,
        0,
        String::new(),
        false,
    );
    let sqlr = Arc::new(SqlResult::Query(q.clone()));
    let mut t_shallow_clone = Vec::new();
    let mut t_rows_snapshot = Vec::new();
    let mut t_update = Vec::new();
    for _ in 0..7 {
        let t = Instant::now();
        let a = SqlResult::clone(&sqlr); // now a shallow Arc<Grid> bump
        t_shallow_clone.push(t.elapsed());
        std::hint::black_box(&a);
        let t = Instant::now();
        let c = q.rows_snapshot();
        t_rows_snapshot.push(t.elapsed());
        std::hint::black_box(&c);
        let mut d = delegate();
        let t = Instant::now();
        d.update(Some(Arc::clone(&sqlr)));
        t_update.push(t.elapsed());
    }
    let med = |mut v: Vec<std::time::Duration>| {
        v.sort();
        v[v.len() / 2]
    };
    println!(
        "bench: shallow_clone={:?} rows_snapshot={:?} update={:?}",
        med(t_shallow_clone),
        med(t_rows_snapshot),
        med(t_update),
    );
}
