use dbstudio_core::models::Environment;

/// A write operation pending review before application.
#[derive(Debug, Clone)]
pub struct PendingWrite {
    pub sql: String,
    pub kind: WriteKind,
    pub label: String,
    /// Best-effort SQL that reverses this edit. Empty when undo cannot be
    /// derived for the edit (e.g. an insert without a primary key value).
    pub inverse_sql: String,
    /// Structured before/after view for the review panel (when derivable).
    pub diff: Option<PendingDiff>,
}

/// Human-readable table diff describing one pending write.
#[derive(Debug, Clone)]
pub struct PendingDiff {
    pub table: String,
    pub op: DiffOp,
    /// Short human description of the affected row(s), e.g. `id = 3`.
    pub row_key: String,
    pub cells: Vec<CellDiff>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffOp {
    Insert,
    Update,
    Delete,
}

impl DiffOp {
    pub fn label(&self) -> &'static str {
        match self {
            DiffOp::Insert => "INSERT",
            DiffOp::Update => "UPDATE",
            DiffOp::Delete => "DELETE",
        }
    }
}

/// One column's before -> after change. `None` means SQL NULL.
#[derive(Debug, Clone)]
pub struct CellDiff {
    pub column: String,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
}

/// A record of an applied edit for undo/redo support.
#[derive(Debug, Clone)]
pub struct EditRecord {
    /// The SQL that was executed (forward direction).
    pub forward_sql: String,
    /// The SQL that reverses this edit (inverse direction).
    pub inverse_sql: String,
    /// Human-readable label for this edit.
    pub label: String,
}

/// What kind of write operation a SQL statement represents.
#[derive(Debug, Clone, PartialEq)]
pub enum WriteKind {
    Select,
    Insert,
    Update,
    UpdateNoWhere,
    Delete,
    DeleteNoWhere,
    Drop,
    Truncate,
    Alter,
    Create,
    OtherDdl,
}

impl WriteKind {
    pub fn is_read_only(&self) -> bool {
        matches!(self, WriteKind::Select)
    }

    pub fn is_destructive(&self) -> bool {
        matches!(
            self,
            WriteKind::Drop
                | WriteKind::Truncate
                | WriteKind::DeleteNoWhere
                | WriteKind::UpdateNoWhere
        )
    }

    pub fn label(&self) -> &'static str {
        match self {
            WriteKind::Select => "SELECT",
            WriteKind::Insert => "INSERT",
            WriteKind::Update => "UPDATE",
            WriteKind::UpdateNoWhere => "UPDATE (no WHERE)",
            WriteKind::Delete => "DELETE",
            WriteKind::DeleteNoWhere => "DELETE (no WHERE)",
            WriteKind::Drop => "DROP",
            WriteKind::Truncate => "TRUNCATE",
            WriteKind::Alter => "ALTER",
            WriteKind::Create => "CREATE",
            WriteKind::OtherDdl => "DDL",
        }
    }
}

/// Classifies the first SQL statement in `sql`. Returns `None` for empty
/// input or for statements that are purely read-only when the environment
/// is forgiving.
///
/// Classification parses the text with the tree-sitter SQL grammar and maps
/// the statement's syntax-tree nodes to a write kind, so comments, string
/// literals and identifiers cannot influence the outcome the way they could
/// influence a keyword scan: a leading `-- comment` never hides a `DROP`, a
/// `/* WHERE */` comment never scopes an unscoped `DELETE`, a column named
/// `deleted` never looks like one, and data-modifying CTEs or multi-statement
/// batches (`SELECT 1; DROP TABLE x`) are found in document order. Statements
/// the Postgres-flavoured grammar cannot parse (USE, PRAGMA, VACUUM, GRANT,
/// CALL, ...) — and input with no statement at all — fall back to the lexical
/// heuristic, which keeps their historical classification.
pub fn classify_sql(sql: &str) -> Option<WriteKind> {
    if sql.trim().is_empty() {
        return None;
    }
    match classify_sql_ast(sql) {
        Some(kind) => Some(kind),
        None => classify_sql_lexical(sql),
    }
}

