use bumpalo::Bump;
use lsp_types::InlayHint;
use lsp_types::InlayHintKind;
use lsp_types::InlayHintLabel;
use lsp_types::InlayHintParams;

use mago_codex::metadata::CodebaseMetadata;
use mago_codex::metadata::function_like::FunctionLikeMetadata;
use mago_codex::ttype::TType;
use mago_database::file::File;
use mago_names::ResolvedNames;
use mago_names::resolver::NameResolver;
use mago_span::HasSpan;
use mago_syntax::ast::Program;
use mago_syntax::ast::ast::Argument;
use mago_syntax::ast::ast::Call;
use mago_syntax::ast::ast::ClassLikeMember;
use mago_syntax::ast::ast::ClassLikeMemberSelector;
use mago_syntax::ast::ast::Expression;
use mago_syntax::ast::ast::Statement;
use mago_syntax::parser::parse_file_content;

use crate::convert;
use crate::error::ServerError;
use crate::state::LspState;

/// Handle `textDocument/inlayHint`.
pub fn handle_inlay_hints(
    state: &LspState,
    params: InlayHintParams,
) -> Result<Option<Vec<InlayHint>>, ServerError> {
    let uri = &params.text_document.uri;
    let range = params.range;

    let Some(file_id) = state.file_id_for_uri(uri) else {
        return Ok(None);
    };
    let Some(file) = state.get_file(&file_id) else {
        return Ok(None);
    };

    let range_start = convert::lsp_position_to_offset(&file, range.start);
    let range_end = convert::lsp_position_to_offset(&file, range.end);

    let arena = Bump::new();
    let program = parse_file_content(&arena, file.id, &file.contents);
    let resolved_names = NameResolver::new(&arena).resolve(program);
    let codebase = state.codebase();

    let mut hints = Vec::new();

    for statement in program.statements.iter() {
        collect_hints_from_statement(
            statement,
            program,
            &resolved_names,
            codebase,
            &file,
            range_start,
            range_end,
            &mut hints,
        );
    }

    if hints.is_empty() {
        Ok(None)
    } else {
        Ok(Some(hints))
    }
}

