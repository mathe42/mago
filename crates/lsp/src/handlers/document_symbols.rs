use lsp_types::DocumentSymbol;
use lsp_types::DocumentSymbolParams;
use lsp_types::DocumentSymbolResponse;
use lsp_types::SymbolKind;

use mago_codex::signature::DefSignatureNode;

use crate::convert;
use crate::error::ServerError;
use crate::state::LspState;

/// Handle `textDocument/documentSymbol`.
pub fn handle_document_symbols(
    state: &LspState,
    params: DocumentSymbolParams,
) -> Result<Option<DocumentSymbolResponse>, ServerError> {
    let uri = &params.text_document.uri;
    let Some(file_id) = state.file_id_for_uri(uri) else {
        return Ok(None);
    };
    let Some(file) = state.get_file(&file_id) else {
        return Ok(None);
    };

    let codebase = state.codebase();
    let Some(file_sig) = codebase.file_signatures.get(&file_id) else {
        return Ok(Some(DocumentSymbolResponse::Nested(vec![])));
    };

    let symbols: Vec<DocumentSymbol> = file_sig
        .nodes()
        .iter()
        .map(|node| sig_node_to_document_symbol(node, &file))
        .collect();

    Ok(Some(DocumentSymbolResponse::Nested(symbols)))
}

fn sig_node_to_document_symbol(
    node: &DefSignatureNode,
    file: &mago_database::file::File,
) -> DocumentSymbol {
    let range = lsp_types::Range {
        start: convert::offset_to_lsp_position(file, node.start_offset),
        end: convert::offset_to_lsp_position(file, node.end_offset),
    };

    // Selection range is the name portion — approximate from start to a smaller range
    let selection_range = range;

    let kind = if node.is_function {
        if node.children.is_empty() {
            SymbolKind::FUNCTION
        } else {
            SymbolKind::METHOD
        }
    } else if node.is_constant {
        SymbolKind::CONSTANT
    } else if !node.children.is_empty() {
        // Has children → likely a class/interface/trait/enum
        SymbolKind::CLASS
    } else {
        SymbolKind::VARIABLE
    };

    let children: Vec<DocumentSymbol> = node
        .children
        .iter()
        .map(|child| sig_node_to_document_symbol(child, file))
        .collect();

    #[allow(deprecated)]
    DocumentSymbol {
        name: node.name.to_string(),
        detail: None,
        kind,
        tags: None,
        deprecated: None,
        range,
        selection_range,
        children: if children.is_empty() { None } else { Some(children) },
    }
}
