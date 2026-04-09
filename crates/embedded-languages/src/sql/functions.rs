//! Static database of common MySQL/SQL functions with signatures and descriptions.

use std::sync::LazyLock;

/// Describes a SQL function with its signature, parameters, and return type.
#[derive(Debug, Clone)]
pub struct SqlFunction {
    pub name: &'static str,
    pub signature: &'static str,
    pub description: &'static str,
    pub parameters: &'static [SqlFunctionParam],
    pub return_type: &'static str,
}

/// A parameter of a SQL function.
#[derive(Debug, Clone)]
pub struct SqlFunctionParam {
    pub name: &'static str,
    pub description: &'static str,
}

static ALL_FUNCTIONS: LazyLock<Vec<SqlFunction>> = LazyLock::new(|| {
    vec![
        // ── Aggregate functions ──────────────────────────────────────
        SqlFunction {
            name: "COUNT",
            signature: "COUNT(expr)",
            description: "Returns the number of rows that match a specified criterion.",
            parameters: &[SqlFunctionParam {
                name: "expr",
                description: "Expression to count. Use * to count all rows.",
            }],
            return_type: "BIGINT",
        },
        SqlFunction {
            name: "SUM",
            signature: "SUM(expr)",
            description: "Returns the total sum of a numeric column.",
            parameters: &[SqlFunctionParam {
                name: "expr",
                description: "Numeric expression to sum.",
            }],
            return_type: "DECIMAL",
        },
        SqlFunction {
            name: "AVG",
            signature: "AVG(expr)",
            description: "Returns the average value of a numeric column.",
            parameters: &[SqlFunctionParam {
                name: "expr",
                description: "Numeric expression to average.",
            }],
            return_type: "DECIMAL",
        },
        SqlFunction {
            name: "MIN",
            signature: "MIN(expr)",
            description: "Returns the minimum value in a set of values.",
            parameters: &[SqlFunctionParam {
                name: "expr",
                description: "Expression to find the minimum of.",
            }],
            return_type: "varies",
        },
        SqlFunction {
            name: "MAX",
            signature: "MAX(expr)",
            description: "Returns the maximum value in a set of values.",
            parameters: &[SqlFunctionParam {
                name: "expr",
                description: "Expression to find the maximum of.",
            }],
            return_type: "varies",
        },
        SqlFunction {
            name: "GROUP_CONCAT",
            signature: "GROUP_CONCAT(expr [ORDER BY ...] [SEPARATOR str])",
            description: "Concatenates values from a group into a single string.",
            parameters: &[
                SqlFunctionParam {
                    name: "expr",
                    description: "Expression whose values are concatenated.",
                },
                SqlFunctionParam {
                    name: "SEPARATOR",
                    description: "String to insert between values (default is comma).",
                },
            ],
            return_type: "VARCHAR",
        },
        // ── String functions ─────────────────────────────────────────
        SqlFunction {
            name: "CONCAT",
            signature: "CONCAT(str1, str2, ...)",
            description: "Concatenates two or more strings.",
            parameters: &[
                SqlFunctionParam {
                    name: "str1",
                    description: "First string.",
                },
                SqlFunctionParam {
                    name: "str2",
                    description: "Second string.",
                },
            ],
            return_type: "VARCHAR",
        },
        SqlFunction {
            name: "SUBSTRING",
            signature: "SUBSTRING(str, pos [, len])",
            description: "Extracts a substring from a string starting at a given position.",
            parameters: &[
                SqlFunctionParam {
                    name: "str",
                    description: "Input string.",
                },
                SqlFunctionParam {
                    name: "pos",
                    description: "Starting position (1-based).",
                },
                SqlFunctionParam {
                    name: "len",
                    description: "Number of characters to extract (optional).",
                },
            ],
            return_type: "VARCHAR",
        },
        SqlFunction {
            name: "REPLACE",
            signature: "REPLACE(str, from_str, to_str)",
            description: "Replaces all occurrences of a substring within a string.",
            parameters: &[
                SqlFunctionParam {
                    name: "str",
                    description: "Input string.",
                },
                SqlFunctionParam {
                    name: "from_str",
                    description: "Substring to find.",
                },
                SqlFunctionParam {
                    name: "to_str",
                    description: "Replacement string.",
                },
            ],
            return_type: "VARCHAR",
        },
        SqlFunction {
            name: "TRIM",
            signature: "TRIM([{BOTH | LEADING | TRAILING} [remstr] FROM] str)",
            description: "Removes leading and trailing whitespace (or specified characters) from a string.",
            parameters: &[SqlFunctionParam {
                name: "str",
                description: "Input string to trim.",
            }],
            return_type: "VARCHAR",
        },
        SqlFunction {
            name: "UPPER",
            signature: "UPPER(str)",
            description: "Converts a string to uppercase.",
            parameters: &[SqlFunctionParam {
                name: "str",
                description: "Input string.",
            }],
            return_type: "VARCHAR",
        },
        SqlFunction {
            name: "LOWER",
            signature: "LOWER(str)",
            description: "Converts a string to lowercase.",
            parameters: &[SqlFunctionParam {
                name: "str",
                description: "Input string.",
            }],
            return_type: "VARCHAR",
        },
        SqlFunction {
            name: "LENGTH",
            signature: "LENGTH(str)",
            description: "Returns the length of a string in bytes.",
            parameters: &[SqlFunctionParam {
                name: "str",
                description: "Input string.",
            }],
            return_type: "INT",
        },
        SqlFunction {
            name: "LEFT",
            signature: "LEFT(str, len)",
            description: "Returns the leftmost len characters from a string.",
            parameters: &[
                SqlFunctionParam {
                    name: "str",
                    description: "Input string.",
                },
                SqlFunctionParam {
                    name: "len",
                    description: "Number of characters to return.",
                },
            ],
            return_type: "VARCHAR",
        },
        SqlFunction {
            name: "RIGHT",
            signature: "RIGHT(str, len)",
            description: "Returns the rightmost len characters from a string.",
            parameters: &[
                SqlFunctionParam {
                    name: "str",
                    description: "Input string.",
                },
                SqlFunctionParam {
                    name: "len",
                    description: "Number of characters to return.",
                },
            ],
            return_type: "VARCHAR",
        },
        SqlFunction {
            name: "LPAD",
            signature: "LPAD(str, len, padstr)",
            description: "Left-pads a string with another string to a specified length.",
            parameters: &[
                SqlFunctionParam {
                    name: "str",
                    description: "Input string.",
                },
                SqlFunctionParam {
                    name: "len",
                    description: "Desired length after padding.",
                },
                SqlFunctionParam {
                    name: "padstr",
                    description: "String to pad with.",
                },
            ],
            return_type: "VARCHAR",
        },
        SqlFunction {
            name: "RPAD",
            signature: "RPAD(str, len, padstr)",
            description: "Right-pads a string with another string to a specified length.",
            parameters: &[
                SqlFunctionParam {
                    name: "str",
                    description: "Input string.",
                },
                SqlFunctionParam {
                    name: "len",
                    description: "Desired length after padding.",
                },
                SqlFunctionParam {
                    name: "padstr",
                    description: "String to pad with.",
                },
            ],
            return_type: "VARCHAR",
        },
        // ── Date/Time functions ──────────────────────────────────────
        SqlFunction {
            name: "NOW",
            signature: "NOW()",
            description: "Returns the current date and time.",
            parameters: &[],
            return_type: "DATETIME",
        },
        SqlFunction {
            name: "CURDATE",
            signature: "CURDATE()",
            description: "Returns the current date.",
            parameters: &[],
            return_type: "DATE",
        },
        SqlFunction {
            name: "CURTIME",
            signature: "CURTIME()",
            description: "Returns the current time.",
            parameters: &[],
            return_type: "TIME",
        },
        SqlFunction {
            name: "DATE_FORMAT",
            signature: "DATE_FORMAT(date, format)",
            description: "Formats a date according to a format string.",
            parameters: &[
                SqlFunctionParam {
                    name: "date",
                    description: "Date value to format.",
                },
                SqlFunctionParam {
                    name: "format",
                    description: "Format string (e.g. '%Y-%m-%d').",
                },
            ],
            return_type: "VARCHAR",
        },
        SqlFunction {
            name: "DATEDIFF",
            signature: "DATEDIFF(date1, date2)",
            description: "Returns the number of days between two dates.",
            parameters: &[
                SqlFunctionParam {
                    name: "date1",
                    description: "First date.",
                },
                SqlFunctionParam {
                    name: "date2",
                    description: "Second date.",
                },
            ],
            return_type: "INT",
        },
        SqlFunction {
            name: "DATE_ADD",
            signature: "DATE_ADD(date, INTERVAL expr unit)",
            description: "Adds a time interval to a date.",
            parameters: &[
                SqlFunctionParam {
                    name: "date",
                    description: "Starting date.",
                },
                SqlFunctionParam {
                    name: "expr",
                    description: "Interval expression (e.g. INTERVAL 1 DAY).",
                },
            ],
            return_type: "DATETIME",
        },
        SqlFunction {
            name: "DATE_SUB",
            signature: "DATE_SUB(date, INTERVAL expr unit)",
            description: "Subtracts a time interval from a date.",
            parameters: &[
                SqlFunctionParam {
                    name: "date",
                    description: "Starting date.",
                },
                SqlFunctionParam {
                    name: "expr",
                    description: "Interval expression (e.g. INTERVAL 1 DAY).",
                },
            ],
            return_type: "DATETIME",
        },
        SqlFunction {
            name: "YEAR",
            signature: "YEAR(date)",
            description: "Extracts the year from a date.",
            parameters: &[SqlFunctionParam {
                name: "date",
                description: "Date value.",
            }],
            return_type: "INT",
        },
        SqlFunction {
            name: "MONTH",
            signature: "MONTH(date)",
            description: "Extracts the month from a date (1-12).",
            parameters: &[SqlFunctionParam {
                name: "date",
                description: "Date value.",
            }],
            return_type: "INT",
        },
        SqlFunction {
            name: "DAY",
            signature: "DAY(date)",
            description: "Extracts the day of the month from a date (1-31).",
            parameters: &[SqlFunctionParam {
                name: "date",
                description: "Date value.",
            }],
            return_type: "INT",
        },
        SqlFunction {
            name: "HOUR",
            signature: "HOUR(time)",
            description: "Extracts the hour from a time or datetime (0-23).",
            parameters: &[SqlFunctionParam {
                name: "time",
                description: "Time or datetime value.",
            }],
            return_type: "INT",
        },
        SqlFunction {
            name: "MINUTE",
            signature: "MINUTE(time)",
            description: "Extracts the minute from a time or datetime (0-59).",
            parameters: &[SqlFunctionParam {
                name: "time",
                description: "Time or datetime value.",
            }],
            return_type: "INT",
        },
        SqlFunction {
            name: "SECOND",
            signature: "SECOND(time)",
            description: "Extracts the second from a time or datetime (0-59).",
            parameters: &[SqlFunctionParam {
                name: "time",
                description: "Time or datetime value.",
            }],
            return_type: "INT",
        },
        SqlFunction {
            name: "UNIX_TIMESTAMP",
            signature: "UNIX_TIMESTAMP([date])",
            description: "Returns a Unix timestamp (seconds since 1970-01-01 00:00:00 UTC).",
            parameters: &[SqlFunctionParam {
                name: "date",
                description: "Optional date. If omitted, returns current timestamp.",
            }],
            return_type: "INT",
        },
        SqlFunction {
            name: "FROM_UNIXTIME",
            signature: "FROM_UNIXTIME(unix_timestamp [, format])",
            description: "Converts a Unix timestamp to a datetime.",
            parameters: &[
                SqlFunctionParam {
                    name: "unix_timestamp",
                    description: "Unix timestamp value.",
                },
                SqlFunctionParam {
                    name: "format",
                    description: "Optional format string.",
                },
            ],
            return_type: "DATETIME",
        },
        // ── Control flow functions ───────────────────────────────────
        SqlFunction {
            name: "IF",
            signature: "IF(condition, value_if_true, value_if_false)",
            description: "Returns one value if a condition is true, another if false.",
            parameters: &[
                SqlFunctionParam {
                    name: "condition",
                    description: "Boolean expression.",
                },
                SqlFunctionParam {
                    name: "value_if_true",
                    description: "Value returned when condition is true.",
                },
                SqlFunctionParam {
                    name: "value_if_false",
                    description: "Value returned when condition is false.",
                },
            ],
            return_type: "varies",
        },
        SqlFunction {
            name: "IFNULL",
            signature: "IFNULL(expr, alt_value)",
            description: "Returns the first argument if it is not NULL, otherwise returns the second.",
            parameters: &[
                SqlFunctionParam {
                    name: "expr",
                    description: "Expression to test for NULL.",
                },
                SqlFunctionParam {
                    name: "alt_value",
                    description: "Value returned if expr is NULL.",
                },
            ],
            return_type: "varies",
        },
        SqlFunction {
            name: "COALESCE",
            signature: "COALESCE(value1, value2, ...)",
            description: "Returns the first non-NULL argument.",
            parameters: &[
                SqlFunctionParam {
                    name: "value1",
                    description: "First value to check.",
                },
                SqlFunctionParam {
                    name: "value2",
                    description: "Second value to check.",
                },
            ],
            return_type: "varies",
        },
        SqlFunction {
            name: "NULLIF",
            signature: "NULLIF(expr1, expr2)",
            description: "Returns NULL if expr1 equals expr2, otherwise returns expr1.",
            parameters: &[
                SqlFunctionParam {
                    name: "expr1",
                    description: "First expression.",
                },
                SqlFunctionParam {
                    name: "expr2",
                    description: "Second expression to compare against.",
                },
            ],
            return_type: "varies",
        },
        SqlFunction {
            name: "GREATEST",
            signature: "GREATEST(value1, value2, ...)",
            description: "Returns the greatest (maximum) value from a list of arguments.",
            parameters: &[
                SqlFunctionParam {
                    name: "value1",
                    description: "First value.",
                },
                SqlFunctionParam {
                    name: "value2",
                    description: "Second value.",
                },
            ],
            return_type: "varies",
        },
        SqlFunction {
            name: "LEAST",
            signature: "LEAST(value1, value2, ...)",
            description: "Returns the smallest (minimum) value from a list of arguments.",
            parameters: &[
                SqlFunctionParam {
                    name: "value1",
                    description: "First value.",
                },
                SqlFunctionParam {
                    name: "value2",
                    description: "Second value.",
                },
            ],
            return_type: "varies",
        },
        // ── Math functions ───────────────────────────────────────────
        SqlFunction {
            name: "ABS",
            signature: "ABS(number)",
            description: "Returns the absolute value of a number.",
            parameters: &[SqlFunctionParam {
                name: "number",
                description: "Numeric expression.",
            }],
            return_type: "DECIMAL",
        },
        SqlFunction {
            name: "CEIL",
            signature: "CEIL(number)",
            description: "Returns the smallest integer value not less than the argument.",
            parameters: &[SqlFunctionParam {
                name: "number",
                description: "Numeric expression.",
            }],
            return_type: "INT",
        },
        SqlFunction {
            name: "FLOOR",
            signature: "FLOOR(number)",
            description: "Returns the largest integer value not greater than the argument.",
            parameters: &[SqlFunctionParam {
                name: "number",
                description: "Numeric expression.",
            }],
            return_type: "INT",
        },
        SqlFunction {
            name: "ROUND",
            signature: "ROUND(number [, decimals])",
            description: "Rounds a number to a specified number of decimal places.",
            parameters: &[
                SqlFunctionParam {
                    name: "number",
                    description: "Numeric expression to round.",
                },
                SqlFunctionParam {
                    name: "decimals",
                    description: "Number of decimal places (default 0).",
                },
            ],
            return_type: "DECIMAL",
        },
        SqlFunction {
            name: "MOD",
            signature: "MOD(dividend, divisor)",
            description: "Returns the remainder of a division.",
            parameters: &[
                SqlFunctionParam {
                    name: "dividend",
                    description: "Number to be divided.",
                },
                SqlFunctionParam {
                    name: "divisor",
                    description: "Number to divide by.",
                },
            ],
            return_type: "DECIMAL",
        },
        SqlFunction {
            name: "RAND",
            signature: "RAND([seed])",
            description: "Returns a random floating-point value between 0 and 1.",
            parameters: &[SqlFunctionParam {
                name: "seed",
                description: "Optional seed for repeatable sequences.",
            }],
            return_type: "DOUBLE",
        },
        SqlFunction {
            name: "POWER",
            signature: "POWER(base, exponent)",
            description: "Returns the value of a number raised to the power of another.",
            parameters: &[
                SqlFunctionParam {
                    name: "base",
                    description: "Base number.",
                },
                SqlFunctionParam {
                    name: "exponent",
                    description: "Power to raise to.",
                },
            ],
            return_type: "DOUBLE",
        },
        SqlFunction {
            name: "SQRT",
            signature: "SQRT(number)",
            description: "Returns the square root of a non-negative number.",
            parameters: &[SqlFunctionParam {
                name: "number",
                description: "Non-negative numeric expression.",
            }],
            return_type: "DOUBLE",
        },
        // ── Other functions ──────────────────────────────────────────
        SqlFunction {
            name: "CAST",
            signature: "CAST(expr AS type)",
            description: "Converts a value to a specified data type.",
            parameters: &[
                SqlFunctionParam {
                    name: "expr",
                    description: "Expression to convert.",
                },
                SqlFunctionParam {
                    name: "type",
                    description: "Target data type (e.g. CHAR, SIGNED, UNSIGNED, DATE).",
                },
            ],
            return_type: "varies",
        },
        SqlFunction {
            name: "CONVERT",
            signature: "CONVERT(expr, type) / CONVERT(expr USING charset)",
            description: "Converts a value to a specified type or character set.",
            parameters: &[
                SqlFunctionParam {
                    name: "expr",
                    description: "Expression to convert.",
                },
                SqlFunctionParam {
                    name: "type",
                    description: "Target data type or character set.",
                },
            ],
            return_type: "varies",
        },
        SqlFunction {
            name: "JSON_EXTRACT",
            signature: "JSON_EXTRACT(json_doc, path [, path] ...)",
            description: "Extracts data from a JSON document at the given path(s).",
            parameters: &[
                SqlFunctionParam {
                    name: "json_doc",
                    description: "JSON document.",
                },
                SqlFunctionParam {
                    name: "path",
                    description: "JSON path expression (e.g. '$.key').",
                },
            ],
            return_type: "JSON",
        },
        SqlFunction {
            name: "JSON_OBJECT",
            signature: "JSON_OBJECT(key, value [, key, value] ...)",
            description: "Creates a JSON object from key-value pairs.",
            parameters: &[
                SqlFunctionParam {
                    name: "key",
                    description: "Object key (string).",
                },
                SqlFunctionParam {
                    name: "value",
                    description: "Object value.",
                },
            ],
            return_type: "JSON",
        },
        SqlFunction {
            name: "JSON_ARRAY",
            signature: "JSON_ARRAY(value [, value] ...)",
            description: "Creates a JSON array from the given values.",
            parameters: &[SqlFunctionParam {
                name: "value",
                description: "Value to include in the array.",
            }],
            return_type: "JSON",
        },
        SqlFunction {
            name: "UUID",
            signature: "UUID()",
            description: "Returns a Universal Unique Identifier (UUID) as a 36-character string.",
            parameters: &[],
            return_type: "VARCHAR",
        },
        SqlFunction {
            name: "LAST_INSERT_ID",
            signature: "LAST_INSERT_ID([expr])",
            description: "Returns the last automatically generated AUTO_INCREMENT value.",
            parameters: &[SqlFunctionParam {
                name: "expr",
                description: "Optional expression to set as the LAST_INSERT_ID value.",
            }],
            return_type: "BIGINT",
        },
    ]
});

