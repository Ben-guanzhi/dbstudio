use super::connection::SqliteConnection;
use dbstudio_core::result::{RowExt, SqlResult};
use dbstudio_core::schema::*;
use anyhow::Result;

impl SqliteConnection {
    pub async fn list_databases(&self) -> Result<Vec<DatabaseInfo>> {
        Ok(vec![])
    }

    pub async fn list_schemas(&self, _database: &str) -> Result<Vec<SchemaInfo>> {
        Ok(vec![])
    }

    pub async fn list_tables(&self, _schema: &str) -> Result<Vec<TableInfo>> {
        let result = self
            .execute(
                "SELECT name, type FROM sqlite_master
                 WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%'
                 ORDER BY name",
            )
            .await?;

        Ok(crate::utils::map_query_result(result, |row| {
            let name = row.first_str();
            let table_type = match row.cell(1) {
                "view" => TableType::View,
                _ => TableType::Table,
            };
            TableInfo {
                name,
                schema: None,
                database: None,
                table_type,
                row_count: None,
                comment: None,
            }
        }))
    }

    pub async fn list_columns(&self, table: &str, _schema: Option<&str>) -> Result<Vec<ColumnInfo>> {
        let sql = format!("PRAGMA table_info({})", crate::utils::quote_backtick(table));
        let result = self.execute(&sql).await?;

        match result {
            SqlResult::Query(q) => Ok(q.rows.into_iter().enumerate().map(|(i, row)| {
                ColumnInfo {
                    name: row.cell_str(1),
                    data_type: row.cell_str(2),
                    ordinal: i as u32,
                    nullable: !row.cell_is(3, "1"),
                    is_primary_key: row.cell_is(5, "1"),
                    default_value: row
                        .get(4)
                        .and_then(|c| c.value_if_not_null()),
                    comment: None,
                    table_name: Some(table.to_string()),
                }
            }).collect()),
            _ => Ok(vec![]),
        }
    }

    pub async fn list_indexes(&self, table: &str, _schema: Option<&str>) -> Result<Vec<IndexInfo>> {
        let sql = format!("PRAGMA index_list({})", crate::utils::quote_backtick(table));
        let result = self.execute(&sql).await?;

        match result {
            SqlResult::Query(q) => {
                let mut indexes = Vec::new();
                for row in q.rows {
                    let name = row.first_str();
                    let is_unique = row.cell_is(1, "1");
                    indexes.push(IndexInfo {
                        name: name.clone(),
                        table_name: table.to_string(),
                        columns: self.index_columns(&name).await.unwrap_or_default(),
                        is_unique,
                        is_primary: false,
                        index_type: None,
                    });
                }
                Ok(indexes)
            }
            _ => Ok(vec![]),
        }
    }

    async fn index_columns(&self, index_name: &str) -> Result<Vec<String>> {
        let sql = format!("PRAGMA index_info({})", crate::utils::quote_backtick(index_name));
        let result = self.execute(&sql).await?;
        Ok(crate::utils::map_query_result(result, |row| row.cell_str(2)))
    }

    pub async fn list_foreign_keys(&self, table: &str, _schema: Option<&str>) -> Result<Vec<ForeignKeyInfo>> {
        let sql = format!("PRAGMA foreign_key_list({})", crate::utils::quote_backtick(table));
        let result = self.execute(&sql).await?;
        let table = table.to_string();

        Ok(crate::utils::map_query_result(result, move |row| {
            ForeignKeyInfo {
                name: String::new(),
                source_table: table.clone(),
                source_columns: vec![row.cell_str(3)],
                target_table: row.cell_str(2),
                target_columns: vec![row.cell_str(4)],
                on_delete: row.get(5).and_then(|c| c.value_if_not_null()),
                on_update: row.get(6).and_then(|c| c.value_if_not_null()),
            }
        }))
    }

    pub async fn get_create_table_sql(&self, table: &str, _schema: Option<&str>) -> Result<String> {
        let sql = format!(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = {}",
            crate::utils::quote_string_literal(table)
        );
        let result = self.execute(&sql).await?;
        Ok(result
            .first_cell()
            .map(|c| c.value.clone())
            .unwrap_or_default())
    }
}