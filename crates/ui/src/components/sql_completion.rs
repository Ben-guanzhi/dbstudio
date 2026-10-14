use std::sync::{Arc, RwLock};

use anyhow::Result;
use gpui::{App, Task, Window};
use gpui_component::input::{CompletionProvider, Rope};
use lsp_types::{CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse};

/// Common SQL keywords offered by the completion menu.
const SQL_KEYWORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "INSERT",
    "INTO",
    "VALUES",
    "UPDATE",
    "SET",
    "DELETE",
    "CREATE",
    "TABLE",
    "DROP",
    "ALTER",
    "ADD",
    "COLUMN",
    "INDEX",
    "VIEW",
    "DISTINCT",
    "AS",
    "ON",
    "JOIN",
    "INNER",
    "LEFT",
    "RIGHT",
    "FULL",
    "OUTER",
    "CROSS",
    "GROUP",
    "BY",
    "ORDER",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "ASC",
    "DESC",
    "AND",
    "OR",
    "NOT",
    "NULL",
    "IS",
    "IN",
    "EXISTS",
    "BETWEEN",
    "LIKE",
    "ILIKE",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "UNION",
    "ALL",
    "EXPLAIN",
    "ANALYZE",
    "BEGIN",
    "COMMIT",
    "ROLLBACK",
    "CAST",
    "COALESCE",
    "COUNT",
    "SUM",
    "AVG",
    "MIN",
    "MAX",
    "NOW",
    "CURRENT_DATE",
    "CURRENT_TIMESTAMP",
    "PRIMARY",
    "KEY",
    "FOREIGN",
    "REFERENCES",
    "UNIQUE",
    "CONSTRAINT",
    "DEFAULT",
    "TRUNCATE",
    "GRANT",
    "REVOKE",
    "USE",
    "SHOW",
    "DATABASE",
    "DATABASES",
    "TABLES",
    "SCHEMA",
    "SCHEMAS",
    "RETURNING",
    "INTERSECT",
    "EXCEPT",
    "WITH",
    "RECURSIVE",
    "OVER",
    "PARTITION",
    "FILTER",
    "WINDOW",
    "ROW",
    "ROWS",
    "PRECEDING",
    "FOLLOWING",
    "CASCADE",
    "RESTRICT",
    "IF",
];

/// Provides SQL keyword and schema-derived completions to the editor.
pub struct SqlCompletionProvider {
    /// Each item carries its lowercased label. The prefix filter runs on
    /// every keystroke and used to lowercase the whole list - keywords and
    /// schema items alike - each time.
    completions: Arc<RwLock<Vec<(CompletionItem, String)>>>,
}

impl SqlCompletionProvider {
    pub fn new() -> Self {
        let completions = SQL_KEYWORDS
            .iter()
            .map(|keyword| {
                indexed(CompletionItem {
                    label: (*keyword).to_string(),
                    kind: Some(CompletionItemKind::KEYWORD),
                    detail: Some("keyword".to_string()),
                    ..Default::default()
                })
            })
            .collect();

        Self {
            completions: Arc::new(RwLock::new(completions)),
        }
    }

    /// Replaces the schema-derived completion items (table names, etc.).
    pub fn set_schema_completions(&self, items: Vec<CompletionItem>) {
        let mut guard = self.completions.write().unwrap_or_else(|e| e.into_inner());
        guard.retain(|(item, _)| item.kind != Some(CompletionItemKind::CLASS));
        guard.extend(items.into_iter().map(indexed));
    }

    /// Check if the cursor is in the middle of a word (not at word start).
    fn is_mid_word(text: &Rope, offset: usize, trigger_len: usize) -> bool {
        if offset <= trigger_len {
            return false;
        }
        let prev_char_offset = offset - trigger_len - 1;
        let prev_char = text
            .slice(prev_char_offset..prev_char_offset + 1)
            .to_string();
        prev_char
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    }

    /// Filter completions by prefix, case-insensitive.
    fn filter_by_prefix(
        items: &[(CompletionItem, String)],
        prefix: &str,
        max_results: usize,
    ) -> Vec<CompletionItem> {
        let query = prefix.to_lowercase();
        items
            .iter()
            .filter(|(_, label_lower)| label_lower.starts_with(&query))
            .take(max_results)
            .map(|(item, _)| item.clone())
            .collect()
    }
}

fn indexed(item: CompletionItem) -> (CompletionItem, String) {
    let label_lower = item.label.to_lowercase();
    (item, label_lower)
}

impl Default for SqlCompletionProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl CompletionProvider for SqlCompletionProvider {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        trigger: CompletionContext,
        _: &mut Window,
        _: &mut App,
    ) -> Task<Result<CompletionResponse>> {
        let Some(trigger_character) = trigger.trigger_character else {
            return Task::ready(Ok(CompletionResponse::Array(vec![])));
        };
        if trigger_character.is_empty() || Self::is_mid_word(text, offset, trigger_character.len())
        {
            return Task::ready(Ok(CompletionResponse::Array(vec![])));
        }

        let items = self.completions.read().unwrap_or_else(|e| e.into_inner());
        let matches = Self::filter_by_prefix(&items, &trigger_character, 10);
        Task::ready(Ok(CompletionResponse::Array(matches)))
    }

    fn is_completion_trigger(&self, _offset: usize, new_text: &str, _: &mut App) -> bool {
        new_text
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
    }
}

#[cfg(test)]
// Inline tests extracted to sql_completion_tests.rs.
#[cfg(test)]
#[path = "sql_completion_tests.rs"]
mod sql_completion_tests;
