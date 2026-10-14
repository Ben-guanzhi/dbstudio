use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use dbstudio_core::result::{QueryResult, ResultCell, SqlResult};
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
    QueryResult {
        columns: columns
            .iter()
            .enumerate()
            .map(|(ix, name)| ColumnInfo::new(*name, "text", ix as u32))
            .collect(),
        rows: rows
            .into_iter()
            .map(|row| row.into_iter().map(ResultCell::text).collect())
            .collect(),
        row_count: 0,
        total_row_count: 0,
        execution_time_ms: 0,
        sql: String::new(),
        truncated: false,
    }
}

fn result(columns: &[&str], rows: Vec<Vec<String>>) -> Option<Arc<SqlResult>> {
    Some(Arc::new(SqlResult::Query(grid(columns, rows))))
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
    let SqlResult::Query(q) = arc.as_ref() else {
        unreachable!("the fixture is a Query");
    };
    assert!(std::ptr::eq(d.rows().as_ptr(), q.rows.as_ptr()));
    assert_eq!(d.rows().len(), 500);
}

#[test]
fn the_shared_handle_still_reads_as_a_snapshot() {
    let rows: Vec<Vec<String>> = (0..50)
        .map(|r| vec![format!("r{r}c0"), format!("r{r}c1")])
        .collect();
    let mut session = Arc::new(SqlResult::Query(grid(&["a", "b"], rows)));
    let mut d = delegate();
    d.update(Some(Arc::clone(&session)));

    if let SqlResult::Query(q) = Arc::make_mut(&mut session) {
        q.rows
            .push(vec![ResultCell::text("late"), ResultCell::text("later")]);
    }
    assert_eq!(d.rows().len(), 50, "the snapshot was not disturbed");

    d.update(Some(Arc::clone(&session)));
    assert_eq!(d.rows().len(), 51, "re-ingest sees the appended row");
    assert_eq!(d.rows()[50][0].value, "late");
}

#[test]
fn a_column_drag_permutes_instead_of_rewriting_rows() {
    let rows: Vec<Vec<String>> = (0..5)
        .map(|r| vec![format!("a{r}"), format!("b{r}"), format!("c{r}")])
        .collect();
    let mut d = delegate();
    d.update(result(&["a", "b", "c"], rows));

    d.reorder_column(1, 2);
    assert_eq!(d.cell_named(3, "a").map(|c| c.value.as_str()), Some("c3"));
    assert_eq!(d.cell_named(3, "b").map(|c| c.value.as_str()), Some("a3"));
    assert_eq!(d.cell_named(3, "c").map(|c| c.value.as_str()), Some("b3"));

    d.sort_col = Some(2);
    d.sort_asc = true;
    d.apply_sort();
    let by_b: Vec<String> = d
        .filtered_order
        .iter()
        .map(|&ix| d.rows()[ix][1].value.clone())
        .collect();
    assert_eq!(by_b, vec!["b0", "b1", "b2", "b3", "b4"]);
}

#[test]
#[ignore = "bench: grid-copy timings"]
fn bench_grid_copy_paths() {
    use std::time::Instant;
    const ROWS: usize = 10_000;
    const COLS: usize = 15;
    let q = QueryResult {
        columns: (0..COLS)
            .map(|ix| ColumnInfo::new(format!("c{ix}"), "text", ix as u32))
            .collect(),
        rows: (0..ROWS)
            .map(|r| {
                (0..COLS)
                    .map(|c| ResultCell::text(format!("v{r}-{c}")))
                    .collect()
            })
            .collect(),
        row_count: ROWS,
        total_row_count: ROWS,
        execution_time_ms: 0,
        sql: String::new(),
        truncated: false,
    };
    let sqlr = Arc::new(SqlResult::Query(q.clone()));
    let mut t_result_clone = Vec::new();
    let mut t_rows_clone = Vec::new();
    let mut t_update = Vec::new();
    for _ in 0..7 {
        let t = Instant::now();
        let a = SqlResult::clone(&sqlr);
        t_result_clone.push(t.elapsed());
        std::hint::black_box(&a);
        let t = Instant::now();
        let c = q.rows.clone();
        t_rows_clone.push(t.elapsed());
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
    let m_result_clone = med(t_result_clone);
    let m_rows_clone = med(t_rows_clone);
    let old = m_result_clone + m_rows_clone * 2;
    let new = med(t_update);
    println!(
        "bench: result.clone={:?} rows.clone={:?} old-per-run(sum)={:?} new.update={:?}",
        m_result_clone, m_rows_clone, old, new,
    );
}
