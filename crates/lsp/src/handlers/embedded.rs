use bumpalo::Bump;
use lsp_types::CompletionItem;
use lsp_types::CompletionItemKind;
use lsp_types::Diagnostic;
use lsp_types::DiagnosticSeverity;

use mago_database::file::File;
use mago_embedded_languages::EmbeddedLanguage;
use mago_embedded_languages::EmbeddedRegion;
use mago_embedded_languages::detect_embedded_regions;
use mago_embedded_languages::sql;
use mago_names::resolver::NameResolver;
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
pub fn get_embedded_completions(file: &File, offset: u32) -> Option<Vec<CompletionItem>> {
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
                return Some(sql_completions_at_offset(&region, offset));
            }
            EmbeddedLanguage::Bash => {
                return Some(bash_completions());
            }
        }
    }

    None
}

fn sql_completions_at_offset(region: &EmbeddedRegion, _offset: u32) -> Vec<CompletionItem> {
    // Return SQL keyword completions.
    sql::sql_keyword_completions()
        .into_iter()
        .map(|kw| CompletionItem {
            label: kw.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some("SQL".to_string()),
            ..Default::default()
        })
        .collect()
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
