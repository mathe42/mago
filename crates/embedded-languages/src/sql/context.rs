//! SQL context detection using text-based heuristics.
//!
//! Since SQL strings in PHP are often incomplete while the user is typing,
//! we use simple text scanning rather than relying on a full AST parse.

/// The detected context at the cursor position within a SQL string.
#[derive(Debug, PartialEq, Eq)]
pub enum SqlContext {
    /// After SELECT and before FROM — expecting column expressions.
    SelectList,
    /// After FROM, JOIN, or similar — expecting a table reference.
    TableRef,
    /// After WHERE, AND, OR, ON, HAVING — expecting a condition expression.
    Condition,
    /// After SET — expecting column = value assignments.
    SetClause,
    /// Inside INSERT INTO table (...) — expecting column names.
    InsertColumns,
    /// Inside VALUES (...) — expecting values.
    InsertValues,
    /// After ORDER BY or GROUP BY — expecting columns.
    OrderBy,
    /// After `table.` — expecting a column of the specified table.
    ColumnOfTable(String),
    /// Inside function call parens like `COUNT(` — expecting function arguments.
    FunctionArgs(String),
    /// No specific context detected — offer general completions.
    General,
}

/// Detect the SQL context at the given byte offset within a virtual document.
///
/// This uses heuristic text scanning: it looks at the text before the cursor,
/// identifies the most recent significant SQL keyword, and determines what kind
/// of token the user is most likely trying to type next.
pub fn detect_sql_context(virtual_document: &str, offset: u32) -> SqlContext {
    let offset = (offset as usize).min(virtual_document.len());
    let before_cursor = &virtual_document[..offset];

    // Check for `table.` pattern first — if the character immediately before
    // the current word is a dot, we're completing a column of a table.
    if let Some(table) = detect_table_dot(before_cursor) {
        return SqlContext::ColumnOfTable(table);
    }

    // Check if we are inside function argument parens.
    if let Some(func_name) = detect_function_args(before_cursor) {
        return SqlContext::FunctionArgs(func_name);
    }

    // Check if we're inside INSERT INTO ... (...) or VALUES (...).
    if let Some(ctx) = detect_insert_context(before_cursor) {
        return ctx;
    }

    // Find the last significant SQL keyword.
    match find_last_keyword(before_cursor) {
        Some(kw) => match kw.as_str() {
            "SELECT" => {
                // SELECT ... but no FROM yet → selecting columns/expressions
                if !has_keyword_after(before_cursor, &kw_pos_end(before_cursor, "SELECT"), "FROM") {
                    SqlContext::SelectList
                } else {
                    SqlContext::General
                }
            }
            "FROM" | "JOIN" | "INNER JOIN" | "LEFT JOIN" | "RIGHT JOIN" | "OUTER JOIN"
            | "LEFT OUTER JOIN" | "RIGHT OUTER JOIN" | "CROSS JOIN" | "NATURAL JOIN" => {
                SqlContext::TableRef
            }
            "WHERE" | "AND" | "OR" | "ON" | "HAVING" => SqlContext::Condition,
            "SET" => SqlContext::SetClause,
            "ORDER BY" | "GROUP BY" => SqlContext::OrderBy,
            _ => SqlContext::General,
        },
        None => SqlContext::General,
    }
}

/// Extract table names referenced in the SQL string.
///
/// Finds names after FROM, JOIN variants, UPDATE, and INSERT INTO
/// using simple text scanning.
pub fn extract_table_names(virtual_document: &str) -> Vec<String> {
    let upper = virtual_document.to_ascii_uppercase();
    let mut tables = Vec::new();

    // Patterns to search for: keyword followed by a table name.
    let keyword_patterns = [
        "FROM ",
        "JOIN ",
        "UPDATE ",
        "INSERT INTO ",
        "INNER JOIN ",
        "LEFT JOIN ",
        "RIGHT JOIN ",
        "OUTER JOIN ",
        "LEFT OUTER JOIN ",
        "RIGHT OUTER JOIN ",
        "CROSS JOIN ",
        "NATURAL JOIN ",
    ];

    for pattern in &keyword_patterns {
        let mut search_start = 0;
        while let Some(pos) = upper[search_start..].find(pattern) {
            let abs_pos = search_start + pos + pattern.len();
            if let Some(name) = extract_identifier_at(virtual_document, abs_pos) {
                if !is_sql_keyword(&name) && !tables.iter().any(|t: &String| t.eq_ignore_ascii_case(&name)) {
                    tables.push(name);
                }
            }
            search_start = abs_pos;
        }
    }

    tables
}

// ── Internal helpers ─────────────────────────────────────────────────────

