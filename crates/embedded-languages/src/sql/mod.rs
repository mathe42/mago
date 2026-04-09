pub mod context;
pub mod functions;
pub mod schema;

use sqlparser::dialect::GenericDialect;
use sqlparser::dialect::MySqlDialect;
use sqlparser::parser::Parser;

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

// ── Context-aware completions ────────────────────────────────────────────

/// A single context-aware SQL completion item.
#[derive(Debug, Clone)]
pub struct SqlCompletionItem {
    pub label: String,
    pub kind: SqlCompletionKind,
    pub detail: Option<String>,
    pub documentation: Option<String>,
}

/// The kind of a SQL completion item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SqlCompletionKind {
    Keyword,
    Table,
    Column,
    Function,
}

/// Produce context-aware SQL completions at the given offset.
///
/// Uses heuristic context detection to determine what kind of token the user
/// is most likely typing, then returns relevant completions from the schema
/// and built-in function database.
pub fn sql_completions(
    virtual_document: &str,
    virtual_offset: u32,
    sql_schema: Option<&schema::SqlSchema>,
) -> Vec<SqlCompletionItem> {
    let ctx = context::detect_sql_context(virtual_document, virtual_offset);
    let referenced_tables = context::extract_table_names(virtual_document);
    let mut items = Vec::new();

    match ctx {
        context::SqlContext::SelectList => {
            // Columns from referenced tables.
            add_columns_from_tables(&mut items, sql_schema, &referenced_tables);
            // Functions.
            add_function_completions(&mut items);
            // Useful keywords.
            add_keyword_items(&mut items, &["*", "DISTINCT", "AS", "CASE", "NULL"]);
        }
        context::SqlContext::TableRef => {
            // Table names from schema.
            add_table_completions(&mut items, sql_schema);
            // Join-related keywords.
            add_keyword_items(&mut items, &["JOIN", "INNER JOIN", "LEFT JOIN", "RIGHT JOIN", "CROSS JOIN", "ON", "AS"]);
        }
        context::SqlContext::Condition => {
            // Columns from referenced tables.
            add_columns_from_tables(&mut items, sql_schema, &referenced_tables);
            // Functions.
            add_function_completions(&mut items);
            // Operators and keywords.
            add_keyword_items(
                &mut items,
                &[
                    "AND", "OR", "NOT", "IN", "BETWEEN", "LIKE", "IS", "NULL", "IS NOT NULL", "EXISTS", "TRUE",
                    "FALSE",
                ],
            );
        }
        context::SqlContext::SetClause => {
            // Columns of the target table (the one after UPDATE).
            let target = extract_update_table(virtual_document);
            if let Some(ref table_name) = target {
                add_columns_of_table(&mut items, sql_schema, table_name);
            }
            // If no schema or no target, offer general columns.
            if items.is_empty() {
                add_columns_from_tables(&mut items, sql_schema, &referenced_tables);
            }
        }
        context::SqlContext::InsertColumns => {
            // Columns of the target table.
            let target = extract_insert_table(virtual_document);
            if let Some(ref table_name) = target {
                add_columns_of_table(&mut items, sql_schema, table_name);
            }
        }
        context::SqlContext::InsertValues => {
            // Functions and keywords useful in value positions.
            add_function_completions(&mut items);
            add_keyword_items(&mut items, &["NULL", "DEFAULT", "TRUE", "FALSE"]);
        }
        context::SqlContext::OrderBy => {
            // Columns from referenced tables.
            add_columns_from_tables(&mut items, sql_schema, &referenced_tables);
            add_keyword_items(&mut items, &["ASC", "DESC"]);
        }
        context::SqlContext::ColumnOfTable(ref table_name) => {
            add_columns_of_table(&mut items, sql_schema, table_name);
        }
        context::SqlContext::FunctionArgs(ref func_name) => {
            // Columns + * for aggregates.
            add_columns_from_tables(&mut items, sql_schema, &referenced_tables);
            if functions::is_aggregate_function(func_name) {
                add_keyword_items(&mut items, &["*", "DISTINCT"]);
            }
        }
        context::SqlContext::General => {
            // Everything: keywords + tables + functions.
            add_all_keyword_items(&mut items);
            add_table_completions(&mut items, sql_schema);
            add_function_completions(&mut items);
        }
    }

    items
}

