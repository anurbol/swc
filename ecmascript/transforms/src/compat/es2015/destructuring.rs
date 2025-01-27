use crate::{
    pass::Pass,
    util::{
        alias_ident_for, alias_if_required, has_rest_pat, is_literal, prop_name_to_expr, undefined,
        ExprFactory, StmtLike,
    },
};
use ast::*;
use serde::Deserialize;
use std::iter;
use swc_common::{Fold, FoldWith, Spanned, SyntaxContext, Visit, VisitWith, DUMMY_SP};

#[cfg(test)]
mod tests;

/// `@babel/plugin-transform-destructuring`
///
/// # Example
/// ## In
/// ```js
/// let {x, y} = obj;
///
/// let [a, b, ...rest] = arr;
/// ```
/// ## Out
/// ```js
/// let _obj = obj,
///     x = _obj.x,
///     y = _obj.y;
///
/// let _arr = arr,
///     _arr2 = _toArray(_arr),
///     a = _arr2[0],
///     b = _arr2[1],
///     rest = _arr2.slice(2);
/// ```
pub fn destructuring(c: Config) -> impl Pass {
    Destructuring { c }
}

struct Destructuring {
    c: Config,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub loose: bool,
}

macro_rules! impl_for_for_stmt {
    ($T:tt) => {
        impl Fold<$T> for Destructuring {
            fn fold(&mut self, mut for_stmt: $T) -> $T {
                let (left, stmt) = match for_stmt.left {
                    VarDeclOrPat::VarDecl(var_decl) => {
                        let has_complex = var_decl.decls.iter().any(|d| match d.name {
                            Pat::Ident(_) => false,
                            _ => true,
                        });

                        if !has_complex {
                            return $T {
                                left: VarDeclOrPat::VarDecl(var_decl),
                                ..for_stmt
                            };
                        }
                        let ref_ident = make_ref_ident_for_for_stmt();
                        let left = VarDeclOrPat::VarDecl(VarDecl {
                            decls: vec![VarDeclarator {
                                span: DUMMY_SP,
                                name: Pat::Ident(ref_ident.clone()),
                                init: None,
                                definite: false,
                            }],
                            ..var_decl
                        });
                        // Unpack variables
                        let stmt = Stmt::Decl(Decl::Var(VarDecl {
                            span: var_decl.span(),
                            kind: VarDeclKind::Let,
                            // I(kdy1) guess var_decl.len() == 1
                            decls: var_decl
                                .decls
                                .into_iter()
                                .map(|decl| VarDeclarator {
                                    init: Some(box Expr::Ident(ref_ident.clone())),
                                    ..decl
                                })
                                .collect::<Vec<_>>()
                                .fold_with(self),
                            declare: false,
                        }));
                        (left, stmt)
                    }
                    VarDeclOrPat::Pat(pat) => match pat {
                        Pat::Ident(..) => {
                            return $T {
                                left: VarDeclOrPat::Pat(pat),
                                ..for_stmt
                            };
                        }
                        _ => {
                            let left_ident = make_ref_ident_for_for_stmt();
                            let left = VarDeclOrPat::Pat(Pat::Ident(left_ident.clone()));
                            // Unpack variables
                            let stmt = Stmt::Expr(box Expr::Assign(AssignExpr {
                                span: DUMMY_SP,
                                left: PatOrExpr::Pat(box pat),
                                op: op!("="),
                                right: box left_ident.into(),
                            }));
                            (left, stmt)
                        }
                    },
                };
                for_stmt.left = left;

                for_stmt.body = box Stmt::Block(match *for_stmt.body {
                    Stmt::Block(BlockStmt { span, stmts }) => BlockStmt {
                        span,
                        stmts: iter::once(stmt).chain(stmts).collect(),
                    },
                    body => BlockStmt {
                        span: DUMMY_SP,
                        stmts: vec![stmt, body],
                    },
                });

                for_stmt
            }
        }
    };
}
impl_for_for_stmt!(ForInStmt);
impl_for_for_stmt!(ForOfStmt);

