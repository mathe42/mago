use lsp_types::DiagnosticSeverity;
use lsp_types::NumberOrString;

use mago_database::file::File;
use mago_reporting::Annotation;
use mago_reporting::AnnotationKind;
use mago_reporting::Issue;
use mago_reporting::Level;
use mago_span::Span;

/// Convert a byte offset in a file to an LSP `Position` (0-based line, UTF-16 character offset).
pub fn offset_to_lsp_position(file: &File, offset: u32) -> lsp_types::Position {
    let offset = offset.min(file.size);
    let line = file.line_number(offset);
    let line_start = file.lines[line as usize];
    let col_byte_offset = (offset - line_start) as usize;

    // Convert byte offset within the line to UTF-16 code units.
    let line_bytes = &file.contents.as_bytes()[line_start as usize..];
    let character = byte_offset_to_utf16(line_bytes, col_byte_offset);

    lsp_types::Position { line, character: character as u32 }
}

/// Convert an LSP `Position` back to a byte offset in the file.
pub fn lsp_position_to_offset(file: &File, position: lsp_types::Position) -> u32 {
    let line = position.line as usize;
    if line >= file.lines.len() {
        return file.size;
    }

    let line_start = file.lines[line] as usize;
    let line_bytes = &file.contents.as_bytes()[line_start..];
    let byte_col = utf16_offset_to_byte(line_bytes, position.character as usize);

    (line_start + byte_col) as u32
}

/// Convert a `Span` to an LSP `Range`.
pub fn span_to_range(file: &File, span: Span) -> lsp_types::Range {
    lsp_types::Range {
        start: offset_to_lsp_position(file, span.start.offset),
        end: offset_to_lsp_position(file, span.end.offset),
    }
}

/// Convert an `Issue` to an LSP `Diagnostic`.
///
/// Uses the primary annotation's span for the diagnostic range.
/// Returns `None` if the issue has no primary annotation.
pub fn issue_to_diagnostic(file: &File, issue: &Issue) -> Option<lsp_types::Diagnostic> {
    let primary = issue.annotations.iter().find(|a| a.kind == AnnotationKind::Primary)?;

    let range = span_to_range(file, primary.span);
    let severity = Some(level_to_severity(issue.level));

    let code = issue.code.as_ref().map(|c| NumberOrString::String(c.clone()));

    let related_information = build_related_information(file, &issue.annotations, &issue.notes);

    let mut message = issue.message.clone();
    if let Some(help) = &issue.help {
        message.push_str("\n\n");
        message.push_str(help);
    }

    Some(lsp_types::Diagnostic {
        range,
        severity,
        code,
        code_description: None,
        source: Some("mago".to_string()),
        message,
        related_information: if related_information.is_empty() { None } else { Some(related_information) },
        tags: None,
        data: None,
    })
}

fn level_to_severity(level: Level) -> DiagnosticSeverity {
    match level {
        Level::Error => DiagnosticSeverity::ERROR,
        Level::Warning => DiagnosticSeverity::WARNING,
        Level::Note => DiagnosticSeverity::INFORMATION,
        Level::Help => DiagnosticSeverity::HINT,
    }
}

fn build_related_information(
    file: &File,
    annotations: &[Annotation],
    notes: &[String],
) -> Vec<lsp_types::DiagnosticRelatedInformation> {
    let mut related = Vec::new();

    // Secondary annotations become related information.
    for annotation in annotations {
        if annotation.kind == AnnotationKind::Secondary {
            if let Some(msg) = &annotation.message {
                let uri = file_uri(file);
                related.push(lsp_types::DiagnosticRelatedInformation {
                    location: lsp_types::Location { uri, range: span_to_range(file, annotation.span) },
                    message: msg.clone(),
                });
            }
        }
    }

    // Notes become related info at the same location (line 0).
    for note in notes {
        let uri = file_uri(file);
        related.push(lsp_types::DiagnosticRelatedInformation {
            location: lsp_types::Location {
                uri,
                range: lsp_types::Range::default(),
            },
            message: note.clone(),
        });
    }

    related
}

/// Build a `file://` URI from a `File`.
pub fn file_uri(file: &File) -> lsp_types::Uri {
    if let Some(path) = &file.path {
        path_to_uri(path)
    } else {
        let uri_str = format!("file:///{}", file.name.replace('\\', "/"));
        uri_str.parse().expect("valid URI from file name")
    }
}