/// AST-driven classification. `None` means "cannot decide from the syntax
/// tree" — the text does not parse cleanly or contains no statement — and the
/// caller falls back to the lexical heuristic.
fn classify_sql_ast(sql: &str) -> Option<WriteKind> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_sequel::LANGUAGE.into())
        .ok()?;
    let tree = parser.parse(sql, None)?;
    let root = tree.root_node();
    if root.has_error() {
        return None;
    }

    let mut saw_select = false;
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        // Comments, marginalia and `;` tokens sit between the statements.
        if !child.is_named() || child.kind() != "statement" {
            continue;
        }
        match classify_statement(child) {
            // A write anywhere in the batch decides the batch's kind.
            Some(kind) if !kind.is_read_only() => return Some(kind),
            Some(_) => saw_select = true,
            // Neutral statements (SET, transactions) say nothing about the
            // batch; keep looking.
            None => {}
        }
    }
    if saw_select {
        return Some(WriteKind::Select);
    }
    None
}

/// Pre-order scan of one statement for its first operation node. Pre-order is
/// document order, so a data-modifying CTE (`WITH d AS (DELETE …) SELECT …`)
/// wins over the outer SELECT that follows it, and wrapper nodes (an `update`
/// node over its `keyword_update` token) shadow keyword tokens nested inside
/// them (`ON DELETE CASCADE` under `create_table`).
fn classify_statement(statement: tree_sitter::Node) -> Option<WriteKind> {
    classify_node(statement, false)
}

fn classify_node(node: tree_sitter::Node, in_cte: bool) -> Option<WriteKind> {
    match node.kind() {
        "select" => Some(WriteKind::Select),
        "insert" => Some(WriteKind::Insert),
        "update" => {
            // WHERE attribution inside a CTE is ambiguous, and so is a WHERE
            // that only belongs to a subquery — both stay unscoped (the
            // destructive variant, which always asks for confirmation).
            let scoped = !in_cte && has_direct_child(node, "where");
            Some(if scoped {
                WriteKind::Update
            } else {
                WriteKind::UpdateNoWhere
            })
        }
        "delete" => Some(delete_kind(node, in_cte)),
        "cte" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(kind) = classify_node(child, true) {
                    return Some(kind);
                }
            }
            None
        }
        // SET/transactions are neither read nor write; do not descend.
        "set_statement" | "reset_statement" | "transaction" => None,
        other => {
            if let Some(kind) = keyword_write_kind(other) {
                return Some(kind);
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(kind) = classify_node(child, in_cte) {
                    return Some(kind);
                }
            }
            None
        }
    }
}

/// Grammar keyword tokens that surface as named nodes (e.g. `TRUNCATE TABLE`
/// has no wrapper node — its statement starts with a `keyword_truncate`).
/// The table mirrors the lexical heuristic's WRITE_KEYWORDS.
fn keyword_write_kind(node_kind: &str) -> Option<WriteKind> {
    let keyword = node_kind.strip_prefix("keyword_")?;
    let upper = keyword.to_uppercase();
    let kind = match upper.as_str() {
        "INSERT" => WriteKind::Insert,
        "UPDATE" => WriteKind::UpdateNoWhere,
        "DELETE" => WriteKind::DeleteNoWhere,
        "TRUNCATE" => WriteKind::Truncate,
        "DROP" => WriteKind::Drop,
        "CREATE" => WriteKind::Create,
        "ALTER" => WriteKind::Alter,
        "MERGE" | "REPLACE" | "GRANT" | "REVOKE" | "COMMENT" | "VACUUM" | "REINDEX" | "ANALYZE"
        | "CALL" | "EXEC" | "EXECUTE" | "RENAME" => WriteKind::OtherDdl,
        _ => return None,
    };
    Some(kind)
}

/// A DELETE is scoped when its statement's `from` clause carries a WHERE —
/// the grammar places it as a child of the `from` sibling, not of the
/// `delete` node itself.
fn delete_kind(delete: tree_sitter::Node, in_cte: bool) -> WriteKind {
    if in_cte {
        return WriteKind::DeleteNoWhere;
    }
    let scoped = delete
        .parent()
        .map(|statement| {
            let mut found = false;
            let mut cursor = statement.walk();
            for child in statement.children(&mut cursor) {
                if child.kind() == "from" && has_direct_child(child, "where") {
                    found = true;
                    break;
                }
            }
            found
        })
        .unwrap_or(false);
    if scoped {
        WriteKind::Delete
    } else {
        WriteKind::DeleteNoWhere
    }
}

fn has_direct_child(node: tree_sitter::Node, kind: &str) -> bool {
    let mut found = false;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            found = true;
            break;
        }
    }
    found
}

