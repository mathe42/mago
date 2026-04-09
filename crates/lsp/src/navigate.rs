use mago_codex::metadata::CodebaseMetadata;
use mago_names::ResolvedNames;
use mago_span::HasSpan;
use mago_span::Span;
use mago_syntax::ast::Program;
use mago_syntax::ast::ast::Access;
use mago_syntax::ast::ast::Call;
use mago_syntax::ast::ast::ClassLikeConstantSelector;
use mago_syntax::ast::ast::ClassLikeMemberSelector;
use mago_syntax::ast::ast::Expression;
use mago_syntax::ast::ast::Hint;
use mago_syntax::ast::ast::Instantiation;
use mago_syntax::ast::ast::Statement;
use mago_syntax::ast::ast::function_like::parameter::FunctionLikeParameterList;

/// What the cursor is pointing at, resolved to a symbol.
#[derive(Debug)]
pub enum SymbolAt<'a> {
    /// A class-like name (class, interface, trait, enum).
    ClassLike { fqn: &'a str, span: Span },
    /// A function name.
    Function { fqn: &'a str, span: Span },
    /// A method call or definition — class + method name.
    Method { class_fqn: String, method_name: String, span: Span },
    /// A property access — class + property name.
    Property { class_fqn: String, property_name: String, span: Span },
    /// A class constant access.
    ClassConstant { class_fqn: String, constant_name: String, span: Span },
    /// A variable reference, optionally resolved to its definition (e.g. a function parameter).
    Variable { name: String, definition_span: Option<Span>, span: Span },
    /// An unknown or unresolvable position.
    Unknown,
}

/// Find the symbol at a given byte offset in the AST.
pub fn find_symbol_at_offset<'ast, 'arena>(
    program: &'ast Program<'arena>,
    resolved_names: &'ast ResolvedNames<'arena>,
    codebase: &CodebaseMetadata,
    offset: u32,
) -> SymbolAt<'ast> {
    for statement in program.statements.iter() {
        if !statement.span().has_offset(offset) {
            continue;
        }

        if let Some(sym) = find_in_statement(statement, resolved_names, offset) {
            // If we found a variable without a definition, try to find its first assignment.
            if let SymbolAt::Variable { ref name, definition_span: None, span } = sym {
                if let Some(def_span) = find_variable_assignment(program.statements.iter(), name) {
                    return SymbolAt::Variable { name: name.clone(), definition_span: Some(def_span), span };
                }
            }
            // If we found a method/property with empty class_fqn, try to resolve the variable type.
            // First try simple resolution, then fall back to expression-based (handles chaining).
            match &sym {
                SymbolAt::Method { class_fqn, method_name, span } if class_fqn.is_empty() => {
                    if let Some(fqn) = resolve_object_class_with_codebase(program, resolved_names, codebase, offset) {
                        return SymbolAt::Method { class_fqn: fqn, method_name: method_name.clone(), span: *span };
                    }
                }
                SymbolAt::Property { class_fqn, property_name, span } if class_fqn.is_empty() => {
                    if let Some(fqn) = resolve_object_class_with_codebase(program, resolved_names, codebase, offset) {
                        return SymbolAt::Property { class_fqn: fqn, property_name: property_name.clone(), span: *span };
                    }
                }
                _ => {}
            }
            return sym;
        }
    }

    SymbolAt::Unknown
}

/// Try to resolve the class of the object in a `$var->member` expression at the given offset.
/// Handles simple variables ($a->), $this->, and chains ($a->method()->).
fn resolve_object_class_at_offset<'ast, 'arena>(
    program: &'ast Program<'arena>,
    resolved_names: &'ast ResolvedNames<'arena>,
    offset: u32,
) -> Option<String> {
    // First try simple variable resolution.
    for statement in program.statements.iter() {
        if !statement.span().has_offset(offset) {
            continue;
        }
        if let Some(var_name) = extract_object_variable(statement, offset) {
            if var_name == "$this" {
                return find_enclosing_class(program, resolved_names, offset);
            }
            if let Some(fqn) = resolve_variable_class(program.statements.iter(), &var_name, resolved_names) {
                return Some(fqn);
            }
        }
    }
    None
}