// ── Hover support ────────────────────────────────────────────────────────

/// Information to display when hovering over a token in SQL.
#[derive(Debug, Clone)]
pub struct SqlHoverInfo {
    pub content: String,
}

/// Get hover information for the token at the given offset.
pub fn sql_hover(
    virtual_document: &str,
    virtual_offset: u32,
    sql_schema: Option<&schema::SqlSchema>,
) -> Option<SqlHoverInfo> {
    let (word, prefix) = word_at_offset(virtual_document, virtual_offset)?;

    // Check for `table.column` pattern.
    if let Some(dot_pos) = prefix.rfind('.') {
        let table_name = extract_word_before(prefix, dot_pos)?;
        if let Some(schema) = sql_schema {
            if let Some(table) = schema.get_table(&table_name) {
                if let Some(col) = table.get_column(&word) {
                    let mut content = format!("**{}.{}** — `{}`", table_name, word, col.data_type);
                    let mut constraints = Vec::new();
                    if col.primary {
                        constraints.push("PRIMARY KEY");
                    }
                    if col.auto_increment {
                        constraints.push("AUTO_INCREMENT");
                    }
                    if col.unique {
                        constraints.push("UNIQUE");
                    }
                    if col.nullable {
                        constraints.push("NULLABLE");
                    } else {
                        constraints.push("NOT NULL");
                    }
                    if !constraints.is_empty() {
                        content.push_str(&format!(" ({})", constraints.join(", ")));
                    }
                    if let Some(ref fk) = col.foreign_key {
                        content.push_str(&format!("\n\nForeign key: {}", fk));
                    }
                    if let Some(ref desc) = col.description {
                        content.push_str(&format!("\n\n{}", desc));
                    }
                    return Some(SqlHoverInfo { content });
                }
            }
        }
    }

    // Check if it's a known SQL function.
    if let Some(func) = functions::get_function(&word) {
        let content = format!(
            "**{}**\n\n`{}`\n\n{}\n\nReturns: `{}`",
            func.name, func.signature, func.description, func.return_type
        );
        return Some(SqlHoverInfo { content });
    }

    // Check if it's a table name in the schema.
    if let Some(schema) = sql_schema {
        if let Some(table) = schema.get_table(&word) {
            let mut content = format!("**Table: {}**", word);
            if let Some(ref desc) = table.description {
                content.push_str(&format!("\n\n{}", desc));
            }
            let columns = table.column_names();
            if !columns.is_empty() {
                content.push_str("\n\nColumns:");
                for col_name in &columns {
                    if let Some(col) = table.get_column(col_name) {
                        content.push_str(&format!("\n- `{}` {}", col_name, col.data_type));
                    }
                }
            }
            return Some(SqlHoverInfo { content });
        }
    }

    // Check if it's a SQL keyword.
    if let Some(desc) = keyword_description(&word) {
        return Some(SqlHoverInfo {
            content: format!("**{}** — {}", word.to_ascii_uppercase(), desc),
        });
    }

    None
}

// ── Signature help ───────────────────────────────────────────────────────

/// Signature information for a function call.
#[derive(Debug, Clone)]
pub struct SqlSignatureInfo {
    pub function_name: String,
    pub signature: String,
    pub parameters: Vec<(String, String)>,
    pub active_parameter: u32,
}