/// Recursively walk a statement and collect inlay hints for function/method calls.
fn collect_hints_from_statement<'arena>(
    statement: &Statement<'arena>,
    program: &Program<'arena>,
    resolved_names: &ResolvedNames<'arena>,
    codebase: &CodebaseMetadata,
    file: &File,
    range_start: u32,
    range_end: u32,
    hints: &mut Vec<InlayHint>,
) {
    match statement {
        Statement::Expression(expr_stmt) => {
            collect_hints_from_expression(
                expr_stmt.expression, program, resolved_names, codebase, file,
                range_start, range_end, hints,
            );
        }
        Statement::Return(ret) => {
            if let Some(value) = &ret.value {
                collect_hints_from_expression(
                    value, program, resolved_names, codebase, file,
                    range_start, range_end, hints,
                );
            }
        }
        Statement::Echo(echo) => {
            for val in echo.values.iter() {
                collect_hints_from_expression(
                    val, program, resolved_names, codebase, file,
                    range_start, range_end, hints,
                );
            }
        }
        Statement::Namespace(ns) => {
            let stmts = match &ns.body {
                mago_syntax::ast::ast::NamespaceBody::Implicit(body) => &body.statements,
                mago_syntax::ast::ast::NamespaceBody::BraceDelimited(block) => &block.statements,
            };
            for stmt in stmts.iter() {
                collect_hints_from_statement(
                    stmt, program, resolved_names, codebase, file,
                    range_start, range_end, hints,
                );
            }
        }
        Statement::Function(func) => {
            for stmt in func.body.statements.iter() {
                collect_hints_from_statement(
                    stmt, program, resolved_names, codebase, file,
                    range_start, range_end, hints,
                );
            }
        }
        Statement::Class(class) => {
            for member in class.members.iter() {
                collect_hints_from_class_member(
                    member, program, resolved_names, codebase, file,
                    range_start, range_end, hints,
                );
            }
        }
        Statement::Trait(r#trait) => {
            for member in r#trait.members.iter() {
                collect_hints_from_class_member(
                    member, program, resolved_names, codebase, file,
                    range_start, range_end, hints,
                );
            }
        }
        Statement::Enum(r#enum) => {
            for member in r#enum.members.iter() {
                collect_hints_from_class_member(
                    member, program, resolved_names, codebase, file,
                    range_start, range_end, hints,
                );
            }
        }
        Statement::Block(block) => {
            for stmt in block.statements.iter() {
                collect_hints_from_statement(
                    stmt, program, resolved_names, codebase, file,
                    range_start, range_end, hints,
                );
            }
        }
        Statement::If(if_stmt) => {
            collect_hints_from_expression(
                if_stmt.condition, program, resolved_names, codebase, file,
                range_start, range_end, hints,
            );
            match &if_stmt.body {
                mago_syntax::ast::ast::control_flow::r#if::IfBody::Statement(body) => {
                    collect_hints_from_statement(
                        body.statement, program, resolved_names, codebase, file,
                        range_start, range_end, hints,
                    );
                    for clause in body.else_if_clauses.iter() {
                        collect_hints_from_expression(
                            clause.condition, program, resolved_names, codebase, file,
                            range_start, range_end, hints,
                        );
                        collect_hints_from_statement(
                            clause.statement, program, resolved_names, codebase, file,
                            range_start, range_end, hints,
                        );
                    }
                    if let Some(else_clause) = &body.else_clause {
                        collect_hints_from_statement(
                            else_clause.statement, program, resolved_names, codebase, file,
                            range_start, range_end, hints,
                        );
                    }
                }
                mago_syntax::ast::ast::control_flow::r#if::IfBody::ColonDelimited(body) => {
                    for stmt in body.statements.iter() {
                        collect_hints_from_statement(
                            stmt, program, resolved_names, codebase, file,
                            range_start, range_end, hints,
                        );
                    }
                    for clause in body.else_if_clauses.iter() {
                        collect_hints_from_expression(
                            clause.condition, program, resolved_names, codebase, file,
                            range_start, range_end, hints,
                        );
                        for stmt in clause.statements.iter() {
                            collect_hints_from_statement(
                                stmt, program, resolved_names, codebase, file,
                                range_start, range_end, hints,
                            );
                        }
                    }
                    if let Some(else_clause) = &body.else_clause {
                        for stmt in else_clause.statements.iter() {
                            collect_hints_from_statement(
                                stmt, program, resolved_names, codebase, file,
                                range_start, range_end, hints,
                            );
                        }
                    }
                }
            }
        }
        Statement::While(while_stmt) => {
            collect_hints_from_expression(
                while_stmt.condition, program, resolved_names, codebase, file,
                range_start, range_end, hints,
            );
            match &while_stmt.body {
                mago_syntax::ast::ast::r#loop::r#while::WhileBody::Statement(stmt) => {
                    collect_hints_from_statement(
                        stmt, program, resolved_names, codebase, file,
                        range_start, range_end, hints,
                    );
                }
                mago_syntax::ast::ast::r#loop::r#while::WhileBody::ColonDelimited(body) => {
                    for stmt in body.statements.iter() {
                        collect_hints_from_statement(
                            stmt, program, resolved_names, codebase, file,
                            range_start, range_end, hints,
                        );
                    }
                }
            }
        }
        Statement::DoWhile(do_while) => {
            collect_hints_from_statement(
                do_while.statement, program, resolved_names, codebase, file,
                range_start, range_end, hints,
            );
            collect_hints_from_expression(
                do_while.condition, program, resolved_names, codebase, file,
                range_start, range_end, hints,
            );
        }
        Statement::For(for_stmt) => {
            for expr in for_stmt.initializations.iter() {
                collect_hints_from_expression(
                    expr, program, resolved_names, codebase, file,
                    range_start, range_end, hints,
                );
            }
            for expr in for_stmt.conditions.iter() {
                collect_hints_from_expression(
                    expr, program, resolved_names, codebase, file,
                    range_start, range_end, hints,
                );
            }
            for expr in for_stmt.increments.iter() {
                collect_hints_from_expression(
                    expr, program, resolved_names, codebase, file,
                    range_start, range_end, hints,
                );
            }
            match &for_stmt.body {
                mago_syntax::ast::ast::r#loop::r#for::ForBody::Statement(stmt) => {
                    collect_hints_from_statement(
                        stmt, program, resolved_names, codebase, file,
                        range_start, range_end, hints,
                    );
                }
                mago_syntax::ast::ast::r#loop::r#for::ForBody::ColonDelimited(body) => {
                    for stmt in body.statements.iter() {
                        collect_hints_from_statement(
                            stmt, program, resolved_names, codebase, file,
                            range_start, range_end, hints,
                        );
                    }
                }
            }
        }
        Statement::Foreach(foreach_stmt) => {
            collect_hints_from_expression(
                foreach_stmt.expression, program, resolved_names, codebase, file,
                range_start, range_end, hints,
            );
            match &foreach_stmt.body {
                mago_syntax::ast::ast::r#loop::foreach::ForeachBody::Statement(stmt) => {
                    collect_hints_from_statement(
                        stmt, program, resolved_names, codebase, file,
                        range_start, range_end, hints,
                    );
                }
                mago_syntax::ast::ast::r#loop::foreach::ForeachBody::ColonDelimited(body) => {
                    for stmt in body.statements.iter() {
                        collect_hints_from_statement(
                            stmt, program, resolved_names, codebase, file,
                            range_start, range_end, hints,
                        );
                    }
                }
            }
        }
        Statement::Switch(switch_stmt) => {
            collect_hints_from_expression(
                switch_stmt.expression, program, resolved_names, codebase, file,
                range_start, range_end, hints,
            );
            for case in switch_stmt.body.cases().iter() {
                let stmts = match case {
                    mago_syntax::ast::ast::control_flow::switch::SwitchCase::Expression(c) => {
                        collect_hints_from_expression(
                            c.expression, program, resolved_names, codebase, file,
                            range_start, range_end, hints,
                        );
                        &c.statements
                    }
                    mago_syntax::ast::ast::control_flow::switch::SwitchCase::Default(c) => {
                        &c.statements
                    }
                };
                for stmt in stmts.iter() {
                    collect_hints_from_statement(
                        stmt, program, resolved_names, codebase, file,
                        range_start, range_end, hints,
                    );
                }
            }
        }
        Statement::Try(try_stmt) => {
            for stmt in try_stmt.block.statements.iter() {
                collect_hints_from_statement(
                    stmt, program, resolved_names, codebase, file,
                    range_start, range_end, hints,
                );
            }
            for catch in try_stmt.catch_clauses.iter() {
                for stmt in catch.block.statements.iter() {
                    collect_hints_from_statement(
                        stmt, program, resolved_names, codebase, file,
                        range_start, range_end, hints,
                    );
                }
            }
            if let Some(finally) = &try_stmt.finally_clause {
                for stmt in finally.block.statements.iter() {
                    collect_hints_from_statement(
                        stmt, program, resolved_names, codebase, file,
                        range_start, range_end, hints,
                    );
                }
            }
        }
        _ => {}
    }
}