/// Resolve object class using full expression type resolution (for chaining).
/// This requires codebase access for return type lookups.
pub fn resolve_object_class_with_codebase<'ast, 'arena>(
    program: &'ast Program<'arena>,
    resolved_names: &'ast ResolvedNames<'arena>,
    codebase: &CodebaseMetadata,
    offset: u32,
) -> Option<String> {
    // First try simple resolution
    if let Some(fqn) = resolve_object_class_at_offset(program, resolved_names, offset) {
        return Some(fqn);
    }
    // Then try expression-based resolution (handles chaining)
    for statement in program.statements.iter() {
        if !statement.span().has_offset(offset) {
            continue;
        }
        if let Some(obj_expr) = find_object_expression(statement, offset) {
            return crate::handlers::hover::resolve_expression_type(obj_expr, program, resolved_names, codebase);
        }
    }
    None
}

/// Find the object expression before `->` at the given offset.
fn find_object_expression<'ast, 'arena>(
    statement: &'ast Statement<'arena>,
    offset: u32,
) -> Option<&'ast Expression<'arena>> {
    match statement {
        Statement::Expression(expr_stmt) => find_obj_expr_in(&expr_stmt.expression, offset),
        Statement::Echo(echo) => {
            for val in echo.values.iter() {
                if let Some(e) = find_obj_expr_in(val, offset) {
                    return Some(e);
                }
            }
            None
        }
        Statement::Return(ret) => ret.value.as_ref().and_then(|v| find_obj_expr_in(v, offset)),
        _ => None,
    }
}

fn find_obj_expr_in<'ast, 'arena>(
    expr: &'ast Expression<'arena>,
    offset: u32,
) -> Option<&'ast Expression<'arena>> {
    if !expr.span().has_offset(offset) {
        return None;
    }
    match expr {
        Expression::Call(Call::Method(call)) => {
            if call.method.span().has_offset(offset) {
                return Some(&call.object);
            }
            find_obj_expr_in(&call.object, offset)
        }
        Expression::Access(Access::Property(access)) => {
            if access.property.span().has_offset(offset) {
                return Some(&access.object);
            }
            find_obj_expr_in(&access.object, offset)
        }
        Expression::Assignment(assign) => {
            find_obj_expr_in(&assign.lhs, offset)
                .or_else(|| find_obj_expr_in(&assign.rhs, offset))
        }
        _ => None,
    }
}

