use anyhow::Result;
use dbstudio_core::models::DatabaseType;
use dbstudio_core::result::SqlResult;
use dbstudio_core::schema::{TableInfo, TableType};
use std::io::Write;
use std::path::Path;

use crate::{get_create_table_sql, list_tables, Connection};

/// Progress callback for export operations.
pub type ExportProgress = Box<dyn Fn(ExportProgressInfo) + Send>;

/// Information about export progress.
#[derive(Debug, Clone)]
pub struct ExportProgressInfo {
    pub current_table: String,
    pub tables_completed: usize,
    pub tables_total: usize,
    pub rows_exported: usize,
}

/// Configuration for database export.
#[derive(Debug, Clone)]
pub struct ExportConfig {
    /// Include CREATE TABLE statements.
    pub include_schema: bool,
    /// Include INSERT statements for data.
    pub include_data: bool,
    /// Include DROP TABLE statements before CREATE.
    pub include_drop: bool,
    /// Tables to export (empty = all tables).
    pub tables: Vec<String>,
    /// Maximum rows per table (0 = unlimited).
    pub max_rows: usize,
}

impl Default for ExportConfig {
    fn default() -> Self {
        Self {
            include_schema: true,
            include_data: true,
            include_drop: true,
            tables: Vec::new(),
            max_rows: 0,
        }
    }
}

/// Quote an identifier with the quoting style for the given DB type.
fn quote_ident(db_type: &DatabaseType, name: &str) -> String {
    db_type.quote_identifier(name)
}

/// Build a fully-qualified table identifier, quoted per dialect.
fn qualified_name(db_type: &DatabaseType, info: &TableInfo) -> String {
    let table = quote_ident(db_type, &info.name);
    // Some drivers report the owning schema in `TableInfo::schema`; when present
    // and not already embedded in the table name, qualify the identifier.
    match info.schema.as_deref() {
        Some(s) if !s.is_empty() && !info.name.contains('.') => {
            format!("{}.{}", quote_ident(db_type, s), table)
        }
        _ => table,
    }
}

/// Build a per-dialect DROP statement for a table.
fn drop_statement(db_type: &DatabaseType, qualified: &str) -> String {
    format!("{};", db_type.drop_table_statement(qualified))
}

/// Build a per-dialect SELECT statement, applying `max_rows` when > 0.
fn select_statement(db_type: &DatabaseType, qualified: &str, max_rows: usize) -> String {
    db_type.preview_statement(qualified, max_rows)
}

/// Build a `TableInfo` shell for an explicitly requested table name (used when
/// `ExportConfig::tables` is set, so schema details are unknown up front).
fn table_shell(name: &str) -> TableInfo {
    TableInfo {
        name: name.to_string(),
        schema: None,
        database: None,
        table_type: TableType::Table,
        row_count: None,
        comment: None,
    }
}

