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
            WriteKind::Drop | WriteKind::Truncate | WriteKind::DeleteNoWhere | WriteKind::UpdateNoWhere
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
pub fn classify_sql(sql: &str) -> Option<WriteKind> {
    let trimmed = sql.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    let first_keyword = trimmed
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_uppercase();

    match first_keyword.as_str() {
        "SELECT" | "SHOW" | "DESCRIBE" | "DESC" | "EXPLAIN" | "PRAGMA" | "WITH" => {
            Some(WriteKind::Select)
        }
        "INSERT" => Some(WriteKind::Insert),
        "UPDATE" => {
            has_where(trimmed).map(|has| if has { WriteKind::Update } else { WriteKind::UpdateNoWhere })
        }
        "DELETE" => {
            has_where(trimmed).map(|has| if has { WriteKind::Delete } else { WriteKind::DeleteNoWhere })
        }
        "DROP" => Some(WriteKind::Drop),
        "TRUNCATE" => Some(WriteKind::Truncate),
        "ALTER" => Some(WriteKind::Alter),
        "CREATE" => Some(WriteKind::Create),
        _ => {
            if is_ddl_keyword(&first_keyword) {
                Some(WriteKind::OtherDdl)
            } else {
                None
            }
        }
    }
}

fn has_where(sql: &str) -> Option<bool> {
    let upper = sql.to_uppercase();
    let after_where = upper.find(" WHERE ");
    Some(after_where.is_some())
}

fn is_ddl_keyword(word: &str) -> bool {
    matches!(
        word,
        "RENAME" | "GRANT" | "REVOKE" | "COMMENT" | "VACUUM" | "REINDEX" | "ANALYZE"
    )
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