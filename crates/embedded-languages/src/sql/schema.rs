use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct SqlSchema {
    pub tables: HashMap<String, TableSchema>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TableSchema {
    pub columns: HashMap<String, ColumnSchema>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ColumnSchema {
    #[serde(rename = "type")]
    pub data_type: String,
    #[serde(default)]
    pub nullable: bool,
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub auto_increment: bool,
    #[serde(default)]
    pub unique: bool,
    #[serde(default)]
    pub foreign_key: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

impl SqlSchema {
    pub fn load_from_file(path: &Path) -> Result<Self, String> {
        let content =
            std::fs::read_to_string(path).map_err(|e| format!("Failed to read SQL schema file: {}", e))?;
        serde_json::from_str(&content).map_err(|e| format!("Failed to parse SQL schema JSON: {}", e))
    }

    pub fn table_names(&self) -> Vec<&str> {
        self.tables.keys().map(|s| s.as_str()).collect()
    }

    pub fn get_table(&self, name: &str) -> Option<&TableSchema> {
        // Case-insensitive lookup
        self.tables.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v)
    }
}

impl TableSchema {
    pub fn column_names(&self) -> Vec<&str> {
        self.columns.keys().map(|s| s.as_str()).collect()
    }

    pub fn get_column(&self, name: &str) -> Option<&ColumnSchema> {
        self.columns.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v)
    }
}