/// Returns all known SQL functions.
pub fn get_all_functions() -> &'static [SqlFunction] {
    &ALL_FUNCTIONS
}

/// Looks up a SQL function by name (case-insensitive).
pub fn get_function(name: &str) -> Option<&'static SqlFunction> {
    ALL_FUNCTIONS.iter().find(|f| f.name.eq_ignore_ascii_case(name))
}

/// Returns `true` if the given name is a known aggregate function.
pub fn is_aggregate_function(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "COUNT" | "SUM" | "AVG" | "MIN" | "MAX" | "GROUP_CONCAT"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_all_functions_not_empty() {
        assert!(!get_all_functions().is_empty());
    }

    #[test]
    fn test_get_function_case_insensitive() {
        assert!(get_function("count").is_some());
        assert!(get_function("COUNT").is_some());
        assert!(get_function("Count").is_some());
    }

    #[test]
    fn test_get_function_unknown() {
        assert!(get_function("NONEXISTENT_FUNC").is_none());
    }

    #[test]
    fn test_is_aggregate() {
        assert!(is_aggregate_function("COUNT"));
        assert!(is_aggregate_function("sum"));
        assert!(!is_aggregate_function("CONCAT"));
    }

    #[test]
    fn test_all_functions_have_signatures() {
        for func in get_all_functions() {
            assert!(!func.name.is_empty());
            assert!(!func.signature.is_empty());
            assert!(!func.description.is_empty());
            assert!(!func.return_type.is_empty());
        }
    }
}