/// Get signature help for a function call at the given offset.
///
/// Walks backwards from the cursor to find an unmatched `(`, extracts the
/// function name, and counts commas to determine the active parameter.
pub fn sql_signature_help(virtual_document: &str, virtual_offset: u32) -> Option<SqlSignatureInfo> {
    let offset = (virtual_offset as usize).min(virtual_document.len());
    let before_cursor = &virtual_document[..offset];

    // Walk backwards to find an unmatched `(`.
    let mut depth = 0i32;
    let mut comma_count = 0u32;
    let mut paren_pos = None;

    for (i, ch) in before_cursor.char_indices().rev() {
        match ch {
            ')' => depth += 1,
            '(' => {
                if depth == 0 {
                    paren_pos = Some(i);
                    break;
                }
                depth -= 1;
            }
            ',' => {
                if depth == 0 {
                    comma_count += 1;
                }
            }
            _ => {}
        }
    }

    let paren_pos = paren_pos?;

    // Extract function name before the `(`.
    let before_paren = before_cursor[..paren_pos].trim_end();
    let func_name = extract_trailing_word(before_paren)?;

    // Look up the function.
    let func = functions::get_function(&func_name)?;

    let params: Vec<(String, String)> = func
        .parameters
        .iter()
        .map(|p| (p.name.to_string(), p.description.to_string()))
        .collect();

    Some(SqlSignatureInfo {
        function_name: func.name.to_string(),
        signature: func.signature.to_string(),
        parameters: params,
        active_parameter: comma_count,
    })
}

// ── Internal helpers ─────────────────────────────────────────────────────

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

/// Add function completions from the built-in database.
fn add_function_completions(items: &mut Vec<SqlCompletionItem>) {
    for func in functions::get_all_functions() {
        items.push(SqlCompletionItem {
            label: func.name.to_string(),
            kind: SqlCompletionKind::Function,
            detail: Some(func.signature.to_string()),
            documentation: Some(func.description.to_string()),
        });
    }
}

/// Add table name completions from the schema.
fn add_table_completions(items: &mut Vec<SqlCompletionItem>, sql_schema: Option<&schema::SqlSchema>) {
    if let Some(schema) = sql_schema {
        for name in schema.table_names() {
            let desc = schema.get_table(name).and_then(|t| t.description.clone());
            items.push(SqlCompletionItem {
                label: name.to_string(),
                kind: SqlCompletionKind::Table,
                detail: Some("Table".to_string()),
                documentation: desc,
            });
        }
    }
}

/// Add column completions for all referenced tables.
fn add_columns_from_tables(
    items: &mut Vec<SqlCompletionItem>,
    sql_schema: Option<&schema::SqlSchema>,
    table_names: &[String],
) {
    if let Some(schema) = sql_schema {
        for tbl_name in table_names {
            if let Some(table) = schema.get_table(tbl_name) {
                for col_name in table.column_names() {
                    let col_detail = table.get_column(col_name).map(|c| c.data_type.clone());
                    items.push(SqlCompletionItem {
                        label: col_name.to_string(),
                        kind: SqlCompletionKind::Column,
                        detail: col_detail,
                        documentation: table
                            .get_column(col_name)
                            .and_then(|c| c.description.clone()),
                    });
                }
            }
        }
    }
}

/// Add column completions for a specific table.
fn add_columns_of_table(
    items: &mut Vec<SqlCompletionItem>,
    sql_schema: Option<&schema::SqlSchema>,
    table_name: &str,
) {
    if let Some(schema) = sql_schema {
        if let Some(table) = schema.get_table(table_name) {
            for col_name in table.column_names() {
                let col_detail = table.get_column(col_name).map(|c| c.data_type.clone());
                items.push(SqlCompletionItem {
                    label: col_name.to_string(),
                    kind: SqlCompletionKind::Column,
                    detail: col_detail,
                    documentation: table
                        .get_column(col_name)
                        .and_then(|c| c.description.clone()),
                });
            }
        }
    }
}

/// Add keyword items by label.
fn add_keyword_items(items: &mut Vec<SqlCompletionItem>, keywords: &[&str]) {
    for kw in keywords {
        items.push(SqlCompletionItem {
            label: kw.to_string(),
            kind: SqlCompletionKind::Keyword,
            detail: Some("SQL keyword".to_string()),
            documentation: None,
        });
    }
}

/// Add all SQL keyword completions as items.
fn add_all_keyword_items(items: &mut Vec<SqlCompletionItem>) {
    for kw in sql_keyword_completions() {
        items.push(SqlCompletionItem {
            label: kw.to_string(),
            kind: SqlCompletionKind::Keyword,
            detail: Some("SQL keyword".to_string()),
            documentation: None,
        });
    }
}

