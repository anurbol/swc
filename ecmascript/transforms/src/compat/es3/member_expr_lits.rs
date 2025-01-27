use crate::util::is_valid_ident;
use ast::*;
use swc_common::{Fold, FoldWith};

/// babel: `transform-member-expression-literals`
///
/// # Input
/// ```js
/// obj["foo"] = "isValid";
///
/// obj.const = "isKeyword";
/// obj["var"] = "isKeyword";
/// ```
///
/// # Output
/// ```js
/// obj.foo = "isValid";
///
/// obj["const"] = "isKeyword";
/// obj["var"] = "isKeyword";
/// ```
#[derive(Default, Clone, Copy)]
pub struct MemberExprLit;

impl Fold<Module> for MemberExprLit {
    fn fold(&mut self, node: Module) -> Module {
        validate!(node.fold_children(self))
    }
}

impl Fold<MemberExpr> for MemberExprLit {
    fn fold(&mut self, e: MemberExpr) -> MemberExpr {
        let mut e = validate!(e.fold_children(self));

        macro_rules! handle {
            ($sym:expr, $span:expr) => {
                if $sym.is_reserved_for_es3() || !is_valid_ident(&$sym) {
                    return MemberExpr {
                        computed: true,
                        prop: box Expr::Lit(Lit::Str(Str {
                            span: $span,
                            value: $sym,
                            has_escape: false,
                        })),
                        ..e
                    };
                } else {
                    return MemberExpr {
                        computed: false,
                        prop: box Expr::Ident(quote_ident!($span, $sym)),
                        ..e
                    };
                }
            };
        }

        e.prop = match *e.prop {
            Expr::Lit(Lit::Str(Str { value, span, .. })) => handle!(value, span),
            Expr::Ident(i) => {
                if e.computed {
                    box Expr::Ident(i)
                } else {
                    handle!(i.sym, i.span)
                }
            }
            _ => e.prop,
        };

        e
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    test!(
        ::swc_ecma_parser::Syntax::default(),
        |_| MemberExprLit,
        basic,
        r#"obj["foo"] = "isValid";

obj.const = "isKeyword";
obj["var"] = "isKeyword";"#,
        r#"obj.foo = "isValid";

obj["const"] = "isKeyword";
obj["var"] = "isKeyword";"#
    );

    test!(
        ::swc_ecma_parser::Syntax::default(),
        |_| MemberExprLit,
        issue_206,
        "const number = foo[bar1][baz1]",
        "const number = foo[bar1][baz1]"
    );

    test!(
        ::swc_ecma_parser::Syntax::default(),
        |_| MemberExprLit,
        issue_211,
        "_query[idx]=$this.attr('data-ref');",
        "_query[idx]=$this.attr('data-ref');"
    );
}