/// Convert a filesystem path to a `file://` URI.
pub fn path_to_uri(path: &std::path::Path) -> lsp_types::Uri {
    let mut path_str = path.to_string_lossy().replace('\\', "/");
    // Strip Windows extended-length path prefix (\\?\)
    if path_str.starts_with("//?/") {
        path_str = path_str[4..].to_string();
    }
    // Percent-encode characters that are invalid in URIs (spaces, etc.)
    let path_str = path_str
        .replace(' ', "%20")
        .replace('#', "%23")
        .replace('?', "%3F");
    let uri_str = if path_str.starts_with('/') {
        format!("file://{path_str}")
    } else {
        format!("file:///{path_str}")
    };
    uri_str.parse().unwrap_or_else(|_| {
        // Fallback: just use the raw path
        format!("file:///{}", path.display()).parse().expect("fallback URI")
    })
}

/// Convert a `file://` URI to a filesystem path.
pub fn uri_to_path(uri: &lsp_types::Uri) -> Option<std::path::PathBuf> {
    let s = uri.as_str();
    if !s.starts_with("file:///") {
        return None;
    }
    let path_part = &s["file:///".len()..];
    // On Windows, paths look like file:///C:/foo, on Unix file:///foo
    Some(std::path::PathBuf::from(path_part.replace('/', &std::path::MAIN_SEPARATOR.to_string())))
}

/// Convert a byte offset within a line to UTF-16 code units.
///
/// Walks the line bytes as UTF-8 characters, counting the number of UTF-16
/// code units consumed until `byte_offset` bytes have been processed.
fn byte_offset_to_utf16(line_bytes: &[u8], byte_offset: usize) -> usize {
    let text = match std::str::from_utf8(&line_bytes[..byte_offset.min(line_bytes.len())]) {
        Ok(t) => t,
        Err(_) => return byte_offset, // fallback: assume ASCII
    };

    text.chars().map(|c| c.len_utf16()).sum()
}

/// Convert a UTF-16 character offset to a byte offset within a line.
fn utf16_offset_to_byte(line_bytes: &[u8], utf16_offset: usize) -> usize {
    let line_str = match std::str::from_utf8(line_bytes) {
        Ok(s) => s,
        Err(e) => match std::str::from_utf8(&line_bytes[..e.valid_up_to()]) {
            Ok(s) => s,
            Err(_) => return utf16_offset.min(line_bytes.len()),
        },
    };

    let mut utf16_count = 0usize;
    let mut byte_count = 0usize;
    for c in line_str.chars() {
        if utf16_count >= utf16_offset {
            break;
        }
        utf16_count += c.len_utf16();
        byte_count += c.len_utf8();
    }

    byte_count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_byte_offset_to_utf16_ascii() {
        let line = b"hello world";
        assert_eq!(byte_offset_to_utf16(line, 0), 0);
        assert_eq!(byte_offset_to_utf16(line, 5), 5);
        assert_eq!(byte_offset_to_utf16(line, 11), 11);
    }

    #[test]
    fn test_byte_offset_to_utf16_multibyte() {
        // "a\u{00E9}b" = "aeb" where e-acute is 2 bytes in UTF-8, 1 code unit in UTF-16
        let text = "a\u{00E9}b";
        let line = text.as_bytes();
        assert_eq!(byte_offset_to_utf16(line, 0), 0); // before 'a'
        assert_eq!(byte_offset_to_utf16(line, 1), 1); // after 'a'
        assert_eq!(byte_offset_to_utf16(line, 3), 2); // after e-acute (2 UTF-8 bytes)
        assert_eq!(byte_offset_to_utf16(line, 4), 3); // after 'b'
    }

    #[test]
    fn test_utf16_offset_to_byte_ascii() {
        let line = b"hello";
        assert_eq!(utf16_offset_to_byte(line, 0), 0);
        assert_eq!(utf16_offset_to_byte(line, 3), 3);
        assert_eq!(utf16_offset_to_byte(line, 5), 5);
    }

    #[test]
    fn test_utf16_offset_to_byte_multibyte() {
        let text = "a\u{00E9}b";
        let line = text.as_bytes();
        assert_eq!(utf16_offset_to_byte(line, 0), 0);
        assert_eq!(utf16_offset_to_byte(line, 1), 1); // after 'a'
        assert_eq!(utf16_offset_to_byte(line, 2), 3); // after e-acute
        assert_eq!(utf16_offset_to_byte(line, 3), 4); // after 'b'
    }
}