/// Walk class-like members (methods) to find calls inside them.
fn collect_hints_from_class_member<'arena>(
    member: &ClassLikeMember<'arena>,
    program: &Program<'arena>,
    resolved_names: &ResolvedNames<'arena>,
    codebase: &CodebaseMetadata,
    file: &File,
    range_start: u32,
    range_end: u32,
    hints: &mut Vec<InlayHint>,
) {
    if let ClassLikeMember::Method(method) = member {
        if let mago_syntax::ast::ast::class_like::method::MethodBody::Concrete(block) = &method.body {
            for stmt in block.statements.iter() {
                collect_hints_from_statement(
                    stmt, program, resolved_names, codebase, file,
                    range_start, range_end, hints,
                );
            }
        }
    }
}

/// Walk an expression tree and collect inlay hints for calls.
fn collect_hints_from_expression<'arena>(
    expr: &Expression<'arena>,
    program: &Program<'arena>,
    resolved_names: &ResolvedNames<'arena>,
    codebase: &CodebaseMetadata,
    file: &File,
    range_start: u32,
    range_end: u32,
    hints: &mut Vec<InlayHint>,
) {
    match expr {
        // Function call: foo(arg1, arg2)
        Expression::Call(Call::Function(call)) => {
            // Try to resolve the function metadata
            if let Expression::Identifier(ident) = &*call.function {
                if let Some(fqn) = resolved_names.resolve(ident) {
                    if let Some(meta) = codebase.get_function(fqn) {
                        emit_parameter_hints(
                            &call.argument_list.arguments, meta, file,
                            range_start, range_end, hints,
                        );
                    }
                }
            }
            // Also recurse into arguments in case they contain nested calls
            for arg in call.argument_list.arguments.iter() {
                collect_hints_from_argument(
                    arg, program, resolved_names, codebase, file,
                    range_start, range_end, hints,
                );
            }
        }
        // Method call: $obj->method(arg1, arg2)
        Expression::Call(Call::Method(call)) => {
            if let ClassLikeMemberSelector::Identifier(method_ident) = &call.method {
                let method_name = method_ident.value.to_string();
                // Try to resolve the object's class
                let class_fqn = resolve_object_class(
                    &call.object, program, resolved_names, codebase,
                );
                if let Some(fqn) = class_fqn {
                    if let Some(meta) = codebase.get_declaring_method(&fqn, &method_name) {
                        emit_parameter_hints(
                            &call.argument_list.arguments, meta, file,
                            range_start, range_end, hints,
                        );
                    }
                }
            }
            // Recurse into object expression
            collect_hints_from_expression(
                &call.object, program, resolved_names, codebase, file,
                range_start, range_end, hints,
            );
            // Recurse into arguments
            for arg in call.argument_list.arguments.iter() {
                collect_hints_from_argument(
                    arg, program, resolved_names, codebase, file,
                    range_start, range_end, hints,
                );
            }
        }
        // Null-safe method call: $obj?->method(arg1, arg2)
        Expression::Call(Call::NullSafeMethod(call)) => {
            if let ClassLikeMemberSelector::Identifier(method_ident) = &call.method {
                let method_name = method_ident.value.to_string();
                let class_fqn = resolve_object_class(
                    &call.object, program, resolved_names, codebase,
                );
                if let Some(fqn) = class_fqn {
                    if let Some(meta) = codebase.get_declaring_method(&fqn, &method_name) {
                        emit_parameter_hints(
                            &call.argument_list.arguments, meta, file,
                            range_start, range_end, hints,
                        );
                    }
                }
            }
            collect_hints_from_expression(
                &call.object, program, resolved_names, codebase, file,
                range_start, range_end, hints,
            );
            for arg in call.argument_list.arguments.iter() {
                collect_hints_from_argument(
                    arg, program, resolved_names, codebase, file,
                    range_start, range_end, hints,
                );
            }
        }
        // Static method call: Class::method(arg1, arg2)
        Expression::Call(Call::StaticMethod(call)) => {
            if let ClassLikeMemberSelector::Identifier(method_ident) = &call.method {
                let method_name = method_ident.value.to_string();
                if let Expression::Identifier(ident) = &*call.class {
                    if let Some(class_fqn) = resolved_names.resolve(ident) {
                        if let Some(meta) = codebase.get_declaring_method(class_fqn, &method_name) {
                            emit_parameter_hints(
                                &call.argument_list.arguments, meta, file,
                                range_start, range_end, hints,
                            );
                        }
                    }
                }
            }
            for arg in call.argument_list.arguments.iter() {
                collect_hints_from_argument(
                    arg, program, resolved_names, codebase, file,
                    range_start, range_end, hints,
                );
            }
        }
        // Instantiation: new ClassName(arg1, arg2)
        Expression::Instantiation(inst) => {
            if let Some(argument_list) = &inst.argument_list {
                if let Expression::Identifier(ident) = &*inst.class {
                    if let Some(class_fqn) = resolved_names.resolve(ident) {
                        if let Some(meta) = codebase.get_declaring_method(class_fqn, "__construct") {
                            emit_parameter_hints(
                                &argument_list.arguments, meta, file,
                                range_start, range_end, hints,
                            );
                        }
                    }
                }
                for arg in argument_list.arguments.iter() {
                    collect_hints_from_argument(
                        arg, program, resolved_names, codebase, file,
                        range_start, range_end, hints,
                    );
                }
            }
        }
        // Recurse into sub-expressions
        Expression::Binary(bin) => {
            collect_hints_from_expression(
                &bin.lhs, program, resolved_names, codebase, file,
                range_start, range_end, hints,
            );
            collect_hints_from_expression(
                &bin.rhs, program, resolved_names, codebase, file,
                range_start, range_end, hints,
            );
        }
        Expression::Assignment(assign) => {
            collect_hints_from_expression(
                &assign.lhs, program, resolved_names, codebase, file,
                range_start, range_end, hints,
            );
            collect_hints_from_expression(
                &assign.rhs, program, resolved_names, codebase, file,
                range_start, range_end, hints,
            );
        }
        Expression::Parenthesized(paren) => {
            collect_hints_from_expression(
                &paren.expression, program, resolved_names, codebase, file,
                range_start, range_end, hints,
            );
        }
        Expression::Conditional(cond) => {
            collect_hints_from_expression(
                &cond.condition, program, resolved_names, codebase, file,
                range_start, range_end, hints,
            );
            if let Some(then_expr) = &cond.then {
                collect_hints_from_expression(
                    then_expr, program, resolved_names, codebase, file,
                    range_start, range_end, hints,
                );
            }
            collect_hints_from_expression(
                &cond.r#else, program, resolved_names, codebase, file,
                range_start, range_end, hints,
            );
        }
        Expression::UnaryPrefix(unary) => {
            collect_hints_from_expression(
                &unary.operand, program, resolved_names, codebase, file,
                range_start, range_end, hints,
            );
        }
        Expression::UnaryPostfix(unary) => {
            collect_hints_from_expression(
                &unary.operand, program, resolved_names, codebase, file,
                range_start, range_end, hints,
            );
        }
        Expression::Access(mago_syntax::ast::ast::Access::Property(access)) => {
            collect_hints_from_expression(
                &access.object, program, resolved_names, codebase, file,
                range_start, range_end, hints,
            );
        }
        Expression::Array(arr) => {
            for element in arr.elements.iter() {
                if let mago_syntax::ast::ast::ArrayElement::KeyValue(kv) = element {
                    collect_hints_from_expression(
                        kv.key, program, resolved_names, codebase, file,
                        range_start, range_end, hints,
                    );
                    collect_hints_from_expression(
                        kv.value, program, resolved_names, codebase, file,
                        range_start, range_end, hints,
                    );
                } else if let mago_syntax::ast::ast::ArrayElement::Value(val) = element {
                    collect_hints_from_expression(
                        val.value, program, resolved_names, codebase, file,
                        range_start, range_end, hints,
                    );
                }
            }
        }
        _ => {}
    }
}

