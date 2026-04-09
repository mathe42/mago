use mago_names::ResolvedNames;
use mago_span::HasSpan;
use mago_span::Span;
use mago_syntax::ast::Program;
use mago_syntax::ast::ast::Argument;
use mago_syntax::ast::ast::Call;
use mago_syntax::ast::ast::ClassLikeMember;
use mago_syntax::ast::ast::ClassLikeMemberSelector;
use mago_syntax::ast::ast::CompositeString;
use mago_syntax::ast::ast::Expression;
use mago_syntax::ast::ast::Literal;
use mago_syntax::ast::ast::MethodBody;
use mago_syntax::ast::ast::Statement;
use mago_syntax::ast::ast::StringPart;

use crate::mapping::MappingSegment;
use crate::mapping::PositionMapping;
use crate::region::DetectionConfidence;
use crate::region::EmbeddedLanguage;
use crate::region::EmbeddedRegion;

/// SQL-related function names (global).
const SQL_FUNCTIONS: &[&str] = &[
    "\\mysqli_query",
    "\\mysqli_prepare",
    "\\mysqli_real_query",
    "\\pg_query",
    "\\pg_prepare",
    "\\pg_query_params",
];

/// Bash-related function names (global).
const BASH_FUNCTIONS: &[&str] = &[
    "\\exec",
    "\\shell_exec",
    "\\system",
    "\\passthru",
    "\\popen",
    "\\proc_open",
];

/// SQL-related method names on objects.
const SQL_METHODS: &[&str] = &["query", "prepare", "exec", "execute"];

/// SQL keywords that indicate a string contains SQL.
const SQL_KEYWORDS: &[&str] = &[
    "SELECT ", "INSERT ", "UPDATE ", "DELETE ", "CREATE ", "ALTER ", "DROP ",
    "WITH ", "REPLACE ", "TRUNCATE ", "EXPLAIN ", "SHOW ", "DESCRIBE ",
];

/// Heredoc/Nowdoc labels that indicate SQL.
const SQL_LABELS: &[&str] = &["SQL", "QUERY", "MYSQL", "PGSQL", "SQLITE"];

/// Heredoc/Nowdoc labels that indicate Bash.
const BASH_LABELS: &[&str] = &["BASH", "SH", "SHELL", "CMD"];

/// Detect embedded language regions in a parsed PHP program.
pub fn detect_embedded_regions<'arena>(
    program: &Program<'arena>,
    resolved_names: &ResolvedNames<'arena>,
) -> Vec<EmbeddedRegion> {
    let mut regions = Vec::new();

    for statement in program.statements.iter() {
        detect_in_statement(statement, resolved_names, &mut regions);
    }

    regions
}

fn detect_in_statement<'arena>(
    stmt: &Statement<'arena>,
    resolved_names: &ResolvedNames<'arena>,
    regions: &mut Vec<EmbeddedRegion>,
) {
    match stmt {
        Statement::Expression(expr_stmt) => {
            detect_in_expression(&expr_stmt.expression, resolved_names, regions);
        }
        Statement::Return(ret) => {
            if let Some(val) = &ret.value {
                detect_in_expression(val, resolved_names, regions);
            }
        }
        Statement::Block(block) => {
            for s in block.statements.iter() {
                detect_in_statement(s, resolved_names, regions);
            }
        }
        Statement::Function(func) => {
            for s in func.body.statements.iter() {
                detect_in_statement(s, resolved_names, regions);
            }
        }
        Statement::Class(class) => {
            for member in class.members.iter() {
                if let ClassLikeMember::Method(method) = member {
                    if let MethodBody::Concrete(block) = &method.body {
                        for s in block.statements.iter() {
                            detect_in_statement(s, resolved_names, regions);
                        }
                    }
                }
            }
        }
        Statement::If(if_stmt) => {
            detect_in_expression(&if_stmt.condition, resolved_names, regions);
            match &if_stmt.body {
                mago_syntax::ast::ast::IfBody::Statement(body) => {
                    detect_in_statement(&body.statement, resolved_names, regions);
                    for elseif in body.else_if_clauses.iter() {
                        detect_in_expression(&elseif.condition, resolved_names, regions);
                        detect_in_statement(&elseif.statement, resolved_names, regions);
                    }
                    if let Some(else_clause) = &body.else_clause {
                        detect_in_statement(&else_clause.statement, resolved_names, regions);
                    }
                }
                _ => {}
            }
        }
        Statement::Namespace(ns) => {
            let stmts = match &ns.body {
                mago_syntax::ast::ast::NamespaceBody::Implicit(body) => &body.statements,
                mago_syntax::ast::ast::NamespaceBody::BraceDelimited(block) => &block.statements,
            };
            for s in stmts.iter() {
                detect_in_statement(s, resolved_names, regions);
            }
        }
        Statement::Foreach(foreach) => {
            detect_in_expression(&foreach.expression, resolved_names, regions);
        }
        Statement::While(while_stmt) => {
            detect_in_expression(&while_stmt.condition, resolved_names, regions);
        }
        _ => {}
    }
}

