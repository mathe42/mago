use bumpalo::Bump;
use lsp_types::CompletionItem;
use lsp_types::CompletionItemKind;
use lsp_types::Diagnostic;
use lsp_types::DiagnosticSeverity;
use lsp_types::Hover;
use lsp_types::HoverContents;
use lsp_types::MarkupContent;
use lsp_types::MarkupKind;
use lsp_types::SignatureHelp;
use lsp_types::SignatureInformation;
use lsp_types::ParameterInformation;
use lsp_types::ParameterLabel;

use mago_database::file::File;
use mago_embedded_languages::EmbeddedLanguage;
use mago_embedded_languages::detect_embedded_regions;
use mago_embedded_languages::sql;
use mago_embedded_languages::sql::schema::SqlSchema;
use mago_names::resolver::NameResolver;
use mago_span::HasSpan;
use mago_syntax::parser::parse_file_content;

use crate::convert;

/// Detect embedded regions in a file and return diagnostics.
pub fn get_embedded_diagnostics(file: &File) -> Vec<Diagnostic> {
    let arena = Bump::new();
    let program = parse_file_content(&arena, file.id, &file.contents);
    let resolved_names = NameResolver::new(&arena).resolve(program);

    let regions = detect_embedded_regions(program, &resolved_names);
    let mut diagnostics = Vec::new();

    for region in &regions {
        match region.language {
            EmbeddedLanguage::Sql => {
                let result = sql::parse_sql(&region.virtual_document);
                for diag in &result.diagnostics {
                    // Map the diagnostic back to PHP source position.
                    // If we have a virtual offset, try to map it; otherwise use the string span.
                    let range = if let Some(v_offset) = diag.virtual_offset {
                        if let Some(php_offset) = region.mapping.virtual_to_php(v_offset) {
                            let pos = convert::offset_to_lsp_position(file, php_offset);
                            lsp_types::Range { start: pos, end: pos }
                        } else {
                            convert::span_to_range(file, region.php_span)
                        }
                    } else {
                        convert::span_to_range(file, region.php_span)
                    };

                    diagnostics.push(Diagnostic {
                        range,
                        severity: Some(DiagnosticSeverity::WARNING),
                        code: None,
                        code_description: None,
                        source: Some("mago-sql".to_string()),
                        message: diag.message.clone(),
                        related_information: None,
                        tags: None,
                        data: None,
                    });
                }
            }
            EmbeddedLanguage::Bash => {
                // Bash diagnostics would go here (tree-sitter-bash).
                // For now, just detect the region without diagnostics.
            }
        }
    }

    diagnostics
}

/// Get completions for embedded languages at a given offset.
pub fn get_embedded_completions(file: &File, offset: u32, sql_schema: Option<&SqlSchema>) -> Option<Vec<CompletionItem>> {
    let arena = Bump::new();
    let program = parse_file_content(&arena, file.id, &file.contents);
    let resolved_names = NameResolver::new(&arena).resolve(program);

    let regions = detect_embedded_regions(program, &resolved_names);

    // Find the region containing the cursor.
    for region in &regions {
        if !region.php_span.has_offset(offset) {
            continue;
        }

        match region.language {
            EmbeddedLanguage::Sql => {
                // Map PHP offset to virtual SQL offset for context-aware completions.
                let virtual_offset = region.mapping.php_to_virtual(offset);
                let items = if let Some(v_off) = virtual_offset {
                    sql_completions_context_aware(&region.virtual_document, v_off, sql_schema)
                } else {
                    sql_completions_fallback(sql_schema)
                };
                return Some(items);
            }
            EmbeddedLanguage::Bash => {
                return Some(bash_completions());
            }
        }
    }

    None
}