fn make_ref_ident_for_for_stmt() -> Ident {
    private_ident!("ref")
}

impl Fold<Vec<VarDeclarator>> for AssignFolder {
    fn fold(&mut self, declarators: Vec<VarDeclarator>) -> Vec<VarDeclarator> {
        let declarators = declarators.fold_children(self);

        let is_complex = declarators.iter().any(|d| match d.name {
            Pat::Ident(..) => false,
            _ => true,
        });
        if !is_complex {
            return declarators;
        }
        let mut decls = Vec::with_capacity(declarators.len());

        for decl in declarators {
            self.fold_var_decl(&mut decls, decl)
        }

        decls
    }
}

impl Fold<Stmt> for AssignFolder {
    fn fold(&mut self, s: Stmt) -> Stmt {
        match s {
            Stmt::Expr(e) => {
                self.ignore_return_value = Some(());
                let e = e.fold_with(self);
                assert_eq!(self.ignore_return_value, None);
                Stmt::Expr(e)
            }
            _ => s.fold_children(self),
        }
    }
}

impl AssignFolder {
    fn fold_var_decl(&mut self, decls: &mut Vec<VarDeclarator>, decl: VarDeclarator) {
        match decl.name {
            Pat::Ident(..) => decls.push(decl),
            Pat::Rest(..) => unreachable!(
                "rest pattern should handled by array pattern handler: {:?}",
                decl.name
            ),
            Pat::Array(ArrayPat { elems, .. }) => {
                assert!(
                    decl.init.is_some(),
                    "destructuring pattern binding requires initializer"
                );

                let init = decl.init.unwrap();

                if is_literal(&init) {
                    match *init {
                        Expr::Array(arr)
                            if elems.len() == arr.elems.len() || has_rest_pat(&elems) =>
                        {
                            let mut arr_elems = Some(arr.elems.into_iter());
                            elems.into_iter().for_each(|p| match p {
                                Some(Pat::Rest(p)) => {
                                    self.fold_var_decl(
                                        decls,
                                        VarDeclarator {
                                            span: p.span(),
                                            name: *p.arg,
                                            init: Some(box Expr::Array(ArrayLit {
                                                span: DUMMY_SP,
                                                elems: arr_elems
                                                    .take()
                                                    .expect("two rest element?")
                                                    .collect(),
                                            })),
                                            definite: false,
                                        },
                                    );
                                }
                                Some(p) => {
                                    let e = arr_elems
                                        .as_mut()
                                        .expect("pattern after rest element?")
                                        .next()
                                        .unwrap();
                                    self.fold_var_decl(
                                        decls,
                                        VarDeclarator {
                                            span: p.span(),
                                            init: e.map(|e| {
                                                debug_assert_eq!(e.spread, None);
                                                e.expr
                                            }),
                                            name: p,
                                            definite: false,
                                        },
                                    )
                                }

                                None => {}
                            });
                            return;
                        }
                        _ => {}
                    }
                }
                // Make ref var if required
                let ref_ident = make_ref_ident_for_array(
                    self.c,
                    if self.exporting {
                        &mut self.vars
                    } else {
                        decls
                    },
                    Some(init),
                    Some(if has_rest_pat(&elems) {
                        std::usize::MAX
                    } else {
                        elems.len()
                    }),
                );

                for (i, elem) in elems.into_iter().enumerate() {
                    let elem: Pat = match elem {
                        Some(elem) => elem,
                        None => continue,
                    };

                    let var_decl = match elem {
                        Pat::Rest(RestPat {
                            dot3_token,
                            box arg,
                            ..
                        }) => VarDeclarator {
                            span: dot3_token,
                            name: arg,
                            init: Some(box Expr::Call(CallExpr {
                                span: DUMMY_SP,
                                callee: ref_ident.clone().member(quote_ident!("slice")).as_callee(),
                                args: vec![Lit::Num(Number {
                                    value: i as f64,
                                    span: dot3_token,
                                })
                                .as_arg()],
                                type_args: Default::default(),
                            })),
                            definite: false,
                        },
                        _ => VarDeclarator {
                            span: elem.span(),
                            // This might be pattern.
                            // So we fold it again.
                            name: elem,
                            init: Some(box make_ref_idx_expr(&ref_ident, i)),
                            definite: false,
                        },
                    };
                    decls.extend(vec![var_decl].fold_with(self));
                }
            }
            Pat::Object(ObjectPat { span, props, .. }) if props.is_empty() => {
                let (ident, aliased) = alias_if_required(&decl.init.as_ref().unwrap(), "ref");
                if aliased {
                    decls.push(VarDeclarator {
                        span: DUMMY_SP,
                        name: Pat::Ident(ident.clone()),
                        init: decl.init,
                        definite: false,
                    });
                }

                // We should convert
                //
                //      var {} = null;
                //
                // to
                //
                //      var _ref = null;
                //      _objectDestructuringEmpty(_ref);
                //
                decls.push(VarDeclarator {
                    span,
                    name: Pat::Ident(ident.clone()),
                    init: Some(box Expr::Cond(CondExpr {
                        span: DUMMY_SP,
                        test: box Expr::Bin(BinExpr {
                            span: DUMMY_SP,
                            left: box Expr::Ident(ident.clone()),
                            op: op!("!=="),
                            right: box Expr::Lit(Lit::Null(Null { span: DUMMY_SP })),
                        }),
                        cons: box Expr::Ident(ident.clone()),
                        alt: box Expr::Call(CallExpr {
                            span: DUMMY_SP,
                            callee: helper!(throw, "throw"),
                            args: vec![
                                // new TypeError("Cannot destructure undefined")
                                NewExpr {
                                    span: DUMMY_SP,
                                    callee: box Expr::Ident(Ident::new(
                                        "TypeError".into(),
                                        DUMMY_SP,
                                    )),
                                    args: Some(vec![Lit::Str(Str {
                                        span: DUMMY_SP,
                                        value: "Cannot destructure undefined".into(),
                                        has_escape: false,
                                    })
                                    .as_arg()]),
                                    type_args: Default::default(),
                                }
                                .as_arg(),
                            ],
                            type_args: Default::default(),
                        }),
                    })),
                    definite: false,
                })
            }

            Pat::Object(ObjectPat { props, .. }) => {
                assert!(
                    decl.init.is_some(),
                    "destructuring pattern binding requires initializer"
                );

                let can_be_null = can_be_null(decl.init.as_ref().unwrap());
                let ref_ident = make_ref_ident(self.c, decls, decl.init);

                let ref_ident = if can_be_null {
                    let init = box Expr::Ident(ref_ident.clone());
                    make_ref_ident(self.c, decls, Some(init))
                } else {
                    ref_ident
                };

                for prop in props {
                    let prop_span = prop.span();

                    match prop {
                        ObjectPatProp::KeyValue(KeyValuePatProp { key, value }) => {
                            let computed = match key {
                                PropName::Computed(..) => true,
                                _ => false,
                            };

                            let var_decl = VarDeclarator {
                                span: prop_span,
                                name: *value,
                                init: Some(box make_ref_prop_expr(
                                    &ref_ident,
                                    box prop_name_to_expr(key),
                                    computed,
                                )),
                                definite: false,
                            };
                            decls.extend(vec![var_decl].fold_with(self));
                        }
                        ObjectPatProp::Assign(AssignPatProp { key, value, .. }) => {
                            let computed = false;

                            match value {
                                Some(value) => {
                                    let ref_ident = make_ref_ident(
                                        self.c,
                                        decls,
                                        Some(box make_ref_prop_expr(
                                            &ref_ident,
                                            box key.clone().into(),
                                            computed,
                                        )),
                                    );

                                    let var_decl = VarDeclarator {
                                        span: prop_span,
                                        name: Pat::Ident(key.clone()),
                                        init: Some(box make_cond_expr(ref_ident, value)),
                                        definite: false,
                                    };
                                    decls.extend(vec![var_decl].fold_with(self));
                                }
                                None => {
                                    let var_decl = VarDeclarator {
                                        span: prop_span,
                                        name: Pat::Ident(key.clone()),
                                        init: Some(box make_ref_prop_expr(
                                            &ref_ident,
                                            box key.clone().into(),
                                            computed,
                                        )),
                                        definite: false,
                                    };
                                    decls.extend(vec![var_decl].fold_with(self));
                                }
                            }
                        }
                        ObjectPatProp::Rest(..) => unreachable!(
                            "Object rest pattern should be removed by es2018::object_rest_spread \
                             pass"
                        ),
                    }
                }
            }
            Pat::Assign(AssignPat {
                span,
                left,
                right: def_value,
                ..
            }) => {
                assert!(
                    decl.init.is_some(),
                    "desturcturing pattern binding requires initializer"
                );

                let tmp_ident = match decl.init {
                    Some(box Expr::Ident(ref i)) if i.span.ctxt() != SyntaxContext::empty() => {
                        i.clone()
                    }
                    _ => {
                        let tmp_ident = private_ident!(span, "tmp");
                        decls.push(VarDeclarator {
                            span: DUMMY_SP,
                            name: Pat::Ident(tmp_ident.clone()),
                            init: decl.init,
                            definite: false,
                        });

                        tmp_ident
                    }
                };

                let var_decl = VarDeclarator {
                    span,
                    name: *left,
                    // tmp === void 0 ? def_value : tmp
                    init: Some(box make_cond_expr(tmp_ident, def_value)),
                    definite: false,
                };
                decls.extend(vec![var_decl].fold_with(self))
            }

            _ => unimplemented!("Pattern {:?}", decl),
        }
    }
}

