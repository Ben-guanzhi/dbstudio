//! Lexical classification of whether a SQL batch mutates anything.
//!
//! Every entry point that decides "may this SQL run?" needs the same answer:
//! the UI's safe-mode gate and the MCP `execute` tool. Keeping one
//! implementation here (instead of one copy per caller) is what makes a
//! loosening in one place impossible — the two call sites cannot drift apart on
//! the keyword sets.
//!
//! Classification runs on a comment-and-literal-stripped copy, so `-- comment`
//! never hides a `DROP`, a `'DROP'` literal is never read as a statement, and
//! a column named `deleted` never looks like one. A batch is not split on `;`
//! — write keywords are scanned across the whole text — which is what catches
//! `SELECT 1; DROP TABLE x` and `WITH d AS (DELETE …) SELECT …`.
//!
//! This is deliberately the *lexical* half of `dbstudio-ui`'s gate only. The UI
//! also parses with tree-sitter so it can tell a scoped `UPDATE` from an
//! unscoped one for its confirmation wording; that extra precision is a UI
//! concern, and the UI's lexical fallback shares the stripping and tokenising
//! helpers here rather than re-implementing them.

use std::collections::HashSet;

/// Whether a SQL batch reads, writes, or cannot be classified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MutationKind {
    /// Purely read-only: `SELECT` / `SHOW` / `DESCRIBE` / `EXPLAIN` /
    /// `PRAGMA` / read-only `WITH`.
    Read,
    /// Mutates rows: `INSERT` / `UPDATE` / `DELETE` / `MERGE` / `REPLACE` /
    /// `TRUNCATE`.
    Data,
    /// Mutates structure or privileges: `DROP` / `CREATE` / `ALTER` /
    /// `GRANT` / `REVOKE` / `COMMENT` / `VACUUM` / `REINDEX` / `ANALYZE` /
    /// `CALL` / `EXEC` / `EXECUTE` / `RENAME`.
    Schema,
    /// Undecidable: the leading keyword is not recognised and no write
    /// keyword appears anywhere (`USE database`, `SET SESSION …`, a dialect
    /// keyword, a parenthesised statement). Callers decide how to treat it.
    Unknown,
}

impl MutationKind {
    /// True for anything that changes rows or schema. `Unknown` is *not*
    /// included, matching the UI gate's lenient semantics. Callers facing an
    /// untrusted author (the MCP server, whose caller is an LLM) can tighten
    /// this by also treating `Unknown` as a mutation — blocking one
    /// unnecessary `USE` costs far less than letting a destructive statement
    /// through.
    pub fn is_mutation(self) -> bool {
        matches!(self, Self::Data | Self::Schema)
    }
}

/// Rows: statements that add, change or remove data.
pub const DATA_KEYWORDS: &[&str] = &["INSERT", "UPDATE", "DELETE", "MERGE", "REPLACE", "TRUNCATE"];

/// Schema and privileges: everything that changes the database's shape.
pub const SCHEMA_KEYWORDS: &[&str] = &[
    "DROP", "CREATE", "ALTER", "GRANT", "REVOKE", "COMMENT", "VACUUM", "REINDEX", "ANALYZE",
    "CALL", "EXEC", "EXECUTE", "RENAME",
];

/// Leading keywords that mark a read-only statement.
pub const READ_KEYWORDS: &[&str] = &[
    "SELECT", "SHOW", "DESCRIBE", "DESC", "EXPLAIN", "PRAGMA", "WITH", "VALUES",
];

/// Classifies `sql`. `None` for blank or comment-only input.
pub fn classify_sql_mutations(sql: &str) -> Option<MutationKind> {
    let stripped = strip_sql_comments_and_literals(sql);
    let tokens = tokenize_sql(&stripped);
    if tokens.is_empty() {
        return None;
    }
    if READ_KEYWORDS.contains(&tokens[0].as_str()) {
        // A read-led batch can still mutate: `SELECT 1; DROP TABLE x`, or a
        // data-modifying CTE where the DELETE precedes the outer SELECT.
        return match find_mutation(&tokens) {
            Some(kind) => Some(kind),
            None => Some(MutationKind::Read),
        };
    }
    // Unrecognised leading keyword. Scan the whole text so a write cannot hide
    // behind a first keyword we don't know; if nothing is found, say so rather
    // than claiming read-only.
    find_mutation(&tokens).or(Some(MutationKind::Unknown))
}

/// First write keyword in `tokens`, if any. `SELECT … FOR UPDATE` is a lock
/// hint, not a write, so an `UPDATE` directly after `FOR` is skipped.
fn find_mutation(tokens: &[String]) -> Option<MutationKind> {
    for (i, token) in tokens.iter().enumerate() {
        if token == "UPDATE" && i > 0 && tokens[i - 1] == "FOR" {
            continue;
        }
        if DATA_KEYWORDS.contains(&token.as_str()) {
            return Some(MutationKind::Data);
        }
        if SCHEMA_KEYWORDS.contains(&token.as_str()) {
            return Some(MutationKind::Schema);
        }
    }
    None
}

/// Upper-cased word tokens: maximal runs of `[A-Za-z0-9_]`, so `t1.x` yields
/// two tokens and `123` is kept for dialect numeric identifiers.
pub fn tokenize_sql(sql: &str) -> Vec<String> {
    sql.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .map(|w| w.to_uppercase())
        .collect()
}

/// Returns the set of write keywords present in `sql` after stripping. Useful
/// when a caller wants to report *why* a statement was blocked rather than
/// just that it was.
pub fn mutation_keywords_in(sql: &str) -> HashSet<&'static str> {
    let tokens = tokenize_sql(&strip_sql_comments_and_literals(sql));
    let mut found = HashSet::new();
    for (i, token) in tokens.iter().enumerate() {
        let t = token.as_str();
        if t == "UPDATE" && i > 0 && tokens[i - 1] == "FOR" {
            continue;
        }
        if let Some(k) = DATA_KEYWORDS
            .iter()
            .chain(SCHEMA_KEYWORDS)
            .find(|k| **k == t)
        {
            found.insert(*k);
        }
    }
    found
}

/// Replaces `--`/`/* */` comments and `'…'`/`"…"`/`` `…` ``/`[…]` literals with
/// spaces so a keyword scan sees only the statement's structure.
pub fn strip_sql_comments_and_literals(sql: &str) -> String {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = String::with_capacity(sql.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' | '"' | '`' | '[' => {
                i = skip_quoted(&chars, i, c);
                out.push(' ');
            }
            '-' if chars.get(i + 1) == Some(&'-') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                out.push(' ');
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                let rest: String = chars[i + 2..].iter().collect();
                match rest.find("*/") {
                    Some(end) => i += 2 + end + 2,
                    None => i = chars.len(),
                }
                out.push(' ');
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// Skips the literal starting at `chars[start]`, handling doubled quotes,
/// MySQL backslash escapes and `]`-terminated bracket identifiers.
fn skip_quoted(chars: &[char], start: usize, quote: char) -> usize {
    let mut i = start + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' if quote == '\'' => i += 2,
            c if c == quote && quote == '[' => return i + 1,
            c if c == quote => {
                if chars.get(i + 1) == Some(&quote) {
                    i += 2;
                } else {
                    return i + 1;
                }
            }
            _ => i += 1,
        }
    }
    chars.len()
}

#[cfg(test)]
#[path = "sql_mutations_tests.rs"]
mod tests;