/// Get hover info for embedded languages at a given offset.
pub fn get_embedded_hover(file: &File, offset: u32, sql_schema: Option<&SqlSchema>) -> Option<Hover> {
    let arena = Bump::new();
    let program = parse_file_content(&arena, file.id, &file.contents);
    let resolved_names = NameResolver::new(&arena).resolve(program);
    let regions = detect_embedded_regions(program, &resolved_names);

    for region in &regions {
        if !region.php_span.has_offset(offset) {
            continue;
        }
        if let EmbeddedLanguage::Sql = region.language {
            let virtual_offset = region.mapping.php_to_virtual(offset)?;
            let hover_info = sql::sql_hover(&region.virtual_document, virtual_offset, sql_schema)?;
            return Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: hover_info.content,
                }),
                range: None,
            });
        }
    }
    None
}

/// Get signature help for embedded SQL at a given offset.
pub fn get_embedded_signature_help(file: &File, offset: u32) -> Option<SignatureHelp> {
    let arena = Bump::new();
    let program = parse_file_content(&arena, file.id, &file.contents);
    let resolved_names = NameResolver::new(&arena).resolve(program);
    let regions = detect_embedded_regions(program, &resolved_names);

    for region in &regions {
        if !region.php_span.has_offset(offset) {
            continue;
        }
        if let EmbeddedLanguage::Sql = region.language {
            let virtual_offset = region.mapping.php_to_virtual(offset)?;
            let sig_info = sql::sql_signature_help(&region.virtual_document, virtual_offset)?;

            let parameters: Vec<ParameterInformation> = sig_info.parameters.iter().map(|(name, desc)| {
                ParameterInformation {
                    label: ParameterLabel::Simple(name.clone()),
                    documentation: if desc.is_empty() {
                        None
                    } else {
                        Some(lsp_types::Documentation::String(desc.clone()))
                    },
                }
            }).collect();

            let sig = SignatureInformation {
                label: sig_info.signature.clone(),
                documentation: None,
                parameters: Some(parameters),
                active_parameter: Some(sig_info.active_parameter),
            };

            return Some(SignatureHelp {
                signatures: vec![sig],
                active_signature: Some(0),
                active_parameter: Some(sig_info.active_parameter),
            });
        }
    }
    None
}

fn sql_completions_context_aware(virtual_document: &str, virtual_offset: u32, schema: Option<&SqlSchema>) -> Vec<CompletionItem> {
    let items = sql::sql_completions(virtual_document, virtual_offset, schema);
    items.into_iter().map(|item| {
        let kind = match item.kind {
            sql::SqlCompletionKind::Keyword => CompletionItemKind::KEYWORD,
            sql::SqlCompletionKind::Table => CompletionItemKind::CLASS,
            sql::SqlCompletionKind::Column => CompletionItemKind::FIELD,
            sql::SqlCompletionKind::Function => CompletionItemKind::FUNCTION,
        };
        CompletionItem {
            label: item.label,
            kind: Some(kind),
            detail: item.detail,
            documentation: item.documentation.map(|d| {
                lsp_types::Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: d,
                })
            }),
            ..Default::default()
        }
    }).collect()
}

fn sql_completions_fallback(schema: Option<&SqlSchema>) -> Vec<CompletionItem> {
    let mut items: Vec<CompletionItem> = sql::sql_keyword_completions()
        .into_iter()
        .map(|kw| CompletionItem {
            label: kw.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some("SQL".to_string()),
            ..Default::default()
        })
        .collect();

    // Add table names from schema
    if let Some(s) = schema {
        for name in s.table_names() {
            items.push(CompletionItem {
                label: name.to_string(),
                kind: Some(CompletionItemKind::CLASS),
                detail: Some("Table".to_string()),
                ..Default::default()
            });
        }
    }

    items
}

fn bash_completions() -> Vec<CompletionItem> {
    let commands = [
        "ls", "cd", "grep", "find", "cat", "echo", "rm", "cp", "mv", "mkdir",
        "chmod", "chown", "curl", "wget", "git", "docker", "npm", "php",
        "composer", "sed", "awk", "sort", "uniq", "wc", "head", "tail",
        "tar", "gzip", "ssh", "scp", "rsync", "kill", "ps", "top",
    ];

    commands
        .iter()
        .map(|cmd| CompletionItem {
            label: cmd.to_string(),
            kind: Some(CompletionItemKind::VALUE),
            detail: Some("Bash command".to_string()),
            ..Default::default()
        })
        .collect()
}
