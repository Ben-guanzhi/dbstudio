use super::connection::PostgresConnection;
use dbstudio_core::result::{RowExt, SqlResult};
use dbstudio_core::schema::*;
use anyhow::Result;

impl PostgresConnection {
    pub async fn list_databases(&self) -> Result<Vec<DatabaseInfo>> {
        let current = self.current_database().await.unwrap_or_default();
        let result = self
            .execute(
                "SELECT datname FROM pg_database WHERE datistemplate = false ORDER BY datname",
            )
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
                "SELECT schema_name FROM information_schema.schemata
                 WHERE schema_name NOT IN ('information_schema', 'pg_catalog', 'pg_toast')
                 ORDER BY schema_name",
            )
            .await?;
        Ok(crate::utils::map_query_result(result, |row| SchemaInfo {
            name: row.first_str(),
            database: None,
        }))
    }

    pub async fn list_tables(&self, schema: &str) -> Result<Vec<TableInfo>> {
        let schema = if schema.is_empty() { "public" } else { schema };
        let sql = "SELECT table_name, table_type, table_schema
                   FROM information_schema.tables
                   WHERE table_schema = $1
                   ORDER BY table_name";
        let result = self.execute_parameterized(sql, &[schema]).await?;
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
        let schema = schema.unwrap_or("public");
        let pk_sql = "SELECT kcu.column_name
                      FROM information_schema.table_constraints tc
                      JOIN information_schema.key_column_usage kcu
                        ON tc.constraint_name = kcu.constraint_name
                      WHERE tc.constraint_type = 'PRIMARY KEY'
                        AND tc.table_schema = $1 AND tc.table_name = $2";
        let pk_result = self.execute_parameterized(pk_sql, &[schema, table]).await?;
        let pk_columns: std::collections::HashSet<String> =
            crate::utils::map_query_result(pk_result, |row| row.first_str())
                .into_iter()
                .collect();

        let sql = "SELECT column_name, data_type, ordinal_position, is_nullable, column_default
                   FROM information_schema.columns
                   WHERE table_schema = $1 AND table_name = $2
                   ORDER BY ordinal_position";
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
        let schema = schema.unwrap_or("public");
        let sql = "SELECT indexname, indexdef FROM pg_indexes
                   WHERE schemaname = $1 AND tablename = $2";
        let result = self.execute_parameterized(sql, &[schema, table]).await?;
        let table = table.to_string();
        Ok(crate::utils::map_query_result(result, move |row| {
            let name = row.first_str();
            IndexInfo {
                name: name.clone(),
                table_name: table.clone(),
                columns: Vec::new(),
                is_unique: row.cell(1).contains("UNIQUE"),
                is_primary: name.contains("_pkey"),
                index_type: row.get(1).and_then(|c| {
                    if c.value.contains("btree") {
                        Some("btree".to_string())
                    } else if c.value.contains("gin") {
                        Some("gin".to_string())
                    } else if c.value.contains("gist") {
                        Some("gist".to_string())
                    } else {
                        None
                    }
                }),
            }
        }))
    }

    pub async fn list_foreign_keys(&self, table: &str, schema: Option<&str>) -> Result<Vec<ForeignKeyInfo>> {
        let schema = schema.unwrap_or("public");
        let sql = "SELECT tc.constraint_name, kcu.column_name,
                          ccu.table_name AS foreign_table_name,
                          ccu.column_name AS foreign_column_name,
                          rc.update_rule, rc.delete_rule
                   FROM information_schema.table_constraints tc
                   JOIN information_schema.key_column_usage kcu ON tc.constraint_name = kcu.constraint_name
                   JOIN information_schema.constraint_column_usage ccu ON tc.constraint_name = ccu.constraint_name
                   JOIN information_schema.referential_constraints rc ON tc.constraint_name = rc.constraint_name
                   WHERE tc.constraint_type = 'FOREIGN KEY' AND tc.table_schema = $1 AND tc.table_name = $2";
        let result = self.execute_parameterized(sql, &[schema, table]).await?;
        let table = table.to_string();
        Ok(crate::utils::map_query_result(result, move |row| ForeignKeyInfo {
            name: row.cell_str(0),
            source_table: table.clone(),
            source_columns: vec![row.cell_str(1)],
            target_table: row.cell_str(2),
            target_columns: vec![row.cell_str(3)],
            on_update: row.get(4).and_then(|c| c.value_if_not_null()),
            on_delete: row.get(5).and_then(|c| c.value_if_not_null()),
        }))
    }

    pub async fn get_create_table_sql(&self, table: &str, schema: Option<&str>) -> Result<String> {
        let schema = schema.unwrap_or("public");
        let sql = "SELECT column_name, data_type, is_nullable, column_default
                   FROM information_schema.columns
                   WHERE table_schema = $1 AND table_name = $2
                   ORDER BY ordinal_position";
        let result = self.execute_parameterized(sql, &[schema, table]).await?;
        match result {
            SqlResult::Query(q) => {
                let mut lines = Vec::new();
                for row in q.rows {
                    let col = row.cell_str(0);
                    let typ = row.cell_str(1);
                    let nullable = !row.cell_is(2, "NO");
                    let default = row.get(3).and_then(|c| c.value_if_not_null());
                    let mut def = format!("    {} {}", crate::utils::quote_double_quote(&col), typ);
                    if !nullable {
                        def.push_str(" NOT NULL");
                    }
                    if let Some(d) = default {
                        def.push_str(&format!(" DEFAULT {}", d));
                    }
                    lines.push(def);
                }
                let stmt = format!(
                    "CREATE TABLE {}.{} (\n{}\n);",
                    crate::utils::quote_double_quote(schema),
                    crate::utils::quote_double_quote(table),
                    lines.join(",\n")
                );
                Ok(stmt)
            }
            _ => Ok(String::new()),
        }
    }
}
