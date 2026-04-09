use bumpalo::Bump;
use lsp_types::Hover;
use lsp_types::HoverContents;
use lsp_types::HoverParams;
use lsp_types::MarkupContent;
use lsp_types::MarkupKind;

use mago_codex::metadata::class_like::ClassLikeMetadata;
use mago_codex::metadata::function_like::FunctionLikeMetadata;
use mago_names::resolver::NameResolver;
use mago_syntax::parser::parse_file_content;

use crate::convert;
use crate::error::ServerError;
use crate::navigate;
use crate::navigate::SymbolAt;
use crate::state::LspState;

/// Handle `textDocument/hover`.
pub fn handle_hover(state: &LspState, params: HoverParams) -> Result<Option<Hover>, ServerError> {
    let uri = &params.text_document_position_params.text_document.uri;
    let position = params.text_document_position_params.position;

    let Some(file_id) = state.file_id_for_uri(uri) else {
        return Ok(None);
    };
    let Some(file) = state.get_file(&file_id) else {
        return Ok(None);
    };

    let offset = convert::lsp_position_to_offset(&file, position);

    let arena = Bump::new();
    let program = parse_file_content(&arena, file.id, &file.contents);
    let resolved_names = NameResolver::new(&arena).resolve(program);

    let codebase = state.codebase();
    let symbol = navigate::find_symbol_at_offset(program, &resolved_names, codebase, offset);

    let hover_range = symbol_span(&symbol).map(|span| convert::span_to_range(&file, span));

    let markdown = match symbol {
        SymbolAt::ClassLike { fqn, .. } => codebase.get_class_like(fqn).map(format_class_like_hover),
        SymbolAt::Function { fqn, .. } => codebase.get_function(fqn).map(format_function_hover),
        SymbolAt::Method { ref class_fqn, ref method_name, .. } if !class_fqn.is_empty() => {
            codebase.get_declaring_method(class_fqn, method_name).map(format_function_hover)
        }
        SymbolAt::Property { ref class_fqn, ref property_name, .. } if !class_fqn.is_empty() => {
            codebase.get_property_type(class_fqn, property_name).map(|t| {
                format!("```php\n${}: {:?}\n```", property_name, t)
            })
        }
        SymbolAt::ClassConstant { ref class_fqn, ref constant_name, .. } => {
            codebase.get_class_constant(class_fqn, constant_name).map(|_meta| {
                format!("```php\n{}::{}\n```", class_fqn, constant_name)
            })
        }
        _ => None,
    };

    let Some(content) = markdown else {
        return Ok(None);
    };

    Ok(Some(Hover {
        contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value: content }),
        range: hover_range,
    }))
}

fn symbol_span(symbol: &SymbolAt<'_>) -> Option<mago_span::Span> {
    match symbol {
        SymbolAt::ClassLike { span, .. }
        | SymbolAt::Function { span, .. }
        | SymbolAt::Method { span, .. }
        | SymbolAt::Property { span, .. }
        | SymbolAt::ClassConstant { span, .. } => Some(*span),
        SymbolAt::Unknown => None,
    }
}

fn format_class_like_hover(meta: &ClassLikeMetadata) -> String {
    let mut parts = Vec::new();

    let kind = format!("{:?}", meta.kind).to_lowercase();
    let name = &meta.original_name;

    let mut signature = format!("{kind} {name}");

    if let Some(parent) = &meta.direct_parent_class {
        signature.push_str(&format!(" extends {parent}"));
    }

    if !meta.direct_parent_interfaces.is_empty() {
        let ifaces: Vec<&str> = meta.direct_parent_interfaces.iter().map(|a| a.as_str()).collect();
        let keyword = if kind == "interface" { "extends" } else { "implements" };
        signature.push_str(&format!(" {keyword} {}", ifaces.join(", ")));
    }

    parts.push(format!("```php\n{signature}\n```"));

    if !meta.methods.is_empty() {
        let count = meta.methods.len();
        parts.push(format!("{count} method(s)"));
    }

    parts.join("\n\n")
}

fn format_function_hover(meta: &FunctionLikeMetadata) -> String {
    let mut parts = Vec::new();
    let name = meta.original_name.as_ref().map(|a| a.as_str()).unwrap_or("anonymous");

    let mut sig = String::new();

    // Visibility for methods
    if let Some(method_meta) = &meta.method_metadata {
        sig.push_str(&format!("{:?} ", method_meta.visibility).to_lowercase());
        if method_meta.is_static {
            sig.push_str("static ");
        }
        if method_meta.is_abstract {
            sig.push_str("abstract ");
        }
    }

    sig.push_str("function ");
    sig.push_str(name);
    sig.push('(');

    // Parameters
    let params: Vec<String> = meta
        .parameters
        .iter()
        .map(|p| {
            let mut param = String::new();
            if let Some(type_meta) = &p.type_metadata {
                param.push_str(&format!("{:?} ", type_meta.type_union));
            }
            param.push_str(&p.name.0.to_string());
            param
        })
        .collect();
    sig.push_str(&params.join(", "));
    sig.push(')');

    // Return type
    if let Some(return_meta) = &meta.return_type_metadata {
        sig.push_str(&format!(": {:?}", return_meta.type_union));
    }

    parts.push(format!("```php\n{sig}\n```"));

    parts.join("\n\n")
}