fn detect_in_expression<'arena>(
    expr: &Expression<'arena>,
    resolved_names: &ResolvedNames<'arena>,
    regions: &mut Vec<EmbeddedRegion>,
) {
    match expr {
        // Backtick strings are always Bash
        Expression::CompositeString(CompositeString::ShellExecute(shell)) => {
            let (doc, mapping) = build_virtual_document_from_parts(&shell.parts, shell.span());
            if !doc.is_empty() {
                regions.push(EmbeddedRegion {
                    language: EmbeddedLanguage::Bash,
                    php_span: shell.span(),
                    confidence: DetectionConfidence::Strong,
                    virtual_document: doc,
                    mapping,
                });
            }
        }

        // Heredoc/Nowdoc — check label
        Expression::CompositeString(CompositeString::Document(doc_str)) => {
            let label_upper = doc_str.label.to_uppercase();
            let language = if SQL_LABELS.contains(&label_upper.as_str()) {
                Some(EmbeddedLanguage::Sql)
            } else if BASH_LABELS.contains(&label_upper.as_str()) {
                Some(EmbeddedLanguage::Bash)
            } else {
                None
            };

            if let Some(lang) = language {
                let (doc, mapping) = build_virtual_document_from_parts(&doc_str.parts, doc_str.span());
                if !doc.is_empty() {
                    regions.push(EmbeddedRegion {
                        language: lang,
                        php_span: doc_str.span(),
                        confidence: DetectionConfidence::Label,
                        virtual_document: doc,
                        mapping,
                    });
                }
            }
        }

        // Function calls — check function name for context
        Expression::Call(Call::Function(call)) => {
            if let Expression::Identifier(ident) = &*call.function {
                if let Some(fqn) = resolved_names.resolve(ident) {
                    let fqn_lower = fqn.to_lowercase();
                    let language = if BASH_FUNCTIONS.iter().any(|f| fqn_lower == *f) {
                        Some(EmbeddedLanguage::Bash)
                    } else if SQL_FUNCTIONS.iter().any(|f| fqn_lower == *f) {
                        Some(EmbeddedLanguage::Sql)
                    } else {
                        None
                    };

                    if let Some(lang) = language {
                        // Get the first string argument
                        if let Some(region) = extract_string_arg_region(
                            &call.argument_list.arguments,
                            0,
                            lang,
                            DetectionConfidence::Strong,
                        ) {
                            regions.push(region);
                        }
                    }
                }
            }

            // Recurse into arguments for nested detection
            for arg in call.argument_list.arguments.iter() {
                detect_in_arg(arg, resolved_names, regions);
            }
        }

        // Method calls — check method name
        Expression::Call(Call::Method(call)) => {
            if let ClassLikeMemberSelector::Identifier(method_ident) = &call.method {
                let method_lower = method_ident.value.to_lowercase();
                if SQL_METHODS.contains(&method_lower.as_str()) {
                    if let Some(region) = extract_string_arg_region(
                        &call.argument_list.arguments,
                        0,
                        EmbeddedLanguage::Sql,
                        DetectionConfidence::Strong,
                    ) {
                        regions.push(region);
                    }
                }
            }
            // Recurse
            detect_in_expression(&*call.object, resolved_names, regions);
            for arg in call.argument_list.arguments.iter() {
                detect_in_arg(arg, resolved_names, regions);
            }
        }

        // Static method calls — check for DB::raw(), etc.
        Expression::Call(Call::StaticMethod(call)) => {
            if let ClassLikeMemberSelector::Identifier(method_ident) = &call.method {
                let method_lower = method_ident.value.to_lowercase();
                let is_db_method = matches!(
                    method_lower.as_str(),
                    "raw" | "select" | "insert" | "update" | "delete" | "statement" | "unprepared"
                );
                if is_db_method {
                    // Check if class name looks like a DB facade
                    if let Expression::Identifier(class_ident) = &*call.class {
                        let class_name = class_ident.value();
                        if class_name.ends_with("DB") || class_name.ends_with("Database") || class_name == "DB" {
                            if let Some(region) = extract_string_arg_region(
                                &call.argument_list.arguments,
                                0,
                                EmbeddedLanguage::Sql,
                                DetectionConfidence::Strong,
                            ) {
                                regions.push(region);
                            }
                        }
                    }
                }
            }
            for arg in call.argument_list.arguments.iter() {
                detect_in_arg(arg, resolved_names, regions);
            }
        }

        // Assignment — recurse into both sides
        Expression::Assignment(assign) => {
            detect_in_expression(&*assign.lhs, resolved_names, regions);
            detect_in_expression(&*assign.rhs, resolved_names, regions);
        }

        // Heuristic: standalone strings that look like SQL
        Expression::Literal(Literal::String(lit_str)) => {
            if let Some(value) = lit_str.value {
                if looks_like_sql(value) {
                    let content_start = lit_str.span.start.offset + 1; // skip quote
                    let mapping = PositionMapping::simple(value.len() as u32, content_start);
                    regions.push(EmbeddedRegion {
                        language: EmbeddedLanguage::Sql,
                        php_span: lit_str.span,
                        confidence: DetectionConfidence::Heuristic,
                        virtual_document: value.to_string(),
                        mapping,
                    });
                }
            }
        }

        // Interpolated strings — heuristic check
        Expression::CompositeString(CompositeString::Interpolated(interp)) => {
            let (doc, mapping) = build_virtual_document_from_parts(&interp.parts, interp.span());
            if looks_like_sql(&doc) {
                regions.push(EmbeddedRegion {
                    language: EmbeddedLanguage::Sql,
                    php_span: interp.span(),
                    confidence: DetectionConfidence::Heuristic,
                    virtual_document: doc,
                    mapping,
                });
            }
        }

        // Binary (concatenation) — recurse
        Expression::Binary(bin) => {
            detect_in_expression(&*bin.lhs, resolved_names, regions);
            detect_in_expression(&*bin.rhs, resolved_names, regions);
        }

        Expression::Parenthesized(p) => {
            detect_in_expression(&*p.expression, resolved_names, regions);
        }

        _ => {}
    }
}

