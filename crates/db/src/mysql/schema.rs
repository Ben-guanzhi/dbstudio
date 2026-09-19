use super::connection::MySqlConnection;
use dbstudio_core::result::{RowExt, SqlResult};
use dbstudio_core::schema::*;
use anyhow::Result;

impl MySqlConnection {
    pub async fn list_databases(&self) -> Result<Vec<DatabaseInfo>> {
        let current = self.current_database().await.unwrap_or_default();
        let result = self.execute("SHOW DATABASES").await?;
        match result {
            SqlResult::Query(q) => {
                let system = ["information_schema", "performance_schema", "mysql", "sys"];
                Ok(q.rows
                    .into_iter()
                    .filter_map(|row| {
                        let name = row.first()?.value.clone();
                        if system.contains(&name.as_str()) {
                            None
                        } else {
                            Some(DatabaseInfo {
                                is_current: name == current,
                                name,
                            })
                        }
                    })
                    .collect())
            }
            _ => Ok(vec![]),
        }
    }

    pub async fn list_schemas(&self, _database: &str) -> Result<Vec<SchemaInfo>> {
        Ok(vec![])
    }

    pub async fn list_tables(&self, _schema: &str) -> Result<Vec<TableInfo>> {
        let result = self
            .execute(
                "SELECT TABLE_NAME, TABLE_TYPE FROM information_schema.TABLES
                 WHERE TABLE_SCHEMA = DATABASE() ORDER BY TABLE_NAME",
            )
            .await?;
        Ok(crate::utils::map_query_result(result, |row| TableInfo {
            name: row.first_str(),
            schema: None,
            database: None,
            table_type: match row.cell(1) {
                "VIEW" => TableType::View,
                _ => TableType::Table,
            },
            row_count: None,
            comment: None,
        }))
    }

    pub async fn list_columns(&self, table: &str, _schema: Option<&str>) -> Result<Vec<ColumnInfo>> {
        let sql = "SELECT COLUMN_NAME, DATA_TYPE, ORDINAL_POSITION, IS_NULLABLE, COLUMN_DEFAULT, COLUMN_KEY
                   FROM information_schema.COLUMNS
                   WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ?
                   ORDER BY ORDINAL_POSITION";
        let result = self.execute_parameterized(sql, &[table]).await?;
        match result {
            SqlResult::Query(q) => Ok(q.rows
                .into_iter()
                .enumerate()
                .map(|(i, row)| ColumnInfo {
                    name: row.cell_str(0),
                    data_type: row.cell_str(1),
                    ordinal: row.cell_u32(2, i as u32),
                    nullable: !row.cell_is(3, "NO"),
                    is_primary_key: row.cell_is(5, "PRI"),
                    default_value: row
                        .get(4)
                        .and_then(|c| c.value_if_not_null()),
                    comment: None,
                    table_name: Some(table.to_string()),
                })
                .collect()),
            _ => Ok(vec![]),
        }
    }

    pub async fn list_indexes(&self, table: &str, _schema: Option<&str>) -> Result<Vec<IndexInfo>> {
        let sql = "SELECT INDEX_NAME, GROUP_CONCAT(COLUMN_NAME ORDER BY SEQ_IN_INDEX) as cols, NON_UNIQUE
                   FROM information_schema.STATISTICS
                   WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ?
                   GROUP BY INDEX_NAME, NON_UNIQUE";
        let result = self.execute_parameterized(sql, &[table]).await?;
        let table = table.to_string();
        Ok(crate::utils::map_query_result(result, move |row| {
            let columns: Vec<String> = row
                .get(1)
                .map(|c| c.value.split(',').map(String::from).collect())
                .unwrap_or_default();
            IndexInfo {
                name: row.first_str(),
                table_name: table.clone(),
                columns,
                is_unique: row.cell_is(2, "0"),
                is_primary: false,
                index_type: None,
            }
        }))
    }

    pub async fn list_foreign_keys(&self, table: &str, _schema: Option<&str>) -> Result<Vec<ForeignKeyInfo>> {
        let sql = "SELECT CONSTRAINT_NAME, COLUMN_NAME, REFERENCED_TABLE_NAME, REFERENCED_COLUMN_NAME
                   FROM information_schema.KEY_COLUMN_USAGE
                   WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ? AND REFERENCED_TABLE_NAME IS NOT NULL";
        let result = self.execute_parameterized(sql, &[table]).await?;
        let table = table.to_string();
        Ok(crate::utils::map_query_result(result, move |row| ForeignKeyInfo {
            name: row.cell_str(0),
            source_table: table.clone(),
            source_columns: vec![row.cell_str(1)],
            target_table: row.cell_str(2),
            target_columns: vec![row.cell_str(3)],
            on_delete: None,
            on_update: None,
        }))
    }

    pub async fn get_create_table_sql(&self, table: &str, _schema: Option<&str>) -> Result<String> {
        let result = self
            .execute(&format!(
                "SHOW CREATE TABLE {}",
                crate::utils::quote_backtick(table)
            ))
            .await?;
        // `SHOW CREATE TABLE` returns (table, ddl) — the DDL is the second column.
        Ok(result
            .first_row()
            .map(|row| row.cell_str(1))
            .unwrap_or_default())
    }
}