/// Lexical fallback for statements the grammar cannot parse. Classification
/// runs on a comment/string-stripped copy of the statement; read-keyword
/// statements (SELECT/WITH/...) are additionally scanned for write keywords
/// anywhere in the text.
fn classify_sql_lexical(sql: &str) -> Option<WriteKind> {
    let stripped = dbstudio_core::sql_mutations::strip_sql_comments_and_literals(sql);
    let trimmed = stripped.trim();
    if trimmed.is_empty() {
        return None;
    }
    let tokens = dbstudio_core::sql_mutations::tokenize_sql(trimmed);
    let first_keyword = tokens.first().map(String::as_str).unwrap_or("");

    match first_keyword {
        "SELECT" | "SHOW" | "DESCRIBE" | "DESC" | "EXPLAIN" | "PRAGMA" | "WITH" => {
            match scan_write_keywords(&tokens) {
                Some(keyword) => Some(write_kind_for(keyword)),
                None => Some(WriteKind::Select),
            }
        }
        "INSERT" => Some(WriteKind::Insert),
        "UPDATE" => Some(if has_where(&tokens) {
            WriteKind::Update
        } else {
            WriteKind::UpdateNoWhere
        }),
        "DELETE" => Some(if has_where(&tokens) {
            WriteKind::Delete
        } else {
            WriteKind::DeleteNoWhere
        }),
        "DROP" => Some(WriteKind::Drop),
        "TRUNCATE" => Some(WriteKind::Truncate),
        "ALTER" => Some(WriteKind::Alter),
        "CREATE" => Some(WriteKind::Create),
        _ => {
            if dbstudio_core::sql_mutations::SCHEMA_KEYWORDS.contains(&first_keyword) {
                Some(WriteKind::OtherDdl)
            } else {
                // Unknown leading token (parenthesized statement, CALL, ...):
                // fall back to the whole-text write scan so a write can't hide
                // behind an unrecognized first keyword.
                scan_write_keywords(&tokens).map(write_kind_for)
            }
        }
    }
}

/// Write keywords that disqualify a statement from the read-only class when
/// they appear anywhere in the token stream. The sets live in `dbstudio-core`
/// so the UI gate and the MCP `execute` gate read from one source — a keyword
/// added on one side now reaches the other automatically.
fn scan_write_keywords(tokens: &[String]) -> Option<&str> {
    for (i, token) in tokens.iter().enumerate() {
        if token == "UPDATE" && i > 0 && tokens[i - 1] == "FOR" {
            continue;
        }
        if dbstudio_core::sql_mutations::DATA_KEYWORDS.contains(&token.as_str())
            || dbstudio_core::sql_mutations::SCHEMA_KEYWORDS.contains(&token.as_str())
        {
            return Some(token.as_str());
        }
    }
    None
}

/// Maps a write keyword found outside the leading position to a write kind.
/// WHERE attribution inside a WITH subtree or a later batch statement is
/// ambiguous, so UPDATE/DELETE land on the (destructive) NoWhere variants.
fn write_kind_for(keyword: &str) -> WriteKind {
    match keyword {
        "INSERT" => WriteKind::Insert,
        "UPDATE" => WriteKind::UpdateNoWhere,
        "DELETE" => WriteKind::DeleteNoWhere,
        "TRUNCATE" => WriteKind::Truncate,
        "DROP" => WriteKind::Drop,
        "CREATE" => WriteKind::Create,
        "ALTER" => WriteKind::Alter,
        _ => WriteKind::OtherDdl,
    }
}

fn has_where(tokens: &[String]) -> bool {
    tokens.iter().any(|t| t == "WHERE")
}

/// Returns true when the SQL should be confirmed before execution given the
/// connection's environment and the global safe-mode flag.
///
/// Safe mode confirms every write. When disabled, confirmation is relaxed to
/// environment rules: production connections always confirm destructive writes,
/// while Dev/Staging skip confirmation for scoped (WHERE-bound) writes.
pub fn requires_confirmation(kind: Option<WriteKind>, env: Environment, safe_mode: bool) -> bool {
    let Some(kind) = kind else {
        return false;
    };
    if kind.is_read_only() {
        return false;
    }
    if safe_mode {
        return true;
    }
    if env.is_production() {
        return true;
    }
    kind.is_destructive()
}