impl_fold_fn!(Destructuring);

impl Destructuring {
    fn fold_fn_like(&mut self, ps: Vec<Pat>, body: BlockStmt) -> (Vec<Pat>, BlockStmt) {
        let mut params = vec![];
        let mut decls = vec![];

        for pat in ps {
            let span = pat.span();
            match pat {
                Pat::Ident(..) => params.push(pat),
                Pat::Array(..) | Pat::Object(..) | Pat::Assign(..) => {
                    let ref_ident = private_ident!(span, "ref");

                    params.push(Pat::Ident(ref_ident.clone()));
                    decls.push(VarDeclarator {
                        span,
                        name: pat,
                        init: Some(box Expr::Ident(ref_ident)),
                        definite: false,
                    })
                }
                _ => {}
            }
        }

        let stmts = if decls.is_empty() {
            body.stmts
        } else {
            iter::once(
                Stmt::Decl(Decl::Var(VarDecl {
                    span: DUMMY_SP,
                    kind: VarDeclKind::Let,
                    decls,
                    declare: false,
                }))
                .fold_with(self),
            )
            .chain(body.stmts)
            .collect()
        };

        (params, BlockStmt { stmts, ..body })
    }
}

struct AssignFolder {
    c: Config,
    exporting: bool,
    vars: Vec<VarDeclarator>,
    /// Used like `.take().is_some()`.
    ignore_return_value: Option<()>,
}