/// Extract the table name after UPDATE.
fn extract_update_table(sql: &str) -> Option<String> {
    let upper = sql.to_ascii_uppercase();
    let pos = upper.rfind("UPDATE ")?;
    let after = &sql[pos + 7..];
    context::extract_table_names(&format!("UPDATE {}", after))
        .into_iter()
        .next()
}

/// Extract the table name after INSERT INTO.
fn extract_insert_table(sql: &str) -> Option<String> {
    let upper = sql.to_ascii_uppercase();
    let pos = upper.rfind("INSERT INTO ")?;
    let after = &sql[pos + 12..];
    context::extract_table_names(&format!("INSERT INTO {}", after))
        .into_iter()
        .next()
}

/// Get the word at the given cursor offset and the text before the word.
fn word_at_offset(text: &str, offset: u32) -> Option<(String, &str)> {
    let offset = (offset as usize).min(text.len());

    // Find word start.
    let before = &text[..offset];
    let word_start = before
        .bytes()
        .rposition(|b| !b.is_ascii_alphanumeric() && b != b'_')
        .map(|p| p + 1)
        .unwrap_or(0);

    // Find word end.
    let after = &text[offset..];
    let word_end_rel = after
        .bytes()
        .position(|b| !b.is_ascii_alphanumeric() && b != b'_')
        .unwrap_or(after.len());
    let word_end = offset + word_end_rel;

    if word_start >= word_end {
        return None;
    }

    let word = text[word_start..word_end].to_string();
    let prefix = &text[..word_start];

    Some((word, prefix))
}

/// Extract a word from the text preceding a given position.
fn extract_word_before(text: &str, pos: usize) -> Option<String> {
    if pos == 0 {
        return None;
    }
    let before = &text[..pos];
    let trimmed = before.trim_end();
    let start = trimmed
        .bytes()
        .rposition(|b| !b.is_ascii_alphanumeric() && b != b'_')
        .map(|p| p + 1)
        .unwrap_or(0);
    let word = &trimmed[start..];
    if word.is_empty() { None } else { Some(word.to_string()) }
}

/// Extract the trailing word (identifier) from a string.
fn extract_trailing_word(text: &str) -> Option<String> {
    let trimmed = text.trim_end();
    if trimmed.is_empty() {
        return None;
    }

    let start = trimmed
        .bytes()
        .rposition(|b| !b.is_ascii_alphanumeric() && b != b'_')
        .map(|p| p + 1)
        .unwrap_or(0);

    let word = &trimmed[start..];
    if word.is_empty() || word.as_bytes()[0].is_ascii_digit() {
        return None;
    }

    Some(word.to_string())
}