fn detect_in_arg<'arena>(
    arg: &Argument<'arena>,
    resolved_names: &ResolvedNames<'arena>,
    regions: &mut Vec<EmbeddedRegion>,
) {
    match arg {
        Argument::Positional(a) => detect_in_expression(&a.value, resolved_names, regions),
        Argument::Named(a) => detect_in_expression(&a.value, resolved_names, regions),
        _ => {}
    }
}

/// Extract a string argument at a given index and create an EmbeddedRegion.
fn extract_string_arg_region<'arena>(
    arguments: &mago_syntax::ast::sequence::TokenSeparatedSequence<'arena, Argument<'arena>>,
    index: usize,
    language: EmbeddedLanguage,
    confidence: DetectionConfidence,
) -> Option<EmbeddedRegion> {
    let arg = arguments.iter().nth(index)?;
    let expr = match arg {
        Argument::Positional(a) => &a.value,
        Argument::Named(a) => &a.value,
        _ => return None,
    };

    match expr {
        Expression::Literal(Literal::String(lit_str)) => {
            let value = lit_str.value?;
            let content_start = lit_str.span.start.offset + 1;
            let mapping = PositionMapping::simple(value.len() as u32, content_start);
            Some(EmbeddedRegion {
                language,
                php_span: lit_str.span,
                confidence,
                virtual_document: value.to_string(),
                mapping,
            })
        }
        Expression::CompositeString(CompositeString::Interpolated(interp)) => {
            let (doc, mapping) = build_virtual_document_from_parts(&interp.parts, interp.span());
            Some(EmbeddedRegion {
                language,
                php_span: interp.span(),
                confidence,
                virtual_document: doc,
                mapping,
            })
        }
        Expression::CompositeString(CompositeString::Document(doc_str)) => {
            let (doc, mapping) = build_virtual_document_from_parts(&doc_str.parts, doc_str.span());
            Some(EmbeddedRegion {
                language,
                php_span: doc_str.span(),
                confidence,
                virtual_document: doc,
                mapping,
            })
        }
        _ => None,
    }
}