impl Fold<ExportDecl> for AssignFolder {
    fn fold(&mut self, decl: ExportDecl) -> ExportDecl {
        let old = self.exporting;
        self.exporting = true;
        let decl = decl.fold_children(self);
        self.exporting = old;
        decl
    }
}

impl Fold<Expr> for AssignFolder {
    fn fold(&mut self, expr: Expr) -> Expr {
        let ignore_return_value = self.ignore_return_value.take().is_some();

        let expr = match expr {
            // Handle iife
            Expr::Fn(..) | Expr::Object(..) => Destructuring { c: self.c }.fold(expr),
            _ => expr.fold_children(self),
        };

        match expr {
            Expr::Assign(AssignExpr {
                span,
                left,
                op: op!("="),
                right,
            }) => match left {
                PatOrExpr::Pat(pat) => match *pat {
                    Pat::Expr(expr) => Expr::Assign(AssignExpr {
                        span,
                        left: PatOrExpr::Expr(expr),
                        op: op!("="),
                        right,
                    }),

                    Pat::Ident(..) => Expr::Assign(AssignExpr {
                        span,
                        left: PatOrExpr::Pat(pat),
                        op: op!("="),
                        right,
                    }),
                    Pat::Array(ArrayPat { elems, .. }) => {
                        let mut exprs = Vec::with_capacity(elems.len() + 1);

                        if is_literal(&right) && ignore_return_value {
                            match *right {
                                Expr::Array(arr)
                                    if elems.len() == arr.elems.len() || has_rest_pat(&elems) =>
                                {
                                    let mut arr_elems = Some(arr.elems.into_iter());
                                    elems.into_iter().for_each(|p| match p {
                                        Some(Pat::Rest(p)) => {
                                            exprs.push(box Expr::Assign(AssignExpr {
                                                span: p.span(),
                                                left: PatOrExpr::Pat(p.arg),
                                                op: op!("="),
                                                right: box Expr::Array(ArrayLit {
                                                    span: DUMMY_SP,
                                                    elems: arr_elems
                                                        .take()
                                                        .expect("two rest element?")
                                                        .collect(),
                                                }),
                                            }));
                                        }
                                        Some(p) => {
                                            let e = arr_elems
                                                .as_mut()
                                                .expect("pattern after rest element?")
                                                .next()
                                                .and_then(|v| v);
                                            let right = e
                                                .map(|e| {
                                                    debug_assert_eq!(e.spread, None);
                                                    e.expr
                                                })
                                                .unwrap_or_else(|| undefined(p.span()));
                                            exprs.push(box Expr::Assign(AssignExpr {
                                                span: p.span(),
                                                left: PatOrExpr::Pat(box p),
                                                op: op!("="),
                                                right,
                                            }));
                                        }

                                        None => {}
                                    });
                                    return SeqExpr { span, exprs }.into();
                                }
                                _ => {}
                            }
                        }

                        // initialized by first element of sequence expression
                        let ref_ident = make_ref_ident_for_array(
                            self.c,
                            &mut self.vars,
                            None,
                            Some(if has_rest_pat(&elems) {
                                std::usize::MAX
                            } else {
                                elems.len()
                            }),
                        );

                        exprs.push(box Expr::Assign(AssignExpr {
                            span: DUMMY_SP,
                            op: op!("="),
                            left: PatOrExpr::Pat(box Pat::Ident(ref_ident.clone())),
                            right,
                        }));

                        for (i, elem) in elems.into_iter().enumerate() {
                            let elem = match elem {
                                Some(elem) => elem,
                                None => continue,
                            };
                            let elem_span = elem.span();

                            match elem {
                                Pat::Assign(AssignPat {
                                    span, left, right, ..
                                }) => {
                                    // initialized by sequence expression.
                                    let assign_ref_ident =
                                        make_ref_ident(self.c, &mut self.vars, None);
                                    exprs.push(box Expr::Assign(AssignExpr {
                                        span: DUMMY_SP,
                                        left: PatOrExpr::Pat(box Pat::Ident(
                                            assign_ref_ident.clone(),
                                        )),
                                        op: op!("="),
                                        right: box ref_ident.clone().computed_member(i as f64),
                                    }));

                                    exprs.push(
                                        box Expr::Assign(AssignExpr {
                                            span,
                                            left: PatOrExpr::Pat(left),
                                            op: op!("="),
                                            right: box make_cond_expr(assign_ref_ident, right),
                                        })
                                        .fold_with(self),
                                    );
                                }
                                Pat::Rest(RestPat { arg, .. }) => exprs.push(
                                    box Expr::Assign(AssignExpr {
                                        span: elem_span,
                                        op: op!("="),
                                        left: PatOrExpr::Pat(arg),
                                        right: box Expr::Call(CallExpr {
                                            span: DUMMY_SP,
                                            callee: ref_ident
                                                .clone()
                                                .member(quote_ident!("slice"))
                                                .as_callee(),
                                            args: vec![(i as f64).as_arg()],
                                            type_args: Default::default(),
                                        }),
                                    })
                                    .fold_with(self),
                                ),
                                _ => exprs.push(
                                    box Expr::Assign(AssignExpr {
                                        span: elem_span,
                                        op: op!("="),
                                        left: PatOrExpr::Pat(box elem),
                                        right: box make_ref_idx_expr(&ref_ident, i),
                                    })
                                    .fold_with(self),
                                ),
                            }
                        }

                        // last one should be `ref`
                        exprs.push(box Expr::Ident(ref_ident));

                        Expr::Seq(SeqExpr {
                            span: DUMMY_SP,
                            exprs,
                        })
                    }
                    Pat::Object(ObjectPat { span, props, .. }) => {
                        let ref_ident = make_ref_ident(self.c, &mut self.vars, None);

                        let mut exprs = vec![];

                        exprs.push(box Expr::Assign(AssignExpr {
                            span,
                            left: PatOrExpr::Pat(box Pat::Ident(ref_ident.clone())),
                            op: op!("="),
                            right,
                        }));

                        for prop in props {
                            let span = prop.span();
                            match prop {
                                ObjectPatProp::KeyValue(KeyValuePatProp { key, value }) => {
                                    let computed = match key {
                                        PropName::Computed(..) => true,
                                        _ => false,
                                    };

                                    exprs.push(box Expr::Assign(AssignExpr {
                                        span,
                                        left: PatOrExpr::Pat(value),
                                        op: op!("="),
                                        right: box make_ref_prop_expr(
                                            &ref_ident,
                                            box prop_name_to_expr(key),
                                            computed,
                                        ),
                                    }));
                                }
                                ObjectPatProp::Assign(AssignPatProp { key, value, .. }) => {
                                    let computed = false;

                                    match value {
                                        Some(value) => {
                                            let prop_ident =
                                                make_ref_ident(self.c, &mut self.vars, None);

                                            exprs.push(box Expr::Assign(AssignExpr {
                                                span,
                                                left: PatOrExpr::Pat(box Pat::Ident(
                                                    prop_ident.clone(),
                                                )),
                                                op: op!("="),
                                                right: box make_ref_prop_expr(
                                                    &ref_ident,
                                                    box key.clone().into(),
                                                    computed,
                                                ),
                                            }));

                                            exprs.push(box Expr::Assign(AssignExpr {
                                                span,
                                                left: PatOrExpr::Pat(box Pat::Ident(key.clone())),
                                                op: op!("="),
                                                right: box make_cond_expr(prop_ident, value),
                                            }));
                                        }
                                        None => {
                                            exprs.push(box Expr::Assign(AssignExpr {
                                                span,
                                                left: PatOrExpr::Pat(box Pat::Ident(key.clone())),
                                                op: op!("="),
                                                right: box make_ref_prop_expr(
                                                    &ref_ident,
                                                    box key.clone().into(),
                                                    computed,
                                                ),
                                            }));
                                        }
                                    }
                                }
                                ObjectPatProp::Rest(_) => unreachable!(
                                    "object rest pattern should be removed by \
                                     es2018::object_rest_spread pass"
                                ),
                            }
                        }

                        // Last one should be object itself.
                        exprs.push(box Expr::Ident(ref_ident));

                        Expr::Seq(SeqExpr {
                            span: DUMMY_SP,
                            exprs,
                        })
                    }
                    Pat::Assign(pat) => unimplemented!("assignment pattern {:?}", pat),
                    Pat::Rest(pat) => unimplemented!("rest pattern {:?}", pat),

                    Pat::Invalid(..) => unreachable!(),
                },
                _ => Expr::Assign(AssignExpr {
                    span,
                    left,
                    op: op!("="),
                    right,
                }),
            },
            _ => expr,
        }
    }
}

