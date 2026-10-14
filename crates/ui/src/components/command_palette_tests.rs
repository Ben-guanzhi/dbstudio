use super::filter_indices;
use super::{PaletteAction, PaletteItem};
use gpui_component::IconName;

fn row(label: &str, detail: &str) -> PaletteItem {
    PaletteItem {
        label: label.to_string(),
        detail: (!detail.is_empty()).then(|| detail.to_string()),
        icon: IconName::SquareTerminal,
        action: PaletteAction::RunCommand(label.to_string()),
    }
}

fn corpus() -> Vec<PaletteItem> {
    vec![
        row("users", "Table · public"),
        row("orders", "Table · public"),
        row("Execute Query", "Execute the current SQL query"),
        row("Clear History", "Clear query history"),
        row("audit_log", ""),
    ]
}

#[test]
fn an_empty_query_shows_every_row_in_build_order() {
    let items = corpus();
    assert_eq!(filter_indices(&items, ""), vec![0, 1, 2, 3, 4]);
}

#[test]
fn a_longer_query_never_adds_rows_back() {
    let items = corpus();
    let mut prefix = String::new();
    let mut previous = items.len();
    for ch in "quer".chars() {
        prefix.push(ch);
        let hits = filter_indices(&items, &prefix).len();
        assert!(
            hits <= previous,
            "{prefix:?} matched {hits}, {previous} before"
        );
        previous = hits;
    }
    assert_eq!(filter_indices(&items, "quer"), vec![2, 3]);
}

#[test]
fn matching_ignores_case_and_reads_the_detail_too() {
    let items = corpus();
    assert_eq!(filter_indices(&items, "EXECUTE"), vec![2]);
    assert_eq!(filter_indices(&items, "public"), vec![0, 1]);
    assert_eq!(filter_indices(&items, "TABLE · PUB"), vec![0, 1]);
}

#[test]
fn a_query_that_matches_nothing_shows_nothing() {
    let items = corpus();
    assert!(filter_indices(&items, "zzz").is_empty());
}