/// Recurse into an argument's value expression to find nested calls.
fn collect_hints_from_argument<'arena>(
    arg: &Argument<'arena>,
    program: &Program<'arena>,
    resolved_names: &ResolvedNames<'arena>,
    codebase: &CodebaseMetadata,
    file: &File,
    range_start: u32,
    range_end: u32,
    hints: &mut Vec<InlayHint>,
) {
    let value = match arg {
        Argument::Positional(a) => a.value,
        Argument::Named(a) => a.value,
    };
    collect_hints_from_expression(
        value, program, resolved_names, codebase, file,
        range_start, range_end, hints,
    );
}

/// Try to resolve the class of an object expression for method call resolution.
fn resolve_object_class<'arena>(
    expr: &Expression<'arena>,
    program: &Program<'arena>,
    resolved_names: &ResolvedNames<'arena>,
    codebase: &CodebaseMetadata,
) -> Option<String> {
    match expr {
        Expression::Variable(mago_syntax::ast::ast::variable::Variable::Direct(dv)) => {
            let var_name = if dv.name.starts_with('$') {
                dv.name.to_string()
            } else {
                format!("${}", dv.name)
            };
            if var_name == "$this" {
                // Find the enclosing class
                return find_enclosing_class_for_this(program, resolved_names, dv.span().start.offset);
            }
            crate::navigate::resolve_variable_class(
                program.statements.iter(), &var_name, resolved_names,
            )
        }
        Expression::Instantiation(inst) => {
            if let Expression::Identifier(ident) = &*inst.class {
                resolved_names.resolve(ident).map(|s| s.to_string())
            } else {
                None
            }
        }
        Expression::Call(Call::Method(call)) => {
            // For chained calls, try to resolve via hover's expression type resolution
            if let ClassLikeMemberSelector::Identifier(method_ident) = &call.method {
                let obj_class = resolve_object_class(&call.object, program, resolved_names, codebase)?;
                if let Some(meta) = codebase.get_declaring_method(&obj_class, &method_ident.value.to_string()) {
                    if let Some(rt) = &meta.return_type_metadata {
                        return Some(rt.type_union.get_id().to_string());
                    }
                }
            }
            None
        }
        Expression::Parenthesized(paren) => {
            resolve_object_class(&paren.expression, program, resolved_names, codebase)
        }
        _ => None,
    }
}