impl<T: StmtLike + VisitWith<DestructuringVisitor>> Fold<Vec<T>> for Destructuring
where
    Vec<T>: FoldWith<Self>,
    T: FoldWith<AssignFolder>,
{
    fn fold(&mut self, stmts: Vec<T>) -> Vec<T> {
        // fast path
        if !has_destruturing(&stmts) {
            return stmts;
        }

        let stmts = stmts.fold_children(self);

        let mut buf = Vec::with_capacity(stmts.len());

        for stmt in stmts {
            let mut folder = AssignFolder {
                c: self.c,
                exporting: false,
                vars: vec![],
                ignore_return_value: None,
            };

            match stmt.try_into_stmt() {
                Err(item) => {
                    let item = item.fold_with(&mut folder);

                    // Add variable declaration
                    // e.g. var ref
                    if !folder.vars.is_empty() {
                        buf.push(T::from_stmt(Stmt::Decl(Decl::Var(VarDecl {
                            span: DUMMY_SP,
                            kind: VarDeclKind::Var,
                            decls: folder.vars,
                            declare: false,
                        }))));
                    }

                    buf.push(item)
                }
                Ok(stmt) => {
                    let stmt = stmt.fold_with(&mut folder);

                    // Add variable declaration
                    // e.g. var ref
                    if !folder.vars.is_empty() {
                        buf.push(T::from_stmt(Stmt::Decl(Decl::Var(VarDecl {
                            span: DUMMY_SP,
                            kind: VarDeclKind::Var,
                            decls: folder.vars,
                            declare: false,
                        }))));
                    }

                    buf.push(T::from_stmt(stmt));
                }
            }
        }

        buf
    }
}

