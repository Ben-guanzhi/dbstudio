use super::connection::MssqlConnection;
use dbstudio_core::result::{RowExt, SqlResult};
use dbstudio_core::schema::*;
use anyhow::Result;

impl MssqlConnection {
    pub async fn list_databases(&self) -> Result<Vec<DatabaseInfo>> {
        let current = self.current_database().await.unwrap_or_default();
        let result = self
            .execute("SELECT name FROM sys.databases WHERE database_id > 4 ORDER BY name")
            .await?;
        Ok(crate::utils::map_query_result(result, |row| {
            let name = row.first_str();
            DatabaseInfo {
                is_current: name == current,
                name,
            }
        }))
    }

    pub async fn list_schemas(&self, _database: &str) -> Result<Vec<SchemaInfo>> {
        let result = self
            .execute(
                "SELECT name FROM sys.schemas
                 WHERE name NOT IN ('INFORMATION_SCHEMA', 'sys', 'guest', 'db_owner',
                     'db_accessadmin', 'db_backupoperator', 'db_datareader', 'db_datawriter',
                     'db_ddladmin', 'db_denydatareader', 'db_denydatawriter', 'db_securityadmin')
                 ORDER BY name",
            )
            .await?;
        Ok(crate::utils::map_query_result(result, |row| SchemaInfo {
            name: row.first_str(),
            database: None,
        }))
    }

    pub async fn list_tables(&self, _schema: &str) -> Result<Vec<TableInfo>> {
        let sql = "SELECT TABLE_NAME, TABLE_TYPE, TABLE_SCHEMA
                   FROM INFORMATION_SCHEMA.TABLES
                   WHERE TABLE_TYPE IN ('BASE TABLE', 'VIEW')
                   ORDER BY TABLE_SCHEMA, TABLE_NAME";
        let result = self.execute(sql).await?;
        Ok(crate::utils::map_query_result(result, |row| TableInfo {
            name: row.first_str(),
            schema: row.get(2).map(|c| c.value.clone()),
            database: None,
            table_type: match row.cell(1) {
                "VIEW" => TableType::View,
                _ => TableType::Table,
            },
            row_count: None,
            comment: None,
        }))
    }

    pub async fn list_columns(&self, table: &str, schema: Option<&str>) -> Result<Vec<ColumnInfo>> {
        let schema = schema.unwrap_or("dbo");
        let pk_sql = "SELECT ccu.COLUMN_NAME
                      FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc
                      JOIN INFORMATION_SCHEMA.KEY_COLUMN_USAGE ccu
                        ON tc.CONSTRAINT_NAME = ccu.CONSTRAINT_NAME
                      WHERE tc.CONSTRAINT_TYPE = 'PRIMARY KEY'
                        AND tc.TABLE_SCHEMA = @p1 AND tc.TABLE_NAME = @p2";
        let pk_result = self.execute_parameterized(pk_sql, &[schema, table]).await?;
        let pk_columns: std::collections::HashSet<String> =
            crate::utils::map_query_result(pk_result, |row| row.first_str())
                .into_iter()
                .collect();

        let sql = "SELECT COLUMN_NAME, DATA_TYPE, ORDINAL_POSITION, IS_NULLABLE, COLUMN_DEFAULT
                   FROM INFORMATION_SCHEMA.COLUMNS
                   WHERE TABLE_SCHEMA = @p1 AND TABLE_NAME = @p2
                   ORDER BY ORDINAL_POSITION";
        let result = self.execute_parameterized(sql, &[schema, table]).await?;
        match result {
            SqlResult::Query(q) => Ok(q.rows
                .into_iter()
                .enumerate()
                .map(|(i, row)| ColumnInfo {
                    name: row.cell_str(0),
                    data_type: row.cell_str(1),
                    ordinal: row.cell_u32(2, i as u32),
                    nullable: !row.cell_is(3, "NO"),
                    is_primary_key: pk_columns.contains(&row.cell_str(0)),
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

    pub async fn list_indexes(&self, table: &str, schema: Option<&str>) -> Result<Vec<IndexInfo>> {
        let schema = schema.unwrap_or("dbo");
        let sql = "SELECT i.name, i.is_unique, i.is_primary_key,
                    STUFF((SELECT ', ' + c.name FROM sys.index_columns ic
                           JOIN sys.columns c ON ic.object_id = c.object_id AND ic.column_id = c.column_id
                           WHERE ic.object_id = i.object_id AND ic.index_id = i.index_id
                           ORDER BY ic.key_ordinal
                           FOR XML PATH('')), 1, 2, '') as columns
             FROM sys.indexes i
             JOIN sys.tables t ON i.object_id = t.object_id
             JOIN sys.schemas s ON t.schema_id = s.schema_id
             WHERE s.name = @p1 AND t.name = @p2";
        let result = self.execute_parameterized(sql, &[schema, table]).await?;
        let table = table.to_string();
        Ok(crate::utils::map_query_result(result, move |row| {
            let columns: Vec<String> = row
                .get(3)
                .map(|c| {
                    c.value
                        .split(',')
                        .map(|x| x.trim().to_string())
                        .filter(|x| !x.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            IndexInfo {
                name: row.cell_str(0),
                table_name: table.clone(),
                columns,
                is_unique: row.cell_is(1, "1"),
                is_primary: row.cell_is(2, "1"),
                index_type: None,
            }
        }))
    }

    pub async fn list_foreign_keys(&self, table: &str, schema: Option<&str>) -> Result<Vec<ForeignKeyInfo>> {
        let schema = schema.unwrap_or("dbo");
        let sql = "SELECT fk.name, COL_NAME(fkc.parent_object_id, fkc.parent_column_id) as col,
                    OBJECT_NAME(fkc.referenced_object_id) as ref_table,
                    COL_NAME(fkc.referenced_object_id, fkc.referenced_column_id) as ref_col
             FROM sys.foreign_keys fk
             JOIN sys.foreign_key_columns fkc ON fk.object_id = fkc.constraint_object_id
             JOIN sys.tables t ON fk.parent_object_id = t.object_id
             JOIN sys.schemas s ON t.schema_id = s.schema_id
             WHERE s.name = @p1 AND t.name = @p2";
        let result = self.execute_parameterized(sql, &[schema, table]).await?;
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

    pub async fn get_create_table_sql(&self, table: &str, schema: Option<&str>) -> Result<String> {
        let schema = schema.unwrap_or("dbo");
        let result = self
            .execute(&format!(
                "SELECT definition FROM sys.sql_modules
                 WHERE object_id = OBJECT_ID({})",
                crate::utils::quote_string_literal(&format!("{}.{}", schema, table))
            ))
            .await?;
        Ok(result
            .first_cell()
            .map(|c| c.value.clone())
            .unwrap_or_default())
    }
}