/// Extract the variable name from the object in a `$var->member` or `$var->method()` expression.
fn extract_object_variable<'arena>(statement: &Statement<'arena>, offset: u32) -> Option<String> {
    match statement {
        Statement::Expression(expr_stmt) => extract_object_var_from_expr(&expr_stmt.expression, offset),
        Statement::Return(ret) => ret.value.as_ref().and_then(|v| extract_object_var_from_expr(v, offset)),
        Statement::Echo(echo) => {
            for val in echo.values.iter() {
                if let Some(name) = extract_object_var_from_expr(val, offset) {
                    return Some(name);
                }
            }
            None
        }
        Statement::Namespace(ns) => {
            let stmts = match &ns.body {
                mago_syntax::ast::ast::NamespaceBody::Implicit(body) => &body.statements,
                mago_syntax::ast::ast::NamespaceBody::BraceDelimited(block) => &block.statements,
            };
            for stmt in stmts.iter() {
                if stmt.span().has_offset(offset) {
                    if let Some(name) = extract_object_variable(stmt, offset) {
                        return Some(name);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

fn extract_object_var_from_expr<'arena>(expr: &Expression<'arena>, offset: u32) -> Option<String> {
    if !expr.span().has_offset(offset) {
        return None;
    }
    match expr {
        Expression::Call(Call::Method(call)) => {
            // If cursor is anywhere in this method call (on method name, args, etc.)
            // resolve the object variable.
            if let Expression::Variable(mago_syntax::ast::ast::variable::Variable::Direct(dv)) = &*call.object {
                return Some(format!("${}", dv.name));
            }
            // Object might be a chained expression
            extract_object_var_from_expr(&*call.object, offset)
        }
        Expression::Access(Access::Property(access)) => {
            if let Expression::Variable(mago_syntax::ast::ast::variable::Variable::Direct(dv)) = &*access.object {
                return Some(format!("${}", dv.name));
            }
            extract_object_var_from_expr(&*access.object, offset)
        }
        Expression::Assignment(assign) => {
            extract_object_var_from_expr(&*assign.lhs, offset)
                .or_else(|| extract_object_var_from_expr(&*assign.rhs, offset))
        }
        Expression::Binary(bin) => {
            extract_object_var_from_expr(&*bin.lhs, offset)
                .or_else(|| extract_object_var_from_expr(&*bin.rhs, offset))
        }
        _ => None,
    }
}

/// Resolve a variable to its class by looking for `$var = new ClassName()` in previous statements.
pub fn resolve_variable_class<'ast, 'arena: 'ast>(
    statements: impl Iterator<Item = &'ast Statement<'arena>>,
    var_name: &str,
    resolved_names: &'ast ResolvedNames<'arena>,
) -> Option<String> {
    for stmt in statements {
        if let Statement::Expression(expr_stmt) = stmt {
            if let Expression::Assignment(assign) = &expr_stmt.expression {
                if let Expression::Variable(mago_syntax::ast::ast::variable::Variable::Direct(dv)) = &*assign.lhs {
                    let lhs_name = if dv.name.starts_with('$') { dv.name.to_string() } else { format!("${}", dv.name) };
                    if lhs_name == var_name {
                        if let Expression::Instantiation(Instantiation { class, .. }) = &*assign.rhs {
                            if let Expression::Identifier(ident) = &**class {
                                if let Some(fqn) = resolved_names.resolve(ident) {
                                    return Some(fqn.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
        if let Statement::Namespace(ns) = stmt {
            let stmts = match &ns.body {
                mago_syntax::ast::ast::NamespaceBody::Implicit(body) => &body.statements,
                mago_syntax::ast::ast::NamespaceBody::BraceDelimited(block) => &block.statements,
            };
            if let Some(fqn) = resolve_variable_class(stmts.iter(), var_name, resolved_names) {
                return Some(fqn);
            }
        }
    }
    None
}

/// Find the enclosing class FQCN for `$this` references.
fn find_enclosing_class<'ast, 'arena>(
    program: &'ast Program<'arena>,
    resolved_names: &'ast ResolvedNames<'arena>,
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

/// Search statements for the first assignment to a variable (e.g. `$message = ...`).
fn find_variable_assignment<'ast, 'arena: 'ast>(
    statements: impl Iterator<Item = &'ast Statement<'arena>>,
    var_name: &str,
) -> Option<Span> {
    for stmt in statements {
        if let Statement::Expression(expr_stmt) = stmt {
            if let Expression::Assignment(assign) = &expr_stmt.expression {
                if let Expression::Variable(mago_syntax::ast::ast::variable::Variable::Direct(dv)) = &*assign.lhs {
                    if dv.name == var_name {
                        return Some(dv.span());
                    }
                }
            }
        }
        // Also search in namespaces
        if let Statement::Namespace(ns) = stmt {
            let stmts = match &ns.body {
                mago_syntax::ast::ast::NamespaceBody::Implicit(body) => &body.statements,
                mago_syntax::ast::ast::NamespaceBody::BraceDelimited(block) => &block.statements,
            };
            if let Some(span) = find_variable_assignment(stmts.iter(), var_name) {
                return Some(span);
            }
        }
    }
    None
}

fn find_in_statement<'ast, 'arena>(
    statement: &'ast Statement<'arena>,
    resolved_names: &'ast ResolvedNames<'arena>,
    offset: u32,
) -> Option<SymbolAt<'ast>> {
    match statement {
        Statement::Class(class) => {
            if class.name.span().has_offset(offset) {
                if let Some(fqn) = resolved_names.resolve(&class.name) {
                    return Some(SymbolAt::ClassLike { fqn, span: class.name.span() });
                }
            }
            if let Some(extends) = &class.extends {
                for parent in extends.types.iter() {
                    if parent.span().has_offset(offset) {
                        if let Some(fqn) = resolved_names.resolve(parent) {
                            return Some(SymbolAt::ClassLike { fqn, span: parent.span() });
                        }
                    }
                }
            }
            if let Some(implements) = &class.implements {
                for iface in implements.types.iter() {
                    if iface.span().has_offset(offset) {
                        if let Some(fqn) = resolved_names.resolve(iface) {
                            return Some(SymbolAt::ClassLike { fqn, span: iface.span() });
                        }
                    }
                }
            }
            let class_fqn_str = resolved_names.resolve(&class.name).unwrap_or(class.name.value);
            for member in class.members.iter() {
                if member.span().has_offset(offset) {
                    if let Some(sym) = find_in_member_expressions(member, class_fqn_str, resolved_names, offset) {
                        return Some(sym);
                    }
                }
            }
            None
        }
        Statement::Interface(iface) => {
            if iface.name.span().has_offset(offset) {
                resolved_names.resolve(&iface.name).map(|fqn| SymbolAt::ClassLike { fqn, span: iface.name.span() })
            } else {
                None
            }
        }
        Statement::Trait(r#trait) => {
            if r#trait.name.span().has_offset(offset) {
                return resolved_names
                    .resolve(&r#trait.name)
                    .map(|fqn| SymbolAt::ClassLike { fqn, span: r#trait.name.span() });
            }
            let trait_fqn = resolved_names.resolve(&r#trait.name).unwrap_or(r#trait.name.value);
            for member in r#trait.members.iter() {
                if member.span().has_offset(offset) {
                    if let Some(sym) = find_in_member_expressions(member, trait_fqn, resolved_names, offset) {
                        return Some(sym);
                    }
                }
            }
            None
        }
        Statement::Enum(r#enum) => {
            if r#enum.name.span().has_offset(offset) {
                resolved_names
                    .resolve(&r#enum.name)
                    .map(|fqn| SymbolAt::ClassLike { fqn, span: r#enum.name.span() })
            } else {
                None
            }
        }
        Statement::Function(func) => {
            if func.name.span().has_offset(offset) {
                if let Some(fqn) = resolved_names.resolve(&func.name) {
                    return Some(SymbolAt::Function { fqn, span: func.name.span() });
                }
            }
            // Check if cursor is on a parameter variable
            for param in func.parameter_list.parameters.iter() {
                if param.variable.span().has_offset(offset) {
                    return Some(SymbolAt::Variable {
                        name: param.variable.name.to_string(),
                        definition_span: Some(param.variable.span()),
                        span: param.variable.span(),
                    });
                }
            }
            for stmt in func.body.statements.iter() {
                if stmt.span().has_offset(offset) {
                    if let Some(sym) = find_in_statement_with_params(stmt, resolved_names, &func.parameter_list, offset) {
                        return Some(sym);
                    }
                }
            }
            None
        }
        Statement::Expression(expr_stmt) => find_in_expression(&expr_stmt.expression, resolved_names, offset),
        Statement::Echo(echo) => {
            for val in echo.values.iter() {
                if let Some(sym) = find_in_expression(val, resolved_names, offset) {
                    return Some(sym);
                }
            }
            None
        }
        Statement::Return(ret) => ret.value.as_ref().and_then(|val| find_in_expression(val, resolved_names, offset)),
        Statement::Block(block) => {
            for stmt in block.statements.iter() {
                if stmt.span().has_offset(offset) {
                    if let Some(sym) = find_in_statement(stmt, resolved_names, offset) {
                        return Some(sym);
                    }
                }
            }
            None
        }
        Statement::If(if_stmt) => find_in_expression(&if_stmt.condition, resolved_names, offset),
        Statement::Namespace(ns) => {
            let stmts = match &ns.body {
                mago_syntax::ast::ast::NamespaceBody::Implicit(body) => &body.statements,
                mago_syntax::ast::ast::NamespaceBody::BraceDelimited(block) => &block.statements,
            };
            for stmt in stmts.iter() {
                if stmt.span().has_offset(offset) {
                    if let Some(sym) = find_in_statement(stmt, resolved_names, offset) {
                        return Some(sym);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

fn find_in_expression<'ast, 'arena>(
    expr: &'ast Expression<'arena>,
    resolved_names: &'ast ResolvedNames<'arena>,
    offset: u32,
) -> Option<SymbolAt<'ast>> {
    find_in_expression_with_params(expr, resolved_names, None, offset)
}

fn find_in_expression_with_params<'ast, 'arena>(
    expr: &'ast Expression<'arena>,
    resolved_names: &'ast ResolvedNames<'arena>,
    params: Option<&'ast FunctionLikeParameterList<'arena>>,
    offset: u32,
) -> Option<SymbolAt<'ast>> {
    if !expr.span().has_offset(offset) {
        return None;
    }

    match expr {
        // Variable reference: $name
        Expression::Variable(mago_syntax::ast::ast::variable::Variable::Direct(var)) => {
            if var.span().has_offset(offset) {
                let def_span = params.and_then(|p| {
                    p.parameters.iter().find(|param| param.variable.name == var.name).map(|param| param.variable.span())
                });
                return Some(SymbolAt::Variable {
                    name: var.name.to_string(),
                    definition_span: def_span,
                    span: var.span(),
                });
            }
            None
        }
        // Function call: foo()
        Expression::Call(Call::Function(call)) => {
            if call.function.span().has_offset(offset) {
                if let Expression::Identifier(ident) = &*call.function {
                    if let Some(fqn) = resolved_names.resolve(ident) {
                        return Some(SymbolAt::Function { fqn, span: ident.span() });
                    }
                }
            }
            for arg in call.argument_list.arguments.iter() {
                if let Some(sym) = find_in_arg_with_params(arg, resolved_names, params, offset) {
                    return Some(sym);
                }
            }
            None
        }
        // Method call: $obj->method()
        Expression::Call(Call::Method(call)) => {
            if let ClassLikeMemberSelector::Identifier(method_ident) = &call.method {
                if method_ident.span().has_offset(offset) {
                    return Some(SymbolAt::Method {
                        class_fqn: String::new(),
                        method_name: method_ident.value.to_string(),
                        span: method_ident.span(),
                    });
                }
            }
            find_in_expression_with_params(&*call.object, resolved_names, params, offset)
        }
        // Static method call: Class::method()
        Expression::Call(Call::StaticMethod(call)) => {
            if let ClassLikeMemberSelector::Identifier(method_ident) = &call.method {
                if method_ident.span().has_offset(offset) {
                    if let Some(fqn) = resolve_class_expression(&*call.class, resolved_names) {
                        return Some(SymbolAt::Method {
                            class_fqn: fqn.to_string(),
                            method_name: method_ident.value.to_string(),
                            span: method_ident.span(),
                        });
                    }
                }
            }
            if call.class.span().has_offset(offset) {
                if let Some(fqn) = resolve_class_expression(&*call.class, resolved_names) {
                    return Some(SymbolAt::ClassLike { fqn, span: call.class.span() });
                }
            }
            None
        }
        // Property access: $obj->prop
        Expression::Access(Access::Property(access)) => {
            if let ClassLikeMemberSelector::Identifier(prop_ident) = &access.property {
                if prop_ident.span().has_offset(offset) {
                    return Some(SymbolAt::Property {
                        class_fqn: String::new(),
                        property_name: prop_ident.value.to_string(),
                        span: prop_ident.span(),
                    });
                }
            }
            find_in_expression_with_params(&*access.object, resolved_names, params, offset)
        }
        // Static property: Class::$prop
        Expression::Access(Access::StaticProperty(access)) => {
            if access.class.span().has_offset(offset) {
                if let Some(fqn) = resolve_class_expression(&*access.class, resolved_names) {
                    return Some(SymbolAt::ClassLike { fqn, span: access.class.span() });
                }
            }
            None
        }
        // Class constant: Class::CONST
        Expression::Access(Access::ClassConstant(access)) => {
            if let Some(fqn) = resolve_class_expression(&*access.class, resolved_names) {
                if access.class.span().has_offset(offset) {
                    return Some(SymbolAt::ClassLike { fqn, span: access.class.span() });
                }
                if let ClassLikeConstantSelector::Identifier(const_ident) = &access.constant {
                    if const_ident.span().has_offset(offset) {
                        return Some(SymbolAt::ClassConstant {
                            class_fqn: fqn.to_string(),
                            constant_name: const_ident.value.to_string(),
                            span: const_ident.span(),
                        });
                    }
                }
            }
            None
        }
        // new ClassName()
        Expression::Instantiation(Instantiation { class, .. }) => {
            if class.span().has_offset(offset) {
                if let Some(fqn) = resolve_class_expression(&*class, resolved_names) {
                    return Some(SymbolAt::ClassLike { fqn, span: class.span() });
                }
            }
            None
        }
        // Bare identifier
        Expression::Identifier(ident) => {
            resolved_names.resolve(ident).map(|fqn| SymbolAt::ClassLike { fqn, span: ident.span() })
        }
        Expression::ConstantAccess(ca) => {
            resolved_names.resolve(&ca.name).map(|fqn| SymbolAt::ClassLike { fqn, span: ca.name.span() })
        }
        // Recurse into sub-expressions
        Expression::Binary(bin) => find_in_expression_with_params(&*bin.lhs, resolved_names, params, offset)
            .or_else(|| find_in_expression_with_params(&*bin.rhs, resolved_names, params, offset)),
        Expression::Assignment(assign) => find_in_expression_with_params(&*assign.lhs, resolved_names, params, offset)
            .or_else(|| find_in_expression_with_params(&*assign.rhs, resolved_names, params, offset)),
        Expression::Parenthesized(paren) => find_in_expression_with_params(&*paren.expression, resolved_names, params, offset),
        _ => None,
    }
}

fn find_in_statement_with_params<'ast, 'arena>(
    statement: &'ast Statement<'arena>,
    resolved_names: &'ast ResolvedNames<'arena>,
    params: &'ast FunctionLikeParameterList<'arena>,
    offset: u32,
) -> Option<SymbolAt<'ast>> {
    match statement {
        Statement::Expression(expr_stmt) => find_in_expression_with_params(&expr_stmt.expression, resolved_names, Some(params), offset),
        Statement::Echo(echo) => {
            for val in echo.values.iter() {
                if let Some(sym) = find_in_expression_with_params(val, resolved_names, Some(params), offset) {
                    return Some(sym);
                }
            }
            None
        }
        Statement::Return(ret) => ret.value.as_ref().and_then(|val| find_in_expression_with_params(val, resolved_names, Some(params), offset)),
        Statement::Block(block) => {
            for stmt in block.statements.iter() {
                if stmt.span().has_offset(offset) {
                    if let Some(sym) = find_in_statement_with_params(stmt, resolved_names, params, offset) {
                        return Some(sym);
                    }
                }
            }
            None
        }
        Statement::If(if_stmt) => find_in_expression_with_params(&if_stmt.condition, resolved_names, Some(params), offset),
        _ => find_in_statement(statement, resolved_names, offset),
    }
}

fn find_in_arg_with_params<'ast, 'arena>(
    arg: &'ast mago_syntax::ast::ast::Argument<'arena>,
    resolved_names: &'ast ResolvedNames<'arena>,
    params: Option<&'ast FunctionLikeParameterList<'arena>>,
    offset: u32,
) -> Option<SymbolAt<'ast>> {
    match arg {
        mago_syntax::ast::ast::Argument::Positional(a) => find_in_expression_with_params(&a.value, resolved_names, params, offset),
        mago_syntax::ast::ast::Argument::Named(a) => find_in_expression_with_params(&a.value, resolved_names, params, offset),
        _ => None,
    }
}

fn find_in_expression_from_arg<'ast, 'arena>(
    arg: &'ast mago_syntax::ast::ast::Argument<'arena>,
    resolved_names: &'ast ResolvedNames<'arena>,
    offset: u32,
) -> Option<SymbolAt<'ast>> {
    match arg {
        mago_syntax::ast::ast::Argument::Positional(a) => find_in_expression(&a.value, resolved_names, offset),
        mago_syntax::ast::ast::Argument::Named(a) => find_in_expression(&a.value, resolved_names, offset),
        _ => None,
    }
}

fn find_in_member_expressions<'ast, 'arena>(
    member: &'ast mago_syntax::ast::ast::ClassLikeMember<'arena>,
    class_fqn: &str,
    resolved_names: &'ast ResolvedNames<'arena>,
    offset: u32,
) -> Option<SymbolAt<'ast>> {
    use mago_syntax::ast::ast::ClassLikeMember;
    match member {
        ClassLikeMember::Method(method) => {
            // Method name hover
            if method.name.span().has_offset(offset) {
                return Some(SymbolAt::Method {
                    class_fqn: class_fqn.to_string(),
                    method_name: method.name.value.to_string(),
                    span: method.name.span(),
                });
            }
            // Parameter hover
            for param in method.parameter_list.parameters.iter() {
                if param.variable.span().has_offset(offset) {
                    return Some(SymbolAt::Variable {
                        name: param.variable.name.to_string(),
                        definition_span: Some(param.variable.span()),
                        span: param.variable.span(),
                    });
                }
            }
            // Return type hint
            if let Some(hint) = &method.return_type_hint {
                if let Some(sym) = find_in_hint(&hint.hint, resolved_names, offset) {
                    return Some(sym);
                }
            }
            // Method body — pass parameters for variable resolution
            if let mago_syntax::ast::ast::MethodBody::Concrete(block) = &method.body {
                for stmt in block.statements.iter() {
                    if stmt.span().has_offset(offset) {
                        if let Some(sym) = find_in_statement_with_params(stmt, resolved_names, &method.parameter_list, offset) {
                            return Some(sym);
                        }
                    }
                }
            }
            None
        }
        ClassLikeMember::Property(prop) => {
            match prop {
                mago_syntax::ast::ast::class_like::property::Property::Plain(plain) => {
                    for item in plain.items.iter() {
                        let var = item.variable();
                        if var.span().has_offset(offset) {
                            return Some(SymbolAt::Property {
                                class_fqn: class_fqn.to_string(),
                                property_name: var.name.to_string(),
                                span: var.span(),
                            });
                        }
                    }
                    // Type hint hover
                    if let Some(hint) = &plain.hint {
                        if let Some(sym) = find_in_hint(hint, resolved_names, offset) {
                            return Some(sym);
                        }
                    }
                }
                mago_syntax::ast::ast::class_like::property::Property::Hooked(hooked) => {
                    let var = hooked.item.variable();
                    if var.span().has_offset(offset) {
                        return Some(SymbolAt::Property {
                            class_fqn: class_fqn.to_string(),
                            property_name: var.name.to_string(),
                            span: var.span(),
                        });
                    }
                    if let Some(hint) = &hooked.hint {
                        if let Some(sym) = find_in_hint(hint, resolved_names, offset) {
                            return Some(sym);
                        }
                    }
                }
            }
            None
        }
        ClassLikeMember::TraitUse(trait_use) => {
            for trait_name in trait_use.trait_names.iter() {
                if trait_name.span().has_offset(offset) {
                    if let Some(fqn) = resolved_names.resolve(trait_name) {
                        return Some(SymbolAt::ClassLike { fqn, span: trait_name.span() });
                    }
                }
            }
            None
        }
        ClassLikeMember::Constant(constant) => {
            // TODO: handle constant hover
            None
        }
        _ => None,
    }
}

fn find_in_hint<'ast, 'arena>(
    hint: &'ast Hint<'arena>,
    resolved_names: &'ast ResolvedNames<'arena>,
    offset: u32,
) -> Option<SymbolAt<'ast>> {
    if !hint.span().has_offset(offset) {
        return None;
    }
    match hint {
        Hint::Identifier(ident) => {
            resolved_names.resolve(ident).map(|fqn| SymbolAt::ClassLike { fqn, span: ident.span() })
        }
        Hint::Nullable(h) => find_in_hint(&*h.hint, resolved_names, offset),
        Hint::Union(h) => {
            find_in_hint(&*h.left, resolved_names, offset).or_else(|| find_in_hint(&*h.right, resolved_names, offset))
        }
        Hint::Intersection(h) => {
            find_in_hint(&*h.left, resolved_names, offset).or_else(|| find_in_hint(&*h.right, resolved_names, offset))
        }
        Hint::Parenthesized(h) => find_in_hint(&*h.hint, resolved_names, offset),
        _ => None,
    }
}

fn resolve_class_expression<'ast, 'arena>(
    expr: &'ast Expression<'arena>,
    resolved_names: &'ast ResolvedNames<'arena>,
) -> Option<&'ast str> {
    match expr {
        Expression::Identifier(ident) => resolved_names.resolve(ident),
        _ => None,
    }
}
