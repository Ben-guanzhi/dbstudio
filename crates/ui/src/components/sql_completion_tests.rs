use super::{SqlCompletionProvider, SQL_KEYWORDS};
use lsp_types::{CompletionItem, CompletionItemKind};

fn schema_item(name: &str) -> CompletionItem {
    CompletionItem {
        label: name.to_string(),
        kind: Some(CompletionItemKind::CLASS),
        ..Default::default()
    }
}

fn labels(provider: &SqlCompletionProvider, kind: CompletionItemKind) -> Vec<String> {
    let guard = provider.completions.read().unwrap();
    guard
        .iter()
        .filter(|(item, _)| item.kind == Some(kind))
        .map(|(item, _)| item.label.clone())
        .collect()
}

#[test]
fn pushing_an_identical_list_is_a_no_op() {
    let provider = SqlCompletionProvider::default();
    provider.set_schema_completions(vec![schema_item("orders"), schema_item("users")]);
    provider.set_schema_completions(vec![schema_item("orders"), schema_item("users")]);

    assert_eq!(
        labels(&provider, CompletionItemKind::CLASS).join(","),
        "orders,users"
    );
    assert_eq!(
        provider.completions.read().unwrap().len(),
        SQL_KEYWORDS.len() + 2
    );
}

#[test]
fn a_new_list_replaces_the_old_one_and_keeps_the_index() {
    let provider = SqlCompletionProvider::default();
    provider.set_schema_completions(vec![schema_item("orders")]);
    provider.set_schema_completions(vec![schema_item("Orders")]);

    assert_eq!(
        labels(&provider, CompletionItemKind::CLASS).join(","),
        "Orders"
    );
    assert_eq!(
        labels(&provider, CompletionItemKind::KEYWORD).len(),
        SQL_KEYWORDS.len()
    );
    let guard = provider.completions.read().unwrap();
    let indexed = guard
        .iter()
        .find(|(item, _)| item.label == "Orders")
        .expect("schema item missing");
    assert_eq!(indexed.1, "orders");
}