/// Check if a string looks like SQL content.
fn looks_like_sql(s: &str) -> bool {
    let trimmed = s.trim_start().to_uppercase();
    SQL_KEYWORDS.iter().any(|kw| trimmed.starts_with(kw))
}

/// Build a virtual document from StringParts, replacing interpolations with placeholders.
fn build_virtual_document_from_parts<'arena>(
    parts: &mago_syntax::ast::Sequence<'arena, StringPart<'arena>>,
    _parent_span: Span,
) -> (String, PositionMapping) {
    let mut doc = String::new();
    let mut mapping = PositionMapping::new();
    let mut virtual_offset: u32 = 0;

    for part in parts.iter() {
        match part {
            StringPart::Literal(lit) => {
                let value = lit.value;
                let len = value.len() as u32;
                let php_start = lit.span.start.offset;
                let php_end = lit.span.end.offset;

                mapping.segments.push(MappingSegment::Literal {
                    virtual_range: virtual_offset..virtual_offset + len,
                    php_range: php_start..php_end,
                });

                doc.push_str(value);
                virtual_offset += len;
            }
            StringPart::Expression(expr) => {
                // Replace with placeholder characters of similar byte length
                let expr_len = (expr.span().end.offset - expr.span().start.offset) as usize;
                let placeholder_len = expr_len.max(1);
                let placeholder: String = std::iter::repeat_n('?', placeholder_len).collect();

                mapping.segments.push(MappingSegment::Placeholder {
                    virtual_range: virtual_offset..virtual_offset + placeholder_len as u32,
                    php_span: expr.span(),
                });

                doc.push_str(&placeholder);
                virtual_offset += placeholder_len as u32;
            }
            StringPart::BracedExpression(braced) => {
                let expr_len = (braced.span().end.offset - braced.span().start.offset) as usize;
                let placeholder_len = expr_len.max(1);
                let placeholder: String = std::iter::repeat_n('?', placeholder_len).collect();

                mapping.segments.push(MappingSegment::Placeholder {
                    virtual_range: virtual_offset..virtual_offset + placeholder_len as u32,
                    php_span: braced.span(),
                });

                doc.push_str(&placeholder);
                virtual_offset += placeholder_len as u32;
            }
        }
    }

    (doc, mapping)
}
