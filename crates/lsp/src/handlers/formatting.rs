use bumpalo::Bump;
use lsp_types::DocumentFormattingParams;
use lsp_types::TextEdit;

use mago_formatter::Formatter;
use mago_php_version::PHPVersion;

use crate::convert;
use crate::error::ServerError;
use crate::state::LspState;

/// Handle `textDocument/formatting`.
pub fn handle_formatting(
    state: &LspState,
    params: DocumentFormattingParams,
) -> Result<Option<Vec<TextEdit>>, ServerError> {
    let uri = &params.text_document.uri;
    let Some(file_id) = state.file_id_for_uri(uri) else {
        return Ok(None);
    };
    let Some(file) = state.get_file(&file_id) else {
        return Ok(None);
    };

    let arena = Bump::new();
    let formatter = Formatter::new(
        &arena,
        PHPVersion::default(),
        mago_formatter::settings::FormatSettings::default(),
    );

    let formatted = match formatter.format_file(&file) {
        Ok(result) => result,
        Err(e) => {
            tracing::warn!("formatting failed for {}: {e:?}", uri.as_str());
            return Ok(None);
        }
    };

    // If content is unchanged, return no edits.
    if formatted == file.contents.as_ref() {
        return Ok(Some(vec![]));
    }

    // Replace the entire document.
    let end_pos = convert::offset_to_lsp_position(&file, file.size);
    let edit = TextEdit {
        range: lsp_types::Range {
            start: lsp_types::Position { line: 0, character: 0 },
            end: end_pos,
        },
        new_text: formatted.to_string(),
    };

    Ok(Some(vec![edit]))
}
