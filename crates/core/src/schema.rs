use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableInfo {
    pub name: String,
    pub schema: Option<String>,
    pub database: Option<String>,
    pub table_type: TableType,
    pub row_count: Option<u64>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TableType {
    Table,
    View,
    MaterializedView,
}

impl TableType {
    pub fn display_name(&self) -> &'static str {
        match self {
            TableType::Table => "Tables",
            TableType::View => "Views",
            TableType::MaterializedView => "Materialized Views",
        }
    }
}

impl std::fmt::Display for TableType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TableType::Table => write!(f, "table"),
            TableType::View => write!(f, "view"),
            TableType::MaterializedView => write!(f, "materialized_view"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub ordinal: u32,
    pub nullable: bool,
    pub is_primary_key: bool,
    pub default_value: Option<String>,
    pub comment: Option<String>,
    pub table_name: Option<String>,
}

impl ColumnInfo {
    pub fn new(name: impl Into<String>, data_type: impl Into<String>, ordinal: u32) -> Self {
        Self {
            name: name.into(),
            data_type: data_type.into(),
            ordinal,
            nullable: true,
            is_primary_key: false,
            default_value: None,
            comment: None,
            table_name: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexInfo {
    pub name: String,
    pub table_name: String,
    pub columns: Vec<String>,
    pub is_unique: bool,
    pub is_primary: bool,
    pub index_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForeignKeyInfo {
    pub name: String,
    pub source_table: String,
    pub source_columns: Vec<String>,
    pub target_table: String,
    pub target_columns: Vec<String>,
    pub on_delete: Option<String>,
    pub on_update: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseSchema {
    pub tables: Vec<TableInfo>,
    pub total_tables: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableSchema {
    pub table_name: String,
    pub columns: Vec<ColumnInfo>,
    pub indexes: Vec<IndexInfo>,
    pub foreign_keys: Vec<ForeignKeyInfo>,
    pub create_sql: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseInfo {
    pub name: String,
    pub is_current: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaInfo {
    pub name: String,
    pub database: Option<String>,
}
