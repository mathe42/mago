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
    /// An unknown or unresolvable position.
    Unknown,
}

/// Find the symbol at a given byte offset in the AST.
pub fn find_symbol_at_offset<'ast, 'arena>(
    program: &'ast Program<'arena>,
    resolved_names: &'ast ResolvedNames<'arena>,
    _codebase: &CodebaseMetadata,
    offset: u32,
) -> SymbolAt<'ast> {
    for statement in program.statements.iter() {
        if !statement.span().has_offset(offset) {
            continue;
        }

        if let Some(sym) = find_in_statement(statement, resolved_names, offset) {
            return sym;
        }
    }

    SymbolAt::Unknown
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
            for member in class.members.iter() {
                if member.span().has_offset(offset) {
                    if let Some(sym) = find_in_member_expressions(member, resolved_names, offset) {
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
                resolved_names
                    .resolve(&r#trait.name)
                    .map(|fqn| SymbolAt::ClassLike { fqn, span: r#trait.name.span() })
            } else {
                None
            }
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
            for stmt in func.body.statements.iter() {
                if stmt.span().has_offset(offset) {
                    if let Some(sym) = find_in_statement(stmt, resolved_names, offset) {
                        return Some(sym);
                    }
                }
            }
            None
        }
        Statement::Expression(expr_stmt) => find_in_expression(&expr_stmt.expression, resolved_names, offset),
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
    if !expr.span().has_offset(offset) {
        return None;
    }

    match expr {
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
                if let Some(sym) = find_in_expression_from_arg(arg, resolved_names, offset) {
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
            find_in_expression(&*call.object, resolved_names, offset)
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
            find_in_expression(&*access.object, resolved_names, offset)
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
        Expression::Binary(bin) => find_in_expression(&*bin.lhs, resolved_names, offset)
            .or_else(|| find_in_expression(&*bin.rhs, resolved_names, offset)),
        Expression::Assignment(assign) => find_in_expression(&*assign.lhs, resolved_names, offset)
            .or_else(|| find_in_expression(&*assign.rhs, resolved_names, offset)),
        Expression::Parenthesized(paren) => find_in_expression(&*paren.expression, resolved_names, offset),
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
    resolved_names: &'ast ResolvedNames<'arena>,
    offset: u32,
) -> Option<SymbolAt<'ast>> {
    use mago_syntax::ast::ast::ClassLikeMember;
    match member {
        ClassLikeMember::Method(method) => {
            if let Some(hint) = &method.return_type_hint {
                if let Some(sym) = find_in_hint(&hint.hint, resolved_names, offset) {
                    return Some(sym);
                }
            }
            if let mago_syntax::ast::ast::MethodBody::Concrete(block) = &method.body {
                for stmt in block.statements.iter() {
                    if stmt.span().has_offset(offset) {
                        if let Some(sym) = find_in_statement(stmt, resolved_names, offset) {
                            return Some(sym);
                        }
                    }
                }
            }
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
