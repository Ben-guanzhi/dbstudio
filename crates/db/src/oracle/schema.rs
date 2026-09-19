use super::connection::OracleConnection;
use dbstudio_core::result::{RowExt, SqlResult};
use dbstudio_core::schema::*;
use anyhow::Result;

impl OracleConnection {
    pub async fn list_databases(&self) -> Result<Vec<DatabaseInfo>> {
        Ok(vec![])
    }

    pub async fn list_schemas(&self, _database: &str) -> Result<Vec<SchemaInfo>> {
        let result = self
            .execute("SELECT username FROM all_users WHERE oracle_maintained = 'N' ORDER BY username")
            .await?;
        Ok(crate::utils::map_query_result(result, |row| SchemaInfo {
            name: row.first_str(),
            database: None,
        }))
    }

    pub async fn list_tables(&self, _schema: &str) -> Result<Vec<TableInfo>> {
        let sql = "SELECT owner, table_name
                   FROM all_tables
                   WHERE owner IN (SELECT username FROM all_users WHERE oracle_maintained = 'N')
                     AND table_name NOT LIKE 'BIN$%'
                   ORDER BY owner, table_name";
        let result = self.execute(sql).await?;
        Ok(crate::utils::map_query_result(result, |row| TableInfo {
            name: row.cell_str(1),
            schema: row.first().map(|c| c.value.clone()).filter(|s| !s.is_empty()),
            database: None,
            table_type: TableType::Table,
            row_count: None,
            comment: None,
        }))
    }

    pub async fn list_columns(&self, table: &str, schema: Option<&str>) -> Result<Vec<ColumnInfo>> {
        let schema = schema.unwrap_or("PUBLIC").to_uppercase();
        let table_upper = table.to_uppercase();
        let pk_sql = "SELECT acc.column_name
                      FROM all_constraints ac
                      JOIN all_cons_columns acc
                        ON ac.constraint_name = acc.constraint_name AND ac.owner = acc.owner
                      WHERE ac.constraint_type = 'P' AND ac.owner = :1 AND ac.table_name = :2";
        let pk_result = self.execute_parameterized(pk_sql, &[&schema, &table_upper]).await?;
        let pk_columns: std::collections::HashSet<String> =
            crate::utils::map_query_result(pk_result, |row| row.first_str())
                .into_iter()
                .collect();

        let sql = "SELECT column_name, data_type, column_id, nullable, data_default
                   FROM all_tab_columns
                   WHERE owner = :1 AND table_name = :2
                   ORDER BY column_id";
        let result = self.execute_parameterized(sql, &[&schema, &table_upper]).await?;
        match result {
            SqlResult::Query(q) => Ok(q.rows
                .into_iter()
                .enumerate()
                .map(|(i, row)| ColumnInfo {
                    name: row.cell_str(0),
                    data_type: row.cell_str(1),
                    ordinal: row.cell_u32(2, i as u32),
                    nullable: !row.cell_is(3, "N"),
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
        let schema = schema.unwrap_or("PUBLIC").to_uppercase();
        let table_upper = table.to_uppercase();
        let sql = "SELECT i.index_name, i.index_type, i.uniqueness,
                    LISTAGG(c.column_name, ',') WITHIN GROUP (ORDER BY c.column_position) as columns
             FROM all_indexes i
             JOIN all_ind_columns c ON i.index_name = c.index_name AND i.owner = c.index_owner
             WHERE i.owner = :1 AND i.table_name = :2
             GROUP BY i.index_name, i.index_type, i.uniqueness";
        let result = self.execute_parameterized(sql, &[&schema, &table_upper]).await?;
        let table = table.to_string();
        Ok(crate::utils::map_query_result(result, move |row| {
            let columns: Vec<String> = row
                .get(3)
                .map(|c| c.value.split(',').map(String::from).collect())
                .unwrap_or_default();
            IndexInfo {
                name: row.cell_str(0),
                table_name: table.clone(),
                columns,
                is_unique: row.cell_is(2, "UNIQUE"),
                is_primary: false,
                index_type: row.get(1).map(|c| c.value.clone()),
            }
        }))
    }

    pub async fn list_foreign_keys(&self, table: &str, schema: Option<&str>) -> Result<Vec<ForeignKeyInfo>> {
        let schema = schema.unwrap_or("PUBLIC").to_uppercase();
        let table_upper = table.to_uppercase();
        let sql = "SELECT c.constraint_name, cc.column_name,
                    r.table_name, rc.column_name, c.delete_rule
             FROM all_constraints c
             JOIN all_cons_columns cc ON c.constraint_name = cc.constraint_name AND c.owner = cc.owner
             JOIN all_constraints r ON c.r_constraint_name = r.constraint_name
             JOIN all_cons_columns rc ON r.constraint_name = rc.constraint_name AND r.owner = rc.owner
             WHERE c.constraint_type = 'R' AND c.owner = :1 AND c.table_name = :2";
        let result = self.execute_parameterized(sql, &[&schema, &table_upper]).await?;
        let table = table.to_string();
        Ok(crate::utils::map_query_result(result, move |row| ForeignKeyInfo {
            name: row.cell_str(0),
            source_table: table.clone(),
            source_columns: vec![row.cell_str(1)],
            target_table: row.cell_str(2),
            target_columns: vec![row.cell_str(3)],
            on_delete: row.get(4).and_then(|c| c.value_if_not_null()),
            on_update: None,
        }))
    }

    pub async fn get_create_table_sql(&self, table: &str, schema: Option<&str>) -> Result<String> {
        let schema = schema.unwrap_or("PUBLIC").to_uppercase();
        let table_upper = table.to_uppercase();
        let sql = "SELECT column_name,
                    data_type || CASE WHEN data_length IS NOT NULL AND data_type IN ('VARCHAR2', 'CHAR')
                        THEN '(' || data_length || ')' ELSE '' END ||
                    CASE WHEN data_precision IS NOT NULL THEN '(' || data_precision ||
                        CASE WHEN data_scale > 0 THEN ',' || data_scale ELSE '' END || ')'
                        ELSE '' END,
                    nullable, data_default
             FROM all_tab_columns
             WHERE owner = :1 AND table_name = :2
             ORDER BY column_id";
        let result = self.execute_parameterized(sql, &[&schema, &table_upper]).await?;
        match result {
            SqlResult::Query(q) => {
                let mut lines = Vec::new();
                for row in q.rows {
                    let col = row.cell_str(0);
                    let typ = row.cell_str(1);
                    let nullable = !row.cell_is(2, "N");
                    let default = row.get(3).and_then(|c| c.value_if_not_null());
                    let mut def = format!("    {} {}", crate::utils::quote_double_quote(&col), typ);
                    if !nullable {
                        def.push_str(" NOT NULL");
                    }
                    if let Some(d) = default {
                        if !d.is_empty() {
                            def.push_str(&format!(" DEFAULT {}", d));
                        }
                    }
                    lines.push(def);
                }
                let stmt = format!(
                    "CREATE TABLE {}.{} (\n{}\n);",
                    crate::utils::quote_double_quote(&schema),
                    crate::utils::quote_double_quote(&table.to_uppercase()),
                    lines.join(",\n")
                );
                Ok(stmt)
            }
            _ => Ok(String::new()),
        }
    }
}