/// Get a brief description for a SQL keyword.
fn keyword_description(keyword: &str) -> Option<&'static str> {
    match keyword.to_ascii_uppercase().as_str() {
        "SELECT" => Some("Retrieves rows from one or more tables."),
        "FROM" => Some("Specifies the table(s) to query."),
        "WHERE" => Some("Filters rows based on a condition."),
        "AND" => Some("Logical AND — both conditions must be true."),
        "OR" => Some("Logical OR — at least one condition must be true."),
        "NOT" => Some("Logical NOT — negates a condition."),
        "IN" => Some("Tests whether a value matches any value in a list or subquery."),
        "BETWEEN" => Some("Tests whether a value is within a range (inclusive)."),
        "LIKE" => Some("Pattern matching with % and _ wildcards."),
        "IS" => Some("Tests for NULL: IS NULL, IS NOT NULL."),
        "NULL" => Some("Represents an unknown or missing value."),
        "AS" => Some("Assigns an alias to a column or table."),
        "ON" => Some("Specifies the join condition."),
        "JOIN" => Some("Combines rows from two or more tables based on a related column."),
        "INNER" => Some("Modifier for JOIN — returns only matching rows from both tables."),
        "LEFT" => Some("Modifier for JOIN — returns all rows from the left table."),
        "RIGHT" => Some("Modifier for JOIN — returns all rows from the right table."),
        "OUTER" => Some("Modifier for JOIN — includes non-matching rows."),
        "CROSS" => Some("Modifier for JOIN — produces the Cartesian product."),
        "GROUP" => Some("Used with BY — groups rows sharing a value."),
        "BY" => Some("Used with GROUP BY or ORDER BY."),
        "ORDER" => Some("Used with BY — sorts the result set."),
        "ASC" => Some("Sorts in ascending order (default)."),
        "DESC" => Some("Sorts in descending order."),
        "HAVING" => Some("Filters groups based on aggregate conditions."),
        "LIMIT" => Some("Restricts the number of rows returned."),
        "OFFSET" => Some("Skips a number of rows before returning results."),
        "UNION" => Some("Combines result sets of two SELECT statements (removes duplicates)."),
        "ALL" => Some("Used with UNION to include duplicate rows."),
        "INSERT" => Some("Adds new rows to a table."),
        "INTO" => Some("Specifies the target table for INSERT."),
        "VALUES" => Some("Specifies the values to insert."),
        "UPDATE" => Some("Modifies existing rows in a table."),
        "SET" => Some("Assigns new values to columns in UPDATE."),
        "DELETE" => Some("Removes rows from a table."),
        "CREATE" => Some("Creates a new database object (table, view, index, etc.)."),
        "TABLE" => Some("Refers to a database table."),
        "ALTER" => Some("Modifies an existing database object."),
        "DROP" => Some("Removes a database object."),
        "INDEX" => Some("A database index for faster queries."),
        "VIEW" => Some("A stored query that acts as a virtual table."),
        "EXISTS" => Some("Tests whether a subquery returns any rows."),
        "PRIMARY" => Some("Modifier for KEY — uniquely identifies each row."),
        "KEY" => Some("Defines a column constraint."),
        "FOREIGN" => Some("Modifier for KEY — references a column in another table."),
        "REFERENCES" => Some("Specifies the referenced table and column for a foreign key."),
        "UNIQUE" => Some("Ensures all values in a column are distinct."),
        "DEFAULT" => Some("Specifies a default value for a column."),
        "DISTINCT" => Some("Removes duplicate rows from the result."),
        "CASE" => Some("Begins a conditional expression (CASE WHEN ... THEN ... END)."),
        "WHEN" => Some("Specifies a condition in a CASE expression."),
        "THEN" => Some("Specifies the result when a CASE condition is true."),
        "ELSE" => Some("Specifies the default result in a CASE expression."),
        "END" => Some("Ends a CASE expression or block."),
        "WITH" => Some("Defines a Common Table Expression (CTE)."),
        "RECURSIVE" => Some("Modifier for WITH — enables recursive CTE."),
        "EXPLAIN" => Some("Shows the execution plan for a query."),
        "ANALYZE" => Some("Updates table statistics for the query optimizer."),
        _ => None,
    }
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
        assert_eq!(
            clean_placeholders("SELECT * FROM ???? WHERE id = ???"),
            "SELECT * FROM NULL WHERE id = NULL"
        );
    }

    #[test]
    fn test_sql_keywords_not_empty() {
        assert!(!sql_keyword_completions().is_empty());
    }

    #[test]
    fn test_sql_completions_select_list() {
        let sql = "SELECT ";
        let items = sql_completions(sql, sql.len() as u32, None);
        // Should include functions and keywords like *, DISTINCT.
        assert!(items.iter().any(|i| i.label == "*"));
        assert!(items.iter().any(|i| i.label == "DISTINCT"));
        assert!(items.iter().any(|i| i.kind == SqlCompletionKind::Function));
    }

    #[test]
    fn test_sql_completions_table_ref() {
        let sql = "SELECT * FROM ";
        let items = sql_completions(sql, sql.len() as u32, None);
        assert!(items.iter().any(|i| i.label == "JOIN"));
    }

    #[test]
    fn test_sql_completions_general() {
        let items = sql_completions("", 0, None);
        // Should have keywords and functions.
        assert!(items.iter().any(|i| i.kind == SqlCompletionKind::Keyword));
        assert!(items.iter().any(|i| i.kind == SqlCompletionKind::Function));
    }

    #[test]
    fn test_sql_hover_function() {
        let sql = "SELECT COUNT(*) FROM users";
        // Hover over COUNT (offset 7..12).
        let hover = sql_hover(sql, 8, None);
        assert!(hover.is_some());
        let info = hover.unwrap();
        assert!(info.content.contains("COUNT"));
    }

    #[test]
    fn test_sql_hover_keyword() {
        let sql = "SELECT * FROM users";
        // Hover over SELECT (offset 0..6).
        let hover = sql_hover(sql, 2, None);
        assert!(hover.is_some());
        let info = hover.unwrap();
        assert!(info.content.contains("SELECT"));
    }

    #[test]
    fn test_sql_hover_no_match() {
        let sql = "SELECT * FROM users";
        // Hover over * (offset 7) — not a keyword/function/table.
        let hover = sql_hover(sql, 7, None);
        // * is not a known keyword in our description map, so None.
        assert!(hover.is_none());
    }

    #[test]
    fn test_sql_signature_help() {
        let sql = "SELECT COUNT(";
        let sig = sql_signature_help(sql, sql.len() as u32);
        assert!(sig.is_some());
        let info = sig.unwrap();
        assert_eq!(info.function_name, "COUNT");
        assert_eq!(info.active_parameter, 0);
    }

    #[test]
    fn test_sql_signature_help_second_param() {
        let sql = "SELECT SUBSTRING(col, ";
        let sig = sql_signature_help(sql, sql.len() as u32);
        assert!(sig.is_some());
        let info = sig.unwrap();
        assert_eq!(info.function_name, "SUBSTRING");
        assert_eq!(info.active_parameter, 1);
    }

    #[test]
    fn test_sql_signature_help_no_function() {
        let sql = "SELECT * FROM users";
        let sig = sql_signature_help(sql, sql.len() as u32);
        assert!(sig.is_none());
    }

    #[test]
    fn test_sql_completions_with_schema() {
        use schema::{ColumnSchema, SqlSchema, TableSchema};
        use std::collections::HashMap;

        let mut columns = HashMap::new();
        columns.insert(
            "id".to_string(),
            ColumnSchema {
                data_type: "INT".to_string(),
                nullable: false,
                primary: true,
                auto_increment: true,
                unique: false,
                foreign_key: None,
                description: None,
            },
        );
        columns.insert(
            "name".to_string(),
            ColumnSchema {
                data_type: "VARCHAR(255)".to_string(),
                nullable: false,
                primary: false,
                auto_increment: false,
                unique: false,
                foreign_key: None,
                description: None,
            },
        );
        let mut tables = HashMap::new();
        tables.insert(
            "users".to_string(),
            TableSchema {
                columns,
                description: Some("User accounts".to_string()),
            },
        );
        let schema = SqlSchema { tables };

        // SELECT from users — should include user columns.
        let sql = "SELECT * FROM users WHERE ";
        let items = sql_completions(sql, sql.len() as u32, Some(&schema));
        assert!(items.iter().any(|i| i.label == "id" && i.kind == SqlCompletionKind::Column));
        assert!(items.iter().any(|i| i.label == "name" && i.kind == SqlCompletionKind::Column));
    }

    #[test]
    fn test_sql_hover_table_in_schema() {
        use schema::{ColumnSchema, SqlSchema, TableSchema};
        use std::collections::HashMap;

        let mut columns = HashMap::new();
        columns.insert(
            "id".to_string(),
            ColumnSchema {
                data_type: "INT".to_string(),
                nullable: false,
                primary: true,
                auto_increment: true,
                unique: false,
                foreign_key: None,
                description: None,
            },
        );
        let mut tables = HashMap::new();
        tables.insert(
            "users".to_string(),
            TableSchema {
                columns,
                description: Some("User accounts".to_string()),
            },
        );
        let schema = SqlSchema { tables };

        let sql = "SELECT * FROM users";
        // Hover over "users" at offset 14..19.
        let hover = sql_hover(sql, 15, Some(&schema));
        assert!(hover.is_some());
        let info = hover.unwrap();
        assert!(info.content.contains("Table: users"));
        assert!(info.content.contains("User accounts"));
    }
}