/// Checks if the cursor is immediately after `identifier.` and returns the
/// identifier (table name) before the dot.
fn detect_table_dot(before_cursor: &str) -> Option<String> {
    let trimmed = before_cursor.trim_end();
    if !trimmed.ends_with('.') {
        return None;
    }

    // Get the identifier before the dot.
    let before_dot = &trimmed[..trimmed.len() - 1];
    let ident = extract_trailing_identifier(before_dot)?;

    // Make sure it's not a SQL keyword being used weirdly.
    if is_sql_keyword(&ident) {
        return None;
    }

    Some(ident)
}

/// Checks if the cursor is inside function call parens, e.g. `COUNT(`.
/// Walks backwards to find an unmatched `(` and then checks if there is
/// an identifier (function name) immediately before it.
fn detect_function_args(before_cursor: &str) -> Option<String> {
    let mut depth = 0i32;
    for (i, ch) in before_cursor.char_indices().rev() {
        match ch {
            ')' => depth += 1,
            '(' => {
                if depth == 0 {
                    // Found unmatched open paren. Look for function name before it.
                    let before_paren = &before_cursor[..i];
                    let func_name = extract_trailing_identifier(before_paren)?;

                    // Verify it looks like a function, not a keyword like WHERE(.
                    if is_sql_keyword(&func_name) && !is_function_keyword(&func_name) {
                        return None;
                    }

                    return Some(func_name.to_ascii_uppercase());
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

/// Detect if the cursor is inside an INSERT context.
fn detect_insert_context(before_cursor: &str) -> Option<SqlContext> {
    let upper = before_cursor.to_ascii_uppercase();

    // Check for VALUES (...
    if let Some(values_pos) = upper.rfind("VALUES") {
        let after_values = &before_cursor[values_pos + 6..];
        let trimmed = after_values.trim_start();
        if trimmed.starts_with('(') {
            // Check for unmatched open paren after VALUES.
            let mut depth = 0i32;
            for ch in trimmed.chars() {
                match ch {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ => {}
                }
            }
            if depth > 0 {
                return Some(SqlContext::InsertValues);
            }
        }
    }

    // Check for INSERT INTO table_name (...
    if let Some(insert_pos) = upper.rfind("INSERT INTO ") {
        let after_insert = &before_cursor[insert_pos + 12..];
        // Skip the table name.
        let after_table = skip_identifier(after_insert);
        let trimmed = after_table.trim_start();
        if trimmed.starts_with('(') {
            // We haven't reached VALUES yet.
            let after_paren = &trimmed[1..];
            if !after_paren.to_ascii_uppercase().contains("VALUES") {
                // Check for unmatched open paren.
                let mut depth = 1i32;
                for ch in after_paren.chars() {
                    match ch {
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        _ => {}
                    }
                }
                if depth > 0 {
                    return Some(SqlContext::InsertColumns);
                }
            }
        }
    }

    None
}

/// Compound keywords we track (order matters: check longer ones first).
const COMPOUND_KEYWORDS: &[&str] = &[
    "LEFT OUTER JOIN",
    "RIGHT OUTER JOIN",
    "INNER JOIN",
    "LEFT JOIN",
    "RIGHT JOIN",
    "OUTER JOIN",
    "CROSS JOIN",
    "NATURAL JOIN",
    "ORDER BY",
    "GROUP BY",
    "INSERT INTO",
];

/// Simple keywords we track.
const SIMPLE_KEYWORDS: &[&str] = &["SELECT", "FROM", "JOIN", "WHERE", "AND", "OR", "ON", "HAVING", "SET"];

/// Find the last significant SQL keyword in the text before cursor.
/// Returns the keyword in uppercase.
fn find_last_keyword(before_cursor: &str) -> Option<String> {
    let upper = before_cursor.to_ascii_uppercase();
    let mut best: Option<(usize, String)> = None;

    // Check compound keywords first.
    for &kw in COMPOUND_KEYWORDS {
        if let Some(pos) = upper.rfind(kw) {
            // Make sure it's at a word boundary.
            if is_keyword_at_boundary(&upper, pos, kw.len()) {
                if best.as_ref().is_none_or(|(best_pos, _)| pos > *best_pos) {
                    best = Some((pos, kw.to_string()));
                }
            }
        }
    }

    // Check simple keywords.
    for &kw in SIMPLE_KEYWORDS {
        if let Some(pos) = upper.rfind(kw) {
            if is_keyword_at_boundary(&upper, pos, kw.len()) {
                if best.as_ref().is_none_or(|(best_pos, _)| pos > *best_pos) {
                    best = Some((pos, kw.to_string()));
                }
            }
        }
    }

    best.map(|(_, kw)| kw)
}

/// Check if a keyword at the given position is at word boundaries
/// (not in the middle of an identifier).
fn is_keyword_at_boundary(text: &str, pos: usize, len: usize) -> bool {
    let bytes = text.as_bytes();
    // Check before
    if pos > 0 {
        let before = bytes[pos - 1];
        if before.is_ascii_alphanumeric() || before == b'_' {
            return false;
        }
    }
    // Check after
    let end = pos + len;
    if end < bytes.len() {
        let after = bytes[end];
        if after.is_ascii_alphanumeric() || after == b'_' {
            return false;
        }
    }
    true
}

/// Check if a specific keyword appears after a given position.
fn has_keyword_after(text: &str, start_pos: &usize, keyword: &str) -> bool {
    let upper = text[*start_pos..].to_ascii_uppercase();
    if let Some(pos) = upper.find(keyword) {
        is_keyword_at_boundary(&upper, pos, keyword.len())
    } else {
        false
    }
}

/// Find the end position of the last occurrence of a keyword.
fn kw_pos_end(text: &str, keyword: &str) -> usize {
    let upper = text.to_ascii_uppercase();
    upper.rfind(keyword).map(|pos| pos + keyword.len()).unwrap_or(0)
}

/// Extract the trailing SQL identifier (unquoted) from the given text.
fn extract_trailing_identifier(text: &str) -> Option<String> {
    let trimmed = text.trim_end();
    if trimmed.is_empty() {
        return None;
    }

    // Handle backtick-quoted identifiers.
    if trimmed.ends_with('`') {
        let inner = &trimmed[..trimmed.len() - 1];
        if let Some(start) = inner.rfind('`') {
            return Some(inner[start + 1..].to_string());
        }
        return None;
    }

    // Regular unquoted identifier: [a-zA-Z0-9_]
    let start = trimmed
        .bytes()
        .rposition(|b| !b.is_ascii_alphanumeric() && b != b'_')
        .map(|p| p + 1)
        .unwrap_or(0);

    let ident = &trimmed[start..];
    if ident.is_empty() || ident.as_bytes()[0].is_ascii_digit() {
        return None;
    }

    Some(ident.to_string())
}

/// Extract an identifier starting at the given byte position.
fn extract_identifier_at(text: &str, pos: usize) -> Option<String> {
    if pos >= text.len() {
        return None;
    }

    let remaining = &text[pos..];
    let trimmed = remaining.trim_start();

    // Handle backtick-quoted identifiers.
    if trimmed.starts_with('`') {
        let inner = &trimmed[1..];
        if let Some(end) = inner.find('`') {
            return Some(inner[..end].to_string());
        }
        return None;
    }

    // Regular identifier.
    let end = trimmed
        .bytes()
        .position(|b| !b.is_ascii_alphanumeric() && b != b'_')
        .unwrap_or(trimmed.len());

    let ident = &trimmed[..end];
    if ident.is_empty() || ident.as_bytes()[0].is_ascii_digit() {
        return None;
    }

    Some(ident.to_string())
}

/// Skip past an identifier and return the remaining text.
fn skip_identifier(text: &str) -> &str {
    let trimmed = text.trim_start();

    if trimmed.starts_with('`') {
        let inner = &trimmed[1..];
        if let Some(end) = inner.find('`') {
            return &inner[end + 1..];
        }
        return "";
    }

    let end = trimmed
        .bytes()
        .position(|b| !b.is_ascii_alphanumeric() && b != b'_')
        .unwrap_or(trimmed.len());

    &trimmed[end..]
}

/// Check if a name is a SQL keyword (and therefore not a table/column name).
fn is_sql_keyword(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "SELECT"
            | "FROM"
            | "WHERE"
            | "AND"
            | "OR"
            | "NOT"
            | "IN"
            | "BETWEEN"
            | "LIKE"
            | "IS"
            | "NULL"
            | "AS"
            | "ON"
            | "JOIN"
            | "INNER"
            | "LEFT"
            | "RIGHT"
            | "OUTER"
            | "CROSS"
            | "GROUP"
            | "BY"
            | "ORDER"
            | "ASC"
            | "DESC"
            | "HAVING"
            | "LIMIT"
            | "OFFSET"
            | "UNION"
            | "ALL"
            | "INSERT"
            | "INTO"
            | "VALUES"
            | "UPDATE"
            | "SET"
            | "DELETE"
            | "CREATE"
            | "TABLE"
            | "ALTER"
            | "DROP"
            | "INDEX"
            | "VIEW"
            | "EXISTS"
            | "PRIMARY"
            | "KEY"
            | "FOREIGN"
            | "REFERENCES"
            | "UNIQUE"
            | "DEFAULT"
            | "CASCADE"
            | "DISTINCT"
            | "CASE"
            | "WHEN"
            | "THEN"
            | "ELSE"
            | "END"
            | "WITH"
            | "RECURSIVE"
            | "EXPLAIN"
            | "ANALYZE"
    )
}

/// Keywords that are also function names (e.g. IF, COALESCE).
fn is_function_keyword(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "IF" | "COALESCE" | "CAST" | "CONVERT" | "COUNT" | "SUM" | "AVG" | "MIN" | "MAX"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_select_list_context() {
        let sql = "SELECT ";
        assert_eq!(detect_sql_context(sql, sql.len() as u32), SqlContext::SelectList);
    }

    #[test]
    fn test_select_list_with_partial() {
        let sql = "SELECT id, na";
        assert_eq!(detect_sql_context(sql, sql.len() as u32), SqlContext::SelectList);
    }

    #[test]
    fn test_table_ref_after_from() {
        let sql = "SELECT * FROM ";
        assert_eq!(detect_sql_context(sql, sql.len() as u32), SqlContext::TableRef);
    }

    #[test]
    fn test_table_ref_after_join() {
        let sql = "SELECT * FROM users JOIN ";
        assert_eq!(detect_sql_context(sql, sql.len() as u32), SqlContext::TableRef);
    }

    #[test]
    fn test_table_ref_after_left_join() {
        let sql = "SELECT * FROM users LEFT JOIN ";
        assert_eq!(detect_sql_context(sql, sql.len() as u32), SqlContext::TableRef);
    }

    #[test]
    fn test_condition_after_where() {
        let sql = "SELECT * FROM users WHERE ";
        assert_eq!(detect_sql_context(sql, sql.len() as u32), SqlContext::Condition);
    }

    #[test]
    fn test_condition_after_and() {
        let sql = "SELECT * FROM users WHERE id = 1 AND ";
        assert_eq!(detect_sql_context(sql, sql.len() as u32), SqlContext::Condition);
    }

    #[test]
    fn test_set_clause() {
        let sql = "UPDATE users SET ";
        assert_eq!(detect_sql_context(sql, sql.len() as u32), SqlContext::SetClause);
    }

    #[test]
    fn test_order_by() {
        let sql = "SELECT * FROM users ORDER BY ";
        assert_eq!(detect_sql_context(sql, sql.len() as u32), SqlContext::OrderBy);
    }

    #[test]
    fn test_group_by() {
        let sql = "SELECT * FROM users GROUP BY ";
        assert_eq!(detect_sql_context(sql, sql.len() as u32), SqlContext::OrderBy);
    }

    #[test]
    fn test_column_of_table() {
        let sql = "SELECT users.";
        assert_eq!(
            detect_sql_context(sql, sql.len() as u32),
            SqlContext::ColumnOfTable("users".to_string())
        );
    }

    #[test]
    fn test_function_args() {
        let sql = "SELECT COUNT(";
        assert_eq!(
            detect_sql_context(sql, sql.len() as u32),
            SqlContext::FunctionArgs("COUNT".to_string())
        );
    }

    #[test]
    fn test_insert_columns() {
        let sql = "INSERT INTO users (";
        assert_eq!(detect_sql_context(sql, sql.len() as u32), SqlContext::InsertColumns);
    }

    #[test]
    fn test_insert_values() {
        let sql = "INSERT INTO users (id, name) VALUES (";
        assert_eq!(detect_sql_context(sql, sql.len() as u32), SqlContext::InsertValues);
    }

    #[test]
    fn test_general_context() {
        let sql = "";
        assert_eq!(detect_sql_context(sql, sql.len() as u32), SqlContext::General);
    }

    #[test]
    fn test_extract_table_names_simple() {
        let sql = "SELECT * FROM users WHERE id = 1";
        let tables = extract_table_names(sql);
        assert_eq!(tables, vec!["users"]);
    }

    #[test]
    fn test_extract_table_names_join() {
        let sql = "SELECT * FROM users JOIN orders ON users.id = orders.user_id";
        let tables = extract_table_names(sql);
        assert!(tables.contains(&"users".to_string()));
        assert!(tables.contains(&"orders".to_string()));
    }

    #[test]
    fn test_extract_table_names_update() {
        let sql = "UPDATE users SET name = 'test'";
        let tables = extract_table_names(sql);
        assert_eq!(tables, vec!["users"]);
    }

    #[test]
    fn test_extract_table_names_insert() {
        let sql = "INSERT INTO users (name) VALUES ('test')";
        let tables = extract_table_names(sql);
        assert_eq!(tables, vec!["users"]);
    }

    #[test]
    fn test_extract_table_names_no_duplicates() {
        let sql = "SELECT * FROM users JOIN users ON 1=1";
        let tables = extract_table_names(sql);
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0], "users");
    }

    #[test]
    fn test_condition_after_on() {
        let sql = "SELECT * FROM users JOIN orders ON ";
        assert_eq!(detect_sql_context(sql, sql.len() as u32), SqlContext::Condition);
    }

    #[test]
    fn test_having_context() {
        let sql = "SELECT COUNT(*) FROM users GROUP BY status HAVING ";
        assert_eq!(detect_sql_context(sql, sql.len() as u32), SqlContext::Condition);
    }
}