/// Export a database to a SQL dump file.
///
/// Works across all supported dialects: table lists, CREATE TABLE statements,
/// identifier quoting and LIMIT clauses are produced per-dialect, so the dump
/// can be re-imported into the same engine.
pub async fn export_database(
    conn: &Connection,
    path: &Path,
    config: &ExportConfig,
    progress: Option<ExportProgress>,
) -> Result<ExportStats> {
    let db_type = conn.db_type();

    // Resolve the table list: either the user-provided subset or everything
    // discoverable in the active schema (drivers handle their own introspection).
    let tables: Vec<TableInfo> = if config.tables.is_empty() {
        list_tables(conn, "").await?
    } else {
        config.tables.iter().map(|name| table_shell(name)).collect()
    };

    let mut file = std::fs::File::create(path)?;
    let mut stats = ExportStats::default();

    // Write header
    writeln!(file, "-- dbstudio SQL dump")?;
    writeln!(file, "-- Database: {}", conn.current_database().await?)?;
    writeln!(
        file,
        "-- Generated: {}",
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    )?;
    writeln!(file)?;

    let tables_total = tables.len();

    for (tables_completed, table) in tables.iter().enumerate() {
        if let Some(ref progress_fn) = progress {
            progress_fn(ExportProgressInfo {
                current_table: table.name.clone(),
                tables_completed,
                tables_total,
                rows_exported: stats.rows_exported,
            });
        }

        let qualified = qualified_name(&db_type, table);

        // Schema: DROP (optional) + CREATE TABLE from driver introspection.
        if config.include_schema {
            let create_sql =
                get_create_table_sql(conn, &table.name, table.schema.as_deref()).await?;
            if !create_sql.trim().is_empty() {
                if config.include_drop {
                    writeln!(file, "{}", drop_statement(&db_type, &qualified))?;
                }
                writeln!(file, "{};", create_sql.trim_end_matches(';'))?;
                writeln!(file)?;
            }
        }

        // Data: skip views (rows cannot be inserted into a view).
        if config.include_data && table.table_type == TableType::Table {
            let select_sql = select_statement(&db_type, &qualified, config.max_rows);

            if let SqlResult::Query(q) = conn.execute_export(&select_sql).await? {
                // The driver caps SELECT results for the interactive grid. If it
                // did, the dump is missing rows even though the export
                // "succeeded" — record that so callers can warn instead of
                // reporting a complete export.
                stats.partial |= q.truncated;
                stats.rows_exported += q.rows.len();
                stats.tables_exported += 1;

                if !q.rows.is_empty() {
                    // Get column names
                    let columns: Vec<String> = q.columns.iter().map(|c| c.name.clone()).collect();
                    let quoted_columns: Vec<String> =
                        columns.iter().map(|c| quote_ident(&db_type, c)).collect();

                    // Generate INSERT statements
                    for row in &q.rows {
                        let values: Vec<String> = row
                            .iter()
                            .map(|cell| {
                                if cell.is_null {
                                    "NULL".to_string()
                                } else {
                                    crate::utils::quote_string_literal(&cell.value)
                                }
                            })
                            .collect();

                        writeln!(
                            file,
                            "INSERT INTO {} ({}) VALUES ({});",
                            qualified,
                            quoted_columns.join(", "),
                            values.join(", ")
                        )?;
                    }
                    writeln!(file)?;
                }
            }
        }
    }

    // Write footer
    writeln!(file, "-- End of dump")?;

    Ok(stats)
}

/// Statistics from an export operation.
#[derive(Debug, Default)]
pub struct ExportStats {
    pub tables_exported: usize,
    pub rows_exported: usize,
    /// `true` when at least one table was capped by the result-fetch limit, so
    /// the dump holds fewer rows than the source. Only engines without a
    /// streaming export path can set this; SQLite exports every row.
    pub partial: bool,
}

/// Import a SQL dump file into a database.
///
/// Statements are separated on `;` at the end of a line. `--` line comments and
/// `/* ... */` block comments are skipped. Errors are counted and logged rather
/// than aborting the whole import.
pub async fn import_database(
    conn: &Connection,
    path: &Path,
    progress: Option<Box<dyn Fn(ImportProgressInfo) + Send>>,
) -> Result<ImportStats> {
    let content = std::fs::read_to_string(path)?;
    let mut stats = ImportStats::default();
    let mut current_statement = String::new();
    let mut in_comment = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with("--") {
            continue;
        }

        // Handle multi-line comments
        if in_comment {
            if trimmed.ends_with("*/") {
                in_comment = false;
            }
            continue;
        }
        if trimmed.starts_with("/*") {
            if trimmed.ends_with("*/") {
                continue;
            }
            in_comment = true;
            continue;
        }

        current_statement.push_str(line);
        current_statement.push('\n');

        // Check if statement is complete (ends with ;)
        if trimmed.ends_with(';') {
            let statement = current_statement.trim().to_string();
            current_statement.clear();

            if !statement.is_empty() {
                if let Some(ref progress_fn) = progress {
                    progress_fn(ImportProgressInfo {
                        statements_executed: stats.statements_executed + 1,
                    });
                }

                match conn.execute(&statement).await {
                    Ok(_) => {
                        stats.statements_executed += 1;
                        if statement.to_uppercase().starts_with("INSERT") {
                            stats.rows_imported += 1;
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to execute statement: {}", e);
                        stats.errors += 1;
                    }
                }
            }
        }
    }

    Ok(stats)
}

/// Progress information for import operations.
#[derive(Debug, Clone)]
pub struct ImportProgressInfo {
    pub statements_executed: usize,
}

/// Statistics from an import operation.
#[derive(Debug, Default)]
pub struct ImportStats {
    pub statements_executed: usize,
    pub rows_imported: usize,
    pub errors: usize,
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;