fn make_ref_idx_expr(ref_ident: &Ident, i: usize) -> Expr {
    ref_ident.clone().computed_member(i as f64)
}

fn make_ref_ident(c: Config, decls: &mut Vec<VarDeclarator>, init: Option<Box<Expr>>) -> Ident {
    make_ref_ident_for_array(c, decls, init, None)
}

fn make_ref_ident_for_array(
    c: Config,
    decls: &mut Vec<VarDeclarator>,
    init: Option<Box<Expr>>,
    elem_cnt: Option<usize>,
) -> Ident {
    match init {
        Some(box Expr::Ident(i)) if elem_cnt.is_none() => i,
        init => {
            let span = init.span();

            let (ref_ident, aliased) = if c.loose {
                if let Some(ref init) = init {
                    alias_if_required(&init, "ref")
                } else {
                    (private_ident!(span, "ref"), true)
                }
            } else {
                if let Some(ref init) = init {
                    (alias_ident_for(&init, "ref"), true)
                } else {
                    (private_ident!(span, "ref"), true)
                }
            };

            if aliased {
                decls.push(VarDeclarator {
                    span,
                    name: Pat::Ident(ref_ident.clone()),
                    init: init.map(|v| {
                        if c.loose
                            || match *v {
                                Expr::Array(..) => true,
                                _ => false,
                            }
                        {
                            v
                        } else {
                            match elem_cnt {
                                None => v,
                                Some(std::usize::MAX) => box CallExpr {
                                    span: DUMMY_SP,
                                    callee: helper!(to_array, "toArray"),
                                    args: vec![v.as_arg()],
                                    type_args: Default::default(),
                                }
                                .into(),
                                Some(value) => box CallExpr {
                                    span: DUMMY_SP,
                                    callee: helper!(sliced_to_array, "slicedToArray"),
                                    args: vec![
                                        v.as_arg(),
                                        Lit::Num(Number {
                                            span: DUMMY_SP,
                                            value: value as _,
                                        })
                                        .as_arg(),
                                    ],
                                    type_args: Default::default(),
                                }
                                .into(),
                            }
                        }
                    }),
                    definite: false,
                });
            }

            ref_ident
        }
    }
}