/// Find the enclosing class FQN for `$this` references.
fn find_enclosing_class_for_this<'arena>(
    program: &Program<'arena>,
    resolved_names: &ResolvedNames<'arena>,
    offset: u32,
) -> Option<String> {
    for stmt in program.statements.iter() {
        match stmt {
            Statement::Class(class) if class.span().has_offset(offset) => {
                return resolved_names.resolve(&class.name).map(|s| s.to_string());
            }
            Statement::Namespace(ns) => {
                let stmts = match &ns.body {
                    mago_syntax::ast::ast::NamespaceBody::Implicit(body) => &body.statements,
                    mago_syntax::ast::ast::NamespaceBody::BraceDelimited(block) => &block.statements,
                };
                for s in stmts.iter() {
                    if let Statement::Class(class) = s {
                        if class.span().has_offset(offset) {
                            return resolved_names.resolve(&class.name).map(|s| s.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// For a given argument list and function metadata, emit an InlayHint for each
/// positional argument whose position matches a known parameter.
fn emit_parameter_hints(
    arguments: &mago_syntax::ast::sequence::TokenSeparatedSequence<'_, Argument<'_>>,
    meta: &FunctionLikeMetadata,
    file: &File,
    range_start: u32,
    range_end: u32,
    hints: &mut Vec<InlayHint>,
) {
    for (i, arg) in arguments.iter().enumerate() {
        // Only emit hints for positional arguments, not named ones
        let arg_value = match arg {
            Argument::Positional(a) => a.value,
            Argument::Named(_) => continue,
        };

        let Some(param) = meta.parameters.get(i) else {
            break;
        };

        let arg_offset = arg_value.span().start.offset;

        // Skip arguments outside the requested range
        if arg_offset < range_start || arg_offset > range_end {
            continue;
        }

        let param_name = param.name.0.to_string();

        // Skip if the argument already matches the parameter name (e.g. passing $name for $name)
        let arg_end = arg_value.span().end_offset() as usize;
        let arg_start = arg_offset as usize;
        if arg_end <= file.contents.len() {
            let arg_text = file.contents[arg_start..arg_end].trim();
            if arg_text == &param_name || arg_text.trim_start_matches('$') == param_name.trim_start_matches('$') {
                continue;
            }
        }

        hints.push(InlayHint {
            position: convert::offset_to_lsp_position(file, arg_offset),
            label: InlayHintLabel::String(format!("{}:", param_name)),
            kind: Some(InlayHintKind::PARAMETER),
            text_edits: None,
            tooltip: None,
            padding_left: None,
            padding_right: Some(true),
            data: None,
        });
    }
}
