use sqlparser::dialect::GenericDialect;
use sqlparser::dialect::MySqlDialect;
use sqlparser::parser::Parser;

use crate::mapping::PositionMapping;

/// Result of parsing an SQL virtual document.
pub struct SqlParseResult {
    /// Syntax errors found in the SQL.
    pub diagnostics: Vec<SqlDiagnostic>,
}

/// An SQL diagnostic (error or warning).
#[derive(Debug, Clone)]
pub struct SqlDiagnostic {
    /// Error message from the SQL parser.
    pub message: String,
    /// Byte offset in the virtual document where the error starts (approximate).
    pub virtual_offset: Option<u32>,
}

/// Parse an SQL virtual document and return diagnostics.
pub fn parse_sql(virtual_document: &str) -> SqlParseResult {
    // Replace placeholder `?` sequences with SQL-valid placeholders.
    // sqlparser can't handle `????` but can handle `NULL` or `1`.
    let cleaned = clean_placeholders(virtual_document);

    let dialect = MySqlDialect {};
    match Parser::parse_sql(&dialect, &cleaned) {
        Ok(_statements) => SqlParseResult { diagnostics: vec![] },
        Err(err) => {
            // Try with generic dialect as fallback
            let generic = GenericDialect {};
            match Parser::parse_sql(&generic, &cleaned) {
                Ok(_) => SqlParseResult { diagnostics: vec![] },
                Err(_) => SqlParseResult {
                    diagnostics: vec![SqlDiagnostic {
                        message: err.to_string(),
                        virtual_offset: None,
                    }],
                },
            }
        }
    }
}

/// SQL keyword completions.
pub fn sql_keyword_completions() -> Vec<&'static str> {
    vec![
        "SELECT", "FROM", "WHERE", "AND", "OR", "NOT", "IN", "BETWEEN",
        "LIKE", "IS", "NULL", "AS", "ON", "JOIN", "INNER", "LEFT", "RIGHT",
        "OUTER", "CROSS", "GROUP", "BY", "ORDER", "ASC", "DESC", "HAVING",
        "LIMIT", "OFFSET", "UNION", "ALL", "INSERT", "INTO", "VALUES",
        "UPDATE", "SET", "DELETE", "CREATE", "TABLE", "ALTER", "DROP",
        "INDEX", "VIEW", "IF", "EXISTS", "NOT", "PRIMARY", "KEY",
        "FOREIGN", "REFERENCES", "UNIQUE", "DEFAULT", "AUTO_INCREMENT",
        "CASCADE", "DISTINCT", "COUNT", "SUM", "AVG", "MIN", "MAX",
        "CASE", "WHEN", "THEN", "ELSE", "END", "COALESCE", "CAST",
        "WITH", "RECURSIVE", "EXPLAIN", "ANALYZE",
    ]
}

/// Replace `????` placeholder sequences with `NULL` so sqlparser can handle them.
fn clean_placeholders(sql: &str) -> String {
    let mut result = String::with_capacity(sql.len());
    let mut in_placeholder = false;

    for ch in sql.chars() {
        if ch == '?' {
            if !in_placeholder {
                result.push_str("NULL");
                in_placeholder = true;
            }
            // Skip additional ? characters
        } else {
            in_placeholder = false;
            result.push(ch);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_sql() {
        let result = parse_sql("SELECT * FROM users WHERE id = 1");
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn test_parse_invalid_sql() {
        let result = parse_sql("SELEC * FORM users");
        assert!(!result.diagnostics.is_empty());
    }

    #[test]
    fn test_parse_sql_with_placeholders() {
        let result = parse_sql("SELECT * FROM ???? WHERE id = ???");
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn test_clean_placeholders() {
        assert_eq!(clean_placeholders("SELECT * FROM ???? WHERE id = ???"), "SELECT * FROM NULL WHERE id = NULL");
    }

    #[test]
    fn test_sql_keywords_not_empty() {
        assert!(!sql_keyword_completions().is_empty());
    }
}