fn make_ref_prop_expr(ref_ident: &Ident, prop: Box<Expr>, mut computed: bool) -> Expr {
    computed |= match *prop {
        Expr::Lit(Lit::Num(..)) | Expr::Lit(Lit::Str(..)) => true,
        _ => false,
    };

    Expr::Member(MemberExpr {
        span: DUMMY_SP,
        obj: ExprOrSuper::Expr(box ref_ident.clone().into()),
        computed,
        prop,
    })
}

/// Creates `tmp === void 0 ? def_value : tmp`
fn make_cond_expr(tmp: Ident, def_value: Box<Expr>) -> Expr {
    Expr::Cond(CondExpr {
        span: DUMMY_SP,
        test: box Expr::Bin(BinExpr {
            span: DUMMY_SP,
            left: box Expr::Ident(tmp.clone()),
            op: op!("==="),
            right: box Expr::Unary(UnaryExpr {
                span: DUMMY_SP,
                op: op!("void"),
                arg: box Expr::Lit(Lit::Num(Number {
                    span: DUMMY_SP,
                    value: 0.0,
                })),
            }),
        }),
        cons: def_value,
        alt: box Expr::Ident(tmp),
    })
}

fn can_be_null(e: &Expr) -> bool {
    match *e {
        Expr::Lit(Lit::Null(..))
        | Expr::This(..)
        | Expr::Ident(..)
        | Expr::PrivateName(..)
        | Expr::Member(..)
        | Expr::Call(..)
        | Expr::New(..)
        | Expr::Yield(..)
        | Expr::Await(..)
        | Expr::MetaProp(..) => true,

        // This does not include null
        Expr::Lit(..) => false,

        Expr::Array(..)
        | Expr::Arrow(..)
        | Expr::Object(..)
        | Expr::Fn(..)
        | Expr::Class(..)
        | Expr::Tpl(..) => false,

        Expr::TaggedTpl(..) => true,

        Expr::Paren(ParenExpr { ref expr, .. }) => can_be_null(expr),
        Expr::Seq(SeqExpr { ref exprs, .. }) => {
            exprs.last().map(|e| can_be_null(e)).unwrap_or(true)
        }
        Expr::Assign(AssignExpr { ref right, .. }) => can_be_null(right),
        Expr::Cond(CondExpr {
            ref cons, ref alt, ..
        }) => can_be_null(cons) || can_be_null(alt),

        // TODO(kdy1): I'm not sure about this.
        Expr::Unary(..) | Expr::Update(..) | Expr::Bin(..) => true,

        Expr::JSXMebmer(..)
        | Expr::JSXNamespacedName(..)
        | Expr::JSXEmpty(..)
        | Expr::JSXElement(..)
        | Expr::JSXFragment(..) => unreachable!("destructuring jsx"),

        // Trust user
        Expr::TsNonNull(..) => false,
        Expr::TsAs(TsAsExpr { ref expr, .. })
        | Expr::TsTypeAssertion(TsTypeAssertion { ref expr, .. })
        | Expr::TsTypeCast(TsTypeCastExpr { ref expr, .. })
        | Expr::TsConstAssertion(TsConstAssertion { ref expr, .. }) => can_be_null(expr),
        Expr::TsOptChain(ref e) => can_be_null(&e.expr),

        Expr::Invalid(..) => unreachable!(),
    }
}

fn has_destruturing<N>(node: &N) -> bool
where
    N: VisitWith<DestructuringVisitor>,
{
    let mut v = DestructuringVisitor { found: false };
    node.visit_with(&mut v);
    v.found
}

struct DestructuringVisitor {
    found: bool,
}

impl Visit<Pat> for DestructuringVisitor {
    fn visit(&mut self, node: &Pat) {
        node.visit_children(self);
        match *node {
            Pat::Ident(..) => {}
            _ => self.found = true,
        }
    }
}
